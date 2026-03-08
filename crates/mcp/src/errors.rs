use serde::Serialize;
use serde_json::json;

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[allow(non_camel_case_types)]
pub enum McpErrorCode {
    ERR_INVALID_PATH,
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
    ERR_INTERNAL,
}

#[derive(Debug, Clone, Serialize)]
pub struct McpError {
    pub code: McpErrorCode,
    pub message: String,
    pub details: serde_json::Value,
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

pub fn map_opendal_error(err: &opendal::Error, fallback: McpErrorCode) -> McpError {
    let code = match err.kind() {
        opendal::ErrorKind::NotFound => McpErrorCode::ERR_PATH_NOT_FOUND,
        opendal::ErrorKind::PermissionDenied => McpErrorCode::ERR_PERMISSION_DENIED,
        opendal::ErrorKind::AlreadyExists => McpErrorCode::ERR_ALREADY_EXISTS,
        _ => fallback,
    };

    err_with_details(
        code,
        "storage operation failed",
        json!({ "backend_error": err.to_string() }),
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
        json!({ "io_error": err.to_string() }),
    )
}
