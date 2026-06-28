//! DB-backed HTTP contract tests for optimistic concurrency via `If-Match`.
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

#[tokio::test]
async fn stale_if_match_returns_409() -> anyhow::Result<()> {
    let _guard = db_guard().await;
    let Some(pool) = common::maybe_pool().await? else {
        return Ok(());
    };
    let router = common::router_for(pool);

    let create_payload = serde_json::json!({
        "title": "If-Match Contract",
        "bodyMarkdown": "# If-Match\nInitial body.",
        "slug": "if-match-contract",
    });
    let created = router
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/documents")
                .header("content-type", "application/json")
                .header("x-api-key", SHARED_KEY)
                .body(Body::from(serde_json::to_vec(&create_payload)?))?,
        )
        .await?;
    assert_eq!(created.status(), StatusCode::CREATED);
    let created_body = body_json(created).await?;
    let version = created_body["version"]
        .as_i64()
        .expect("version in create response");

    let first_patch = serde_json::json!({ "title": "If-Match Contract Updated" });
    let updated = router
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::PATCH)
                .uri("/documents/if-match-contract")
                .header("content-type", "application/json")
                .header("x-api-key", SHARED_KEY)
                .header("if-match", version.to_string())
                .body(Body::from(serde_json::to_vec(&first_patch)?))?,
        )
        .await?;
    assert_eq!(updated.status(), StatusCode::OK);
    let updated_body = body_json(updated).await?;
    let updated_version = updated_body["version"]
        .as_i64()
        .expect("version in update response");
    assert!(
        updated_version > version,
        "successful update must advance version"
    );

    let stale_patch = serde_json::json!({ "title": "If-Match Stale Write" });
    let stale = router
        .oneshot(
            Request::builder()
                .method(Method::PATCH)
                .uri("/documents/if-match-contract")
                .header("content-type", "application/json")
                .header("x-api-key", SHARED_KEY)
                .header("if-match", version.to_string())
                .body(Body::from(serde_json::to_vec(&stale_patch)?))?,
        )
        .await?;
    assert_eq!(stale.status(), StatusCode::CONFLICT);
    Ok(())
}
