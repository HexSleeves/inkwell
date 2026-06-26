//! Pragmatic write rate limiting (CIL-128).
//!
//! A single global GCRA limiter (the [`governor`] crate) throttles *mutation*
//! traffic — every `POST`/`PUT`/`PATCH`/`DELETE` plus the expensive `/ask`
//! endpoint (which answers via Voyage + Anthropic and is `GET|POST`). Read
//! paths and the public HTML site are never consulted by the limiter, so the
//! garden stays fast for visitors.
//!
//! Requests are bucketed by the calling **principal** when one is present and
//! by **client IP** otherwise, in the same precedence the auth layer resolves a
//! principal:
//! - an `x-api-key` request buckets by the SHA-256 of that credential (never the
//!   raw secret) so each token / the shared key gets its own quota without a
//!   database round-trip;
//! - when browser login is on (`INKWELL_BROWSER_LOGIN=true`), a cookie-
//!   authenticated request buckets by the SHA-256 of its `inkwell_session`
//!   token, so two logged-in users behind one NAT/IP don't throttle each other;
//! - an anonymous request (e.g. public `/ask` or inbound `/webmention`) is
//!   bucketed by client IP, preferring the platform proxy's forwarded headers
//!   (Railway terminates TLS at the edge) and falling back to the peer address.
//!
//! The limit is configured by `INKWELL_WRITE_RATE_LIMIT` (requests per minute,
//! see [`crate::config`]). `0` disables limiting entirely. Over-limit requests
//! get `429 Too Many Requests` with a `Retry-After` header (see
//! [`crate::error::AppError::TooManyRequests`]).

use std::net::SocketAddr;
use std::num::NonZeroU32;
use std::sync::Arc;

use axum::extract::{ConnectInfo, Request, State};
use axum::http::{HeaderMap, Method};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use governor::clock::{Clock, DefaultClock};
use governor::{DefaultKeyedRateLimiter, Quota, RateLimiter};

use crate::domain::token;
use crate::error::AppError;
use crate::http::auth_session::extract_session_cookie;

/// Shared rate-limit state: one keyed GCRA limiter for the whole process.
///
/// `limiter` is `None` when `INKWELL_WRITE_RATE_LIMIT=0` (limiting disabled),
/// in which case [`check`](RateLimitState::check) always allows the request.
pub struct RateLimitState {
    limiter: Option<DefaultKeyedRateLimiter<String>>,
    clock: DefaultClock,
    /// Whether to bucket browser-session callers by their `inkwell_session`
    /// cookie (set from `INKWELL_BROWSER_LOGIN`). Mirrors the auth layer, which
    /// only consults the cookie when browser login is enabled.
    session_keying: bool,
}

impl RateLimitState {
    /// Build the limiter for `per_minute` mutating requests per key. `0`
    /// disables limiting (the limiter is `None`). `session_keying` enables
    /// per-session bucketing of cookie-authenticated callers (pass
    /// `config.browser_login`).
    pub fn new(per_minute: u32, session_keying: bool) -> Self {
        let limiter = NonZeroU32::new(per_minute).map(|n| RateLimiter::keyed(Quota::per_minute(n)));
        Self {
            limiter,
            clock: DefaultClock::default(),
            session_keying,
        }
    }

    /// Check one request against `key`. `Ok(())` allows it; `Err(secs)` rejects
    /// it and reports how many whole seconds until the next request would pass
    /// (always at least 1, for the `Retry-After` header).
    fn check(&self, key: &str) -> Result<(), u64> {
        let Some(limiter) = self.limiter.as_ref() else {
            return Ok(());
        };
        match limiter.check_key(&key.to_owned()) {
            Ok(()) => Ok(()),
            Err(not_until) => {
                let wait = not_until.wait_time_from(self.clock.now());
                // Round up: never advertise a 0-second Retry-After.
                let secs = wait.as_secs() + u64::from(wait.subsec_nanos() > 0);
                Err(secs.max(1))
            }
        }
    }
}

/// Axum middleware: throttle mutating + `/ask` traffic, pass everything else
/// through untouched.
pub async fn rate_limit(
    State(state): State<Arc<RateLimitState>>,
    request: Request,
    next: Next,
) -> Response {
    if !should_limit(request.method(), request.uri().path()) {
        return next.run(request).await;
    }
    match state.check(&client_key(&request, state.session_keying)) {
        Ok(()) => next.run(request).await,
        Err(retry_after_secs) => AppError::TooManyRequests { retry_after_secs }.into_response(),
    }
}

/// Which requests the limiter governs: any non-idempotent mutation, plus `/ask`
/// (which is `GET|POST` but drives two AI providers). Reads (`GET`/`HEAD`) and
/// the public HTML site are deliberately excluded.
fn should_limit(method: &Method, path: &str) -> bool {
    matches!(
        *method,
        Method::POST | Method::PUT | Method::PATCH | Method::DELETE
    ) || path == "/ask"
}

