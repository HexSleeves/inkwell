//! Media upload, serving, and deletion (`POST /media`, `GET|DELETE /media/{id}`).
//!
//! **Upload** (`POST /media`):
//! - Auth-gated: requires a valid principal with the `write` scope.
//! - Body: raw image bytes; `Content-Type` header names the MIME type.
//! - Allowlist: `image/png`, `image/jpeg`, `image/gif`, `image/webp`.
//!   SVG is intentionally excluded — it can carry embedded script (`<script>`,
//!   event-handler attributes) and browsers execute it as active content when
//!   served with `Content-Type: image/svg+xml`, making it a stored-XSS vector.
//! - The declared type is **verified against the sniffed magic bytes**; a
//!   mismatch (e.g. HTML declared as `image/png`) is refused.
//! - Size cap: `INKWELL_MEDIA_MAX_BYTES` (default 5 MiB) — 413 if exceeded.
//! - Bytes go to the configured [`MediaStore`](crate::media::MediaStore) under a
//!   content-addressed key; only metadata lands in Postgres (ADR 0013).
//! - Re-uploading identical bytes returns the existing row (200 instead of 201)
//!   rather than duplicating a blob.
//!
//! **Serve** (`GET /media/{id}`): public (no auth). Returns the stored bytes with
//! the original `Content-Type`, explicit `Content-Length`, a strong `ETag` (the
//! content SHA-256), a long immutable `Cache-Control`, and
//! `X-Content-Type-Options: nosniff` (set globally by the security-headers
//! middleware). Honours `If-None-Match` with a 304.
//!
//! **Delete** (`DELETE /media/{id}`): requires `write` scope and ownership (or
//! `admin`). Removes the row, then the blob only when no other row still points
//! at the same content-addressed key.

use axum::Json;
use axum::body::Bytes;
use axum::extract::{Extension, Path, State};
use axum::http::{HeaderMap, HeaderValue, Method, StatusCode, header};
use axum::response::{Html, IntoResponse, Response};
use serde::Serialize;
use uuid::Uuid;

use crate::db::media::{self, MediaRow, NewMedia};
use crate::domain::author::{Principal, Scope};
use crate::error::AppError;
use crate::http::AppState;
use crate::http::auth::require_principal;
use crate::http::auth_session::extract_session_cookie;
use crate::http::security_headers::CspNonce;
use crate::media::{ALLOWED_MIME_TYPES, checksum_hex, sniff::sniff_image, storage_key_for};
use crate::views::layout::SiteMeta;
use crate::views::media::render_media_upload_page;

/// Cache directive for served media. Blobs are content-addressed, so a given
/// `/media/{id}` body never changes: a year plus `immutable` is safe.
const MEDIA_CACHE_CONTROL: &str = "public, max-age=31536000, immutable";

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
    headers: HeaderMap,
) -> Result<Response, AppError> {
    serve(state, id, &headers).await
}

/// `DELETE /media/{id}` — delete a media row (and its blob when unreferenced).
pub async fn media_delete(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    headers: HeaderMap,
) -> Result<Response, AppError> {
    delete(state, id, &headers).await
}

/// `GET /media/new` — the browser upload page. Flag-gated in the router.
pub async fn media_new_page(
    State(state): State<AppState>,
    Extension(csp_nonce): Extension<CspNonce>,
    headers: HeaderMap,
) -> Response {
    let site = SiteMeta::from_config(&state.config);
    let logged_in = extract_session_cookie(&headers).is_some();
    Html(render_media_upload_page(
        &site,
        Some(csp_nonce.as_str()),
        logged_in,
    ))
    .into_response()
}

