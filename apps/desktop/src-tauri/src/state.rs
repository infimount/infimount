use chrono::{DateTime, Utc};
use infimount_core::{config, secrets, CoreError, SecretStore, Source, SourceKind};
use infimount_mcp::confirmation::ConfirmationManager;
use infimount_mcp::errors::{err, err_with_details, McpError, McpErrorCode, McpResult};
use infimount_mcp::registry::{StorageRecord, StorageRegistry};
use infimount_mcp::runtime::{
    start_http_server_from_settings, McpHttpServerHandle, HTTP_ENDPOINT_PATH,
};
use infimount_mcp::session::SessionManager;
use infimount_mcp::settings::{
    resolve_auth_token, McpSettings, McpSettingsStore, McpTransport, MCP_AUTH_TOKEN_ACCOUNT,
};
use infimount_mcp::tools_fs::FsToolsContext;
use opendal::Operator;
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex as StdMutex};
use tokio::sync::Mutex;

use crate::app_settings::AppSettingsStore;

pub struct AppState {
    pub registry: StorageRegistry,
    pub settings_store: McpSettingsStore,
    pub app_settings_store: AppSettingsStore,
    pub confirmations: ConfirmationManager,
    pub sessions: SessionManager,
    pub secret_store: Arc<dyn SecretStore>,
    pub pending_oauth: PendingOAuthStore,
    http_runtime: Mutex<Option<McpHttpServerHandle>>,
    transfer_cancellations: StdMutex<HashSet<String>>,
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
}

impl AppState {
    pub fn new() -> McpResult<Self> {
        let secret_store = Arc::new(secrets::NativeSecretStore::new());
        infimount_mcp::registry::retry_pending_secret_cleanup(secret_store.as_ref())?;
        let registry = StorageRegistry::with_secret_store(None, secret_store.clone());
        migrate_legacy_sources_if_needed(&registry)?;
        registry.load_all()?;
        let settings_store = McpSettingsStore::with_secret_store(None, secret_store.clone());
        settings_store.load()?;

        Ok(Self {
            registry,
            settings_store,
            app_settings_store: AppSettingsStore::new(None),
            confirmations: ConfirmationManager::new(),
            sessions: SessionManager::new(),
            secret_store,
            pending_oauth: PendingOAuthStore::new(),
            http_runtime: Mutex::new(None),
            transfer_cancellations: StdMutex::new(HashSet::new()),
        })
    }

    pub fn fs_context(&self) -> McpResult<FsToolsContext> {
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
        self.registry.load_all()
    }

    pub fn find_storage_by_id(&self, storage_id: &str) -> McpResult<StorageRecord> {
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
        let storage = self
            .find_storage_by_id(storage_id)
            .map_err(mcp_error_to_core_error)?;
        let resolved = self.registry.resolve_storage(&storage).map_err(|e| {
            CoreError::Config(format!("failed to resolve storage secrets: {}", e.message))
        })?;
        let source = resolved_record_to_source(&resolved)?;
        infimount_core::registry::build_operator(&source)
            .map_err(|_| CoreError::Config("storage backend configuration failed".to_string()))
    }

