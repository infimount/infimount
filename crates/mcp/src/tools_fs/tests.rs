use super::*;
use crate::errors::McpErrorCode;
use crate::policy::{McpAccessMode, McpPathRule, McpRuleSource, McpStoragePolicy};
use crate::registry::StorageRecord;
use serde_json::json;
use std::collections::HashMap;
use tempfile::TempDir;

fn registry_in(dir: &TempDir) -> crate::registry::StorageRegistry {
    crate::registry::StorageRegistry::new(Some(dir.path().join("storages.json")))
}

fn sessions_in() -> crate::session::SessionManager {
    crate::session::SessionManager::new()
}

#[tokio::test]
async fn path_policy_denies_reads_and_writes_before_backend_operation() {
    let dir = TempDir::new().unwrap();
    let local_root = dir.path().join("local");
    std::fs::create_dir_all(local_root.join("private")).unwrap();
    std::fs::write(local_root.join("private").join("secret.txt"), b"secret").unwrap();

    let registry = registry_in(&dir);
    let mut storage = StorageRecord::new(
        "Local".to_string(),
        "local".to_string(),
        json!({"root": local_root}),
    );
    storage.mcp_exposed = true;
    storage.mcp_policy = McpStoragePolicy {
        default_access: McpAccessMode::ReadWrite,
        version: 2,
        denied_paths: vec!["private".to_string()],
        ..Default::default()
    };
    registry.save_all_atomic(&[storage]).unwrap();

    let ctx = FsToolsContext {
        registry,
        sessions: sessions_in(),
        allow_insecure: true,
        auth_token: None,
    };

    let read_error = read_file(
        &ctx,
        ReadFileInput {
            path: "/Local/private/secret.txt".to_string(),
            offset_bytes: 0,
            max_bytes: 1024,
            as_text: true,
            encoding: "utf-8".to_string(),
            session_id: None,
        },
    )
    .await
    .unwrap_err();
    assert_eq!(read_error.code, McpErrorCode::ERR_MCP_POLICY_DENIED);

    let write_error = write_file(
        &ctx,
        WriteFileInput {
            path: "/Local/private/new.txt".to_string(),
            content: "nope".to_string(),
            encoding: "utf-8".to_string(),
            overwrite: true,
            create_parents: false,
            confirmation_id: None,
            session_id: None,
            user_metadata: None,
        },
    )
    .await
    .unwrap_err();
    assert_eq!(write_error.code, McpErrorCode::ERR_MCP_POLICY_DENIED);
    assert!(!dir.path().join("local/private/new.txt").exists());
}

#[tokio::test]
async fn recursive_operations_enforce_denied_descendant_policy() {
    let dir = TempDir::new().unwrap();
    let src_root = dir.path().join("src");
    let dst_root = dir.path().join("dst");
    std::fs::create_dir_all(src_root.join("public").join("private")).unwrap();
    std::fs::create_dir_all(&dst_root).unwrap();
    std::fs::write(src_root.join("public").join("visible.txt"), "visible").unwrap();
    std::fs::write(
        src_root.join("public").join("private").join("secret.txt"),
        "secret",
    )
    .unwrap();

    let registry = registry_in(&dir);
    let mut src = StorageRecord::new(
        "Src".to_string(),
        "local".to_string(),
        json!({"root": src_root.clone()}),
    );
    src.mcp_exposed = true;
    src.mcp_policy = McpStoragePolicy {
        default_access: McpAccessMode::None,
        version: 2,
        rules: vec![McpPathRule {
            id: "public".to_string(),
            prefix: "public".to_string(),
            access: McpAccessMode::ReadWrite,
            source: McpRuleSource::Manual,
            confirmation_rules: None,
        }],
        denied_paths: vec!["public/private".to_string()],
        ..Default::default()
    };
    let mut dst = StorageRecord::new(
        "Dst".to_string(),
        "local".to_string(),
        json!({"root": dst_root.clone()}),
    );
    dst.mcp_exposed = true;
    dst.mcp_policy.default_access = McpAccessMode::ReadWrite;
    registry.save_all_atomic(&[src, dst]).unwrap();
    let ctx = FsToolsContext {
        registry,
        sessions: sessions_in(),
        allow_insecure: true,
        auth_token: None,
    };

    let listed = list_dir(
        &ctx,
        ListDirInput {
            session_id: None,
            path: "/Src/public".to_string(),
            recursive: true,
            limit: 200,
            cursor: None,
        },
    )
    .await
    .unwrap();
    let listed_paths = listed
        .entries
        .into_iter()
        .map(|entry| entry.path)
        .collect::<Vec<_>>();
    assert!(listed_paths.contains(&"/Src/public/visible.txt".to_string()));
    assert!(!listed_paths.iter().any(|path| path.contains("private")));

    let searched = search_paths(
        &ctx,
        SearchPathsInput {
            session_id: None,
            path: "/Src/public".to_string(),
            pattern: "secret".to_string(),
            max_results: 10,
        },
    )
    .await
    .unwrap();
    assert!(searched.matches.is_empty());

    let copy_error = copy_path(
        &ctx,
        CopyPathInput {
            session_id: None,
            confirmation_id: None,
            src: "/Src/public".to_string(),
            dst: "/Dst/public".to_string(),
            overwrite: false,
            recursive: true,
        },
    )
    .await
    .unwrap_err();
    assert_eq!(copy_error.code, McpErrorCode::ERR_MCP_POLICY_DENIED);
    assert!(!dst_root.join("public").exists());

    std::fs::create_dir_all(dst_root.join("public")).unwrap();
    std::fs::write(dst_root.join("public").join("keep.txt"), "keep").unwrap();
    let overwrite_copy_error = copy_path(
        &ctx,
        CopyPathInput {
            session_id: None,
            confirmation_id: None,
            src: "/Src/public".to_string(),
            dst: "/Dst/public".to_string(),
            overwrite: true,
            recursive: true,
        },
    )
    .await
    .unwrap_err();
    assert_eq!(
        overwrite_copy_error.code,
        McpErrorCode::ERR_MCP_POLICY_DENIED
    );
    assert_eq!(
        std::fs::read_to_string(dst_root.join("public").join("keep.txt")).unwrap(),
        "keep"
    );

    let delete_error = delete_path(
        &ctx,
        DeletePathInput {
            session_id: None,
            confirmation_id: None,
            path: "/Src/public".to_string(),
            recursive: true,
        },
    )
    .await
    .unwrap_err();
    assert_eq!(delete_error.code, McpErrorCode::ERR_MCP_POLICY_DENIED);
    assert!(src_root.join("public").join("visible.txt").exists());
    assert!(src_root
        .join("public")
        .join("private")
        .join("secret.txt")
        .exists());
}

#[tokio::test]
async fn copy_path_recursive_enforces_destination_descendant_policy_before_mutation() {
    let dir = TempDir::new().unwrap();
    let src_root = dir.path().join("src");
    let dst_root = dir.path().join("dst");
    std::fs::create_dir_all(src_root.join("public").join("private")).unwrap();
    std::fs::create_dir_all(&dst_root).unwrap();
    std::fs::write(src_root.join("public").join("visible.txt"), "visible").unwrap();
    std::fs::write(
        src_root.join("public").join("private").join("secret.txt"),
        "secret",
    )
    .unwrap();

    let registry = registry_in(&dir);
    let mut src = StorageRecord::new(
        "Src".to_string(),
        "local".to_string(),
        json!({"root": src_root.clone()}),
    );
    src.mcp_exposed = true;
    src.mcp_policy.default_access = McpAccessMode::ReadWrite;
    let mut dst = StorageRecord::new(
        "Dst".to_string(),
        "local".to_string(),
        json!({"root": dst_root.clone()}),
    );
    dst.mcp_exposed = true;
    dst.mcp_policy = McpStoragePolicy {
        default_access: McpAccessMode::None,
        version: 2,
        rules: vec![McpPathRule {
            id: "public".to_string(),
            prefix: "public".to_string(),
            access: McpAccessMode::ReadWrite,
            source: McpRuleSource::Manual,
            confirmation_rules: None,
        }],
        denied_paths: vec!["public/private".to_string()],
        ..Default::default()
    };
    registry.save_all_atomic(&[src, dst]).unwrap();
    let ctx = FsToolsContext {
        registry,
        sessions: sessions_in(),
        allow_insecure: true,
        auth_token: None,
    };

    let err = copy_path(
        &ctx,
        CopyPathInput {
            session_id: None,
            confirmation_id: None,
            src: "/Src/public".to_string(),
            dst: "/Dst/public".to_string(),
            overwrite: false,
            recursive: true,
        },
    )
    .await
    .unwrap_err();

    assert_eq!(err.code, McpErrorCode::ERR_MCP_POLICY_DENIED);
    assert!(!dst_root.join("public").join("visible.txt").exists());
    assert!(!dst_root.join("public").join("private").exists());
}

