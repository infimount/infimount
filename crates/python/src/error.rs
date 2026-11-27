//! Public errors contain stable codes, never backend diagnostics or credentials.
use infimount_core::CoreError;
use infimount_mcp::{McpError, McpErrorCode};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    Configuration,
    InvalidRequest,
    PermissionDenied,
    ConfirmationRequired,
    NotFound,
    AlreadyExists,
    LimitExceeded,
    Unsupported,
    Unavailable,
    Closed,
    Forked,
}

pub type Result<T> = std::result::Result<T, Error>;

impl Error {
    pub fn code(self) -> &'static str {
        match self {
            Self::Configuration => "invalid_configuration",
            Self::InvalidRequest => "invalid_request",
            Self::PermissionDenied => "permission_denied",
            Self::ConfirmationRequired => "confirmation_required",
            Self::NotFound => "not_found",
            Self::AlreadyExists => "already_exists",
            Self::LimitExceeded => "limit_exceeded",
            Self::Unsupported => "unsupported_operation",
            Self::Unavailable => "backend_unavailable",
            Self::Closed => "client_closed",
            Self::Forked => "client_cannot_be_used_after_fork",
        }
    }
}

impl From<opendal::Error> for Error {
    fn from(error: opendal::Error) -> Self {
        match error.kind() {
            opendal::ErrorKind::NotFound => Self::NotFound,
            opendal::ErrorKind::PermissionDenied => Self::PermissionDenied,
            opendal::ErrorKind::AlreadyExists | opendal::ErrorKind::ConditionNotMatch => {
                Self::AlreadyExists
            }
            opendal::ErrorKind::Unsupported => Self::Unsupported,
            opendal::ErrorKind::IsADirectory | opendal::ErrorKind::NotADirectory => {
                Self::InvalidRequest
            }
            _ => Self::Unavailable,
        }
    }
}

impl From<CoreError> for Error {
    fn from(error: CoreError) -> Self {
        match error {
            CoreError::Storage(error) => error.into(),
            CoreError::Config(message) if message.contains("cursor") => Self::InvalidRequest,
            CoreError::Config(_) => Self::InvalidRequest,
            _ => Self::Unavailable,
        }
    }
}

impl From<McpError> for Error {
    fn from(error: McpError) -> Self {
        match error.code {
            McpErrorCode::ERR_CONFIRMATION_REQUIRED => Self::ConfirmationRequired,
            McpErrorCode::ERR_MCP_POLICY_DENIED
            | McpErrorCode::ERR_PERMISSION_DENIED
            | McpErrorCode::ERR_STORAGE_DISABLED
            | McpErrorCode::ERR_STORAGE_NOT_EXPOSED
            | McpErrorCode::ERR_STORAGE_READ_ONLY => Self::PermissionDenied,
            McpErrorCode::ERR_PATH_NOT_FOUND | McpErrorCode::ERR_STORAGE_NOT_FOUND => {
                Self::NotFound
            }
            McpErrorCode::ERR_ALREADY_EXISTS => Self::AlreadyExists,
            McpErrorCode::ERR_BACKEND_UNSUPPORTED | McpErrorCode::ERR_PRESIGN_NOT_SUPPORTED => {
                Self::Unsupported
            }
            McpErrorCode::ERR_INTERNAL => Self::Unavailable,
            _ => Self::InvalidRequest,
        }
    }
}
