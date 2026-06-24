//! DB-backed contract test: `set_document_status` (publish/unpublish) must bump
//! `version` and `updated_at` so that status changes are visible to the
//! optimistic-concurrency `If-Match` guard and to ETag/`updated_at`-based caching.
//!
//! Skipped unless `DATABASE_URL` is set (or forced via `INKWELL_REQUIRE_DB_TESTS=1`).

mod common;

use axum::body::{Body, to_bytes};
use http::{Method, Request, StatusCode};
use std::sync::LazyLock;
use tokio::sync::{Mutex, MutexGuard};
use tower::ServiceExt;

static DB_TEST_LOCK: LazyLock<Mutex<()>> = LazyLock::new(|| Mutex::new(()));

async fn db_guard() -> MutexGuard<'static, ()> {
    DB_TEST_LOCK.lock().await
}

const SHARED_KEY: &str = "test-secret-key";

async fn body_json(response: axum::response::Response) -> anyhow::Result<serde_json::Value> {
    let bytes = to_bytes(response.into_body(), usize::MAX).await?;
    Ok(serde_json::from_slice(&bytes)?)
}

/// Publish and unpublish each bump `version` by exactly +1 and advance `updated_at`.
///
/// Prior to the fix, `set_document_status` only wrote `status`; a
/// publish/unpublish was therefore invisible to the `If-Match` concurrency guard
/// and to any ETag/`updated_at`-derived cache. This test asserts the correct
/// post-fix behaviour.
#[tokio::test]
async fn publish_and_unpublish_bump_version_and_updated_at() -> anyhow::Result<()> {
    let _guard = db_guard().await;
    let Some(pool) = common::maybe_pool().await? else {
        return Ok(());
    };
    let router = common::router_for(pool);

    // ── 1. Create a document and capture initial version / updated_at ────────
    let response = router
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/documents")
                .header("content-type", "application/json")
                .header("x-api-key", SHARED_KEY)
                .body(Body::from(
                    r##"{"title":"Status Version Test","bodyMarkdown":"# hi"}"##,
                ))?,
        )
        .await?;
    assert_eq!(response.status(), StatusCode::CREATED);
    let created = body_json(response).await?;
    let slug = created["slug"].as_str().expect("slug in response");
    let initial_version = created["version"].as_i64().expect("version in response");
    let initial_updated_at = created["updatedAt"]
        .as_str()
        .expect("updatedAt in response")
        .to_string();

    // ── 2. Publish — version must increase by exactly 1, updatedAt must advance ─
    let publish_uri = format!("/documents/{slug}/publish");
    let response = router
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri(&publish_uri)
                .header("x-api-key", SHARED_KEY)
                .body(Body::empty())?,
        )
        .await?;
    assert_eq!(response.status(), StatusCode::OK, "publish should succeed");
    let after_publish = body_json(response).await?;

    let published_version = after_publish["version"]
        .as_i64()
        .expect("version in publish response");
    let published_updated_at = after_publish["updatedAt"]
        .as_str()
        .expect("updatedAt in publish response")
        .to_string();

    assert_eq!(
        published_version,
        initial_version + 1,
        "publish must bump version by exactly 1 (was {initial_version}, got {published_version})"
    );
    assert_ne!(
        published_updated_at, initial_updated_at,
        "publish must advance updated_at"
    );
    assert_eq!(
        after_publish["status"].as_str(),
        Some("published"),
        "status must be 'published' after publish"
    );

    // ── 3. Unpublish — version must increase by 1 again ─────────────────────
    let unpublish_uri = format!("/documents/{slug}/unpublish");
    let response = router
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri(&unpublish_uri)
                .header("x-api-key", SHARED_KEY)
                .body(Body::empty())?,
        )
        .await?;
    assert_eq!(
        response.status(),
        StatusCode::OK,
        "unpublish should succeed"
    );
    let after_unpublish = body_json(response).await?;

    let unpublished_version = after_unpublish["version"]
        .as_i64()
        .expect("version in unpublish response");

    assert_eq!(
        unpublished_version,
        published_version + 1,
        "unpublish must bump version by exactly 1 (was {published_version}, got {unpublished_version})"
    );
    assert_eq!(
        after_unpublish["status"].as_str(),
        Some("draft"),
        "status must be 'draft' after unpublish"
    );

    Ok(())
}