#[tokio::test]
async fn list_dir_root_is_sorted_and_filtered() {
    let dir = TempDir::new().unwrap();
    let registry = registry_in(&dir);
    let mut a = StorageRecord::new(
        "zeta".to_string(),
        "local".to_string(),
        json!({"root": "/tmp"}),
    );
    a.enabled = true;
    a.mcp_exposed = true;
    a.mcp_policy.default_access = McpAccessMode::ReadWrite;

    let mut b = StorageRecord::new(
        "alpha".to_string(),
        "local".to_string(),
        json!({"root": "/tmp"}),
    );
    b.enabled = true;
    b.mcp_exposed = true;
    b.mcp_policy.default_access = McpAccessMode::ReadWrite;

    let mut hidden = StorageRecord::new(
        "hidden".to_string(),
        "local".to_string(),
        json!({"root": "/tmp"}),
    );
    hidden.enabled = true;
    hidden.mcp_exposed = false;

    registry.save_all_atomic(&[a, hidden, b]).unwrap();

    let sessions = sessions_in();
    let ctx = FsToolsContext {
        registry,
        sessions,
        allow_insecure: true,
        auth_token: None,
    };
    let out = list_dir(
        &ctx,
        ListDirInput {
            session_id: None,
            path: "/".to_string(),
            recursive: false,
            limit: 200,
            cursor: None,
        },
    )
    .await
    .unwrap();

    let names = out
        .entries
        .iter()
        .map(|e| e.name.clone())
        .collect::<Vec<_>>();
    assert_eq!(names, vec!["alpha", "zeta"]);
}

