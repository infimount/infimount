use super::*;
use crate::errors::McpErrorCode;
use crate::tools_fs::FsToolsContext;
use tempfile::TempDir;

fn registry_in(dir: &TempDir) -> crate::registry::StorageRegistry {
    crate::registry::StorageRegistry::new(Some(dir.path().join("storages.json")))
}

#[tokio::test]
async fn list_storages_masks_secrets() {
    let dir = TempDir::new().unwrap();
    let registry = registry_in(&dir);
    let storage = crate::registry::StorageRecord::new(
        "S3".to_string(),
        "s3".to_string(),
        serde_json::json!({"access_key": "abc", "region": "us-east-1"}),
    );
    registry.save_all_atomic(&[storage]).unwrap();
    let ctx = FsToolsContext { registry };

    let out = list_storages(&ctx).await.unwrap();
    assert_eq!(out.storages[0].config["access_key"], "********");
    assert_eq!(out.storages[0].config["region"], "us-east-1");
}

#[tokio::test]
async fn add_edit_remove_storage_round_trip() {
    let dir = TempDir::new().unwrap();
    let registry = registry_in(&dir);
    let ctx = FsToolsContext { registry };

    let added = add_storage(
        &ctx,
        AddStorageInput {
            name: "  Local  ".to_string(),
            backend: "local".to_string(),
            config: serde_json::json!({"root": "/tmp"}),
            enabled: true,
            mcp_exposed: true,
            read_only: false,
        },
    )
    .await
    .unwrap();
    assert_eq!(added.storage.name, "Local");

    let edited = edit_storage(
        &ctx,
        EditStorageInput {
            name: "Local".to_string(),
            patch: EditStoragePatch {
                backend: None,
                config: None,
                enabled: Some(false),
                mcp_exposed: Some(false),
                read_only: Some(true),
                new_name: Some("Archive".to_string()),
            },
        },
    )
    .await
    .unwrap();
    assert_eq!(edited.storage.name, "Archive");
    assert!(!edited.storage.enabled);
    assert!(!edited.storage.mcp_exposed);
    assert!(edited.storage.read_only);

    let removed = remove_storage(
        &ctx,
        RemoveStorageInput {
            name: "Archive".to_string(),
        },
    )
    .await
    .unwrap();
    assert!(removed.removed);
}

#[tokio::test]
async fn export_config_masks_by_default() {
    let dir = TempDir::new().unwrap();
    let registry = registry_in(&dir);
    let storage = crate::registry::StorageRecord::new(
        "S3".to_string(),
        "s3".to_string(),
        serde_json::json!({"token": "secret", "region": "us-east-1"}),
    );
    registry.save_all_atomic(&[storage]).unwrap();
    let ctx = FsToolsContext { registry };

    let out = export_config(
        &ctx,
        ExportConfigInput {
            include_secrets: false,
        },
    )
    .await
    .unwrap();
    assert!(out.json.contains("********"));
    assert!(out.json.contains("us-east-1"));
}

#[tokio::test]
async fn import_config_rename_conflict_appends_suffix() {
    let dir = TempDir::new().unwrap();
    let registry = registry_in(&dir);
    let existing = crate::registry::StorageRecord::new(
        "Local".to_string(),
        "local".to_string(),
        serde_json::json!({"root": "/tmp/one"}),
    );
    registry.save_all_atomic(&[existing]).unwrap();
    let ctx = FsToolsContext { registry };

    let out = import_config(
        &ctx,
        ImportConfigInput {
            json: serde_json::json!([
                {
                    "name": "Local",
                    "backend": "local",
                    "config": { "root": "/tmp/two" }
                }
            ])
            .to_string(),
            mode: "merge".to_string(),
            on_conflict: "rename".to_string(),
        },
    )
    .await
    .unwrap();

    let names = out
        .storages
        .iter()
        .map(|storage| storage.name.clone())
        .collect::<Vec<_>>();
    assert!(names.contains(&"Local".to_string()));
    assert!(names.contains(&"Local (2)".to_string()));
}

#[tokio::test]
async fn validate_storage_local_root_succeeds() {
    let dir = TempDir::new().unwrap();
    let local_root = dir.path().join("local");
    std::fs::create_dir_all(&local_root).unwrap();

    let registry = registry_in(&dir);
    let storage = crate::registry::StorageRecord::new(
        "Local".to_string(),
        "local".to_string(),
        serde_json::json!({"root": local_root}),
    );
    registry.save_all_atomic(&[storage]).unwrap();
    let ctx = FsToolsContext { registry };

    let out = validate_storage(
        &ctx,
        ValidateStorageInput {
            name: "Local".to_string(),
        },
    )
    .await
    .unwrap();

    assert!(out.valid);
    assert!(out.capabilities.read);
}

#[tokio::test]
async fn validate_storage_invalid_root_returns_valid_false() {
    let dir = TempDir::new().unwrap();
    let invalid_root = dir.path().join("not-a-directory.txt");
    std::fs::write(&invalid_root, b"not a directory").unwrap();

    let registry = registry_in(&dir);
    let storage = crate::registry::StorageRecord::new(
        "Broken".to_string(),
        "local".to_string(),
        serde_json::json!({"root": invalid_root}),
    );
    registry.save_all_atomic(&[storage]).unwrap();
    let ctx = FsToolsContext { registry };

    let out = validate_storage(
        &ctx,
        ValidateStorageInput {
            name: "Broken".to_string(),
        },
    )
    .await
    .unwrap();

    assert!(!out.valid);
    assert_eq!(out.details, "storage validation failed");
}

#[tokio::test]
async fn remove_storage_missing_returns_not_found() {
    let dir = TempDir::new().unwrap();
    let registry = registry_in(&dir);
    let ctx = FsToolsContext { registry };

    let err = remove_storage(
        &ctx,
        RemoveStorageInput {
            name: "Missing".to_string(),
        },
    )
    .await
    .unwrap_err();

    assert_eq!(err.code, McpErrorCode::ERR_STORAGE_NOT_FOUND);
}
