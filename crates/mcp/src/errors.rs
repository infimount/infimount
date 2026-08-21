use serde::Serialize;
use serde_json::json;

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[allow(non_camel_case_types)]
pub enum McpErrorCode {
    ERR_INVALID_PATH,
    ERR_INVALID_POLICY,
    ERR_ROOT_OPERATION_NOT_ALLOWED,
    ERR_STORAGE_NOT_FOUND,
    ERR_STORAGE_DISABLED,
    ERR_STORAGE_NOT_EXPOSED,
    ERR_STORAGE_READ_ONLY,
    ERR_INVALID_STORAGE_NAME,
    ERR_STORAGE_NAME_CONFLICT,
    ERR_PATH_NOT_FOUND,
    ERR_NOT_A_DIRECTORY,
    ERR_IS_A_DIRECTORY,
    ERR_PARENT_NOT_FOUND,
    ERR_NOT_EMPTY_OR_DIR,
    ERR_ALREADY_EXISTS,
    ERR_PERMISSION_DENIED,
    ERR_TEXT_DECODE_FAILED,
    ERR_PRESIGN_NOT_SUPPORTED,
    ERR_REGISTRY_LOCK_TIMEOUT,
    ERR_BACKEND_UNSUPPORTED,
    ERR_VERSIONS_NOT_SUPPORTED,
    ERR_VERSIONS_NOT_ENABLED,
    ERR_SESSION_NOT_FOUND,
    ERR_SESSION_FORBIDDEN,
    ERR_UNAUTHORIZED,
    ERR_MCP_POLICY_DENIED,
    ERR_CONFIRMATION_REQUIRED,
    ERR_SECRET_MIGRATION_FAILED,
    ERR_SECRET_STORE_UNAVAILABLE,
    ERR_SECRET_NOT_FOUND,
    ERR_SECRET_CLEANUP_PENDING,
    ERR_OAUTH_SESSION_EXPIRED,
    ERR_OAUTH_SESSION_ALREADY_USED,
    ERR_OAUTH_SESSION_IN_USE,
    ERR_OAUTH_SESSION_NOT_FOUND,
    ERR_IMPORT_PREVIEW_STALE,
    ERR_IMPORT_PREVIEW_EXPIRED,
    ERR_IMPORT_PREVIEW_MISMATCH,
    ERR_IMPORT_CONFIRMATION_REQUIRED,
    ERR_BACKUP_DECRYPTION_FAILED,
    ERR_WORKSPACE_SCHEMA_UNSUPPORTED,
    ERR_WORKSPACE_STORAGE_NAMESPACE_CHANGED,
    ERR_STORAGE_NAMESPACE_IN_USE,
    ERR_STORAGE_HAS_WORKSPACES,
    ERR_WORKSPACE_POLICY_MANAGED,
    ERR_TRANSFER_NAMESPACE_CONFLICT,
    ERR_INVALID_STORAGE_CREDENTIALS,
    ERR_INTERNAL,
}

#[derive(Debug, Clone, Serialize)]
pub struct McpError {
    pub code: McpErrorCode,
    pub message: String,
    pub details: serde_json::Value,
}

impl std::fmt::Display for McpError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "[{:?}] {}: {}", self.code, self.message, self.details)
    }
}

pub type McpResult<T> = Result<T, McpError>;

#[derive(Debug, Clone, Serialize)]
pub struct SuccessEnvelope<T>
where
    T: Serialize,
{
    pub ok: bool,
    pub data: T,
}

#[derive(Debug, Clone, Serialize)]
pub struct ErrorEnvelope {
    pub ok: bool,
    pub error: McpError,
}

pub fn ok<T: Serialize>(data: T) -> SuccessEnvelope<T> {
    SuccessEnvelope { ok: true, data }
}

pub fn fail(error: McpError) -> ErrorEnvelope {
    ErrorEnvelope { ok: false, error }
}

