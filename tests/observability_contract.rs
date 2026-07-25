//! Contract: structured logs, route-template metric labels, and `/metrics` (CYP-46).
//!
//! The first group needs no database: it drives the real
//! [`inkwell::http::observability::observe`] middleware over a tiny probe router
//! so the label/log behaviour is asserted directly.
//!
//!   (a) two different document ids produce **one** metric series, because the
//!       label is the route template and not the raw path;
//!   (b) exactly **one** log event per request, carrying `request_id`, `route`,
//!       `status`, and `latency_ms`;
//!   (c) no secret material — bearer token, API key, cookie, or a preview token
//!       in the query string — reaches the log stream.
//!
//! The second group is database-backed and asserts the wired `/metrics` and
//! `/healthz` / `/readyz` surfaces on the real router.

mod common;

use std::io::Write;
use std::sync::{Arc, LazyLock, Mutex};

use axum::body::{Body, to_bytes};
use axum::middleware;
use axum::routing::get;
use axum::{Router, http::Request};
use http::{Method, StatusCode};
use inkwell::http::health;
use inkwell::http::metrics::Metrics;
use inkwell::http::observability::observe;
use inkwell::http::request_id;
use tokio::sync::{Mutex as AsyncMutex, MutexGuard};
use tower::ServiceExt;
use tracing_subscriber::fmt::MakeWriter;

// Shares the single test database with the other contract suites; serialize so
// the truncation in `maybe_pool` cannot race a sibling test.
static DB_TEST_LOCK: LazyLock<AsyncMutex<()>> = LazyLock::new(|| AsyncMutex::new(()));

async fn db_guard() -> MutexGuard<'static, ()> {
    DB_TEST_LOCK.lock().await
}

/// A probe router carrying the real observability + request-id middleware over
/// handlers that touch nothing, so these assertions need no database.
fn probe_router(metrics: Arc<Metrics>) -> Router {
    Router::new()
        .route("/documents/{slug}", get(|| async { "ok" }))
        .route(
            "/documents/{slug}/preview",
            get(|| async { StatusCode::FORBIDDEN }),
        )
        .route("/healthz", get(health::liveness))
        .layer(middleware::from_fn_with_state(metrics, observe))
        .layer(middleware::from_fn(request_id::propagate_request_id))
}

/// Collects log output so a test can assert on the emitted JSON.
#[derive(Clone)]
struct CapturedLogs(Arc<Mutex<Vec<u8>>>);

impl CapturedLogs {
    fn new() -> Self {
        Self(Arc::new(Mutex::new(Vec::new())))
    }

    fn lines(&self) -> Vec<String> {
        let bytes = self.0.lock().expect("log buffer is not poisoned").clone();
        String::from_utf8_lossy(&bytes)
            .lines()
            .filter(|line| !line.trim().is_empty())
            .map(str::to_owned)
            .collect()
    }
}

impl Write for CapturedLogs {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.0
            .lock()
            .expect("log buffer is not poisoned")
            .extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

impl<'a> MakeWriter<'a> for CapturedLogs {
    type Writer = CapturedLogs;