/// Bucket key, in the same precedence the auth layer resolves a principal:
/// the `x-api-key` credential, then (when browser login is on) the
/// `inkwell_session` cookie, then the client IP. Keying sessions by their own
/// token means two logged-in users behind one NAT/IP get independent quotas
/// instead of throttling each other.
fn client_key(request: &Request, session_keying: bool) -> String {
    if let Some(hash) = credential_hash(request.headers()) {
        return format!("k:{hash}");
    }
    if session_keying && let Some(raw) = extract_session_cookie(request.headers()) {
        // Hash the session token; the raw cookie never enters the limiter map.
        return format!("s:{}", token::sha256_hex(&raw));
    }
    format!("ip:{}", client_ip(request))
}

/// SHA-256 of the single ASCII `x-api-key` credential, or `None` when the
/// header is absent, duplicated, or non-ASCII (mirrors the auth layer's
/// single-header rule). The raw secret never enters the limiter map.
fn credential_hash(headers: &HeaderMap) -> Option<String> {
    let mut values = headers.get_all("x-api-key").iter();
    let value = values.next()?;
    if values.next().is_some() {
        // More than one credential header: bucket by IP rather than guess.
        return None;
    }
    Some(token::sha256_hex(value.to_str().ok()?))
}

/// Best-effort client IP. Prefers the platform proxy's forwarded headers
/// (Railway/edge), then the peer address from [`ConnectInfo`]. Falls back to a
/// constant when none is available (e.g. tower `oneshot` in tests) — anonymous
/// callers then share one bucket, a safe over-approximation.
fn client_ip(request: &Request) -> String {
    let headers = request.headers();
    if let Some(forwarded) = headers
        .get("x-forwarded-for")
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.split(',').next())
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        return forwarded.to_string();
    }
    if let Some(real) = headers
        .get("x-real-ip")
        .and_then(|value| value.to_str().ok())
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        return real.to_string();
    }
    if let Some(ConnectInfo(addr)) = request.extensions().get::<ConnectInfo<SocketAddr>>() {
        return addr.ip().to_string();
    }
    "unknown".to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn should_limit_targets_writes_and_ask_only() {
        assert!(should_limit(&Method::POST, "/documents"));
        assert!(should_limit(&Method::PUT, "/documents/x"));
        assert!(should_limit(&Method::PATCH, "/documents/x"));
        assert!(should_limit(&Method::DELETE, "/documents/x"));
        assert!(should_limit(&Method::POST, "/media"));
        // `/ask` is throttled on any method — it is GET|POST and expensive.
        assert!(should_limit(&Method::GET, "/ask"));
        // Reads and the public HTML site stay unthrottled.
        assert!(!should_limit(&Method::GET, "/documents"));
        assert!(!should_limit(&Method::GET, "/"));
        assert!(!should_limit(&Method::GET, "/media/abc"));
        assert!(!should_limit(&Method::HEAD, "/"));
    }

    #[test]
    fn limiter_allows_burst_up_to_limit_then_blocks() {
        let state = RateLimitState::new(2, false);
        assert!(state.check("k:a").is_ok());
        assert!(state.check("k:a").is_ok());
        let third = state.check("k:a");
        assert!(third.is_err(), "third request over a limit of 2 must block");
        assert!(
            third.unwrap_err() >= 1,
            "Retry-After must be at least one second"
        );
    }

    #[test]
    fn limiter_buckets_are_independent_per_key() {
        let state = RateLimitState::new(1, false);
        assert!(state.check("k:a").is_ok());
        assert!(state.check("k:a").is_err());
        // A different principal / IP has its own quota.
        assert!(state.check("ip:1.2.3.4").is_ok());
    }

    #[test]
    fn zero_disables_rate_limiting() {
        let state = RateLimitState::new(0, false);
        for _ in 0..1000 {
            assert!(state.check("k:a").is_ok());
        }
    }

    fn post_with(headers: &[(&str, &str)]) -> Request {
        let mut builder = axum::http::Request::builder()
            .method(Method::POST)
            .uri("/documents");
        for (name, value) in headers {
            builder = builder.header(*name, *value);
        }
        builder.body(axum::body::Body::empty()).unwrap()
    }

    #[test]
    fn session_cookie_buckets_distinct_users_when_enabled() {
        // Browser login ON: two distinct session tokens behind one NAT/IP get
        // independent `s:` buckets instead of sharing an `ip:` bucket.
        let a = client_key(&post_with(&[("cookie", "inkwell_session=aaa")]), true);
        let b = client_key(&post_with(&[("cookie", "inkwell_session=bbb")]), true);
        assert!(a.starts_with("s:"), "session key, got {a}");
        assert_ne!(a, b, "distinct sessions must not share a bucket");

        // Browser login OFF: the cookie is ignored (mirrors auth), so both fall
        // to the same IP bucket.
        let a_off = client_key(&post_with(&[("cookie", "inkwell_session=aaa")]), false);
        let b_off = client_key(&post_with(&[("cookie", "inkwell_session=bbb")]), false);
        assert!(a_off.starts_with("ip:"), "ip key, got {a_off}");
        assert_eq!(a_off, b_off);
    }

    #[test]
    fn api_key_takes_precedence_over_session_cookie() {
        // A present x-api-key wins even with session keying on (matches auth).
        let key = client_key(
            &post_with(&[("x-api-key", "shared"), ("cookie", "inkwell_session=aaa")]),
            true,
        );
        assert!(key.starts_with("k:"), "credential key, got {key}");
    }
}
