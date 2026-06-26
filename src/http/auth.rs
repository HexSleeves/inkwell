//! Request authentication (ADR 0009, plan 023, slice 2; ADR 0010, browser sessions).
//!
//! [`authenticate`] resolves the [`Principal`] behind a request. Three credential
//! families are accepted, in order:
//!
//! 1. **Static key** — the shared `INKWELL_API_KEY`, mapping to the all-powerful
//!    bootstrap-admin principal (audit label `"shared-key"`). Compared in
//!    constant time. (The separate `INKWELL_MCP_KEY` was retired in slice 4; the
//!    MCP server now authenticates with a scoped token via `INKWELL_API_KEY`.)
//! 2. **Scoped tokens** — `ink_<prefix>_<secret>` (see [`crate::domain::token`]).
//!    Looked up by the public `prefix`, then a constant-time hash compare; a
//!    revoked token never authenticates. Resolves to the owning author's
//!    principal with the token's scopes.
//! 3. **Browser session cookie** (`inkwell_session`) — **only when
//!    `INKWELL_BROWSER_LOGIN=true`** and no `x-api-key` header is present.
//!    Resolves to the owning author's principal carrying EXACTLY the scopes the
//!    session inherited from its minting token (capped to read/write/publish at
//!    login — never admin). A read-only token's session stays read-only. When
//!    the flag is off, the `Cookie` header is never consulted and the existing
//!    auth paths are byte-for-byte unchanged.
//!
//! The token path is the **only** branch that touches the database, and it runs
//! only when the presented key both fails the static compare and looks like a
//! token — so anonymous and shared-key requests never pay a token lookup.
//!
//! Slice 2 resolves and audits principals but does not yet enforce scope or
//! ownership on document routes (slice 3). The admin token-management surface is
//! the exception: it is admin-gated from creation (see [`crate::http::admin`]).

use std::collections::HashSet;

use axum::http::HeaderMap;
use sha2::{Digest, Sha256};
use sqlx::PgPool;
use subtle::ConstantTimeEq;
use time::OffsetDateTime;

use crate::config::Config;
use crate::db::{sessions, tokens};
use crate::domain::author::{BOOTSTRAP_ADMIN_ID, Principal, Scope};
use crate::domain::token;
use crate::error::AppError;
use crate::http::auth_session::extract_session_cookie;

/// Resolve the [`Principal`] behind a request, or `None` for an unauthenticated
/// (public) caller. See the module docs for the resolution order.
pub async fn authenticate(
    headers: &HeaderMap,
    config: &Config,
    pool: &PgPool,
) -> Option<Principal> {
    // 1 & 2: x-api-key path takes FULL precedence. If the header is present at
    // all — even duplicated or non-ASCII — authenticate via it and NEVER fall
    // through to a session cookie. `provided_key` returns None for a
    // malformed/duplicated header; the `?` below then rejects (returns None)
    // rather than leaking authentication to the cookie path.
    if headers.contains_key("x-api-key") {
        let provided = provided_key(headers)?;
        // 1. Static key (no DB).
        if let Some(principal) = match_static_key(provided, config) {
            return Some(principal);
        }

        // 2. Scoped token. Only reached for a well-formed `ink_…` value, so
        //    public and shared-key requests never hit the database.
        let prefix = token::parse_prefix(provided)?;
        let resolved = tokens::find_token_by_prefix(pool, prefix).await.ok()??;
        if resolved.revoked {
            return None;
        }
        // Constant-time compare of the (fixed 64-char) hex digests.
        let provided_hash = token::sha256_hex(provided);
        if !bool::from(
            provided_hash
                .as_bytes()
                .ct_eq(resolved.token_hash.as_bytes()),
        ) {
            return None;
        }
        // Best-effort usage stamp: a stale `last_used_at` never affects auth.
        if let Err(error) = tokens::touch_last_used(pool, prefix).await {
            tracing::warn!(prefix = prefix, %error, "touch_last_used failed; token still authenticated");
        }
        let scopes = resolved
            .scopes
            .iter()
            .filter_map(|s| Scope::parse(s))
            .collect();
        return Some(Principal {
            author_id: Some(resolved.author_id),
            label: resolved.author_name,
            scopes,
        });
    }

    // 3. Browser session cookie (only when INKWELL_BROWSER_LOGIN is on, and only
    //    when no x-api-key header was presented — the key path takes full precedence).
    if config.browser_login
        && let Some(principal) = resolve_session_cookie(headers, pool).await
    {
        return Some(principal);
    }

    None
}

/// Require an authenticated principal, mapping the anonymous case to `401`. Used
/// by every mutating endpoint and by the admin surface (which then also checks
/// for [`Scope::Admin`]).
pub async fn require_principal(
    headers: &HeaderMap,
    config: &Config,
    pool: &PgPool,
) -> Result<Principal, AppError> {
    authenticate(headers, config, pool)
        .await
        .ok_or(AppError::Unauthorized)
}

