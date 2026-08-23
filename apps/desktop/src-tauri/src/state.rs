use base64::Engine;
use chrono::{DateTime, Utc};
use infimount_core::runtime::OperatorCache;
use infimount_core::workspaces::WorkspaceRegistry;
use infimount_core::{
    config, secrets, CoreError, SecretStore, SecretStoreStatus, Source, SourceKind,
};
use infimount_mcp::confirmation::ConfirmationManager;
use infimount_mcp::errors::{err, err_with_details, McpError, McpErrorCode, McpResult};
use infimount_mcp::registry::{StorageRecord, StorageRegistry};
use infimount_mcp::runtime::{
    is_loopback_bind_address, start_http_server_from_settings, McpHttpServerHandle,
    HTTP_ENDPOINT_PATH,
};
use infimount_mcp::session::SessionManager;
use infimount_mcp::settings::{
    resolve_auth_token, McpSettings, McpSettingsStore, McpTransport, MCP_AUTH_TOKEN_ACCOUNT,
};
use infimount_mcp::telemetry::{ProductEvent, ProductEventName, ProductEventStore};
use infimount_mcp::tools_fs::FsToolsContext;
use opendal::Operator;
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex as StdMutex};
use tokio::sync::Mutex;

use crate::app_settings::AppSettingsStore;

use rand::Rng;