    fn make_writer(&'a self) -> Self::Writer {
        self.clone()
    }
}

/// Run `request` through a probe router with a JSON subscriber installed, and
/// return the captured log lines. A current-thread runtime keeps every future on
/// the thread that owns the thread-local subscriber.
fn logs_for(request: Request<Body>) -> Vec<String> {
    let captured = CapturedLogs::new();
    let subscriber = tracing_subscriber::fmt()
        .json()
        .with_current_span(true)
        .with_writer(captured.clone())
        .with_max_level(tracing::Level::INFO)
        .finish();

    tracing::subscriber::with_default(subscriber, || {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("current-thread runtime builds");
        runtime.block_on(async {
            probe_router(Arc::new(Metrics::new()))
                .oneshot(request)
                .await
                .expect("probe router responds");
        });
    });

    captured.lines()
}

#[tokio::test]
async fn route_template_labels_collapse_distinct_document_ids() -> anyhow::Result<()> {
    let metrics = Arc::new(Metrics::new());

    for slug in ["first-note", "second-note"] {
        let response = probe_router(metrics.clone())
            .oneshot(
                Request::builder()
                    .method(Method::GET)
                    .uri(format!("/documents/{slug}"))
                    .body(Body::empty())?,
            )
            .await?;
        assert_eq!(response.status(), StatusCode::OK);
    }

    // The whole point of the path *template*: unbounded ids must not create
    // unbounded label values.
    assert_eq!(
        metrics.series_count(),
        1,
        "two different document ids share one metric series"
    );
    let body = metrics.render(inkwell::http::metrics::RuntimeGauges {
        db_pool_connections: 0,
        db_pool_idle: 0,
    });
    assert!(
        body.contains(
            "inkwell_http_requests_total{method=\"GET\",route=\"/documents/{slug}\",status=\"200\"} 2"
        ),
        "counter is labelled with the template and counted twice:\n{body}"
    );
    assert!(
        !body.contains("first-note") && !body.contains("second-note"),
        "raw slugs must never appear as label values:\n{body}"
    );

    Ok(())
}

#[tokio::test]
async fn unmatched_requests_share_one_bounded_label() -> anyhow::Result<()> {
    let metrics = Arc::new(Metrics::new());

    for path in ["/nope-one", "/nope-two"] {
        probe_router(metrics.clone())
            .oneshot(
                Request::builder()
                    .method(Method::GET)
                    .uri(path)
                    .body(Body::empty())?,
            )
            .await?;
    }

    assert_eq!(metrics.series_count(), 1);
    let body = metrics.render(inkwell::http::metrics::RuntimeGauges {
        db_pool_connections: 0,
        db_pool_idle: 0,
    });
    assert!(body.contains(&format!(
        "route=\"{}\"",
        inkwell::http::metrics::UNMATCHED_ROUTE
    )));
    assert!(!body.contains("nope-one"));

    Ok(())
}

#[test]
fn emits_exactly_one_json_event_per_request_with_id_status_and_latency() -> anyhow::Result<()> {
    let lines = logs_for(
        Request::builder()
            .method(Method::GET)
            .uri("/documents/some-note")
            .header("x-request-id", "trace-observability-1")
            .body(Body::empty())?,
    );

    assert_eq!(
        lines.len(),
        1,
        "one request produces exactly one log event, got: {lines:?}"
    );
    let event: serde_json::Value = serde_json::from_str(&lines[0])?;

    // Valid JSON carrying the correlation fields an operator needs.
    assert_eq!(event["level"], "INFO");
    assert_eq!(event["fields"]["message"], "request completed");
    assert_eq!(event["fields"]["request_id"], "trace-observability-1");
    assert_eq!(event["fields"]["route"], "/documents/{slug}");
    assert_eq!(event["fields"]["method"], "GET");
    assert_eq!(event["fields"]["status"], 200);
    assert!(
        event["fields"]["latency_ms"].is_number(),
        "latency_ms is numeric: {event}"
    );
    // The surrounding span repeats the correlation fields, so anything a handler
    // logs is joinable on request_id.
    assert_eq!(event["span"]["request_id"], "trace-observability-1");
    assert_eq!(event["span"]["route"], "/documents/{slug}");

    Ok(())
}

#[test]
fn never_logs_tokens_api_keys_cookies_or_authorization() -> anyhow::Result<()> {
    // Every value here is sentinel secret material. None may appear in the logs.
    let lines = logs_for(
        Request::builder()
            .method(Method::GET)
            // A preview token rides in the query string, which is exactly why the
            // log carries the route template rather than the raw URI.
            .uri("/documents/some-note/preview?token=pvw_SENTINEL_PREVIEW")
            .header("authorization", "Bearer SENTINEL_BEARER_TOKEN")
            .header("x-api-key", "SENTINEL_API_KEY")
            .header("cookie", "inkwell_session=SENTINEL_SESSION_COOKIE")
            .body(Body::empty())?,
    );

    let joined = lines.join("\n");
    for secret in [
        "SENTINEL_PREVIEW",
        "SENTINEL_BEARER_TOKEN",
        "SENTINEL_API_KEY",
        "SENTINEL_SESSION_COOKIE",
        "pvw_",
        "Bearer",
        "inkwell_session",
    ] {
        assert!(
            !joined.contains(secret),
            "log output leaked {secret:?}:\n{joined}"
        );
    }
    // Sanity: we really did capture the request event, so the assertions above
    // aren't passing on an empty buffer.
    assert_eq!(lines.len(), 1, "captured the request event: {lines:?}");
    let event: serde_json::Value = serde_json::from_str(&lines[0])?;
    assert_eq!(event["fields"]["route"], "/documents/{slug}/preview");
    assert_eq!(event["fields"]["status"], 403);

    Ok(())
}

#[tokio::test]
async fn healthz_is_liveness_only_and_needs_no_database() -> anyhow::Result<()> {
    let response = probe_router(Arc::new(Metrics::new()))
        .oneshot(
            Request::builder()
                .method(Method::GET)
                .uri("/healthz")
                .body(Body::empty())?,
        )
        .await?;

    assert_eq!(response.status(), StatusCode::OK);
    let body = to_bytes(response.into_body(), 64 * 1024).await?;
    let json: serde_json::Value = serde_json::from_slice(&body)?;
    assert_eq!(json["status"], "ok");
    assert!(
        json.get("db").is_none(),
        "liveness reports no database opinion: {json}"
    );

    Ok(())
}

// ---------------------------------------------------------------------------
// Database-backed: the wired router.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn readyz_reports_database_reachability() -> anyhow::Result<()> {
    let _guard = db_guard().await;
    let Some(pool) = common::maybe_pool().await? else {
        return Ok(());
    };

