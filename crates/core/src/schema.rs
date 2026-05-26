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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn schemas_include_v0_5_backend_expansion_fields() {
        let schemas = list_storage_schemas().expect("schemas should parse");
        let b2 = schemas
            .iter()
            .find(|schema| schema.id == "backblaze-b2")
            .expect("Backblaze B2 schema should exist");
        assert!(matches!(b2.kind, SourceKind::B2));
        assert!(b2.fields.iter().any(|field| field.name == "bucketId"));

        let s3 = schemas
            .iter()
            .find(|schema| schema.id == "aws-s3")
            .expect("S3 schema should exist");
        assert!(s3.fields.iter().any(|field| field.name == "defaultAcl"));

        let webdav = schemas
            .iter()
            .find(|schema| schema.id == "webdav")
            .expect("WebDAV schema should exist");
        assert!(webdav
            .fields
            .iter()
            .any(|field| field.name == "disableCreateDir" && field.input_type == "checkbox"));
    }
}
