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
//! # The credential channel decides how a rejection is reported (ADR 0015)
//!
//! [`authenticate`] returns an [`AuthOutcome`], not an `Option<Principal>`,
//! because "no credential was presented" and "a credential was presented and
//! **rejected**" are materially different and get different status codes on read
//! routes:
//!
//! | Outcome | Cause | Read route | Write/admin route |
//! |---|---|---|---|
//! | [`AuthOutcome::Authenticated`] | credential accepted | `200` at the principal's visibility | scope-checked |
//! | [`AuthOutcome::Anonymous`] | no credential, **or** an expired/unknown `inkwell_session` cookie | `200`, published only | `401` |
//! | [`AuthOutcome::Rejected`] | `x-api-key` presented and rejected | **`401`** | `401` |
//! | [`AuthOutcome::Unavailable`] | token lookup hit a database error | **`503`** | `503` |
//!
//! The split is on the **channel**, not the route. `x-api-key` is only ever set
//! deliberately — by the `inkwell author` CLI, the MCP server, or a script — so a
//! rejection there is a real client misconfiguration and staying silent about it
//! makes it invisible in logs. `inkwell_session` is sent automatically by any
//! browser that once logged in, including on plain reads of public pages, so a
//! stale cookie must never break the public site for a reader who cannot see it;
//! that path folds into [`AuthOutcome::Anonymous`] and keeps failing open.
//! Anonymous page loads never present either credential and are unaffected.
//!
//! "Rejected `x-api-key`" covers a revoked token, an unknown prefix, a hash
//! mismatch, a value that is neither the static key nor token-shaped, and a
//! duplicated or non-ASCII `x-api-key` header. It does **not** cover a database
//! error during the lookup: that is [`AuthOutcome::Unavailable`] (`503`), because
//! the credential was never actually judged.
//!
//! Note that a credential which authenticates but lacks [`Scope::Read`] is *not*
//! a rejected credential; it is a successful authentication with no read
//! privilege, and [`resolve_visibility`] returns [`Visibility::Public`] for it.
//!
//! Two callers deliberately keep the old fail-open shape via
//! [`authenticate_optional`], which folds `Rejected`/`Unavailable` back into
//! `None`: the write rate limiter (a rejected key must fall back to an IP bucket,
//! not `503` the limiter) and the `/settings` HTML panel (an ambient page render).
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
use crate::db::links::Visibility;
use crate::db::{sessions, tokens};
use crate::domain::author::{BOOTSTRAP_ADMIN_ID, Principal, Scope};
use crate::domain::token;
use crate::error::AppError;
use crate::http::AppState;
use crate::http::auth_session::extract_session_cookie;

/// The four distinguishable results of resolving a request's credentials.
///
/// `Option<Principal>` cannot express "presented and rejected" separately from
/// "absent", which is exactly the distinction ADR 0015 turns into different
/// status codes on read routes. See the module docs for the mapping table.
#[derive(Debug)]
pub(crate) enum AuthOutcome {
    /// A credential was accepted; this is who the caller is.
    Authenticated(Principal),
    /// No credential was presented — or a session cookie was presented and is
    /// expired/unknown, which the cookie channel deliberately folds in here so
    /// public pages keep loading. Read routes serve published content.
    Anonymous,
    /// An `x-api-key` was presented and rejected. Read routes return `401`.
    Rejected,
    /// The credential could not be judged because the token lookup failed
    /// against Postgres. Every route returns `503`; never `401`, never `200`.
    Unavailable,
}

impl AuthOutcome {
    /// Fold back to the pre-ADR-0015 fail-open shape: any non-authenticated
    /// outcome becomes `None`. Only for callers that must not surface a
    /// credential failure as a status code — see [`authenticate_optional`].
    fn into_option(self) -> Option<Principal> {
        match self {
            Self::Authenticated(principal) => Some(principal),
            Self::Anonymous | Self::Rejected | Self::Unavailable => None,
        }
    }
}