    let response = common::router_for(pool)
        .oneshot(
            Request::builder()
                .method(Method::GET)
                .uri("/readyz")
                .body(Body::empty())?,
        )
        .await?;

    assert_eq!(response.status(), StatusCode::OK);
    let body = to_bytes(response.into_body(), 64 * 1024).await?;
    let json: serde_json::Value = serde_json::from_slice(&body)?;
    assert_eq!(json["status"], "ok");
    assert_eq!(json["db"], "up", "readiness asserts the DB is reachable");

    Ok(())
}

#[tokio::test]
async fn metrics_endpoint_is_absent_unless_enabled() -> anyhow::Result<()> {
    let _guard = db_guard().await;
    let Some(pool) = common::maybe_pool().await? else {
        return Ok(());
    };

    // Default config: the route is never registered, so `/metrics` falls through
    // to the public `/{slug}` document route and 404s.
    let response = common::router_for(pool)
        .oneshot(
            Request::builder()
                .method(Method::GET)
                .uri("/metrics")
                .body(Body::empty())?,
        )
        .await?;

    assert_eq!(response.status(), StatusCode::NOT_FOUND);
    Ok(())
}

#[tokio::test]
async fn metrics_endpoint_serves_prometheus_text_and_counters_move() -> anyhow::Result<()> {
    let _guard = db_guard().await;
    let Some(pool) = common::maybe_pool().await? else {
        return Ok(());
    };
    let router = common::router_for_with_metrics(pool, None);

    // Drive a little load first so the counters have something to report.
    for _ in 0..3 {
        router
            .clone()
            .oneshot(
                Request::builder()
                    .method(Method::GET)
                    .uri("/healthz")
                    .body(Body::empty())?,
            )
            .await?;
    }

    let response = router
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::GET)
                .uri("/metrics")
                .body(Body::empty())?,
        )
        .await?;
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        response
            .headers()
            .get(http::header::CONTENT_TYPE)
            .and_then(|value| value.to_str().ok()),
        Some("text/plain; version=0.0.4; charset=utf-8")
    );
    let body = to_bytes(response.into_body(), 1024 * 1024).await?;
    let body = String::from_utf8(body.to_vec())?;

    // Every family declares HELP/TYPE, which is what makes the body valid
    // Prometheus text rather than an ad-hoc dump.
    for family in [
        "inkwell_build_info",
        "inkwell_process_uptime_seconds",
        "inkwell_db_pool_connections",
        "inkwell_http_requests_total",
        "inkwell_http_request_duration_seconds",
    ] {
        assert!(
            body.contains(&format!("# TYPE {family} ")),
            "{family}:\n{body}"
        );
    }
    assert!(
        body.contains(
            "inkwell_http_requests_total{method=\"GET\",route=\"/healthz\",status=\"200\"} 3"
        ),
        "the three probe requests are counted:\n{body}"
    );
    assert!(
        body.contains(
            "inkwell_http_request_duration_seconds_count{method=\"GET\",route=\"/healthz\",status=\"200\"} 3"
        ),
        "the latency histogram moved too:\n{body}"
    );

    // One more request, then re-scrape: the counter must advance.
    router
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::GET)
                .uri("/healthz")
                .body(Body::empty())?,
        )
        .await?;
    let response = router
        .oneshot(
            Request::builder()
                .method(Method::GET)
                .uri("/metrics")
                .body(Body::empty())?,
        )
        .await?;
    let body = to_bytes(response.into_body(), 1024 * 1024).await?;
    let body = String::from_utf8(body.to_vec())?;
    assert!(
        body.contains(
            "inkwell_http_requests_total{method=\"GET\",route=\"/healthz\",status=\"200\"} 4"
        ),
        "counters keep moving under load:\n{body}"
    );

    Ok(())
}

#[tokio::test]
async fn metrics_scrape_token_is_enforced() -> anyhow::Result<()> {
    let _guard = db_guard().await;
    let Some(pool) = common::maybe_pool().await? else {
        return Ok(());
    };
    let router = common::router_for_with_metrics(pool, Some("scrape-secret"));

    let unauthenticated = router
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::GET)
                .uri("/metrics")
                .body(Body::empty())?,
        )
        .await?;
    assert_eq!(unauthenticated.status(), StatusCode::UNAUTHORIZED);

    let wrong = router
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::GET)
                .uri("/metrics")
                .header("authorization", "Bearer nope")
                .body(Body::empty())?,
        )
        .await?;
    assert_eq!(wrong.status(), StatusCode::UNAUTHORIZED);

    let authorized = router
        .oneshot(
            Request::builder()
                .method(Method::GET)
                .uri("/metrics")
                .header("authorization", "Bearer scrape-secret")
                .body(Body::empty())?,
        )
        .await?;
    assert_eq!(authorized.status(), StatusCode::OK);

    Ok(())
}