#[tokio::test]
async fn list_dir_storage_dirs_first_then_files() {
    let dir = TempDir::new().unwrap();
    let local_root = dir.path().join("local");
    std::fs::create_dir_all(local_root.join("docs")).unwrap();
    std::fs::write(local_root.join("b.txt"), b"b").unwrap();
    std::fs::write(local_root.join("a.txt"), b"a").unwrap();

    let registry = registry_in(&dir);
    let mut storage = StorageRecord::new(
        "Local".to_string(),
        "local".to_string(),
        json!({"root": local_root}),
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
    let out = list_dir(
        &ctx,
        ListDirInput {
            session_id: None,
            path: "/Local".to_string(),
            recursive: false,
            limit: 200,
            cursor: None,
        },
    )
    .await
    .unwrap();

    let names = out
        .entries
        .iter()
        .map(|e| e.name.clone())
        .collect::<Vec<_>>();
    assert_eq!(names, vec!["docs", "a.txt", "b.txt"]);
    assert_eq!(out.entries[0].entry_type, EntryType::Dir);
}

#[tokio::test]
async fn list_dir_recursive_is_flat_and_sorted_by_full_path() {
    let dir = TempDir::new().unwrap();
    let local_root = dir.path().join("local");
    std::fs::create_dir_all(local_root.join("docs")).unwrap();
    std::fs::write(local_root.join("z.txt"), b"z").unwrap();
    std::fs::write(local_root.join("docs").join("a.txt"), b"a").unwrap();

    let registry = registry_in(&dir);
    let mut storage = StorageRecord::new(
        "Local".to_string(),
        "local".to_string(),
        json!({"root": local_root}),
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
    let out = list_dir(
        &ctx,
        ListDirInput {
            session_id: None,
            path: "/Local".to_string(),
            recursive: true,
            limit: 200,
            cursor: None,
        },
    )
    .await
    .unwrap();

    let paths = out
        .entries
        .iter()
        .map(|e| e.path.clone())
        .collect::<Vec<_>>();
    assert_eq!(
        paths,
        vec!["/Local/docs", "/Local/docs/a.txt", "/Local/z.txt"]
    );
}

#[tokio::test]
async fn list_dir_recursive_pages_are_bounded_and_filter_denied_nested_paths() {
    let dir = TempDir::new().unwrap();
    let local_root = dir.path().join("local");
    for index in 0..75 {
        std::fs::create_dir_all(local_root.join("allowed/nested")).unwrap();
        std::fs::create_dir_all(local_root.join("private/nested")).unwrap();
        std::fs::write(
            local_root.join(format!("allowed/nested/{index:03}.txt")),
            b"visible",
        )
        .unwrap();
        std::fs::write(
            local_root.join(format!("private/nested/{index:03}.txt")),
            b"hidden",
        )
        .unwrap();
    }

    let registry = registry_in(&dir);
    let mut storage = StorageRecord::new(
        "Local".to_string(),
        "local".to_string(),
        json!({"root": local_root}),
    );
    storage.mcp_exposed = true;
    storage.mcp_policy.default_access = McpAccessMode::ReadWrite;
    storage.mcp_policy.denied_paths = vec!["private".to_string()];
    registry.save_all_atomic(&[storage]).unwrap();
    let ctx = FsToolsContext {
        registry,
        sessions: sessions_in(),
        allow_insecure: true,
        auth_token: None,
    };

    let mut cursor = None;
    let mut paths = Vec::new();
    let mut pages = 0usize;
    loop {
        let page = list_dir(
            &ctx,
            ListDirInput {
                session_id: None,
                path: "/Local".to_string(),
                recursive: true,
                limit: 11,
                cursor,
            },
        )
        .await
        .unwrap();
        assert!(page.entries.len() <= 11);
        assert!(page
            .entries
            .iter()
            .all(|entry| !entry.path.contains("/private")));
        paths.extend(page.entries.into_iter().map(|entry| entry.path));
        cursor = page.next_cursor;
        pages += 1;
        assert!(pages < 20, "cursor did not make bounded progress");
        if cursor.is_none() {
            break;
        }
    }
    assert!(paths.contains(&"/Local/allowed/nested/074.txt".to_string()));
    let unique = paths.iter().collect::<std::collections::HashSet<_>>();
    assert_eq!(unique.len(), paths.len());
}

#[tokio::test]
async fn list_dir_cursor_offset_applies_after_sorting() {
    let dir = TempDir::new().unwrap();
    let registry = registry_in(&dir);

    let mut s1 = StorageRecord::new(
        "zeta".to_string(),
        "local".to_string(),
        json!({"root": "/tmp"}),
    );
    s1.mcp_exposed = true;
    s1.mcp_policy.default_access = McpAccessMode::ReadWrite;
    let mut s2 = StorageRecord::new(
        "alpha".to_string(),
        "local".to_string(),
        json!({"root": "/tmp"}),
    );
    s2.mcp_exposed = true;
    s2.mcp_policy.default_access = McpAccessMode::ReadWrite;
    let mut s3 = StorageRecord::new(
        "beta".to_string(),
        "local".to_string(),
        json!({"root": "/tmp"}),
    );
    s3.mcp_exposed = true;
    s3.mcp_policy.default_access = McpAccessMode::ReadWrite;
    registry.save_all_atomic(&[s1, s2, s3]).unwrap();
    let sessions = sessions_in();
    let ctx = FsToolsContext {
        registry,
        sessions,
        allow_insecure: true,
        auth_token: None,
    };

    let first = list_dir(
        &ctx,
        ListDirInput {
            session_id: None,
            path: "/".to_string(),
            recursive: false,
            limit: 1,
            cursor: None,
        },
    )
    .await
    .unwrap();
    assert_eq!(first.entries[0].name, "alpha");
    let cursor = first.next_cursor.clone().unwrap();

    let second = list_dir(
        &ctx,
        ListDirInput {
            session_id: None,
            path: "/".to_string(),
            recursive: false,
            limit: 1,
            cursor: Some(cursor),
        },
    )
    .await
    .unwrap();
    assert_eq!(second.entries[0].name, "beta");
}

#[tokio::test]
async fn malformed_cursor_returns_invalid_path() {
    let dir = TempDir::new().unwrap();
    let registry = registry_in(&dir);
    let sessions = sessions_in();
    let ctx = FsToolsContext {
        registry,
        sessions,
        allow_insecure: true,
        auth_token: None,
    };

    let err = list_dir(
        &ctx,
        ListDirInput {
            session_id: None,
            path: "/".to_string(),
            recursive: false,
            limit: 10,
            cursor: Some("%%%".to_string()),
        },
    )
    .await
    .unwrap_err();

    assert_eq!(err.code, McpErrorCode::ERR_INVALID_PATH);
}

#[tokio::test]
async fn stat_path_root_special_case() {
    let dir = TempDir::new().unwrap();
    let registry = registry_in(&dir);
    let sessions = sessions_in();
    let ctx = FsToolsContext {
        registry,
        sessions,
        allow_insecure: true,
        auth_token: None,
    };

    let out = stat_path(
        &ctx,
        StatPathInput {
            session_id: None,
            path: "/".to_string(),
        },
    )
    .await
    .unwrap();

    assert_eq!(out.path, "/");
    assert_eq!(out.entry_type, EntryType::Dir);
    assert!(out.size_bytes.is_none());
}

#[tokio::test]
async fn read_file_root_is_rejected() {
    let dir = TempDir::new().unwrap();
    let registry = registry_in(&dir);
    let sessions = sessions_in();
    let ctx = FsToolsContext {
        registry,
        sessions,
        allow_insecure: true,
        auth_token: None,
    };

    let err = read_file(
        &ctx,
        ReadFileInput {
            session_id: None,
            path: "/".to_string(),
            offset_bytes: 0,
            max_bytes: 262_144,
            as_text: true,
            encoding: "utf-8".to_string(),
        },
    )
    .await
    .unwrap_err();

    assert_eq!(err.code, McpErrorCode::ERR_ROOT_OPERATION_NOT_ALLOWED);
}

#[tokio::test]
async fn read_file_stat_checks_missing_and_directory() {
    let dir = TempDir::new().unwrap();
    let local_root = dir.path().join("local");
    std::fs::create_dir_all(local_root.join("docs")).unwrap();

    let registry = registry_in(&dir);
    let mut storage = StorageRecord::new(
        "Local".to_string(),
        "local".to_string(),
        json!({"root": local_root}),
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

    let missing_err = read_file(
        &ctx,
        ReadFileInput {
            session_id: None,
            path: "/Local/missing.txt".to_string(),
            offset_bytes: 0,
            max_bytes: 262_144,
            as_text: true,
            encoding: "utf-8".to_string(),
        },
    )
    .await
    .unwrap_err();
    assert_eq!(missing_err.code, McpErrorCode::ERR_PATH_NOT_FOUND);

    let dir_err = read_file(
        &ctx,
        ReadFileInput {
            session_id: None,
            path: "/Local/docs".to_string(),
            offset_bytes: 0,
            max_bytes: 262_144,
            as_text: true,
            encoding: "utf-8".to_string(),
        },
    )
    .await
    .unwrap_err();
    assert_eq!(dir_err.code, McpErrorCode::ERR_IS_A_DIRECTORY);
}

#[tokio::test]
async fn read_file_caps_bytes_and_sets_truncated() {
    let dir = TempDir::new().unwrap();
    let local_root = dir.path().join("local");
    std::fs::create_dir_all(&local_root).unwrap();
    std::fs::write(local_root.join("hello.txt"), b"hello world").unwrap();

    let registry = registry_in(&dir);
    let mut storage = StorageRecord::new(
        "Local".to_string(),
        "local".to_string(),
        json!({"root": local_root}),
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

    let out = read_file(
        &ctx,
        ReadFileInput {
            session_id: None,
            path: "/Local/hello.txt".to_string(),
            offset_bytes: 0,
            max_bytes: 5,
            as_text: true,
            encoding: "utf-8".to_string(),
        },
    )
    .await
    .unwrap();

    assert_eq!(out.content, "hello");
    assert_eq!(out.read_bytes, 5);
    assert!(out.truncated);
}

#[tokio::test]
async fn read_file_binary_base64_mode() {
    let dir = TempDir::new().unwrap();
    let local_root = dir.path().join("local");
    std::fs::create_dir_all(&local_root).unwrap();
    std::fs::write(local_root.join("bin.dat"), vec![0xff, 0x00, 0x01]).unwrap();

    let registry = registry_in(&dir);
    let mut storage = StorageRecord::new(
        "Local".to_string(),
        "local".to_string(),
        json!({"root": local_root}),
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

    let out = read_file(
        &ctx,
        ReadFileInput {
            session_id: None,
            path: "/Local/bin.dat".to_string(),
            offset_bytes: 0,
            max_bytes: 262_144,
            as_text: false,
            encoding: "utf-8".to_string(),
        },
    )
    .await
    .unwrap();

    assert_eq!(out.content, "/wAB");
    assert_eq!(out.read_bytes, 3);
    assert!(!out.truncated);
}

#[tokio::test]
async fn read_file_text_decode_failure_has_hint() {
    let dir = TempDir::new().unwrap();
    let local_root = dir.path().join("local");
    std::fs::create_dir_all(&local_root).unwrap();
    std::fs::write(local_root.join("bad.txt"), vec![0xff, 0xfe, 0x00]).unwrap();

    let registry = registry_in(&dir);
    let mut storage = StorageRecord::new(
        "Local".to_string(),
        "local".to_string(),
        json!({"root": local_root}),
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

    let err = read_file(
        &ctx,
        ReadFileInput {
            session_id: None,
            path: "/Local/bad.txt".to_string(),
            offset_bytes: 0,
            max_bytes: 262_144,
            as_text: true,
            encoding: "utf-8".to_string(),
        },
    )
    .await
    .unwrap_err();

    assert_eq!(err.code, McpErrorCode::ERR_TEXT_DECODE_FAILED);
    assert_eq!(err.details["hint"], "use as_text=false");
}

#[tokio::test]
async fn read_file_rejects_invalid_max_bytes() {
    let dir = TempDir::new().unwrap();
    let registry = registry_in(&dir);
    let sessions = sessions_in();
    let ctx = FsToolsContext {
        registry,
        sessions,
        allow_insecure: true,
        auth_token: None,
    };

    let err = read_file(
        &ctx,
        ReadFileInput {
            session_id: None,
            path: "/Local/file.txt".to_string(),
            offset_bytes: 0,
            max_bytes: 2_097_153,
            as_text: true,
            encoding: "utf-8".to_string(),
        },
    )
    .await
    .unwrap_err();

    assert_eq!(err.code, McpErrorCode::ERR_INVALID_PATH);
}

#[test]
fn read_file_input_defaults_are_applied() {
    let input: ReadFileInput = serde_json::from_value(json!({
        "path": "/Local/file.txt"
    }))
    .unwrap();

    assert_eq!(input.offset_bytes, 0);
    assert_eq!(input.max_bytes, 262_144);
    assert!(input.as_text);
    assert_eq!(input.encoding, "utf-8");
}

#[tokio::test]
async fn mkdir_rejects_read_only_storage() {
    let dir = TempDir::new().unwrap();
    let local_root = dir.path().join("local");
    std::fs::create_dir_all(&local_root).unwrap();

    let registry = registry_in(&dir);
    let mut storage = StorageRecord::new(
        "Local".to_string(),
        "local".to_string(),
        json!({"root": local_root}),
    );
    storage.mcp_exposed = true;
    storage.mcp_policy.default_access = McpAccessMode::ReadWrite;
    storage.read_only = true;
    registry.save_all_atomic(&[storage]).unwrap();
    let sessions = sessions_in();
    let ctx = FsToolsContext {
        registry,
        sessions,
        allow_insecure: true,
        auth_token: None,
    };

    let err = mkdir(
        &ctx,
        MkdirInput {
            session_id: None,
            confirmation_id: None,
            path: "/Local/newdir".to_string(),
            parents: true,
            exist_ok: true,
        },
    )
    .await
    .unwrap_err();

    assert_eq!(err.code, McpErrorCode::ERR_STORAGE_READ_ONLY);
}

#[tokio::test]
async fn mkdir_requires_parent_when_parents_false() {
    let dir = TempDir::new().unwrap();
    let local_root = dir.path().join("local");
    std::fs::create_dir_all(&local_root).unwrap();

    let registry = registry_in(&dir);
    let mut storage = StorageRecord::new(
        "Local".to_string(),
        "local".to_string(),
        json!({"root": local_root}),
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

    let err = mkdir(
        &ctx,
        MkdirInput {
            session_id: None,
            confirmation_id: None,
            path: "/Local/missing/child".to_string(),
            parents: false,
            exist_ok: true,
        },
    )
    .await
    .unwrap_err();

    assert_eq!(err.code, McpErrorCode::ERR_PARENT_NOT_FOUND);
}

#[tokio::test]
async fn mkdir_creates_nested_directories_when_parents_true() {
    let dir = TempDir::new().unwrap();
    let local_root = dir.path().join("local");
    std::fs::create_dir_all(&local_root).unwrap();

    let registry = registry_in(&dir);
    let mut storage = StorageRecord::new(
        "Local".to_string(),
        "local".to_string(),
        json!({"root": local_root.clone()}),
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

    let out = mkdir(
        &ctx,
        MkdirInput {
            session_id: None,
            confirmation_id: None,
            path: "/Local/a/b".to_string(),
            parents: true,
            exist_ok: true,
        },
    )
    .await
    .unwrap();

    assert!(out.created);
    assert!(local_root.join("a").join("b").is_dir());
}

#[tokio::test]
async fn mkdir_exist_ok_false_returns_already_exists() {
    let dir = TempDir::new().unwrap();
    let local_root = dir.path().join("local");
    std::fs::create_dir_all(local_root.join("docs")).unwrap();

    let registry = registry_in(&dir);
    let mut storage = StorageRecord::new(
        "Local".to_string(),
        "local".to_string(),
        json!({"root": local_root}),
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

    let err = mkdir(
        &ctx,
        MkdirInput {
            session_id: None,
            confirmation_id: None,
            path: "/Local/docs".to_string(),
            parents: true,
            exist_ok: false,
        },
    )
    .await
    .unwrap_err();

    assert_eq!(err.code, McpErrorCode::ERR_ALREADY_EXISTS);
}

#[tokio::test]
async fn mkdir_existing_dir_with_exist_ok_true_returns_not_created() {
    let dir = TempDir::new().unwrap();
    let local_root = dir.path().join("local");
    std::fs::create_dir_all(local_root.join("docs")).unwrap();

    let registry = registry_in(&dir);
    let mut storage = StorageRecord::new(
        "Local".to_string(),
        "local".to_string(),
        json!({"root": local_root}),
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

    let out = mkdir(
        &ctx,
        MkdirInput {
            session_id: None,
            confirmation_id: None,
            path: "/Local/docs".to_string(),
            parents: true,
            exist_ok: true,
        },
    )
    .await
    .unwrap();

    assert!(!out.created);
}

#[tokio::test]
async fn write_file_rejects_read_only_storage() {
    let dir = TempDir::new().unwrap();
    let local_root = dir.path().join("local");
    std::fs::create_dir_all(&local_root).unwrap();

    let registry = registry_in(&dir);
    let mut storage = StorageRecord::new(
        "Local".to_string(),
        "local".to_string(),
        json!({"root": local_root}),
    );
    storage.mcp_exposed = true;
    storage.mcp_policy.default_access = McpAccessMode::ReadWrite;
    storage.read_only = true;
    registry.save_all_atomic(&[storage]).unwrap();
    let sessions = sessions_in();
    let ctx = FsToolsContext {
        registry,
        sessions,
        allow_insecure: true,
        auth_token: None,
    };

    let err = write_file(
        &ctx,
        WriteFileInput {
            session_id: None,
            user_metadata: None,
            confirmation_id: None,
            path: "/Local/file.txt".to_string(),
            content: "hello".to_string(),
            encoding: "utf-8".to_string(),
            overwrite: true,
            create_parents: false,
        },
    )
    .await
    .unwrap_err();

    assert_eq!(err.code, McpErrorCode::ERR_STORAGE_READ_ONLY);
}

#[tokio::test]
async fn write_file_requires_parent_when_create_parents_false() {
    let dir = TempDir::new().unwrap();
    let local_root = dir.path().join("local");
    std::fs::create_dir_all(&local_root).unwrap();

    let registry = registry_in(&dir);
    let mut storage = StorageRecord::new(
        "Local".to_string(),
        "local".to_string(),
        json!({"root": local_root}),
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

    let err = write_file(
        &ctx,
        WriteFileInput {
            session_id: None,
            user_metadata: None,
            confirmation_id: None,
            path: "/Local/missing/file.txt".to_string(),
            content: "hello".to_string(),
            encoding: "utf-8".to_string(),
            overwrite: true,
            create_parents: false,
        },
    )
    .await
    .unwrap_err();

    assert_eq!(err.code, McpErrorCode::ERR_PARENT_NOT_FOUND);
}

#[tokio::test]
async fn write_file_creates_parents_when_requested() {
    let dir = TempDir::new().unwrap();
    let local_root = dir.path().join("local");
    std::fs::create_dir_all(&local_root).unwrap();

    let registry = registry_in(&dir);
    let mut storage = StorageRecord::new(
        "Local".to_string(),
        "local".to_string(),
        json!({"root": local_root.clone()}),
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

    let out = write_file(
        &ctx,
        WriteFileInput {
            session_id: None,
            user_metadata: None,
            confirmation_id: None,
            path: "/Local/a/b/file.txt".to_string(),
            content: "hello".to_string(),
            encoding: "utf-8".to_string(),
            overwrite: true,
            create_parents: true,
        },
    )
    .await
    .unwrap();

    assert_eq!(out.written_bytes, 5);
    assert_eq!(
        std::fs::read_to_string(local_root.join("a").join("b").join("file.txt")).unwrap(),
        "hello"
    );
}

#[tokio::test]
async fn write_file_respects_overwrite_flag() {
    let dir = TempDir::new().unwrap();
    let local_root = dir.path().join("local");
    std::fs::create_dir_all(&local_root).unwrap();
    std::fs::write(local_root.join("file.txt"), "old").unwrap();

    let registry = registry_in(&dir);
    let mut storage = StorageRecord::new(
        "Local".to_string(),
        "local".to_string(),
        json!({"root": local_root.clone()}),
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

    let err = write_file(
        &ctx,
        WriteFileInput {
            session_id: None,
            user_metadata: None,
            confirmation_id: None,
            path: "/Local/file.txt".to_string(),
            content: "new".to_string(),
            encoding: "utf-8".to_string(),
            overwrite: false,
            create_parents: false,
        },
    )
    .await
    .unwrap_err();
    assert_eq!(err.code, McpErrorCode::ERR_ALREADY_EXISTS);

    let out = write_file(
        &ctx,
        WriteFileInput {
            session_id: None,
            user_metadata: None,
            confirmation_id: None,
            path: "/Local/file.txt".to_string(),
            content: "new".to_string(),
            encoding: "utf-8".to_string(),
            overwrite: true,
            create_parents: false,
        },
    )
    .await
    .unwrap();
    assert_eq!(out.written_bytes, 3);
    assert_eq!(
        std::fs::read_to_string(local_root.join("file.txt")).unwrap(),
        "new"
    );
}

#[tokio::test]
async fn write_file_rejects_directory_target_and_non_utf8_encoding() {
    let dir = TempDir::new().unwrap();
    let local_root = dir.path().join("local");
    std::fs::create_dir_all(local_root.join("docs")).unwrap();

    let registry = registry_in(&dir);
    let mut storage = StorageRecord::new(
        "Local".to_string(),
        "local".to_string(),
        json!({"root": local_root}),
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

    let dir_err = write_file(
        &ctx,
        WriteFileInput {
            session_id: None,
            user_metadata: None,
            confirmation_id: None,
            path: "/Local/docs".to_string(),
            content: "hello".to_string(),
            encoding: "utf-8".to_string(),
            overwrite: true,
            create_parents: false,
        },
    )
    .await
    .unwrap_err();
    assert_eq!(dir_err.code, McpErrorCode::ERR_IS_A_DIRECTORY);

    let encoding_err = write_file(
        &ctx,
        WriteFileInput {
            session_id: None,
            user_metadata: None,
            confirmation_id: None,
            path: "/Local/file.txt".to_string(),
            content: "hello".to_string(),
            encoding: "utf-16".to_string(),
            overwrite: true,
            create_parents: false,
        },
    )
    .await
    .unwrap_err();
    assert_eq!(encoding_err.code, McpErrorCode::ERR_TEXT_DECODE_FAILED);
}

#[test]
fn write_file_input_defaults_are_applied() {
    let input: WriteFileInput = serde_json::from_value(json!({
        "path": "/Local/file.txt",
        "content": "hello"
    }))
    .unwrap();

    assert_eq!(input.encoding, "utf-8");
    assert!(input.overwrite);
    assert!(!input.create_parents);
    assert!(input.user_metadata.is_none());
}

#[tokio::test]
async fn write_file_accepts_user_metadata_when_backend_reports_support() {
    let dir = TempDir::new().unwrap();
    let local_root = dir.path().join("local");
    std::fs::create_dir_all(&local_root).unwrap();

    let registry = registry_in(&dir);
    let mut storage = StorageRecord::new(
        "Local".to_string(),
        "local".to_string(),
        json!({"root": local_root}),
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

    let out = write_file(
        &ctx,
        WriteFileInput {
            session_id: None,
            confirmation_id: None,
            path: "/Local/file.txt".to_string(),
            content: "hello".to_string(),
            encoding: "utf-8".to_string(),
            overwrite: true,
            create_parents: false,
            user_metadata: Some(HashMap::from([
                (" project ".to_string(), "alpha".to_string()),
                ("".to_string(), "ignored".to_string()),
            ])),
        },
    )
    .await
    .unwrap();

    assert_eq!(out.written_bytes, 5);
}

#[tokio::test]
async fn delete_path_root_is_rejected() {
    let dir = TempDir::new().unwrap();
    let registry = registry_in(&dir);
    let sessions = sessions_in();
    let ctx = FsToolsContext {
        registry,
        sessions,
        allow_insecure: true,
        auth_token: None,
    };

    let err = delete_path(
        &ctx,
        DeletePathInput {
            session_id: None,
            confirmation_id: None,
            path: "/".to_string(),
            recursive: false,
        },
    )
    .await
    .unwrap_err();

    assert_eq!(err.code, McpErrorCode::ERR_ROOT_OPERATION_NOT_ALLOWED);
}

#[tokio::test]
async fn delete_path_rejects_read_only_storage() {
    let dir = TempDir::new().unwrap();
    let local_root = dir.path().join("local");
    std::fs::create_dir_all(&local_root).unwrap();
    std::fs::write(local_root.join("file.txt"), "x").unwrap();

    let registry = registry_in(&dir);
    let mut storage = StorageRecord::new(
        "Local".to_string(),
        "local".to_string(),
        json!({"root": local_root}),
    );
    storage.mcp_exposed = true;
    storage.mcp_policy.default_access = McpAccessMode::ReadWrite;
    storage.read_only = true;
    registry.save_all_atomic(&[storage]).unwrap();
    let sessions = sessions_in();
    let ctx = FsToolsContext {
        registry,
        sessions,
        allow_insecure: true,
        auth_token: None,
    };

    let err = delete_path(
        &ctx,
        DeletePathInput {
            session_id: None,
            confirmation_id: None,
            path: "/Local/file.txt".to_string(),
            recursive: false,
        },
    )
    .await
    .unwrap_err();

    assert_eq!(err.code, McpErrorCode::ERR_STORAGE_READ_ONLY);
}

#[tokio::test]
async fn delete_path_file_success_and_missing_returns_not_found() {
    let dir = TempDir::new().unwrap();
    let local_root = dir.path().join("local");
    std::fs::create_dir_all(&local_root).unwrap();
    std::fs::write(local_root.join("file.txt"), "x").unwrap();

    let registry = registry_in(&dir);
    let mut storage = StorageRecord::new(
        "Local".to_string(),
        "local".to_string(),
        json!({"root": local_root.clone()}),
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

    let out = delete_path(
        &ctx,
        DeletePathInput {
            session_id: None,
            confirmation_id: None,
            path: "/Local/file.txt".to_string(),
            recursive: false,
        },
    )
    .await
    .unwrap();
    assert!(out.deleted);
    assert!(!local_root.join("file.txt").exists());

    let err = delete_path(
        &ctx,
        DeletePathInput {
            session_id: None,
            confirmation_id: None,
            path: "/Local/file.txt".to_string(),
            recursive: false,
        },
    )
    .await
    .unwrap_err();
    assert_eq!(err.code, McpErrorCode::ERR_PATH_NOT_FOUND);
}

#[tokio::test]
async fn delete_path_directory_requires_recursive() {
    let dir = TempDir::new().unwrap();
    let local_root = dir.path().join("local");
    std::fs::create_dir_all(local_root.join("docs")).unwrap();

    let registry = registry_in(&dir);
    let mut storage = StorageRecord::new(
        "Local".to_string(),
        "local".to_string(),
        json!({"root": local_root}),
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

    let err = delete_path(
        &ctx,
        DeletePathInput {
            session_id: None,
            confirmation_id: None,
            path: "/Local/docs".to_string(),
            recursive: false,
        },
    )
    .await
    .unwrap_err();

    assert_eq!(err.code, McpErrorCode::ERR_NOT_EMPTY_OR_DIR);
}

#[tokio::test]
async fn delete_path_recursive_deletes_nested_structure() {
    let dir = TempDir::new().unwrap();
    let local_root = dir.path().join("local");
    std::fs::create_dir_all(local_root.join("docs").join("nested")).unwrap();
    std::fs::write(local_root.join("docs").join("a.txt"), "a").unwrap();
    std::fs::write(local_root.join("docs").join("nested").join("b.txt"), "b").unwrap();

    let registry = registry_in(&dir);
    let mut storage = StorageRecord::new(
        "Local".to_string(),
        "local".to_string(),
        json!({"root": local_root.clone()}),
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

    let out = delete_path(
        &ctx,
        DeletePathInput {
            session_id: None,
            confirmation_id: None,
            path: "/Local/docs".to_string(),
            recursive: true,
        },
    )
    .await
    .unwrap();

    assert!(out.deleted);
    assert!(!local_root.join("docs").exists());
}

#[tokio::test]
async fn copy_path_rejects_read_only_destination() {
    let dir = TempDir::new().unwrap();
    let src_root = dir.path().join("src");
    let dst_root = dir.path().join("dst");
    std::fs::create_dir_all(&src_root).unwrap();
    std::fs::create_dir_all(&dst_root).unwrap();
    std::fs::write(src_root.join("file.txt"), "hello").unwrap();

    let registry = registry_in(&dir);
    let mut src = StorageRecord::new(
        "Src".to_string(),
        "local".to_string(),
        json!({"root": src_root}),
    );
    src.mcp_exposed = true;
    src.mcp_policy.default_access = McpAccessMode::ReadWrite;
    let mut dst = StorageRecord::new(
        "Dst".to_string(),
        "local".to_string(),
        json!({"root": dst_root}),
    );
    dst.mcp_exposed = true;
    dst.mcp_policy.default_access = McpAccessMode::ReadWrite;
    dst.read_only = true;
    registry.save_all_atomic(&[src, dst]).unwrap();
    let sessions = sessions_in();
    let ctx = FsToolsContext {
        registry,
        sessions,
        allow_insecure: true,
        auth_token: None,
    };

    let err = copy_path(
        &ctx,
        CopyPathInput {
            session_id: None,
            confirmation_id: None,
            src: "/Src/file.txt".to_string(),
            dst: "/Dst/file.txt".to_string(),
            overwrite: false,
            recursive: false,
        },
    )
    .await
    .unwrap_err();

    assert_eq!(err.code, McpErrorCode::ERR_STORAGE_READ_ONLY);
}

#[tokio::test]
async fn copy_path_rejects_directory_without_recursive() {
    let dir = TempDir::new().unwrap();
    let src_root = dir.path().join("src");
    let dst_root = dir.path().join("dst");
    std::fs::create_dir_all(src_root.join("docs")).unwrap();
    std::fs::create_dir_all(&dst_root).unwrap();

    let registry = registry_in(&dir);
    let mut src = StorageRecord::new(
        "Src".to_string(),
        "local".to_string(),
        json!({"root": src_root}),
    );
    src.mcp_exposed = true;
    src.mcp_policy.default_access = McpAccessMode::ReadWrite;
    let mut dst = StorageRecord::new(
        "Dst".to_string(),
        "local".to_string(),
        json!({"root": dst_root}),
    );
    dst.mcp_exposed = true;
    dst.mcp_policy.default_access = McpAccessMode::ReadWrite;
    registry.save_all_atomic(&[src, dst]).unwrap();
    let sessions = sessions_in();
    let ctx = FsToolsContext {
        registry,
        sessions,
        allow_insecure: true,
        auth_token: None,
    };

    let err = copy_path(
        &ctx,
        CopyPathInput {
            session_id: None,
            confirmation_id: None,
            src: "/Src/docs".to_string(),
            dst: "/Dst/docs".to_string(),
            overwrite: false,
            recursive: false,
        },
    )
    .await
    .unwrap_err();

    assert_eq!(err.code, McpErrorCode::ERR_IS_A_DIRECTORY);
}

#[tokio::test]
async fn copy_path_overwrite_false_rejects_existing_destination() {
    let dir = TempDir::new().unwrap();
    let src_root = dir.path().join("src");
    let dst_root = dir.path().join("dst");
    std::fs::create_dir_all(&src_root).unwrap();
    std::fs::create_dir_all(&dst_root).unwrap();
    std::fs::write(src_root.join("file.txt"), "hello").unwrap();
    std::fs::write(dst_root.join("file.txt"), "existing").unwrap();

    let registry = registry_in(&dir);
    let mut src = StorageRecord::new(
        "Src".to_string(),
        "local".to_string(),
        json!({"root": src_root}),
    );
    src.mcp_exposed = true;
    src.mcp_policy.default_access = McpAccessMode::ReadWrite;
    let mut dst = StorageRecord::new(
        "Dst".to_string(),
        "local".to_string(),
        json!({"root": dst_root}),
    );
    dst.mcp_exposed = true;
    dst.mcp_policy.default_access = McpAccessMode::ReadWrite;
    registry.save_all_atomic(&[src, dst]).unwrap();
    let sessions = sessions_in();
    let ctx = FsToolsContext {
        registry,
        sessions,
        allow_insecure: true,
        auth_token: None,
    };

    let err = copy_path(
        &ctx,
        CopyPathInput {
            session_id: None,
            confirmation_id: None,
            src: "/Src/file.txt".to_string(),
            dst: "/Dst/file.txt".to_string(),
            overwrite: false,
            recursive: false,
        },
    )
    .await
    .unwrap_err();

    assert_eq!(err.code, McpErrorCode::ERR_ALREADY_EXISTS);
}

#[tokio::test]
async fn copy_path_requires_existing_parent_for_file_destination() {
    let dir = TempDir::new().unwrap();
    let root = dir.path().join("local");
    std::fs::create_dir_all(&root).unwrap();
    std::fs::write(root.join("a.txt"), "hello").unwrap();
    let registry = registry_in(&dir);
    let mut storage = StorageRecord::new(
        "Local".to_string(),
        "local".to_string(),
        json!({"root": root}),
    );
    storage.mcp_exposed = true;
    storage.mcp_policy.default_access = McpAccessMode::ReadWrite;
    registry.save_all_atomic(&[storage]).unwrap();
    let ctx = FsToolsContext {
        registry,
        sessions: sessions_in(),
        allow_insecure: true,
        auth_token: None,
    };

    let err = copy_path(
        &ctx,
        CopyPathInput {
            session_id: None,
            confirmation_id: None,
            src: "/Local/a.txt".to_string(),
            dst: "/Local/missing/b.txt".to_string(),
            overwrite: false,
            recursive: false,
        },
    )
    .await
    .unwrap_err();

    assert_eq!(err.code, McpErrorCode::ERR_PARENT_NOT_FOUND);
    assert!(!root.join("missing").exists());
}

#[tokio::test]
async fn copy_path_same_storage_file_success() {
    let dir = TempDir::new().unwrap();
    let root = dir.path().join("root");
    std::fs::create_dir_all(root.join("out")).unwrap();
    std::fs::write(root.join("file.txt"), "hello").unwrap();

    let registry = registry_in(&dir);
    let mut storage = StorageRecord::new(
        "Local".to_string(),
        "local".to_string(),
        json!({"root": root.clone()}),
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

    let out = copy_path(
        &ctx,
        CopyPathInput {
            session_id: None,
            confirmation_id: None,
            src: "/Local/file.txt".to_string(),
            dst: "/Local/out/file.txt".to_string(),
            overwrite: false,
            recursive: false,
        },
    )
    .await
    .unwrap();

    assert!(out.copied);
    assert_eq!(
        std::fs::read_to_string(root.join("out").join("file.txt")).unwrap(),
        "hello"
    );
}

#[tokio::test]
async fn copy_path_cross_storage_streams_large_file() {
    let dir = TempDir::new().unwrap();
    let src_root = dir.path().join("src");
    let dst_root = dir.path().join("dst");
    std::fs::create_dir_all(&src_root).unwrap();
    std::fs::create_dir_all(&dst_root).unwrap();
    let payload = vec![b'x'; (8 * 1024 * 1024) + 17];
    std::fs::write(src_root.join("large.bin"), &payload).unwrap();

    let registry = registry_in(&dir);
    let mut src = StorageRecord::new(
        "Src".to_string(),
        "local".to_string(),
        json!({"root": src_root}),
    );
    src.mcp_exposed = true;
    src.mcp_policy.default_access = McpAccessMode::ReadWrite;
    let mut dst = StorageRecord::new(
        "Dst".to_string(),
        "local".to_string(),
        json!({"root": dst_root.clone()}),
    );
    dst.mcp_exposed = true;
    dst.mcp_policy.default_access = McpAccessMode::ReadWrite;
    registry.save_all_atomic(&[src, dst]).unwrap();
    let sessions = sessions_in();
    let ctx = FsToolsContext {
        registry,
        sessions,
        allow_insecure: true,
        auth_token: None,
    };

    let out = copy_path(
        &ctx,
        CopyPathInput {
            session_id: None,
            confirmation_id: None,
            src: "/Src/large.bin".to_string(),
            dst: "/Dst/large.bin".to_string(),
            overwrite: false,
            recursive: false,
        },
    )
    .await
    .unwrap();

    assert!(out.copied);
    assert_eq!(std::fs::read(dst_root.join("large.bin")).unwrap(), payload);
}

#[tokio::test]
async fn copy_path_rejects_alias_copy_into_own_child() {
    let dir = TempDir::new().unwrap();
    let shared_root = dir.path().join("shared");
    std::fs::create_dir_all(shared_root.join("foo")).unwrap();
    std::fs::write(shared_root.join("foo").join("file.txt"), "hello").unwrap();

    let registry = registry_in(&dir);
    let mut a = StorageRecord::new(
        "A".to_string(),
        "local".to_string(),
        json!({"root": shared_root.clone()}),
    );
    a.mcp_exposed = true;
    a.mcp_policy.default_access = McpAccessMode::ReadWrite;
    let mut b = StorageRecord::new(
        "B".to_string(),
        "local".to_string(),
        json!({"root": shared_root}),
    );
    b.mcp_exposed = true;
    b.mcp_policy.default_access = McpAccessMode::ReadWrite;
    registry.save_all_atomic(&[a, b]).unwrap();
    let ctx = FsToolsContext {
        registry,
        sessions: sessions_in(),
        allow_insecure: true,
        auth_token: None,
    };

    let error = copy_path(
        &ctx,
        CopyPathInput {
            session_id: None,
            confirmation_id: None,
            src: "/A/foo".to_string(),
            dst: "/B/foo/child".to_string(),
            overwrite: false,
            recursive: true,
        },
    )
    .await
    .unwrap_err();
    assert_eq!(error.code, McpErrorCode::ERR_TRANSFER_NAMESPACE_CONFLICT);
    assert!(
        !dir.path().join("shared/foo/child").exists(),
        "rejected before destination creation"
    );
}

#[tokio::test]
async fn copy_path_rejects_alias_into_nested_root() {
    let dir = TempDir::new().unwrap();
    let root = dir.path().join("root");
    let nested = root.join("foo");
    std::fs::create_dir_all(&nested).unwrap();
    std::fs::write(nested.join("file.txt"), "hello").unwrap();

    let registry = registry_in(&dir);
    let mut a = StorageRecord::new("A".to_string(), "local".to_string(), json!({"root": root}));
    a.mcp_exposed = true;
    a.mcp_policy.default_access = McpAccessMode::ReadWrite;
    let mut b = StorageRecord::new(
        "B".to_string(),
        "local".to_string(),
        json!({"root": nested}),
    );
    b.mcp_exposed = true;
    b.mcp_policy.default_access = McpAccessMode::ReadWrite;
    registry.save_all_atomic(&[a, b]).unwrap();
    let ctx = FsToolsContext {
        registry,
        sessions: sessions_in(),
        allow_insecure: true,
        auth_token: None,
    };

    let error = copy_path(
        &ctx,
        CopyPathInput {
            session_id: None,
            confirmation_id: None,
            src: "/A/foo".to_string(),
            dst: "/B/child".to_string(),
            overwrite: false,
            recursive: true,
        },
    )
    .await
    .unwrap_err();
    assert_eq!(error.code, McpErrorCode::ERR_TRANSFER_NAMESPACE_CONFLICT);
}

#[tokio::test]
async fn copy_path_rejects_copy_into_own_subdirectory_same_storage() {
    let dir = TempDir::new().unwrap();
    let root = dir.path().join("root");
    std::fs::create_dir_all(root.join("foo")).unwrap();
    std::fs::write(root.join("foo").join("file.txt"), "hello").unwrap();

    let registry = registry_in(&dir);
    let mut storage = StorageRecord::new(
        "Local".to_string(),
        "local".to_string(),
        json!({"root": root.clone()}),
    );
    storage.mcp_exposed = true;
    storage.mcp_policy.default_access = McpAccessMode::ReadWrite;
    registry.save_all_atomic(&[storage]).unwrap();
    let ctx = FsToolsContext {
        registry,
        sessions: sessions_in(),
        allow_insecure: true,
        auth_token: None,
    };

    let error = copy_path(
        &ctx,
        CopyPathInput {
            session_id: None,
            confirmation_id: None,
            src: "/Local/foo".to_string(),
            dst: "/Local/foo/child".to_string(),
            overwrite: false,
            recursive: true,
        },
    )
    .await
    .unwrap_err();
    assert_eq!(error.code, McpErrorCode::ERR_TRANSFER_NAMESPACE_CONFLICT);
    assert!(!root.join("foo/child").exists());
}

#[tokio::test]
async fn copy_path_recursive_preserves_structure() {
    let dir = TempDir::new().unwrap();
    let src_root = dir.path().join("src");
    let dst_root = dir.path().join("dst");
    std::fs::create_dir_all(src_root.join("docs").join("nested")).unwrap();
    std::fs::create_dir_all(&dst_root).unwrap();
    std::fs::write(src_root.join("docs").join("a.txt"), "a").unwrap();
    std::fs::write(src_root.join("docs").join("nested").join("b.txt"), "b").unwrap();

    let registry = registry_in(&dir);
    let mut src = StorageRecord::new(
        "Src".to_string(),
        "local".to_string(),
        json!({"root": src_root}),
    );
    src.mcp_exposed = true;
    src.mcp_policy.default_access = McpAccessMode::ReadWrite;
    let mut dst = StorageRecord::new(
        "Dst".to_string(),
        "local".to_string(),
        json!({"root": dst_root.clone()}),
    );
    dst.mcp_exposed = true;
    dst.mcp_policy.default_access = McpAccessMode::ReadWrite;
    registry.save_all_atomic(&[src, dst]).unwrap();
    let sessions = sessions_in();
    let ctx = FsToolsContext {
        registry,
        sessions,
        allow_insecure: true,
        auth_token: None,
    };

    let out = copy_path(
        &ctx,
        CopyPathInput {
            session_id: None,
            confirmation_id: None,
            src: "/Src/docs".to_string(),
            dst: "/Dst/copied".to_string(),
            overwrite: false,
            recursive: true,
        },
    )
    .await
    .unwrap();

    assert!(out.copied);
    assert_eq!(
        std::fs::read_to_string(dst_root.join("copied").join("a.txt")).unwrap(),
        "a"
    );
    assert_eq!(
        std::fs::read_to_string(dst_root.join("copied").join("nested").join("b.txt")).unwrap(),
        "b"
    );
}

#[tokio::test]
async fn move_path_requires_existing_parent_for_file_destination() {
    let dir = TempDir::new().unwrap();
    let root = dir.path().join("local");
    std::fs::create_dir_all(&root).unwrap();
    std::fs::write(root.join("a.txt"), "hello").unwrap();
    let registry = registry_in(&dir);
    let mut storage = StorageRecord::new(
        "Local".to_string(),
        "local".to_string(),
        json!({"root": root.clone()}),
    );
    storage.mcp_exposed = true;
    storage.mcp_policy.default_access = McpAccessMode::ReadWrite;
    registry.save_all_atomic(&[storage]).unwrap();
    let ctx = FsToolsContext {
        registry,
        sessions: sessions_in(),
        allow_insecure: true,
        auth_token: None,
    };

    let err = move_path(
        &ctx,
        MovePathInput {
            session_id: None,
            confirmation_id: None,
            src: "/Local/a.txt".to_string(),
            dst: "/Local/missing/b.txt".to_string(),
            overwrite: false,
        },
    )
    .await
    .unwrap_err();

    assert_eq!(err.code, McpErrorCode::ERR_PARENT_NOT_FOUND);
    assert!(root.join("a.txt").exists());
    assert!(!root.join("missing").exists());
}

#[tokio::test]
async fn move_path_same_storage_file_success() {
    let dir = TempDir::new().unwrap();
    let root = dir.path().join("root");
    std::fs::create_dir_all(root.join("out")).unwrap();
    std::fs::write(root.join("file.txt"), "hello").unwrap();

    let registry = registry_in(&dir);
    let mut storage = StorageRecord::new(
        "Local".to_string(),
        "local".to_string(),
        json!({"root": root.clone()}),
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

    let out = move_path(
        &ctx,
        MovePathInput {
            session_id: None,
            confirmation_id: None,
            src: "/Local/file.txt".to_string(),
            dst: "/Local/out/file.txt".to_string(),
            overwrite: false,
        },
    )
    .await
    .unwrap();

    assert!(out.moved);
    assert!(!root.join("file.txt").exists());
    assert_eq!(
        std::fs::read_to_string(root.join("out").join("file.txt")).unwrap(),
        "hello"
    );
}

#[tokio::test]
async fn move_path_cross_storage_success() {
    let dir = TempDir::new().unwrap();
    let src_root = dir.path().join("src");
    let dst_root = dir.path().join("dst");
    std::fs::create_dir_all(&src_root).unwrap();
    std::fs::create_dir_all(&dst_root).unwrap();
    std::fs::write(src_root.join("file.txt"), "hello").unwrap();

    let registry = registry_in(&dir);
    let mut src = StorageRecord::new(
        "Src".to_string(),
        "local".to_string(),
        json!({"root": src_root.clone()}),
    );
    src.mcp_exposed = true;
    src.mcp_policy.default_access = McpAccessMode::ReadWrite;
    let mut dst = StorageRecord::new(
        "Dst".to_string(),
        "local".to_string(),
        json!({"root": dst_root.clone()}),
    );
    dst.mcp_exposed = true;
    dst.mcp_policy.default_access = McpAccessMode::ReadWrite;
    registry.save_all_atomic(&[src, dst]).unwrap();
    let sessions = sessions_in();
    let ctx = FsToolsContext {
        registry,
        sessions,
        allow_insecure: true,
        auth_token: None,
    };

    let out = move_path(
        &ctx,
        MovePathInput {
            session_id: None,
            confirmation_id: None,
            src: "/Src/file.txt".to_string(),
            dst: "/Dst/file.txt".to_string(),
            overwrite: false,
        },
    )
    .await
    .unwrap();

    assert!(out.moved);
    assert!(!src_root.join("file.txt").exists());
    assert_eq!(
        std::fs::read_to_string(dst_root.join("file.txt")).unwrap(),
        "hello"
    );
}

#[tokio::test]
async fn move_path_overwrite_false_rejects_existing_destination() {
    let dir = TempDir::new().unwrap();
    let src_root = dir.path().join("src");
    let dst_root = dir.path().join("dst");
    std::fs::create_dir_all(&src_root).unwrap();
    std::fs::create_dir_all(&dst_root).unwrap();
    std::fs::write(src_root.join("file.txt"), "hello").unwrap();
    std::fs::write(dst_root.join("file.txt"), "existing").unwrap();

    let registry = registry_in(&dir);
    let mut src = StorageRecord::new(
        "Src".to_string(),
        "local".to_string(),
        json!({"root": src_root}),
    );
    src.mcp_exposed = true;
    src.mcp_policy.default_access = McpAccessMode::ReadWrite;
    let mut dst = StorageRecord::new(
        "Dst".to_string(),
        "local".to_string(),
        json!({"root": dst_root}),
    );
    dst.mcp_exposed = true;
    dst.mcp_policy.default_access = McpAccessMode::ReadWrite;
    registry.save_all_atomic(&[src, dst]).unwrap();
    let sessions = sessions_in();
    let ctx = FsToolsContext {
        registry,
        sessions,
        allow_insecure: true,
        auth_token: None,
    };

    let err = move_path(
        &ctx,
        MovePathInput {
            session_id: None,
            confirmation_id: None,
            src: "/Src/file.txt".to_string(),
            dst: "/Dst/file.txt".to_string(),
            overwrite: false,
        },
    )
    .await
    .unwrap_err();

    assert_eq!(err.code, McpErrorCode::ERR_ALREADY_EXISTS);
}

#[tokio::test]
async fn move_path_overwrite_true_replaces_existing_destination() {
    let dir = TempDir::new().unwrap();
    let src_root = dir.path().join("src");
    let dst_root = dir.path().join("dst");
    std::fs::create_dir_all(&src_root).unwrap();
    std::fs::create_dir_all(&dst_root).unwrap();
    std::fs::write(src_root.join("file.txt"), "hello").unwrap();
    std::fs::write(dst_root.join("file.txt"), "existing").unwrap();

    let registry = registry_in(&dir);
    let mut src = StorageRecord::new(
        "Src".to_string(),
        "local".to_string(),
        json!({"root": src_root.clone()}),
    );
    src.mcp_exposed = true;
    src.mcp_policy.default_access = McpAccessMode::ReadWrite;
    let mut dst = StorageRecord::new(
        "Dst".to_string(),
        "local".to_string(),
        json!({"root": dst_root.clone()}),
    );
    dst.mcp_exposed = true;
    dst.mcp_policy.default_access = McpAccessMode::ReadWrite;
    registry.save_all_atomic(&[src, dst]).unwrap();
    let sessions = sessions_in();
    let ctx = FsToolsContext {
        registry,
        sessions,
        allow_insecure: true,
        auth_token: None,
    };

    let out = move_path(
        &ctx,
        MovePathInput {
            session_id: None,
            confirmation_id: None,
            src: "/Src/file.txt".to_string(),
            dst: "/Dst/file.txt".to_string(),
            overwrite: true,
        },
    )
    .await
    .unwrap();

    assert!(out.moved);
    assert!(!src_root.join("file.txt").exists());
    assert_eq!(
        std::fs::read_to_string(dst_root.join("file.txt")).unwrap(),
        "hello"
    );
}

#[tokio::test]
async fn move_path_missing_source_returns_not_found() {
    let dir = TempDir::new().unwrap();
    let src_root = dir.path().join("src");
    let dst_root = dir.path().join("dst");
    std::fs::create_dir_all(&src_root).unwrap();
    std::fs::create_dir_all(&dst_root).unwrap();

    let registry = registry_in(&dir);
    let mut src = StorageRecord::new(
        "Src".to_string(),
        "local".to_string(),
        json!({"root": src_root}),
    );
    src.mcp_exposed = true;
    src.mcp_policy.default_access = McpAccessMode::ReadWrite;
    let mut dst = StorageRecord::new(
        "Dst".to_string(),
        "local".to_string(),
        json!({"root": dst_root}),
    );
    dst.mcp_exposed = true;
    dst.mcp_policy.default_access = McpAccessMode::ReadWrite;
    registry.save_all_atomic(&[src, dst]).unwrap();
    let sessions = sessions_in();
    let ctx = FsToolsContext {
        registry,
        sessions,
        allow_insecure: true,
        auth_token: None,
    };

    let err = move_path(
        &ctx,
        MovePathInput {
            session_id: None,
            confirmation_id: None,
            src: "/Src/file.txt".to_string(),
            dst: "/Dst/file.txt".to_string(),
            overwrite: false,
        },
    )
    .await
    .unwrap_err();

    assert_eq!(err.code, McpErrorCode::ERR_PATH_NOT_FOUND);
}

#[tokio::test]
async fn move_path_rejects_read_only_source_or_destination() {
    let dir = TempDir::new().unwrap();
    let src_root = dir.path().join("src");
    let dst_root = dir.path().join("dst");
    std::fs::create_dir_all(&src_root).unwrap();
    std::fs::create_dir_all(&dst_root).unwrap();
    std::fs::write(src_root.join("file.txt"), "hello").unwrap();

    let registry = registry_in(&dir);
    let mut src = StorageRecord::new(
        "Src".to_string(),
        "local".to_string(),
        json!({"root": src_root.clone()}),
    );
    src.mcp_exposed = true;
    src.mcp_policy.default_access = McpAccessMode::ReadWrite;
    src.read_only = true;
    let mut dst = StorageRecord::new(
        "Dst".to_string(),
        "local".to_string(),
        json!({"root": dst_root.clone()}),
    );
    dst.mcp_exposed = true;
    dst.mcp_policy.default_access = McpAccessMode::ReadWrite;
    registry.save_all_atomic(&[src, dst]).unwrap();
    let sessions = sessions_in();
    let ctx = FsToolsContext {
        registry,
        sessions,
        allow_insecure: true,
        auth_token: None,
    };

    let src_err = move_path(
        &ctx,
        MovePathInput {
            session_id: None,
            confirmation_id: None,
            src: "/Src/file.txt".to_string(),
            dst: "/Dst/file.txt".to_string(),
            overwrite: false,
        },
    )
    .await
    .unwrap_err();
    assert_eq!(src_err.code, McpErrorCode::ERR_STORAGE_READ_ONLY);

    let registry = registry_in(&dir);
    let mut src = StorageRecord::new(
        "Src".to_string(),
        "local".to_string(),
        json!({"root": src_root.clone()}),
    );
    src.mcp_exposed = true;
    src.mcp_policy.default_access = McpAccessMode::ReadWrite;
    let mut dst = StorageRecord::new(
        "Dst".to_string(),
        "local".to_string(),
        json!({"root": dst_root.clone()}),
    );
    dst.mcp_exposed = true;
    dst.mcp_policy.default_access = McpAccessMode::ReadWrite;
    dst.read_only = true;
    registry.save_all_atomic(&[src, dst]).unwrap();
    let sessions = sessions_in();
    let ctx = FsToolsContext {
        registry,
        sessions,
        allow_insecure: true,
        auth_token: None,
    };

    let dst_err = move_path(
        &ctx,
        MovePathInput {
            session_id: None,
            confirmation_id: None,
            src: "/Src/file.txt".to_string(),
            dst: "/Dst/file.txt".to_string(),
            overwrite: false,
        },
    )
    .await
    .unwrap_err();
    assert_eq!(dst_err.code, McpErrorCode::ERR_STORAGE_READ_ONLY);
}

#[tokio::test]
async fn move_path_rejects_directory_source() {
    let dir = TempDir::new().unwrap();
    let src_root = dir.path().join("src");
    let dst_root = dir.path().join("dst");
    std::fs::create_dir_all(src_root.join("docs")).unwrap();
    std::fs::create_dir_all(&dst_root).unwrap();

    let registry = registry_in(&dir);
    let mut src = StorageRecord::new(
        "Src".to_string(),
        "local".to_string(),
        json!({"root": src_root}),
    );
    src.mcp_exposed = true;
    src.mcp_policy.default_access = McpAccessMode::ReadWrite;
    let mut dst = StorageRecord::new(
        "Dst".to_string(),
        "local".to_string(),
        json!({"root": dst_root}),
    );
    dst.mcp_exposed = true;
    dst.mcp_policy.default_access = McpAccessMode::ReadWrite;
    registry.save_all_atomic(&[src, dst]).unwrap();
    let sessions = sessions_in();
    let ctx = FsToolsContext {
        registry,
        sessions,
        allow_insecure: true,
        auth_token: None,
    };

    let err = move_path(
        &ctx,
        MovePathInput {
            session_id: None,
            confirmation_id: None,
            src: "/Src/docs".to_string(),
            dst: "/Dst/docs".to_string(),
            overwrite: false,
        },
    )
    .await
    .unwrap_err();

    assert_eq!(err.code, McpErrorCode::ERR_IS_A_DIRECTORY);
}

#[tokio::test]
async fn search_paths_returns_lexicographic_matches() {
    let dir = TempDir::new().unwrap();
    let local_root = dir.path().join("local");
    std::fs::create_dir_all(local_root.join("docs").join("nested")).unwrap();
    std::fs::write(local_root.join("docs").join("alpha.txt"), "a").unwrap();
    std::fs::write(
        local_root.join("docs").join("nested").join("alpha-2.txt"),
        "b",
    )
    .unwrap();
    std::fs::write(local_root.join("docs").join("beta.txt"), "c").unwrap();

    let registry = registry_in(&dir);
    let mut storage = StorageRecord::new(
        "Local".to_string(),
        "local".to_string(),
        json!({"root": local_root}),
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

    let out = search_paths(
        &ctx,
        SearchPathsInput {
            session_id: None,
            path: "/Local/docs".to_string(),
            pattern: "alpha".to_string(),
            max_results: 10,
        },
    )
    .await
    .unwrap();

    assert_eq!(
        out.matches,
        vec![
            "/Local/docs/alpha.txt".to_string(),
            "/Local/docs/nested/alpha-2.txt".to_string()
        ]
    );
}

#[tokio::test]
async fn generate_download_link_local_backend_returns_presign_not_supported() {
    let dir = TempDir::new().unwrap();
    let local_root = dir.path().join("local");
    std::fs::create_dir_all(&local_root).unwrap();
    std::fs::write(local_root.join("file.txt"), "hello").unwrap();

    let registry = registry_in(&dir);
    let mut storage = StorageRecord::new(
        "Local".to_string(),
        "local".to_string(),
        json!({"root": local_root}),
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

    let err = generate_download_link(
        &ctx,
        GenerateDownloadLinkInput {
            session_id: None,
            confirmation_id: None,
            path: "/Local/file.txt".to_string(),
            expires_seconds: 900,
        },
    )
    .await
    .unwrap_err();

    assert_eq!(err.code, McpErrorCode::ERR_PRESIGN_NOT_SUPPORTED);
}

#[tokio::test]
async fn list_versions_local_backend_returns_not_supported() {
    let dir = TempDir::new().unwrap();
    let local_root = dir.path().join("local");
    std::fs::create_dir_all(&local_root).unwrap();
    std::fs::write(local_root.join("file.txt"), "hello").unwrap();

    let registry = registry_in(&dir);
    let mut storage = StorageRecord::new(
        "Local".to_string(),
        "local".to_string(),
        json!({"root": local_root}),
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

    let err = list_versions(
        &ctx,
        ListVersionsInput {
            path: "/Local/file.txt".to_string(),
            limit: 100,
            cursor: None,
            session_id: None,
        },
    )
    .await
    .unwrap_err();

    assert_eq!(err.code, McpErrorCode::ERR_VERSIONS_NOT_SUPPORTED);
}

#[tokio::test]
async fn write_file_enforces_four_mib_cap() {
    let dir = TempDir::new().unwrap();
    let local_root = dir.path().join("local");
    std::fs::create_dir_all(&local_root).unwrap();

    let registry = registry_in(&dir);
    let mut storage = StorageRecord::new(
        "Local".to_string(),
        "local".to_string(),
        json!({"root": local_root}),
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

    let max = super::write_file::MAX_MCP_WRITE_BYTES;
    let ok = write_file(
        &ctx,
        WriteFileInput {
            session_id: None,
            user_metadata: None,
            confirmation_id: None,
            path: "/Local/at-limit.txt".to_string(),
            content: "x".repeat(max),
            encoding: "utf-8".to_string(),
            overwrite: false,
            create_parents: false,
        },
    )
    .await
    .unwrap();
    assert_eq!(ok.written_bytes as usize, max);

    let err = write_file(
        &ctx,
        WriteFileInput {
            session_id: None,
            user_metadata: None,
            confirmation_id: None,
            path: "/Local/over-limit.txt".to_string(),
            content: "x".repeat(max + 1),
            encoding: "utf-8".to_string(),
            overwrite: false,
            create_parents: false,
        },
    )
    .await
    .unwrap_err();
    assert_eq!(err.code, McpErrorCode::ERR_INVALID_PATH);
    assert!(!local_root.join("over-limit.txt").exists());
}

#[tokio::test]
async fn write_file_atomic_create_succeeds_and_never_overwrites() {
    let dir = TempDir::new().unwrap();
    let local_root = dir.path().join("local");
    std::fs::create_dir_all(&local_root).unwrap();
    std::fs::write(local_root.join("existing.txt"), "original").unwrap();

    let registry = registry_in(&dir);
    let mut storage = StorageRecord::new(
        "Local".to_string(),
        "local".to_string(),
        json!({"root": local_root}),
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

    let fresh = write_file(
        &ctx,
        WriteFileInput {
            session_id: None,
            user_metadata: None,
            confirmation_id: None,
            path: "/Local/fresh.txt".to_string(),
            content: "first".to_string(),
            encoding: "utf-8".to_string(),
            overwrite: false,
            create_parents: false,
        },
    )
    .await
    .unwrap();
    assert_eq!(fresh.written_bytes, 5);

    // A concurrent second creator gets AlreadyExists and never overwrites.
    let err = write_file(
        &ctx,
        WriteFileInput {
            session_id: None,
            user_metadata: None,
            confirmation_id: None,
            path: "/Local/fresh.txt".to_string(),
            content: "overwritten".to_string(),
            encoding: "utf-8".to_string(),
            overwrite: false,
            create_parents: false,
        },
    )
    .await
    .unwrap_err();
    assert_eq!(err.code, McpErrorCode::ERR_ALREADY_EXISTS);
    assert_eq!(
        std::fs::read_to_string(local_root.join("fresh.txt")).unwrap(),
        "first"
    );
    assert_eq!(
        std::fs::read_to_string(local_root.join("existing.txt")).unwrap(),
        "original"
    );
}

#[test]
fn write_file_rejects_unsupported_atomic_no_overwrite_backend() {
    let capability = opendal::Capability::default();
    assert!(!super::write_file::supports_atomic_no_overwrite(
        &capability
    ));
    let supported = opendal::Capability {
        write_with_if_not_exists: true,
        ..Default::default()
    };
    assert!(super::write_file::supports_atomic_no_overwrite(&supported));
}
