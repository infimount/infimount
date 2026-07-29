use serde::{Deserialize, Serialize};
use std::fmt;

fn format_storage_error(error: &opendal::Error) -> String {
    let status = if error.is_temporary() {
        "temporary"
    } else if error.is_persistent() {
        "persistent"
    } else {
        "permanent"
    };
    format!("{} ({})", error.kind(), status)
}

/// Core error type used across the backend.
#[derive(thiserror::Error, Debug)]
pub enum CoreError {
    #[error("source not found: {0}")]
    SourceNotFound(String),

    #[error("unsupported source kind: {0:?}")]
    UnsupportedSourceKind(SourceKind),

    #[error("config error: {0}")]
    Config(String),

    #[error("storage error: {}", format_storage_error(.0))]
    Storage(#[from] opendal::Error),

    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    #[error("serialization error: {0}")]
    Serde(#[from] serde_json::Error),

    #[error("backup error: {0}")]
    Backup(#[from] crate::backup::BackupError),
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ErrorCode {
    NotFound,
    PermissionDenied,
    AlreadyExists,
    ConfigError,
    IoError,
    Unknown,
}

impl CoreError {
    pub fn code(&self) -> ErrorCode {
        match self {
            CoreError::SourceNotFound(_) => ErrorCode::NotFound,
            CoreError::UnsupportedSourceKind(_) => ErrorCode::ConfigError,
            CoreError::Config(_) => ErrorCode::ConfigError,
            CoreError::Storage(e) => match e.kind() {
                opendal::ErrorKind::NotFound => ErrorCode::NotFound,
                opendal::ErrorKind::PermissionDenied => ErrorCode::PermissionDenied,
                opendal::ErrorKind::AlreadyExists => ErrorCode::AlreadyExists,
                _ => ErrorCode::Unknown,
            },
            CoreError::Io(e) => match e.kind() {
                std::io::ErrorKind::NotFound => ErrorCode::NotFound,
                std::io::ErrorKind::PermissionDenied => ErrorCode::PermissionDenied,
                std::io::ErrorKind::AlreadyExists => ErrorCode::AlreadyExists,
                _ => ErrorCode::IoError,
            },
            CoreError::Serde(_) => ErrorCode::Unknown,
            CoreError::Backup(_) => ErrorCode::Unknown,
        }
    }
}

impl Serialize for CoreError {
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        use serde::ser::SerializeStruct;
        let mut state = serializer.serialize_struct("CoreError", 2)?;
        state.serialize_field("code", &self.code())?;
        state.serialize_field("message", &self.to_string())?;
        state.end()
    }
}

pub type Result<T> = std::result::Result<T, CoreError>;

/// A configured storage source (currently only local filesystem).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Source {
    pub id: String,
    pub name: String,
    pub kind: SourceKind,
    /// Root path for this source (for local filesystem).
    #[serde(default)]
    pub root: String,
    /// Configuration for the source (credentials, endpoint, etc.).
    #[serde(default)]
    pub config: serde_json::Value,
}

/// Types of storage that can back a Source.
///
/// Only `Local` is implemented initially; other variants are
/// placeholders for future backends like S3, WebDAV, Azure Blob, etc.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum SourceKind {
    #[serde(rename = "local")]
    Local,
    #[serde(rename = "s3")]
    S3,
    #[serde(rename = "webdav")]
    WebDav,
    #[serde(rename = "azure_blob")]
    AzureBlob,
    #[serde(rename = "gcs")]
    Gcs,
    #[serde(rename = "b2")]
    B2,
    #[serde(rename = "oss")]
    Oss,
    #[serde(rename = "cos")]
    Cos,
    #[serde(rename = "obs")]
    Obs,
    #[serde(rename = "sftp")]
    Sftp,
    #[serde(rename = "ftp")]
    Ftp,
    #[serde(rename = "gdrive")]
    Gdrive,
    #[serde(rename = "onedrive")]
    Onedrive,
}

impl fmt::Display for SourceKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SourceKind::Local => write!(f, "local"),
            SourceKind::S3 => write!(f, "s3"),
            SourceKind::WebDav => write!(f, "webdav"),
            SourceKind::AzureBlob => write!(f, "azure_blob"),
            SourceKind::Gcs => write!(f, "gcs"),
            SourceKind::B2 => write!(f, "b2"),
            SourceKind::Oss => write!(f, "oss"),
            SourceKind::Cos => write!(f, "cos"),
            SourceKind::Obs => write!(f, "obs"),
            SourceKind::Sftp => write!(f, "sftp"),
            SourceKind::Ftp => write!(f, "ftp"),
            SourceKind::Gdrive => write!(f, "gdrive"),
            SourceKind::Onedrive => write!(f, "onedrive"),
        }
    }
}

