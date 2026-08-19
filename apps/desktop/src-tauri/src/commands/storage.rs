#![allow(non_snake_case)]

use chrono::Utc;
use fs2::FileExt;
use infimount_core::{
    models::{ListEntriesPage, ReadFileRangeResult},
    operations,
    schema::StorageKindSchema,
    secrets, CoreError, Entry,
};
use infimount_mcp::errors::{err, err_with_details, McpError, McpErrorCode, McpResult};
use infimount_mcp::opendal_adapter::{get_capabilities, StorageBackendCapabilities};
use infimount_mcp::policy::{
    migrate_legacy_policy, normalize_policy_path, normalize_storage_policy, McpAccessMode,
    McpRuleSource, McpStoragePolicy,
};
use infimount_mcp::registry::{ensure_unique_name, validate_storage_name, StorageRecord};
use infimount_mcp::tools_storage::{
    apply_storage_import_with_validator, export_config, preview_storage_import,
    validate_storage_record, ApplyStorageImportInput, ApplyStorageImportResult, ExportConfigOutput,
    PreviewStorageImportInput, StorageImportPreview, ValidateStorageOutput,
};
use serde::Deserialize;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::fs::OpenOptions;
use std::path::PathBuf;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc, Mutex as StdMutex, OnceLock,
};
use std::time::Duration;
use tauri::State;
use tokio::io::AsyncWriteExt;
use uuid::Uuid;

use crate::state::{AppState, PendingOAuthClaim, PendingOAuthSession, SecretMutation};

const MAX_LEGACY_IPC_READ_BYTES: u64 = 8 * 1024 * 1024;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StorageDraft {
    #[serde(default)]
    pub storage_id: Option<String>,
    pub name: String,
    pub backend: String,
    pub config: Value,
    pub enabled: bool,
    pub mcp_exposed: bool,
    pub read_only: bool,
    #[serde(default)]
    pub secret_mutations: HashMap<String, SecretMutation>,
    #[serde(default)]
    pub oauth_session_id: Option<String>,
}

#[tauri::command]
pub async fn list_entries(
    state: State<'_, AppState>,
    sourceId: String,
    path: String,
) -> Result<Vec<Entry>, CoreError> {
    let op = state.operator_for_storage_id(&sourceId)?;
    operations::list_entries(&op, &path).await
}

#[tauri::command]
pub async fn list_entries_recursive(
    state: State<'_, AppState>,
    sourceId: String,
    path: String,
) -> Result<Vec<Entry>, CoreError> {
    let op = state.operator_for_storage_id(&sourceId)?;
    operations::list_entries_recursive(&op, &path).await
}

#[tauri::command]
pub async fn list_entries_page(
    state: State<'_, AppState>,
    sourceId: String,
    path: String,
    limit: Option<u32>,
    cursor: Option<String>,
    recursive: Option<bool>,
) -> Result<ListEntriesPage, CoreError> {
    let (op, revision) = state.operator_and_revision_for_storage_id(&sourceId)?;
    operations::list_entries_page(
        &op,
        &path,
        limit.unwrap_or(200),
        cursor,
        recursive.unwrap_or(false),
        revision,
    )
    .await
}

#[tauri::command]
pub async fn read_file_range(
    state: State<'_, AppState>,
    sourceId: String,
    path: String,
    offset: u64,
    maxBytes: Option<u64>,
) -> Result<ReadFileRangeResult, CoreError> {
    let op = state.operator_for_storage_id(&sourceId)?;
    operations::read_file_range(
        &op,
        &path,
        offset,
        maxBytes.unwrap_or(infimount_core::models::DEFAULT_PREVIEW_MAX),
    )
    .await
}

#[tauri::command]
pub async fn stat_entry(
    state: State<'_, AppState>,
    sourceId: String,
    path: String,
) -> Result<Entry, CoreError> {
    let op = state.operator_for_storage_id(&sourceId)?;
    operations::stat_entry(&op, &path).await
}

#[tauri::command]
pub async fn read_file(
    state: State<'_, AppState>,
    sourceId: String,
    path: String,
) -> Result<Vec<u8>, CoreError> {
    let op = state.operator_for_storage_id(&sourceId)?;
    let result = operations::read_file_range(&op, &path, 0, MAX_LEGACY_IPC_READ_BYTES).await?;
    if result.truncated || result.total_size > MAX_LEGACY_IPC_READ_BYTES {
        return Err(CoreError::Config(format!(
            "legacy IPC reads are limited to {MAX_LEGACY_IPC_READ_BYTES} bytes; use native download or ranged preview"
        )));
    }
    Ok(result.bytes)
}

#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DownloadFileResult {
    pub file_name: String,
    pub bytes: u64,
}

fn unique_download_path(directory: &std::path::Path, file_name: &str) -> PathBuf {
    let candidate = directory.join(file_name);
    if !candidate.exists() {
        return candidate;
    }
    let path = std::path::Path::new(file_name);
    let stem = path
        .file_stem()
        .and_then(|value| value.to_str())
        .unwrap_or("download");
    let extension = path.extension().and_then(|value| value.to_str());
    for index in 2_u32.. {
        let renamed = match extension {
            Some(extension) => format!("{stem} ({index}).{extension}"),
            None => format!("{stem} ({index})"),
        };
        let candidate = directory.join(renamed);
        if !candidate.exists() {
            return candidate;
        }
    }
    unreachable!("unbounded download suffix search")
}

#[tauri::command]
pub async fn download_file_to_downloads(
    state: State<'_, AppState>,
    sourceId: String,
    path: String,
) -> Result<DownloadFileResult, CoreError> {
    let file_name = std::path::Path::new(path.trim_end_matches('/'))
        .file_name()
        .and_then(|value| value.to_str())
        .filter(|value| !value.is_empty() && *value != "." && *value != "..")
        .ok_or_else(|| CoreError::Config("download path has no valid file name".to_string()))?
        .to_string();
    let directory = dirs::download_dir().ok_or_else(|| {
        CoreError::Config("the operating system Downloads directory is unavailable".to_string())
    })?;
    tokio::fs::create_dir_all(&directory).await?;
    let destination = unique_download_path(&directory, &file_name);
    let staging = directory.join(format!(".infimount-download-{}.part", Uuid::new_v4()));
    let op = state.operator_for_storage_id(&sourceId)?;
    let bytes = operations::download_file_to_local_path(&op, &path, &staging).await?;
    if let Err(error) = tokio::fs::hard_link(&staging, &destination).await {
        let _ = tokio::fs::remove_file(&staging).await;
        return Err(error.into());
    }
    tokio::fs::remove_file(&staging).await?;
    Ok(DownloadFileResult {
        file_name: destination
            .file_name()
            .and_then(|value| value.to_str())
            .unwrap_or(&file_name)
            .to_string(),
        bytes,
    })
}

#[tauri::command]
pub async fn download_file_version_to_downloads(
    state: State<'_, AppState>,
    sourceId: String,
    path: String,
    version: String,
) -> Result<DownloadFileResult, CoreError> {
    if version.trim().is_empty() {
        return Err(CoreError::Config("version must not be empty".to_string()));
    }
    let file_name = std::path::Path::new(path.trim_end_matches('/'))
        .file_name()
        .and_then(|value| value.to_str())
        .filter(|value| !value.is_empty() && *value != "." && *value != "..")
        .ok_or_else(|| CoreError::Config("download path has no valid file name".to_string()))?
        .to_string();
    let directory = dirs::download_dir().ok_or_else(|| {
        CoreError::Config("the operating system Downloads directory is unavailable".to_string())
    })?;
    tokio::fs::create_dir_all(&directory).await?;
    let destination = unique_download_path(&directory, &file_name);
    let staging = directory.join(format!(
        ".infimount-version-download-{}.part",
        Uuid::new_v4()
    ));
    let op = state.operator_for_storage_id(&sourceId)?;
    let bytes =
        operations::download_file_version_to_local_path(&op, &path, &version, &staging).await?;
    if let Err(error) = tokio::fs::hard_link(&staging, &destination).await {
        let _ = tokio::fs::remove_file(&staging).await;
        return Err(error.into());
    }
    tokio::fs::remove_file(&staging).await?;
    Ok(DownloadFileResult {
        file_name: destination
            .file_name()
            .and_then(|value| value.to_str())
            .unwrap_or(&file_name)
            .to_string(),
        bytes,
    })
}

#[tauri::command]
pub async fn write_file(
    state: State<'_, AppState>,
    sourceId: String,
    path: String,
    data: Vec<u8>,
    userMetadata: Option<HashMap<String, String>>,
) -> Result<(), CoreError> {
    let op = state.operator_for_storage_id(&sourceId)?;
    operations::write_full_with_user_metadata(&op, &path, &data, userMetadata).await
}

const MAX_UPLOAD_CHUNK_BYTES: usize = 4 * 1024 * 1024;
const MAX_STAGED_UPLOAD_BYTES: u64 = 10 * 1024 * 1024 * 1024;
const MAX_STAGED_UPLOAD_AGGREGATE_BYTES: u64 = 20 * 1024 * 1024 * 1024;
const MAX_UPLOAD_SESSIONS: usize = 32;
const STALE_UPLOAD_AGE: Duration = Duration::from_secs(24 * 60 * 60);

fn validate_upload_usage(
    session_count: usize,
    aggregate_bytes: u64,
    additional_bytes: u64,
) -> Result<(), CoreError> {
    if session_count >= MAX_UPLOAD_SESSIONS {
        return Err(CoreError::Config(format!(
            "too many staged uploads (maximum {MAX_UPLOAD_SESSIONS})"
        )));
    }
    if aggregate_bytes.saturating_add(additional_bytes) > MAX_STAGED_UPLOAD_AGGREGATE_BYTES {
        return Err(CoreError::Config(
            "staged upload aggregate exceeds 20 GiB".to_string(),
        ));
    }
    Ok(())
}

fn upload_staging_lock() -> &'static tokio::sync::Mutex<()> {
    static LOCK: OnceLock<tokio::sync::Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| tokio::sync::Mutex::new(()))
}

