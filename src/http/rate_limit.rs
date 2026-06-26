//! Pragmatic write rate limiting (CIL-128).
//!
//! A single global GCRA limiter (the [`governor`] crate) throttles *mutation*
//! traffic — every `POST`/`PUT`/`PATCH`/`DELETE` plus the expensive `/ask`
//! endpoint (which answers via Voyage + Anthropic and is `GET|POST`). Read
//! paths and the public HTML site are never consulted by the limiter, so the
//! garden stays fast for visitors.
//!
//! Requests are bucketed by the **validated principal** when one resolves and
//! by **client IP** otherwise. Keying goes through the same [`authenticate`]
//! the handlers use, so the credential is *verified* before it can mint a
//! bucket — an attacker cannot evade the per-IP limit (or grow the limiter map)
//! by spraying random `x-api-key` / `inkwell_session` values, because an invalid
//! credential resolves to no principal and falls back to the IP bucket:
//! - a valid `x-api-key` (shared key or scoped token) or a valid browser session
//!   buckets by `p:<author-id>` — bounded by the number of real principals;
//! - everything else buckets by client IP. Forwarded headers
//!   (`X-Forwarded-For` / `X-Real-IP`) are trusted **only** when
//!   `INKWELL_TRUST_FORWARDED_HEADERS=true` (a deployment behind a trusted proxy
//!   that overwrites them, e.g. Railway). Off by default, so a directly-exposed
//!   instance can't be spoofed and the IP keyspace stays bounded by real peers.
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
use sqlx::PgPool;

use crate::config::Config;
use crate::error::AppError;
use crate::http::auth::authenticate;

/// The keyed GCRA limiter plus its clock. Split out from [`RateLimitState`] so
/// the limiter algorithm is unit-testable without a `Config`/`PgPool`.
struct Limiter {
    /// `None` when `INKWELL_WRITE_RATE_LIMIT=0` (limiting disabled).
    inner: Option<DefaultKeyedRateLimiter<String>>,
    clock: DefaultClock,
}

impl Limiter {
    fn new(per_minute: u32) -> Self {
        let inner = NonZeroU32::new(per_minute).map(|n| RateLimiter::keyed(Quota::per_minute(n)));
        Self {
            inner,
            clock: DefaultClock::default(),
        }
    }

    fn enabled(&self) -> bool {
        self.inner.is_some()
    }

