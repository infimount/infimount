//! Stable, deliberately non-diagnostic errors for the public network boundary.
use axum::{http::StatusCode, response::IntoResponse, Json};
use serde_json::json;

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum HostedError {
    #[error("authentication required")]
    Unauthorized,
    #[error("access denied")]
    Forbidden,
    #[error("invalid request")]
    InvalidRequest,
    #[error("object or grant unavailable")]
    NotFound,
    #[error("destination exists or transfer state changed")]
    Conflict,
    #[error("configured limit exceeded")]
    Limit,
    #[error("operation is not supported by this storage")]
    Unsupported,
    #[error("service temporarily unavailable")]
    Unavailable,
    #[error("operation could not be completed")]
    Internal,
    #[error("invalid hosted configuration: {0}")]
    Config(&'static str),
}

pub type Result<T> = std::result::Result<T, HostedError>;

impl HostedError {
    pub fn code(self) -> &'static str {
        match self {
            Self::Unauthorized => "unauthorized",
            Self::Forbidden => "forbidden",
            Self::InvalidRequest => "invalid_request",
            Self::NotFound => "not_found",
            Self::Conflict => "conflict",
            Self::Limit => "limit_exceeded",
            Self::Unsupported => "unsupported",
            Self::Unavailable => "unavailable",
            Self::Internal | Self::Config(_) => "internal_error",
        }
    }

    pub fn status(self) -> StatusCode {
        match self {
            Self::Unauthorized => StatusCode::UNAUTHORIZED,
            Self::Forbidden => StatusCode::FORBIDDEN,
            Self::InvalidRequest => StatusCode::BAD_REQUEST,
            Self::NotFound => StatusCode::NOT_FOUND,
            Self::Conflict => StatusCode::CONFLICT,
            Self::Limit => StatusCode::PAYLOAD_TOO_LARGE,
            Self::Unsupported => StatusCode::NOT_IMPLEMENTED,
            Self::Unavailable => StatusCode::SERVICE_UNAVAILABLE,
            Self::Internal | Self::Config(_) => StatusCode::INTERNAL_SERVER_ERROR,
        }
    }
}

impl IntoResponse for HostedError {
    fn into_response(self) -> axum::response::Response {
        // Never expose backend URLs, credentials, tokens, paths, or parse input.
        (self.status(), Json(json!({"error": self.code()}))).into_response()
    }
}
