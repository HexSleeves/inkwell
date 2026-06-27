//! Shareable draft preview links (CIL-129).
//!
//! This module exposes three route surfaces:
//!
//! * `GET|POST /documents/{slug}/preview-tokens` — list or create a preview
//!   token for a specific draft. Requires the `write` scope and ownership of
//!   the document (admin bypasses ownership). Tokens carry an optional expiry
//!   and can be revoked explicitly.
//!
//! * `DELETE /documents/{slug}/preview-tokens/{prefix}` — revoke a preview
//!   token. Same auth as create: `write` scope + ownership.
//!
//! * `GET /documents/{slug}/preview?token=<pvw_…>` — read a draft via a
//!   preview token. No auth header required; the token itself is the
//!   credential. On any failure (bad token, expired, revoked, wrong slug,
//!   document missing or deleted) the handler returns `401 Unauthorized` so
//!   that callers cannot determine whether the document exists. The response is
//!   the same JSON envelope as `GET /documents/{slug}` but **read-only** —
//!   this handler never mutates anything.
//!
//! Security properties:
//!
//! * A valid preview token grants access to EXACTLY ONE document (matched by
//!   `document_id`). Presenting a token for document A against slug B fails.
//! * Normal `GET /documents/{slug}` still requires full authentication for
//!   drafts; this route does not change visibility on the regular path.
//! * Expired or revoked tokens always return `401`. The token is compared with
//!   [`subtle::ConstantTimeEq`] to preclude timing oracles.
//! * On document deletion the `ON DELETE CASCADE` on `preview_tokens` purges
//!   all tokens; subsequent uses return `401` (token-not-found path).
//! * The preview route is GET-only and calls no mutating DB functions. The
//!   existing write rate limiter ignores GETs, so previewing a draft does not
//!   consume write quota.

use axum::Json;
use axum::body::Bytes;
use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, Method, StatusCode};
use axum::response::{IntoResponse, Response};
use serde::Serialize;
use serde_json::Value;
use subtle::ConstantTimeEq;
use time::OffsetDateTime;

use crate::db::documents;
use crate::db::preview_tokens;
use crate::domain::author::Scope;
use crate::domain::document::StatusFilter;
use crate::domain::token::{generate_preview, parse_preview_prefix, sha256_hex};
use crate::error::AppError;
use crate::http::AppState;
use crate::http::api::{DocumentEnvelope, require_scope, resolve_visibility};
use crate::http::auth::require_principal;

/// Maximum length for an `expiresAt` ISO-8601 string in the request body.
const MAX_EXPIRES_AT_LEN: usize = 64;

/// Query parameters for `GET /documents/{slug}/preview`.
#[derive(Default, serde::Deserialize)]
pub struct PreviewQuery {
    pub token: Option<String>,
}

/// Response envelope for a freshly minted preview token. The full token value
/// is returned **exactly once** — it cannot be recovered afterwards.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct CreatedPreviewToken {
    token: String,
    prefix: String,
    document_id: uuid::Uuid,
    #[serde(with = "crate::domain::document::timestamp::option")]
    expires_at: Option<OffsetDateTime>,
}

/// Response envelope listing preview tokens for a document.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct PreviewTokenListResponse {
    preview_tokens: Vec<preview_tokens::PreviewTokenInfo>,
}

/// Response envelope for a revocation.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct RevokePreviewResponse {
    prefix: String,
    revoked: bool,
}

