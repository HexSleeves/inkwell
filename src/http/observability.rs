//! Request observability: per-request span, one structured log event, and the
//! `/metrics` scrape endpoint (CYP-46).
//!
//! ## Why not `TraceLayer`
//!
//! `tower_http`'s `TraceLayer` emits *two* events per request (on-request and
//! on-response) at `DEBUG`, so at the default filter Inkwell logged nothing at
//! all for a request. [`observe`] replaces it with exactly **one** `INFO` event
//! per request carrying `request_id`, `method`, `route`, `status`, and
//! `latency_ms`, plus a surrounding span so anything a handler logs inherits the
//! same correlation fields.
//!
//! ## Redaction
//!
//! The event and span deliberately carry only the **route template** (from
//! [`MatchedPath`]), never the raw URI — a raw path would leak the preview
//! token in `/documents/{slug}/preview?token=…`. No header, cookie, body, or
//! query value is ever logged here, so tokens, API keys, and `Authorization`
//! cannot reach the log stream through this layer. This complements the
//! `Debug`-level redaction of [`Config`](crate::config::Config) from CYP-35.

use std::sync::Arc;
use std::time::Instant;

use axum::extract::{MatchedPath, Request, State};
use axum::http::{HeaderMap, StatusCode, header};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use sha2::{Digest, Sha256};
use subtle::ConstantTimeEq;
use tracing::Instrument;

use crate::error::AppError;
use crate::http::AppState;
use crate::http::metrics::{Metrics, RuntimeGauges, UNMATCHED_ROUTE};
use crate::http::request_id;

/// `Content-Type` for the Prometheus text exposition format.
const PROMETHEUS_CONTENT_TYPE: &str = "text/plain; version=0.0.4; charset=utf-8";

/// Middleware: time the request, record it against the metrics registry, and
/// emit the single structured log event for it.
///
/// Sits inside [`request_id::propagate_request_id`] (so the correlation id is in
/// scope) and outside the rate limiter and security headers (so a 429 or a
/// rejected request is still counted and logged).
pub async fn observe(
    State(metrics): State<Arc<Metrics>>,
    request: Request,
    next: Next,
) -> Response {
    // `MatchedPath` is inserted by the router before `Router::layer` middleware
    // runs, so the template is available here. It is absent only for requests
    // that matched nothing, which share one bounded label value.
    let route = request
        .extensions()
        .get::<MatchedPath>()
        .map(|matched| matched.as_str().to_owned())
        .unwrap_or_else(|| UNMATCHED_ROUTE.to_owned());
    let method = request.method().clone();
    let request_id = request_id::current().unwrap_or_default();

    let span = tracing::info_span!(
        "http_request",
        %method,
        route = %route,
        %request_id,
    );

    let started = Instant::now();
    let response = next.run(request).instrument(span.clone()).await;
    let latency = started.elapsed();
    let status = response.status();

    metrics.record(method.as_str(), &route, status.as_u16(), latency);

    // One event per request. `parent: &span` keeps it correlated with anything
    // the handler logged, while repeating the fields makes each line
    // self-contained for log search.
    tracing::info!(
        parent: &span,
        %method,
        route = %route,
        status = status.as_u16(),
        // Rounded to microsecond precision: full f64 latency prints 15 noisy
        // digits and nothing downstream needs them.
        latency_ms = (latency.as_secs_f64() * 1_000_000.0).round() / 1_000.0,
        %request_id,
        "request completed"
    );

    response
}

/// `GET /metrics` — Prometheus text exposition.
///
/// Registered only when `INKWELL_METRICS_ENABLED=true`, so the endpoint does not
/// exist on a default install. When `INKWELL_METRICS_TOKEN` is set, a matching
/// `Authorization: Bearer <token>` is required.
pub async fn metrics(
    State(state): State<AppState>,
    method: axum::http::Method,
    headers: HeaderMap,
) -> Result<Response, AppError> {
    if method != axum::http::Method::GET {
        return Err(AppError::MethodNotAllowed(vec!["GET"]));
    }
    authorize_scrape(&headers, state.config.metrics_token.as_deref())?;

    let gauges = RuntimeGauges {
        db_pool_connections: state.pool.size(),
        // `num_idle` is a `usize`; the pool is capped well below `u32::MAX`.
        db_pool_idle: u32::try_from(state.pool.num_idle()).unwrap_or(u32::MAX),
    };
    let body = state.metrics.render(gauges);

    Ok((
        StatusCode::OK,
        [(header::CONTENT_TYPE, PROMETHEUS_CONTENT_TYPE)],
        body,
    )
        .into_response())
}

/// Enforce the optional scrape token. `None` means the operator enabled metrics
/// without a token and is relying on network isolation instead.
fn authorize_scrape(headers: &HeaderMap, expected: Option<&str>) -> Result<(), AppError> {
    let Some(expected) = expected.filter(|token| !token.is_empty()) else {
        return Ok(());
    };
    let presented = headers
        .get(header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.strip_prefix("Bearer "))
        .map(str::trim)
        .unwrap_or_default();
    // Hash both sides before comparing so the comparison is constant-time and
    // independent of token length, matching `http::auth::match_static_key`.
    let presented_hash = Sha256::digest(presented.as_bytes());
    let expected_hash = Sha256::digest(expected.as_bytes());
    if bool::from(presented_hash.ct_eq(&expected_hash)) {
        Ok(())
    } else {
        Err(AppError::Unauthorized)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::HeaderValue;

    fn headers_with_auth(value: &str) -> HeaderMap {
        let mut headers = HeaderMap::new();
        headers.insert(
            header::AUTHORIZATION,
            HeaderValue::from_str(value).expect("test header is valid"),
        );
        headers
    }

    #[test]
    fn no_configured_token_allows_any_scrape() {
        assert!(authorize_scrape(&HeaderMap::new(), None).is_ok());
        assert!(authorize_scrape(&HeaderMap::new(), Some("")).is_ok());
    }

    #[test]
    fn configured_token_requires_matching_bearer() {
        let expected = Some("scrape-secret");
        assert!(authorize_scrape(&headers_with_auth("Bearer scrape-secret"), expected).is_ok());
        assert!(matches!(
            authorize_scrape(&headers_with_auth("Bearer wrong"), expected),
            Err(AppError::Unauthorized)
        ));
        assert!(matches!(
            authorize_scrape(&headers_with_auth("scrape-secret"), expected),
            Err(AppError::Unauthorized)
        ));
        assert!(matches!(
            authorize_scrape(&HeaderMap::new(), expected),
            Err(AppError::Unauthorized)
        ));
    }
}
