use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use serde::{Deserialize, Serialize};
use serde_json::json;
use sha2::{Digest, Sha256};
use std::sync::OnceLock;

use crate::errors::{err_with_details, map_opendal_error, McpErrorCode, McpResult};
use crate::opendal_adapter;
use crate::path::{enforce_root_operation, parse_mcp_path, resolve_storage_path, FsOp};
use crate::policy::McpOperation;

use super::common::{
    core_error_to_mcp_error, default_limit, enforce_storage_policy, sort_entries, EntryType,
    FsToolsContext, ListDirEntry,
};

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ListDirInput {
    pub path: String,
    #[serde(default)]
    pub recursive: bool,
    #[serde(default = "default_limit")]
    pub limit: u32,
    #[serde(default)]
    pub cursor: Option<String>,
    #[serde(default)]
    pub session_id: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct ListDirOutput {
    pub path: String,
    pub entries: Vec<ListDirEntry>,
    pub next_cursor: Option<String>,
    pub truncated: bool,
}

#[derive(Debug, Serialize, Deserialize)]
struct CursorV1 {
    v: u8,
    path: String,
    recursive: bool,
    revision: u64,
    offset: usize,
}

pub async fn list_dir(ctx: &FsToolsContext, input: ListDirInput) -> McpResult<ListDirOutput> {
    if input.limit == 0 || input.limit > 1000 {
        return Err(err_with_details(
            McpErrorCode::ERR_INVALID_PATH,
            "limit must be between 1 and 1000",
            json!({ "limit": input.limit }),
        ));
    }

    let parsed = parse_mcp_path(&input.path)?;
    enforce_root_operation(FsOp::ListDir, &parsed)?;

    if parsed.is_root {
        let storages = ctx.registry.list_exposed_enabled()?;
        let revision = storages
            .iter()
            .fold(storages.len() as u64, |revision, storage| {
                revision.wrapping_mul(31).wrapping_add(storage.revision)
            });
        let offset = decode_cursor(
            input.cursor.as_deref(),
            &parsed.normalized,
            input.recursive,
            revision,
        )?;
        let entries = storages
            .into_iter()
            .map(|storage| ListDirEntry {
                name: storage.name.clone(),
                path: format!("/{}", storage.name),
                entry_type: EntryType::Dir,
                size_bytes: None,
                modified_at: None,
                etag: None,
            })
            .collect::<Vec<_>>();

        return Ok(paginate_entries(
            parsed.normalized,
            entries,
            offset,
            input.limit as usize,
            input.recursive,
            revision,
        ));
    }

    let resolved = resolve_storage_path(&ctx.registry, &parsed.normalized)?;
    ctx.validate_session(
        input.session_id.as_deref(),
        &resolved.storage.name,
        Some(&resolved.parsed.backend_path),
    )
    .await?;
    enforce_storage_policy(
        &resolved.storage,
        &resolved.parsed.backend_path,
        McpOperation::List,
        false,
        false,
    )?;
    let op = opendal_adapter::build_operator(&resolved.storage, &ctx.registry)?;

    if !parsed.backend_path.is_empty() {
        let meta = op
            .stat(&parsed.backend_path)
            .await
            .map_err(|e| map_opendal_error(&e, McpErrorCode::ERR_INTERNAL))?;
        if !meta.is_dir() {
            return Err(err_with_details(
                McpErrorCode::ERR_NOT_A_DIRECTORY,
                "path is not a directory",
                json!({ "path": parsed.normalized }),
            ));
        }
    }

    let filter = |full_path: &str| -> infimount_core::models::Result<bool> {
        match enforce_storage_policy(
            &resolved.storage,
            full_path,
            McpOperation::List,
            false,
            false,
        ) {
            Ok(()) => Ok(true),
            Err(error) if error.code == McpErrorCode::ERR_MCP_POLICY_DENIED => Ok(false),
            Err(_) => Err(infimount_core::models::CoreError::Config(
                "storage policy evaluation failed".to_string(),
            )),
        }
    };
    let page = infimount_core::operations::list_entries_page_with_filter(
        &op,
        &resolved.parsed.backend_path,
        input.limit,
        input.cursor,
        input.recursive,
        resolved.storage.revision,
        filter,
    )
    .await
    .map_err(core_error_to_mcp_error)?;
    let storage_name = &resolved.storage.name;
    let mut entries = page
        .entries
        .into_iter()
        .map(|entry| {
            let backend_path = entry.path.trim_matches('/');
            ListDirEntry {
                name: entry.name,
                path: if backend_path.is_empty() {
                    format!("/{storage_name}")
                } else {
                    format!("/{storage_name}/{backend_path}")
                },
                entry_type: if entry.is_dir {
                    EntryType::Dir
                } else {
                    EntryType::File
                },
                size_bytes: (!entry.is_dir).then_some(entry.size),
                modified_at: entry.modified_at,
                etag: entry.etag,
            }
        })
        .collect::<Vec<_>>();
    sort_entries(&mut entries, input.recursive);

    Ok(ListDirOutput {
        path: parsed.normalized,
        entries,
        next_cursor: page.next_cursor,
        truncated: page.truncated,
    })
}

fn paginate_entries(
    path: String,
    entries: Vec<ListDirEntry>,
    offset: usize,
    limit: usize,
    recursive: bool,
    revision: u64,
) -> ListDirOutput {
    let start = offset.min(entries.len());
    let end = (start + limit).min(entries.len());
    let page = entries[start..end].to_vec();
    let next_cursor = if end < entries.len() {
        Some(encode_cursor(&path, recursive, revision, end))
    } else {
        None
    };

    ListDirOutput {
        path,
        entries: page,
        next_cursor,
        truncated: false,
    }
}

fn root_cursor_key() -> &'static [u8; 32] {
    static KEY: OnceLock<[u8; 32]> = OnceLock::new();
    KEY.get_or_init(|| {
        let mut digest = Sha256::new();
        digest.update(uuid::Uuid::new_v4().as_bytes());
        digest.update(uuid::Uuid::new_v4().as_bytes());
        digest.finalize().into()
    })
}