fn active_uploads() -> &'static StdMutex<HashMap<String, Arc<AtomicBool>>> {
    static UPLOADS: OnceLock<StdMutex<HashMap<String, Arc<AtomicBool>>>> = OnceLock::new();
    UPLOADS.get_or_init(|| StdMutex::new(HashMap::new()))
}

fn upload_cancel_flag(upload_id: &str) -> Result<Arc<AtomicBool>, CoreError> {
    active_uploads()
        .lock()
        .map_err(|_| CoreError::Config("upload session state is unavailable".to_string()))?
        .get(upload_id)
        .cloned()
        .ok_or_else(|| CoreError::Config("upload session is not active".to_string()))
}

fn upload_staging_dir() -> PathBuf {
    std::env::temp_dir().join("infimount-upload-staging")
}

fn upload_staging_path(upload_id: &str) -> Result<PathBuf, CoreError> {
    let id = Uuid::parse_str(upload_id)
        .map_err(|_| CoreError::Config("invalid upload session".to_string()))?;
    Ok(upload_staging_dir().join(format!("{id}.part")))
}

async fn cleanup_stale_uploads_and_usage() -> Result<(usize, u64), CoreError> {
    let dir = upload_staging_dir();
    let mut entries = match tokio::fs::read_dir(&dir).await {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok((0, 0)),
        Err(error) => return Err(error.into()),
    };
    let mut count = 0usize;
    let mut total = 0_u64;
    while let Some(entry) = entries.next_entry().await? {
        let path = entry.path();
        let tracked = matches!(
            path.extension().and_then(|value| value.to_str()),
            Some("part" | "uploading")
        );
        if !tracked {
            continue;
        }
        let metadata = match entry.metadata().await {
            Ok(metadata) if metadata.is_file() => metadata,
            _ => continue,
        };
        let stale = metadata
            .modified()
            .ok()
            .and_then(|modified| modified.elapsed().ok())
            .is_some_and(|age| age > STALE_UPLOAD_AGE);
        if stale {
            let _ = tokio::fs::remove_file(path).await;
            continue;
        }
        count += 1;
        total = total.saturating_add(metadata.len());
    }
    Ok((count, total))
}

#[tauri::command]
pub async fn begin_file_upload() -> Result<String, CoreError> {
    let _guard = upload_staging_lock().lock().await;
    let dir = upload_staging_dir();
    tokio::fs::create_dir_all(&dir).await?;
    let (session_count, aggregate_bytes) = cleanup_stale_uploads_and_usage().await?;
    validate_upload_usage(session_count, aggregate_bytes, 0)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        tokio::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700)).await?;
    }
    let id = Uuid::new_v4();
    let path = dir.join(format!("{id}.part"));
    tokio::fs::OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(&path)
        .await?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        tokio::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).await?;
    }
    let upload_id = id.to_string();
    active_uploads()
        .lock()
        .map_err(|_| CoreError::Config("upload session state is unavailable".to_string()))?
        .insert(upload_id.clone(), Arc::new(AtomicBool::new(false)));
    Ok(upload_id)
}

#[tauri::command]
pub async fn append_file_upload_chunk(uploadId: String, data: Vec<u8>) -> Result<(), CoreError> {
    let cancel_flag = upload_cancel_flag(&uploadId)?;
    if cancel_flag.load(Ordering::Acquire) {
        return Err(CoreError::Config("upload cancelled".to_string()));
    }
    if data.len() > MAX_UPLOAD_CHUNK_BYTES {
        return Err(CoreError::Config(format!(
            "upload chunk exceeds {MAX_UPLOAD_CHUNK_BYTES} bytes"
        )));
    }
    let _guard = upload_staging_lock().lock().await;
    let path = upload_staging_path(&uploadId)?;
    let current_len = tokio::fs::metadata(&path).await?.len();
    if current_len.saturating_add(data.len() as u64) > MAX_STAGED_UPLOAD_BYTES {
        return Err(CoreError::Config(
            "staged upload exceeds 10 GiB".to_string(),
        ));
    }
    let (_, aggregate_bytes) = cleanup_stale_uploads_and_usage().await?;
    validate_upload_usage(0, aggregate_bytes, data.len() as u64)?;
    let mut file = tokio::fs::OpenOptions::new()
        .append(true)
        .open(path)
        .await?;
    file.write_all(&data).await?;
    file.flush().await?;
    Ok(())
}

async fn finish_file_upload_inner(
    state: &AppState,
    uploadId: String,
    sourceId: String,
    targetPath: String,
) -> Result<(), CoreError> {
    let path = upload_staging_path(&uploadId)?;
    let transfer_path = path.with_extension("uploading");
    let cancel_flag = upload_cancel_flag(&uploadId)?;
    if cancel_flag.load(Ordering::Acquire) {
        let _ = tokio::fs::remove_file(&path).await;
        if let Ok(mut uploads) = active_uploads().lock() {
            uploads.remove(&uploadId);
        }
        return Err(CoreError::Config("upload cancelled".to_string()));
    }
    {
        let _guard = upload_staging_lock().lock().await;
        if let Err(error) = tokio::fs::rename(&path, &transfer_path).await {
            if let Ok(mut uploads) = active_uploads().lock() {
                uploads.remove(&uploadId);
            }
            return Err(error.into());
        }
    }
    let result = match state.operator_for_storage_id(&sourceId) {
        Ok(op) => {
            let flag = cancel_flag.clone();
            operations::upload_local_file_to_path_cancellable(
                &op,
                &transfer_path,
                &targetPath,
                move || flag.load(Ordering::Acquire),
            )
            .await
        }
        Err(error) => Err(error),
    };
    let cleanup = tokio::fs::remove_file(&transfer_path).await;
    if let Ok(mut uploads) = active_uploads().lock() {
        uploads.remove(&uploadId);
    }
    match result {
        Err(error) => Err(error),
        Ok(()) => cleanup.map_err(CoreError::from),
    }
}

#[tauri::command]
pub async fn finish_file_upload(
    state: State<'_, AppState>,
    uploadId: String,
    sourceId: String,
    targetPath: String,
) -> Result<(), CoreError> {
    finish_file_upload_inner(&state, uploadId, sourceId, targetPath).await
}

#[tauri::command]
pub async fn cancel_file_upload(uploadId: String) -> Result<(), CoreError> {
    let flag = active_uploads()
        .lock()
        .map_err(|_| CoreError::Config("upload session state is unavailable".to_string()))?
        .get(&uploadId)
        .cloned();
    if let Some(flag) = flag {
        flag.store(true, Ordering::Release);
    }

    let _guard = upload_staging_lock().lock().await;
    let path = upload_staging_path(&uploadId)?;
    let uploading = path.with_extension("uploading");
    // The active finisher owns an `.uploading` file and observes the cancellation flag. Removing
    // that file here is not portable (notably on Windows) and could race its reader.
    let finishing = tokio::fs::metadata(&uploading).await.is_ok();
    match tokio::fs::remove_file(&path).await {
        Ok(()) => {
            if !finishing {
                if let Ok(mut uploads) = active_uploads().lock() {
                    uploads.remove(&uploadId);
                }
            }
            Ok(())
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            if !finishing {
                if let Ok(mut uploads) = active_uploads().lock() {
                    uploads.remove(&uploadId);
                }
            }
            Ok(())
        }
        Err(error) => Err(error.into()),
    }
}

fn new_activation_demo_root(config_dir: &std::path::Path) -> PathBuf {
    config_dir.join(format!("activation-demo-{}", Uuid::new_v4()))
}

#[tauri::command]
pub async fn create_activation_demo_storage(
    state: State<'_, AppState>,
) -> Result<StorageRecord, McpError> {
    state.require_operational()?;
    let _lifecycle = state.lifecycle_mutation.lock().await;
    let config_dir = state
        .registry
        .path()
        .parent()
        .unwrap_or_else(|| std::path::Path::new("."));
    let existing = state.registry.load_all()?.into_iter().find(|storage| {
        let Some(root) = storage.config.get("root").and_then(Value::as_str) else {
            return false;
        };
        let root = std::path::Path::new(root);
        storage.backend == "local"
            && root.parent() == Some(config_dir)
            && root
                .file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with("activation-demo-"))
    });
    if let Some(existing) = existing {
        return Ok(existing);
    }

    // Always create a fresh, invocation-owned root. This prevents stale or user-created
    // data at a predictable path from ever being overwritten or deleted on rollback.
    let root = new_activation_demo_root(config_dir);
    tokio::fs::create_dir(&root).await.map_err(|_| {
        err(
            McpErrorCode::ERR_INTERNAL,
            "failed to reserve a private demo root",
        )
    })?;
    let root_text = root.to_string_lossy().to_string();
    let create_result: Result<(), McpError> = async {
        let workspace_dir = root.join("workspace");
        let outside_dir = root.join("outside");
        tokio::fs::create_dir(&workspace_dir).await.map_err(|_| {
            err(
                McpErrorCode::ERR_INTERNAL,
                "failed to create demo workspace",
            )
        })?;
        tokio::fs::create_dir(&outside_dir).await.map_err(|_| {
            err(
                McpErrorCode::ERR_INTERNAL,
                "failed to create demo deny fixture",
            )
        })?;
        for (path, contents) in [
            (
                workspace_dir.join("README.md"),
                b"# Infimount demo workspace\n\nThis read-only fixture is safe for activation checks.\n".as_slice(),
            ),
            (
                workspace_dir.join("sample.txt"),
                b"Infimount activation sample\n".as_slice(),
            ),
            (
                outside_dir.join("denied.txt"),
                b"This fixture must be denied by policy.\n".as_slice(),
            ),
        ] {
            infimount_core::atomic_file::atomic_write_file(
                &path,
                contents,
                infimount_core::atomic_file::FILE_MODE,
            )
            .map_err(|_| err(McpErrorCode::ERR_INTERNAL, "failed to write demo fixture"))?;
        }
        Ok(())
    }
    .await;
    if let Err(error) = create_result {
        let _ = std::fs::remove_dir_all(&root);
        return Err(error);
    }

    let mut record = StorageRecord::new(
        "Infimount Activation Demo".to_string(),
        "local".to_string(),
        json!({ "root": root_text }),
    );
    record.enabled = true;
    record.mcp_exposed = true;
    record.read_only = true;
    record.mcp_policy.denied_paths = vec!["outside".to_string()];
    let result = state.registry.with_locked_mutation(|storages| {
        ensure_unique_name(storages, &record.name, None)?;
        storages.push(record.clone());
        Ok(record.clone())
    });
    if result.is_err() {
        let _ = std::fs::remove_dir_all(&root);
    }
    result
}