/// Extract the single ASCII `x-api-key` header value. Returns `None` when the
/// header is missing, duplicated, or non-ASCII — preserving the rejection rules
/// the pre-token implementation enforced.
fn provided_key(headers: &HeaderMap) -> Option<&str> {
    let mut values = headers.get_all("x-api-key").iter();
    let value = values.next()?;
    if values.next().is_some() {
        // More than one `x-api-key` header: reject rather than guess.
        return None;
    }
    value.to_str().ok()
}

/// Resolve a browser session cookie to a [`Principal`].
///
/// Called only when `INKWELL_BROWSER_LOGIN` is on and no `x-api-key` was
/// presented. Extracts the `inkwell_session` cookie, hashes it, looks up the
/// session row, checks expiry, and constructs a `Principal` carrying EXACTLY the
/// scopes the session inherited from its originating scoped token — never more
/// (a `read`-only token's session stays read-only). Admin operations still
/// require the shared key or an admin-scoped token.
async fn resolve_session_cookie(headers: &HeaderMap, pool: &PgPool) -> Option<Principal> {
    let raw = extract_session_cookie(headers)?;
    // SHA-256 hash of the raw session token for constant-time DB lookup.
    use std::fmt::Write as _;
    let digest = Sha256::digest(raw.as_bytes());
    let mut hash = String::with_capacity(64);
    for byte in digest {
        let _ = write!(hash, "{byte:02x}");
    }
    let row = sessions::find_session_by_hash(pool, &hash).await.ok()??;
    if row.expires_at < OffsetDateTime::now_utc() {
        // Expired session: treat as unauthenticated (don't delete — let a sweep do it).
        return None;
    }
    let scopes: HashSet<Scope> = row.scopes.iter().filter_map(|s| Scope::parse(s)).collect();
    Some(Principal {
        author_id: Some(row.author_id),
        label: row.author_name,
        scopes,
    })
}

/// Match a presented key against the configured shared `INKWELL_API_KEY`,
/// constant-time. A match yields the all-powerful bootstrap-admin principal; an
/// unset or empty configured key never matches.
fn match_static_key(provided: &str, config: &Config) -> Option<Principal> {
    let candidate = config.api_key.as_deref().filter(|c| !c.is_empty())?;
    let provided_hash = Sha256::digest(provided.as_bytes());
    let expected = Sha256::digest(candidate.as_bytes());
    bool::from(provided_hash.ct_eq(&expected))
        .then(|| Principal::admin(BOOTSTRAP_ADMIN_ID, "shared-key"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::HeaderValue;

    fn config_with(api_key: Option<&str>) -> Config {
        Config {
            database_url: "postgres://localhost/db".to_string(),
            host: "127.0.0.1".to_string(),
            port: 3000,
            api_key: api_key.map(str::to_string),
            site_url: None,
            voyage_api_key: None,
            anthropic_api_key: None,
            llm_model: crate::config::DEFAULT_LLM_MODEL.to_string(),
            webmention_send: false,
            browser_login: false,
            write_rate_limit: 0,
            trust_forwarded_headers: false,
        }
    }

    fn headers_with_key(key: &str) -> HeaderMap {
        let mut headers = HeaderMap::new();
        headers.insert("x-api-key", HeaderValue::from_str(key).unwrap());
        headers
    }

    #[test]
    fn static_match_accepts_the_shared_key_as_admin() {
        let config = config_with(Some("author-key"));

        let admin = match_static_key("author-key", &config).expect("api key authenticates");
        assert_eq!(admin.label, "shared-key");
        assert_eq!(admin.author_id, Some(BOOTSTRAP_ADMIN_ID));
        assert!(admin.has(Scope::Admin));
    }

    #[test]
    fn static_match_rejects_unknown_or_empty_keys() {
        let config = config_with(Some("author-key"));
        assert!(match_static_key("wrong", &config).is_none());

        let blank = config_with(Some(""));
        assert!(match_static_key("", &blank).is_none());

        let none = config_with(None);
        assert!(match_static_key("anything", &none).is_none());
    }

    #[test]
    fn static_match_ignores_token_shaped_keys() {
        // A token is not a static key; it must go through the DB path instead.
        let config = config_with(Some("author-key"));
        assert!(match_static_key("ink_abc_def", &config).is_none());
    }

    #[test]
    fn provided_key_requires_exactly_one_ascii_header() {
        assert_eq!(provided_key(&headers_with_key("k")), Some("k"));
        assert_eq!(provided_key(&HeaderMap::new()), None);

        let mut dup = HeaderMap::new();
        dup.append("x-api-key", HeaderValue::from_static("k"));
        dup.append("x-api-key", HeaderValue::from_static("k"));
        assert_eq!(provided_key(&dup), None);
    }
}
