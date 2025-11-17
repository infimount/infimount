use serde::{Deserialize, Serialize};

use crate::models::{Result, SourceKind};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StorageFieldSchema {
    pub name: String,
    pub label: String,
    #[serde(default)]
    pub input_type: String,
    #[serde(default)]
    pub required: bool,
    #[serde(default)]
    pub secret: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StorageKindSchema {
    /// UI/storage type id, e.g. "aws-s3"
    pub id: String,
    pub label: String,
    pub kind: SourceKind,
    #[serde(default)]
    pub fields: Vec<StorageFieldSchema>,
}

pub fn list_storage_schemas() -> Result<Vec<StorageKindSchema>> {
    // For now schemas are embedded as a JSON blob in the binary.
    // This keeps things dynamic for the frontend without hard-coding
    // field definitions in TypeScript.
    const JSON: &str = include_str!("../storage_schemas.json");
    let items: Vec<StorageKindSchema> = serde_json::from_str(JSON)?;
    Ok(items)
}

