use serde::{Deserialize, Serialize};
use std::fmt;

/// Core error type used across the backend.
#[derive(thiserror::Error, Debug)]
pub enum CoreError {
    #[error("source not found: {0}")]
    SourceNotFound(String),

    #[error("unsupported source kind: {0:?}")]
    UnsupportedSourceKind(SourceKind),

    #[error("config error: {0}")]
    Config(String),

    #[error("storage error: {0}")]
    Storage(#[from] opendal::Error),

    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    #[error("serialization error: {0}")]
    Serde(#[from] serde_json::Error),
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
    pub root: String,
    /// Configuration for the source (credentials, endpoint, etc.).
    pub config: Option<std::collections::HashMap<String, String>>,
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
}

impl fmt::Display for SourceKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SourceKind::Local => write!(f, "local"),
            SourceKind::S3 => write!(f, "s3"),
            SourceKind::WebDav => write!(f, "webdav"),
            SourceKind::AzureBlob => write!(f, "azure_blob"),
            SourceKind::Gcs => write!(f, "gcs"),
        }
    }
}

/// A single entry returned from listing or stat operations.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Entry {
    pub path: String,
    pub name: String,
    pub is_dir: bool,
    pub size: u64,
    pub modified_at: Option<String>,
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
}