impl std::str::FromStr for SourceKind {
    type Err = ();

    fn from_str(s: &str) -> std::result::Result<Self, Self::Err> {
        match s.trim().to_ascii_lowercase().as_str() {
            "local" | "fs" => Ok(SourceKind::Local),
            "s3" => Ok(SourceKind::S3),
            "webdav" => Ok(SourceKind::WebDav),
            "azure_blob" | "azblob" => Ok(SourceKind::AzureBlob),
            "gcs" => Ok(SourceKind::Gcs),
            "b2" | "backblaze_b2" => Ok(SourceKind::B2),
            "oss" | "aliyun_oss" => Ok(SourceKind::Oss),
            "cos" | "tencent_cos" => Ok(SourceKind::Cos),
            "obs" | "huawei_obs" => Ok(SourceKind::Obs),
            "sftp" => Ok(SourceKind::Sftp),
            "ftp" => Ok(SourceKind::Ftp),
            "gdrive" | "google_drive" | "google-drive" => Ok(SourceKind::Gdrive),
            "onedrive" | "one_drive" | "one-drive" => Ok(SourceKind::Onedrive),
            _ => Err(()),
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StorageBackendCapabilities {
    pub list_with_versions: bool,
    pub read_with_version: bool,
    pub delete_with_version: bool,
    pub presign_read: bool,
    pub versioning_disabled: bool,
    pub write_with_user_metadata: bool,
}

/// A single entry returned from listing or stat operations.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Entry {
    pub path: String,
    pub name: String,
    pub is_dir: bool,
    pub size: u64,
    pub modified_at: Option<String>,
    pub etag: Option<String>,
}

/// Request to list entries under a path.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ListRequest {
    pub source_id: String,
    pub path: String,
}

/// Request to read a full object.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReadRequest {
    pub source_id: String,
    pub path: String,
}

/// Request to write a full object.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WriteRequest {
    pub source_id: String,
    pub path: String,
    pub data: Vec<u8>,
}

/// Paginated listing request.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ListEntriesPageRequest {
    pub source_id: String,
    pub path: String,
    #[serde(default = "default_page_limit")]
    pub limit: u32,
    pub cursor: Option<String>,
    #[serde(default)]
    pub recursive: bool,
}

fn default_page_limit() -> u32 {
    200
}

/// Paginated listing response.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ListEntriesPage {
    pub entries: Vec<Entry>,
    pub next_cursor: Option<String>,
    pub truncated: bool,
}

/// Range read result.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReadFileRangeResult {
    pub total_size: u64,
    pub offset: u64,
    pub bytes: Vec<u8>,
    pub truncated: bool,
}

pub const DEFAULT_PREVIEW_MAX: u64 = 256 * 1024; // 256 KiB
pub const MAX_READ_RANGE_BYTES: u64 = 2 * 1024 * 1024; // 2 MiB
pub const MAX_LIST_LIMIT: u32 = 1000;
pub const MAX_RECURSIVE_ITEMS: u32 = 10000;

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_error_serialization() {
        let err = CoreError::SourceNotFound("foo".to_string());
        let json = serde_json::to_value(&err).unwrap();
        assert_eq!(
            json,
            json!({
                "code": "NOT_FOUND",
                "message": "source not found: foo"
            })
        );
    }

    #[test]
    fn test_error_code_mapping() {
        let err = CoreError::Config("bad config".to_string());
        assert_eq!(err.code(), ErrorCode::ConfigError);
    }

    #[test]
    fn source_deserializes_legacy_missing_config() {
        let source: Source = serde_json::from_value(json!({
            "id": "legacy",
            "name": "Legacy",
            "kind": "local",
            "root": "/tmp"
        }))
        .unwrap();

        assert_eq!(source.config, serde_json::Value::Null);
    }

    #[test]
    fn storage_errors_are_sanitized_before_serialization() {
        let err = CoreError::Storage(
            opendal::Error::new(opendal::ErrorKind::Unexpected, "backend failed")
                .with_operation("stat")
                .with_context("uri", "https://storage.example/private/path?token=secret"),
        );

        let value = serde_json::to_value(&err).unwrap();
        let message = value["message"].as_str().unwrap();
        assert_eq!(message, "storage error: Unexpected (permanent)");
        assert!(!message.contains("backend failed"));
        assert!(!message.contains("https://storage.example"));
        assert!(!message.contains("token=secret"));
    }
}
