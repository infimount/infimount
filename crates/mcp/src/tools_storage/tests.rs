use super::import_config::{import_config, ImportConfigInput};
use super::*;
use crate::errors::McpErrorCode;
use crate::policy::McpAccessMode;
use crate::tools_fs::FsToolsContext;
use tempfile::TempDir;

fn registry_in(dir: &TempDir) -> crate::registry::StorageRegistry {
    crate::registry::StorageRegistry::with_secret_store(
        Some(dir.path().join("storages.json")),
        std::sync::Arc::new(infimount_core::secrets::MemorySecretStore::new()),
    )
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
    std::fs::write(
        registry.path(),
        serde_json::to_vec_pretty(&vec![storage]).unwrap(),
    )
    .unwrap();
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
        (
            "Google Drive",
            "gdrive",
            serde_json::json!({
                "refreshToken": "refresh-token",
                "clientId": "client-id",
                "clientSecret": "client-secret"
            }),
        ),
        (
            "OneDrive",
            "onedrive",
            serde_json::json!({
                "refreshToken": "refresh-token",
                "clientId": "client-id"
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
        ("google_drive", "gdrive"),
        ("one_drive", "onedrive"),
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
async fn export_config_is_shareable() {
    let dir = TempDir::new().unwrap();
    let registry = registry_in(&dir);
    let storage = crate::registry::StorageRecord::new(
        "S3".to_string(),
        "s3".to_string(),
        serde_json::json!({"token": "secret", "service_account_json": "raw-service-account-json", "region": "us-east-1"}),
    );
    std::fs::write(
        registry.path(),
        serde_json::to_vec_pretty(&vec![storage]).unwrap(),
    )
    .unwrap();
    let sessions = sessions_in();
    let ctx = FsToolsContext {
        registry,
        sessions,
        allow_insecure: true,
        auth_token: None,
    };

    let out = export_config(&ctx).await.unwrap();
    assert!(out
        .json
        .contains("\"kind\": \"infimount-shareable-config\""));
    assert!(out.json.contains("\"mcpExposed\": false"));
    assert!(out.json.contains("requiredSecretFields"));
    assert_eq!(
        super::import_config::secret_field_to_pointer(r"nested.a\.b.c/d~e"),
        "/nested/a.b/c~1d~0e"
    );
    assert!(!out.json.contains("\"id\":"));
    assert!(!out.json.contains("raw-service-account-json"));
    assert!(!out.json.contains("\"token\": \"secret\""));
    assert!(!out.json.contains("secret_ref"));
    assert!(!out.json.contains("secret_fields"));
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
async fn legacy_overwrite_increments_revision_without_bumping_untouched_records() {
    let dir = TempDir::new().unwrap();
    let registry = registry_in(&dir);
    let mut overwritten = crate::registry::StorageRecord::new(
        "Local".to_string(),
        "local".to_string(),
        serde_json::json!({"root": "/tmp/one"}),
    );
    overwritten.revision = 7;
    let mut untouched = crate::registry::StorageRecord::new(
        "Other".to_string(),
        "local".to_string(),
        serde_json::json!({"root": "/tmp/other"}),
    );
    untouched.revision = 4;
    registry
        .save_all_atomic(&[overwritten.clone(), untouched.clone()])
        .unwrap();
    let ctx = FsToolsContext {
        registry,
        sessions: sessions_in(),
        allow_insecure: true,
        auth_token: None,
    };

    import_config(
        &ctx,
        ImportConfigInput {
            json: serde_json::json!([{
                "name": "Local",
                "backend": "local",
                "config": {"root": "/tmp/two"}
            }])
            .to_string(),
            mode: "merge".to_string(),
            on_conflict: "overwrite".to_string(),
        },
    )
    .await
    .unwrap();

    let records = ctx.registry.load_all().unwrap();
    assert_eq!(
        records
            .iter()
            .find(|item| item.name == "Local")
            .unwrap()
            .revision,
        8
    );
    assert_eq!(
        records
            .iter()
            .find(|item| item.name == "Other")
            .unwrap()
            .revision,
        4
    );
}

#[tokio::test]
async fn shareable_preview_and_apply_honor_policy_secrets_and_exposure() {
    let dir = TempDir::new().unwrap();
    let registry = registry_in(&dir);
    let mut existing = crate::registry::StorageRecord::new(
        "Docs".to_string(),
        "s3".to_string(),
        serde_json::json!({"bucket": "old"}),
    );
    existing.mcp_exposed = true;
    existing.secret_ref = Some("storage/docs".to_string());
    existing.secret_fields = vec!["accessKeyId".to_string()];
    registry
        .secret_store()
        .put_json(
            "storage/docs",
            &serde_json::json!({"accessKeyId": "stored-key"}),
        )
        .unwrap();
    registry
        .save_all_atomic(std::slice::from_ref(&existing))
        .unwrap();
    let original_registry = std::fs::read(registry.path()).unwrap();
    let ctx = FsToolsContext {
        registry,
        sessions: sessions_in(),
        allow_insecure: true,
        auth_token: None,
    };

    let preview = preview_storage_import(
        &ctx,
        PreviewStorageImportInput {
            json: serde_json::json!({
            "schemaVersion": 2,
            "kind": "infimount-shareable-config",
            "storages": [{
                "name": "Docs",
                "backend": "s3",
                "config": {"bucket": "new", "secretAccessKey": "supplied-secret"},
                "requiredSecretFields": ["/accessKeyId", "/secretAccessKey"],
                "enabled": true,
                "mcpExposed": true,
                "readOnly": true,
                "mcpPolicy": {
                    "version": 2,
                    "default_access": "read_only",
                    "rules": [],
                    "denied_paths": ["private"],
                    "confirmation_rules": {
                        "require_for_write": true,
                        "require_for_overwrite": true,
                        "require_for_delete": true,
                        "require_for_version_delete": true,
                        "require_for_presign": true,
                        "require_for_cross_storage_copy": true
                    }
                }
            }]
            })
            .to_string(),
            mode: "merge".to_string(),
            on_conflict: "overwrite".to_string(),
        },
    )
    .await
    .unwrap();

    assert_eq!(preview.updates.len(), 1);
    assert_eq!(preview.policy_changes.len(), 1);
    assert_eq!(preview.exposure_changes.len(), 1);
    assert!(preview.missing_secret_fields.is_empty());
    assert_eq!(preview.warnings.len(), 1);

    let result = apply_storage_import(
        &ctx,
        ApplyStorageImportInput {
            preview_id: preview.preview_id,
            confirmed: false,
        },
    )
    .await
    .unwrap();
    assert_eq!(result.applied, 1);

    let stored = ctx.registry.load_all().unwrap().remove(0);
    assert_eq!(stored.config, serde_json::json!({"bucket": "new"}));
    assert_eq!(stored.revision, existing.revision + 1);
    assert!(!stored.mcp_exposed);
    assert!(stored.read_only);
    assert_eq!(stored.mcp_policy.default_access, McpAccessMode::ReadOnly);
    assert_eq!(stored.mcp_policy.denied_paths, vec!["private"]);
    assert_ne!(stored.secret_ref.as_deref(), Some("storage/docs"));
    let secret_bundle = ctx
        .registry
        .secret_store()
        .get_json(stored.secret_ref.as_deref().unwrap())
        .unwrap()
        .unwrap();
    assert_eq!(secret_bundle["accessKeyId"], "stored-key");
    assert_eq!(secret_bundle["secretAccessKey"], "supplied-secret");
    assert!(ctx
        .registry
        .secret_store()
        .get_json("storage/docs")
        .unwrap()
        .is_none());
    assert_eq!(
        stored.secret_fields,
        vec!["accessKeyId".to_string(), "secretAccessKey".to_string()]
    );

    let backups = std::fs::read_dir(dir.path().join("backups"))
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
    assert!(
        backups.is_empty(),
        "successful imports must remove the pending rollback journal"
    );
    assert_ne!(
        std::fs::read(ctx.registry.path()).unwrap(),
        original_registry
    );
}

#[tokio::test]
async fn full_registry_change_invalidates_shareable_preview() {
    let dir = TempDir::new().unwrap();
    let registry = registry_in(&dir);
    let existing = crate::registry::StorageRecord::new(
        "Local".to_string(),
        "local".to_string(),
        serde_json::json!({"root": "/tmp/one"}),
    );
    registry
        .save_all_atomic(std::slice::from_ref(&existing))
        .unwrap();
    let ctx = FsToolsContext {
        registry,
        sessions: sessions_in(),
        allow_insecure: true,
        auth_token: None,
    };
    let preview = preview_storage_import(
        &ctx,
        PreviewStorageImportInput {
            json: serde_json::json!({"storages": [{
                "name": "Other",
                "backend": "local",
                "config": {"root": "/tmp/other"}
            }]})
            .to_string(),
            mode: "merge".to_string(),
            on_conflict: "error".to_string(),
        },
    )
    .await
    .unwrap();

    let mut changed = existing;
    changed.config = serde_json::json!({"root": "/tmp/two"});
    // Deliberately retain the same per-record revision and timestamp. The full
    // registry snapshot, rather than max(revision), must invalidate the preview.
    ctx.registry.save_all_atomic(&[changed.clone()]).unwrap();

    let error = apply_storage_import(
        &ctx,
        ApplyStorageImportInput {
            preview_id: preview.preview_id,
            confirmed: false,
        },
    )
    .await
    .unwrap_err();
    assert_eq!(error.code, McpErrorCode::ERR_IMPORT_PREVIEW_STALE);
    assert_eq!(ctx.registry.load_all().unwrap()[0].config, changed.config);
}

#[tokio::test]
async fn expired_shareable_preview_cannot_be_applied() {
    let dir = TempDir::new().unwrap();
    let ctx = FsToolsContext {
        registry: registry_in(&dir),
        sessions: sessions_in(),
        allow_insecure: true,
        auth_token: None,
    };
    let preview = preview_storage_import(
        &ctx,
        PreviewStorageImportInput {
            json: serde_json::json!([{"name": "Local", "backend": "local", "config": {"root": "/tmp"}}])
                .to_string(),
            mode: "merge".to_string(),
            on_conflict: "error".to_string(),
        },
    )
    .await
    .unwrap();
    super::import_config::expire_storage_import_preview(&preview.preview_id);

    let error = apply_storage_import(
        &ctx,
        ApplyStorageImportInput {
            preview_id: preview.preview_id,
            confirmed: false,
        },
    )
    .await
    .unwrap_err();
    assert_eq!(error.code, McpErrorCode::ERR_IMPORT_PREVIEW_EXPIRED);
    assert!(ctx.registry.load_all().unwrap().is_empty());
}

#[tokio::test]
async fn missing_import_credentials_block_apply_without_consuming_preview() {
    let dir = TempDir::new().unwrap();
    let ctx = FsToolsContext {
        registry: registry_in(&dir),
        sessions: sessions_in(),
        allow_insecure: true,
        auth_token: None,
    };
    let preview = preview_storage_import(
        &ctx,
        PreviewStorageImportInput {
            json: serde_json::json!({"storages": [{
                "name": "S3",
                "backend": "s3",
                "config": {"bucket": "docs"},
                "requiredSecretFields": ["/secretAccessKey"]
            }]})
            .to_string(),
            mode: "merge".to_string(),
            on_conflict: "error".to_string(),
        },
    )
    .await
    .unwrap();
    assert_eq!(preview.missing_secret_fields[0].name, "/secretAccessKey");

    for _ in 0..2 {
        let error = apply_storage_import(
            &ctx,
            ApplyStorageImportInput {
                preview_id: preview.preview_id.clone(),
                confirmed: false,
            },
        )
        .await
        .unwrap_err();
        assert_eq!(error.code, McpErrorCode::ERR_SECRET_NOT_FOUND);
    }
    assert!(ctx.registry.load_all().unwrap().is_empty());
}

#[tokio::test]
async fn preview_is_bound_to_rename_strategy_and_replace_removals() {
    let dir = TempDir::new().unwrap();
    let registry = registry_in(&dir);
    let existing = crate::registry::StorageRecord::new(
        "Local".to_string(),
        "local".to_string(),
        serde_json::json!({"root": "/tmp/original"}),
    );
    registry.save_all_atomic(&[existing]).unwrap();
    let ctx = FsToolsContext {
        registry,
        sessions: sessions_in(),
        allow_insecure: true,
        auth_token: None,
    };
    let json =
        serde_json::json!([{"name": "Local", "backend": "local", "config": {"root": "/tmp/new"}}])
            .to_string();
    let rename = preview_storage_import(
        &ctx,
        PreviewStorageImportInput {
            json: json.clone(),
            mode: "merge".to_string(),
            on_conflict: "rename".to_string(),
        },
    )
    .await
    .unwrap();
    assert_eq!(rename.mode, "merge");
    assert_eq!(rename.on_conflict, "rename");
    assert_eq!(rename.renames.len(), 1);
    assert!(rename.removals.is_empty());

    let replace = preview_storage_import(
        &ctx,
        PreviewStorageImportInput {
            json: serde_json::json!([{"name": "Other", "backend": "local", "config": {"root": "/tmp/other"}}]).to_string(),
            mode: "replace".to_string(),
            on_conflict: "error".to_string(),
        },
    )
    .await
    .unwrap();
    assert_eq!(replace.removals.len(), 1);
}

#[tokio::test]
async fn desktop_import_validator_runs_before_registry_or_secret_mutation() {
    let dir = TempDir::new().unwrap();
    let registry = registry_in(&dir);
    let existing = crate::registry::StorageRecord::new(
        "Original".to_string(),
        "local".to_string(),
        serde_json::json!({"root": "/tmp/original"}),
    );
    registry
        .save_all_atomic(std::slice::from_ref(&existing))
        .unwrap();
    let original_bytes = std::fs::read(registry.path()).unwrap();
    let ctx = FsToolsContext {
        registry,
        sessions: sessions_in(),
        allow_insecure: true,
        auth_token: None,
    };
    let preview = preview_storage_import(
        &ctx,
        PreviewStorageImportInput {
            json: serde_json::json!([{"name": "Replacement", "backend": "local", "config": {"root": "/tmp/replacement"}}]).to_string(),
            mode: "replace".to_string(),
            on_conflict: "error".to_string(),
        },
    )
    .await
    .unwrap();

    let error = super::import_config::apply_storage_import_with_validator(
        &ctx,
        ApplyStorageImportInput {
            preview_id: preview.preview_id,
            confirmed: true,
        },
        |_| {
            Err(crate::errors::err(
                McpErrorCode::ERR_IMPORT_PREVIEW_MISMATCH,
                "workspace reference would be broken",
            ))
        },
    )
    .await
    .unwrap_err();
    assert_eq!(error.code, McpErrorCode::ERR_IMPORT_PREVIEW_MISMATCH);
    assert_eq!(std::fs::read(ctx.registry.path()).unwrap(), original_bytes);
    assert!(!dir.path().join("backups").exists());
}

#[tokio::test]
async fn pre_import_backup_failure_preserves_registry() {
    let dir = TempDir::new().unwrap();
    let registry = registry_in(&dir);
    let existing = crate::registry::StorageRecord::new(
        "Original".to_string(),
        "local".to_string(),
        serde_json::json!({"root": "/tmp/original"}),
    );
    registry
        .save_all_atomic(std::slice::from_ref(&existing))
        .unwrap();
    let original_bytes = std::fs::read(registry.path()).unwrap();
    let ctx = FsToolsContext {
        registry,
        sessions: sessions_in(),
        allow_insecure: true,
        auth_token: None,
    };
    let preview = preview_storage_import(
        &ctx,
        PreviewStorageImportInput {
            json: serde_json::json!([{"name": "Added", "backend": "local", "config": {"root": "/tmp/added"}}])
                .to_string(),
            mode: "merge".to_string(),
            on_conflict: "error".to_string(),
        },
    )
    .await
    .unwrap();
    std::fs::write(dir.path().join("backups"), b"not a directory").unwrap();

    apply_storage_import(
        &ctx,
        ApplyStorageImportInput {
            preview_id: preview.preview_id,
            confirmed: false,
        },
    )
    .await
    .unwrap_err();
    assert_eq!(std::fs::read(ctx.registry.path()).unwrap(), original_bytes);
    assert_eq!(ctx.registry.load_all().unwrap()[0].name, existing.name);
}

#[tokio::test]
async fn import_readback_failure_restores_registry_and_keeps_preview_recoverable() {
    let dir = TempDir::new().unwrap();
    let registry = registry_in(&dir);
    let original = crate::registry::StorageRecord::new(
        "Original".to_string(),
        "local".to_string(),
        serde_json::json!({"root": "/tmp/original"}),
    );
    registry
        .save_all_atomic(std::slice::from_ref(&original))
        .unwrap();
    let ctx = FsToolsContext {
        registry,
        sessions: sessions_in(),
        allow_insecure: true,
        auth_token: None,
    };
    let preview = preview_storage_import(
        &ctx,
        PreviewStorageImportInput {
            json: serde_json::json!([{"name": "Replacement", "backend": "local", "config": {"root": "/tmp/replacement"}}]).to_string(),
            mode: "replace".to_string(),
            on_conflict: "error".to_string(),
        },
    )
    .await
    .unwrap();
    ctx.registry.fail_next_import_readback();
    let error = apply_storage_import(
        &ctx,
        ApplyStorageImportInput {
            preview_id: preview.preview_id,
            confirmed: true,
        },
    )
    .await
    .unwrap_err();
    assert_eq!(error.code, McpErrorCode::ERR_INTERNAL);
    assert_eq!(ctx.registry.load_all().unwrap()[0].name, original.name);
}

#[test]
fn import_race_child() {
    let Some(path) = std::env::var_os("INFIMOUNT_IMPORT_RACE_PATH") else {
        return;
    };
    let registry = crate::registry::StorageRegistry::with_secret_store(
        Some(path.into()),
        std::sync::Arc::new(infimount_core::secrets::MemorySecretStore::new()),
    );
    let mut current = registry.load_all().unwrap();
    current[0].name = "Later process mutation".to_string();
    current[0].revision += 1;
    registry.save_all_atomic(&current).unwrap();
}

#[test]
fn conditional_import_rollback_preserves_later_process_mutation() {
    let dir = TempDir::new().unwrap();
    let first = registry_in(&dir);
    let original = crate::registry::StorageRecord::new(
        "Original".to_string(),
        "local".to_string(),
        serde_json::json!({"root": "/tmp/original"}),
    );
    first
        .save_all_atomic(std::slice::from_ref(&original))
        .unwrap();
    let mut imported = original.clone();
    imported.name = "Imported".to_string();
    imported.revision += 1;
    first
        .save_all_atomic_if_unchanged(
            std::slice::from_ref(&original),
            std::slice::from_ref(&imported),
        )
        .unwrap();
    let status = std::process::Command::new(std::env::current_exe().unwrap())
        .arg("--exact")
        .arg("tools_storage::tests::import_race_child")
        .arg("--nocapture")
        .env("INFIMOUNT_IMPORT_RACE_PATH", first.path())
        .status()
        .unwrap();
    assert!(
        status.success(),
        "later-process mutation child failed: {status}"
    );
    assert!(!first
        .restore_all_if_matches(
            std::slice::from_ref(&imported),
            std::slice::from_ref(&original)
        )
        .unwrap());
    assert_eq!(first.load_all().unwrap()[0].name, "Later process mutation");
}

#[tokio::test]
async fn validate_storage_local_root_succeeds() {
    let dir = TempDir::new().unwrap();
    let local_root = dir.path().join("local");
    std::fs::create_dir_all(&local_root).unwrap();

    let registry = registry_in(&dir);
    let mut storage = crate::registry::StorageRecord::new(
        "Local".to_string(),
        "local".to_string(),
        serde_json::json!({"root": local_root}),
    );
    storage.mcp_exposed = true;
    storage.mcp_policy.default_access = McpAccessMode::ReadWrite;
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
    let mut storage = crate::registry::StorageRecord::new(
        "Broken".to_string(),
        "local".to_string(),
        serde_json::json!({"root": invalid_root}),
    );
    storage.mcp_exposed = true;
    storage.mcp_policy.default_access = McpAccessMode::ReadWrite;
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