    pub async fn apply_mcp_settings_with_auth(
        &self,
        settings: McpSettings,
        auth_mutation: AuthTokenMutation,
    ) -> McpResult<McpRuntimeStatus> {
        let existing = self.settings_store.load()?;
        let mut final_settings = settings;
        final_settings.auth_token_ref = existing.auth_token_ref.clone();
        final_settings.auth_token = None;
        let account = MCP_AUTH_TOKEN_ACCOUNT;
        let mut previous = None;
        let mut changed_secret = false;
        match auth_mutation {
            AuthTokenMutation::Set { value } => {
                previous = self.secret_store.get_json(account).map_err(|_| {
                    err(
                        McpErrorCode::ERR_SECRET_STORE_UNAVAILABLE,
                        "failed to access native secret storage",
                    )
                })?;
                let token = value.trim();
                if token.is_empty() || token == "********" {
                    return Err(err(
                        McpErrorCode::ERR_INVALID_PATH,
                        "auth token must not be empty or masked",
                    ));
                }
                if self
                    .secret_store
                    .put_json(account, &json!({"token": token}))
                    .is_err()
                {
                    restore_secret_bundle(self.secret_store.as_ref(), account, previous.as_ref())?;
                    return Err(err(
                        McpErrorCode::ERR_SECRET_STORE_UNAVAILABLE,
                        "failed to store auth token",
                    ));
                }
                final_settings.auth_token_ref = Some(account.to_string());
                changed_secret = true;
            }
            AuthTokenMutation::Clear => {
                if existing.auth_token_ref.is_some() {
                    previous = self.secret_store.get_json(account).map_err(|_| {
                        err(
                            McpErrorCode::ERR_SECRET_STORE_UNAVAILABLE,
                            "failed to access native secret storage",
                        )
                    })?;
                    if self.secret_store.delete(account).is_err() {
                        restore_secret_bundle(
                            self.secret_store.as_ref(),
                            account,
                            previous.as_ref(),
                        )?;
                        return Err(err(
                            McpErrorCode::ERR_SECRET_STORE_UNAVAILABLE,
                            "failed to clear auth token",
                        ));
                    }
                    changed_secret = true;
                }
                final_settings.auth_token_ref = None;
            }
            AuthTokenMutation::Keep => {}
        }

        if let Err(error) = self.settings_store.save_atomic(&final_settings) {
            if changed_secret {
                restore_secret_bundle(self.secret_store.as_ref(), account, previous.as_ref())?;
            }
            return Err(error);
        }
        let persisted = self.settings_store.load();
        if !matches!(persisted, Ok(ref value) if value.auth_token_ref == final_settings.auth_token_ref)
        {
            self.settings_store.save_atomic(&existing).map_err(|_| {
                err(
                    McpErrorCode::ERR_SECRET_MIGRATION_FAILED,
                    "failed to restore MCP settings after verification failure",
                )
            })?;
            if changed_secret {
                restore_secret_bundle(self.secret_store.as_ref(), account, previous.as_ref())?;
            }
            return Err(err(
                McpErrorCode::ERR_SECRET_MIGRATION_FAILED,
                "failed to verify MCP settings update",
            ));
        }
        match self.mcp_status().await {
            Ok(status) => Ok(status),
            Err(error) => {
                self.settings_store.save_atomic(&existing).map_err(|_| {
                    err(
                        McpErrorCode::ERR_SECRET_MIGRATION_FAILED,
                        "failed to restore MCP settings after runtime failure",
                    )
                })?;
                if changed_secret {
                    restore_secret_bundle(self.secret_store.as_ref(), account, previous.as_ref())?;
                }
                Err(error)
            }
        }
    }

    pub async fn start_http_server(&self) -> McpResult<McpRuntimeStatus> {
        let settings = self.settings_store.load()?;
        if settings.transport != McpTransport::Http {
            return Err(err_with_details(
                McpErrorCode::ERR_INTERNAL,
                "MCP transport is not set to HTTP",
                json!({ "transport": settings.transport }),
            ));
        }

        self.stop_http_server_inner().await?;
        let auth_token = resolve_auth_token(&settings.auth_token_ref, self.secret_store.as_ref())?;
        let allow_insecure =
            auth_token.is_none() && is_loopback_bind_address(&settings.bind_address);
        let mut runtime_settings = settings.clone();
        runtime_settings.auth_token = auth_token;
        let server = start_http_server_from_settings(
            self.registry.clone(),
            &runtime_settings,
            allow_insecure,
            self.confirmations.clone(),
            self.sessions.clone(),
        )
        .await
        .map_err(map_runtime_io_error)?;
        let endpoint = server.endpoint().to_string();

        let mut guard = self.http_runtime.lock().await;
        *guard = Some(server);
        drop(guard);

        self.status_with_endpoint(settings, Some(endpoint))
    }

    pub async fn stop_http_server(&self) -> McpResult<McpRuntimeStatus> {
        let settings = self.settings_store.load()?;
        self.stop_http_server_inner().await?;
        self.status_with_endpoint(settings, None)
    }

    pub async fn ensure_runtime_from_settings(&self) -> McpResult<()> {
        let settings = self.settings_store.load()?;
        if settings.enabled && settings.transport == McpTransport::Http {
            let _ = self.start_http_server().await?;
        } else {
            let _ = self.stop_http_server().await?;
        }
        Ok(())
    }

    pub async fn mcp_status(&self) -> McpResult<McpRuntimeStatus> {
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

        Ok(McpClientSnippets {
            stdio: serde_json::to_string_pretty(&json!({
                "mcpServers": {
                    "infimount": {
                        "command": "infimount_mcp",
                        "args": ["--transport", "stdio"]
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

        let auth_token_configured = settings.auth_token_ref.is_some()
            || std::env::var("INFIMOUNT_AUTH_TOKEN")
                .map(|value| !value.trim().is_empty())
                .unwrap_or(false);

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

fn is_loopback_bind_address(value: &str) -> bool {
    let normalized = value.trim().to_ascii_lowercase();
    normalized == "localhost"
        || normalized == "::1"
        || normalized == "[::1]"
        || normalized.starts_with("127.")
}

fn restore_secret_bundle(
    secret_store: &dyn SecretStore,
    account: &str,
    previous: Option<&Value>,
) -> McpResult<()> {
    let restored = match previous {
        Some(value) => secret_store.put_json(account, value),
        None => secret_store.delete(account),
    };
    restored.map_err(|_| {
        err(
            McpErrorCode::ERR_SECRET_MIGRATION_FAILED,
            "MCP auth rollback failed; manual secret-store repair is required",
        )
    })
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
}