pub fn wrap_json<T: Serialize>(result: McpResult<T>) -> serde_json::Value {
    match result {
        Ok(data) => serde_json::to_value(ok(data)).unwrap_or_else(|_| {
            json!({
                "ok": false,
                "error": {
                    "code": "ERR_INTERNAL",
                    "message": "failed to serialize success envelope",
                    "details": {}
                }
            })
        }),
        Err(error) => serde_json::to_value(fail(error)).unwrap_or_else(|_| {
            json!({
                "ok": false,
                "error": {
                    "code": "ERR_INTERNAL",
                    "message": "failed to serialize error envelope",
                    "details": {}
                }
            })
        }),
    }
}

pub fn err(code: McpErrorCode, message: impl Into<String>) -> McpError {
    McpError {
        code,
        message: message.into(),
        details: json!({}),
    }
}

pub fn err_with_details(
    code: McpErrorCode,
    message: impl Into<String>,
    details: serde_json::Value,
) -> McpError {
    McpError {
        code,
        message: message.into(),
        details,
    }
}

/// Convert a Serde parse failure into a sanitized error that exposes only a
/// safe category and the line/column when available, never the raw Serde
/// diagnostic (which can echo credential-bearing input) nor the offending value.
pub fn sanitized_parse_error(
    code: McpErrorCode,
    message: impl Into<String>,
    category: &str,
    error: &serde_json::Error,
) -> McpError {
    let mut details = serde_json::Map::new();
    details.insert("category".to_string(), json!(category));
    let line = error.line();
    let column = error.column();
    if line > 0 {
        details.insert("line".to_string(), json!(line));
    }
    if column > 0 {
        details.insert("column".to_string(), json!(column));
    }
    err_with_details(code, message, serde_json::Value::Object(details))
}

pub fn map_opendal_error(err: &opendal::Error, fallback: McpErrorCode) -> McpError {
    let code = match err.kind() {
        opendal::ErrorKind::NotFound => McpErrorCode::ERR_PATH_NOT_FOUND,
        opendal::ErrorKind::PermissionDenied => McpErrorCode::ERR_PERMISSION_DENIED,
        opendal::ErrorKind::AlreadyExists => McpErrorCode::ERR_ALREADY_EXISTS,
        _ => fallback,
    };

    let kind_str = format!("{:?}", err.kind());
    err_with_details(
        code,
        "storage operation failed",
        json!({
            "kind": kind_str,
            "temporary": err.is_temporary(),
            "operation": "storage",
        }),
    )
}

pub fn map_core_error(err: &infimount_core::CoreError) -> McpError {
    let code = match err {
        infimount_core::CoreError::Config(_) => McpErrorCode::ERR_INTERNAL,
        infimount_core::CoreError::Io(io_err) => match io_err.kind() {
            std::io::ErrorKind::NotFound => McpErrorCode::ERR_PATH_NOT_FOUND,
            std::io::ErrorKind::PermissionDenied => McpErrorCode::ERR_PERMISSION_DENIED,
            std::io::ErrorKind::AlreadyExists => McpErrorCode::ERR_ALREADY_EXISTS,
            _ => McpErrorCode::ERR_INTERNAL,
        },
        _ => McpErrorCode::ERR_INTERNAL,
    };
    err_with_details(
        code,
        "storage configuration operation failed",
        json!({ "kind": "Core", "temporary": false, "operation": "configuration" }),
    )
}

pub fn map_io_error(err: &std::io::Error, fallback: McpErrorCode) -> McpError {
    let code = match err.kind() {
        std::io::ErrorKind::NotFound => McpErrorCode::ERR_PATH_NOT_FOUND,
        std::io::ErrorKind::PermissionDenied => McpErrorCode::ERR_PERMISSION_DENIED,
        std::io::ErrorKind::AlreadyExists => McpErrorCode::ERR_ALREADY_EXISTS,
        _ => fallback,
    };

    err_with_details(
        code,
        "I/O operation failed",
        json!({ "kind": format!("{:?}", err.kind()), "temporary": false }),
    )
}
