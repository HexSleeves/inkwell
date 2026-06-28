//! Database-backed contract tests for the write-audit history API.
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

async fn mint_token(router: &axum::Router, name: &str, scopes: &[&str]) -> anyhow::Result<String> {
    let payload = serde_json::json!({ "name": name, "scopes": scopes });
    let response = router
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/admin/tokens")
                .header("content-type", "application/json")
                .header("x-api-key", SHARED_KEY)
                .body(Body::from(serde_json::to_vec(&payload)?))?,
        )
        .await?;
    assert_eq!(response.status(), StatusCode::CREATED);
    Ok(body_json(response)
        .await?
        .get("token")
        .and_then(|token| token.as_str())
        .expect("token in response")
        .to_string())
}

async fn create_document(
    router: &axum::Router,
    title: &str,
    body: &str,
    key: &str,
) -> anyhow::Result<String> {
    let payload = serde_json::json!({ "title": title, "bodyMarkdown": body });
    let response = router
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/documents")
                .header("content-type", "application/json")
                .header("x-api-key", key)
                .body(Body::from(serde_json::to_vec(&payload)?))?,
        )
        .await?;
    assert_eq!(response.status(), StatusCode::CREATED);
    Ok(body_json(response)
        .await?
        .get("slug")
        .and_then(|slug| slug.as_str())
        .expect("slug in response")
        .to_string())
}

async fn update_document(router: &axum::Router, slug: &str, key: &str) -> anyhow::Result<()> {
    let payload = serde_json::json!({ "bodyMarkdown": "# Edited" });
    let response = router
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::PATCH)
                .uri(format!("/documents/{slug}"))
                .header("content-type", "application/json")
                .header("x-api-key", key)
                .body(Body::from(serde_json::to_vec(&payload)?))?,
        )
        .await?;
    assert_eq!(response.status(), StatusCode::OK);
    Ok(())
}

async fn publish_document(router: &axum::Router, slug: &str, key: &str) -> anyhow::Result<()> {
    let response = router
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri(format!("/documents/{slug}/publish"))
                .header("x-api-key", key)
                .body(Body::empty())?,
        )
        .await?;
    assert_eq!(response.status(), StatusCode::OK);
    Ok(())
}

async fn get_json(
    router: &axum::Router,
    uri: &str,
    key: Option<&str>,
) -> anyhow::Result<(StatusCode, serde_json::Value)> {
    let mut builder = Request::builder().method(Method::GET).uri(uri);
    if let Some(key) = key {
        builder = builder.header("x-api-key", key);
    }
    let response = router.clone().oneshot(builder.body(Body::empty())?).await?;
    let status = response.status();
    let json = body_json(response).await.unwrap_or(serde_json::Value::Null);
    Ok((status, json))
}

#[tokio::test]
async fn admin_reads_document_history_newest_first() -> anyhow::Result<()> {
    let _guard = db_guard().await;
    let Some(pool) = common::maybe_pool().await? else {
        return Ok(());
    };
    let router = common::router_for(pool.clone());

    let slug = create_document(&router, "Audit History Admin", "# First", SHARED_KEY).await?;
    update_document(&router, &slug, SHARED_KEY).await?;
    publish_document(&router, &slug, SHARED_KEY).await?;

    let (status, body) = get_json(
        &router,
        &format!("/documents/{slug}/history"),
        Some(SHARED_KEY),
    )
    .await?;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["slug"], slug);
    let history = body["history"].as_array().expect("history array");
    assert!(
        history.len() >= 3,
        "history should include create, update, publish: {history:?}"
    );
    let actions: Vec<&str> = history
        .iter()
        .take(3)
        .filter_map(|entry| entry["action"].as_str())
        .collect();
    assert_eq!(actions, ["publish", "update", "create"]);
    assert_eq!(history[0]["actorLabel"], "shared-key");
    assert!(
        history[0]["at"].as_str().is_some(),
        "timestamp is serialized"
    );

    let unrelated = mint_token(&router, "Unrelated", &["read"]).await?;
    let (status, _) = get_json(
        &router,
        &format!("/documents/{slug}/history"),
        Some(&unrelated),
    )
    .await?;
    assert_eq!(
        status,
        StatusCode::NOT_FOUND,
        "published history is still owner/admin only"
    );

    Ok(())
}

#[tokio::test]
async fn owner_reads_own_history() -> anyhow::Result<()> {
    let _guard = db_guard().await;
    let Some(pool) = common::maybe_pool().await? else {
        return Ok(());
    };
    let router = common::router_for(pool.clone());

    let owner = mint_token(&router, "Owner", &["read", "write", "publish"]).await?;
    let slug = create_document(&router, "Audit History Owner", "# First", &owner).await?;
    update_document(&router, &slug, &owner).await?;

    let (status, body) = get_json(
        &router,
        &format!("/documents/{slug}/history?limit=1"),
        Some(&owner),
    )
    .await?;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["slug"], slug);
    let history = body["history"].as_array().expect("history array");
    assert_eq!(history.len(), 1);
    assert_eq!(history[0]["action"], "update");
    assert_eq!(history[0]["actorLabel"], "Owner");

    Ok(())
}
