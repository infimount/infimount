use crate::errors::{err_with_details, McpErrorCode, McpResult};
use crate::registry::{StorageRecord, StorageRegistry};
use serde_json::json;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FsOp {
    ListDir,
    StatPath,
    ReadFile,
    WriteFile,
    Mkdir,
    MovePath,
    CopyPath,
    DeletePath,
    SearchPaths,
    GenerateDownloadLink,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedPath {
    pub normalized: String,
    pub storage_name: Option<String>,
    pub backend_path: String,
    pub is_root: bool,
}

#[derive(Debug, Clone)]
pub struct ResolvedStoragePath {
    pub parsed: ParsedPath,
    pub storage: StorageRecord,
}

pub fn normalize_absolute_path(input: &str) -> McpResult<String> {
    let trimmed = input.trim();
    if trimmed.is_empty() || !trimmed.starts_with('/') {
        return Err(err_with_details(
            McpErrorCode::ERR_INVALID_PATH,
            "path must be an absolute path starting with '/'",
            json!({ "path": input }),
        ));
    }

    let parts: Vec<&str> = trimmed.split('/').filter(|s| !s.is_empty()).collect();
    if parts.is_empty() {
        return Ok("/".to_string());
    }

    Ok(format!("/{}", parts.join("/")))
}

pub fn parse_mcp_path(input: &str) -> McpResult<ParsedPath> {
    let normalized = normalize_absolute_path(input)?;

    if normalized == "/" {
        return Ok(ParsedPath {
            normalized,
            storage_name: None,
            backend_path: String::new(),
            is_root: true,
        });
    }

    let parts: Vec<&str> = normalized
        .trim_start_matches('/')
        .split('/')
        .collect::<Vec<_>>();

    let storage_name = parts.first().map(|s| (*s).to_string());
    let backend_path = if parts.len() > 1 {
        parts[1..].join("/")
    } else {
        String::new()
    };

    Ok(ParsedPath {
        normalized,
        storage_name,
        backend_path,
        is_root: false,
    })
}

pub fn enforce_root_operation(op: FsOp, parsed: &ParsedPath) -> McpResult<()> {
    if !parsed.is_root {
        return Ok(());
    }

    let allowed = matches!(op, FsOp::ListDir | FsOp::StatPath);
    if allowed {
        return Ok(());
    }

    Err(err_with_details(
        McpErrorCode::ERR_ROOT_OPERATION_NOT_ALLOWED,
        "operation not allowed on root path '/'",
        json!({ "path": parsed.normalized, "operation": format!("{op:?}") }),
    ))
}

pub fn resolve_storage_path(
    registry: &StorageRegistry,
    path: &str,
) -> McpResult<ResolvedStoragePath> {
    let parsed = parse_mcp_path(path)?;
    let storage_name = parsed.storage_name.clone().ok_or_else(|| {
        err_with_details(
            McpErrorCode::ERR_STORAGE_NOT_FOUND,
            "storage name is required",
            json!({ "path": parsed.normalized }),
        )
    })?;
    let storage = registry.find_by_name(&storage_name)?;

    Ok(ResolvedStoragePath { parsed, storage })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::registry::StorageRegistry;
    use tempfile::TempDir;

    #[test]
    fn normalize_collapses_slashes_and_trailing() {
        let got = normalize_absolute_path("//photos///2026//").unwrap();
        assert_eq!(got, "/photos/2026");
    }

    #[test]
    fn parse_storage_root_backend_empty() {
        let parsed = parse_mcp_path("/PhotosS3/").unwrap();
        assert_eq!(parsed.storage_name.as_deref(), Some("PhotosS3"));
        assert_eq!(parsed.backend_path, "");
        assert!(!parsed.is_root);
    }

    #[test]
    fn root_write_denied() {
        let parsed = parse_mcp_path("/").unwrap();
        let err = enforce_root_operation(FsOp::WriteFile, &parsed).unwrap_err();
        assert_eq!(err.code, McpErrorCode::ERR_ROOT_OPERATION_NOT_ALLOWED);
    }

    #[test]
    fn resolver_does_not_apply_root_guard() {
        let dir = TempDir::new().unwrap();
        let registry = StorageRegistry::new(Some(dir.path().join("storages.json")));
        let err = resolve_storage_path(&registry, "/").unwrap_err();
        assert_ne!(err.code, McpErrorCode::ERR_ROOT_OPERATION_NOT_ALLOWED);
    }
}