    /// `Ok(())` allows the request; `Err(secs)` rejects it and reports how many
    /// whole seconds until the next request would pass (always at least 1, for
    /// the `Retry-After` header).
    fn check(&self, key: &str) -> Result<(), u64> {
        let Some(limiter) = self.inner.as_ref() else {
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

/// Shared rate-limit state for the whole process: the limiter plus the handles
/// [`authenticate`] needs to *validate* a credential before it keys a bucket.
pub struct RateLimitState {
    limiter: Limiter,
    config: Arc<Config>,
    pool: PgPool,
    /// Trust `X-Forwarded-For` / `X-Real-IP` for IP keying. From
    /// `INKWELL_TRUST_FORWARDED_HEADERS`; only safe behind a proxy that
    /// overwrites those headers.
    trust_forwarded: bool,
}

impl RateLimitState {
    /// Build the limiter from `config.write_rate_limit` (`0` disables) and wire
    /// in the `config`/`pool` used to validate credentials during keying.
    pub fn new(config: Arc<Config>, pool: PgPool) -> Self {
        let limiter = Limiter::new(config.write_rate_limit);
        let trust_forwarded = config.trust_forwarded_headers;
        Self {
            limiter,
            config,
            pool,
            trust_forwarded,
        }
    }

    /// Resolve the bucket key: `p:<author-id>` for a validated principal, else
    /// `ip:<client-ip>`. Validation reuses [`authenticate`] so an invalid or
    /// forged credential never produces its own bucket.
    ///
    /// Takes `&HeaderMap` + the peer address rather than `&Request`: the request
    /// body (`axum::body::Body`) is `!Sync`, so holding `&Request` across the
    /// auth `.await` would make the middleware future non-`Send`. `HeaderMap` is
    /// `Sync`, so a borrow of it is fine across the await.
    async fn resolve_key(&self, headers: &HeaderMap, peer: Option<SocketAddr>) -> String {
        if let Some(principal) = authenticate(headers, &self.config, &self.pool).await {
            let id = principal
                .author_id
                .map(|id| id.to_string())
                .unwrap_or_else(|| principal.label.clone());
            return format!("p:{id}");
        }
        format!("ip:{}", client_ip(headers, peer, self.trust_forwarded))
    }
}

/// Axum middleware: throttle mutating + `/ask` traffic, pass everything else
/// through untouched. When limiting is disabled (or the request isn't a
/// mutation) it short-circuits before resolving any principal — a true no-op,
/// no auth or key work.
pub async fn rate_limit(
    State(state): State<Arc<RateLimitState>>,
    request: Request,
    next: Next,
) -> Response {
    if !state.limiter.enabled() || !should_limit(request.method(), request.uri().path()) {
        return next.run(request).await;
    }
    // Pull the peer address before the await so we don't hold a `&Request`
    // (whose body is `!Sync`) across `resolve_key`.
    let peer = request
        .extensions()
        .get::<ConnectInfo<SocketAddr>>()
        .map(|connect_info| connect_info.0);
    let key = state.resolve_key(request.headers(), peer).await;
    match state.limiter.check(&key) {
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

/// Best-effort client IP. When `trust_forwarded` is set, prefers the platform
/// proxy's forwarded headers (`X-Forwarded-For` first hop, then `X-Real-IP`);
/// otherwise ignores them so they can't be spoofed. Falls back to the `peer`
/// address (from [`ConnectInfo`]), then a constant when none is available (e.g.
/// tower `oneshot` in tests) — anonymous callers then share one bucket, a safe
/// over-approximation.
fn client_ip(headers: &HeaderMap, peer: Option<SocketAddr>, trust_forwarded: bool) -> String {
    if trust_forwarded {
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
    }
    match peer {
        Some(addr) => addr.ip().to_string(),
        None => "unknown".to_string(),
    }
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
        let limiter = Limiter::new(2);
        assert!(limiter.check("p:a").is_ok());
        assert!(limiter.check("p:a").is_ok());
        let third = limiter.check("p:a");
        assert!(third.is_err(), "third request over a limit of 2 must block");
        assert!(
            third.unwrap_err() >= 1,
            "Retry-After must be at least one second"
        );
    }

    #[test]
    fn limiter_buckets_are_independent_per_key() {
        let limiter = Limiter::new(1);
        assert!(limiter.check("p:a").is_ok());
        assert!(limiter.check("p:a").is_err());
        // A different principal / IP has its own quota.
        assert!(limiter.check("ip:1.2.3.4").is_ok());
    }

    #[test]
    fn zero_disables_rate_limiting() {
        let limiter = Limiter::new(0);
        assert!(!limiter.enabled());
        for _ in 0..1000 {
            assert!(limiter.check("p:a").is_ok());
        }
    }

    #[test]
    fn client_ip_ignores_forwarded_headers_unless_trusted() {
        let headers = |xff: &str| {
            let mut map = HeaderMap::new();
            map.insert("x-forwarded-for", xff.parse().unwrap());
            map
        };
        let peer: Option<SocketAddr> = Some("203.0.113.7:51000".parse().unwrap());
        // Untrusted: the spoofable header is ignored; the real peer wins.
        assert_eq!(client_ip(&headers("9.9.9.9"), peer, false), "203.0.113.7");
        // Untrusted with no peer (e.g. tower oneshot) falls back to the constant.
        assert_eq!(client_ip(&headers("9.9.9.9"), None, false), "unknown");
        // Trusted: the first forwarded hop is honored.
        assert_eq!(
            client_ip(&headers("9.9.9.9, 10.0.0.1"), peer, true),
            "9.9.9.9"
        );
    }
}