#[tauri::command]
pub async fn create_directory(
    state: State<'_, AppState>,
    sourceId: String,
    path: String,
) -> Result<(), CoreError> {
    let op = state.operator_for_storage_id(&sourceId)?;
    operations::create_directory(&op, &path).await
}

#[tauri::command]
pub async fn delete_path(
    state: State<'_, AppState>,
    sourceId: String,
    path: String,
) -> Result<(), CoreError> {
    let op = state.operator_for_storage_id(&sourceId)?;
    operations::delete(&op, &path).await
}

#[tauri::command]
pub async fn upload_dropped_files(
    state: State<'_, AppState>,
    sourceId: String,
    paths: Vec<String>,
    targetDir: String,
) -> Result<(), CoreError> {
    let op = state.operator_for_storage_id(&sourceId)?;
    operations::upload_files_from_paths(&op, paths, targetDir).await
}

#[tauri::command]
pub fn list_storages(state: State<'_, AppState>) -> Result<Vec<StorageRecord>, McpError> {
    state.list_storages()
}

fn claim_oauth_session(state: &State<'_, AppState>, id: &str) -> McpResult<PendingOAuthSession> {
    match state.pending_oauth.claim(id) {
        PendingOAuthClaim::Session(session) => Ok(session),
        PendingOAuthClaim::Expired => Err(err(
            McpErrorCode::ERR_OAUTH_SESSION_EXPIRED,
            "OAuth session expired",
        )),
        PendingOAuthClaim::AlreadyUsed => Err(err(
            McpErrorCode::ERR_OAUTH_SESSION_ALREADY_USED,
            "OAuth session was already used",
        )),
        PendingOAuthClaim::InUse => Err(err(
            McpErrorCode::ERR_OAUTH_SESSION_IN_USE,
            "OAuth session is already being saved",
        )),
        PendingOAuthClaim::NotFound => Err(err(
            McpErrorCode::ERR_OAUTH_SESSION_NOT_FOUND,
            "OAuth session was not found",
        )),
    }
}

#[tauri::command]
pub async fn add_storage(
    state: State<'_, AppState>,
    storage: StorageDraft,
) -> Result<StorageRecord, McpError> {
    state.require_operational()?;
    let _lifecycle = state.lifecycle_mutation.lock().await;
    validate_storage_draft(&storage)?;
    let name = validate_storage_name(&storage.name)?;
    let schema_secret_names = secrets::discover_secret_field_names();

    // Handle OAuth session if present
    if let Some(oauth_id) = storage.oauth_session_id.clone() {
        let session = claim_oauth_session(&state, &oauth_id)?;
        let expected_backend = if session.provider == "gdrive" {
            "gdrive"
        } else {
            "onedrive"
        };
        if storage.backend != expected_backend {
            state.pending_oauth.restore(session);
            return Err(err(
                McpErrorCode::ERR_OAUTH_SESSION_NOT_FOUND,
                "OAuth provider does not match storage backend",
            ));
        }
        let public = secrets::merge_secret_config(&storage.config, &session.public_config);
        let merged = secrets::merge_secret_config(&public, &session.secret_config);
        return match add_storage_with_config(&state, name, storage, merged, &schema_secret_names) {
            Ok(result) => {
                state.pending_oauth.complete(session);
                Ok(result)
            }
            Err(error) => {
                state.pending_oauth.restore(session);
                Err(error)
            }
        };
    }

    let config = storage.config.clone();
    add_storage_with_config(&state, name, storage, config, &schema_secret_names)
}

fn add_storage_with_config(
    state: &State<'_, AppState>,
    name: String,
    storage: StorageDraft,
    config: Value,
    schema_secret_names: &[String],
) -> Result<StorageRecord, McpError> {
    let mut record = StorageRecord::new(name.clone(), storage.backend.clone(), config);
    record.enabled = storage.enabled;
    record.mcp_exposed = storage.mcp_exposed;
    record.read_only = storage.read_only;
    let extracted = secrets::extract_secret_fields(&record.config, schema_secret_names);
    secrets::strip_secret_fields(&mut record.config, schema_secret_names);
    let mut bundle = Value::Object(extracted.iter().cloned().collect());
    apply_secret_mutations_to_bundle(&mut bundle, &storage.secret_mutations)?;
    let account = format!("storage/{}", record.id);
    let has_secrets = bundle.as_object().is_some_and(|object| !object.is_empty());
    if has_secrets {
        if state.secret_store.put_json(&account, &bundle).is_err() {
            if state.secret_store.delete(&account).is_err() {
                let journal_path =
                    infimount_mcp::registry::default_config_dir().join("secret-cleanup.json");
                append_secret_cleanup(&journal_path, &account).map_err(|_| {
                    err(
                        McpErrorCode::ERR_SECRET_MIGRATION_FAILED,
                        "credential rollback failed and could not be journaled",
                    )
                })?;
                return Err(err(
                    McpErrorCode::ERR_SECRET_MIGRATION_FAILED,
                    "credential write failed; cleanup is pending",
                ));
            }
            return Err(err(
                McpErrorCode::ERR_SECRET_STORE_UNAVAILABLE,
                "failed to store credentials",
            ));
        }
        record.secret_ref = Some(account.clone());
        record.secret_fields = bundle.as_object().unwrap().keys().cloned().collect();
    }
    let result = state.registry.with_locked_mutation(|storages| {
        ensure_unique_name(storages, &name, None)?;
        storages.push(record.clone());
        Ok(record.clone())
    });
    if result.is_err() && has_secrets && state.secret_store.delete(&account).is_err() {
        let journal_path =
            infimount_mcp::registry::default_config_dir().join("secret-cleanup.json");
        append_secret_cleanup(&journal_path, &account)?;
    }
    if let Ok(created) = &result {
        let mut event = infimount_mcp::telemetry::ProductEvent::new(
            infimount_mcp::telemetry::ProductEventName::StorageAdded,
        );
        event.backend_type = Some(created.backend.clone());
        event.success = Some(true);
        let _ = state.product_events.record(event);
    }
    result
}

fn apply_secret_mutations_to_bundle(
    bundle: &mut Value,
    mutations: &HashMap<String, SecretMutation>,
) -> McpResult<()> {
    let object = bundle.as_object_mut().ok_or_else(|| {
        err(
            McpErrorCode::ERR_SECRET_STORE_UNAVAILABLE,
            "stored secret bundle is invalid",
        )
    })?;
    for (field, mutation) in mutations {
        match mutation {
            SecretMutation::Set { value } => {
                let value = value.trim();
                if value.is_empty() || value == "********" {
                    return Err(err(
                        McpErrorCode::ERR_INVALID_PATH,
                        "secret value must not be empty or masked",
                    ));
                }
                object.insert(field.clone(), Value::String(value.to_string()));
            }
            SecretMutation::Clear => {
                object.remove(field);
            }
            SecretMutation::Keep => {}
        }
    }
    Ok(())
}

#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateStorageResult {
    pub storage: StorageRecord,
    pub warning: Option<String>,
}

#[tauri::command]
pub async fn update_storage(
    state: State<'_, AppState>,
    storageId: String,
    mut storage: StorageDraft,
    confirmWorkspaceCredentialChange: bool,
) -> Result<UpdateStorageResult, McpError> {
    state.require_operational()?;
    let _lifecycle = state.lifecycle_mutation.lock().await;
    let claimed = if let Some(oauth_id) = storage.oauth_session_id.as_deref() {
        let session = claim_oauth_session(&state, oauth_id)?;
        let expected_backend = if session.provider == "gdrive" {
            "gdrive"
        } else {
            "onedrive"
        };
        if storage.backend != expected_backend {
            state.pending_oauth.restore(session);
            return Err(err(
                McpErrorCode::ERR_OAUTH_SESSION_NOT_FOUND,
                "OAuth provider does not match storage backend",
            ));
        }
        let public = secrets::merge_secret_config(&storage.config, &session.public_config);
        storage.config = secrets::merge_secret_config(&public, &session.secret_config);
        if session.secret_config.get("refreshToken").is_some() {
            storage
                .secret_mutations
                .insert("accessToken".to_string(), SecretMutation::Clear);
        }
        if session.secret_config.get("accessToken").is_some() {
            storage
                .secret_mutations
                .insert("refreshToken".to_string(), SecretMutation::Clear);
        }
        Some(session)
    } else {
        None
    };

    let credentials_changed = claimed.is_some()
        || storage
            .secret_mutations
            .values()
            .any(|mutation| !matches!(mutation, SecretMutation::Keep));
    if credentials_changed && confirmWorkspaceCredentialChange {
        validate_confirmed_credential_change(&state, &storageId, &storage, claimed.is_some())
            .await?;
    }

    let result = update_storage_with_draft(
        &state,
        storageId,
        storage,
        claimed.is_some(),
        confirmWorkspaceCredentialChange,
    );
    if let Some(session) = claimed {
        if result.is_ok() {
            state.pending_oauth.complete(session);
        } else {
            state.pending_oauth.restore(session);
        }
    }
    result.map(|(storage, warning)| UpdateStorageResult { storage, warning })
}

