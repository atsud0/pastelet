use axum::{
    Json,
    http::StatusCode,
    response::{IntoResponse, Response},
};
use serde::Serialize;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum AppError {
    #[error("{0}")]
    BadRequest(&'static str),
    #[error("Unauthorized")]
    Unauthorized,
    #[error("Not found")]
    NotFound,
    #[error("Content too large")]
    PayloadTooLarge,
    #[error("Method not allowed")]
    MethodNotAllowed,
    #[error("database operation failed")]
    Database(#[from] sqlx::Error),
    #[error("template rendering failed")]
    Template(#[from] askama::Error),
    #[error("syntax rendering failed")]
    Highlight(#[from] syntect::Error),
    #[error("background computation failed")]
    Worker(#[from] tokio::task::JoinError),
    #[error("internal failure: {0}")]
    Internal(&'static str),
}

#[derive(Serialize)]
struct ErrorEnvelope {
    error: ErrorBody,
}

#[derive(Serialize)]
struct ErrorBody {
    code: &'static str,
    message: &'static str,
}

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        let (status, code, message) = match &self {
            Self::BadRequest(message) => (StatusCode::BAD_REQUEST, "bad_request", *message),
            Self::Unauthorized => (StatusCode::UNAUTHORIZED, "unauthorized", "Unauthorized"),
            Self::NotFound => (StatusCode::NOT_FOUND, "not_found", "Not found"),
            Self::PayloadTooLarge => (
                StatusCode::PAYLOAD_TOO_LARGE,
                "payload_too_large",
                "Content too large",
            ),
            Self::MethodNotAllowed => (
                StatusCode::METHOD_NOT_ALLOWED,
                "method_not_allowed",
                "Method not allowed",
            ),
            Self::Database(_)
            | Self::Template(_)
            | Self::Highlight(_)
            | Self::Worker(_)
            | Self::Internal(_) => {
                tracing::error!(error = ?self, "request failed");
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "internal_error",
                    "Internal error",
                )
            }
        };
        (
            status,
            Json(ErrorEnvelope {
                error: ErrorBody { code, message },
            }),
        )
            .into_response()
    }
}