fn root_cursor_signature(payload: &str) -> [u8; 32] {
    let mut digest = Sha256::new();
    digest.update(root_cursor_key());
    digest.update(payload.as_bytes());
    digest.update(root_cursor_key());
    digest.finalize().into()
}

fn decode_cursor(
    cursor: Option<&str>,
    path: &str,
    recursive: bool,
    revision: u64,
) -> McpResult<usize> {
    let Some(cursor) = cursor else {
        return Ok(0);
    };
    if cursor.len() > 8 * 1024 {
        return Err(err_with_details(
            McpErrorCode::ERR_INVALID_PATH,
            "invalid cursor encoding",
            json!({}),
        ));
    }
    let (payload, signature) = cursor.split_once('.').ok_or_else(|| {
        err_with_details(
            McpErrorCode::ERR_INVALID_PATH,
            "invalid cursor encoding",
            json!({}),
        )
    })?;
    let supplied_signature = URL_SAFE_NO_PAD.decode(signature).map_err(|_| {
        err_with_details(
            McpErrorCode::ERR_INVALID_PATH,
            "invalid cursor encoding",
            json!({}),
        )
    })?;
    let expected_signature = root_cursor_signature(payload);
    if supplied_signature.len() != expected_signature.len()
        || supplied_signature
            .iter()
            .zip(expected_signature)
            .fold(0u8, |difference, (left, right)| difference | (left ^ right))
            != 0
    {
        return Err(err_with_details(
            McpErrorCode::ERR_INVALID_PATH,
            "invalid cursor signature",
            json!({}),
        ));
    }
    let raw = URL_SAFE_NO_PAD.decode(payload).map_err(|_| {
        err_with_details(
            McpErrorCode::ERR_INVALID_PATH,
            "invalid cursor encoding",
            json!({}),
        )
    })?;
    let parsed: CursorV1 = serde_json::from_slice(&raw).map_err(|_| {
        err_with_details(
            McpErrorCode::ERR_INVALID_PATH,
            "invalid cursor payload",
            json!({}),
        )
    })?;
    if parsed.v != 1 {
        return Err(err_with_details(
            McpErrorCode::ERR_INVALID_PATH,
            "unsupported cursor version",
            json!({ "cursor_version": parsed.v }),
        ));
    }
    if parsed.path != path || parsed.recursive != recursive || parsed.revision != revision {
        return Err(err_with_details(
            McpErrorCode::ERR_INVALID_PATH,
            "cursor does not match the current query or storage revision",
            json!({}),
        ));
    }
    Ok(parsed.offset)
}

fn encode_cursor(path: &str, recursive: bool, revision: u64, offset: usize) -> String {
    let bytes = serde_json::to_vec(&CursorV1 {
        v: 1,
        path: path.to_string(),
        recursive,
        revision,
        offset,
    })
    .unwrap_or_else(|_| b"{}".to_vec());
    let payload = URL_SAFE_NO_PAD.encode(bytes);
    let signature = URL_SAFE_NO_PAD.encode(root_cursor_signature(&payload));
    format!("{payload}.{signature}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cursor_is_bound_to_query_and_revision() {
        let cursor = encode_cursor("/storage/path", true, 9, 20);
        assert_eq!(
            decode_cursor(Some(&cursor), "/storage/path", true, 9).unwrap(),
            20
        );
        assert!(decode_cursor(Some(&cursor), "/storage/other", true, 9).is_err());
        assert!(decode_cursor(Some(&cursor), "/storage/path", false, 9).is_err());
        assert!(decode_cursor(Some(&cursor), "/storage/path", true, 10).is_err());
        let mut forged = cursor.into_bytes();
        forged[0] = if forged[0] == b'a' { b'b' } else { b'a' };
        assert!(decode_cursor(
            Some(std::str::from_utf8(&forged).unwrap()),
            "/storage/path",
            true,
            9,
        )
        .is_err());
    }
}