async fn upload(state: AppState, headers: HeaderMap, body: Bytes) -> Result<Response, AppError> {
    let principal = require_principal(&headers, &state.config, &state.pool).await?;
    require_write(&principal)?;

    if body.len() > state.config.media_max_bytes {
        return Err(AppError::PayloadTooLarge);
    }

    // Extract and normalise the declared MIME type (strip parameters like
    // "; charset=…").
    let raw_ct = headers
        .get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_lowercase();
    let declared = raw_ct.split(';').next().unwrap_or("").trim().to_string();

    if !ALLOWED_MIME_TYPES.contains(&declared.as_str()) {
        return Err(AppError::BadRequest(format!(
            "Unsupported media type \"{declared}\". Allowed: {}.",
            ALLOWED_MIME_TYPES.join(", ")
        )));
    }

    // Never trust the declared type: sniff the bytes and require agreement, so
    // "HTML declared as image/png" cannot be stored and later served.
    let Some(sniffed) = sniff_image(&body) else {
        return Err(AppError::BadRequest(format!(
            "Uploaded bytes are not a recognised {declared} image. Allowed: {}.",
            ALLOWED_MIME_TYPES.join(", ")
        )));
    };
    if sniffed != declared {
        return Err(AppError::BadRequest(format!(
            "Declared content type \"{declared}\" does not match the uploaded bytes (\"{sniffed}\")."
        )));
    }

    // Stamp owner from the resolved principal; nil fallback fails closed
    // (matches no author FK) rather than silently dropping the owner.
    let owner_id = principal.author_id.unwrap_or_else(Uuid::nil);
    let checksum = checksum_hex(&body);

    // Content addressing makes uploads idempotent: identical bytes from the same
    // owner return the existing row instead of a second row over one blob.
    if let Some(existing) = media::find_by_owner_and_checksum(&state.pool, owner_id, &checksum)
        .await?
        .filter(|row| row.content_type == sniffed)
    {
        return Ok(upload_response(StatusCode::OK, existing.id));
    }

    let storage_key = storage_key_for(&checksum, sniffed).ok_or_else(|| {
        // Unreachable: `sniffed` came from the allowlist and `checksum` is hex.
        AppError::Internal(anyhow::anyhow!(
            "could not derive a storage key for content type \"{sniffed}\""
        ))
    })?;

    // Write the blob BEFORE the row, so a committed row always has bytes behind
    // it. The reverse order could serve a 500 for a row whose blob never landed.
    state
        .media_store
        .put(&storage_key, &body)
        .await
        .map_err(|error| AppError::Internal(anyhow::anyhow!(error)))?;

    let id = media::insert_media(
        &state.pool,
        NewMedia {
            filename: None,
            content_type: sniffed,
            byte_size: body.len() as i64,
            checksum_sha256: &checksum,
            storage_key: &storage_key,
            storage_backend: state.media_store.backend(),
            owner_id,
        },
    )
    .await?;

    Ok(upload_response(StatusCode::CREATED, id))
}

fn upload_response(status: StatusCode, id: Uuid) -> Response {
    (
        status,
        Json(UploadResponse {
            id,
            url: format!("/media/{id}"),
        }),
    )
        .into_response()
}

async fn serve(state: AppState, id: Uuid, headers: &HeaderMap) -> Result<Response, AppError> {
    let Some(row) = media::get_media(&state.pool, id).await? else {
        return Err(AppError::NotFound(format!("No media with id \"{id}\".")));
    };

    // Strong ETag: the content hash. Content-addressed bytes never change under
    // an id, so a conditional request is answered without reading the blob.
    let etag = row
        .checksum_sha256
        .as_deref()
        .map(|hex| format!("\"{hex}\""));
    if let Some(etag) = etag.as_deref()
        && request_matches_etag(headers, etag)
    {
        return Ok(not_modified(etag));
    }

    let bytes = load_bytes(&state, &row).await?;

    let mut response = axum::http::Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, &row.content_type)
        .header(header::CONTENT_LENGTH, bytes.len())
        .header(header::CACHE_CONTROL, MEDIA_CACHE_CONTROL)
        .body(axum::body::Body::from(bytes))
        .map_err(|e| AppError::Internal(anyhow::anyhow!(e)))?;
    if let Some(etag) = etag
        && let Ok(value) = HeaderValue::from_str(&etag)
    {
        response.headers_mut().insert(header::ETAG, value);
    }
    Ok(response)
}

/// Read a row's bytes: from the configured store when the row is storage-backed,
/// else from the legacy inline `data` column (rows written before migration 0025).
async fn load_bytes(state: &AppState, row: &MediaRow) -> Result<Vec<u8>, AppError> {
    if let Some(key) = row.storage_key.as_deref() {
        let bytes = state
            .media_store
            .get(key)
            .await
            .map_err(|error| AppError::Internal(anyhow::anyhow!(error)))?;
        // A row without its blob means the backend lost data (wrong backend
        // configured, volume not mounted, blob removed out of band). Report a 404
        // with the id so an operator can trace it, and log loudly.
        return bytes.ok_or_else(|| {
            tracing::error!(
                media_id = %row.id,
                storage_key = key,
                backend = state.media_store.backend(),
                "media blob missing from the configured storage backend"
            );
            AppError::NotFound(format!("Media \"{}\" has no stored bytes.", row.id))
        });
    }
    row.data.clone().ok_or_else(|| {
        AppError::Internal(anyhow::anyhow!(
            "media row {} has neither a storage key nor inline data",
            row.id
        ))
    })
}