/// Resolve the credentials behind a request. See the module docs for the
/// resolution order and for how each [`AuthOutcome`] maps to a status code.
pub(crate) async fn authenticate(
    headers: &HeaderMap,
    config: &Config,
    pool: &PgPool,
) -> AuthOutcome {
    // 1 & 2: x-api-key path takes FULL precedence. If the header is present at
    // all — even duplicated or non-ASCII — authenticate via it and NEVER fall
    // through to a session cookie. A malformed/duplicated header is a rejection,
    // not an invitation to try the cookie path.
    if headers.contains_key("x-api-key") {
        let Some(provided) = provided_key(headers) else {
            return AuthOutcome::Rejected;
        };
        // 1. Static key (no DB).
        if let Some(principal) = match_static_key(provided, config) {
            return AuthOutcome::Authenticated(principal);
        }

        // 2. Scoped token. Only reached for a well-formed `ink_…` value, so
        //    public and shared-key requests never hit the database.
        let Some(prefix) = token::parse_prefix(provided) else {
            return AuthOutcome::Rejected;
        };
        // A lookup FAILURE is not a rejection: the credential was never judged,
        // so it must surface as 503 rather than collapsing into "unknown token".
        let resolved = match tokens::find_token_by_prefix(pool, prefix).await {
            Ok(Some(resolved)) => resolved,
            Ok(None) => return AuthOutcome::Rejected,
            Err(error) => {
                tracing::error!(prefix = prefix, %error, "token lookup failed; credential unjudged");
                return AuthOutcome::Unavailable;
            }
        };
        if resolved.revoked {
            return AuthOutcome::Rejected;
        }
        // Constant-time compare of the (fixed 64-char) hex digests.
        let provided_hash = token::sha256_hex(provided);
        if !bool::from(
            provided_hash
                .as_bytes()
                .ct_eq(resolved.token_hash.as_bytes()),
        ) {
            return AuthOutcome::Rejected;
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
        return AuthOutcome::Authenticated(Principal {
            author_id: Some(resolved.author_id),
            label: resolved.author_name,
            scopes,
        });
    }

    // 3. Browser session cookie (only when INKWELL_BROWSER_LOGIN is on, and only
    //    when no x-api-key header was presented — the key path takes full
    //    precedence). An expired or unknown cookie is Anonymous, NOT Rejected:
    //    browsers attach it to every public page load (ADR 0015).
    if config.browser_login
        && let Some(principal) = resolve_session_cookie(headers, pool).await
    {
        return AuthOutcome::Authenticated(principal);
    }

    AuthOutcome::Anonymous
}

/// [`authenticate`] with the pre-ADR-0015 fail-open shape: every non-authenticated
/// outcome is `None`.
///
/// Two callers want this and nothing else should. The write rate limiter uses it
/// so a rejected key falls back to an IP bucket instead of turning the limiter
/// into a `503` source, and the `/settings` account panel uses it because it is an
/// ambient HTML render, not an API read.
pub(crate) async fn authenticate_optional(
    headers: &HeaderMap,
    config: &Config,
    pool: &PgPool,
) -> Option<Principal> {
    authenticate(headers, config, pool).await.into_option()
}

/// Require an authenticated principal. [`AuthOutcome::Anonymous`] and
/// [`AuthOutcome::Rejected`] both map to `401` — unchanged from before ADR 0015 —
/// and [`AuthOutcome::Unavailable`] maps to `503`. Used by every mutating endpoint
/// and by the admin surface (which then also checks for [`Scope::Admin`]).
pub(crate) async fn require_principal(
    headers: &HeaderMap,
    config: &Config,
    pool: &PgPool,
) -> Result<Principal, AppError> {
    match authenticate(headers, config, pool).await {
        AuthOutcome::Authenticated(principal) => Ok(principal),
        AuthOutcome::Anonymous | AuthOutcome::Rejected => Err(AppError::Unauthorized),
        AuthOutcome::Unavailable => Err(AppError::ServiceUnavailable),
    }
}

/// Resolve the request's credentials to the correct [`Visibility`] for read
/// surfaces (ADR 0009, slice 3b):
///   - No credential / no `read` scope → [`Visibility::Public`]
///   - Admin (`admin` scope or shared key) → [`Visibility::All`]
///   - Non-admin with `read` scope + known author id → [`Visibility::Owner(id)`]
///
/// This is the SINGLE place read-visibility is derived for every API surface
/// that exposes note content; callers must NOT re-derive this rule.
///
/// It is fallible because a **rejected** `x-api-key` is reported rather than
/// downgraded (ADR 0015): [`AuthOutcome::Rejected`] becomes
/// [`AppError::Unauthorized`] and [`AuthOutcome::Unavailable`] becomes
/// [`AppError::ServiceUnavailable`]. An expired or unknown session cookie is
/// [`AuthOutcome::Anonymous`], so it still resolves to [`Visibility::Public`] and
/// public pages keep loading for a browser holding a stale cookie.
pub(crate) async fn resolve_visibility(
    headers: &HeaderMap,
    state: &AppState,
) -> Result<Visibility, AppError> {
    let principal = match authenticate(headers, &state.config, &state.pool).await {
        AuthOutcome::Authenticated(principal) => principal,
        AuthOutcome::Anonymous => return Ok(Visibility::Public),
        AuthOutcome::Rejected => return Err(AppError::Unauthorized),
        AuthOutcome::Unavailable => return Err(AppError::ServiceUnavailable),
    };
    if principal.has(Scope::Admin) {
        return Ok(Visibility::All);
    }
    if principal.has(Scope::Read)
        && let Some(author_id) = principal.author_id
    {
        return Ok(Visibility::Owner(author_id));
    }
    Ok(Visibility::Public)
}

/// Require the principal to hold `scope` (admin implies all). 403 otherwise.
pub(crate) fn require_scope(principal: &Principal, scope: Scope) -> Result<(), AppError> {
    if principal.has(scope) {
        Ok(())
    } else {
        Err(AppError::Forbidden(format!(
            "This action requires the \"{scope}\" scope."
        )))
    }
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
            min_similarity: 0.0,
            webmention_send: false,
            browser_login: false,
            write_rate_limit: 0,
            trust_forwarded_headers: false,
            site_title: crate::config::DEFAULT_SITE_TITLE.to_string(),
            site_description: None,
            site_author: None,
            custom_css_url: None,
            theme: None,
            metrics_enabled: false,
            metrics_token: None,
            media_backend: crate::config::MediaBackend::Local,
            media_dir: crate::config::DEFAULT_MEDIA_DIR.to_string(),
            media_max_bytes: crate::config::DEFAULT_MEDIA_MAX_BYTES,
            webhooks_enabled: false,
            webhook_urls: Vec::new(),
            webhook_secret: None,
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