pub struct AppState {
    pub registry: StorageRegistry,
    pub settings_store: McpSettingsStore,
    pub app_settings_store: AppSettingsStore,
    pub confirmations: ConfirmationManager,
    pub sessions: SessionManager,
    pub secret_store: Arc<dyn SecretStore>,
    pub pending_oauth: PendingOAuthStore,
    pub workspaces: WorkspaceRegistry,
    pub product_events: ProductEventStore,
    pub operator_cache: OperatorCache,
    http_runtime: Mutex<Option<McpHttpServerHandle>>,
    pub(crate) lifecycle_mutation: Mutex<()>,
    transfer_cancellations: StdMutex<HashSet<String>>,
    /// Sanitized machine-readable failure code. Raw keyring/config errors never cross
    /// the Tauri boundary.
    startup_error: StdMutex<Option<String>>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StartupHealth {
    pub operational: bool,
    pub recovery_available: bool,
    pub error_code: Option<String>,
    pub message: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct McpRuntimeSettings {
    pub enabled: bool,
    pub transport: McpTransport,
    pub bind_address: String,
    pub port: u16,
    pub enabled_tools: Vec<String>,
    pub security_baseline_version: u32,
    pub auth_token_configured: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct McpRuntimeStatus {
    pub settings: McpRuntimeSettings,
    pub running_http: bool,
    pub endpoint: Option<String>,
    pub endpoint_display: String,
    pub auth_token_configured: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct McpClientSnippets {
    pub stdio: String,
    pub http: String,
}

pub struct PendingOAuthStore {
    sessions: StdMutex<HashMap<String, PendingOAuthSession>>,
    claimed: StdMutex<HashSet<String>>,
    consumed: StdMutex<HashSet<String>>,
    expired: StdMutex<HashSet<String>>,
}

pub enum PendingOAuthClaim {
    Session(PendingOAuthSession),
    Expired,
    AlreadyUsed,
    InUse,
    NotFound,
}

pub struct PendingOAuthSession {
    pub id: String,
    pub provider: String,
    pub secret_config: Value,
    pub public_config: Value,
    pub expires_at: DateTime<Utc>,
    pub consumed: AtomicBool,
}

impl PendingOAuthStore {
    pub fn new() -> Self {
        Self {
            sessions: StdMutex::new(HashMap::new()),
            claimed: StdMutex::new(HashSet::new()),
            consumed: StdMutex::new(HashSet::new()),
            expired: StdMutex::new(HashSet::new()),
        }
    }

    pub fn insert(&self, session: PendingOAuthSession) {
        let mut sessions = self.sessions.lock().unwrap();
        sessions.insert(session.id.clone(), session);
    }

    pub fn snapshot(&self, id: &str) -> Option<(String, Value, Value)> {
        self.remove_expired();
        let sessions = self.sessions.lock().ok()?;
        let session = sessions.get(id)?;
        Some((
            session.provider.clone(),
            session.public_config.clone(),
            session.secret_config.clone(),
        ))
    }

    pub fn claim(&self, id: &str) -> PendingOAuthClaim {
        self.remove_expired();
        if self.was_consumed(id) {
            return PendingOAuthClaim::AlreadyUsed;
        }
        if self.expired.lock().is_ok_and(|items| items.contains(id)) {
            return PendingOAuthClaim::Expired;
        }
        if self.claimed.lock().is_ok_and(|items| items.contains(id)) {
            return PendingOAuthClaim::InUse;
        }
        let Some(session) = self
            .sessions
            .lock()
            .ok()
            .and_then(|mut items| items.remove(id))
        else {
            return PendingOAuthClaim::NotFound;
        };
        if let Ok(mut claimed) = self.claimed.lock() {
            claimed.insert(id.to_string());
        }
        PendingOAuthClaim::Session(session)
    }

    pub fn complete(&self, session: PendingOAuthSession) {
        if let Ok(mut claimed) = self.claimed.lock() {
            claimed.remove(&session.id);
        }
        session.consumed.store(true, Ordering::SeqCst);
        if let Ok(mut consumed) = self.consumed.lock() {
            consumed.insert(session.id.clone());
        }
    }

    pub fn restore(&self, session: PendingOAuthSession) {
        if let Ok(mut claimed) = self.claimed.lock() {
            claimed.remove(&session.id);
        }
        if Utc::now() <= session.expires_at && !session.consumed.load(Ordering::SeqCst) {
            self.insert(session);
        } else if let Ok(mut expired) = self.expired.lock() {
            expired.insert(session.id.clone());
        }
    }

    pub fn was_consumed(&self, id: &str) -> bool {
        self.consumed
            .lock()
            .map(|used| used.contains(id))
            .unwrap_or(false)
    }

    pub fn cancel(&self, id: &str) -> bool {
        self.sessions
            .lock()
            .map(|mut sessions| sessions.remove(id).is_some())
            .unwrap_or(false)
    }

    pub fn remove_expired(&self) {
        let now = Utc::now();
        let mut sessions = self.sessions.lock().unwrap();
        let expired_ids = sessions
            .iter()
            .filter(|(_, session)| now > session.expires_at)
            .map(|(id, _)| id.clone())
            .collect::<Vec<_>>();
        sessions.retain(|_, session| {
            now <= session.expires_at && !session.consumed.load(Ordering::SeqCst)
        });
        drop(sessions);
        if let Ok(mut expired) = self.expired.lock() {
            expired.extend(expired_ids);
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(tag = "action", rename_all = "snake_case")]
pub enum SecretMutation {
    Keep,
    Set { value: String },
    Clear,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "action", rename_all = "snake_case")]
pub enum AuthTokenMutation {
    Keep,
    Set { value: String },
    Clear,
    Rotate,
}

impl AppState {
    pub fn new() -> McpResult<Self> {
        let config_dir = infimount_mcp::registry::default_config_dir();
        let mut startup_error: Option<String> = None;

        let native_store = Arc::new(secrets::NativeSecretStore::new());
        let native_available = matches!(native_store.status(), SecretStoreStatus::Available);
        let secret_store: Arc<dyn SecretStore> = if native_available {
            native_store
        } else {
            startup_error = Some("ERR_STARTUP_SECRET_STORE_UNAVAILABLE".to_string());
            Arc::new(secrets::UnavailableSecretStore::new(
                "desktop is in restricted recovery mode",
            ))
        };
        let registry = StorageRegistry::with_secret_store(None, secret_store.clone());
        let settings_store = McpSettingsStore::with_secret_store(None, secret_store.clone());

        // Configuration parsing and migration failures must not discard an available
        // native store: a valid encrypted recovery backup may be the only safe repair.
        if native_available {
            let initialization = (|| -> McpResult<()> {
                let _config_transaction = registry.acquire_configuration_transaction()?;
                infimount_mcp::migration_cleanup::retry_pending_plaintext_cleanup(&config_dir)?;
                registry.recover_pending_imports_locked()?;
                infimount_mcp::registry::retry_pending_secret_cleanup_at(
                    registry.path(),
                    secret_store.as_ref(),
                )?;
                migrate_legacy_sources_if_needed(&registry)?;
                let storages = registry.load_all()?;
                let settings = settings_store.load()?;
                infimount_mcp::registry::recover_pending_secret_transactions(
                    registry.path(),
                    &storages,
                    secret_store.as_ref(),
                    settings.auth_token_ref.as_deref(),
                )?;
                Ok(())
            })();
            if let Err(error) = initialization {
                startup_error = Some(if error.code == McpErrorCode::ERR_SECRET_MIGRATION_FAILED {
                    "ERR_STARTUP_MIGRATION_CLEANUP".to_string()
                } else {
                    "ERR_STARTUP_INITIALIZATION".to_string()
                });
            }
        }

        let workspaces = WorkspaceRegistry::new(&config_dir);

        let state = Self {
            registry,
            settings_store,
            app_settings_store: AppSettingsStore::new(None),
            confirmations: ConfirmationManager::new(),
            sessions: SessionManager::new(),
            secret_store,
            pending_oauth: PendingOAuthStore::new(),
            workspaces,
            product_events: ProductEventStore::new(None),
            operator_cache: OperatorCache::new(),
            http_runtime: Mutex::new(None),
            lifecycle_mutation: Mutex::new(()),
            transfer_cancellations: StdMutex::new(HashSet::new()),
            startup_error: StdMutex::new(startup_error),
        };
        // Interrupted restore recovery is security-critical. Keep the initialized
        // native store available so the restricted recovery UI can retry safely.
        if crate::commands::backup::recover_interrupted_restore(&state).is_err() {
            if let Ok(mut startup_error) = state.startup_error.lock() {
                *startup_error = Some("ERR_STARTUP_RESTORE_RECOVERY".to_string());
            }
        }
        if let Ok(settings) = state.app_settings_store.load() {
            state
                .product_events
                .set_persistence(settings.local_event_persistence);
        }
        let mut event = ProductEvent::new(ProductEventName::AppLaunched);
        event.success = Some(state.startup_error.lock().unwrap().is_none());
        let _ = state.product_events.record(event);
        Ok(state)
    }

    /// Create a degraded-but-functional AppState when keyring is unavailable.
    /// The startup_error is set so the frontend can show a recovery UI.
    pub fn degraded(error_code: impl Into<String>) -> Self {
        let store: Arc<dyn SecretStore> = Arc::new(secrets::UnavailableSecretStore::new(
            "desktop is in restricted recovery mode",
        ));
        Self {
            registry: StorageRegistry::with_secret_store(None, store.clone()),
            settings_store: McpSettingsStore::with_secret_store(None, store.clone()),
            app_settings_store: AppSettingsStore::new(None),
            confirmations: ConfirmationManager::new(),
            sessions: SessionManager::new(),
            secret_store: store,
            pending_oauth: PendingOAuthStore::new(),
            workspaces: WorkspaceRegistry::new(&infimount_mcp::registry::default_config_dir()),
            product_events: ProductEventStore::new(None),
            operator_cache: OperatorCache::new(),
            http_runtime: Mutex::new(None),
            lifecycle_mutation: Mutex::new(()),
            transfer_cancellations: StdMutex::new(HashSet::new()),
            startup_error: StdMutex::new(Some(error_code.into())),
        }
    }

    pub fn startup_health(&self) -> StartupHealth {
        let error_code = self
            .startup_error
            .lock()
            .ok()
            .and_then(|value| value.clone());
        StartupHealth {
            operational: error_code.is_none(),
            recovery_available: matches!(self.secret_store.status(), SecretStoreStatus::Available),
            message: error_code.as_ref().map(|_| {
                "Infimount started in restricted recovery mode. Storage and MCP operations are disabled until the startup problem is resolved and the app is restarted.".to_string()
            }),
            error_code,
        }
    }

    pub fn require_operational(&self) -> McpResult<()> {
        if self.startup_health().operational {
            Ok(())
        } else {
            Err(err(
                McpErrorCode::ERR_SECRET_STORE_UNAVAILABLE,
                "desktop is in restricted recovery mode",
            ))
        }
    }

    /// Recover every durable configuration transaction before a
    /// new mutation. The caller must already hold the cross-process
    /// configuration transaction lock.
    pub(crate) fn recover_and_require_clean_configuration_locked(&self) -> McpResult<()> {
        self.registry.ensure_no_configuration_blocked()?;
        self.registry.recover_pending_imports_locked()?;
        infimount_mcp::registry::retry_pending_secret_cleanup_at(
            self.registry.path(),
            self.secret_store.as_ref(),
        )?;
        let storages = self.registry.load_all()?;
        let settings = self.settings_store.load()?;
        infimount_mcp::registry::recover_pending_secret_transactions(
            self.registry.path(),
            &storages,
            self.secret_store.as_ref(),
            settings.auth_token_ref.as_deref(),
        )?;
        infimount_mcp::registry::ensure_no_pending_secret_transaction(self.registry.path())?;
        crate::commands::backup::ensure_no_pending_restore_transaction(self)?;
        self.registry.ensure_no_configuration_blocked()
    }

    #[cfg(test)]
    pub(crate) fn new_for_test(
        config_dir: &std::path::Path,
        secret_store: Arc<dyn SecretStore>,
    ) -> Self {
        Self {
            registry: StorageRegistry::with_secret_store(
                Some(config_dir.join("storages.json")),
                secret_store.clone(),
            ),
            settings_store: McpSettingsStore::with_secret_store(
                Some(config_dir.join("mcp_settings.json")),
                secret_store.clone(),
            ),
            app_settings_store: AppSettingsStore::new(Some(config_dir.join("app_settings.json"))),
            confirmations: ConfirmationManager::new(),
            sessions: SessionManager::new(),
            secret_store,
            pending_oauth: PendingOAuthStore::new(),
            workspaces: WorkspaceRegistry::new(config_dir),
            product_events: ProductEventStore::new(Some(config_dir.join("events.jsonl"))),
            operator_cache: OperatorCache::new(),
            http_runtime: Mutex::new(None),
            lifecycle_mutation: Mutex::new(()),
            transfer_cancellations: StdMutex::new(HashSet::new()),
            startup_error: StdMutex::new(None),
        }
    }

    pub fn fs_context(&self) -> McpResult<FsToolsContext> {
        self.require_operational()?;
        let settings = self.settings_store.load()?;
        let auth_token = resolve_auth_token(&settings.auth_token_ref, self.secret_store.as_ref())?;
        Ok(FsToolsContext {
            registry: self.registry.clone(),
            sessions: self.sessions.clone(),
            allow_insecure: auth_token.is_none()
                && is_loopback_bind_address(&settings.bind_address),
            auth_token,
        })
    }

    pub fn list_storages(&self) -> McpResult<Vec<StorageRecord>> {
        self.require_operational()?;
        self.registry.load_all()
    }

    pub fn find_storage_by_id(&self, storage_id: &str) -> McpResult<StorageRecord> {
        self.require_operational()?;
        self.registry
            .load_all()?
            .into_iter()
            .find(|storage| storage.id == storage_id)
            .ok_or_else(|| {
                err_with_details(
                    McpErrorCode::ERR_STORAGE_NOT_FOUND,
                    format!("storage '{storage_id}' not found"),
                    json!({ "storage_id": storage_id }),
                )
            })
    }

    pub fn request_transfer_cancel(&self, job_id: &str) {
        if let Ok(mut cancellations) = self.transfer_cancellations.lock() {
            cancellations.insert(job_id.to_string());
        }
    }

    pub fn clear_transfer_cancel(&self, job_id: &str) {
        if let Ok(mut cancellations) = self.transfer_cancellations.lock() {
            cancellations.remove(job_id);
        }
    }

    pub fn is_transfer_cancelled(&self, job_id: &str) -> bool {
        self.transfer_cancellations
            .lock()
            .map(|cancellations| cancellations.contains(job_id))
            .unwrap_or(false)
    }

    pub fn operator_for_storage_id(&self, storage_id: &str) -> Result<Operator, CoreError> {
        self.operator_and_revision_for_storage_id(storage_id)
            .map(|(operator, _)| operator)
    }

    pub fn operator_and_revision_for_storage_id(
        &self,
        storage_id: &str,
    ) -> Result<(Operator, u64), CoreError> {
        let storage = self
            .find_storage_by_id(storage_id)
            .map_err(mcp_error_to_core_error)?;
        let revision = storage.revision;
        if let Some(operator) = self.operator_cache.get_for_storage(storage_id, revision) {
            return Ok((operator, revision));
        }
        let resolved = self.registry.resolve_storage(&storage).map_err(|e| {
            CoreError::Config(format!("failed to resolve storage secrets: {}", e.message))
        })?;
        let source = resolved_record_to_source(&resolved)?;
        let operator = infimount_core::runtime::get_or_create_operator(
            &self.operator_cache,
            &source,
            revision,
        )
        .map_err(|_| CoreError::Config("storage backend configuration failed".to_string()))?;
        Ok((operator, revision))
    }

    pub async fn apply_mcp_settings_with_auth(
        &self,
        settings: McpSettings,
        auth_mutation: AuthTokenMutation,
    ) -> McpResult<McpRuntimeStatus> {
        self.require_operational()?;
        let _lifecycle = self.lifecycle_mutation.lock().await;
        let _config_transaction = self.registry.acquire_configuration_transaction()?;
        self.recover_and_require_clean_configuration_locked()?;
        let existing = self.settings_store.load()?;
        let old_was_running = self.http_runtime.lock().await.is_some();
        let previous_ref = existing.auth_token_ref.clone();
        let previous_secret = match previous_ref.as_deref() {
            Some(account) => Some(
                self.secret_store
                    .get_json(account)
                    .map_err(|_| {
                        err(
                            McpErrorCode::ERR_SECRET_STORE_UNAVAILABLE,
                            "failed to access native secret storage",
                        )
                    })?
                    .ok_or_else(|| {
                        err(
                            McpErrorCode::ERR_SECRET_NOT_FOUND,
                            "configured HTTP auth token is missing",
                        )
                    })?,
            ),
            None => None,
        };
        let old_token = resolve_auth_token(&existing.auth_token_ref, self.secret_store.as_ref())?;

        let mut final_settings = settings;
        final_settings.auth_token_ref = previous_ref.clone();
        final_settings.auth_token = None;

        let mut transaction_id: Option<String> = None;
        let mut desired_ref = previous_ref.clone();
        let mut desired_secret: Option<Value> = None;
        let mut expected_token = old_token.clone();
        let mut secret_changed = false;

        match auth_mutation {
            AuthTokenMutation::Keep => {}
            AuthTokenMutation::Set { ref value } => {
                let token = value.trim();
                if token.is_empty() || token == "********" {
                    return Err(err(
                        McpErrorCode::ERR_INVALID_PATH,
                        "auth token must not be empty or masked",
                    ));
                }
                let id = uuid::Uuid::new_v4().to_string();
                desired_ref = Some(format!("{MCP_AUTH_TOKEN_ACCOUNT}/revision/{id}"));
                desired_secret = Some(json!({"token": token}));
                expected_token = Some(token.to_string());
                transaction_id = Some(id);
                secret_changed = true;
            }
            AuthTokenMutation::Clear => {
                desired_ref = None;
                desired_secret = None;
                // Desktop-managed HTTP authentication is keyring-managed
                // only. INFIMOUNT_AUTH_TOKEN is a headless-sidecar override.
                expected_token = None;
                secret_changed = previous_ref.is_some();
                if secret_changed {
                    transaction_id = Some(uuid::Uuid::new_v4().to_string());
                }
            }
            AuthTokenMutation::Rotate => {
                if previous_ref.is_none()
                    && std::env::var("INFIMOUNT_AUTH_TOKEN")
                        .map(|value| !value.trim().is_empty())
                        .unwrap_or(false)
                {
                    return Err(err(
                        McpErrorCode::ERR_SECRET_MIGRATION_FAILED,
                        "cannot rotate an environment-provided auth token",
                    ));
                }
                if previous_ref.is_none() {
                    return Err(err(
                        McpErrorCode::ERR_SECRET_MIGRATION_FAILED,
                        "cannot rotate: no managed auth token is configured",
                    ));
                }
                let managed_old_token = token_from_bundle(previous_secret.as_ref())?;
                if managed_old_token.is_none() || managed_old_token != old_token {
                    return Err(err(
                        McpErrorCode::ERR_SECRET_MIGRATION_FAILED,
                        "cannot rotate: the existing managed auth token is missing or invalid",
                    ));
                }
                let token = generate_auth_token();
                let id = uuid::Uuid::new_v4().to_string();
                desired_ref = Some(format!("{MCP_AUTH_TOKEN_ACCOUNT}/revision/{id}"));
                desired_secret = Some(json!({"token": token}));
                expected_token = desired_secret
                    .as_ref()
                    .and_then(|value| value.get("token"))
                    .and_then(Value::as_str)
                    .map(str::to_string);
                transaction_id = Some(id);
                secret_changed = true;
            }
        }

        final_settings.auth_token_ref = desired_ref.clone();

        if secret_changed {
            let transaction_id = transaction_id.as_deref().ok_or_else(|| {
                err(
                    McpErrorCode::ERR_SECRET_MIGRATION_FAILED,
                    "secret transaction id is missing",
                )
            })?;
            let obsolete_refs = previous_ref
                .iter()
                .filter(|account| desired_ref.as_deref() != Some(account.as_str()))
                .cloned()
                .collect::<Vec<_>>();
            let journal = infimount_mcp::registry::SecretTransactionJournal {
                version: infimount_mcp::registry::SECRET_TRANSACTION_JOURNAL_VERSION,
                transaction_id: transaction_id.to_string(),
                created_at: Utc::now().to_rfc3339(),
                state: infimount_mcp::registry::SecretTransactionState::Prepared,
                target: infimount_mcp::registry::SecretTransactionTarget::McpAuth,
                previous_ref: previous_ref.clone(),
                desired_ref: desired_ref.clone(),
                obsolete_refs,
            };
            infimount_mcp::registry::begin_secret_transaction(self.registry.path(), &journal)?;

            if let (Some(account), Some(secret)) = (desired_ref.as_deref(), desired_secret.as_ref())
            {
                if let Err(error) =
                    persist_secret_bundle(self.secret_store.as_ref(), account, Some(secret))
                {
                    let cleaned =
                        persist_secret_bundle(self.secret_store.as_ref(), account, None).is_ok();
                    if cleaned {
                        infimount_mcp::registry::abandon_secret_transaction_after_rollback(
                            self.registry.path(),
                            transaction_id,
                        )?;
                    }
                    return Err(error);
                }
                if let Err(error) = infimount_mcp::registry::advance_secret_transaction(
                    self.registry.path(),
                    transaction_id,
                    infimount_mcp::registry::SecretTransactionState::Prepared,
                    infimount_mcp::registry::SecretTransactionState::SecretWritten,
                ) {
                    self.recover_secret_transaction_locked()?;
                    return Err(error);
                }
            }

            if let Err(error) = self.settings_store.save_atomic(&final_settings) {
                let rollback_errors = self
                    .rollback_auth_reference_transaction_async(
                        &existing,
                        desired_ref.as_deref(),
                        Some(transaction_id),
                        old_was_running,
                        old_token.as_deref(),
                    )
                    .await;
                return Err(if rollback_errors.is_empty() {
                    error
                } else {
                    auth_rollback_error(&rollback_errors)
                });
            }
        } else {
            self.settings_store.save_atomic(&final_settings)?;
        }

        let persisted = match self.settings_store.load() {
            Ok(settings) => settings,
            Err(error) => {
                let rollback_errors = self
                    .rollback_auth_reference_transaction_async(
                        &existing,
                        desired_ref.as_deref(),
                        transaction_id.as_deref(),
                        old_was_running,
                        old_token.as_deref(),
                    )
                    .await;
                return Err(if rollback_errors.is_empty() {
                    error
                } else {
                    auth_rollback_error(&rollback_errors)
                });
            }
        };
        let persisted_json = serde_json::to_value(&persisted).unwrap_or_default();
        let expected_json = serde_json::to_value(&final_settings).unwrap_or_default();
        let readback_token =
            match resolve_auth_token(&persisted.auth_token_ref, self.secret_store.as_ref()) {
                Ok(token) => token,
                Err(error) => {
                    let rollback_errors = self
                        .rollback_auth_reference_transaction_async(
                            &existing,
                            desired_ref.as_deref(),
                            transaction_id.as_deref(),
                            old_was_running,
                            old_token.as_deref(),
                        )
                        .await;
                    return Err(if rollback_errors.is_empty() {
                        error
                    } else {
                        auth_rollback_error(&rollback_errors)
                    });
                }
            };
        if persisted_json != expected_json || readback_token != expected_token {
            let rollback_errors = self
                .rollback_auth_reference_transaction_async(
                    &existing,
                    desired_ref.as_deref(),
                    transaction_id.as_deref(),
                    old_was_running,
                    old_token.as_deref(),
                )
                .await;
            return Err(if rollback_errors.is_empty() {
                err(
                    McpErrorCode::ERR_SECRET_MIGRATION_FAILED,
                    "failed to verify persisted MCP authentication settings",
                )
            } else {
                auth_rollback_error(&rollback_errors)
            });
        }

        if let Err(error) = self
            .reconcile_runtime_inner(&final_settings, expected_token.as_deref())
            .await
        {
            let rollback_errors = self
                .rollback_auth_reference_transaction_async(
                    &existing,
                    desired_ref.as_deref(),
                    transaction_id.as_deref(),
                    old_was_running,
                    old_token.as_deref(),
                )
                .await;
            return Err(if rollback_errors.is_empty() {
                error
            } else {
                auth_rollback_error(&rollback_errors)
            });
        }

        let endpoint = self
            .http_runtime
            .lock()
            .await
            .as_ref()
            .map(|server| server.endpoint().to_string());
        if let (Some(endpoint), Some(token)) = (endpoint.as_deref(), expected_token.as_deref()) {
            let new_accepted = verify_auth_token_accepted(endpoint, token).await;
            let old_rejected = match old_token.as_deref() {
                Some(old) if old != token => verify_auth_token_rejected(endpoint, old).await,
                _ => true,
            };
            if !new_accepted || !old_rejected {
                let rollback_errors = self
                    .rollback_auth_reference_transaction_async(
                        &existing,
                        desired_ref.as_deref(),
                        transaction_id.as_deref(),
                        old_was_running,
                        old_token.as_deref(),
                    )
                    .await;
                return Err(if rollback_errors.is_empty() {
                    err(
                        McpErrorCode::ERR_SECRET_MIGRATION_FAILED,
                        "live MCP authentication verification failed",
                    )
                } else {
                    auth_rollback_error(&rollback_errors)
                });
            }
        }

        if let Some(transaction_id) = transaction_id.as_deref() {
            let expected_state = if desired_ref.is_some() {
                infimount_mcp::registry::SecretTransactionState::SecretWritten
            } else {
                infimount_mcp::registry::SecretTransactionState::Prepared
            };
            if infimount_mcp::registry::advance_secret_transaction(
                self.registry.path(),
                transaction_id,
                expected_state,
                infimount_mcp::registry::SecretTransactionState::ReferenceCommitted,
            )
            .is_err()
            {
                self.recover_secret_transaction_locked()?;
            } else {
                if let Some(previous) = previous_ref
                    .as_deref()
                    .filter(|previous| desired_ref.as_deref() != Some(*previous))
                {
                    if self.secret_store.delete(previous).is_err() {
                        return Err(err(
                            McpErrorCode::ERR_SECRET_STORE_UNAVAILABLE,
                            "previous MCP authentication cleanup is pending recovery",
                        ));
                    }
                }
                if infimount_mcp::registry::finish_secret_transaction(
                    self.registry.path(),
                    transaction_id,
                )
                .is_err()
                {
                    self.recover_secret_transaction_locked()?;
                }
            }
        }

        self.mcp_status().await
    }

    fn recover_secret_transaction_locked(&self) -> McpResult<()> {
        let storages = self.registry.load_all()?;
        let settings = self.settings_store.load()?;
        infimount_mcp::registry::recover_pending_secret_transactions(
            self.registry.path(),
            &storages,
            self.secret_store.as_ref(),
            settings.auth_token_ref.as_deref(),
        )
    }

    pub async fn start_http_server(&self) -> McpResult<McpRuntimeStatus> {
        self.require_operational()?;
        let _lifecycle = self.lifecycle_mutation.lock().await;
        let settings = self.settings_store.load()?;
        if settings.transport != McpTransport::Http {
            return Err(err_with_details(
                McpErrorCode::ERR_INTERNAL,
                "MCP transport is not set to HTTP",
                json!({ "transport": settings.transport }),
            ));
        }
        let auth_token = resolve_auth_token(&settings.auth_token_ref, self.secret_store.as_ref())?;
        self.reconcile_runtime_inner(&settings, auth_token.as_deref())
            .await?;
        let endpoint = self
            .http_runtime
            .lock()
            .await
            .as_ref()
            .map(|server| server.endpoint().to_string());
        self.status_with_endpoint(settings, endpoint)
    }

    pub async fn stop_http_server(&self) -> McpResult<McpRuntimeStatus> {
        let _lifecycle = self.lifecycle_mutation.lock().await;
        let settings = self.settings_store.load()?;
        self.stop_http_server_inner().await?;
        self.status_with_endpoint(settings, None)
    }

    pub async fn ensure_runtime_from_settings(&self) -> McpResult<()> {
        self.require_operational()?;
        let _lifecycle = self.lifecycle_mutation.lock().await;
        self.ensure_runtime_from_settings_locked().await
    }

    pub(crate) async fn ensure_runtime_from_settings_locked(&self) -> McpResult<()> {
        infimount_mcp::opendal_adapter::clear_operator_cache();
        let settings = self.settings_store.load()?;
        let auth_token = resolve_auth_token(&settings.auth_token_ref, self.secret_store.as_ref())?;
        self.reconcile_runtime_inner(&settings, auth_token.as_deref())
            .await
    }

    pub(crate) async fn stop_http_server_locked(&self) -> McpResult<()> {
        self.stop_http_server_inner().await
    }

    pub async fn is_http_running(&self) -> bool {
        self.http_runtime.lock().await.is_some()
    }

    pub async fn mcp_status(&self) -> McpResult<McpRuntimeStatus> {
        self.require_operational()?;
        let settings = self.settings_store.load()?;
        let endpoint = self
            .http_runtime
            .lock()
            .await
            .as_ref()
            .map(|server| server.endpoint().to_string());
        self.status_with_endpoint(settings, endpoint)
    }

    pub async fn client_snippets(&self) -> McpResult<McpClientSnippets> {
        let status = self.mcp_status().await?;
        let http_endpoint = status.endpoint.clone().unwrap_or_else(|| {
            suggested_http_endpoint(&status.settings.bind_address, status.settings.port)
        });

        let stdio_command = crate::activation_probe::verified_sidecar_path()
            .map_err(|code| {
                infimount_mcp::errors::err(
                    infimount_mcp::errors::McpErrorCode::ERR_INTERNAL,
                    format!("bundled MCP sidecar is unavailable ({code})"),
                )
            })?
            .to_string_lossy()
            .to_string();

        Ok(McpClientSnippets {
            stdio: serde_json::to_string_pretty(&json!({
                "mcpServers": {
                    "infimount": {
                        "command": stdio_command,
                        "args": ["serve", "--transport", "stdio"]
                    }
                }
            }))
            .unwrap_or_default(),
            http: serde_json::to_string_pretty(&http_client_snippet(
                &http_endpoint,
                status.auth_token_configured,
            ))
            .unwrap_or_default(),
        })
    }

    async fn stop_http_server_inner(&self) -> McpResult<()> {
        let existing = {
            let mut guard = self.http_runtime.lock().await;
            guard.take()
        };

        if let Some(server) = existing {
            server.stop().await.map_err(map_runtime_io_error)?;
            self.sessions.clear().await;
        }
        Ok(())
    }

    async fn reconcile_runtime_inner(
        &self,
        settings: &McpSettings,
        auth_token: Option<&str>,
    ) -> McpResult<()> {
        self.stop_http_server_inner().await?;
        if !settings.enabled || settings.transport != McpTransport::Http {
            return Ok(());
        }
        let allow_insecure =
            auth_token.is_none() && is_loopback_bind_address(&settings.bind_address);
        let mut runtime_settings = settings.clone();
        runtime_settings.auth_token = auth_token.map(str::to_string);
        let server = start_http_server_from_settings(
            self.registry.clone(),
            &runtime_settings,
            allow_insecure,
            self.confirmations.clone(),
            self.sessions.clone(),
        )
        .await
        .map_err(map_runtime_io_error)?;
        *self.http_runtime.lock().await = Some(server);
        Ok(())
    }

    async fn rollback_auth_reference_transaction_async(
        &self,
        settings: &McpSettings,
        desired_ref: Option<&str>,
        transaction_id: Option<&str>,
        runtime_was_running: bool,
        token: Option<&str>,
    ) -> Vec<&'static str> {
        let mut failures = Vec::new();
        if self.stop_http_server_inner().await.is_err() {
            failures.push("stop_runtime");
        }

        let settings_restored = self.settings_store.save_atomic(settings).is_ok();
        if !settings_restored {
            failures.push("restore_settings");
        }

        if settings_restored {
            if let Some(desired_ref) =
                desired_ref.filter(|desired| settings.auth_token_ref.as_deref() != Some(*desired))
            {
                if self.secret_store.delete(desired_ref).is_err() {
                    failures.push("remove_staged_secret");
                }
            }
        }

        if runtime_was_running && failures.is_empty() {
            if self.reconcile_runtime_inner(settings, token).await.is_err() {
                failures.push("restore_runtime");
            } else if let Some(token) = token {
                let endpoint = self
                    .http_runtime
                    .lock()
                    .await
                    .as_ref()
                    .map(|server| server.endpoint().to_string());
                let restored_token_works = match endpoint {
                    Some(endpoint) => verify_auth_token_accepted(&endpoint, token).await,
                    None => false,
                };
                if !restored_token_works {
                    failures.push("verify_restored_runtime");
                }
            }
        }

        if failures.is_empty() {
            if let Some(transaction_id) = transaction_id {
                if infimount_mcp::registry::abandon_secret_transaction_after_rollback(
                    self.registry.path(),
                    transaction_id,
                )
                .is_err()
                {
                    failures.push("remove_transaction_journal");
                }
            }
        } else {
            let _ = self.stop_http_server_inner().await;
        }

        failures
    }

    fn status_with_endpoint(
        &self,
        settings: McpSettings,
        endpoint: Option<String>,
    ) -> McpResult<McpRuntimeStatus> {
        let endpoint_display = if let Some(endpoint) = &endpoint {
            endpoint.clone()
        } else if settings.transport == McpTransport::Http {
            if settings.port == 0 {
                format!(
                    "Starts on {}:<auto>{}",
                    settings.bind_address, HTTP_ENDPOINT_PATH
                )
            } else {
                suggested_http_endpoint(&settings.bind_address, settings.port)
            }
        } else {
            "stdio transport".to_string()
        };

        let auth_token_configured = settings.auth_token_ref.is_some();

        let public_settings = McpRuntimeSettings {
            enabled: settings.enabled,
            transport: settings.transport,
            bind_address: settings.bind_address,
            port: settings.port,
            enabled_tools: settings.enabled_tools,
            security_baseline_version: settings.security_baseline_version,
            auth_token_configured,
        };
        Ok(McpRuntimeStatus {
            settings: public_settings,
            running_http: endpoint.is_some(),
            endpoint,
            endpoint_display,
            auth_token_configured,
        })
    }
}

fn http_client_snippet(endpoint: &str, auth_token_configured: bool) -> Value {
    let mut server = Map::new();
    server.insert("url".to_string(), Value::String(endpoint.to_string()));
    if auth_token_configured {
        server.insert(
            "headers".to_string(),
            json!({ "Authorization": "Bearer ${INFIMOUNT_AUTH_TOKEN}" }),
        );
    }

    json!({
        "mcpServers": {
            "infimount": Value::Object(server)
        }
    })
}

fn persist_secret_bundle(
    secret_store: &dyn SecretStore,
    account: &str,
    value: Option<&Value>,
) -> McpResult<()> {
    let result = match value {
        Some(value) => secret_store.put_json(account, value),
        None => secret_store.delete(account),
    };
    result.map_err(|_| {
        err(
            McpErrorCode::ERR_SECRET_STORE_UNAVAILABLE,
            "failed to update native MCP authentication state",
        )
    })
}

fn token_from_bundle(bundle: Option<&Value>) -> McpResult<Option<String>> {
    let Some(bundle) = bundle else {
        return Ok(None);
    };
    let token = bundle
        .as_object()
        .and_then(|object| object.get("token"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|token| !token.is_empty())
        .ok_or_else(|| {
            err(
                McpErrorCode::ERR_SECRET_MIGRATION_FAILED,
                "stored MCP authentication state is malformed",
            )
        })?;
    Ok(Some(token.to_string()))
}

fn auth_rollback_error(stages: &[&str]) -> McpError {
    err_with_details(
        McpErrorCode::ERR_SECRET_MIGRATION_FAILED,
        "MCP authentication rollback failed; the HTTP runtime was stopped",
        json!({ "rollbackFailedStages": stages }),
    )
}

fn generate_auth_token() -> String {
    let bytes: [u8; 32] = rand::rng().random();
    let engine = base64::engine::general_purpose::URL_SAFE_NO_PAD;
    engine.encode(bytes)
}

/// Make a test request to the MCP HTTP endpoint and return true if the
/// response indicates the given auth token is accepted.
async fn verify_auth_token_accepted(endpoint: &str, token: &str) -> bool {
    let Ok(client) = auth_verification_client() else {
        return false;
    };
    let body = r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18","capabilities":{},"clientInfo":{"name":"infimount-verify","version":"0.0.0"}}}"#;
    let response = client
        .post(endpoint)
        .header("Content-Type", "application/json")
        .header("Accept", "application/json, text/event-stream")
        .header("Authorization", format!("Bearer {token}"))
        .body(body)
        .send()
        .await;
    let Ok(response) = response else {
        return false;
    };
    if !response.status().is_success() && response.status().as_u16() != 202 {
        return false;
    }
    response
        .text()
        .await
        .ok()
        .is_some_and(|body| body.contains("\"id\":1") && body.contains("\"result\""))
}

/// Make a test request with the given token and verify it is rejected.
async fn verify_auth_token_rejected(endpoint: &str, token: &str) -> bool {
    let Ok(client) = auth_verification_client() else {
        return false;
    };
    let body = r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18","capabilities":{},"clientInfo":{"name":"infimount-verify","version":"0.0.0"}}}"#;
    client
        .post(endpoint)
        .header("Content-Type", "application/json")
        .header("Accept", "application/json, text/event-stream")
        .header("Authorization", format!("Bearer {token}"))
        .body(body)
        .send()
        .await
        .is_ok_and(|response| response.status().as_u16() == 401)
}

fn auth_verification_client() -> Result<reqwest::Client, reqwest::Error> {
    reqwest::Client::builder()
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(std::time::Duration::from_secs(3))
        .build()
}

fn suggested_http_endpoint(bind_address: &str, configured_port: u16) -> String {
    let port = if configured_port == 0 {
        "<auto>".to_string()
    } else {
        configured_port.to_string()
    };
    format!("http://{bind_address}:{port}{HTTP_ENDPOINT_PATH}")
}

fn migrate_legacy_sources_if_needed(registry: &StorageRegistry) -> McpResult<()> {
    let registry_exists = registry.path().exists();
    let legacy_sources = config::load_sources().map_err(|_| {
        err_with_details(
            McpErrorCode::ERR_INTERNAL,
            "failed to load legacy storage config",
            json!({}),
        )
    })?;

    if legacy_sources.is_empty() {
        return Ok(());
    }
    let legacy_records = legacy_sources
        .into_iter()
        .map(legacy_source_to_storage)
        .collect::<Vec<_>>();
    let storages = if registry_exists {
        let mut existing = registry.load_all()?;
        for legacy in legacy_records {
            if existing.iter().any(|record| record.id == legacy.id) {
                continue;
            }
            if existing.iter().any(|record| record.name == legacy.name) {
                return Err(err(
                    McpErrorCode::ERR_SECRET_MIGRATION_FAILED,
                    "legacy storage name conflicts with an existing record; migration was not changed",
                ));
            }
            existing.push(legacy);
        }
        existing
    } else {
        legacy_records
    };
    registry.save_legacy_records_secure(storages)?;
    config::remove_legacy_config().map_err(|_| {
        err(
            McpErrorCode::ERR_SECRET_MIGRATION_FAILED,
            "storage migration succeeded but legacy config cleanup failed",
        )
    })?;
    Ok(())
}

fn legacy_source_to_storage(source: Source) -> StorageRecord {
    let backend = match source.kind {
        SourceKind::Local => "local",
        SourceKind::S3 => "s3",
        SourceKind::WebDav => "webdav",
        SourceKind::AzureBlob => "azure_blob",
        SourceKind::Gcs => "gcs",
        SourceKind::B2 => "b2",
        SourceKind::Oss => "oss",
        SourceKind::Cos => "cos",
        SourceKind::Obs => "obs",
        SourceKind::Sftp => "sftp",
        SourceKind::Ftp => "ftp",
        SourceKind::Gdrive => "gdrive",
        SourceKind::Onedrive => "onedrive",
    }
    .to_string();

    let mut config_map = Map::new();
    if let Some(map) = source.config.as_object() {
        for (key, value) in map {
            config_map.insert(key.clone(), value.clone());
        }
    }

    if matches!(backend.as_str(), "local" | "fs") && !source.root.trim().is_empty() {
        config_map
            .entry("root".to_string())
            .or_insert(Value::String(source.root));
    }

    let mut storage = StorageRecord::new(source.name, backend, Value::Object(config_map));
    storage.mcp_exposed = false;
    storage
}

fn resolved_record_to_source(
    resolved: &infimount_mcp::registry::ResolvedStorageRecord,
) -> Result<Source, CoreError> {
    use std::str::FromStr;
    let kind = SourceKind::from_str(&resolved.record.backend)
        .map_err(|_| CoreError::Config("unsupported storage backend".to_string()))?;

    Ok(Source {
        id: resolved.record.id.clone(),
        name: resolved.record.name.clone(),
        kind,
        root: String::new(),
        config: resolved.resolved_config.clone(),
    })
}

pub fn mcp_error_to_core_error(err: McpError) -> CoreError {
    match err.code {
        McpErrorCode::ERR_STORAGE_NOT_FOUND | McpErrorCode::ERR_PATH_NOT_FOUND => CoreError::Io(
            std::io::Error::new(std::io::ErrorKind::NotFound, err.message),
        ),
        McpErrorCode::ERR_PERMISSION_DENIED | McpErrorCode::ERR_STORAGE_READ_ONLY => CoreError::Io(
            std::io::Error::new(std::io::ErrorKind::PermissionDenied, err.message),
        ),
        McpErrorCode::ERR_ALREADY_EXISTS | McpErrorCode::ERR_STORAGE_NAME_CONFLICT => {
            CoreError::Io(std::io::Error::new(
                std::io::ErrorKind::AlreadyExists,
                err.message,
            ))
        }
        _ => CoreError::Config(err.message),
    }
}

fn map_runtime_io_error(_error: std::io::Error) -> McpError {
    err_with_details(
        McpErrorCode::ERR_INTERNAL,
        "failed to manage MCP HTTP runtime",
        json!({ "kind": "Io", "temporary": false }),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn degraded_state_uses_unavailable_store_and_sanitized_health() {
        let state = AppState::degraded("ERR_STARTUP_TEST");
        let health = state.startup_health();
        assert!(!health.operational);
        assert_eq!(health.error_code.as_deref(), Some("ERR_STARTUP_TEST"));
        assert!(!health.message.unwrap().contains("keyring"));
        assert!(state.require_operational().is_err());
        assert!(matches!(
            state.secret_store.status(),
            SecretStoreStatus::Unavailable { .. }
        ));
        assert!(state.secret_store.get_json("storage/private").is_err());
    }

    #[test]
    fn http_snippet_never_contains_stored_token() {
        let snippet = http_client_snippet("http://127.0.0.1:7331/mcp", true);
        let serialized = serde_json::to_string(&snippet).unwrap();
        assert!(serialized.contains("INFIMOUNT_AUTH_TOKEN"));
        assert!(!serialized.contains("seeded-http-token"));
    }

    #[test]
    fn pending_oauth_sessions_are_single_use_and_expire() {
        let store = PendingOAuthStore::new();
        store.insert(PendingOAuthSession {
            id: "active".to_string(),
            provider: "gdrive".to_string(),
            secret_config: json!({ "accessToken": "seeded-token" }),
            public_config: json!({ "rootPath": "/" }),
            expires_at: Utc::now() + chrono::Duration::minutes(10),
            consumed: AtomicBool::new(false),
        });
        let PendingOAuthClaim::Session(session) = store.claim("active") else {
            panic!("active session should be claimable");
        };
        assert!(matches!(store.claim("active"), PendingOAuthClaim::InUse));
        store.complete(session);
        assert!(store.was_consumed("active"));
        assert!(matches!(
            store.claim("active"),
            PendingOAuthClaim::AlreadyUsed
        ));

        store.insert(PendingOAuthSession {
            id: "expired".to_string(),
            provider: "onedrive".to_string(),
            secret_config: json!({ "refreshToken": "seeded-token" }),
            public_config: json!({}),
            expires_at: Utc::now() - chrono::Duration::seconds(1),
            consumed: AtomicBool::new(false),
        });
        assert!(matches!(store.claim("expired"), PendingOAuthClaim::Expired));
        assert!(matches!(
            store.claim("missing"),
            PendingOAuthClaim::NotFound
        ));
    }

    #[test]
    fn storage_record_conversion_rejects_unsupported_backend() {
        let storage = StorageRecord::new(
            "Unsupported".to_string(),
            "mystery".to_string(),
            serde_json::json!({}),
        );

        let resolved = infimount_mcp::registry::ResolvedStorageRecord {
            record: storage,
            resolved_config: serde_json::json!({}),
        };
        let err = resolved_record_to_source(&resolved).unwrap_err();
        assert!(err.to_string().contains("unsupported storage backend"));
    }

    #[test]
    fn legacy_source_migration_defaults_to_not_mcp_exposed() {
        let source = Source {
            id: "legacy-local".to_string(),
            name: "Legacy Local".to_string(),
            kind: SourceKind::Local,
            root: "/tmp".to_string(),
            config: serde_json::json!({}),
        };

        let storage = legacy_source_to_storage(source);
        assert_eq!(storage.backend, "local");
        assert!(!storage.mcp_exposed);
    }

    #[test]
    fn generate_auth_token_produces_urlsafe_base64() {
        let token = generate_auth_token();
        assert_eq!(
            token.len(),
            43,
            "32 bytes = 43 chars in URL-safe base64 without padding"
        );
        assert!(
            token
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_'),
            "token should be URL-safe base64: {token}"
        );
    }

    #[test]
    fn auth_token_mutation_rotate_serde_roundtrip() {
        let rotate: AuthTokenMutation = serde_json::from_str(r#"{"action":"rotate"}"#).unwrap();
        assert!(matches!(rotate, AuthTokenMutation::Rotate));

        let set: AuthTokenMutation =
            serde_json::from_str(r#"{"action":"set","value":"my-token"}"#).unwrap();
        assert!(matches!(set, AuthTokenMutation::Set { .. }));
        if let AuthTokenMutation::Set { value } = set {
            assert_eq!(value, "my-token");
        }

        let keep: AuthTokenMutation = serde_json::from_str(r#"{"action":"keep"}"#).unwrap();
        assert!(matches!(keep, AuthTokenMutation::Keep));

        let clear: AuthTokenMutation = serde_json::from_str(r#"{"action":"clear"}"#).unwrap();
        assert!(matches!(clear, AuthTokenMutation::Clear));
    }

    fn test_app_state(dir: &tempfile::TempDir, secret_store: Arc<dyn SecretStore>) -> AppState {
        AppState {
            registry: StorageRegistry::with_secret_store(
                Some(dir.path().join("storages.json")),
                secret_store.clone(),
            ),
            settings_store: McpSettingsStore::with_secret_store(
                Some(dir.path().join("mcp_settings.json")),
                secret_store.clone(),
            ),
            app_settings_store: AppSettingsStore::new(Some(dir.path().join("app_settings.json"))),
            confirmations: ConfirmationManager::new(),
            sessions: SessionManager::new(),
            secret_store,
            pending_oauth: PendingOAuthStore::new(),
            workspaces: WorkspaceRegistry::new(dir.path()),
            product_events: ProductEventStore::new(Some(dir.path().join("events.jsonl"))),
            operator_cache: OperatorCache::new(),
            http_runtime: Mutex::new(None),
            lifecycle_mutation: Mutex::new(()),
            transfer_cancellations: StdMutex::new(HashSet::new()),
            startup_error: StdMutex::new(None),
        }
    }

    #[test]
    fn desktop_operator_cache_resolves_secrets_only_on_revision_miss() {
        let dir = tempfile::tempdir().unwrap();
        let secret_store = Arc::new(infimount_core::secrets::MemorySecretStore::new());
        let state = test_app_state(&dir, secret_store.clone());
        let mut record = StorageRecord::new(
            "Cached".to_string(),
            "s3".to_string(),
            json!({ "bucket": "example", "region": "us-east-1" }),
        );
        let account = format!("storage/{}", record.id);
        secret_store
            .put_json(
                &account,
                &json!({ "accessKeyId": "id", "secretAccessKey": "secret" }),
            )
            .unwrap();
        record.secret_ref = Some(account.clone());
        state.registry.save_all_atomic(&[record.clone()]).unwrap();

        state
            .operator_for_storage_id(&record.id)
            .expect("initial operator build");
        secret_store.delete(&account).unwrap();
        state
            .operator_for_storage_id(&record.id)
            .expect("cache hit must not resolve secrets again");

        record.revision += 1;
        state.registry.save_all_atomic(&[record.clone()]).unwrap();
        assert!(state.operator_for_storage_id(&record.id).is_err());
    }

    #[tokio::test]
    async fn auth_token_verify_helpers_use_managed_endpoint_and_mcp_headers() {
        use infimount_mcp::runtime::start_http_server;
        use tempfile::tempdir;

        let dir = tempdir().unwrap();
        let registry = StorageRegistry::with_secret_store(
            Some(dir.path().join("storages.json")),
            Arc::new(infimount_core::secrets::MemorySecretStore::new()),
        );
        let token = generate_auth_token();
        let old_token = generate_auth_token();
        let server = start_http_server(
            registry,
            "127.0.0.1",
            0,
            infimount_mcp::server::all_tool_names(),
            false,
            Some(token.clone()),
            ConfirmationManager::new(),
            SessionManager::new(),
        )
        .await
        .expect("start authenticated MCP server");

        assert!(verify_auth_token_accepted(server.endpoint(), &token).await);
        assert!(verify_auth_token_rejected(server.endpoint(), &old_token).await);

        server.stop().await.expect("stop MCP server");
    }

    #[tokio::test]
    async fn rotate_and_set_replace_live_http_credentials() {
        use infimount_core::secrets::MemorySecretStore;
        use tempfile::tempdir;

        let dir = tempdir().unwrap();
        let secret_store: Arc<dyn SecretStore> = Arc::new(MemorySecretStore::new());
        secret_store
            .put_json(MCP_AUTH_TOKEN_ACCOUNT, &json!({"token": "old-token"}))
            .unwrap();
        let state = test_app_state(&dir, secret_store.clone());
        let settings = McpSettings {
            enabled: true,
            transport: McpTransport::Http,
            bind_address: "127.0.0.1".to_string(),
            port: 0,
            auth_token_ref: Some(MCP_AUTH_TOKEN_ACCOUNT.to_string()),
            ..McpSettings::default()
        };
        state.settings_store.save_atomic(&settings).unwrap();
        state.ensure_runtime_from_settings().await.unwrap();

        let rotated = state
            .apply_mcp_settings_with_auth(settings.clone(), AuthTokenMutation::Rotate)
            .await
            .expect("rotate live token");
        let endpoint = rotated.endpoint.expect("running endpoint");
        let rotated_settings = state.settings_store.load().unwrap();
        let rotated_ref = rotated_settings.auth_token_ref.clone().unwrap();
        assert!(rotated_ref.starts_with("mcp/http-auth/revision/"));
        let rotated_token = resolve_auth_token(&Some(rotated_ref), secret_store.as_ref())
            .unwrap()
            .unwrap();
        assert_ne!(rotated_token, "old-token");
        assert!(verify_auth_token_accepted(&endpoint, &rotated_token).await);
        assert!(verify_auth_token_rejected(&endpoint, "old-token").await);

        let replaced = state
            .apply_mcp_settings_with_auth(
                settings,
                AuthTokenMutation::Set {
                    value: "replacement-token".to_string(),
                },
            )
            .await
            .expect("replace live token");
        let endpoint = replaced.endpoint.expect("running endpoint");
        assert!(verify_auth_token_accepted(&endpoint, "replacement-token").await);
        assert!(verify_auth_token_rejected(&endpoint, &rotated_token).await);

        state.stop_http_server().await.unwrap();
    }

    #[tokio::test]
    async fn concurrent_auth_mutations_are_serialized() {
        use infimount_core::secrets::MemorySecretStore;
        use tempfile::tempdir;

        let dir = tempdir().unwrap();
        let secret_store: Arc<dyn SecretStore> = Arc::new(MemorySecretStore::new());
        secret_store
            .put_json(MCP_AUTH_TOKEN_ACCOUNT, &json!({"token": "initial"}))
            .unwrap();
        let state = test_app_state(&dir, secret_store.clone());
        let settings = McpSettings {
            enabled: true,
            transport: McpTransport::Http,
            bind_address: "127.0.0.1".to_string(),
            port: 0,
            auth_token_ref: Some(MCP_AUTH_TOKEN_ACCOUNT.to_string()),
            ..McpSettings::default()
        };
        state.settings_store.save_atomic(&settings).unwrap();
        state.ensure_runtime_from_settings().await.unwrap();

        let first = state.apply_mcp_settings_with_auth(
            settings.clone(),
            AuthTokenMutation::Set {
                value: "first-token".to_string(),
            },
        );
        let second = state.apply_mcp_settings_with_auth(
            settings,
            AuthTokenMutation::Set {
                value: "second-token".to_string(),
            },
        );
        let (first_result, second_result) = tokio::join!(first, second);
        assert!(first_result.is_ok());
        assert!(second_result.is_ok());

        let final_settings = state.settings_store.load().unwrap();
        let final_ref = final_settings.auth_token_ref.clone().unwrap();
        assert!(final_ref.starts_with("mcp/http-auth/revision/"));
        let final_token = resolve_auth_token(&Some(final_ref), secret_store.as_ref())
            .unwrap()
            .unwrap();
        let endpoint = state.mcp_status().await.unwrap().endpoint.unwrap();
        assert!(verify_auth_token_accepted(&endpoint, &final_token).await);
        assert!(final_token == "first-token" || final_token == "second-token");
        state.stop_http_server().await.unwrap();
    }

    #[test]
    fn token_bundle_validation_rejects_missing_or_empty_tokens() {
        assert_eq!(
            token_from_bundle(Some(&json!({"token": "valid"}))).unwrap(),
            Some("valid".to_string())
        );
        assert!(token_from_bundle(Some(&json!({"token": ""}))).is_err());
        assert!(token_from_bundle(Some(&json!({"wrong": "field"}))).is_err());
        assert_eq!(token_from_bundle(None).unwrap(), None);
    }

    /// Serializes INFIMOUNT_CONFIG mutations: the legacy config path is
    /// process-global and other tests may resolve it concurrently.
    fn legacy_config_env_scope() -> std::sync::MutexGuard<'static, ()> {
        static LOCK: std::sync::OnceLock<StdMutex<()>> = std::sync::OnceLock::new();
        LOCK.get_or_init(|| StdMutex::new(()))
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    fn write_legacy_config(path: &std::path::Path) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        // Exact v0.7.1 fixture shape previously exercised by the packaged
        // Linux artifact smoke.
        std::fs::write(
            path,
            r#"[{
  "id": "legacy-local",
  "name": "Linux Artifact Smoke Home",
  "kind": "local",
  "root": "/tmp",
  "config": {}
}]"#,
        )
        .unwrap();
    }

    #[test]
    fn legacy_config_migration_creates_registry_and_removes_legacy_file() {
        let _env = legacy_config_env_scope();
        let home = tempfile::tempdir().unwrap();
        let legacy_path = home.path().join("config.json");
        write_legacy_config(&legacy_path);
        // SAFETY (single-threaded test body): guarded by legacy_config_env_scope.
        std::env::set_var("INFIMOUNT_CONFIG", &legacy_path);

        let registry_dir = tempfile::tempdir().unwrap();
        let registry = StorageRegistry::with_secret_store(
            Some(registry_dir.path().join("storages.json")),
            std::sync::Arc::new(secrets::MemorySecretStore::new()),
        );
        assert!(!registry.path().exists());

        migrate_legacy_sources_if_needed(&registry).unwrap();

        let migrated = registry.load_all().unwrap();
        assert_eq!(migrated.len(), 1);
        assert_eq!(migrated[0].name, "Linux Artifact Smoke Home");
        assert_eq!(migrated[0].backend, "local");
        assert_eq!(
            migrated[0].config.get("root").and_then(Value::as_str),
            Some("/tmp")
        );
        assert!(!legacy_path.exists());
        std::env::remove_var("INFIMOUNT_CONFIG");
    }

    #[test]
    fn legacy_migration_name_conflict_fails_closed_and_keeps_legacy_file() {
        let _env = legacy_config_env_scope();
        let home = tempfile::tempdir().unwrap();
        let legacy_path = home.path().join("config.json");
        write_legacy_config(&legacy_path);
        std::env::set_var("INFIMOUNT_CONFIG", &legacy_path);

        let registry_dir = tempfile::tempdir().unwrap();
        let registry = StorageRegistry::with_secret_store(
            Some(registry_dir.path().join("storages.json")),
            std::sync::Arc::new(secrets::MemorySecretStore::new()),
        );
        registry
            .save_all_atomic(&[StorageRecord::new(
                "Linux Artifact Smoke Home".to_string(),
                "local".to_string(),
                json!({ "root": "/elsewhere" }),
            )])
            .unwrap();

        let error = migrate_legacy_sources_if_needed(&registry).unwrap_err();
        assert_eq!(error.code, McpErrorCode::ERR_SECRET_MIGRATION_FAILED);
        // Fail-closed: the legacy source of truth is preserved for a retry.
        assert!(legacy_path.exists());
        std::env::remove_var("INFIMOUNT_CONFIG");
    }

    #[test]
    fn legacy_migration_is_noop_without_a_legacy_file() {
        let _env = legacy_config_env_scope();
        let home = tempfile::tempdir().unwrap();
        let legacy_path = home.path().join("config.json");
        std::env::set_var("INFIMOUNT_CONFIG", &legacy_path);

        let registry_dir = tempfile::tempdir().unwrap();
        let registry = StorageRegistry::with_secret_store(
            Some(registry_dir.path().join("storages.json")),
            std::sync::Arc::new(secrets::MemorySecretStore::new()),
        );

        migrate_legacy_sources_if_needed(&registry).unwrap();

        assert!(!registry.path().exists());
        assert!(!legacy_path.exists());
        std::env::remove_var("INFIMOUNT_CONFIG");
    }
}