async fn delete(state: AppState, id: Uuid, headers: &HeaderMap) -> Result<Response, AppError> {
    let principal = require_principal(headers, &state.config, &state.pool).await?;
    require_write(&principal)?;

    let Some(row) = media::get_media(&state.pool, id).await? else {
        return Err(AppError::NotFound(format!("No media with id \"{id}\".")));
    };

    // Owner-scoped: a `write` token may only delete its own uploads. `admin`
    // (which the shared key holds) may delete any.
    let is_owner = principal.author_id == Some(row.owner_id);
    if !is_owner && !principal.has(Scope::Admin) {
        return Err(AppError::Forbidden(
            "You can only delete media you uploaded.".to_string(),
        ));
    }

    if !media::delete_media(&state.pool, id).await? {
        // Lost a race with a concurrent delete; the end state is what was asked.
        return Ok(StatusCode::NO_CONTENT.into_response());
    }

    // Orphan handling: blobs are content-addressed and therefore shared, so the
    // bytes go only once nothing references them (see ADR 0013). A failure here
    // leaves an orphaned blob, never a broken row, so it is logged not surfaced.
    if let Some(key) = row.storage_key.as_deref() {
        match media::count_rows_for_storage_key(&state.pool, key).await {
            Ok(0) => {
                if let Err(error) = state.media_store.delete(key).await {
                    tracing::error!(
                        media_id = %id,
                        storage_key = key,
                        error = %error,
                        "failed to delete media blob; row is gone, blob orphaned"
                    );
                }
            }
            Ok(remaining) => tracing::debug!(
                storage_key = key,
                remaining,
                "media blob still referenced; keeping bytes"
            ),
            Err(error) => tracing::error!(
                storage_key = key,
                error = %error,
                "could not count media references; keeping blob to avoid data loss"
            ),
        }
    }

    Ok(StatusCode::NO_CONTENT.into_response())
}

fn not_modified(etag: &str) -> Response {
    let mut response = StatusCode::NOT_MODIFIED.into_response();
    let headers = response.headers_mut();
    if let Ok(value) = HeaderValue::from_str(etag) {
        headers.insert(header::ETAG, value);
    }
    headers.insert(
        header::CACHE_CONTROL,
        HeaderValue::from_static(MEDIA_CACHE_CONTROL),
    );
    response
}

/// Whether `If-None-Match` covers `etag` (also accepting the weak form and `*`),
/// mirroring the HTML/XML cache path in [`crate::http::cache`].
fn request_matches_etag(headers: &HeaderMap, etag: &str) -> bool {
    let weak = format!("W/{etag}");
    headers
        .get(header::IF_NONE_MATCH)
        .and_then(|value| value.to_str().ok())
        .is_some_and(|value| {
            value
                .split(',')
                .map(str::trim)
                .any(|candidate| candidate == "*" || candidate == etag || candidate == weak)
        })
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

#[cfg(test)]
mod tests {
    use super::*;

    fn headers_with_if_none_match(value: &str) -> HeaderMap {
        let mut headers = HeaderMap::new();
        headers.insert(
            header::IF_NONE_MATCH,
            HeaderValue::from_str(value).expect("valid header value"),
        );
        headers
    }

    #[test]
    fn if_none_match_accepts_strong_weak_wildcard_and_lists() {
        let etag = "\"abc\"";
        assert!(request_matches_etag(
            &headers_with_if_none_match("\"abc\""),
            etag
        ));
        assert!(request_matches_etag(
            &headers_with_if_none_match("W/\"abc\""),
            etag
        ));
        assert!(request_matches_etag(&headers_with_if_none_match("*"), etag));
        assert!(request_matches_etag(
            &headers_with_if_none_match("\"other\", \"abc\""),
            etag
        ));
    }

    #[test]
    fn if_none_match_ignores_other_etags_and_absent_header() {
        let etag = "\"abc\"";
        assert!(!request_matches_etag(
            &headers_with_if_none_match("\"nope\""),
            etag
        ));
        assert!(!request_matches_etag(&HeaderMap::new(), etag));
    }
}