fn build_prospective_storage_record(
    state: &AppState,
    current: &StorageRecord,
    storage: &StorageDraft,
    name: String,
) -> Result<StorageRecord, McpError> {
    let secret_store = state.secret_store.clone();
    let schema_secret_names = secrets::discover_secret_field_names();
    let previous_account = current
        .secret_ref
        .clone()
        .unwrap_or_else(|| format!("storage/{}", current.id));
    let mut bundle = secret_store
        .get_json(&previous_account)
        .map_err(|_| {
            err(
                McpErrorCode::ERR_SECRET_STORE_UNAVAILABLE,
                "failed to access stored credentials",
            )
        })?
        .unwrap_or_else(|| json!({}));
    let extracted = secrets::extract_secret_fields(&storage.config, &schema_secret_names);
    if let Some(object) = bundle.as_object_mut() {
        object.extend(extracted);
    }
    apply_secret_mutations_to_bundle(&mut bundle, &storage.secret_mutations)?;
    let has_secrets = bundle.as_object().is_some_and(|object| !object.is_empty());
    let mut prospective = current.clone();
    prospective.name = name;
    prospective.backend = storage.backend.clone();
    prospective.enabled = storage.enabled;
    prospective.mcp_exposed = storage.mcp_exposed;
    prospective.read_only = storage.read_only;
    prospective.config = secrets::merge_secret_config(&storage.config, &bundle);
    prospective.secret_ref = has_secrets.then_some(previous_account);
    prospective.secret_fields = bundle
        .as_object()
        .map(|object| object.keys().cloned().collect())
        .unwrap_or_default();
    Ok(prospective)
}

async fn validate_confirmed_credential_change(
    state: &AppState,
    storage_id: &str,
    storage: &StorageDraft,
    oauth_session_claimed: bool,
) -> McpResult<()> {
    let credentials_changed = oauth_session_claimed
        || storage
            .secret_mutations
            .values()
            .any(|mutation| !matches!(mutation, SecretMutation::Keep));
    if !credentials_changed {
        return Ok(());
    }
    let dependent = state
        .workspaces
        .load_all()
        .map_err(|_| {
            err(
                McpErrorCode::ERR_INTERNAL,
                "failed to load workspaces while updating storage",
            )
        })?
        .into_iter()
        .filter(|workspace| workspace.storage_id == storage_id)
        .collect::<Vec<_>>();
    if dependent.is_empty() {
        return Ok(());
    }
    let current = state.find_storage_by_id(storage_id)?;
    let prospective = build_prospective_storage_record(
        state,
        &current,
        storage,
        validate_storage_name(&storage.name)?,
    )?;
    let validation = match validate_storage_record(&prospective).await {
        Ok(output) => output,
        Err(_) => {
            return Err(err_with_details(
                McpErrorCode::ERR_INVALID_STORAGE_CREDENTIALS,
                "the updated storage could not be validated; the update was rejected",
                json!({ "details": "storage validation failed" }),
            ));
        }
    };
    if !validation.valid {
        return Err(err_with_details(
            McpErrorCode::ERR_INVALID_STORAGE_CREDENTIALS,
            "the updated storage could not be validated; the update was rejected",
            json!({ "details": validation.details }),
        ));
    }
    Ok(())
}

fn update_storage_with_draft(
    state: &AppState,
    storageId: String,
    storage: StorageDraft,
    oauth_session_claimed: bool,
    confirm_workspace_credential_change: bool,
) -> Result<(StorageRecord, Option<String>), McpError> {
    validate_storage_draft(&storage)?;
    let name = validate_storage_name(&storage.name)?;
    let secret_store = state.secret_store.clone();
    let schema_secret_names = secrets::discover_secret_field_names();
    let mut previous_account = String::new();
    let mut staged_account = String::new();
    let mut previous_bundle: Option<Value> = None;
    let mut staged_secret = false;
    let dependent = state
        .workspaces
        .load_all()
        .map_err(|_| {
            err(
                McpErrorCode::ERR_INTERNAL,
                "failed to load workspaces while updating storage",
            )
        })?
        .into_iter()
        .filter(|workspace| workspace.storage_id == storageId)
        .collect::<Vec<_>>();
    let credentials_changed = oauth_session_claimed
        || storage
            .secret_mutations
            .values()
            .any(|mutation| !matches!(mutation, SecretMutation::Keep));
    let result = state.registry.with_locked_mutation(|storages| {
        let idx = storages
            .iter()
            .position(|item| item.id == storageId)
            .ok_or_else(|| {
                err_with_details(
                    McpErrorCode::ERR_STORAGE_NOT_FOUND,
                    "storage not found",
                    json!({ "storage_id": storageId }),
                )
            })?;
        ensure_unique_name(storages, &name, Some(storageId.as_str()))?;

        previous_account = storages[idx]
            .secret_ref
            .clone()
            .unwrap_or_else(|| format!("storage/{storageId}"));
        staged_account = format!(
            "storage/{storageId}/revision/{}/{}",
            storages[idx].revision.saturating_add(1),
            uuid::Uuid::new_v4()
        );
        previous_bundle = secret_store.get_json(&previous_account).map_err(|_| {
            err(
                McpErrorCode::ERR_SECRET_STORE_UNAVAILABLE,
                "failed to access stored credentials",
            )
        })?;
        if previous_bundle.is_none()
            && (storages[idx].secret_ref.is_some() || !storages[idx].secret_fields.is_empty())
        {
            return Err(err(
                McpErrorCode::ERR_SECRET_NOT_FOUND,
                "stored credentials are missing",
            ));
        }
        let mut staged_bundle = previous_bundle.clone().unwrap_or_else(|| json!({}));
        let extracted = secrets::extract_secret_fields(&storage.config, &schema_secret_names);
        if let Some(object) = staged_bundle.as_object_mut() {
            object.extend(extracted);
        }
        apply_secret_mutations_to_bundle(&mut staged_bundle, &storage.secret_mutations)?;
        let has_secrets = staged_bundle
            .as_object()
            .is_some_and(|object| !object.is_empty());
        if has_secrets {
            if secret_store
                .put_json(&staged_account, &staged_bundle)
                .is_err()
            {
                if secret_store.delete(&staged_account).is_err() {
                    let journal_path =
                        infimount_mcp::registry::default_config_dir().join("secret-cleanup.json");
                    append_secret_cleanup(&journal_path, &staged_account)?;
                }
                return Err(err(
                    McpErrorCode::ERR_SECRET_STORE_UNAVAILABLE,
                    "failed to stage updated credentials",
                ));
            }
            staged_secret = true;
        }

        let mut updated = storages[idx].clone();
        updated.name = name.clone();
        updated.backend = storage.backend.clone();
        updated.enabled = storage.enabled;
        updated.mcp_exposed = storage.mcp_exposed;
        updated.read_only = storage.read_only;
        updated.config = storage.config.clone();
        secrets::strip_secret_fields(&mut updated.config, &schema_secret_names);
        updated.secret_ref = has_secrets.then(|| staged_account.clone());
        updated.secret_fields = staged_bundle
            .as_object()
            .map(|object| object.keys().cloned().collect())
            .unwrap_or_default();
        updated.revision = updated.revision.saturating_add(1);
        updated.updated_at = Utc::now().to_rfc3339();
        if !dependent.is_empty() {
            let previous_namespace =
                infimount_mcp::storage_namespace::storage_namespace_fingerprint(&storages[idx])
                    .map_err(|e| {
                        err(
                            McpErrorCode::ERR_INTERNAL,
                            format!("failed to fingerprint current storage: {e}"),
                        )
                    })?;
            let prospective_namespace =
                infimount_mcp::storage_namespace::storage_namespace_fingerprint(&updated)
                    .map_err(|e| {
                        err(
                            McpErrorCode::ERR_INTERNAL,
                            format!("failed to fingerprint updated storage: {e}"),
                        )
                    })?;
            if previous_namespace != prospective_namespace {
                let workspaces = dependent
                    .iter()
                    .map(|workspace| {
                        serde_json::json!({
                            "workspaceId": workspace.id,
                            "workspaceName": workspace.name,
                        })
                    })
                    .collect::<Vec<_>>();
                return Err(err_with_details(
                    McpErrorCode::ERR_STORAGE_NAMESPACE_IN_USE,
                    "storage namespace change is rejected while workspaces are bound; delete or recreate the workspaces first",
                    serde_json::json!({ "workspaces": workspaces }),
                ));
            }
            if credentials_changed && !confirm_workspace_credential_change {
                return Err(err_with_details(
                    McpErrorCode::ERR_CONFIRMATION_REQUIRED,
                    "changing credentials may point this storage at a different account; confirm the workspace credential change",
                    serde_json::json!({ "workspaceCount": dependent.len() }),
                ));
            }
        }
        storages[idx] = updated.clone();
        Ok(updated)
    });
    if result.is_err() && staged_secret && secret_store.delete(&staged_account).is_err() {
        let journal_path =
            infimount_mcp::registry::default_config_dir().join("secret-cleanup.json");
        append_secret_cleanup(&journal_path, &staged_account)?;
    }

    let mut warning = None;
    if result.is_ok()
        && previous_bundle.is_some()
        && previous_account != staged_account
        && secret_store.delete(&previous_account).is_err()
    {
        let journal_path =
            infimount_mcp::registry::default_config_dir().join("secret-cleanup.json");
        warning = Some(
            if append_secret_cleanup(&journal_path, &previous_account).is_ok() {
                "Previous credential cleanup is pending and will be retried."
            } else {
                "Previous credential cleanup failed and could not be journaled; remove the old native secret-store entry manually."
            }
            .to_string(),
        );
    }
    if result.is_ok() {
        state.operator_cache.invalidate(&storageId);
    }
    result.map(|storage| (storage, warning))
}

#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RemoveStorageResult {
    pub removed: bool,
    pub warning: Option<String>,
}

#[tauri::command]
pub async fn remove_storage(
    state: State<'_, AppState>,
    storageId: String,
) -> Result<RemoveStorageResult, McpError> {
    state.require_operational()?;
    let _lifecycle = state.lifecycle_mutation.lock().await;
    let secret_store = state.secret_store.clone();
    let mut secret_ref_to_delete: Option<String> = None;

    ensure_storage_removable(&state, &storageId)?;

    state.registry.with_locked_mutation(|storages| {
        let original_len = storages.len();
        let target = storages.iter().find(|s| s.id == storageId);
        secret_ref_to_delete = target.and_then(|s| s.secret_ref.clone());
        storages.retain(|storage| storage.id != storageId);
        if storages.len() == original_len {
            return Err(err_with_details(
                McpErrorCode::ERR_STORAGE_NOT_FOUND,
                format!("storage '{}' not found", storageId),
                json!({ "storage_id": storageId }),
            ));
        }
        Ok(())
    })?;
    state.operator_cache.invalidate(&storageId);

    // Delete keyring entry after successful registry mutation
    if let Some(ref secret_ref) = secret_ref_to_delete {
        if secret_store.delete(secret_ref).is_err() {
            let journal_path =
                infimount_mcp::registry::default_config_dir().join("secret-cleanup.json");
            let warning = if append_secret_cleanup(&journal_path, secret_ref).is_ok() {
                "Credential cleanup is pending and will be retried.".to_string()
            } else {
                "Credential cleanup failed and could not be journaled; remove the native secret-store entry manually.".to_string()
            };
            return Ok(RemoveStorageResult {
                removed: true,
                warning: Some(warning),
            });
        }
    }

    Ok(RemoveStorageResult {
        removed: true,
        warning: None,
    })
}

