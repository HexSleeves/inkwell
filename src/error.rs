use axum::Json;
use axum::http::{HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use serde::Serialize;

use crate::db::documents::DbError;

#[derive(Debug, thiserror::Error)]
pub enum AppError {
    #[error("{0}")]
    BadRequest(String),
    #[error("Missing or invalid API key.")]
    Unauthorized,
    #[error("{0}")]
    Forbidden(String),
    #[error("Request body too large.")]
    PayloadTooLarge,
    #[error("{0}")]
    NotFound(String),
    #[error("{0}")]
    Conflict(String),
    #[error("Method not allowed.")]
    MethodNotAllowed(Vec<&'static str>),
    #[error("Rate limit exceeded.")]
    TooManyRequests { retry_after_secs: u64 },
    #[error(transparent)]
    Database(#[from] sqlx::Error),
    #[error(transparent)]
    Db(#[from] DbError),
    #[error(transparent)]
    Internal(#[from] anyhow::Error),
}

#[derive(Serialize)]
struct ErrorBody<'a> {
    error: ErrorPayload<'a>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ErrorPayload<'a> {
    message: &'a str,
    /// Correlation id of the request, so a user-reported error is traceable to
    /// its logs. Present whenever the error is produced inside a request scope
    /// (always, in practice); `None` only for errors built outside one.
    #[serde(skip_serializing_if = "Option::is_none")]
    request_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    slug: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    allow: Option<String>,
}

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        match self {
            Self::BadRequest(message) => json_error(StatusCode::BAD_REQUEST, &message, None, None),
            Self::Unauthorized => json_error(
                StatusCode::UNAUTHORIZED,
                "Missing or invalid API key.",
                None,
                None,
            ),
            Self::Forbidden(message) => json_error(StatusCode::FORBIDDEN, &message, None, None),
            Self::PayloadTooLarge => json_error(
                StatusCode::PAYLOAD_TOO_LARGE,
                "Request body too large.",
                None,
                None,
            ),
            Self::NotFound(message) => json_error(StatusCode::NOT_FOUND, &message, None, None),
            Self::Conflict(message) => json_error(StatusCode::CONFLICT, &message, None, None),
            Self::MethodNotAllowed(allow) => json_error(
                StatusCode::METHOD_NOT_ALLOWED,
                "Method not allowed.",
                None,
                Some(allow.join(", ")),
            ),
            Self::TooManyRequests { retry_after_secs } => {
                let mut response = json_error(
                    StatusCode::TOO_MANY_REQUESTS,
                    "Rate limit exceeded. Slow down and retry later.",
                    None,
                    None,
                );
                if let Ok(value) = HeaderValue::from_str(&retry_after_secs.to_string()) {
                    response.headers_mut().insert(header::RETRY_AFTER, value);
                }
                response
            }
            Self::Db(DbError::DuplicateSlug { slug }) => json_error(
                StatusCode::CONFLICT,
                &format!("A document with slug \"{slug}\" already exists."),
                Some(&slug),
                None,
            ),
            Self::Db(DbError::Sqlx(error)) | Self::Database(error) => {
                tracing::error!(error = %error, "database error");
                json_error(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "Internal server error.",
                    None,
                    None,
                )
            }
            Self::Internal(error) => {
                tracing::error!(error = %error, "internal error");
                json_error(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "Internal server error.",
                    None,
                    None,
                )
            }
        }
    }
}

fn json_error(
    status: StatusCode,
    message: &str,
    slug: Option<&str>,
    allow: Option<String>,
) -> Response {
    (
        status,
        Json(ErrorBody {
            error: ErrorPayload {
                message,
                request_id: crate::http::request_id::current(),
                slug,
                allow,
            },
        }),
    )
        .into_response()
}
