use super::*;
use crate::errors::McpErrorCode;
use crate::tools_fs::FsToolsContext;
use tempfile::TempDir;

fn registry_in(dir: &TempDir) -> crate::registry::StorageRegistry {
    crate::registry::StorageRegistry::new(Some(dir.path().join("storages.json")))
}

fn sessions_in() -> crate::session::SessionManager {
    crate::session::SessionManager::new()
}

#[tokio::test]
async fn list_storages_masks_secrets() {
    let dir = TempDir::new().unwrap();
    let registry = registry_in(&dir);
    let storage = crate::registry::StorageRecord::new(
        "S3".to_string(),
        "s3".to_string(),
        serde_json::json!({"access_key": "abc", "service_account_json": "{}", "region": "us-east-1"}),
    );
    registry.save_all_atomic(&[storage]).unwrap();
    let sessions = sessions_in();
    let ctx = FsToolsContext {
        registry,
        sessions,
        allow_insecure: true,
        auth_token: None,
    };

    let out = list_storages(&ctx).await.unwrap();
    assert_eq!(out.storages[0].config["access_key"], "********");
    assert_eq!(out.storages[0].config["service_account_json"], "********");
    assert_eq!(out.storages[0].config["region"], "us-east-1");
}

#[tokio::test]
async fn add_edit_remove_storage_round_trip() {
    let dir = TempDir::new().unwrap();
    let registry = registry_in(&dir);
    let sessions = sessions_in();
    let ctx = FsToolsContext {
        registry,
        sessions,
        allow_insecure: true,
        auth_token: None,
    };

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
async fn add_storage_defaults_to_not_mcp_exposed_when_omitted() {
    let dir = TempDir::new().unwrap();
    let registry = registry_in(&dir);
    let sessions = sessions_in();
    let ctx = FsToolsContext {
        registry,
        sessions,
        allow_insecure: true,
        auth_token: None,
    };

    let input: AddStorageInput = serde_json::from_value(serde_json::json!({
        "name": "Local",
        "backend": "local",
        "config": { "root": "/tmp" }
    }))
    .unwrap();

    let out = add_storage(&ctx, input).await.unwrap();
    assert!(!out.storage.mcp_exposed);
}

#[tokio::test]
async fn add_storage_rejects_unsupported_backend() {
    let dir = TempDir::new().unwrap();
    let registry = registry_in(&dir);
    let sessions = sessions_in();
    let ctx = FsToolsContext {
        registry,
        sessions,
        allow_insecure: true,
        auth_token: None,
    };

    let err = add_storage(
        &ctx,
        AddStorageInput {
            name: "Unsupported".to_string(),
            backend: "dropbox".to_string(),
            config: serde_json::json!({}),
            enabled: true,
            mcp_exposed: false,
            read_only: false,
        },
    )
    .await
    .unwrap_err();

    assert_eq!(err.code, McpErrorCode::ERR_BACKEND_UNSUPPORTED);
}

#[tokio::test]
async fn storage_management_accepts_v0_7_and_remote_file_backends() {
    let dir = TempDir::new().unwrap();
    let registry = registry_in(&dir);
    let sessions = sessions_in();
    let ctx = FsToolsContext {
        registry,
        sessions,
        allow_insecure: true,
        auth_token: None,
    };

    for (name, backend, config) in [
        (
            "OSS",
            "oss",
            serde_json::json!({
                "bucket": "bucket-name",
                "endpoint": "https://oss-cn-beijing.aliyuncs.com",
                "accessKeyId": "key-id",
                "accessKeySecret": "key-secret"
            }),
        ),
        (
            "COS",
            "cos",
            serde_json::json!({
                "bucket": "bucket-name",
                "endpoint": "https://cos.ap-singapore.myqcloud.com",
                "secretId": "secret-id",
                "secretKey": "secret-key"
            }),
        ),
        (
            "OBS",
            "obs",
            serde_json::json!({
                "bucket": "bucket-name",
                "endpoint": "https://obs.cn-north-4.myhuaweicloud.com",
                "accessKeyId": "key-id",
                "secretAccessKey": "key-secret"
            }),
        ),
        (
            "SFTP",
            "sftp",
            serde_json::json!({
                "endpoint": "ssh://example.com:22",
                "user": "alice",
                "privateKeyPath": "/home/alice/.ssh/id_ed25519"
            }),
        ),
        (
            "FTP",
            "ftp",
            serde_json::json!({
                "endpoint": "ftp://example.com:21",
                "user": "alice",
                "password": "password"
            }),
        ),
    ] {
        let out = add_storage(
            &ctx,
            AddStorageInput {
                name: name.to_string(),
                backend: backend.to_string(),
                config,
                enabled: true,
                mcp_exposed: false,
                read_only: false,
            },
        )
        .await
        .unwrap();
        assert_eq!(out.storage.backend, backend);
    }
}

#[tokio::test]
async fn storage_management_canonicalizes_backend_aliases() {
    let dir = TempDir::new().unwrap();
    let registry = registry_in(&dir);
    let sessions = sessions_in();
    let ctx = FsToolsContext {
        registry,
        sessions,
        allow_insecure: true,
        auth_token: None,
    };

    for (alias, canonical) in [
        ("fs", "local"),
        ("backblaze_b2", "b2"),
        ("azblob", "azure_blob"),
        ("aliyun_oss", "oss"),
        ("tencent_cos", "cos"),
        ("huawei_obs", "obs"),
    ] {
        let out = add_storage(
            &ctx,
            AddStorageInput {
                name: format!("Storage {alias}"),
                backend: alias.to_string(),
                config: serde_json::json!({"root": "/tmp"}),
                enabled: true,
                mcp_exposed: false,
                read_only: false,
            },
        )
        .await
        .unwrap();
        assert_eq!(out.storage.backend, canonical);
    }

    let edited = edit_storage(
        &ctx,
        EditStorageInput {
            name: "Storage fs".to_string(),
            patch: EditStoragePatch {
                backend: Some("aliyun_oss".to_string()),
                config: None,
                enabled: None,
                mcp_exposed: None,
                read_only: None,
                new_name: None,
            },
        },
    )
    .await
    .unwrap();
    assert_eq!(edited.storage.backend, "oss");
}

#[tokio::test]
async fn import_config_defaults_to_not_mcp_exposed() {
    let dir = TempDir::new().unwrap();
    let registry = registry_in(&dir);
    let sessions = sessions_in();
    let ctx = FsToolsContext {
        registry,
        sessions,
        allow_insecure: true,
        auth_token: None,
    };

    let out = import_config(
        &ctx,
        ImportConfigInput {
            json: serde_json::json!([
                {
                    "name": "OSS",
                    "backend": "aliyun_oss",
                    "config": {
                        "bucket": "bucket-name",
                        "endpoint": "https://oss-cn-beijing.aliyuncs.com"
                    }
                }
            ])
            .to_string(),
            mode: "merge".to_string(),
            on_conflict: "error".to_string(),
        },
    )
    .await
    .unwrap();

    assert_eq!(out.imported, 1);
    assert_eq!(out.storages[0].backend, "oss");
    assert!(!out.storages[0].mcp_exposed);
}

#[tokio::test]
async fn export_config_masks_by_default() {
    let dir = TempDir::new().unwrap();
    let registry = registry_in(&dir);
    let storage = crate::registry::StorageRecord::new(
        "S3".to_string(),
        "s3".to_string(),
        serde_json::json!({"token": "secret", "service_account_json": "raw-service-account-json", "region": "us-east-1"}),
    );
    registry.save_all_atomic(&[storage]).unwrap();
    let sessions = sessions_in();
    let ctx = FsToolsContext {
        registry,
        sessions,
        allow_insecure: true,
        auth_token: None,
    };

    let out = export_config(
        &ctx,
        ExportConfigInput {
            include_secrets: false,
        },
    )
    .await
    .unwrap();
    assert!(out.json.contains("********"));
    assert!(!out.json.contains("service-account-json"));
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
    let sessions = sessions_in();
    let ctx = FsToolsContext {
        registry,
        sessions,
        allow_insecure: true,
        auth_token: None,
    };

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
    let sessions = sessions_in();
    let ctx = FsToolsContext {
        registry,
        sessions,
        allow_insecure: true,
        auth_token: None,
    };

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
    assert!(out.fix_hints.is_empty());
    assert!(out
        .warnings
        .iter()
        .any(|warning| warning.contains("writable")));
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
    let sessions = sessions_in();
    let ctx = FsToolsContext {
        registry,
        sessions,
        allow_insecure: true,
        auth_token: None,
    };

    let out = validate_storage(
        &ctx,
        ValidateStorageInput {
            name: "Broken".to_string(),
        },
    )
    .await
    .unwrap();

    assert!(!out.valid);
    assert_eq!(out.details, "local root is not an existing directory");
    assert!(out
        .fix_hints
        .iter()
        .any(|hint| hint.contains("existing folder")));
}

#[tokio::test]
async fn remove_storage_missing_returns_not_found() {
    let dir = TempDir::new().unwrap();
    let registry = registry_in(&dir);
    let sessions = sessions_in();
    let ctx = FsToolsContext {
        registry,
        sessions,
        allow_insecure: true,
        auth_token: None,
    };

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