fn ensure_storage_removable(state: &AppState, storage_id: &str) -> McpResult<()> {
    let dependent = state
        .workspaces
        .load_all()
        .map_err(|_| {
            err(
                McpErrorCode::ERR_INTERNAL,
                "failed to load workspaces while removing storage",
            )
        })?
        .into_iter()
        .filter(|workspace| workspace.storage_id == storage_id)
        .collect::<Vec<_>>();
    if !dependent.is_empty() {
        let workspaces = dependent
            .iter()
            .map(|workspace| {
                serde_json::json!({
                    "workspaceId": workspace.id,
                    "workspaceName": workspace.name,
                })
            })
            .collect::<Vec<_>>();
        return Err(err_with_details(
            McpErrorCode::ERR_STORAGE_HAS_WORKSPACES,
            "storage removal is rejected while workspaces are bound; delete the workspaces first",
            serde_json::json!({ "workspaces": workspaces }),
        ));
    }
    Ok(())
}

fn append_secret_cleanup(path: &std::path::Path, account: &str) -> McpResult<()> {
    let lock_path = path.with_extension("lock");
    if let Some(parent) = lock_path.parent() {
        infimount_core::atomic_file::create_dir_all(parent)
            .map_err(|_| err(McpErrorCode::ERR_INTERNAL, "failed to lock cleanup journal"))?;
    }
    let lock = OpenOptions::new()
        .create(true)
        .read(true)
        .write(true)
        .truncate(false)
        .open(lock_path)
        .map_err(|_| err(McpErrorCode::ERR_INTERNAL, "failed to lock cleanup journal"))?;
    let start = std::time::Instant::now();
    loop {
        match lock.try_lock_exclusive() {
            Ok(()) => break,
            Err(_) if start.elapsed() >= std::time::Duration::from_secs(2) => {
                return Err(err(
                    McpErrorCode::ERR_REGISTRY_LOCK_TIMEOUT,
                    "timed out acquiring secret cleanup journal lock",
                ));
            }
            Err(_) => std::thread::sleep(std::time::Duration::from_millis(50)),
        }
    }
    let mut document = if path.exists() {
        serde_json::from_slice::<Value>(
            &std::fs::read(path)
                .map_err(|_| err(McpErrorCode::ERR_INTERNAL, "failed to read cleanup journal"))?,
        )
        .unwrap_or_else(|_| json!({ "pending": [] }))
    } else {
        json!({ "pending": [] })
    };
    let pending = document
        .get_mut("pending")
        .and_then(Value::as_array_mut)
        .ok_or_else(|| {
            err(
                McpErrorCode::ERR_INTERNAL,
                "secret cleanup journal is invalid",
            )
        })?;
    if !pending
        .iter()
        .any(|item| item.get("account").and_then(Value::as_str) == Some(account))
    {
        pending.push(json!({ "account": account, "createdAt": Utc::now().to_rfc3339() }));
    }
    let payload = serde_json::to_vec_pretty(&document).map_err(|_| {
        err(
            McpErrorCode::ERR_INTERNAL,
            "failed to create cleanup journal",
        )
    })?;
    infimount_core::atomic_file::atomic_write_file(
        path,
        &payload,
        infimount_core::atomic_file::FILE_MODE,
    )
    .map_err(|_| {
        err(
            McpErrorCode::ERR_INTERNAL,
            "failed to persist cleanup journal",
        )
    })
}

#[tauri::command]
pub async fn update_mcp_storage_policy(
    state: State<'_, AppState>,
    storageId: String,
    policy: McpStoragePolicy,
) -> Result<StorageRecord, McpError> {
    state.require_operational()?;
    let _lifecycle = state.lifecycle_mutation.lock().await;
    let workspace_roots = state
        .workspaces
        .load_all()
        .map_err(|_| {
            err(
                McpErrorCode::ERR_INTERNAL,
                "failed to load workspaces while updating storage policy",
            )
        })?
        .into_iter()
        .filter(|workspace| workspace.storage_id == storageId)
        .filter_map(|workspace| normalize_policy_path(&workspace.root_path).ok())
        .collect::<Vec<_>>();
    let prospective = normalize_mcp_policy(policy)?;
    state.registry.with_locked_mutation(|storages| {
        let storage = storages
            .iter_mut()
            .find(|item| item.id == storageId)
            .ok_or_else(|| {
                err_with_details(
                    McpErrorCode::ERR_STORAGE_NOT_FOUND,
                    format!("storage '{}' not found", storageId),
                    serde_json::json!({ "storage_id": storageId }),
                )
            })?;

        assert_generic_policy_respects_workspace_rules(
            &storage.mcp_policy,
            &prospective,
            &workspace_roots,
        )?;

        storage.mcp_policy = prospective;
        storage.updated_at = Utc::now().to_rfc3339();
        Ok(storage.clone())
    })
}

fn assert_generic_policy_respects_workspace_rules(
    current: &McpStoragePolicy,
    prospective: &McpStoragePolicy,
    workspace_roots: &[String],
) -> McpResult<()> {
    if workspace_rule_signatures(current) != workspace_rule_signatures(prospective) {
        return Err(err_with_details(
            McpErrorCode::ERR_WORKSPACE_POLICY_MANAGED,
            "workspace-managed policy rules cannot be edited from the generic policy editor",
            serde_json::json!({}),
        ));
    }
    for rule in &prospective.rules {
        if matches!(rule.source, McpRuleSource::Manual) && workspace_roots.contains(&rule.prefix) {
            return Err(err_with_details(
                McpErrorCode::ERR_WORKSPACE_POLICY_MANAGED,
                "a manual policy rule cannot be created at a workspace root",
                serde_json::json!({}),
            ));
        }
    }
    Ok(())
}

fn workspace_rule_signatures(policy: &McpStoragePolicy) -> Vec<String> {
    let mut signatures = policy
        .rules
        .iter()
        .filter_map(|rule| {
            let McpRuleSource::Workspace { workspace_id } = &rule.source else {
                return None;
            };
            let confirmation = rule
                .confirmation_rules
                .as_ref()
                .map(|c| {
                    format!(
                        "w:{}o:{}d:{}vd:{}p:{}c:{}",
                        c.require_for_write,
                        c.require_for_overwrite,
                        c.require_for_delete,
                        c.require_for_version_delete,
                        c.require_for_presign,
                        c.require_for_cross_storage_copy,
                    )
                })
                .unwrap_or_default();
            Some(format!(
                "{}|{}|{}|{:?}|{}",
                workspace_id, rule.id, rule.prefix, rule.access, confirmation
            ))
        })
        .collect::<Vec<_>>();
    signatures.sort();
    signatures
}

fn normalize_mcp_policy(mut policy: McpStoragePolicy) -> McpResult<McpStoragePolicy> {
    migrate_legacy_policy(&mut policy)?;
    normalize_storage_policy(&mut policy)?;
    Ok(policy)
}

#[tauri::command]
pub async fn verify_storage(
    state: State<'_, AppState>,
    storage: StorageDraft,
) -> Result<ValidateStorageOutput, McpError> {
    state.require_operational()?;
    validate_storage_draft(&storage)?;
    let name = validate_storage_name(&storage.name)?;
    let mut config = storage.config.clone();
    let mut bundle = if let Some(storage_id) = storage.storage_id.as_deref() {
        let record = state.find_storage_by_id(storage_id)?;
        if record.backend != storage.backend {
            return Err(err(
                McpErrorCode::ERR_INVALID_PATH,
                "storage verification reference does not match the selected backend",
            ));
        }
        let account = record
            .secret_ref
            .clone()
            .unwrap_or_else(|| format!("storage/{}", record.id));
        let stored = state.secret_store.get_json(&account).map_err(|_| {
            err(
                McpErrorCode::ERR_SECRET_STORE_UNAVAILABLE,
                "failed to access stored credentials",
            )
        })?;
        if stored.is_none() && (record.secret_ref.is_some() || !record.secret_fields.is_empty()) {
            return Err(err(
                McpErrorCode::ERR_SECRET_NOT_FOUND,
                "stored credentials are missing",
            ));
        }
        stored.unwrap_or_else(|| json!({}))
    } else {
        json!({})
    };
    apply_secret_mutations_to_bundle(&mut bundle, &storage.secret_mutations)?;
    config = secrets::merge_secret_config(&config, &bundle);
    if let Some(session_id) = storage.oauth_session_id.as_deref() {
        let (provider, public_config, secret_config) =
            state.pending_oauth.snapshot(session_id).ok_or_else(|| {
                err(
                    McpErrorCode::ERR_OAUTH_SESSION_EXPIRED,
                    "OAuth session is missing or expired",
                )
            })?;
        if provider != storage.backend {
            return Err(err(
                McpErrorCode::ERR_INVALID_PATH,
                "OAuth session does not match the selected storage backend",
            ));
        }
        config = secrets::merge_secret_config(&config, &public_config);
        config = secrets::merge_secret_config(&config, &secret_config);
    }
    let mut record = StorageRecord::new(name, storage.backend, config);
    record.enabled = storage.enabled;
    record.mcp_exposed = storage.mcp_exposed;
    record.read_only = storage.read_only;
    let backend = record.backend.clone();
    let result = validate_storage_record(&record).await;
    let mut event = infimount_mcp::telemetry::ProductEvent::new(
        infimount_mcp::telemetry::ProductEventName::StorageValidationCompleted,
    );
    event.backend_type = Some(backend);
    event.success = Some(result.as_ref().is_ok_and(|output| output.valid));
    let _ = state.product_events.record(event);
    result
}

