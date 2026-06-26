//! Database-backed contract tests for pragmatic write rate limiting (CIL-128).
//!
//! Asserts the three behaviours the brief calls for:
//! - a burst of writes over the configured limit yields `429 Too Many Requests`
//!   carrying a positive `Retry-After`;
//! - a normal write rate (under the limit) passes (`201 Created`);
//! - read routes and the public HTML site are never throttled, even after the
//!   write bucket for that principal is exhausted.
//!
//! Skipped unless `DATABASE_URL` is set (or forced via `INKWELL_REQUIRE_DB_TESTS=1`).

mod common;

use axum::body::Body;
use common::{maybe_pool, router_for_with_rate_limit};
use http::{Method, Request, StatusCode, header};
use std::sync::LazyLock;
use tokio::sync::{Mutex, MutexGuard};
use tower::ServiceExt;

static DB_TEST_LOCK: LazyLock<Mutex<()>> = LazyLock::new(|| Mutex::new(()));

async fn db_guard() -> MutexGuard<'static, ()> {
    DB_TEST_LOCK.lock().await
}

const SHARED_KEY: &str = "test-secret-key";

/// `POST /documents` with the shared (admin) key and a unique title (so each
/// would-be-success creates a distinct slug rather than colliding with a 409).
async fn post_doc(router: &axum::Router, title: &str) -> anyhow::Result<axum::response::Response> {
    let payload = serde_json::json!({ "title": title, "bodyMarkdown": "# Hi" });
    Ok(router
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/documents")
                .header("content-type", "application/json")
                .header("x-api-key", SHARED_KEY)
                .body(Body::from(serde_json::to_vec(&payload)?))?,
        )
        .await?)
}

/// An unauthenticated `GET` (a read / public-site request).
async fn get(router: &axum::Router, uri: &str) -> anyhow::Result<axum::response::Response> {
    Ok(router
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::GET)
                .uri(uri)
                .body(Body::empty())?,
        )
        .await?)
}

#[tokio::test]
async fn write_burst_over_limit_returns_429_with_retry_after() -> anyhow::Result<()> {
    let _guard = db_guard().await;
    let Some(pool) = maybe_pool().await? else {
        return Ok(());
    };
    let limit: u32 = 3;
    let router = router_for_with_rate_limit(pool, limit);

    let mut statuses = Vec::new();
    let mut first_throttled: Option<axum::response::Response> = None;
    for i in 0..(limit + 3) {
        let response = post_doc(&router, &format!("rate-limit-doc-{i}")).await?;
        let status = response.status();
        statuses.push(status);
        if status == StatusCode::TOO_MANY_REQUESTS && first_throttled.is_none() {
            first_throttled = Some(response);
        }
    }

    // Normal rate (the first `limit` writes) passes.
    for (i, status) in statuses.iter().take(limit as usize).enumerate() {
        assert_eq!(
            *status,
            StatusCode::CREATED,
            "write {i} (under the limit) should succeed; statuses={statuses:?}"
        );
    }

    // A burst over the limit yields 429.
    assert!(
        statuses.contains(&StatusCode::TOO_MANY_REQUESTS),
        "a burst over the limit must yield 429; statuses={statuses:?}"
    );

    // The 429 advertises a positive integer Retry-After (seconds).
    let throttled = first_throttled.expect("at least one throttled response");
    let retry_after = throttled
        .headers()
        .get(header::RETRY_AFTER)
        .expect("Retry-After header present on 429")
        .to_str()?
        .parse::<u64>()?;
    assert!(retry_after >= 1, "Retry-After must be >= 1 second");

    Ok(())
}

#[tokio::test]
async fn read_routes_and_public_site_are_not_rate_limited() -> anyhow::Result<()> {
    let _guard = db_guard().await;
    let Some(pool) = maybe_pool().await? else {
        return Ok(());
    };
    // Limit of 1 write/minute so a single extra write trips the limiter.
    let router = router_for_with_rate_limit(pool, 1);

    // Exhaust the write bucket for the shared-key principal.
    assert_eq!(
        post_doc(&router, "seed-doc").await?.status(),
        StatusCode::CREATED
    );
    assert_eq!(
        post_doc(&router, "seed-doc-2").await?.status(),
        StatusCode::TOO_MANY_REQUESTS,
        "second write over a limit of 1 must be throttled"
    );

    // Reads stay unthrottled: many GETs all succeed, never 429.
    for _ in 0..6 {
        assert_eq!(
            get(&router, "/documents").await?.status(),
            StatusCode::OK,
            "GET /documents must never be rate limited"
        );
    }
    // The public HTML index is likewise unthrottled.
    assert_eq!(
        get(&router, "/").await?.status(),
        StatusCode::OK,
        "public HTML site must never be rate limited"
    );

    Ok(())
}
