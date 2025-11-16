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

pub type Result<T> = std::result::Result<T, CoreError>;

/// A configured storage source (currently only local filesystem).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Source {
    pub id: String,
    pub name: String,
    pub kind: SourceKind,
    /// Root path for this source (for local filesystem).
    pub root: String,
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