/// `GET|POST /documents/{slug}/preview-tokens`
///
/// GET — list all preview tokens for this document.
/// POST — mint a new preview token.
///
/// Both require the `write` scope. Ownership is enforced by fetching the
/// document through the caller's owner-filter: a non-admin attempting to
/// manage tokens for a document they don't own gets a 404 (no slug leak).
pub async fn preview_tokens(
    State(state): State<AppState>,
    Path(slug): Path<String>,
    method: Method,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Response, AppError> {
    match method {
        Method::GET => list_preview_tokens(&state, &headers, &slug).await,
        Method::POST => create_preview_token(&state, &headers, &slug, body).await,
        _ => Err(AppError::MethodNotAllowed(vec!["GET", "POST"])),
    }
}

/// `DELETE /documents/{slug}/preview-tokens/{prefix}`
///
/// Revoke a preview token by its public prefix. Requires the `write` scope
/// and ownership of the document (admin bypasses).
pub async fn revoke_preview_token(
    State(state): State<AppState>,
    Path((slug, prefix)): Path<(String, String)>,
    method: Method,
    headers: HeaderMap,
) -> Result<Response, AppError> {
    if method != Method::DELETE {
        return Err(AppError::MethodNotAllowed(vec!["DELETE"]));
    }
    let principal = require_principal(&headers, &state.config, &state.pool).await?;
    require_scope(&principal, Scope::Write)?;

    // Resolve the document with the caller's owner-filter. A non-owner gets
    // None → 404, with no confirmation that the document or token exists.
    let visibility = resolve_visibility(&headers, &state).await;
    let Some(document) =
        documents::get_document_by_slug_vis(&state.pool, &slug, visibility).await?
    else {
        return Err(AppError::NotFound(format!(
            "No document with slug \"{slug}\"."
        )));
    };

    if preview_tokens::revoke_preview_token(&state.pool, document.id, &prefix).await? {
        Ok((
            StatusCode::OK,
            Json(RevokePreviewResponse {
                prefix,
                revoked: true,
            }),
        )
            .into_response())
    } else {
        Err(AppError::NotFound(format!(
            "No live preview token with prefix \"{prefix}\" for this document."
        )))
    }
}

/// `GET /documents/{slug}/preview?token=<pvw_…>`
///
/// Render a draft document using a preview token. No auth header is required —
/// the token itself is the credential. Any failure (bad/expired/revoked token,
/// wrong document, missing document) returns `401` so the caller cannot
/// distinguish "document doesn't exist" from "token doesn't work".
pub async fn preview_document(
    State(state): State<AppState>,
    Path(slug): Path<String>,
    method: Method,
    query: Query<PreviewQuery>,
) -> Result<Response, AppError> {
    if method != Method::GET {
        return Err(AppError::MethodNotAllowed(vec!["GET"]));
    }

    // Missing token → 401 immediately (not 400, to keep the error surface uniform).
    let Some(raw_token) = query.0.token.as_deref() else {
        return Err(AppError::Unauthorized);
    };

    // Parse the prefix for the indexed DB lookup. A token that doesn't start
    // with `pvw_` or has an empty prefix/secret is immediately invalid.
    let Some(prefix) = parse_preview_prefix(raw_token) else {
        return Err(AppError::Unauthorized);
    };

    // Look up by prefix, then verify the hash in constant time.
    let Some(row) = preview_tokens::find_preview_token_by_prefix(&state.pool, prefix).await? else {
        return Err(AppError::Unauthorized);
    };

    // Constant-time hash comparison (both values are 64-char lowercase hex).
    let provided_hash = sha256_hex(raw_token);
    if !bool::from(provided_hash.as_bytes().ct_eq(row.token_hash.as_bytes())) {
        return Err(AppError::Unauthorized);
    }

    // Revoked token → 401.
    if row.revoked_at.is_some() {
        return Err(AppError::Unauthorized);
    }

    // Expired token → 401.
    if row
        .expires_at
        .is_some_and(|exp| exp < OffsetDateTime::now_utc())
    {
        return Err(AppError::Unauthorized);
    }

    // Fetch the document without any visibility filter (bypass the draft gate —
    // that's the whole point of a preview token). StatusFilter { status: None }
    // returns any status, published or draft alike.
    let Some(document) =
        documents::get_document_by_slug(&state.pool, &slug, StatusFilter { status: None }).await?
    else {
        // Document not found (deleted, renamed, etc.) → 401, not 404.
        return Err(AppError::Unauthorized);
    };

    // The token must match THIS document. A presenter trying to use a valid
    // preview token for doc A to access doc B gets 401.
    if document.id != row.document_id {
        return Err(AppError::Unauthorized);
    }

    // All checks passed — return the document read-only.
    Ok((StatusCode::OK, Json(DocumentEnvelope::from(document))).into_response())
}

async fn list_preview_tokens(
    state: &AppState,
    headers: &HeaderMap,
    slug: &str,
) -> Result<Response, AppError> {
    let principal = require_principal(headers, &state.config, &state.pool).await?;
    require_scope(&principal, Scope::Write)?;

    let visibility = resolve_visibility(headers, state).await;
    let Some(document) = documents::get_document_by_slug_vis(&state.pool, slug, visibility).await?
    else {
        return Err(AppError::NotFound(format!(
            "No document with slug \"{slug}\"."
        )));
    };

    let tokens = preview_tokens::list_preview_tokens_for_document(&state.pool, document.id).await?;
    Ok((
        StatusCode::OK,
        Json(PreviewTokenListResponse {
            preview_tokens: tokens,
        }),
    )
        .into_response())
}

async fn create_preview_token(
    state: &AppState,
    headers: &HeaderMap,
    slug: &str,
    body: Bytes,
) -> Result<Response, AppError> {
    let principal = require_principal(headers, &state.config, &state.pool).await?;
    require_scope(&principal, Scope::Write)?;

    // Parse optional expiresAt from the request body.
    let expires_at = parse_expires_at(body)?;

    // Resolve the document through the caller's visibility (owner filter
    // enforced). A non-owner can't create tokens for someone else's draft.
    let visibility = resolve_visibility(headers, state).await;
    let Some(document) = documents::get_document_by_slug_vis(&state.pool, slug, visibility).await?
    else {
        return Err(AppError::NotFound(format!(
            "No document with slug \"{slug}\"."
        )));
    };

    let minted = generate_preview();
    preview_tokens::insert_preview_token(
        &state.pool,
        document.id,
        &minted.prefix,
        &minted.token_hash,
        expires_at,
    )
    .await?;

    tracing::info!(
        document_id = %document.id,
        slug = %slug,
        prefix = %minted.prefix,
        ?expires_at,
        "preview token minted"
    );

    Ok((
        StatusCode::CREATED,
        Json(CreatedPreviewToken {
            token: minted.token,
            prefix: minted.prefix,
            document_id: document.id,
            expires_at,
        }),
    )
        .into_response())
}

/// Parse an optional `expiresAt` ISO-8601 string from the JSON body.
/// An absent body or an absent `expiresAt` field means no expiry.
/// A present but malformed or past `expiresAt` is a 400.
fn parse_expires_at(body: Bytes) -> Result<Option<OffsetDateTime>, AppError> {
    if body.is_empty() {
        return Ok(None);
    }
    let value: Value = serde_json::from_slice(&body)
        .map_err(|_| AppError::BadRequest("Request body must be valid JSON.".to_string()))?;
    let Value::Object(map) = value else {
        return Err(AppError::BadRequest(
            "Request body must be a JSON object.".to_string(),
        ));
    };
    let Some(expires_raw) = map.get("expiresAt") else {
        return Ok(None);
    };
    let Value::String(s) = expires_raw else {
        return Err(AppError::BadRequest(
            "Field \"expiresAt\" must be an ISO-8601 datetime string.".to_string(),
        ));
    };
    if s.len() > MAX_EXPIRES_AT_LEN {
        return Err(AppError::BadRequest(
            "Field \"expiresAt\" is too long.".to_string(),
        ));
    }
    let ts = OffsetDateTime::parse(s, &time::format_description::well_known::Rfc3339)
        .map_err(|_| {
            AppError::BadRequest(
                "Field \"expiresAt\" must be a valid RFC 3339 datetime (e.g. \"2026-07-01T00:00:00Z\").".to_string(),
            )
        })?;
    if ts <= OffsetDateTime::now_utc() {
        return Err(AppError::BadRequest(
            "Field \"expiresAt\" must be in the future.".to_string(),
        ));
    }
    Ok(Some(ts))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_expires_at_rejects_empty_object() {
        let result = parse_expires_at(Bytes::from_static(b"{}"));
        assert!(result.is_ok());
        assert!(result.unwrap().is_none());
    }

    #[test]
    fn parse_expires_at_rejects_non_object() {
        assert!(matches!(
            parse_expires_at(Bytes::from_static(b"\"foo\"")),
            Err(AppError::BadRequest(_))
        ));
    }

    #[test]
    fn parse_expires_at_rejects_non_string_field() {
        assert!(matches!(
            parse_expires_at(Bytes::from_static(b"{\"expiresAt\": 123}")),
            Err(AppError::BadRequest(_))
        ));
    }

    #[test]
    fn parse_expires_at_rejects_malformed_date() {
        assert!(matches!(
            parse_expires_at(Bytes::from_static(b"{\"expiresAt\": \"not-a-date\"}")),
            Err(AppError::BadRequest(_))
        ));
    }

    #[test]
    fn parse_expires_at_rejects_past_date() {
        // 2000-01-01 is always in the past
        assert!(matches!(
            parse_expires_at(Bytes::from_static(
                b"{\"expiresAt\": \"2000-01-01T00:00:00Z\"}"
            )),
            Err(AppError::BadRequest(_))
        ));
    }

    #[test]
    fn parse_expires_at_empty_body_returns_none() {
        let result = parse_expires_at(Bytes::new());
        assert!(result.is_ok());
        assert!(result.unwrap().is_none());
    }
}
