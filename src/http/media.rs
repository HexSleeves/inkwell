//! Media upload and serving (`POST /media`, `GET /media/{id}`).
//!
//! **Upload** (`POST /media`):
//! - Auth-gated: requires a valid principal with the `write` scope.
//! - Body: raw image bytes; `Content-Type` header names the MIME type.
//! - Allowlist: `image/png`, `image/jpeg`, `image/gif`, `image/webp`.
//!   SVG is intentionally excluded for v1 — it can carry embedded script
//!   (`<script>`, event-handler attributes) and browsers execute it as HTML
//!   when served with `Content-Type: image/svg+xml`, making it a stored-XSS
//!   vector. The simplest safe choice is to exclude it entirely.
//! - Size cap: [`MAX_MEDIA_BYTES`] (5 MiB) — 413 if exceeded.
//! - Returns 201 `{ "id": "…", "url": "/media/<id>" }` on success.
//!
//! **Serve** (`GET /media/{id}`):
//! - Public (no auth). Returns stored bytes with the original `Content-Type`,
//!   a long immutable `Cache-Control`, and `X-Content-Type-Options: nosniff`
//!   (also set globally by the security-headers middleware).

use axum::Json;
use axum::body::Bytes;
use axum::extract::{Path, State};
use axum::http::{HeaderMap, Method, StatusCode};
use axum::response::{IntoResponse, Response};
use serde::Serialize;
use uuid::Uuid;

use crate::db::media;
use crate::domain::author::{Principal, Scope};
use crate::error::AppError;
use crate::http::AppState;
use crate::http::auth::require_principal;

/// Maximum upload size: 5 MiB.
pub const MAX_MEDIA_BYTES: usize = 5 * 1024 * 1024;

/// Allowed image MIME types. SVG excluded (see module docs).
const ALLOWED_CONTENT_TYPES: &[&str] = &["image/png", "image/jpeg", "image/gif", "image/webp"];

#[derive(Serialize)]
struct UploadResponse {
    id: Uuid,
    url: String,
}

/// `POST /media` — upload raw image bytes.
pub async fn media_upload(
    State(state): State<AppState>,
    method: Method,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Response, AppError> {
    match method {
        Method::POST => upload(state, headers, body).await,
        _ => Err(AppError::MethodNotAllowed(vec!["POST"])),
    }
}

/// `GET /media/{id}` — serve stored bytes. Routed via `get(...)`, so axum also
/// answers `HEAD` automatically (same headers, no body) for cache/probe clients.
pub async fn media_serve(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<Response, AppError> {
    serve(state, id).await
}

async fn upload(state: AppState, headers: HeaderMap, body: Bytes) -> Result<Response, AppError> {
    let principal = require_principal(&headers, &state.config, &state.pool).await?;
    require_write(&principal)?;

    if body.len() > MAX_MEDIA_BYTES {
        return Err(AppError::PayloadTooLarge);
    }

    // Extract and normalise the MIME type (strip parameters like "; charset=…").
    let raw_ct = headers
        .get(axum::http::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_lowercase();
    let mime_type = raw_ct.split(';').next().unwrap_or("").trim().to_string();

    if !ALLOWED_CONTENT_TYPES.contains(&mime_type.as_str()) {
        return Err(AppError::BadRequest(format!(
            "Unsupported media type \"{mime_type}\". Allowed: {}.",
            ALLOWED_CONTENT_TYPES.join(", ")
        )));
    }

    // Stamp owner from the resolved principal; nil fallback fails closed
    // (matches no author FK) rather than silently dropping the owner.
    let owner_id = principal.author_id.unwrap_or_else(Uuid::nil);

    let id = media::insert_media(&state.pool, None, &mime_type, &body, owner_id).await?;

    Ok((
        StatusCode::CREATED,
        Json(UploadResponse {
            id,
            url: format!("/media/{id}"),
        }),
    )
        .into_response())
}

async fn serve(state: AppState, id: Uuid) -> Result<Response, AppError> {
    let Some(row) = media::get_media(&state.pool, id).await? else {
        return Err(AppError::NotFound(format!("No media with id \"{id}\".")));
    };

    // Build the response manually so we control Content-Type precisely.
    // `X-Content-Type-Options: nosniff` is set by the global security-headers
    // middleware and does not need to be added here.
    axum::http::Response::builder()
        .status(StatusCode::OK)
        .header(axum::http::header::CONTENT_TYPE, &row.content_type)
        .header(
            axum::http::header::CACHE_CONTROL,
            "public, max-age=31536000, immutable",
        )
        .body(axum::body::Body::from(row.data))
        .map_err(|e| AppError::Internal(anyhow::anyhow!(e)))
}

/// Require the `write` scope; 403 otherwise.
fn require_write(principal: &Principal) -> Result<(), AppError> {
    if principal.has(Scope::Write) {
        Ok(())
    } else {
        Err(AppError::Forbidden(
            "This action requires the \"write\" scope.".to_string(),
        ))
    }
}
