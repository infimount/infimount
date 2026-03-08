use super::*;
use crate::errors::McpErrorCode;
use crate::registry::StorageRecord;
use serde_json::json;
use tempfile::TempDir;

fn registry_in(dir: &TempDir) -> crate::registry::StorageRegistry {
    crate::registry::StorageRegistry::new(Some(dir.path().join("storages.json")))
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

    let mut b = StorageRecord::new(
        "alpha".to_string(),
        "local".to_string(),
        json!({"root": "/tmp"}),
    );
    b.enabled = true;
    b.mcp_exposed = true;

    let mut hidden = StorageRecord::new(
        "hidden".to_string(),
        "local".to_string(),
        json!({"root": "/tmp"}),
    );
    hidden.enabled = true;
    hidden.mcp_exposed = false;

    registry.save_all_atomic(&[a, hidden, b]).unwrap();

    let ctx = FsToolsContext { registry };
    let out = list_dir(
        &ctx,
        ListDirInput {
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
    let storage = StorageRecord::new(
        "Local".to_string(),
        "local".to_string(),
        json!({"root": local_root}),
    );
    registry.save_all_atomic(&[storage]).unwrap();

    let ctx = FsToolsContext { registry };
    let out = list_dir(
        &ctx,
        ListDirInput {
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
    let storage = StorageRecord::new(
        "Local".to_string(),
        "local".to_string(),
        json!({"root": local_root}),
    );
    registry.save_all_atomic(&[storage]).unwrap();

    let ctx = FsToolsContext { registry };
    let out = list_dir(
        &ctx,
        ListDirInput {
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
async fn list_dir_cursor_offset_applies_after_sorting() {
    let dir = TempDir::new().unwrap();
    let registry = registry_in(&dir);

    let s1 = StorageRecord::new(
        "zeta".to_string(),
        "local".to_string(),
        json!({"root": "/tmp"}),
    );
    let s2 = StorageRecord::new(
        "alpha".to_string(),
        "local".to_string(),
        json!({"root": "/tmp"}),
    );
    let s3 = StorageRecord::new(
        "beta".to_string(),
        "local".to_string(),
        json!({"root": "/tmp"}),
    );
    registry.save_all_atomic(&[s1, s2, s3]).unwrap();
    let ctx = FsToolsContext { registry };

    let first = list_dir(
        &ctx,
        ListDirInput {
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
    let ctx = FsToolsContext { registry };

    let err = list_dir(
        &ctx,
        ListDirInput {
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
    let ctx = FsToolsContext { registry };

    let out = stat_path(
        &ctx,
        StatPathInput {
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
    let ctx = FsToolsContext { registry };

    let err = read_file(
        &ctx,
        ReadFileInput {
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
    let storage = StorageRecord::new(
        "Local".to_string(),
        "local".to_string(),
        json!({"root": local_root}),
    );
    registry.save_all_atomic(&[storage]).unwrap();
    let ctx = FsToolsContext { registry };

    let missing_err = read_file(
        &ctx,
        ReadFileInput {
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
    let storage = StorageRecord::new(
        "Local".to_string(),
        "local".to_string(),
        json!({"root": local_root}),
    );
    registry.save_all_atomic(&[storage]).unwrap();
    let ctx = FsToolsContext { registry };

    let out = read_file(
        &ctx,
        ReadFileInput {
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
    let storage = StorageRecord::new(
        "Local".to_string(),
        "local".to_string(),
        json!({"root": local_root}),
    );
    registry.save_all_atomic(&[storage]).unwrap();
    let ctx = FsToolsContext { registry };

    let out = read_file(
        &ctx,
        ReadFileInput {
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
    let storage = StorageRecord::new(
        "Local".to_string(),
        "local".to_string(),
        json!({"root": local_root}),
    );
    registry.save_all_atomic(&[storage]).unwrap();
    let ctx = FsToolsContext { registry };

    let err = read_file(
        &ctx,
        ReadFileInput {
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
    let ctx = FsToolsContext { registry };

    let err = read_file(
        &ctx,
        ReadFileInput {
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
    storage.read_only = true;
    registry.save_all_atomic(&[storage]).unwrap();
    let ctx = FsToolsContext { registry };

    let err = mkdir(
        &ctx,
        MkdirInput {
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
    let storage = StorageRecord::new(
        "Local".to_string(),
        "local".to_string(),
        json!({"root": local_root}),
    );
    registry.save_all_atomic(&[storage]).unwrap();
    let ctx = FsToolsContext { registry };

    let err = mkdir(
        &ctx,
        MkdirInput {
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
    let storage = StorageRecord::new(
        "Local".to_string(),
        "local".to_string(),
        json!({"root": local_root.clone()}),
    );
    registry.save_all_atomic(&[storage]).unwrap();
    let ctx = FsToolsContext { registry };

    let out = mkdir(
        &ctx,
        MkdirInput {
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
    let storage = StorageRecord::new(
        "Local".to_string(),
        "local".to_string(),
        json!({"root": local_root}),
    );
    registry.save_all_atomic(&[storage]).unwrap();
    let ctx = FsToolsContext { registry };

    let err = mkdir(
        &ctx,
        MkdirInput {
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
    let storage = StorageRecord::new(
        "Local".to_string(),
        "local".to_string(),
        json!({"root": local_root}),
    );
    registry.save_all_atomic(&[storage]).unwrap();
    let ctx = FsToolsContext { registry };

    let out = mkdir(
        &ctx,
        MkdirInput {
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
    storage.read_only = true;
    registry.save_all_atomic(&[storage]).unwrap();
    let ctx = FsToolsContext { registry };

    let err = write_file(
        &ctx,
        WriteFileInput {
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
    let storage = StorageRecord::new(
        "Local".to_string(),
        "local".to_string(),
        json!({"root": local_root}),
    );
    registry.save_all_atomic(&[storage]).unwrap();
    let ctx = FsToolsContext { registry };

    let err = write_file(
        &ctx,
        WriteFileInput {
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
    let storage = StorageRecord::new(
        "Local".to_string(),
        "local".to_string(),
        json!({"root": local_root.clone()}),
    );
    registry.save_all_atomic(&[storage]).unwrap();
    let ctx = FsToolsContext { registry };

    let out = write_file(
        &ctx,
        WriteFileInput {
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
    let storage = StorageRecord::new(
        "Local".to_string(),
        "local".to_string(),
        json!({"root": local_root.clone()}),
    );
    registry.save_all_atomic(&[storage]).unwrap();
    let ctx = FsToolsContext { registry };

    let err = write_file(
        &ctx,
        WriteFileInput {
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
    let storage = StorageRecord::new(
        "Local".to_string(),
        "local".to_string(),
        json!({"root": local_root}),
    );
    registry.save_all_atomic(&[storage]).unwrap();
    let ctx = FsToolsContext { registry };

    let dir_err = write_file(
        &ctx,
        WriteFileInput {
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
}

#[tokio::test]
async fn delete_path_root_is_rejected() {
    let dir = TempDir::new().unwrap();
    let registry = registry_in(&dir);
    let ctx = FsToolsContext { registry };

    let err = delete_path(
        &ctx,
        DeletePathInput {
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
    storage.read_only = true;
    registry.save_all_atomic(&[storage]).unwrap();
    let ctx = FsToolsContext { registry };

    let err = delete_path(
        &ctx,
        DeletePathInput {
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
    let storage = StorageRecord::new(
        "Local".to_string(),
        "local".to_string(),
        json!({"root": local_root.clone()}),
    );
    registry.save_all_atomic(&[storage]).unwrap();
    let ctx = FsToolsContext { registry };

    let out = delete_path(
        &ctx,
        DeletePathInput {
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
    let storage = StorageRecord::new(
        "Local".to_string(),
        "local".to_string(),
        json!({"root": local_root}),
    );
    registry.save_all_atomic(&[storage]).unwrap();
    let ctx = FsToolsContext { registry };

    let err = delete_path(
        &ctx,
        DeletePathInput {
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
    let storage = StorageRecord::new(
        "Local".to_string(),
        "local".to_string(),
        json!({"root": local_root.clone()}),
    );
    registry.save_all_atomic(&[storage]).unwrap();
    let ctx = FsToolsContext { registry };

    let out = delete_path(
        &ctx,
        DeletePathInput {
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
    let src = StorageRecord::new(
        "Src".to_string(),
        "local".to_string(),
        json!({"root": src_root}),
    );
    let mut dst = StorageRecord::new(
        "Dst".to_string(),
        "local".to_string(),
        json!({"root": dst_root}),
    );
    dst.read_only = true;
    registry.save_all_atomic(&[src, dst]).unwrap();
    let ctx = FsToolsContext { registry };

    let err = copy_path(
        &ctx,
        CopyPathInput {
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
    let src = StorageRecord::new(
        "Src".to_string(),
        "local".to_string(),
        json!({"root": src_root}),
    );
    let dst = StorageRecord::new(
        "Dst".to_string(),
        "local".to_string(),
        json!({"root": dst_root}),
    );
    registry.save_all_atomic(&[src, dst]).unwrap();
    let ctx = FsToolsContext { registry };

    let err = copy_path(
        &ctx,
        CopyPathInput {
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
    let src = StorageRecord::new(
        "Src".to_string(),
        "local".to_string(),
        json!({"root": src_root}),
    );
    let dst = StorageRecord::new(
        "Dst".to_string(),
        "local".to_string(),
        json!({"root": dst_root}),
    );
    registry.save_all_atomic(&[src, dst]).unwrap();
    let ctx = FsToolsContext { registry };

    let err = copy_path(
        &ctx,
        CopyPathInput {
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
async fn copy_path_same_storage_file_success() {
    let dir = TempDir::new().unwrap();
    let root = dir.path().join("root");
    std::fs::create_dir_all(root.join("out")).unwrap();
    std::fs::write(root.join("file.txt"), "hello").unwrap();

    let registry = registry_in(&dir);
    let storage = StorageRecord::new(
        "Local".to_string(),
        "local".to_string(),
        json!({"root": root.clone()}),
    );
    registry.save_all_atomic(&[storage]).unwrap();
    let ctx = FsToolsContext { registry };

    let out = copy_path(
        &ctx,
        CopyPathInput {
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
    let payload = vec![b'x'; (common::COPY_CHUNK_SIZE as usize) + 17];
    std::fs::write(src_root.join("large.bin"), &payload).unwrap();

    let registry = registry_in(&dir);
    let src = StorageRecord::new(
        "Src".to_string(),
        "local".to_string(),
        json!({"root": src_root}),
    );
    let dst = StorageRecord::new(
        "Dst".to_string(),
        "local".to_string(),
        json!({"root": dst_root.clone()}),
    );
    registry.save_all_atomic(&[src, dst]).unwrap();
    let ctx = FsToolsContext { registry };

    let out = copy_path(
        &ctx,
        CopyPathInput {
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
async fn copy_path_recursive_preserves_structure() {
    let dir = TempDir::new().unwrap();
    let src_root = dir.path().join("src");
    let dst_root = dir.path().join("dst");
    std::fs::create_dir_all(src_root.join("docs").join("nested")).unwrap();
    std::fs::create_dir_all(&dst_root).unwrap();
    std::fs::write(src_root.join("docs").join("a.txt"), "a").unwrap();
    std::fs::write(src_root.join("docs").join("nested").join("b.txt"), "b").unwrap();

    let registry = registry_in(&dir);
    let src = StorageRecord::new(
        "Src".to_string(),
        "local".to_string(),
        json!({"root": src_root}),
    );
    let dst = StorageRecord::new(
        "Dst".to_string(),
        "local".to_string(),
        json!({"root": dst_root.clone()}),
    );
    registry.save_all_atomic(&[src, dst]).unwrap();
    let ctx = FsToolsContext { registry };

    let out = copy_path(
        &ctx,
        CopyPathInput {
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
async fn move_path_same_storage_file_success() {
    let dir = TempDir::new().unwrap();
    let root = dir.path().join("root");
    std::fs::create_dir_all(root.join("out")).unwrap();
    std::fs::write(root.join("file.txt"), "hello").unwrap();

    let registry = registry_in(&dir);
    let storage = StorageRecord::new(
        "Local".to_string(),
        "local".to_string(),
        json!({"root": root.clone()}),
    );
    registry.save_all_atomic(&[storage]).unwrap();
    let ctx = FsToolsContext { registry };

    let out = move_path(
        &ctx,
        MovePathInput {
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
    let src = StorageRecord::new(
        "Src".to_string(),
        "local".to_string(),
        json!({"root": src_root.clone()}),
    );
    let dst = StorageRecord::new(
        "Dst".to_string(),
        "local".to_string(),
        json!({"root": dst_root.clone()}),
    );
    registry.save_all_atomic(&[src, dst]).unwrap();
    let ctx = FsToolsContext { registry };

    let out = move_path(
        &ctx,
        MovePathInput {
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
    let src = StorageRecord::new(
        "Src".to_string(),
        "local".to_string(),
        json!({"root": src_root}),
    );
    let dst = StorageRecord::new(
        "Dst".to_string(),
        "local".to_string(),
        json!({"root": dst_root}),
    );
    registry.save_all_atomic(&[src, dst]).unwrap();
    let ctx = FsToolsContext { registry };

    let err = move_path(
        &ctx,
        MovePathInput {
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
    let src = StorageRecord::new(
        "Src".to_string(),
        "local".to_string(),
        json!({"root": src_root.clone()}),
    );
    let dst = StorageRecord::new(
        "Dst".to_string(),
        "local".to_string(),
        json!({"root": dst_root.clone()}),
    );
    registry.save_all_atomic(&[src, dst]).unwrap();
    let ctx = FsToolsContext { registry };

    let out = move_path(
        &ctx,
        MovePathInput {
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
    let src = StorageRecord::new(
        "Src".to_string(),
        "local".to_string(),
        json!({"root": src_root}),
    );
    let dst = StorageRecord::new(
        "Dst".to_string(),
        "local".to_string(),
        json!({"root": dst_root}),
    );
    registry.save_all_atomic(&[src, dst]).unwrap();
    let ctx = FsToolsContext { registry };

    let err = move_path(
        &ctx,
        MovePathInput {
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
    src.read_only = true;
    let dst = StorageRecord::new(
        "Dst".to_string(),
        "local".to_string(),
        json!({"root": dst_root.clone()}),
    );
    registry.save_all_atomic(&[src, dst]).unwrap();
    let ctx = FsToolsContext { registry };

    let src_err = move_path(
        &ctx,
        MovePathInput {
            src: "/Src/file.txt".to_string(),
            dst: "/Dst/file.txt".to_string(),
            overwrite: false,
        },
    )
    .await
    .unwrap_err();
    assert_eq!(src_err.code, McpErrorCode::ERR_STORAGE_READ_ONLY);

    let registry = registry_in(&dir);
    let src = StorageRecord::new(
        "Src".to_string(),
        "local".to_string(),
        json!({"root": src_root.clone()}),
    );
    let mut dst = StorageRecord::new(
        "Dst".to_string(),
        "local".to_string(),
        json!({"root": dst_root.clone()}),
    );
    dst.read_only = true;
    registry.save_all_atomic(&[src, dst]).unwrap();
    let ctx = FsToolsContext { registry };

    let dst_err = move_path(
        &ctx,
        MovePathInput {
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
    let src = StorageRecord::new(
        "Src".to_string(),
        "local".to_string(),
        json!({"root": src_root}),
    );
    let dst = StorageRecord::new(
        "Dst".to_string(),
        "local".to_string(),
        json!({"root": dst_root}),
    );
    registry.save_all_atomic(&[src, dst]).unwrap();
    let ctx = FsToolsContext { registry };

    let err = move_path(
        &ctx,
        MovePathInput {
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
    let storage = StorageRecord::new(
        "Local".to_string(),
        "local".to_string(),
        json!({"root": local_root}),
    );
    registry.save_all_atomic(&[storage]).unwrap();
    let ctx = FsToolsContext { registry };

    let out = search_paths(
        &ctx,
        SearchPathsInput {
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
    let storage = StorageRecord::new(
        "Local".to_string(),
        "local".to_string(),
        json!({"root": local_root}),
    );
    registry.save_all_atomic(&[storage]).unwrap();
    let ctx = FsToolsContext { registry };

    let err = generate_download_link(
        &ctx,
        GenerateDownloadLinkInput {
            path: "/Local/file.txt".to_string(),
            expires_seconds: 900,
        },
    )
    .await
    .unwrap_err();

    assert_eq!(err.code, McpErrorCode::ERR_PRESIGN_NOT_SUPPORTED);
}