fn invalidate_operator_caches_after_import(state: &AppState) {
    state.operator_cache.clear();
    infimount_mcp::opendal_adapter::clear_operator_cache();
}

#[tauri::command]
pub async fn export_shareable_config(
    state: State<'_, AppState>,
) -> Result<ExportConfigOutput, McpError> {
    export_config(&state.fs_context()?).await
}

#[tauri::command]
pub async fn preview_storage_import_cmd(
    state: State<'_, AppState>,
    request: PreviewStorageImportInput,
) -> Result<StorageImportPreview, McpError> {
    preview_storage_import(&state.fs_context()?, request).await
}

fn workspace_access_mode(profile: &str) -> Option<McpAccessMode> {
    match profile {
        "none" => Some(McpAccessMode::None),
        "read_only" => Some(McpAccessMode::ReadOnly),
        "read_write" => Some(McpAccessMode::ReadWrite),
        _ => None,
    }
}

fn validate_import_workspace_references(
    workspaces: &[infimount_core::workspaces::WorkspaceRecord],
    resulting_storages: &[StorageRecord],
) -> McpResult<()> {
    for workspace in workspaces {
        let details = || {
            json!({
                "workspaceId": workspace.id,
                "storageId": workspace.storage_id,
                "policyRuleId": workspace.policy_rule_id,
            })
        };
        let storage = resulting_storages
            .iter()
            .find(|storage| storage.id == workspace.storage_id)
            .ok_or_else(|| {
                err_with_details(
                    McpErrorCode::ERR_IMPORT_PREVIEW_MISMATCH,
                    "storage import would remove a storage referenced by a workspace",
                    details(),
                )
            })?;
        if !storage.enabled {
            return Err(err_with_details(
                McpErrorCode::ERR_IMPORT_PREVIEW_MISMATCH,
                "storage import would disable a storage referenced by a workspace",
                details(),
            ));
        }

        let resulting_fingerprint =
            infimount_mcp::storage_namespace::storage_namespace_fingerprint(storage).map_err(
                |error| {
                    err_with_details(
                        McpErrorCode::ERR_IMPORT_PREVIEW_MISMATCH,
                        format!("failed to fingerprint imported storage namespace: {error}"),
                        details(),
                    )
                },
            )?;
        if workspace.storage_namespace_fingerprint != resulting_fingerprint {
            return Err(err_with_details(
                McpErrorCode::ERR_IMPORT_PREVIEW_MISMATCH,
                "storage import would retarget a workspace to a different namespace",
                details(),
            ));
        }

        let expected_access =
            workspace_access_mode(&workspace.access_profile).ok_or_else(|| {
                err_with_details(
                    McpErrorCode::ERR_IMPORT_PREVIEW_MISMATCH,
                    "workspace has an invalid access profile",
                    details(),
                )
            })?;
        let expected_prefix = normalize_policy_path(&workspace.root_path).map_err(|_| {
            err_with_details(
                McpErrorCode::ERR_IMPORT_PREVIEW_MISMATCH,
                "workspace has an invalid root path",
                details(),
            )
        })?;

        let Some(rule_id) = workspace.policy_rule_id.as_deref() else {
            if expected_access != McpAccessMode::None
                || storage.mcp_policy.rules.iter().any(|rule| {
                    matches!(
                        &rule.source,
                        McpRuleSource::Workspace { workspace_id }
                            if workspace_id == &workspace.id
                    )
                })
            {
                return Err(err_with_details(
                    McpErrorCode::ERR_IMPORT_PREVIEW_MISMATCH,
                    "storage import would break a workspace policy binding",
                    details(),
                ));
            }
            continue;
        };

        let rule = storage
            .mcp_policy
            .rules
            .iter()
            .find(|rule| rule.id == rule_id)
            .ok_or_else(|| {
                err_with_details(
                    McpErrorCode::ERR_IMPORT_PREVIEW_MISMATCH,
                    "storage import would remove a workspace policy rule",
                    details(),
                )
            })?;
        if rule.prefix != expected_prefix
            || rule.access != expected_access
            || rule.confirmation_rules.is_some()
            || !matches!(
                &rule.source,
                McpRuleSource::Workspace { workspace_id } if workspace_id == &workspace.id
            )
        {
            return Err(err_with_details(
                McpErrorCode::ERR_IMPORT_PREVIEW_MISMATCH,
                "storage import would change a workspace policy binding",
                details(),
            ));
        }
    }

    // Validate the reverse edge as well. An imported policy must not introduce an
    // orphaned workspace source, bind a managed rule to another storage, or leave
    // an extra rule that merely names an otherwise valid workspace.
    for storage in resulting_storages {
        for rule in &storage.mcp_policy.rules {
            let McpRuleSource::Workspace { workspace_id } = &rule.source else {
                continue;
            };
            let details = || {
                json!({
                    "workspaceId": workspace_id,
                    "storageId": storage.id,
                    "policyRuleId": rule.id,
                })
            };
            let workspace = workspaces
                .iter()
                .find(|workspace| workspace.id == *workspace_id)
                .ok_or_else(|| {
                    err_with_details(
                        McpErrorCode::ERR_IMPORT_PREVIEW_MISMATCH,
                        "storage import would create an orphaned workspace policy rule",
                        details(),
                    )
                })?;
            let expected_access =
                workspace_access_mode(&workspace.access_profile).ok_or_else(|| {
                    err_with_details(
                        McpErrorCode::ERR_IMPORT_PREVIEW_MISMATCH,
                        "workspace has an invalid access profile",
                        details(),
                    )
                })?;
            let expected_prefix = normalize_policy_path(&workspace.root_path).map_err(|_| {
                err_with_details(
                    McpErrorCode::ERR_IMPORT_PREVIEW_MISMATCH,
                    "workspace has an invalid root path",
                    details(),
                )
            })?;
            if workspace.storage_id != storage.id
                || workspace.policy_rule_id.as_deref() != Some(rule.id.as_str())
                || rule.prefix != expected_prefix
                || rule.access != expected_access
                || rule.confirmation_rules.is_some()
            {
                return Err(err_with_details(
                    McpErrorCode::ERR_IMPORT_PREVIEW_MISMATCH,
                    "storage import would create a mismatched workspace policy binding",
                    details(),
                ));
            }
        }
    }
    Ok(())
}

#[tauri::command]
pub async fn apply_storage_import_cmd(
    state: State<'_, AppState>,
    request: ApplyStorageImportInput,
) -> Result<ApplyStorageImportResult, McpError> {
    let _lifecycle = state.lifecycle_mutation.lock().await;
    let workspaces = state.workspaces.load_all().map_err(|_| {
        err(
            McpErrorCode::ERR_INTERNAL,
            "failed to validate workspace references before storage import",
        )
    })?;
    let result = apply_storage_import_with_validator(&state.fs_context()?, request, |storages| {
        validate_import_workspace_references(&workspaces, storages)
    })
    .await;
    if result.is_ok() {
        invalidate_operator_caches_after_import(&state);
    }
    result
}

#[tauri::command]
pub fn list_storage_schemas() -> Result<Vec<StorageKindSchema>, CoreError> {
    infimount_core::schema::list_storage_schemas()
}

#[tauri::command]
pub fn get_storage_capabilities(
    state: State<'_, AppState>,
    storageId: String,
) -> Result<StorageBackendCapabilities, CoreError> {
    let op = state.operator_for_storage_id(&storageId)?;
    Ok(get_capabilities(&op))
}

#[tauri::command]
pub async fn generate_download_link(
    state: State<'_, AppState>,
    sourceId: String,
    path: String,
    expiresSeconds: u64,
) -> Result<String, CoreError> {
    if !(60..=86_400).contains(&expiresSeconds) {
        return Err(CoreError::Config(
            "expiresSeconds must be between 60 and 86400".to_string(),
        ));
    }

    let op = state.operator_for_storage_id(&sourceId)?;
    let caps = op.info().capability();
    if !caps.presign_read {
        return Err(CoreError::Config(
            "storage backend does not support presigned download links".to_string(),
        ));
    }

    let metadata = op.stat(&path).await?;
    if metadata.is_dir() {
        return Err(CoreError::Config(
            "download links can only be created for files".to_string(),
        ));
    }

    let presigned = op
        .presign_read(&path, Duration::from_secs(expiresSeconds))
        .await?;
    Ok(presigned.uri().to_string())
}

#[tauri::command]
pub async fn list_versions(
    state: State<'_, AppState>,
    sourceId: String,
    path: String,
    limit: Option<u32>,
    cursor: Option<String>,
) -> Result<Value, CoreError> {
    let op = state.operator_for_storage_id(&sourceId)?;
    let result =
        operations::list_file_versions(&op, &path, limit.unwrap_or(100), cursor.as_deref()).await?;
    Ok(serde_json::to_value(result).unwrap_or(Value::Null))
}

#[tauri::command]
pub async fn read_file_version(
    state: State<'_, AppState>,
    sourceId: String,
    path: String,
    version: String,
) -> Result<Vec<u8>, CoreError> {
    let op = state.operator_for_storage_id(&sourceId)?;
    let metadata = op.stat_with(&path).version(&version).await?;
    if metadata.is_dir() || metadata.content_length() > MAX_LEGACY_IPC_READ_BYTES {
        return Err(CoreError::Config(format!(
            "version IPC reads are limited to {MAX_LEGACY_IPC_READ_BYTES} bytes; use native version download"
        )));
    }
    operations::read_file_version(&op, &path, &version).await
}

#[tauri::command]
pub async fn delete_version(
    state: State<'_, AppState>,
    sourceId: String,
    path: String,
    version: String,
) -> Result<Value, CoreError> {
    let op = state.operator_for_storage_id(&sourceId)?;
    operations::delete_file_version(&op, &path, &version).await?;
    Ok(serde_json::json!({ "deleted": true, "path": path, "version": version }))
}

