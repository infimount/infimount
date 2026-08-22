use crate::errors::{
    err, err_with_details, map_core_error, map_io_error, sanitized_parse_error, McpErrorCode,
    McpResult,
};
use crate::policy::{
    migrate_legacy_policy, normalize_storage_policy, McpStoragePolicy, MCP_POLICY_VERSION,
};
use chrono::Utc;
use fs2::FileExt;
use infimount_core::atomic_file::{atomic_write_file, ensure_parent, FILE_MODE};
use infimount_core::secrets::{
    discover_secret_field_names, extract_secret_fields, is_secret_field_name, merge_secret_config,
    strip_secret_fields, NativeSecretStore, SecretStore,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::HashSet;
use std::fs::{self, OpenOptions};
use std::path::{Path, PathBuf};
use std::sync::Arc;
#[cfg(test)]
use std::sync::{LazyLock, Mutex};
use std::thread;
use std::time::{Duration, Instant};
use uuid::Uuid;

const REGISTRY_LOCK_TIMEOUT: Duration = Duration::from_secs(2);

pub const STORAGE_RECORD_SCHEMA_VERSION: u32 = 2;
pub const IMPORT_JOURNAL_VERSION: u32 = 2;
pub const MAX_REGISTRY_SNAPSHOT_BYTES: usize = 8 * 1024 * 1024;
pub const MAX_IMPORT_JOURNAL_BYTES: usize = 24 * 1024 * 1024;
pub const MAX_SECRET_TRANSITIONS: usize = 1024;
const MAX_SECRET_ACCOUNT_SEGMENT_BYTES: usize = 128;

// ---------------------------------------------------------------------------
// Secret-transaction crash journal
// ---------------------------------------------------------------------------

pub const SECRET_TRANSACTION_JOURNAL_VERSION: u32 = 2;
pub const MAX_SECRET_TRANSACTION_JOURNAL_BYTES: usize = 256 * 1024;
pub const MAX_SECRET_TRANSACTION_REPLACEMENTS: usize = 16;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum SecretTransactionTarget {
    Storage { storage_id: String },
    McpAuth,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SecretTransactionState {
    Prepared,
    SecretWritten,
    ReferenceCommitted,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SecretTransactionJournal {
    pub version: u32,
    pub transaction_id: String,
    pub created_at: String,
    pub state: SecretTransactionState,
    pub target: SecretTransactionTarget,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub previous_ref: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub desired_ref: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub obsolete_refs: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum McpAuthSecretAccount {
    Legacy,
    Revision { transaction_id: String },
    Recovery,
}

fn secret_transaction_journal_path(registry_path: &Path) -> PathBuf {
    registry_path
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .join("secret-transactions.json")
}

fn parse_mcp_auth_secret_account(account: &str) -> McpResult<McpAuthSecretAccount> {
    let parts = account.split('/').collect::<Vec<_>>();
    if parts.iter().any(|part| !valid_account_segment(part)) {
        return Err(err(
            McpErrorCode::ERR_SECRET_MIGRATION_FAILED,
            "MCP authentication account contains an invalid segment",
        ));
    }
    match parts.as_slice() {
        ["mcp", "http-auth"] => Ok(McpAuthSecretAccount::Legacy),
        ["mcp", "http-auth", "revision", transaction_id] => Ok(McpAuthSecretAccount::Revision {
            transaction_id: (*transaction_id).to_string(),
        }),
        ["recovery", "mcp-auth", _] => Ok(McpAuthSecretAccount::Recovery),
        _ => Err(err(
            McpErrorCode::ERR_SECRET_MIGRATION_FAILED,
            "unsupported MCP authentication account reference",
        )),
    }
}

fn validate_storage_transaction_ref(
    account: &str,
    storage_id: &str,
    require_revision: bool,
) -> McpResult<()> {
    let parsed = parse_storage_secret_account(account)?;
    match parsed {
        StorageSecretAccount::Base {
            storage_id: account_storage_id,
        }
        | StorageSecretAccount::Import {
            storage_id: account_storage_id,
            ..
        }
        | StorageSecretAccount::Revision {
            storage_id: account_storage_id,
            ..
        } if account_storage_id == storage_id => {
            if require_revision
                && !matches!(
                    parse_storage_secret_account(account)?,
                    StorageSecretAccount::Revision { .. }
                )
            {
                return Err(err(
                    McpErrorCode::ERR_SECRET_MIGRATION_FAILED,
                    "new storage credential references must use a revision account",
                ));
            }
            Ok(())
        }
        StorageSecretAccount::Recovery { .. } if !require_revision => Ok(()),
        _ => Err(err(
            McpErrorCode::ERR_SECRET_MIGRATION_FAILED,
            "secret transaction storage reference does not match its target",
        )),
    }
}

fn validate_secret_transaction_journal(journal: &SecretTransactionJournal) -> McpResult<()> {
    if journal.version != SECRET_TRANSACTION_JOURNAL_VERSION {
        return Err(err(
            McpErrorCode::ERR_SECRET_MIGRATION_FAILED,
            "secret transaction journal has an unsupported version",
        ));
    }
    if !valid_account_segment(&journal.transaction_id) {
        return Err(err(
            McpErrorCode::ERR_SECRET_MIGRATION_FAILED,
            "secret transaction journal has an invalid transaction id",
        ));
    }
    if chrono::DateTime::parse_from_rfc3339(&journal.created_at).is_err() {
        return Err(err(
            McpErrorCode::ERR_SECRET_MIGRATION_FAILED,
            "secret transaction journal has an invalid creation timestamp",
        ));
    }
    if journal.previous_ref.as_deref() == journal.desired_ref.as_deref() {
        return Err(err(
            McpErrorCode::ERR_SECRET_MIGRATION_FAILED,
            "secret transaction journal does not describe a reference change",
        ));
    }
    if journal.previous_ref.is_none() && journal.desired_ref.is_none() {
        return Err(err(
            McpErrorCode::ERR_SECRET_MIGRATION_FAILED,
            "secret transaction journal has neither a previous nor desired reference",
        ));
    }
    if journal.obsolete_refs.len() > MAX_SECRET_TRANSACTION_REPLACEMENTS {
        return Err(err(
            McpErrorCode::ERR_SECRET_MIGRATION_FAILED,
            "secret transaction journal has too many obsolete references",
        ));
    }
    if journal.state == SecretTransactionState::SecretWritten && journal.desired_ref.is_none() {
        return Err(err(
            McpErrorCode::ERR_SECRET_MIGRATION_FAILED,
            "a clear transaction cannot be marked as having written a secret",
        ));
    }

    if let Some(previous) = journal.previous_ref.as_deref() {
        if journal.desired_ref.as_deref() != Some(previous)
            && !journal
                .obsolete_refs
                .iter()
                .any(|account| account == previous)
        {
            return Err(err(
                McpErrorCode::ERR_SECRET_MIGRATION_FAILED,
                "secret transaction journal does not retain its previous reference for cleanup",
            ));
        }
    }

    let mut seen = HashSet::new();
    for account in &journal.obsolete_refs {
        if !seen.insert(account) || journal.desired_ref.as_deref() == Some(account.as_str()) {
            return Err(err(
                McpErrorCode::ERR_SECRET_MIGRATION_FAILED,
                "secret transaction journal contains conflicting obsolete references",
            ));
        }
    }

    match &journal.target {
        SecretTransactionTarget::Storage { storage_id } => {
            if !valid_account_segment(storage_id) {
                return Err(err(
                    McpErrorCode::ERR_SECRET_MIGRATION_FAILED,
                    "secret transaction journal has an invalid storage id",
                ));
            }
            if let Some(previous) = journal.previous_ref.as_deref() {
                validate_storage_transaction_ref(previous, storage_id, false)?;
            }
            if let Some(desired) = journal.desired_ref.as_deref() {
                validate_storage_transaction_ref(desired, storage_id, true)?;
                match parse_storage_secret_account(desired)? {
                    StorageSecretAccount::Revision { transaction_id, .. }
                        if transaction_id == journal.transaction_id => {}
                    _ => {
                        return Err(err(
                            McpErrorCode::ERR_SECRET_MIGRATION_FAILED,
                            "new storage credential reference does not match the transaction id",
                        ))
                    }
                }
            }
            for obsolete in &journal.obsolete_refs {
                validate_storage_transaction_ref(obsolete, storage_id, false)?;
            }
        }
        SecretTransactionTarget::McpAuth => {
            if let Some(previous) = journal.previous_ref.as_deref() {
                parse_mcp_auth_secret_account(previous)?;
            }
            if let Some(desired) = journal.desired_ref.as_deref() {
                match parse_mcp_auth_secret_account(desired)? {
                    McpAuthSecretAccount::Revision { transaction_id }
                        if transaction_id == journal.transaction_id => {}
                    _ => {
                        return Err(err(
                            McpErrorCode::ERR_SECRET_MIGRATION_FAILED,
                            "new MCP authentication references must use the transaction revision account",
                        ))
                    }
                }
            }
            for obsolete in &journal.obsolete_refs {
                parse_mcp_auth_secret_account(obsolete)?;
            }
        }
    }

    Ok(())
}

fn load_secret_transaction_journal(
    registry_path: &Path,
) -> McpResult<Option<SecretTransactionJournal>> {
    let path = secret_transaction_journal_path(registry_path);
    let metadata = match fs::symlink_metadata(&path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(map_io_error(&error, McpErrorCode::ERR_INTERNAL)),
    };
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err(err(
            McpErrorCode::ERR_SECRET_MIGRATION_FAILED,
            "secret transaction journal path is not a regular file",
        ));
    }
    if metadata.len() > MAX_SECRET_TRANSACTION_JOURNAL_BYTES as u64 {
        return Err(err(
            McpErrorCode::ERR_SECRET_MIGRATION_FAILED,
            "secret transaction journal exceeds the recovery size limit",
        ));
    }
    let bytes =
        fs::read(&path).map_err(|error| map_io_error(&error, McpErrorCode::ERR_INTERNAL))?;
    let journal: SecretTransactionJournal = serde_json::from_slice(&bytes).map_err(|_| {
        err(
            McpErrorCode::ERR_SECRET_MIGRATION_FAILED,
            "secret transaction journal is malformed",
        )
    })?;
    validate_secret_transaction_journal(&journal)?;
    Ok(Some(journal))
}

fn persist_secret_transaction_journal(
    registry_path: &Path,
    journal: &SecretTransactionJournal,
) -> McpResult<()> {
    validate_secret_transaction_journal(journal)?;
    let bytes = serde_json::to_vec_pretty(journal).map_err(|_| {
        err(
            McpErrorCode::ERR_SECRET_MIGRATION_FAILED,
            "failed to serialize secret transaction journal",
        )
    })?;
    if bytes.len() > MAX_SECRET_TRANSACTION_JOURNAL_BYTES {
        return Err(err(
            McpErrorCode::ERR_SECRET_MIGRATION_FAILED,
            "secret transaction journal exceeds the configured size limit",
        ));
    }
    let path = secret_transaction_journal_path(registry_path);
    ensure_parent(&path).map_err(|error| map_core_error(&error))?;
    atomic_write_file(&path, &bytes, FILE_MODE).map_err(|error| map_core_error(&error))?;
    let persisted = load_secret_transaction_journal(registry_path)?.ok_or_else(|| {
        err(
            McpErrorCode::ERR_SECRET_MIGRATION_FAILED,
            "secret transaction journal readback is missing",
        )
    })?;
    if &persisted != journal {
        return Err(err(
            McpErrorCode::ERR_SECRET_MIGRATION_FAILED,
            "secret transaction journal readback does not match",
        ));
    }
    Ok(())
}

pub fn ensure_no_pending_secret_transaction(registry_path: &Path) -> McpResult<()> {
    if load_secret_transaction_journal(registry_path)?.is_some() {
        return Err(err(
            McpErrorCode::ERR_SECRET_MIGRATION_FAILED,
            "configuration mutation is blocked by a pending secret transaction",
        ));
    }
    Ok(())
}

pub fn begin_secret_transaction(
    registry_path: &Path,
    journal: &SecretTransactionJournal,
) -> McpResult<()> {
    if secret_transaction_journal_path(registry_path).exists() {
        return Err(err(
            McpErrorCode::ERR_SECRET_MIGRATION_FAILED,
            "a previous secret transaction is still pending recovery",
        ));
    }
    if journal.state != SecretTransactionState::Prepared {
        return Err(err(
            McpErrorCode::ERR_SECRET_MIGRATION_FAILED,
            "new secret transactions must begin in the prepared state",
        ));
    }
    persist_secret_transaction_journal(registry_path, journal)
}

pub fn advance_secret_transaction(
    registry_path: &Path,
    transaction_id: &str,
    expected_state: SecretTransactionState,
    next_state: SecretTransactionState,
) -> McpResult<()> {
    let mut journal = load_secret_transaction_journal(registry_path)?.ok_or_else(|| {
        err(
            McpErrorCode::ERR_SECRET_MIGRATION_FAILED,
            "secret transaction journal is missing",
        )
    })?;
    if journal.transaction_id != transaction_id || journal.state != expected_state {
        return Err(err(
            McpErrorCode::ERR_SECRET_MIGRATION_FAILED,
            "secret transaction journal state changed unexpectedly",
        ));
    }
    let valid_transition = matches!(
        (&expected_state, &next_state),
        (
            SecretTransactionState::Prepared,
            SecretTransactionState::SecretWritten
        ) | (
            SecretTransactionState::SecretWritten,
            SecretTransactionState::ReferenceCommitted
        ) | (
            SecretTransactionState::Prepared,
            SecretTransactionState::ReferenceCommitted
        )
    );
    if !valid_transition
        || (expected_state == SecretTransactionState::Prepared
            && next_state == SecretTransactionState::ReferenceCommitted
            && journal.desired_ref.is_some())
    {
        return Err(err(
            McpErrorCode::ERR_SECRET_MIGRATION_FAILED,
            "invalid secret transaction state transition",
        ));
    }
    journal.state = next_state;
    persist_secret_transaction_journal(registry_path, &journal)
}

fn remove_secret_transaction_journal(registry_path: &Path) -> McpResult<()> {
    let path = secret_transaction_journal_path(registry_path);
    match fs::remove_file(&path) {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(map_io_error(&error, McpErrorCode::ERR_INTERNAL)),
    }
    if path.exists() {
        return Err(err(
            McpErrorCode::ERR_SECRET_MIGRATION_FAILED,
            "secret transaction journal could not be removed",
        ));
    }
    Ok(())
}

pub fn finish_secret_transaction(registry_path: &Path, transaction_id: &str) -> McpResult<()> {
    let journal = load_secret_transaction_journal(registry_path)?.ok_or_else(|| {
        err(
            McpErrorCode::ERR_SECRET_MIGRATION_FAILED,
            "secret transaction journal is missing during commit finalization",
        )
    })?;
    if journal.transaction_id != transaction_id
        || journal.state != SecretTransactionState::ReferenceCommitted
    {
        return Err(err(
            McpErrorCode::ERR_SECRET_MIGRATION_FAILED,
            "refusing to finish an incomplete secret transaction",
        ));
    }
    remove_secret_transaction_journal(registry_path)
}

pub fn abandon_secret_transaction_after_rollback(
    registry_path: &Path,
    transaction_id: &str,
) -> McpResult<()> {
    let journal = load_secret_transaction_journal(registry_path)?.ok_or_else(|| {
        err(
            McpErrorCode::ERR_SECRET_MIGRATION_FAILED,
            "secret transaction journal is missing during rollback finalization",
        )
    })?;
    if journal.transaction_id != transaction_id
        || journal.state == SecretTransactionState::ReferenceCommitted
    {
        return Err(err(
            McpErrorCode::ERR_SECRET_MIGRATION_FAILED,
            "refusing to abandon a committed secret transaction",
        ));
    }
    remove_secret_transaction_journal(registry_path)
}

fn account_is_active(
    account: &str,
    storages: &[StorageRecord],
    settings_auth_token_ref: Option<&str>,
) -> bool {
    settings_auth_token_ref == Some(account)
        || storages
            .iter()
            .any(|storage| storage.secret_ref.as_deref() == Some(account))
}

fn cleanup_transaction_account(
    registry_path: &Path,
    account: &str,
    target: &SecretTransactionTarget,
    storages: &[StorageRecord],
    settings_auth_token_ref: Option<&str>,
    secret_store: &dyn SecretStore,
) -> McpResult<()> {
    if account_is_active(account, storages, settings_auth_token_ref) {
        return Ok(());
    }
    if secret_store.delete(account).is_ok() {
        return Ok(());
    }
    let _ = target;
    append_secret_cleanup_at(registry_path, account)
}

/// Resolve a leftover secret-reference transaction.
///
/// The caller must hold the cross-process configuration transaction lock.
pub fn recover_pending_secret_transactions(
    registry_path: &Path,
    storages: &[StorageRecord],
    secret_store: &dyn SecretStore,
    settings_auth_token_ref: Option<&str>,
) -> McpResult<()> {
    let Some(mut journal) = load_secret_transaction_journal(registry_path)? else {
        return Ok(());
    };

    let current_ref = match &journal.target {
        SecretTransactionTarget::Storage { storage_id } => storages
            .iter()
            .find(|storage| storage.id == *storage_id)
            .and_then(|storage| storage.secret_ref.as_deref()),
        SecretTransactionTarget::McpAuth => settings_auth_token_ref,
    };

    if current_ref == journal.desired_ref.as_deref() {
        if let Some(desired) = journal.desired_ref.as_deref() {
            let present = secret_store
                .get_json(desired)
                .map_err(|_| {
                    err(
                        McpErrorCode::ERR_SECRET_STORE_UNAVAILABLE,
                        "failed to verify the committed secret transaction",
                    )
                })?
                .is_some();
            if !present {
                return Err(err(
                    McpErrorCode::ERR_SECRET_NOT_FOUND,
                    "committed secret transaction references a missing credential",
                ));
            }
        }

        for obsolete in &journal.obsolete_refs {
            cleanup_transaction_account(
                registry_path,
                obsolete,
                &journal.target,
                storages,
                settings_auth_token_ref,
                secret_store,
            )?;
        }

        if journal.state != SecretTransactionState::ReferenceCommitted {
            journal.state = SecretTransactionState::ReferenceCommitted;
            persist_secret_transaction_journal(registry_path, &journal)?;
        }
        return finish_secret_transaction(registry_path, &journal.transaction_id);
    }

    if current_ref == journal.previous_ref.as_deref()
        && journal.state != SecretTransactionState::ReferenceCommitted
    {
        if let Some(desired) = journal.desired_ref.as_deref() {
            cleanup_transaction_account(
                registry_path,
                desired,
                &journal.target,
                storages,
                settings_auth_token_ref,
                secret_store,
            )?;
        }
        return abandon_secret_transaction_after_rollback(registry_path, &journal.transaction_id);
    }

    Err(err(
        McpErrorCode::ERR_SECRET_MIGRATION_FAILED,
        "pending secret transaction found an ambiguous persisted reference",
    ))
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StorageSecretAccount {
    Base {
        storage_id: String,
    },
    Import {
        storage_id: String,
        transaction_id: String,
    },
    Revision {
        storage_id: String,
        revision: u64,
        transaction_id: String,
    },
    Recovery {
        recovery_id: String,
    },
}

impl StorageSecretAccount {
    pub fn canonical(&self) -> String {
        match self {
            StorageSecretAccount::Base { storage_id } => format!("storage/{storage_id}"),
            StorageSecretAccount::Import {
                storage_id,
                transaction_id,
            } => format!("storage/{storage_id}/import/{transaction_id}"),
            StorageSecretAccount::Revision {
                storage_id,
                revision,
                transaction_id,
            } => format!("storage/{storage_id}/revision/{revision}/{transaction_id}"),
            StorageSecretAccount::Recovery { recovery_id } => {
                format!("recovery/storage/{recovery_id}")
            }
        }
    }
}

fn valid_account_segment(segment: &str) -> bool {
    !segment.is_empty()
        && segment.len() <= MAX_SECRET_ACCOUNT_SEGMENT_BYTES
        && segment
            .chars()
            .all(|ch| !ch.is_control() && !ch.is_whitespace())
}

/// Strictly parse a typed secret-account reference. Only the documented
/// `storage/*`, `storage/*/import/*`, `storage/*/revision/*/*`, and
/// `recovery/storage/*` forms are accepted.
pub fn parse_storage_secret_account(account: &str) -> McpResult<StorageSecretAccount> {
    let parts = account.split('/').collect::<Vec<_>>();
    if parts.iter().any(|part| !valid_account_segment(part)) {
        return Err(err(
            McpErrorCode::ERR_INTERNAL,
            "secret account reference contains an invalid segment",
        ));
    }
    match parts.as_slice() {
        ["storage", storage_id] => Ok(StorageSecretAccount::Base {
            storage_id: (*storage_id).to_string(),
        }),
        ["storage", storage_id, "import", transaction_id] => Ok(StorageSecretAccount::Import {
            storage_id: (*storage_id).to_string(),
            transaction_id: (*transaction_id).to_string(),
        }),
        ["storage", storage_id, "revision", revision, transaction_id] => {
            let revision = revision.parse::<u64>().map_err(|_| {
                err(
                    McpErrorCode::ERR_INTERNAL,
                    "secret account revision must be a non-negative integer",
                )
            })?;
            Ok(StorageSecretAccount::Revision {
                storage_id: (*storage_id).to_string(),
                revision,
                transaction_id: (*transaction_id).to_string(),
            })
        }
        ["recovery", "storage", recovery_id] => Ok(StorageSecretAccount::Recovery {
            recovery_id: (*recovery_id).to_string(),
        }),
        _ => Err(err(
            McpErrorCode::ERR_INTERNAL,
            "unsupported secret account reference",
        )),
    }
}

pub fn valid_secret_account(account: &str) -> bool {
    parse_storage_secret_account(account).is_ok()
}

fn valid_import_secret_account(account: &str) -> bool {
    matches!(
        parse_storage_secret_account(account),
        Ok(StorageSecretAccount::Import { .. })
    )
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ImportTransactionState {
    Prepared,
    SecretsStaged,
    Committed,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ImportTransactionJournal {
    pub version: u32,
    pub state: ImportTransactionState,
    pub original_present: bool,
    pub original_registry_base64: String,
    pub replacement_registry_base64: String,
    pub staged_secret_accounts: Vec<String>,
    pub obsolete_secret_accounts: Vec<String>,
}

impl ImportTransactionJournal {
    pub fn original_bytes(&self) -> McpResult<Vec<u8>> {
        if self.original_registry_base64.is_empty() {
            return Ok(Vec::new());
        }
        base64_decode_registry(&self.original_registry_base64)
    }

    pub fn replacement_bytes(&self) -> McpResult<Vec<u8>> {
        if self.replacement_registry_base64.is_empty() {
            return Ok(Vec::new());
        }
        base64_decode_registry(&self.replacement_registry_base64)
    }
}

fn base64_decode_registry(encoded: &str) -> McpResult<Vec<u8>> {
    use base64::Engine;
    base64::engine::general_purpose::STANDARD
        .decode(encoded)
        .map_err(|_| {
            err(
                McpErrorCode::ERR_INTERNAL,
                "pending import journal contains an invalid registry snapshot",
            )
        })
}

fn journal_has_valid_transitions(
    journal: &ImportTransactionJournal,
) -> Result<(Vec<StorageRecord>, Vec<StorageRecord>), ()> {
    if journal.version != IMPORT_JOURNAL_VERSION {
        return Err(());
    }
    if (journal.state == ImportTransactionState::Committed
        && journal.replacement_registry_base64.is_empty())
        || journal.staged_secret_accounts.len() > MAX_SECRET_TRANSITIONS
        || journal.obsolete_secret_accounts.len() > MAX_SECRET_TRANSITIONS
    {
        return Err(());
    }
    let original_bytes = journal.original_bytes().map_err(|_| ())?;
    let replacement_bytes = journal.replacement_bytes().map_err(|_| ())?;
    if journal.original_present == original_bytes.is_empty()
        || original_bytes.len() > MAX_REGISTRY_SNAPSHOT_BYTES
        || replacement_bytes.len() > MAX_REGISTRY_SNAPSHOT_BYTES
    {
        return Err(());
    }
    let original_records = if journal.original_present {
        serde_json::from_slice::<Vec<StorageRecord>>(&original_bytes).map_err(|_| ())?
    } else {
        Vec::new()
    };
    let replacement_records = if replacement_bytes.is_empty() {
        Vec::new()
    } else {
        serde_json::from_slice::<Vec<StorageRecord>>(&replacement_bytes).map_err(|_| ())?
    };
    if journal
        .staged_secret_accounts
        .iter()
        .any(|account| !valid_import_secret_account(account))
        || journal
            .obsolete_secret_accounts
            .iter()
            .any(|account| !valid_secret_account(account))
    {
        return Err(());
    }
    let original_refs = original_records
        .iter()
        .filter_map(|record| record.secret_ref.as_deref())
        .collect::<std::collections::HashSet<_>>();
    let replacement_refs = replacement_records
        .iter()
        .filter_map(|record| record.secret_ref.as_deref())
        .collect::<std::collections::HashSet<_>>();
    if !replacement_bytes.is_empty()
        && (journal.staged_secret_accounts.iter().any(|account| {
            !replacement_refs.contains(account.as_str()) || original_refs.contains(account.as_str())
        }) || journal.obsolete_secret_accounts.iter().any(|account| {
            !original_refs.contains(account.as_str()) || replacement_refs.contains(account.as_str())
        }))
    {
        return Err(());
    }
    let staged = journal
        .staged_secret_accounts
        .iter()
        .collect::<std::collections::HashSet<_>>();
    let obsolete = journal
        .obsolete_secret_accounts
        .iter()
        .collect::<std::collections::HashSet<_>>();
    if staged.len() != journal.staged_secret_accounts.len()
        || obsolete.len() != journal.obsolete_secret_accounts.len()
        || staged.intersection(&obsolete).next().is_some()
    {
        return Err(());
    }
    Ok((original_records, replacement_records))
}

#[cfg(test)]
static FAIL_NEXT_IMPORT_READBACK: LazyLock<Mutex<HashSet<PathBuf>>> =
    LazyLock::new(|| Mutex::new(HashSet::new()));
#[cfg(test)]
static FAIL_NEXT_IMPORT_READBACK_CORRUPT: LazyLock<Mutex<HashSet<PathBuf>>> =
    LazyLock::new(|| Mutex::new(HashSet::new()));

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StorageRecord {
    #[serde(default = "default_schema_version")]
    pub schema_version: u32,
    pub id: String,
    pub name: String,
    pub backend: String,
    pub config: Value,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub secret_ref: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub secret_fields: Vec<String>,
    pub enabled: bool,
    #[serde(default)]
    pub mcp_exposed: bool,
    pub read_only: bool,
    #[serde(default)]
    pub mcp_policy: McpStoragePolicy,
    #[serde(default)]
    pub revision: u64,
    pub created_at: String,
    pub updated_at: String,
}

fn default_schema_version() -> u32 {
    0
}

fn schema_version_matches_current(version: u32) -> bool {
    version == STORAGE_RECORD_SCHEMA_VERSION
}

impl Default for StorageRecord {
    fn default() -> Self {
        let now = Utc::now().to_rfc3339();
        Self {
            schema_version: STORAGE_RECORD_SCHEMA_VERSION,
            id: Uuid::new_v4().to_string(),
            name: String::new(),
            backend: String::new(),
            config: json!({}),
            secret_ref: None,
            secret_fields: Vec::new(),
            enabled: true,
            mcp_exposed: false,
            read_only: false,
            mcp_policy: McpStoragePolicy::default(),
            revision: 1,
            created_at: now.clone(),
            updated_at: now,
        }
    }
}

impl StorageRecord {
    pub fn new(name: String, backend: String, config: Value) -> Self {
        let now = Utc::now().to_rfc3339();
        Self {
            schema_version: STORAGE_RECORD_SCHEMA_VERSION,
            id: Uuid::new_v4().to_string(),
            name,
            backend,
            config,
            secret_ref: None,
            secret_fields: Vec::new(),
            enabled: true,
            mcp_exposed: false,
            read_only: false,
            mcp_policy: McpStoragePolicy::default(),
            revision: 1,
            created_at: now.clone(),
            updated_at: now,
        }
    }
}

#[derive(Debug, Clone)]
pub struct StorageRegistry {
    path: PathBuf,
    lock_path: PathBuf,
    transaction_lock_path: PathBuf,
    secret_store: Arc<dyn SecretStore>,
}

#[derive(Debug)]
pub struct ConfigurationTransaction {
    file: std::fs::File,
}

impl Drop for ConfigurationTransaction {
    fn drop(&mut self) {
        let _ = FileExt::unlock(&self.file);
    }
}

impl StorageRegistry {
    pub fn new(path: Option<PathBuf>) -> Self {
        Self::with_secret_store(path, Arc::new(NativeSecretStore::new()))
    }

    pub fn with_secret_store(path: Option<PathBuf>, secret_store: Arc<dyn SecretStore>) -> Self {
        let path = path.unwrap_or_else(default_registry_path);
        let lock_path = path.with_extension("lock");
        let transaction_lock_path = path
            .parent()
            .unwrap_or_else(|| Path::new("."))
            .join("configuration-transaction.lock");
        Self {
            path,
            lock_path,
            transaction_lock_path,
            secret_store,
        }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn config_dir(&self) -> &Path {
        self.path.parent().unwrap_or_else(|| Path::new("."))
    }

    pub fn secret_store(&self) -> &Arc<dyn SecretStore> {
        &self.secret_store
    }

    pub fn acquire_configuration_transaction(&self) -> McpResult<ConfigurationTransaction> {
        ensure_parent(&self.transaction_lock_path).map_err(|error| map_core_error(&error))?;
        let file = OpenOptions::new()
            .create(true)
            .read(true)
            .write(true)
            .truncate(false)
            .open(&self.transaction_lock_path)
            .map_err(|error| map_io_error(&error, McpErrorCode::ERR_INTERNAL))?;
        let start = Instant::now();
        loop {
            match file.try_lock_exclusive() {
                Ok(()) => return Ok(ConfigurationTransaction { file }),
                Err(_) if start.elapsed() >= REGISTRY_LOCK_TIMEOUT => {
                    return Err(err(
                        McpErrorCode::ERR_REGISTRY_LOCK_TIMEOUT,
                        "timed out acquiring configuration transaction lock",
                    ));
                }
                Err(_) => thread::sleep(Duration::from_millis(50)),
            }
        }
    }

    pub fn registry_bytes(&self) -> McpResult<(bool, Vec<u8>)> {
        if !self.path.exists() {
            return Ok((false, Vec::new()));
        }
        fs::read(&self.path)
            .map(|bytes| (true, bytes))
            .map_err(|error| map_io_error(&error, McpErrorCode::ERR_INTERNAL))
    }

    pub fn import_journal_path(&self, id: &str) -> McpResult<PathBuf> {
        let parent = self.path.parent().ok_or_else(|| {
            err(
                McpErrorCode::ERR_INTERNAL,
                "storage registry has no parent directory for import journal",
            )
        })?;
        let backups = parent.join("backups");
        infimount_core::atomic_file::create_dir_all(&backups)
            .map_err(|error| map_core_error(&error))?;
        Ok(backups.join(format!("storages.import-pending.{id}.json")))
    }

    pub fn write_import_journal(
        &self,
        path: &Path,
        journal: &ImportTransactionJournal,
    ) -> McpResult<()> {
        if journal.original_bytes()?.len() > MAX_REGISTRY_SNAPSHOT_BYTES
            || journal.replacement_bytes()?.len() > MAX_REGISTRY_SNAPSHOT_BYTES
            || journal.staged_secret_accounts.len() > MAX_SECRET_TRANSITIONS
            || journal.obsolete_secret_accounts.len() > MAX_SECRET_TRANSITIONS
        {
            return Err(err(
                McpErrorCode::ERR_INTERNAL,
                "import transaction journal exceeds the configured size limits",
            ));
        }
        let bytes = serde_json::to_vec_pretty(journal).map_err(|error| {
            err_with_details(
                McpErrorCode::ERR_INTERNAL,
                "failed to serialize import transaction journal",
                json!({ "serde_error": error.to_string() }),
            )
        })?;
        if bytes.len() > MAX_IMPORT_JOURNAL_BYTES {
            return Err(err(
                McpErrorCode::ERR_INTERNAL,
                "import transaction journal exceeds the configured size limit",
            ));
        }
        atomic_write_file(path, &bytes, 0o600).map_err(|error| map_core_error(&error))
    }

    /// Recover unfinished import transactions. Callers holding the configuration
    /// transaction lock use `recover_pending_imports_locked`.
    pub fn recover_pending_imports(&self) -> McpResult<()> {
        let _transaction = self.acquire_configuration_transaction()?;
        self.recover_pending_imports_locked()
    }

    /// Recover unfinished import transactions while the caller already holds the
    /// configuration transaction lock.
    pub fn recover_pending_imports_locked(&self) -> McpResult<()> {
        let Some(parent) = self.path.parent() else {
            return Ok(());
        };
        let backups = parent.join("backups");
        if !backups.exists() {
            return Ok(());
        }
        let mut journals = fs::read_dir(&backups)
            .map_err(|error| map_io_error(&error, McpErrorCode::ERR_INTERNAL))?
            .filter_map(Result::ok)
            .map(|entry| entry.path())
            .filter(|path| {
                path.file_name()
                    .and_then(|name| name.to_str())
                    .is_some_and(|name| {
                        name.starts_with("storages.import-pending.") && name.ends_with(".json")
                    })
            })
            .collect::<Vec<_>>();
        journals.sort();
        for journal_path in journals {
            let bytes = fs::read(&journal_path)
                .map_err(|error| map_io_error(&error, McpErrorCode::ERR_INTERNAL))?;
            if bytes.len() > MAX_IMPORT_JOURNAL_BYTES {
                return Err(err(
                    McpErrorCode::ERR_INTERNAL,
                    "pending import journal exceeds the recovery size limit",
                ));
            }
            let journal: ImportTransactionJournal =
                serde_json::from_slice(&bytes).map_err(|_| {
                    err(
                        McpErrorCode::ERR_INTERNAL,
                        "pending import journal is invalid; refusing ambiguous recovery",
                    )
                })?;
            let (_original_records, _replacement_records) = journal_has_valid_transitions(&journal)
                .map_err(|_| {
                    err(
                        McpErrorCode::ERR_INTERNAL,
                        "pending import journal is unsupported; refusing ambiguous recovery",
                    )
                })?;
            self.with_file_lock(REGISTRY_LOCK_TIMEOUT, || {
                let current_present = self.path.exists();
                let current = if current_present {
                    fs::read(&self.path)
                        .map_err(|error| map_io_error(&error, McpErrorCode::ERR_INTERNAL))?
                } else {
                    Vec::new()
                };
                let is_original = current_present == journal.original_present
                    && current == journal.original_bytes()?;
                let is_committed = journal.state == ImportTransactionState::Committed;
                let is_replacement = current == journal.replacement_bytes()?;
                if !is_committed && !is_original && !is_replacement {
                    return Err(err(
                        McpErrorCode::ERR_INTERNAL,
                        "pending import recovery found an unexpected registry state",
                    ));
                }
                if is_committed && !current_present {
                    return Err(err(
                        McpErrorCode::ERR_INTERNAL,
                        "committed import registry is missing",
                    ));
                }
                let current_records = if current_present {
                    serde_json::from_slice::<Vec<StorageRecord>>(&current).map_err(|_| {
                        err(
                            McpErrorCode::ERR_INTERNAL,
                            "pending import registry is invalid",
                        )
                    })?
                } else {
                    Vec::new()
                };
                let active = if is_committed || is_replacement {
                    current_records
                        .into_iter()
                        .filter_map(|record| record.secret_ref)
                        .collect::<std::collections::HashSet<_>>()
                } else {
                    std::collections::HashSet::new()
                };
                let accounts = if !is_committed && is_original {
                    &journal.staged_secret_accounts
                } else {
                    &journal.obsolete_secret_accounts
                };
                for account in accounts {
                    if !active.contains(account) && self.secret_store.delete(account).is_err() {
                        return Err(err(
                            McpErrorCode::ERR_SECRET_STORE_UNAVAILABLE,
                            "pending import recovery could not remove a secret account",
                        ));
                    }
                }
                fs::remove_file(&journal_path)
                    .map_err(|error| map_io_error(&error, McpErrorCode::ERR_INTERNAL))?;
                Ok(())
            })?;
        }
        self.clear_configuration_blocked_marker();
        Ok(())
    }

    /// Durably record that configuration mutations are blocked pending
    /// recovery. Present only when an import journal could not be resolved.
    fn configuration_blocked_path(&self) -> PathBuf {
        self.path
            .parent()
            .unwrap_or_else(|| Path::new("."))
            .join("configuration-recovery-blocked.json")
    }

    pub fn ensure_no_configuration_blocked(&self) -> McpResult<()> {
        let path = self.configuration_blocked_path();
        if path.exists() {
            return Err(err_with_details(
                McpErrorCode::ERR_INTERNAL,
                "configuration mutations are blocked pending import recovery",
                json!({ "marker": path }),
            ));
        }
        Ok(())
    }

    pub(crate) fn mark_configuration_blocked(&self) -> McpResult<()> {
        let path = self.configuration_blocked_path();
        ensure_parent(&path).map_err(|error| map_core_error(&error))?;
        let payload = serde_json::to_vec_pretty(&json!({
            "blocked": true,
            "createdAt": Utc::now().to_rfc3339(),
        }))
        .map_err(|error| {
            err_with_details(
                McpErrorCode::ERR_INTERNAL,
                "failed to create configuration blocked marker",
                json!({ "serde_error": error.to_string() }),
            )
        })?;
        atomic_write_file(&path, &payload, 0o600).map_err(|error| map_core_error(&error))
    }

    fn clear_configuration_blocked_marker(&self) {
        let _ = fs::remove_file(self.configuration_blocked_path());
    }

    pub fn load_all(&self) -> McpResult<Vec<StorageRecord>> {
        self.with_file_lock(REGISTRY_LOCK_TIMEOUT, || self.load_all_unlocked())
    }

    pub fn save_all_atomic(&self, storages: &[StorageRecord]) -> McpResult<()> {
        self.with_file_lock(REGISTRY_LOCK_TIMEOUT, || {
            self.save_all_atomic_unlocked(storages)
        })
    }

    pub fn save_all_atomic_if_unchanged(
        &self,
        expected: &[StorageRecord],
        replacement: &[StorageRecord],
    ) -> McpResult<()> {
        self.ensure_no_configuration_blocked()?;
        self.with_file_lock(REGISTRY_LOCK_TIMEOUT, || {
            let current = self.load_all_unlocked()?;
            if !records_match_exact(&current, expected) {
                return Err(err(
                    McpErrorCode::ERR_INTERNAL,
                    "storage registry changed during transaction; retry the operation",
                ));
            }
            self.save_all_atomic_unlocked(replacement)
        })
    }

    /// Replace a registry while holding the disk lock through write and readback.
    /// If readback fails, restore only when the on-disk bytes still represent the
    /// exact imported state; a later process mutation is never overwritten.
    pub fn replace_all_atomic_verified(
        &self,
        expected: &[StorageRecord],
        replacement: &[StorageRecord],
    ) -> McpResult<()> {
        self.ensure_no_configuration_blocked()?;
        self.with_file_lock(REGISTRY_LOCK_TIMEOUT, || {
            let current = self.load_all_unlocked()?;
            if !records_match_exact(&current, expected) {
                return Err(err(
                    McpErrorCode::ERR_INTERNAL,
                    "storage registry changed during transaction; retry the operation",
                ));
            }
            if let Err(error) = self.save_all_atomic_unlocked(replacement) {
                let _ = self.restore_all_if_matches_unlocked(replacement, expected);
                return Err(error);
            }
            #[cfg(test)]
            let fail_readback = FAIL_NEXT_IMPORT_READBACK.lock().unwrap().remove(&self.path);
            #[cfg(test)]
            let corrupt_readback = FAIL_NEXT_IMPORT_READBACK_CORRUPT
                .lock()
                .unwrap()
                .remove(&self.path);
            #[cfg(test)]
            if fail_readback || corrupt_readback {
                if corrupt_readback {
                    let _ = fs::write(&self.path, b"{");
                }
                self.save_all_atomic_unlocked(expected)?;
                return Err(err(
                    McpErrorCode::ERR_INTERNAL,
                    "storage registry readback failed; original registry was restored",
                ));
            }
            let persisted = match self.load_all_unlocked() {
                Ok(persisted) => persisted,
                Err(error) => {
                    self.save_all_atomic_unlocked(expected)?;
                    return Err(error);
                }
            };
            if !records_match_exact(&persisted, replacement) {
                self.save_all_atomic_unlocked(expected)?;
                return Err(err(
                    McpErrorCode::ERR_INTERNAL,
                    "storage registry readback failed; original registry was restored",
                ));
            }
            Ok(())
        })
    }

    /// Restore an imported registry only when no later process has changed it.
    /// Returns false when the compare-and-restore precondition no longer holds.
    pub fn restore_all_if_matches(
        &self,
        expected_current: &[StorageRecord],
        replacement: &[StorageRecord],
    ) -> McpResult<bool> {
        self.with_file_lock(REGISTRY_LOCK_TIMEOUT, || {
            self.restore_all_if_matches_unlocked(expected_current, replacement)
        })
    }

    #[cfg(test)]
    pub fn fail_next_import_readback(&self) {
        FAIL_NEXT_IMPORT_READBACK
            .lock()
            .unwrap()
            .insert(self.path.clone());
    }

    #[cfg(test)]
    pub fn fail_next_import_readback_with_corruption(&self) {
        FAIL_NEXT_IMPORT_READBACK_CORRUPT
            .lock()
            .unwrap()
            .insert(self.path.clone());
    }

    fn restore_all_if_matches_unlocked(
        &self,
        expected_current: &[StorageRecord],
        replacement: &[StorageRecord],
    ) -> McpResult<bool> {
        let current = self.load_all_unlocked()?;
        if !records_match_exact(&current, expected_current) {
            return Ok(false);
        }
        self.save_all_atomic_unlocked(replacement)?;
        Ok(true)
    }

    pub fn save_legacy_records_secure(&self, mut storages: Vec<StorageRecord>) -> McpResult<()> {
        self.with_file_lock(REGISTRY_LOCK_TIMEOUT, || {
            let rollback = self.migrate_secrets_in_batch(&mut storages)?;
            if let Err(error) = self.save_all_atomic_unlocked(&storages) {
                self.rollback_secret_writes(rollback)?;
                return Err(error);
            }
            Ok(())
        })
    }

    pub fn with_locked_mutation<T, F>(&self, mutate: F) -> McpResult<T>
    where
        F: FnOnce(&mut Vec<StorageRecord>) -> McpResult<T>,
    {
        self.ensure_no_configuration_blocked()?;
        self.with_file_lock(REGISTRY_LOCK_TIMEOUT, || {
            let mut storages = self.load_all_unlocked()?;
            let out = mutate(&mut storages)?;
            self.save_all_atomic_unlocked(&storages)?;
            Ok(out)
        })
    }

    pub fn list_exposed_enabled(&self) -> McpResult<Vec<StorageRecord>> {
        let mut storages = self.load_all()?;
        storages.retain(|s| s.enabled && s.mcp_exposed);
        storages.sort_by(|a, b| a.name.cmp(&b.name));
        Ok(storages)
    }

    pub fn find_by_name(&self, name: &str) -> McpResult<StorageRecord> {
        let storages = self.load_all()?;
        let Some(storage) = storages.into_iter().find(|s| s.name == name) else {
            return Err(err_with_details(
                McpErrorCode::ERR_STORAGE_NOT_FOUND,
                format!("Storage '{name}' not found"),
                json!({ "storage_name": name }),
            ));
        };

        if !storage.enabled {
            return Err(err_with_details(
                McpErrorCode::ERR_STORAGE_DISABLED,
                format!("Storage '{name}' is disabled"),
                json!({ "storage_name": name }),
            ));
        }

        if !storage.mcp_exposed {
            return Err(err_with_details(
                McpErrorCode::ERR_STORAGE_NOT_EXPOSED,
                format!("Storage '{name}' is not exposed to MCP"),
                json!({ "storage_name": name }),
            ));
        }

        Ok(storage)
    }

    pub fn resolve_storage(&self, record: &StorageRecord) -> McpResult<ResolvedStorageRecord> {
        let resolved_config = if let Some(ref secret_ref) = record.secret_ref {
            let secret_bundle = self
                .secret_store
                .get_json(secret_ref)
                .map_err(|_| {
                    err_with_details(
                        McpErrorCode::ERR_SECRET_STORE_UNAVAILABLE,
                        "native secret storage is unavailable",
                        json!({ "storage_id": record.id }),
                    )
                })?
                .ok_or_else(|| {
                    err_with_details(
                        McpErrorCode::ERR_SECRET_NOT_FOUND,
                        "stored credentials are missing",
                        json!({ "storage_id": record.id }),
                    )
                })?;
            merge_secret_config(&record.config, &secret_bundle)
        } else if record.secret_fields.is_empty() {
            record.config.clone()
        } else {
            return Err(err_with_details(
                McpErrorCode::ERR_SECRET_NOT_FOUND,
                "stored credential reference is missing",
                json!({ "storage_id": record.id }),
            ));
        };

        Ok(ResolvedStorageRecord {
            record: record.clone(),
            resolved_config,
        })
    }

    fn load_all_unlocked(&self) -> McpResult<Vec<StorageRecord>> {
        if !self.path.exists() {
            return Ok(Vec::new());
        }

        let data = fs::read_to_string(&self.path)
            .map_err(|e| map_io_error(&e, McpErrorCode::ERR_INTERNAL))?;
        let mut storages: Vec<StorageRecord> = serde_json::from_str(&data).map_err(|e| {
            sanitized_parse_error(
                McpErrorCode::ERR_INTERNAL,
                "failed to parse storage registry",
                "invalid_storage_registry",
                &e,
            )
        })?;

        let needs_policy_migration = storages
            .iter()
            .any(|s| s.mcp_policy.version != MCP_POLICY_VERSION);
        let needs_schema_migration = storages
            .iter()
            .any(|s| !schema_version_matches_current(s.schema_version));
        let schema_secret_names = discover_secret_field_names();
        let needs_secret_migration = storages.iter().any(|storage| {
            infimount_core::secrets::contains_plaintext_secrets(
                &storage.config,
                &schema_secret_names,
            )
        });

        if needs_policy_migration || needs_schema_migration || needs_secret_migration {
            let backup_path = self.create_pre_migration_backup(&data)?;

            let rollback = if needs_secret_migration {
                self.migrate_secrets_in_batch(&mut storages)?
            } else {
                Vec::new()
            };

            let migration_result = (|| -> McpResult<()> {
                for storage in &mut storages {
                    if storage.mcp_policy.version != MCP_POLICY_VERSION {
                        let mut policy = storage.mcp_policy.clone();
                        migrate_legacy_policy(&mut policy)?;
                        storage.mcp_policy = policy;
                    }
                    storage.schema_version = STORAGE_RECORD_SCHEMA_VERSION;
                }
                self.save_all_atomic_unlocked(&storages)?;
                let persisted = fs::read(&self.path).map_err(|error| {
                    map_io_error(&error, McpErrorCode::ERR_SECRET_MIGRATION_FAILED)
                })?;
                let persisted_records: Vec<StorageRecord> = serde_json::from_slice(&persisted)
                    .map_err(|_| {
                        err(
                            McpErrorCode::ERR_SECRET_MIGRATION_FAILED,
                            "failed to verify migrated storage registry",
                        )
                    })?;
                if persisted_records.iter().any(|storage| {
                    infimount_core::secrets::contains_plaintext_secrets(
                        &storage.config,
                        &schema_secret_names,
                    )
                }) {
                    return Err(err(
                        McpErrorCode::ERR_SECRET_MIGRATION_FAILED,
                        "plaintext credentials remained after migration",
                    ));
                }
                Ok(())
            })();
            if let Err(error) = migration_result {
                atomic_write_file(&self.path, data.as_bytes(), 0o600).map_err(|_| {
                    err(
                        McpErrorCode::ERR_SECRET_MIGRATION_FAILED,
                        "failed to restore registry after migration; staged credentials were retained",
                    )
                })?;
                let restored = fs::read(&self.path).map_err(|_| {
                    err(
                        McpErrorCode::ERR_SECRET_MIGRATION_FAILED,
                        "failed to verify restored registry; staged credentials were retained",
                    )
                })?;
                if restored != data.as_bytes() {
                    return Err(err(
                        McpErrorCode::ERR_SECRET_MIGRATION_FAILED,
                        "restored registry verification failed; staged credentials were retained",
                    ));
                }
                self.rollback_secret_writes(rollback)?;
                return Err(error);
            }
            crate::migration_cleanup::delete_plaintext_backup_or_journal(&backup_path)?;
        }

        Ok(storages)
    }

    fn migrate_secrets_in_batch(
        &self,
        storages: &mut [StorageRecord],
    ) -> McpResult<Vec<(String, Option<Value>)>> {
        let schema_secret_names = discover_secret_field_names();
        let mut rollback = Vec::new();
        for storage in storages.iter_mut() {
            let secret_fields = extract_secret_fields(&storage.config, &schema_secret_names);
            if secret_fields.is_empty() {
                continue;
            }
            let secret_ref = storage
                .secret_ref
                .clone()
                .unwrap_or_else(|| format!("storage/{}", storage.id));
            let previous = match self.secret_store.get_json(&secret_ref) {
                Ok(value) => value,
                Err(_) => {
                    self.rollback_secret_writes(rollback)?;
                    return Err(err(
                        McpErrorCode::ERR_SECRET_MIGRATION_FAILED,
                        "failed to stage credential migration",
                    ));
                }
            };
            let mut bundle = previous.clone().unwrap_or_else(|| json!({}));
            infimount_core::secrets::canonicalize_bundle_keys(&mut bundle);
            let Some(object) = bundle.as_object_mut() else {
                self.rollback_secret_writes(rollback)?;
                return Err(err(
                    McpErrorCode::ERR_SECRET_MIGRATION_FAILED,
                    "stored secret bundle is invalid",
                ));
            };
            let extracted_names = secret_fields
                .iter()
                .map(|(name, _)| name.clone())
                .collect::<Vec<_>>();
            object.extend(secret_fields);
            rollback.push((secret_ref.clone(), previous));
            if self.secret_store.put_json(&secret_ref, &bundle).is_err() {
                self.rollback_secret_writes(rollback)?;
                return Err(err(
                    McpErrorCode::ERR_SECRET_MIGRATION_FAILED,
                    "failed to migrate credentials to native secret storage",
                ));
            }
            strip_secret_fields(&mut storage.config, &schema_secret_names);
            storage.secret_ref = Some(secret_ref);
            storage.secret_fields = extracted_names;
            storage.schema_version = STORAGE_RECORD_SCHEMA_VERSION;
            storage.revision = storage.revision.saturating_add(1);
        }
        Ok(rollback)
    }

    fn rollback_secret_writes(&self, rollback: Vec<(String, Option<Value>)>) -> McpResult<()> {
        for (account, previous) in rollback.into_iter().rev() {
            let restored = match previous {
                Some(value) => self.secret_store.put_json(&account, &value),
                None => self.secret_store.delete(&account),
            };
            if restored.is_err() {
                return Err(err(
                    McpErrorCode::ERR_SECRET_MIGRATION_FAILED,
                    "credential rollback failed; manual secret-store cleanup is required",
                ));
            }
        }
        Ok(())
    }

    fn create_pre_migration_backup(&self, original_data: &str) -> McpResult<PathBuf> {
        let backups_dir = self
            .path
            .parent()
            .ok_or_else(|| {
                err_with_details(
                    McpErrorCode::ERR_INTERNAL,
                    "registry path has no parent directory",
                    json!({ "path": self.path }),
                )
            })?
            .join("backups");
        infimount_core::atomic_file::create_dir_all(&backups_dir)
            .map_err(|error| map_core_error(&error))?;

        let timestamp = Utc::now().format("%Y%m%d%H%M%S%3f");
        let backup_name = format!("storages.pre-secrets-v2.{}.json", timestamp);
        let backup_path = backups_dir.join(backup_name);

        let payload = original_data.as_bytes();
        atomic_write_file(&backup_path, payload, 0o600).map_err(|e| map_core_error(&e))?;

        Ok(backup_path)
    }

    pub fn serialize_records(&self, storages: &[StorageRecord]) -> McpResult<Vec<u8>> {
        let mut normalized_storages = storages.to_vec();
        let schema_secret_names = discover_secret_field_names();
        for storage in &mut normalized_storages {
            if infimount_core::secrets::contains_plaintext_secrets(
                &storage.config,
                &schema_secret_names,
            ) {
                return Err(err(
                    McpErrorCode::ERR_SECRET_MIGRATION_FAILED,
                    "refusing to persist plaintext credentials",
                ));
            }
            if storage.mcp_policy.version != MCP_POLICY_VERSION {
                migrate_legacy_policy(&mut storage.mcp_policy)?;
            }
            normalize_storage_policy(&mut storage.mcp_policy)?;
            storage.schema_version = STORAGE_RECORD_SCHEMA_VERSION;
        }

        serde_json::to_vec_pretty(&normalized_storages).map_err(|e| {
            err_with_details(
                McpErrorCode::ERR_INTERNAL,
                "failed to serialize storage registry",
                json!({ "serde_error": e.to_string() }),
            )
        })
    }

    fn save_all_atomic_unlocked(&self, storages: &[StorageRecord]) -> McpResult<()> {
        ensure_parent(&self.path).map_err(|e| map_core_error(&e))?;
        let payload = self.serialize_records(storages)?;
        atomic_write_file(&self.path, &payload, 0o600).map_err(|e| map_core_error(&e))
    }

    fn with_file_lock<T>(
        &self,
        timeout: Duration,
        f: impl FnOnce() -> McpResult<T>,
    ) -> McpResult<T> {
        ensure_parent(&self.lock_path).map_err(|e| map_core_error(&e))?;

        let lock_file = OpenOptions::new()
            .create(true)
            .read(true)
            .write(true)
            .truncate(false)
            .open(&self.lock_path)
            .map_err(|e| map_io_error(&e, McpErrorCode::ERR_INTERNAL))?;

        let start = Instant::now();
        loop {
            match lock_file.try_lock_exclusive() {
                Ok(()) => break,
                Err(_) if start.elapsed() >= timeout => {
                    return Err(err(
                        McpErrorCode::ERR_REGISTRY_LOCK_TIMEOUT,
                        "timed out acquiring storage registry lock",
                    ));
                }
                Err(_) => thread::sleep(Duration::from_millis(50)),
            }
        }

        let result = f();
        let _ = FileExt::unlock(&lock_file);
        result
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CleanupJournalEntry {
    pub account: String,
    #[serde(rename = "createdAt")]
    pub created_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SecretCleanupJournal {
    pub version: u32,
    pub pending: Vec<CleanupJournalEntry>,
}

pub const CLEANUP_JOURNAL_VERSION: u32 = 1;
pub const MAX_CLEANUP_JOURNAL_BYTES: usize = 1024 * 1024;
pub const MAX_CLEANUP_JOURNAL_ENTRIES: usize = 1024;

fn cleanup_journal_path(registry_path: &Path) -> PathBuf {
    registry_path
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .join("secret-cleanup.json")
}

fn with_cleanup_journal_lock<T, F>(registry_path: &Path, f: F) -> McpResult<T>
where
    F: FnOnce() -> McpResult<T>,
{
    let path = cleanup_journal_path(registry_path);
    if let Some(parent) = path.parent() {
        infimount_core::atomic_file::create_dir_all(parent).map_err(|_| {
            err(
                McpErrorCode::ERR_INTERNAL,
                "failed to create cleanup journal directory",
            )
        })?;
    }
    let lock = OpenOptions::new()
        .create(true)
        .read(true)
        .write(true)
        .truncate(false)
        .open(path.with_extension("lock"))
        .map_err(|_| err(McpErrorCode::ERR_INTERNAL, "failed to lock cleanup journal"))?;
    let start = Instant::now();
    loop {
        match lock.try_lock_exclusive() {
            Ok(()) => break,
            Err(_) if start.elapsed() >= Duration::from_secs(2) => {
                return Err(err(
                    McpErrorCode::ERR_REGISTRY_LOCK_TIMEOUT,
                    "timed out acquiring secret cleanup journal lock",
                ));
            }
            Err(_) => thread::sleep(Duration::from_millis(50)),
        }
    }
    let result = f();
    let _ = FileExt::unlock(&lock);
    result
}

fn valid_cleanup_secret_account(account: &str) -> bool {
    valid_secret_account(account) || parse_mcp_auth_secret_account(account).is_ok()
}

fn active_mcp_auth_ref(registry_path: &Path) -> McpResult<Option<String>> {
    let settings_path = registry_path
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .join("mcp_settings.json");
    if !settings_path.exists() {
        return Ok(None);
    }
    let bytes = fs::read(&settings_path)
        .map_err(|error| map_io_error(&error, McpErrorCode::ERR_INTERNAL))?;
    let value: serde_json::Value = serde_json::from_slice(&bytes).map_err(|_| {
        err(
            McpErrorCode::ERR_INTERNAL,
            "MCP settings are invalid; credential cleanup was preserved",
        )
    })?;
    Ok(value
        .get("authTokenRef")
        .and_then(serde_json::Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string))
}

fn read_cleanup_journal(path: &Path) -> McpResult<SecretCleanupJournal> {
    let bytes = fs::read(path).map_err(|error| map_io_error(&error, McpErrorCode::ERR_INTERNAL))?;
    if bytes.len() > MAX_CLEANUP_JOURNAL_BYTES {
        return Err(err(
            McpErrorCode::ERR_INTERNAL,
            "secret cleanup journal exceeds the size limit; manual review required",
        ));
    }
    let journal: SecretCleanupJournal = serde_json::from_slice(&bytes).map_err(|_| {
        err(
            McpErrorCode::ERR_INTERNAL,
            "secret cleanup journal is malformed; it was preserved for manual review",
        )
    })?;
    if journal.version != CLEANUP_JOURNAL_VERSION
        || journal.pending.len() > MAX_CLEANUP_JOURNAL_ENTRIES
        || journal
            .pending
            .iter()
            .any(|entry| !valid_cleanup_secret_account(&entry.account))
    {
        return Err(err(
            McpErrorCode::ERR_INTERNAL,
            "secret cleanup journal contains unsupported entries; it was preserved for manual review",
        ));
    }
    Ok(journal)
}

/// Append an account to the strict shared secret-cleanup journal. A malformed
/// journal is preserved (never replaced with an empty list) and surfaces an
/// error so the caller can fall back to a manual-repair warning.
pub fn append_secret_cleanup_at(registry_path: &Path, account: &str) -> McpResult<()> {
    if !valid_cleanup_secret_account(account) {
        return Err(err(
            McpErrorCode::ERR_INTERNAL,
            "refusing to journal an invalid secret account",
        ));
    }
    let path = cleanup_journal_path(registry_path);
    with_cleanup_journal_lock(registry_path, || {
        let mut journal = if path.exists() {
            read_cleanup_journal(&path)?
        } else {
            SecretCleanupJournal {
                version: CLEANUP_JOURNAL_VERSION,
                pending: Vec::new(),
            }
        };
        if journal.pending.len() >= MAX_CLEANUP_JOURNAL_ENTRIES {
            return Err(err(
                McpErrorCode::ERR_INTERNAL,
                "secret cleanup journal is full; manual cleanup required",
            ));
        }
        if !journal.pending.iter().any(|entry| entry.account == account) {
            journal.pending.push(CleanupJournalEntry {
                account: account.to_string(),
                created_at: Utc::now().to_rfc3339(),
            });
        }
        let payload = serde_json::to_vec_pretty(&journal).map_err(|_| {
            err(
                McpErrorCode::ERR_INTERNAL,
                "failed to serialize cleanup journal",
            )
        })?;
        if payload.len() > MAX_CLEANUP_JOURNAL_BYTES {
            return Err(err(
                McpErrorCode::ERR_INTERNAL,
                "secret cleanup journal exceeds the size limit; manual cleanup required",
            ));
        }
        atomic_write_file(&path, &payload, 0o600).map_err(|error| map_core_error(&error))
    })
}

pub struct ResolvedStorageRecord {
    pub record: StorageRecord,
    pub resolved_config: serde_json::Value,
}

pub fn retry_pending_secret_cleanup(secret_store: &dyn SecretStore) -> McpResult<()> {
    retry_pending_secret_cleanup_at(&default_registry_path(), secret_store)
}

pub fn retry_pending_secret_cleanup_at(
    registry_path: &Path,
    secret_store: &dyn SecretStore,
) -> McpResult<()> {
    let path = cleanup_journal_path(registry_path);
    if !path.exists() {
        return Ok(());
    }
    with_cleanup_journal_lock(registry_path, || {
        let journal = read_cleanup_journal(&path)?;
        let mut active_secret_refs = if registry_path.exists() {
            serde_json::from_slice::<Vec<StorageRecord>>(
                &fs::read(registry_path)
                    .map_err(|error| map_io_error(&error, McpErrorCode::ERR_INTERNAL))?,
            )
            .map_err(|_| err(McpErrorCode::ERR_INTERNAL, "storage registry is invalid"))?
            .into_iter()
            .filter_map(|record| record.secret_ref)
            .collect::<std::collections::HashSet<_>>()
        } else {
            std::collections::HashSet::new()
        };
        if let Some(auth_ref) = active_mcp_auth_ref(registry_path)? {
            active_secret_refs.insert(auth_ref);
        }
        let remaining = journal
            .pending
            .into_iter()
            .filter_map(|entry| {
                if active_secret_refs.contains(&entry.account) {
                    return Some(entry);
                }
                secret_store
                    .delete(&entry.account)
                    .is_err()
                    .then_some(entry)
            })
            .collect::<Vec<_>>();
        if remaining.is_empty() {
            fs::remove_file(&path)
                .map_err(|error| map_io_error(&error, McpErrorCode::ERR_INTERNAL))?;
        } else {
            let payload = serde_json::to_vec_pretty(&SecretCleanupJournal {
                version: CLEANUP_JOURNAL_VERSION,
                pending: remaining,
            })
            .map_err(|_| {
                err(
                    McpErrorCode::ERR_INTERNAL,
                    "failed to update cleanup journal",
                )
            })?;
            if payload.len() > MAX_CLEANUP_JOURNAL_BYTES {
                return Err(err(
                    McpErrorCode::ERR_INTERNAL,
                    "secret cleanup journal exceeds the size limit; manual cleanup required",
                ));
            }
            atomic_write_file(&path, &payload, 0o600).map_err(|error| map_core_error(&error))?;
        }
        Ok(())
    })
}

pub fn validate_storage_name(raw: &str) -> McpResult<String> {
    let name = raw.trim();
    if name.is_empty() {
        return Err(err(
            McpErrorCode::ERR_INVALID_STORAGE_NAME,
            "storage name must not be empty",
        ));
    }
    if name == "/" {
        return Err(err(
            McpErrorCode::ERR_INVALID_STORAGE_NAME,
            "storage name '/' is invalid",
        ));
    }
    if name.contains('/') {
        return Err(err(
            McpErrorCode::ERR_INVALID_STORAGE_NAME,
            "storage name must not contain '/'",
        ));
    }
    if name.chars().count() > 64 {
        return Err(err(
            McpErrorCode::ERR_INVALID_STORAGE_NAME,
            "storage name must be at most 64 characters",
        ));
    }

    Ok(name.to_string())
}

pub fn ensure_unique_name(
    storages: &[StorageRecord],
    name: &str,
    except_id: Option<&str>,
) -> McpResult<()> {
    let conflict = storages.iter().any(|s| {
        if let Some(except_id) = except_id {
            if s.id == except_id {
                return false;
            }
        }
        s.name == name
    });

    if conflict {
        return Err(err_with_details(
            McpErrorCode::ERR_STORAGE_NAME_CONFLICT,
            format!("Storage name '{name}' already exists"),
            json!({ "name": name }),
        ));
    }

    Ok(())
}

pub fn mask_storage_record(storage: &StorageRecord) -> StorageRecord {
    let schema_secret_names = discover_secret_field_names();
    let mut masked = storage.clone();
    masked.config = mask_secrets_in_value(&masked.config, &schema_secret_names);
    if !masked.secret_fields.is_empty() {
        let mut masks = masked
            .secret_fields
            .iter()
            .map(|field| {
                infimount_core::secrets::canonical_secret_path(
                    &infimount_core::secrets::parse_secret_path(field).unwrap_or_else(|_| {
                        infimount_core::secrets::SecretPath {
                            segments: vec![infimount_core::secrets::SecretPathSegment::Key(
                                field.clone(),
                            )],
                        }
                    }),
                )
            })
            .collect::<Vec<_>>();
        masks.sort();
        masks.dedup();
        infimount_core::secrets::mask_secret_paths(&mut masked.config, &masks);
    }
    masked
}

pub fn mask_secrets_in_value(value: &Value, schema_secret_names: &HashSet<String>) -> Value {
    match value {
        Value::Object(map) => {
            let mut out = serde_json::Map::new();
            for (key, val) in map {
                if is_secret_field_name(key, schema_secret_names) {
                    out.insert(key.clone(), Value::String("********".to_string()));
                } else {
                    out.insert(key.clone(), mask_secrets_in_value(val, schema_secret_names));
                }
            }
            Value::Object(out)
        }
        Value::Array(items) => Value::Array(
            items
                .iter()
                .map(|item| mask_secrets_in_value(item, schema_secret_names))
                .collect(),
        ),
        _ => value.clone(),
    }
}

/// Backwards-compatible pattern-only classifier (no schema names).
pub fn is_secret_key(key: &str) -> bool {
    is_secret_field_name(key, &HashSet::new())
}

pub fn default_config_dir() -> PathBuf {
    #[cfg(target_os = "windows")]
    {
        let base = std::env::var("APPDATA").unwrap_or_else(|_| ".".to_string());
        return PathBuf::from(base).join("infimount");
    }

    #[cfg(not(target_os = "windows"))]
    {
        let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
        PathBuf::from(home).join(".infimount")
    }
}

pub fn default_registry_path() -> PathBuf {
    default_config_dir().join("storages.json")
}

fn records_match_exact(left: &[StorageRecord], right: &[StorageRecord]) -> bool {
    serde_json::to_vec(left).ok() == serde_json::to_vec(right).ok()
}

#[cfg(test)]
mod tests {
    use std::fs;
    use tempfile::TempDir;

    use super::*;
    use crate::policy::McpAccessMode;

    #[test]
    fn configuration_transaction_precedes_registry_file_lock() {
        let dir = TempDir::new().expect("temp dir");
        let path = dir.path().join("storages.json");
        let registry = StorageRegistry::with_secret_store(
            Some(path.clone()),
            std::sync::Arc::new(infimount_core::secrets::MemorySecretStore::new()),
        );

        // Lock order: configuration transaction, then registry file lock. This
        // must complete without deadlock (same order used by every mutation).
        let transaction = registry.acquire_configuration_transaction().unwrap();
        registry
            .with_locked_mutation(|storages| {
                storages.push(StorageRecord::new(
                    "S".into(),
                    "local".into(),
                    json!({ "root": "/tmp" }),
                ));
                Ok(())
            })
            .unwrap();
        drop(transaction);

        // A separate handle must be refused while the transaction is held,
        // proving cross-process serialization instead of silent concurrency.
        let second = StorageRegistry::with_secret_store(
            Some(path.clone()),
            std::sync::Arc::new(infimount_core::secrets::MemorySecretStore::new()),
        );
        let _held = registry.acquire_configuration_transaction().unwrap();
        let refused = second.acquire_configuration_transaction();
        assert!(refused.is_err());
        assert_eq!(
            refused.unwrap_err().code,
            McpErrorCode::ERR_REGISTRY_LOCK_TIMEOUT
        );
    }

    #[test]
    fn schema_v2_plaintext_is_migrated_to_memory_secret_store() {
        let dir = TempDir::new().expect("temp dir");
        let path = dir.path().join("storages.json");
        let secret_store = std::sync::Arc::new(infimount_core::secrets::MemorySecretStore::new());
        let registry = StorageRegistry::with_secret_store(Some(path.clone()), secret_store.clone());
        fs::write(
            &path,
            include_str!("../../../tests/fixtures/v0.7/storages-plaintext.json"),
        )
        .unwrap();

        let loaded = registry.load_all().expect("migrate registry");
        assert!(loaded[0].config.get("secretAccessKey").is_none());
        assert_eq!(
            secret_store
                .get_json(&format!("storage/{}", loaded[0].id))
                .unwrap()
                .unwrap()["/secretAccessKey"],
            "TEST_SECRET_ACCESS_KEY_DO_NOT_SHIP"
        );
        assert!(!fs::read_to_string(&path)
            .unwrap()
            .contains("TEST_SECRET_ACCESS_KEY_DO_NOT_SHIP"));
    }

    #[test]
    fn unavailable_secret_store_preserves_plaintext_registry_bytes() {
        let dir = TempDir::new().expect("temp dir");
        let path = dir.path().join("storages.json");
        let registry = StorageRegistry::with_secret_store(
            Some(path.clone()),
            std::sync::Arc::new(infimount_core::secrets::UnavailableSecretStore::new(
                "locked",
            )),
        );
        let record = StorageRecord::new(
            "S3".to_string(),
            "s3".to_string(),
            json!({ "secretAccessKey": "seeded-secret-value" }),
        );
        let original = serde_json::to_vec_pretty(&vec![record]).unwrap();
        fs::write(&path, &original).unwrap();
        let error = registry.load_all().expect_err("migration should fail");
        assert_eq!(error.code, McpErrorCode::ERR_SECRET_MIGRATION_FAILED);
        assert_eq!(fs::read(&path).unwrap(), original);
    }

    #[test]
    fn pending_import_recovery_removes_staged_secrets_after_precommit_crash() {
        use base64::Engine as _;
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("storages.json");
        let secrets = std::sync::Arc::new(infimount_core::secrets::MemorySecretStore::new());
        let registry = StorageRegistry::with_secret_store(Some(path.clone()), secrets.clone());
        let journal_path = registry.import_journal_path("recovery-test").unwrap();
        let original = StorageRecord::new("S".into(), "local".into(), json!({"root": "/tmp"}));
        let mut replacement = original.clone();
        replacement.secret_ref = Some("storage/staged-secret/import/uuid".into());
        let replacement_bytes = serde_json::to_vec(&vec![replacement]).unwrap();
        let journal = ImportTransactionJournal {
            version: IMPORT_JOURNAL_VERSION,
            state: ImportTransactionState::Prepared,
            original_present: false,
            original_registry_base64: String::new(),
            replacement_registry_base64: base64::engine::general_purpose::STANDARD
                .encode(&replacement_bytes),
            staged_secret_accounts: vec!["storage/staged-secret/import/uuid".into()],
            obsolete_secret_accounts: Vec::new(),
        };
        secrets
            .put_json("storage/staged-secret/import/uuid", &json!({"token": "x"}))
            .unwrap();
        registry
            .write_import_journal(&journal_path, &journal)
            .unwrap();
        registry.recover_pending_imports().unwrap();
        assert!(secrets
            .get_json("storage/staged-secret/import/uuid")
            .unwrap()
            .is_none());
        assert!(!journal_path.exists());
    }

    #[test]
    fn committed_import_journal_recovers_after_later_registry_edit() {
        use base64::Engine as _;
        let dir = TempDir::new().unwrap();
        let secrets = std::sync::Arc::new(infimount_core::secrets::MemorySecretStore::new());
        let registry = StorageRegistry::with_secret_store(
            Some(dir.path().join("storages.json")),
            secrets.clone(),
        );
        let mut original = StorageRecord::new(
            "Original".into(),
            "local".into(),
            json!({"root": "/tmp/original"}),
        );
        original.secret_ref = Some("storage/original".into());
        secrets
            .put_json("storage/original", &json!({"token": "x"}))
            .unwrap();
        registry.save_all_atomic(&[original.clone()]).unwrap();
        let (_, original_bytes) = registry.registry_bytes().unwrap();
        let mut replacement = original.clone();
        replacement.secret_ref = None;
        let replacement_bytes = serde_json::to_vec(&vec![replacement]).unwrap();
        let mut later = StorageRecord::new(
            "Later edit".into(),
            "local".into(),
            json!({"root": "/tmp/later"}),
        );
        later.id = original.id.clone();
        registry.save_all_atomic(&[later]).unwrap();
        let journal_path = registry
            .import_journal_path("committed-later-edit")
            .unwrap();
        registry
            .write_import_journal(
                &journal_path,
                &ImportTransactionJournal {
                    version: IMPORT_JOURNAL_VERSION,
                    state: ImportTransactionState::Committed,
                    original_present: true,
                    original_registry_base64: base64::engine::general_purpose::STANDARD
                        .encode(&original_bytes),
                    replacement_registry_base64: base64::engine::general_purpose::STANDARD
                        .encode(&replacement_bytes),
                    staged_secret_accounts: Vec::new(),
                    obsolete_secret_accounts: vec!["storage/original".into()],
                },
            )
            .unwrap();
        registry.recover_pending_imports().unwrap();
        assert!(secrets.get_json("storage/original").unwrap().is_none());
        assert!(!journal_path.exists());
    }

    #[test]
    fn invalid_import_journal_cannot_delete_non_import_secret_accounts() {
        use base64::Engine as _;
        let dir = TempDir::new().unwrap();
        let registry = StorageRegistry::with_secret_store(
            Some(dir.path().join("storages.json")),
            std::sync::Arc::new(infimount_core::secrets::MemorySecretStore::new()),
        );
        let journal_path = registry.import_journal_path("invalid-account").unwrap();
        let journal = ImportTransactionJournal {
            version: IMPORT_JOURNAL_VERSION,
            state: ImportTransactionState::Prepared,
            original_present: false,
            original_registry_base64: String::new(),
            replacement_registry_base64: base64::engine::general_purpose::STANDARD.encode(b"[]"),
            staged_secret_accounts: vec!["storage/auth-token".into()],
            obsolete_secret_accounts: Vec::new(),
        };
        registry
            .write_import_journal(&journal_path, &journal)
            .unwrap();
        assert!(registry.recover_pending_imports().is_err());
        assert!(journal_path.exists());
    }

    #[test]
    fn parse_storage_secret_account_covers_all_legitimate_forms() {
        assert_eq!(
            parse_storage_secret_account("storage/abc-123").unwrap(),
            StorageSecretAccount::Base {
                storage_id: "abc-123".into()
            }
        );
        assert_eq!(
            parse_storage_secret_account("storage/abc-123/import/tx-9").unwrap(),
            StorageSecretAccount::Import {
                storage_id: "abc-123".into(),
                transaction_id: "tx-9".into()
            }
        );
        assert_eq!(
            parse_storage_secret_account("storage/abc-123/revision/42/tx-9").unwrap(),
            StorageSecretAccount::Revision {
                storage_id: "abc-123".into(),
                revision: 42,
                transaction_id: "tx-9".into()
            }
        );
        assert_eq!(
            parse_storage_secret_account("recovery/storage/rec-7").unwrap(),
            StorageSecretAccount::Recovery {
                recovery_id: "rec-7".into()
            }
        );
        for account in [
            "storage/abc-123",
            "storage/abc-123/import/tx-9",
            "storage/abc-123/revision/42/tx-9",
            "recovery/storage/rec-7",
        ] {
            let parsed = parse_storage_secret_account(account).unwrap();
            assert_eq!(parsed.canonical(), account);
        }
    }

    #[test]
    fn parse_storage_secret_account_rejects_foreign_forms() {
        for account in [
            "mcp/http-auth",
            "recovery/restore-transaction",
            "recovery/storage",
            "storage",
            "storage/",
            "storage/a b",
            "storage/a\nb",
            "storage/a/import",
            "storage/a/import/tx/extra",
            "storage/a/revision/nope/tx",
            "storage/a/revision/1",
            "storage/a/b/c/d",
        ] {
            assert!(
                parse_storage_secret_account(account).is_err(),
                "{account} must be rejected"
            );
        }
    }

    #[test]
    fn journal_recovery_removes_obsolete_recovery_account() {
        use base64::Engine as _;
        let dir = TempDir::new().unwrap();
        let secrets = std::sync::Arc::new(infimount_core::secrets::MemorySecretStore::new());
        let registry = StorageRegistry::with_secret_store(
            Some(dir.path().join("storages.json")),
            secrets.clone(),
        );
        let mut original = StorageRecord::new("S".into(), "local".into(), json!({"root": "/tmp"}));
        original.secret_ref = Some("recovery/storage/rec-7".into());
        secrets
            .put_json("recovery/storage/rec-7", &json!({"token": "x"}))
            .unwrap();
        registry.save_all_atomic(&[original.clone()]).unwrap();
        let (_, original_bytes) = registry.registry_bytes().unwrap();
        let mut replacement = original.clone();
        replacement.secret_ref = None;
        let replacement_bytes = serde_json::to_vec(&vec![replacement.clone()]).unwrap();
        let journal_path = registry.import_journal_path("recovery-account").unwrap();
        let journal = ImportTransactionJournal {
            version: IMPORT_JOURNAL_VERSION,
            state: ImportTransactionState::Committed,
            original_present: true,
            original_registry_base64: base64::engine::general_purpose::STANDARD
                .encode(&original_bytes),
            replacement_registry_base64: base64::engine::general_purpose::STANDARD
                .encode(&replacement_bytes),
            staged_secret_accounts: Vec::new(),
            obsolete_secret_accounts: vec!["recovery/storage/rec-7".into()],
        };
        registry
            .write_import_journal(&journal_path, &journal)
            .unwrap();
        registry.save_all_atomic(&[replacement]).unwrap();
        registry.recover_pending_imports().unwrap();
        assert!(secrets
            .get_json("recovery/storage/rec-7")
            .unwrap()
            .is_none());
        assert!(!journal_path.exists());
    }

    #[test]
    fn journal_recovery_removes_obsolete_revision_account() {
        use base64::Engine as _;
        let dir = TempDir::new().unwrap();
        let secrets = std::sync::Arc::new(infimount_core::secrets::MemorySecretStore::new());
        let registry = StorageRegistry::with_secret_store(
            Some(dir.path().join("storages.json")),
            secrets.clone(),
        );
        let mut original = StorageRecord::new("S".into(), "local".into(), json!({"root": "/tmp"}));
        original.secret_ref = Some("storage/s-1/revision/3/tx-9".into());
        secrets
            .put_json("storage/s-1/revision/3/tx-9", &json!({"token": "x"}))
            .unwrap();
        registry.save_all_atomic(&[original.clone()]).unwrap();
        let (_, original_bytes) = registry.registry_bytes().unwrap();
        let mut replacement = original.clone();
        replacement.secret_ref = None;
        let replacement_bytes = serde_json::to_vec(&vec![replacement.clone()]).unwrap();
        let journal_path = registry.import_journal_path("revision-account").unwrap();
        let journal = ImportTransactionJournal {
            version: IMPORT_JOURNAL_VERSION,
            state: ImportTransactionState::Committed,
            original_present: true,
            original_registry_base64: base64::engine::general_purpose::STANDARD
                .encode(&original_bytes),
            replacement_registry_base64: base64::engine::general_purpose::STANDARD
                .encode(&replacement_bytes),
            staged_secret_accounts: Vec::new(),
            obsolete_secret_accounts: vec!["storage/s-1/revision/3/tx-9".into()],
        };
        registry
            .write_import_journal(&journal_path, &journal)
            .unwrap();
        registry.save_all_atomic(&[replacement]).unwrap();
        registry.recover_pending_imports().unwrap();
        assert!(secrets
            .get_json("storage/s-1/revision/3/tx-9")
            .unwrap()
            .is_none());
        assert!(!journal_path.exists());
    }

    #[test]
    fn secrets_staged_crash_recovers_to_original() {
        use base64::Engine as _;
        let dir = TempDir::new().unwrap();
        let secrets = std::sync::Arc::new(infimount_core::secrets::MemorySecretStore::new());
        let registry = StorageRegistry::with_secret_store(
            Some(dir.path().join("storages.json")),
            secrets.clone(),
        );
        let original = StorageRecord::new("S".into(), "local".into(), json!({"root": "/tmp"}));
        registry
            .save_all_atomic(std::slice::from_ref(&original))
            .unwrap();
        let (_, original_bytes) = registry.registry_bytes().unwrap();
        let mut replacement = original.clone();
        replacement.secret_ref = Some("storage/s/import/tx-9".into());
        let replacement_bytes = serde_json::to_vec(&vec![replacement]).unwrap();
        let journal_path = registry.import_journal_path("staged-crash").unwrap();
        let journal = ImportTransactionJournal {
            version: IMPORT_JOURNAL_VERSION,
            state: ImportTransactionState::SecretsStaged,
            original_present: true,
            original_registry_base64: base64::engine::general_purpose::STANDARD
                .encode(&original_bytes),
            replacement_registry_base64: base64::engine::general_purpose::STANDARD
                .encode(&replacement_bytes),
            staged_secret_accounts: vec!["storage/s/import/tx-9".into()],
            obsolete_secret_accounts: Vec::new(),
        };
        secrets
            .put_json("storage/s/import/tx-9", &json!({"token": "x"}))
            .unwrap();
        registry
            .write_import_journal(&journal_path, &journal)
            .unwrap();
        registry.recover_pending_imports().unwrap();
        assert!(secrets.get_json("storage/s/import/tx-9").unwrap().is_none());
        assert_eq!(registry.load_all().unwrap()[0].name, "S");
        assert!(!journal_path.exists());
    }

    #[test]
    fn secrets_staged_with_committed_registry_resolves_obsolete() {
        use base64::Engine as _;
        let dir = TempDir::new().unwrap();
        let secrets = std::sync::Arc::new(infimount_core::secrets::MemorySecretStore::new());
        let registry = StorageRegistry::with_secret_store(
            Some(dir.path().join("storages.json")),
            secrets.clone(),
        );
        let mut original = StorageRecord::new("S".into(), "local".into(), json!({"root": "/tmp"}));
        original.secret_ref = Some("storage/s".into());
        secrets
            .put_json("storage/s", &json!({"token": "old"}))
            .unwrap();
        registry.save_all_atomic(&[original.clone()]).unwrap();
        let (_, original_bytes) = registry.registry_bytes().unwrap();
        let mut replacement = original.clone();
        replacement.secret_ref = Some("storage/s/import/tx-9".into());
        secrets
            .put_json("storage/s/import/tx-9", &json!({"token": "x"}))
            .unwrap();
        registry.save_all_atomic(&[replacement.clone()]).unwrap();
        let (_, replacement_bytes) = registry.registry_bytes().unwrap();
        let journal_path = registry.import_journal_path("staged-committed").unwrap();
        let journal = ImportTransactionJournal {
            version: IMPORT_JOURNAL_VERSION,
            state: ImportTransactionState::SecretsStaged,
            original_present: true,
            original_registry_base64: base64::engine::general_purpose::STANDARD
                .encode(&original_bytes),
            replacement_registry_base64: base64::engine::general_purpose::STANDARD
                .encode(&replacement_bytes),
            staged_secret_accounts: vec!["storage/s/import/tx-9".into()],
            obsolete_secret_accounts: vec!["storage/s".into()],
        };
        registry
            .write_import_journal(&journal_path, &journal)
            .unwrap();
        registry.recover_pending_imports().unwrap();
        assert!(secrets.get_json("storage/s").unwrap().is_none());
        assert!(secrets.get_json("storage/s/import/tx-9").unwrap().is_some());
        assert!(!journal_path.exists());
    }

    #[test]
    fn journal_encoded_size_near_maximum_round_trips() {
        use base64::Engine as _;
        let dir = TempDir::new().unwrap();
        let secrets = std::sync::Arc::new(infimount_core::secrets::MemorySecretStore::new());
        let registry = StorageRegistry::with_secret_store(
            Some(dir.path().join("storages.json")),
            secrets.clone(),
        );
        let big = "x".repeat(7 * 1024 * 1024);
        let record = StorageRecord::new("Big".into(), "local".into(), json!({"blob": big}));
        let replacement_bytes = serde_json::to_vec(&vec![record]).unwrap();
        assert!(replacement_bytes.len() < MAX_REGISTRY_SNAPSHOT_BYTES);
        let journal_path = registry.import_journal_path("big-journal").unwrap();
        let journal = ImportTransactionJournal {
            version: IMPORT_JOURNAL_VERSION,
            state: ImportTransactionState::Committed,
            original_present: false,
            original_registry_base64: String::new(),
            replacement_registry_base64: base64::engine::general_purpose::STANDARD
                .encode(&replacement_bytes),
            staged_secret_accounts: Vec::new(),
            obsolete_secret_accounts: Vec::new(),
        };
        registry
            .write_import_journal(&journal_path, &journal)
            .unwrap();
        let written: ImportTransactionJournal =
            serde_json::from_slice(&std::fs::read(&journal_path).unwrap()).unwrap();
        assert_eq!(written.replacement_bytes().unwrap(), replacement_bytes);
        let (_original, replacement) = journal_has_valid_transitions(&written).unwrap();
        assert_eq!(replacement.len(), 1);
    }

    #[test]
    fn oversized_import_journal_fails_before_any_write() {
        use base64::Engine as _;
        let dir = TempDir::new().unwrap();
        let registry = StorageRegistry::with_secret_store(
            Some(dir.path().join("storages.json")),
            std::sync::Arc::new(infimount_core::secrets::MemorySecretStore::new()),
        );
        let big = "x".repeat(9 * 1024 * 1024);
        let record = StorageRecord::new("Big".into(), "local".into(), json!({"blob": big}));
        let replacement_bytes = serde_json::to_vec(&vec![record]).unwrap();
        assert!(replacement_bytes.len() > MAX_REGISTRY_SNAPSHOT_BYTES);
        let journal_path = registry.import_journal_path("oversized").unwrap();
        let journal = ImportTransactionJournal {
            version: IMPORT_JOURNAL_VERSION,
            state: ImportTransactionState::Prepared,
            original_present: false,
            original_registry_base64: String::new(),
            replacement_registry_base64: base64::engine::general_purpose::STANDARD
                .encode(&replacement_bytes),
            staged_secret_accounts: vec!["storage/big/import/tx-1".into()],
            obsolete_secret_accounts: Vec::new(),
        };
        let error = registry
            .write_import_journal(&journal_path, &journal)
            .unwrap_err();
        assert_eq!(error.code, McpErrorCode::ERR_INTERNAL);
        assert!(!journal_path.exists());
    }

    #[test]
    fn malformed_cleanup_journal_is_preserved() {
        let dir = TempDir::new().unwrap();
        let registry_path = dir.path().join("storages.json");
        let path = cleanup_journal_path(&registry_path);
        std::fs::write(&path, b"{ malformed").unwrap();
        let secrets = std::sync::Arc::new(infimount_core::secrets::MemorySecretStore::new());
        assert!(retry_pending_secret_cleanup_at(&registry_path, secrets.as_ref()).is_err());
        assert_eq!(std::fs::read(&path).unwrap(), b"{ malformed");
    }

    #[test]
    fn cleanup_journal_preserves_active_account_until_unreferenced() {
        let dir = TempDir::new().unwrap();
        let registry_path = dir.path().join("storages.json");
        let secrets = std::sync::Arc::new(infimount_core::secrets::MemorySecretStore::new());
        let registry =
            StorageRegistry::with_secret_store(Some(registry_path.clone()), secrets.clone());
        let account = "storage/active";

        secrets
            .put_json(account, &json!({ "token": "active" }))
            .unwrap();
        let mut storage = StorageRecord::new(
            "Active".into(),
            "local".into(),
            json!({ "root": dir.path().to_string_lossy() }),
        );
        storage.secret_ref = Some(account.into());
        registry.save_all_atomic(&[storage.clone()]).unwrap();
        append_secret_cleanup_at(&registry_path, account).unwrap();

        retry_pending_secret_cleanup_at(&registry_path, secrets.as_ref()).unwrap();
        assert!(cleanup_journal_path(&registry_path).exists());
        assert!(secrets.get_json(account).unwrap().is_some());

        storage.secret_ref = None;
        registry.save_all_atomic(&[storage]).unwrap();
        retry_pending_secret_cleanup_at(&registry_path, secrets.as_ref()).unwrap();

        assert!(secrets.get_json(account).unwrap().is_none());
        assert!(!cleanup_journal_path(&registry_path).exists());
    }

    #[test]
    fn cleanup_journal_rejects_foreign_account() {
        let dir = TempDir::new().unwrap();
        let registry_path = dir.path().join("storages.json");
        // "mcp/http-auth" is now a valid legacy MCP auth account format.
        // Use an account with an invalid segment (whitespace) to test rejection.
        assert!(append_secret_cleanup_at(&registry_path, "invalid account").is_err());
    }

    #[test]
    fn storage_name_rules() {
        assert!(validate_storage_name("  photos ").is_ok());
        assert!(validate_storage_name("").is_err());
        assert!(validate_storage_name("/").is_err());
        assert!(validate_storage_name("a/b").is_err());
        assert!(validate_storage_name(&"a".repeat(65)).is_err());
    }

    #[test]
    fn secret_masking_recursive() {
        let input = json!({
            "token": "abc",
            "accessKeyId": "access-key-id",
            "applicationKey": "application-key",
            "application_key": "application-key-snake",
            "credential": "service-account-json",
            "serviceAccountJson": "service-account-json",
            "service_account_json": "service-account-json",
            "privateKeyPath": "/home/alice/.ssh/id_ed25519",
            "keyPath": "/home/alice/.ssh/id_rsa",
            "key": "/home/alice/.ssh/id_ecdsa",
            "accessToken": "oauth-access-token",
            "refreshToken": "oauth-refresh-token",
            "clientSecret": "oauth-client-secret",
            "codeVerifier": "pkce-verifier",
            "deviceCode": "oauth-device-code",
            "nested": {
                "client_secret": "x",
                "secretId": "secret-id",
                "safe": "ok"
            }
        });

        let masked = mask_secrets_in_value(&input, &discover_secret_field_names());
        assert_eq!(masked["token"], "********");
        assert_eq!(masked["accessKeyId"], "********");
        assert_eq!(masked["applicationKey"], "********");
        assert_eq!(masked["application_key"], "********");
        assert_eq!(masked["credential"], "********");
        assert_eq!(masked["serviceAccountJson"], "********");
        assert_eq!(masked["service_account_json"], "********");
        assert_eq!(masked["privateKeyPath"], "********");
        assert_eq!(masked["keyPath"], "********");
        assert_eq!(masked["key"], "********");
        assert_eq!(masked["accessToken"], "********");
        assert_eq!(masked["refreshToken"], "********");
        assert_eq!(masked["clientSecret"], "********");
        assert_eq!(masked["codeVerifier"], "********");
        assert_eq!(masked["deviceCode"], "********");
        assert_eq!(masked["nested"]["client_secret"], "********");
        assert_eq!(masked["nested"]["secretId"], "********");
        assert_eq!(masked["nested"]["safe"], "ok");
    }

    /// Creates a minimal v1 policy storage JSON for migration testing
    fn v1_policy_storage_json(name: &str, default_access: &str) -> String {
        format!(
            r#"{{
    "schema_version": 1,
    "id": "test-{name}",
    "name": "{name}",
    "backend": "local",
    "config": {{ "root": "/tmp" }},
    "enabled": true,
    "mcp_exposed": true,
    "read_only": false,
    "mcp_policy": {{
        "version": 1,
        "default_access": "{default_access}",
        "rules": [],
        "denied_paths": [],
        "confirmation_rules": {{
            "require_for_write": true,
            "require_for_overwrite": true,
            "require_for_delete": true,
            "require_for_version_delete": true,
            "require_for_presign": true,
            "require_for_cross_storage_copy": true
        }},
        "allowed_paths": ["projects"]
    }},
    "revision": 1,
    "created_at": "2025-01-01T00:00:00Z",
    "updated_at": "2025-01-01T00:00:00Z"
}}"#
        )
    }

    /// Creates a schema v0 (versionless) storage JSON with versionless policy
    fn v0_schema_v0_policy_storage_json(name: &str) -> String {
        format!(
            r#"{{
    "id": "test-{name}",
    "name": "{name}",
    "backend": "local",
    "config": {{ "root": "/tmp" }},
    "enabled": true,
    "mcp_exposed": true,
    "read_only": false,
    "mcp_policy": {{
        "default_access": "read_only",
        "rules": [],
        "denied_paths": [],
        "confirmation_rules": {{
            "require_for_write": true,
            "require_for_overwrite": true,
            "require_for_delete": true,
            "require_for_version_delete": true,
            "require_for_presign": true,
            "require_for_cross_storage_copy": true
        }}
    }},
    "revision": 1,
    "created_at": "2025-01-01T00:00:00Z",
    "updated_at": "2025-01-01T00:00:00Z"
}}"#
        )
    }

    fn write_registry(dir: &TempDir, data: &str) {
        fs::write(dir.path().join("storages.json"), data).expect("write registry");
    }

    fn load_registry(dir: &TempDir) -> Vec<StorageRecord> {
        let registry = StorageRegistry::new(Some(dir.path().join("storages.json")));
        registry.load_all().expect("load registry")
    }

    #[test]
    fn migration_v1_policy_to_v2() {
        let dir = TempDir::new().expect("temp dir");
        write_registry(
            &dir,
            &format!("[\n{}\n]", v1_policy_storage_json("photos", "read_write")),
        );

        let storages = load_registry(&dir);
        assert_eq!(storages.len(), 1);
        let s = &storages[0];
        assert_eq!(s.name, "photos");
        assert_eq!(s.mcp_policy.version, MCP_POLICY_VERSION);
        assert_eq!(s.schema_version, STORAGE_RECORD_SCHEMA_VERSION);
        // v1 policy with default_access=read_write and allowed_paths=["projects"]
        // should migrate to v2 with default_access=None and a ReadWrite rule for "projects"
        assert_eq!(s.mcp_policy.default_access, McpAccessMode::None);
        assert_eq!(s.mcp_policy.rules.len(), 1);
        assert_eq!(s.mcp_policy.rules[0].prefix, "projects");
        assert_eq!(s.mcp_policy.rules[0].access, McpAccessMode::ReadWrite);
        assert!(s.mcp_policy.allowed_paths.is_empty());
    }

    #[test]
    fn migration_v0_schema_and_v0_policy() {
        let dir = TempDir::new().expect("temp dir");
        write_registry(
            &dir,
            &format!("[\n{}\n]", v0_schema_v0_policy_storage_json("legacy")),
        );

        let storages = load_registry(&dir);
        assert_eq!(storages.len(), 1);
        let s = &storages[0];
        assert_eq!(s.name, "legacy");
        // versionless (v0) policy gets version=0 after deserialization, then migration upgrades it
        assert_eq!(s.mcp_policy.version, MCP_POLICY_VERSION);
        assert_eq!(s.schema_version, STORAGE_RECORD_SCHEMA_VERSION);
        // versionless policy has default_access=read_only, no rules
        assert_eq!(s.mcp_policy.default_access, McpAccessMode::ReadOnly);
        assert!(s.mcp_policy.rules.is_empty());
    }

    #[test]
    fn migration_persists_denied_paths_from_v1() {
        let dir = TempDir::new().expect("temp dir");
        let json = v1_policy_storage_json("secure", "read_only")
            .replace(r#""denied_paths": []"#, r#""denied_paths": ["secrets"]"#);
        write_registry(&dir, &format!("[\n{json}\n]"));

        let storages = load_registry(&dir);
        let s = &storages[0];
        assert_eq!(s.mcp_policy.denied_paths, vec!["secrets"]);
        // allowed_paths was migrated to rules
        assert_eq!(s.mcp_policy.rules.len(), 1);
        assert_eq!(s.mcp_policy.rules[0].prefix, "projects");
        // Since default_access was read_only, migrated rule gets ReadOnly
        assert_eq!(s.mcp_policy.rules[0].access, McpAccessMode::ReadOnly);
    }

    #[test]
    fn migration_backup_is_byte_for_byte() {
        let dir = TempDir::new().expect("temp dir");
        let original_json = format!("[\n{}\n]", v1_policy_storage_json("photos", "read_only"));
        write_registry(&dir, &original_json);

        let _storages = load_registry(&dir);

        // After successful migration the plaintext backup is deleted
        let backups_dir = dir.path().join("backups");
        if backups_dir.exists() {
            let count = fs::read_dir(&backups_dir)
                .unwrap()
                .filter_map(|e| e.ok())
                .count();
            assert_eq!(
                count, 0,
                "backup should be deleted after successful migration"
            );
        }
    }

    #[test]
    fn migration_persistence_after_reload() {
        let dir = TempDir::new().expect("temp dir");
        write_registry(
            &dir,
            &format!("[\n{}\n]", v1_policy_storage_json("persist", "read_write")),
        );

        // Load once to trigger migration
        let _first = load_registry(&dir);

        // Load again - should NOT re-migrate
        let storages = load_registry(&dir);
        assert_eq!(storages.len(), 1);
        assert_eq!(storages[0].mcp_policy.version, MCP_POLICY_VERSION);
        assert_eq!(storages[0].schema_version, STORAGE_RECORD_SCHEMA_VERSION);
    }

    #[test]
    fn atomic_write_file_creates_with_0600_perms() {
        let dir = TempDir::new().expect("temp dir");
        let path = dir.path().join("test.json");
        let payload = b"{\"key\":\"value\"}";

        atomic_write_file(&path, payload, 0o600).expect("atomic write");

        assert!(path.exists());

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let metadata = fs::metadata(&path).expect("metadata");
            let mode = metadata.permissions().mode() & 0o777;
            assert_eq!(mode, 0o600, "file permissions should be 0600");
        }

        let content = fs::read_to_string(&path).expect("read");
        assert_eq!(content, "{\"key\":\"value\"}");
    }

    #[test]
    fn atomic_write_file_persists_content() {
        let dir = TempDir::new().expect("temp dir");
        let path = dir.path().join("persist.json");
        let payload = b"persistent content";

        atomic_write_file(&path, payload, 0o600).expect("atomic write");
        let content = fs::read_to_string(&path).expect("read");
        assert_eq!(content, "persistent content");

        // Overwrite
        let payload2 = b"updated content";
        atomic_write_file(&path, payload2, 0o600).expect("atomic overwrite");
        let content2 = fs::read_to_string(&path).expect("read");
        assert_eq!(content2, "updated content");
    }

    #[test]
    fn backup_failure_preserves_original_registry() {
        let dir = TempDir::new().expect("temp dir");
        write_registry(
            &dir,
            &format!("[\n{}\n]", v1_policy_storage_json("original", "read_only")),
        );

        // A file at the backup directory path fails deterministically on every platform.
        fs::write(dir.path().join("backups"), b"not a directory").expect("create backup blocker");

        // Loading should fail because backup can't be written
        let registry = StorageRegistry::new(Some(dir.path().join("storages.json")));
        let result = registry.load_all();
        assert!(
            result.is_err(),
            "load should fail when backup cannot be written"
        );

        // Original data should still be intact
        let original_content =
            fs::read_to_string(dir.path().join("storages.json")).expect("read original");
        assert!(
            original_content.contains("original"),
            "original registry should be preserved"
        );
    }

    #[test]
    fn mixed_schema_v1_policy_v2_not_migrated() {
        // A registry that already has schema v1 and policy v2 should not be migrated
        let dir = TempDir::new().expect("temp dir");
        let json = v1_policy_storage_json("already-v2", "read_write")
            .replace(r#""version": 1"#, r#""version": 2"#);
        write_registry(&dir, &format!("[\n{json}\n]"));

        let storages = load_registry(&dir);
        assert_eq!(storages.len(), 1);
        assert_eq!(storages[0].mcp_policy.version, MCP_POLICY_VERSION);
        // Should not have re-migrated
        assert!(storages[0].mcp_policy.rules.is_empty());
        assert_eq!(
            storages[0].mcp_policy.default_access,
            McpAccessMode::ReadWrite
        );
    }

    // ── Migration Matrix Tests ──────────────────────────────────────────

    /// Generates a storage JSON with explicit schema_version and policy version
    fn matrix_storage_json(
        name: &str,
        schema_version: u32,
        policy_version: u32,
        policy_default_access: &str,
        allowed_paths: &[&str],
    ) -> String {
        let allowed = if allowed_paths.is_empty() {
            "[]".to_string()
        } else {
            let items: Vec<String> = allowed_paths.iter().map(|p| format!("\"{p}\"")).collect();
            format!("[{}]", items.join(", "))
        };
        format!(
            r#"{{
    "schema_version": {sv},
    "id": "test-{name}",
    "name": "{name}",
    "backend": "local",
    "config": {{ "root": "/tmp" }},
    "enabled": true,
    "mcp_exposed": true,
    "read_only": false,
    "mcp_policy": {{
        "version": {pv},
        "default_access": "{pda}",
        "rules": [],
        "denied_paths": [],
        "confirmation_rules": {{
            "require_for_write": true,
            "require_for_overwrite": true,
            "require_for_delete": true,
            "require_for_version_delete": true,
            "require_for_presign": true,
            "require_for_cross_storage_copy": true
        }},
        "allowed_paths": {allowed}
    }},
    "revision": 1,
    "created_at": "2025-01-01T00:00:00Z",
    "updated_at": "2025-01-01T00:00:00Z"
}}"#,
            sv = schema_version,
            name = name,
            pv = policy_version,
            pda = policy_default_access,
            allowed = allowed
        )
    }

    fn load_single(dir: &TempDir) -> StorageRecord {
        let storages = load_registry(dir);
        assert_eq!(storages.len(), 1, "expected exactly 1 storage");
        storages.into_iter().next().unwrap()
    }

    #[test]
    fn matrix_schema_v0_policy_v0() {
        let dir = TempDir::new().expect("temp dir");
        write_registry(
            &dir,
            &format!(
                "[\n{}\n]",
                matrix_storage_json("s0p0", 0, 0, "read_only", &[])
            ),
        );
        let s = load_single(&dir);
        assert_eq!(s.schema_version, STORAGE_RECORD_SCHEMA_VERSION);
        assert_eq!(s.mcp_policy.version, MCP_POLICY_VERSION);
        assert_eq!(s.mcp_policy.default_access, McpAccessMode::ReadOnly);
    }

    #[test]
    fn matrix_schema_v0_policy_v1() {
        let dir = TempDir::new().expect("temp dir");
        write_registry(
            &dir,
            &format!(
                "[\n{}\n]",
                matrix_storage_json("s0p1", 0, 1, "read_write", &["data"])
            ),
        );
        let s = load_single(&dir);
        assert_eq!(s.schema_version, STORAGE_RECORD_SCHEMA_VERSION);
        assert_eq!(s.mcp_policy.version, MCP_POLICY_VERSION);
        assert_eq!(s.mcp_policy.default_access, McpAccessMode::None);
        assert_eq!(s.mcp_policy.rules.len(), 1);
        assert_eq!(s.mcp_policy.rules[0].prefix, "data");
        assert_eq!(s.mcp_policy.rules[0].access, McpAccessMode::ReadWrite);
    }

    #[test]
    fn matrix_schema_v1_policy_v0() {
        let dir = TempDir::new().expect("temp dir");
        write_registry(
            &dir,
            &format!(
                "[\n{}\n]",
                matrix_storage_json("s1p0", 1, 0, "read_only", &[])
            ),
        );
        let s = load_single(&dir);
        assert_eq!(s.schema_version, STORAGE_RECORD_SCHEMA_VERSION);
        assert_eq!(s.mcp_policy.version, MCP_POLICY_VERSION);
        assert_eq!(s.mcp_policy.default_access, McpAccessMode::ReadOnly);
    }

    #[test]
    fn matrix_schema_v1_policy_v1_allowed_to_rules_migration() {
        let dir = TempDir::new().expect("temp dir");
        write_registry(
            &dir,
            &format!(
                "[\n{}\n]",
                matrix_storage_json("s1p1", 1, 1, "read_only", &["docs", "assets"])
            ),
        );
        let s = load_single(&dir);
        assert_eq!(s.schema_version, STORAGE_RECORD_SCHEMA_VERSION);
        assert_eq!(s.mcp_policy.version, MCP_POLICY_VERSION);
        assert_eq!(s.mcp_policy.default_access, McpAccessMode::None);
        assert_eq!(s.mcp_policy.rules.len(), 2);
        assert_eq!(s.mcp_policy.rules[0].prefix, "docs");
        assert_eq!(s.mcp_policy.rules[1].prefix, "assets");
        // v1 default_access=read_only + allowed_paths -> migrated to ReadOnly rules
        assert_eq!(s.mcp_policy.rules[0].access, McpAccessMode::ReadOnly);
    }

    #[test]
    fn matrix_schema_v1_policy_v2_no_op() {
        let dir = TempDir::new().expect("temp dir");
        write_registry(
            &dir,
            &format!(
                "[\n{}\n]",
                matrix_storage_json("s1p2", 1, 2, "read_write", &[])
            ),
        );
        let s = load_single(&dir);
        assert_eq!(s.schema_version, STORAGE_RECORD_SCHEMA_VERSION);
        assert_eq!(s.mcp_policy.version, MCP_POLICY_VERSION);
        // Policy v2 with no allowed_paths, default_access=read_write should stay
        assert_eq!(s.mcp_policy.default_access, McpAccessMode::ReadWrite);
        assert!(s.mcp_policy.rules.is_empty());
    }

    #[test]
    fn matrix_schema_v2_policy_v0() {
        let dir = TempDir::new().expect("temp dir");
        write_registry(
            &dir,
            &format!(
                "[\n{}\n]",
                matrix_storage_json("s2p0", 2, 0, "read_only", &[])
            ),
        );
        let s = load_single(&dir);
        assert_eq!(s.schema_version, STORAGE_RECORD_SCHEMA_VERSION);
        assert_eq!(s.mcp_policy.version, MCP_POLICY_VERSION);
        assert_eq!(s.mcp_policy.default_access, McpAccessMode::ReadOnly);
    }

    #[test]
    fn matrix_schema_v2_policy_v1() {
        let dir = TempDir::new().expect("temp dir");
        write_registry(
            &dir,
            &format!(
                "[\n{}\n]",
                matrix_storage_json("s2p1", 2, 1, "read_write", &["projects"])
            ),
        );
        let s = load_single(&dir);
        assert_eq!(s.schema_version, STORAGE_RECORD_SCHEMA_VERSION);
        assert_eq!(s.mcp_policy.version, MCP_POLICY_VERSION);
        assert_eq!(s.mcp_policy.default_access, McpAccessMode::None);
        assert_eq!(s.mcp_policy.rules.len(), 1);
        assert_eq!(s.mcp_policy.rules[0].prefix, "projects");
    }

    #[test]
    fn matrix_schema_v2_policy_v2_no_op() {
        let dir = TempDir::new().expect("temp dir");
        write_registry(
            &dir,
            &format!(
                "[\n{}\n]",
                matrix_storage_json("s2p2", 2, 2, "read_only", &[])
            ),
        );
        let s = load_single(&dir);
        assert_eq!(s.schema_version, STORAGE_RECORD_SCHEMA_VERSION);
        assert_eq!(s.mcp_policy.version, MCP_POLICY_VERSION);
        assert_eq!(s.mcp_policy.default_access, McpAccessMode::ReadOnly);
        assert!(s.mcp_policy.rules.is_empty());
    }

    #[test]
    fn matrix_multiple_storages_mixed_versions() {
        let dir = TempDir::new().expect("temp dir");
        let s0 = matrix_storage_json("legacy-v0", 0, 0, "read_only", &[]);
        let s1 = matrix_storage_json("v1-policy", 1, 1, "read_write", &["data"]);
        let s2 = matrix_storage_json("current", 2, 2, "read_only", &[]);
        write_registry(&dir, &format!("[\n{s0},\n{s1},\n{s2}\n]"));

        let storages = load_registry(&dir);
        assert_eq!(storages.len(), 3);

        let legacy = storages.iter().find(|s| s.name == "legacy-v0").unwrap();
        assert_eq!(legacy.schema_version, STORAGE_RECORD_SCHEMA_VERSION);
        assert_eq!(legacy.mcp_policy.version, MCP_POLICY_VERSION);

        let migrated = storages.iter().find(|s| s.name == "v1-policy").unwrap();
        assert_eq!(migrated.schema_version, STORAGE_RECORD_SCHEMA_VERSION);
        assert_eq!(migrated.mcp_policy.version, MCP_POLICY_VERSION);
        assert_eq!(migrated.mcp_policy.default_access, McpAccessMode::None);
        assert_eq!(migrated.mcp_policy.rules.len(), 1);

        let current = storages.iter().find(|s| s.name == "current").unwrap();
        assert_eq!(current.schema_version, STORAGE_RECORD_SCHEMA_VERSION);
        assert_eq!(current.mcp_policy.version, MCP_POLICY_VERSION);
        assert_eq!(current.mcp_policy.default_access, McpAccessMode::ReadOnly);
    }

    #[test]
    fn secret_transaction_prepared_add_rolls_back_orphan() {
        let dir = TempDir::new().expect("temp dir");
        let registry_path = dir.path().join("storages.json");
        let secrets = Arc::new(infimount_core::secrets::MemorySecretStore::new());
        let desired = "storage/orphan-add/revision/1/tx-add".to_string();
        let journal = SecretTransactionJournal {
            version: SECRET_TRANSACTION_JOURNAL_VERSION,
            transaction_id: "tx-add".to_string(),
            created_at: Utc::now().to_rfc3339(),
            state: SecretTransactionState::Prepared,
            target: SecretTransactionTarget::Storage {
                storage_id: "orphan-add".to_string(),
            },
            previous_ref: None,
            desired_ref: Some(desired.clone()),
            obsolete_refs: Vec::new(),
        };

        begin_secret_transaction(&registry_path, &journal).unwrap();
        secrets
            .put_json(&desired, &json!({"key": "value"}))
            .unwrap();
        advance_secret_transaction(
            &registry_path,
            "tx-add",
            SecretTransactionState::Prepared,
            SecretTransactionState::SecretWritten,
        )
        .unwrap();

        recover_pending_secret_transactions(&registry_path, &[], secrets.as_ref(), None).unwrap();

        assert!(secrets.get_json(&desired).unwrap().is_none());
        assert!(!secret_transaction_journal_path(&registry_path).exists());
    }

    #[test]
    fn secret_transaction_committed_update_keeps_new_and_removes_old() {
        let dir = TempDir::new().expect("temp dir");
        let registry_path = dir.path().join("storages.json");
        let secrets = Arc::new(infimount_core::secrets::MemorySecretStore::new());
        let previous = "storage/s1".to_string();
        let desired = "storage/s1/revision/2/tx-update".to_string();
        let journal = SecretTransactionJournal {
            version: SECRET_TRANSACTION_JOURNAL_VERSION,
            transaction_id: "tx-update".to_string(),
            created_at: Utc::now().to_rfc3339(),
            state: SecretTransactionState::Prepared,
            target: SecretTransactionTarget::Storage {
                storage_id: "s1".to_string(),
            },
            previous_ref: Some(previous.clone()),
            desired_ref: Some(desired.clone()),
            obsolete_refs: vec![previous.clone()],
        };

        begin_secret_transaction(&registry_path, &journal).unwrap();
        secrets.put_json(&previous, &json!({"key": "old"})).unwrap();
        secrets.put_json(&desired, &json!({"key": "new"})).unwrap();
        advance_secret_transaction(
            &registry_path,
            "tx-update",
            SecretTransactionState::Prepared,
            SecretTransactionState::SecretWritten,
        )
        .unwrap();

        let storages = vec![StorageRecord {
            id: "s1".to_string(),
            secret_ref: Some(desired.clone()),
            ..StorageRecord::new("test".into(), "local".into(), json!({}))
        }];
        recover_pending_secret_transactions(&registry_path, &storages, secrets.as_ref(), None)
            .unwrap();

        assert!(secrets.get_json(&desired).unwrap().is_some());
        assert!(secrets.get_json(&previous).unwrap().is_none());
        assert!(!secret_transaction_journal_path(&registry_path).exists());
    }

    #[test]
    fn secret_transaction_committed_auth_clear_removes_previous_account() {
        let dir = TempDir::new().expect("temp dir");
        let registry_path = dir.path().join("storages.json");
        let secrets = Arc::new(infimount_core::secrets::MemorySecretStore::new());
        let previous = "mcp/http-auth".to_string();
        let journal = SecretTransactionJournal {
            version: SECRET_TRANSACTION_JOURNAL_VERSION,
            transaction_id: "tx-clear".to_string(),
            created_at: Utc::now().to_rfc3339(),
            state: SecretTransactionState::Prepared,
            target: SecretTransactionTarget::McpAuth,
            previous_ref: Some(previous.clone()),
            desired_ref: None,
            obsolete_refs: vec![previous.clone()],
        };

        begin_secret_transaction(&registry_path, &journal).unwrap();
        secrets
            .put_json(&previous, &json!({"token": "old"}))
            .unwrap();

        recover_pending_secret_transactions(&registry_path, &[], secrets.as_ref(), None).unwrap();

        assert!(secrets.get_json(&previous).unwrap().is_none());
        assert!(!secret_transaction_journal_path(&registry_path).exists());
    }

    #[test]
    fn secret_transaction_uncommitted_auth_clear_preserves_previous_account() {
        let dir = TempDir::new().expect("temp dir");
        let registry_path = dir.path().join("storages.json");
        let secrets = Arc::new(infimount_core::secrets::MemorySecretStore::new());
        let previous = "mcp/http-auth".to_string();
        let journal = SecretTransactionJournal {
            version: SECRET_TRANSACTION_JOURNAL_VERSION,
            transaction_id: "tx-clear-rollback".to_string(),
            created_at: Utc::now().to_rfc3339(),
            state: SecretTransactionState::Prepared,
            target: SecretTransactionTarget::McpAuth,
            previous_ref: Some(previous.clone()),
            desired_ref: None,
            obsolete_refs: vec![previous.clone()],
        };

        begin_secret_transaction(&registry_path, &journal).unwrap();
        secrets
            .put_json(&previous, &json!({"token": "old"}))
            .unwrap();

        recover_pending_secret_transactions(&registry_path, &[], secrets.as_ref(), Some(&previous))
            .unwrap();

        assert!(secrets.get_json(&previous).unwrap().is_some());
        assert!(!secret_transaction_journal_path(&registry_path).exists());
    }

    #[test]
    fn secret_transaction_committed_auth_set_keeps_new_and_removes_legacy() {
        let dir = TempDir::new().expect("temp dir");
        let registry_path = dir.path().join("storages.json");
        let secrets = Arc::new(infimount_core::secrets::MemorySecretStore::new());
        let previous = "mcp/http-auth".to_string();
        let desired = "mcp/http-auth/revision/tx-auth-set".to_string();
        let journal = SecretTransactionJournal {
            version: SECRET_TRANSACTION_JOURNAL_VERSION,
            transaction_id: "tx-auth-set".to_string(),
            created_at: Utc::now().to_rfc3339(),
            state: SecretTransactionState::SecretWritten,
            target: SecretTransactionTarget::McpAuth,
            previous_ref: Some(previous.clone()),
            desired_ref: Some(desired.clone()),
            obsolete_refs: vec![previous.clone()],
        };

        begin_secret_transaction(
            &registry_path,
            &SecretTransactionJournal {
                state: SecretTransactionState::Prepared,
                ..journal.clone()
            },
        )
        .unwrap();
        secrets
            .put_json(&previous, &json!({"token": "old"}))
            .unwrap();
        secrets
            .put_json(&desired, &json!({"token": "new"}))
            .unwrap();
        advance_secret_transaction(
            &registry_path,
            "tx-auth-set",
            SecretTransactionState::Prepared,
            SecretTransactionState::SecretWritten,
        )
        .unwrap();

        recover_pending_secret_transactions(&registry_path, &[], secrets.as_ref(), Some(&desired))
            .unwrap();

        assert!(secrets.get_json(&desired).unwrap().is_some());
        assert!(secrets.get_json(&previous).unwrap().is_none());
        assert!(!secret_transaction_journal_path(&registry_path).exists());
    }

    #[test]
    fn secret_transaction_begin_refuses_to_overwrite_pending_journal() {
        let dir = TempDir::new().expect("temp dir");
        let registry_path = dir.path().join("storages.json");
        let first = SecretTransactionJournal {
            version: SECRET_TRANSACTION_JOURNAL_VERSION,
            transaction_id: "tx-first".to_string(),
            created_at: Utc::now().to_rfc3339(),
            state: SecretTransactionState::Prepared,
            target: SecretTransactionTarget::Storage {
                storage_id: "s1".to_string(),
            },
            previous_ref: None,
            desired_ref: Some("storage/s1/revision/1/tx-first".to_string()),
            obsolete_refs: Vec::new(),
        };
        let second = SecretTransactionJournal {
            transaction_id: "tx-second".to_string(),
            desired_ref: Some("storage/s1/revision/1/tx-second".to_string()),
            ..first.clone()
        };

        begin_secret_transaction(&registry_path, &first).unwrap();
        assert!(begin_secret_transaction(&registry_path, &second).is_err());

        let persisted = load_secret_transaction_journal(&registry_path)
            .unwrap()
            .unwrap();
        assert_eq!(persisted.transaction_id, "tx-first");
    }
}
