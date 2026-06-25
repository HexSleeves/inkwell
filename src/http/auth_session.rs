//! Browser session login/logout handlers (ADR 0010).
//!
//! These handlers are **only reachable when `INKWELL_BROWSER_LOGIN=true`**;
//! the router registers the routes only when the flag is on. With the flag off
//! (the default), any request to `/auth/*` receives a 404 — the routes do not
//! exist — and the existing auth paths are completely unchanged.
//!
//! # Login flow
//! `POST /auth/login` with `{ "token": "ink_…" }` validates the scoped token
//! via the same path as `auth::authenticate` (no new credential surface), then
//! creates a session row (hashed token only) and returns a `Set-Cookie` header.
//!
//! # Logout flow
//! `POST /auth/logout` reads the session cookie, deletes the session row, and
//! clears the cookie.

use axum::body::Bytes;
use axum::extract::State;
use axum::http::{HeaderMap, Method, StatusCode, header};
use axum::response::{IntoResponse, Response};
use sha2::{Digest, Sha256};
use subtle::ConstantTimeEq;
use time::OffsetDateTime;
use uuid::Uuid;

use crate::db::{sessions, tokens};
use crate::domain::token;
use crate::error::AppError;
use crate::http::AppState;
use crate::http::extractors::{parse_json_body, require_object};

/// The cookie name used for browser sessions.
pub(crate) const SESSION_COOKIE_NAME: &str = "inkwell_session";

/// Session lifetime: 7 days in seconds.
const SESSION_TTL_SECS: i64 = 7 * 24 * 60 * 60;

/// `POST /auth/login` — exchange a scoped token for an httpOnly session cookie.
///
/// Validates the token via the existing `find_token_by_prefix` + constant-time
/// hash compare path, creates a session row (storing only the SHA-256 hash of a
/// freshly generated session token), and sets:
/// ```text
/// Set-Cookie: inkwell_session=<token>; HttpOnly; Secure; SameSite=Strict; Path=/; Max-Age=604800
/// ```
///
/// A revoked, invalid, or missing token → 401. Any method other than POST → 405.
pub async fn login(
    State(state): State<AppState>,
    method: Method,
    body: Bytes,
) -> Result<Response, AppError> {
    if method != Method::POST {
        return Err(AppError::MethodNotAllowed(vec!["POST"]));
    }

    let obj = require_object(parse_json_body(body)?)?;
    let raw_token = obj
        .get("token")
        .and_then(|v| v.as_str())
        .ok_or_else(|| AppError::BadRequest("body must contain a \"token\" string field".into()))?;

    // Validate the scoped token — identical to the path in auth::authenticate.
    let prefix = token::parse_prefix(raw_token).ok_or(AppError::Unauthorized)?;
    let resolved = tokens::find_token_by_prefix(&state.pool, prefix)
        .await?
        .ok_or(AppError::Unauthorized)?;
    if resolved.revoked {
        return Err(AppError::Unauthorized);
    }
    let provided_hash = token::sha256_hex(raw_token);
    if !bool::from(
        provided_hash
            .as_bytes()
            .ct_eq(resolved.token_hash.as_bytes()),
    ) {
        return Err(AppError::Unauthorized);
    }

    // Mint a session token: 64 hex chars from two independent v4 UUIDs
    // (≈ 244 bits of entropy). Only its SHA-256 hash is stored.
    let raw_session_token = format!("{}{}", Uuid::new_v4().simple(), Uuid::new_v4().simple());
    let session_hash = sha256_hex(raw_session_token.as_bytes());
    let expires_at = OffsetDateTime::now_utc() + time::Duration::seconds(SESSION_TTL_SECS);

    // The session inherits EXACTLY the token's scopes — a read-only token must
    // never become a write/publish session (no privilege escalation).
    sessions::create_session(
        &state.pool,
        resolved.author_id,
        &session_hash,
        &resolved.scopes,
        expires_at,
    )
    .await?;

    let cookie = format!(
        "{SESSION_COOKIE_NAME}={raw_session_token}; HttpOnly; Secure; SameSite=Strict; Path=/; Max-Age={SESSION_TTL_SECS}"
    );
    Ok((StatusCode::OK, [(header::SET_COOKIE, cookie)]).into_response())
}

/// `POST /auth/logout` — delete the session and clear the cookie.
///
/// Idempotent: if no session cookie is present or it does not match a row, the
/// cookie is still cleared and a 200 is returned.
pub async fn logout(
    State(state): State<AppState>,
    method: Method,
    headers: HeaderMap,
) -> Result<Response, AppError> {
    if method != Method::POST {
        return Err(AppError::MethodNotAllowed(vec!["POST"]));
    }

    if let Some(session_token) = extract_session_cookie(&headers) {
        let hash = sha256_hex(session_token.as_bytes());
        // Propagate a delete failure as a 500: clearing the client's cookie while
        // leaving the server-side session row alive would let any other copy of
        // the token keep authenticating. The client must know logout didn't take.
        sessions::delete_session_by_hash(&state.pool, &hash).await?;
    }

    let clear =
        format!("{SESSION_COOKIE_NAME}=; HttpOnly; Secure; SameSite=Strict; Path=/; Max-Age=0");
    Ok((StatusCode::OK, [(header::SET_COOKIE, clear)]).into_response())
}

/// Extract the `inkwell_session` cookie value from the `Cookie` request header.
///
/// Handles multi-cookie headers: `Cookie: a=1; inkwell_session=<token>; b=2`.
/// Returns `None` when the header is absent or the cookie is not present.
pub(crate) fn extract_session_cookie(headers: &HeaderMap) -> Option<String> {
    let cookie_header = headers.get(header::COOKIE)?.to_str().ok()?;
    for pair in cookie_header.split(';') {
        let pair = pair.trim();
        if let Some(value) = pair.strip_prefix(SESSION_COOKIE_NAME)
            && let Some(value) = value.strip_prefix('=')
        {
            let value = value.trim();
            if !value.is_empty() {
                return Some(value.to_string());
            }
        }
    }
    None
}

/// SHA-256 of `input` bytes, lowercase hex.
fn sha256_hex(input: &[u8]) -> String {
    use std::fmt::Write;
    let digest = Sha256::digest(input);
    let mut out = String::with_capacity(64);
    for byte in digest {
        let _ = write!(out, "{byte:02x}");
    }
    out
}