fn validate_storage_draft(storage: &StorageDraft) -> McpResult<()> {
    if !storage.config.is_object() {
        return Err(err_with_details(
            McpErrorCode::ERR_INTERNAL,
            "storage config must be a JSON object",
            serde_json::json!({}),
        ));
    }

    if !matches!(
        storage.backend.as_str(),
        "local"
            | "s3"
            | "b2"
            | "oss"
            | "cos"
            | "obs"
            | "azure_blob"
            | "webdav"
            | "gcs"
            | "gdrive"
            | "onedrive"
            | "sftp"
            | "ftp"
    ) {
        return Err(err_with_details(
            McpErrorCode::ERR_BACKEND_UNSUPPORTED,
            format!("unsupported backend '{}'", storage.backend),
            serde_json::json!({ "backend": storage.backend }),
        ));
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validate_storage_draft_accepts_all_desktop_backends() {
        for backend in [
            "local",
            "s3",
            "b2",
            "oss",
            "cos",
            "obs",
            "azure_blob",
            "webdav",
            "gcs",
            "gdrive",
            "onedrive",
            "sftp",
            "ftp",
        ] {
            let storage = StorageDraft {
                storage_id: None,
                name: format!("{backend} storage"),
                backend: backend.to_string(),
                config: serde_json::json!({}),
                enabled: true,
                mcp_exposed: false,
                read_only: false,
                oauth_session_id: None,
                secret_mutations: HashMap::new(),
            };
            validate_storage_draft(&storage).expect("backend should be accepted");
        }
    }

    #[test]
    fn activation_demo_roots_are_unique_and_do_not_reuse_predictable_data() {
        let config = std::path::Path::new("/tmp/infimount-test-config");
        let first = new_activation_demo_root(config);
        let second = new_activation_demo_root(config);
        assert_ne!(first, second);
        assert_eq!(first.parent(), Some(config));
        assert!(first
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| name.starts_with("activation-demo-")));
    }

    #[test]
    fn staged_upload_usage_rejects_session_and_aggregate_exhaustion() {
        assert!(validate_upload_usage(MAX_UPLOAD_SESSIONS, 0, 0).is_err());
        assert!(validate_upload_usage(0, MAX_STAGED_UPLOAD_AGGREGATE_BYTES, 1,).is_err());
        assert!(validate_upload_usage(
            MAX_UPLOAD_SESSIONS - 1,
            MAX_STAGED_UPLOAD_AGGREGATE_BYTES - 1,
            1,
        )
        .is_ok());
    }

    #[tokio::test]
    async fn staged_upload_is_bounded_and_cancellable() {
        let upload_id = begin_file_upload().await.expect("begin upload");
        append_file_upload_chunk(upload_id.clone(), b"hello".to_vec())
            .await
            .expect("append chunk");
        let path = upload_staging_path(&upload_id).expect("staging path");
        assert_eq!(tokio::fs::read(&path).await.unwrap(), b"hello");

        let oversized = vec![0; MAX_UPLOAD_CHUNK_BYTES + 1];
        assert!(append_file_upload_chunk(upload_id.clone(), oversized)
            .await
            .is_err());
        assert_eq!(tokio::fs::read(&path).await.unwrap(), b"hello");

        cancel_file_upload(upload_id.clone())
            .await
            .expect("cancel upload");
        assert!(!path.exists());
        cancel_file_upload(upload_id)
            .await
            .expect("cancel is idempotent");
    }

    #[tokio::test]
    async fn finish_upload_cleans_staging_when_storage_is_invalid() {
        use infimount_core::secrets::MemorySecretStore;
        use std::sync::Arc;
        let dir = tempfile::tempdir().unwrap();
        let state = AppState::new_for_test(dir.path(), Arc::new(MemorySecretStore::new()));
        let upload_id = begin_file_upload().await.unwrap();
        append_file_upload_chunk(upload_id.clone(), b"data".to_vec())
            .await
            .unwrap();
        let part = upload_staging_path(&upload_id).unwrap();
        let uploading = part.with_extension("uploading");

        let result = finish_file_upload_inner(
            &state,
            upload_id.clone(),
            "missing-storage".to_string(),
            "target.bin".to_string(),
        )
        .await;

        assert!(result.is_err());
        assert!(!part.exists());
        assert!(!uploading.exists());
        assert!(upload_cancel_flag(&upload_id).is_err());
    }

    #[test]
    fn import_invalidation_clears_desktop_operator_cache() {
        use infimount_core::runtime::CacheKey;
        use infimount_core::secrets::MemorySecretStore;
        use opendal::services::Memory;
        use std::sync::Arc;
        let dir = tempfile::tempdir().unwrap();
        let state = AppState::new_for_test(dir.path(), Arc::new(MemorySecretStore::new()));
        let key = CacheKey {
            storage_id: "storage-1".to_string(),
            revision: 1,
        };
        let op = opendal::Operator::new(Memory::default()).unwrap();
        state.operator_cache.insert(key.clone(), op);
        assert!(state.operator_cache.get(&key).is_some());
        invalidate_operator_caches_after_import(&state);
        assert!(state.operator_cache.get(&key).is_none());
    }

    #[test]
    fn import_preserves_exact_workspace_storage_and_policy_binding() {
        let mut storage = StorageRecord::new(
            "Workspace storage".to_string(),
            "local".to_string(),
            json!({ "root": "/tmp" }),
        );
        let workspace = infimount_core::workspaces::WorkspaceRecord {
            id: "workspace-1".to_string(),
            schema_version: infimount_core::workspaces::WORKSPACE_RECORD_SCHEMA_VERSION,
            storage_id: storage.id.clone(),
            name: "Workspace".to_string(),
            root_path: "workspace".to_string(),
            template_id: "coding".to_string(),
            access_profile: "read_only".to_string(),
            policy_rule_id: Some("workspace:workspace-1".to_string()),
            storage_namespace_fingerprint:
                infimount_mcp::storage_namespace::storage_namespace_fingerprint(&storage)
                    .expect("storage namespace fingerprint"),
            created_at: "2026-01-01T00:00:00Z".to_string(),
            updated_at: "2026-01-01T00:00:00Z".to_string(),
            memory_files: vec![
                "memory/tasks.md".to_string(),
                "memory/decisions.md".to_string(),
                "memory/handoff.md".to_string(),
            ],
            checkpoint_ids: vec![],
        };
        storage
            .mcp_policy
            .rules
            .push(infimount_mcp::policy::McpPathRule {
                id: workspace.policy_rule_id.clone().unwrap(),
                prefix: workspace.root_path.clone(),
                access: McpAccessMode::ReadOnly,
                source: McpRuleSource::Workspace {
                    workspace_id: workspace.id.clone(),
                },
                confirmation_rules: None,
            });

        let assert_rejected = |candidate: Vec<StorageRecord>, reason: &str| {
            let error =
                validate_import_workspace_references(std::slice::from_ref(&workspace), &candidate)
                    .expect_err(reason);
            assert_eq!(error.code, McpErrorCode::ERR_IMPORT_PREVIEW_MISMATCH);
        };
        assert_rejected(vec![], "referenced storage removal must fail closed");

        let mut disabled = storage.clone();
        disabled.enabled = false;
        assert_rejected(
            vec![disabled],
            "referenced storage disable must fail closed",
        );

        let mut missing_rule = storage.clone();
        missing_rule.mcp_policy.rules.clear();
        assert_rejected(vec![missing_rule], "managed rule removal must fail closed");

        let mut changed_source = storage.clone();
        changed_source.mcp_policy.rules[0].source = McpRuleSource::Manual;
        assert_rejected(
            vec![changed_source],
            "managed rule source must remain exact",
        );

        let mut changed_prefix = storage.clone();
        changed_prefix.mcp_policy.rules[0].prefix = "other".to_string();
        assert_rejected(
            vec![changed_prefix],
            "managed rule prefix must remain exact",
        );

        let mut changed_access = storage.clone();
        changed_access.mcp_policy.rules[0].access = McpAccessMode::ReadWrite;
        assert_rejected(
            vec![changed_access],
            "managed rule access must remain exact",
        );

        let mut changed_confirmation = storage.clone();
        changed_confirmation.mcp_policy.rules[0].confirmation_rules =
            Some(infimount_mcp::policy::McpConfirmationRules {
                require_for_write: false,
                require_for_overwrite: false,
                require_for_delete: false,
                require_for_version_delete: false,
                require_for_presign: false,
                require_for_cross_storage_copy: false,
            });
        assert_rejected(
            vec![changed_confirmation],
            "managed rule confirmation override must remain absent",
        );

        let mut orphaned_rule = storage.clone();
        orphaned_rule
            .mcp_policy
            .rules
            .push(infimount_mcp::policy::McpPathRule {
                id: "workspace:missing".to_string(),
                prefix: "missing".to_string(),
                access: McpAccessMode::ReadOnly,
                source: McpRuleSource::Workspace {
                    workspace_id: "missing".to_string(),
                },
                confirmation_rules: None,
            });
        assert_rejected(
            vec![orphaned_rule],
            "orphaned workspace policy source must fail closed",
        );

        let mut duplicate_binding = storage.clone();
        let mut extra_rule = duplicate_binding.mcp_policy.rules[0].clone();
        extra_rule.id = "workspace:workspace-1:extra".to_string();
        duplicate_binding.mcp_policy.rules.push(extra_rule);
        assert_rejected(
            vec![duplicate_binding],
            "extra workspace binding must fail closed",
        );

        let mut wrong_storage = StorageRecord::new(
            "Other".to_string(),
            "local".to_string(),
            json!({ "root": "/tmp/other" }),
        );
        wrong_storage
            .mcp_policy
            .rules
            .push(storage.mcp_policy.rules[0].clone());
        assert_rejected(
            vec![storage.clone(), wrong_storage],
            "workspace rule on another storage must fail closed",
        );

        validate_import_workspace_references(&[workspace], &[storage]).unwrap();
    }

    fn workspace_bound_state(
        dir: &tempfile::TempDir,
    ) -> (
        AppState,
        StorageRecord,
        infimount_core::workspaces::WorkspaceRecord,
    ) {
        use infimount_core::secrets::MemorySecretStore;
        use std::sync::Arc;
        let state = AppState::new_for_test(dir.path(), Arc::new(MemorySecretStore::new()));
        let mut storage = StorageRecord::new(
            "Bound".to_string(),
            "local".to_string(),
            json!({ "root": "/tmp" }),
        );
        storage.id = "bound-storage".to_string();
        let fingerprint =
            infimount_mcp::storage_namespace::storage_namespace_fingerprint(&storage).unwrap();
        let workspace = infimount_core::workspaces::WorkspaceRecord {
            id: "bound-ws".to_string(),
            schema_version: infimount_core::workspaces::WORKSPACE_RECORD_SCHEMA_VERSION,
            storage_id: storage.id.clone(),
            name: "Bound".to_string(),
            root_path: "workspace".to_string(),
            template_id: "coding".to_string(),
            access_profile: "read_only".to_string(),
            policy_rule_id: Some("workspace:bound-ws".to_string()),
            storage_namespace_fingerprint: fingerprint,
            created_at: "2026-01-01T00:00:00Z".to_string(),
            updated_at: "2026-01-01T00:00:00Z".to_string(),
            memory_files: infimount_core::workspaces::memory_files_for("coding"),
            checkpoint_ids: vec![],
        };
        storage
            .mcp_policy
            .rules
            .push(infimount_mcp::policy::McpPathRule {
                id: workspace.policy_rule_id.clone().unwrap(),
                prefix: "workspace".to_string(),
                access: McpAccessMode::ReadOnly,
                source: McpRuleSource::Workspace {
                    workspace_id: workspace.id.clone(),
                },
                confirmation_rules: None,
            });
        state.registry.save_all_atomic(&[storage.clone()]).unwrap();
        state.workspaces.create(&workspace).unwrap();
        (state, storage, workspace)
    }

    #[tokio::test]
    async fn storage_namespace_edit_with_dependent_workspace_is_rejected() {
        let dir = tempfile::tempdir().unwrap();
        let (state, storage, _workspace) = workspace_bound_state(&dir);
        let draft = StorageDraft {
            storage_id: None,
            name: storage.name.clone(),
            backend: "local".to_string(),
            config: json!({ "root": "/tmp/other" }),
            enabled: true,
            mcp_exposed: false,
            read_only: false,
            oauth_session_id: None,
            secret_mutations: HashMap::new(),
        };
        let error =
            update_storage_with_draft(&state, storage.id.clone(), draft, false, false).unwrap_err();
        assert_eq!(error.code, McpErrorCode::ERR_STORAGE_NAMESPACE_IN_USE);
    }

    #[tokio::test]
    async fn storage_credential_change_requires_confirmation_when_workspaces_bound() {
        let dir = tempfile::tempdir().unwrap();
        let (state, storage, _workspace) = workspace_bound_state(&dir);
        let mut mutations = HashMap::new();
        mutations.insert(
            "token".to_string(),
            SecretMutation::Set {
                value: "new-token".to_string(),
            },
        );
        let unconfirmed = StorageDraft {
            storage_id: None,
            name: storage.name.clone(),
            backend: "local".to_string(),
            config: storage.config.clone(),
            enabled: true,
            mcp_exposed: false,
            read_only: false,
            oauth_session_id: None,
            secret_mutations: mutations,
        };
        let error =
            update_storage_with_draft(&state, storage.id.clone(), unconfirmed, false, false)
                .unwrap_err();
        assert_eq!(error.code, McpErrorCode::ERR_CONFIRMATION_REQUIRED);

        let mut confirmed_mutations = HashMap::new();
        confirmed_mutations.insert(
            "token".to_string(),
            SecretMutation::Set {
                value: "new-token".to_string(),
            },
        );
        let confirmed = StorageDraft {
            storage_id: None,
            name: storage.name.clone(),
            backend: "local".to_string(),
            config: storage.config.clone(),
            enabled: true,
            mcp_exposed: false,
            read_only: false,
            oauth_session_id: None,
            secret_mutations: confirmed_mutations,
        };
        let (updated, _) =
            update_storage_with_draft(&state, storage.id.clone(), confirmed, false, true).unwrap();
        assert_eq!(updated.id, storage.id);
    }

    #[tokio::test]
    async fn confirmed_credential_change_with_bound_workspaces_rejects_invalid_prospective_storage()
    {
        let dir = tempfile::tempdir().unwrap();
        let (state, storage, _workspace) = workspace_bound_state(&dir);
        let mut mutations = HashMap::new();
        mutations.insert(
            "token".to_string(),
            SecretMutation::Set {
                value: "new-token".to_string(),
            },
        );
        let draft = StorageDraft {
            storage_id: None,
            name: storage.name.clone(),
            backend: "local".to_string(),
            config: json!({ "root": dir.path().join("blocker/does-not-exist") }),
            enabled: true,
            mcp_exposed: false,
            read_only: false,
            oauth_session_id: None,
            secret_mutations: mutations,
        };
        std::fs::write(dir.path().join("blocker"), b"blocker").unwrap();
        let error = validate_confirmed_credential_change(&state, &storage.id, &draft, false)
            .await
            .unwrap_err();
        assert_eq!(error.code, McpErrorCode::ERR_INVALID_STORAGE_CREDENTIALS);
    }

    #[tokio::test]
    async fn confirmed_credential_change_with_bound_workspaces_accepts_valid_prospective_storage() {
        let dir = tempfile::tempdir().unwrap();
        let (state, storage, _workspace) = workspace_bound_state(&dir);
        let mut mutations = HashMap::new();
        mutations.insert(
            "token".to_string(),
            SecretMutation::Set {
                value: "new-token".to_string(),
            },
        );
        let draft = StorageDraft {
            storage_id: None,
            name: storage.name.clone(),
            backend: "local".to_string(),
            config: json!({ "root": "/tmp" }),
            enabled: true,
            mcp_exposed: false,
            read_only: false,
            oauth_session_id: None,
            secret_mutations: mutations,
        };
        validate_confirmed_credential_change(&state, &storage.id, &draft, false)
            .await
            .unwrap();
    }

    #[test]
    fn storage_removal_with_dependent_workspace_is_rejected() {
        let dir = tempfile::tempdir().unwrap();
        let (state, storage, _workspace) = workspace_bound_state(&dir);
        let error = ensure_storage_removable(&state, &storage.id).unwrap_err();
        assert_eq!(error.code, McpErrorCode::ERR_STORAGE_HAS_WORKSPACES);
    }

    #[test]
    fn manual_policy_update_cannot_alter_workspace_rule() {
        let dir = tempfile::tempdir().unwrap();
        let (state, storage, workspace) = workspace_bound_state(&dir);
        let roots = vec!["workspace".to_string()];
        let policy = storage.mcp_policy.clone();
        assert_eq!(policy.rules.len(), 1);
        assert!(matches!(
            &policy.rules[0].source,
            McpRuleSource::Workspace { workspace_id } if workspace_id == &workspace.id
        ));

        let mut changed = policy.clone();
        changed.rules[0].access = McpAccessMode::ReadWrite;
        let error =
            assert_generic_policy_respects_workspace_rules(&storage.mcp_policy, &changed, &roots)
                .unwrap_err();
        assert_eq!(error.code, McpErrorCode::ERR_WORKSPACE_POLICY_MANAGED);

        let mut manual_at_root = policy.clone();
        manual_at_root
            .rules
            .push(infimount_mcp::policy::McpPathRule {
                id: "manual-root".to_string(),
                prefix: "workspace".to_string(),
                access: McpAccessMode::ReadWrite,
                source: McpRuleSource::Manual,
                confirmation_rules: None,
            });
        let error = assert_generic_policy_respects_workspace_rules(
            &storage.mcp_policy,
            &manual_at_root,
            &roots,
        )
        .unwrap_err();
        assert_eq!(error.code, McpErrorCode::ERR_WORKSPACE_POLICY_MANAGED);

        let mut manual_elsewhere = policy.clone();
        manual_elsewhere
            .rules
            .push(infimount_mcp::policy::McpPathRule {
                id: "manual-elsewhere".to_string(),
                prefix: "other".to_string(),
                access: McpAccessMode::ReadWrite,
                source: McpRuleSource::Manual,
                confirmation_rules: None,
            });
        assert_generic_policy_respects_workspace_rules(
            &storage.mcp_policy,
            &manual_elsewhere,
            &roots,
        )
        .unwrap();
        let _ = state;
    }

    #[test]
    fn import_cannot_retarget_a_workspace() {
        let storage = StorageRecord::new(
            "Bound".to_string(),
            "local".to_string(),
            json!({ "root": "/tmp" }),
        );
        let mut retargeted = storage.clone();
        retargeted.config = json!({ "root": "/tmp/other" });
        let workspace = infimount_core::workspaces::WorkspaceRecord {
            id: "bound-ws".to_string(),
            schema_version: infimount_core::workspaces::WORKSPACE_RECORD_SCHEMA_VERSION,
            storage_id: storage.id.clone(),
            name: "Bound".to_string(),
            root_path: "workspace".to_string(),
            template_id: "coding".to_string(),
            access_profile: "read_only".to_string(),
            policy_rule_id: Some("workspace:bound-ws".to_string()),
            storage_namespace_fingerprint:
                infimount_mcp::storage_namespace::storage_namespace_fingerprint(&storage)
                    .expect("storage namespace fingerprint"),
            created_at: "2026-01-01T00:00:00Z".to_string(),
            updated_at: "2026-01-01T00:00:00Z".to_string(),
            memory_files: vec![],
            checkpoint_ids: vec![],
        };
        let error = validate_import_workspace_references(&[workspace], &[retargeted]).unwrap_err();
        assert_eq!(error.code, McpErrorCode::ERR_IMPORT_PREVIEW_MISMATCH);
    }

    #[test]
    fn normalize_mcp_policy_uses_shared_v2_normalization() {
        let policy = McpStoragePolicy {
            denied_paths: vec!["shared/tmp/../public".to_string()],
            ..McpStoragePolicy::default()
        };
        let normalized = normalize_mcp_policy(policy).expect("normalize policy");
        assert_eq!(normalized.denied_paths, vec!["shared/public"]);
    }
}
