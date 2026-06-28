mod common;

use axum::body::{Body, to_bytes};
use http::{Method, Request, StatusCode};
use serde_json::json;
use std::sync::LazyLock;
use tokio::sync::{Mutex, MutexGuard};
use tower::ServiceExt;

const SHARED_KEY: &str = "test-secret-key";

/// These tests share one database and `maybe_pool` truncates it on entry, so
/// they must not run concurrently (libtest runs a binary's tests on parallel
/// threads). Hold this lock for the whole test to serialize them. Cargo already
/// runs separate test binaries sequentially, so this is sufficient.
static DB_TEST_LOCK: LazyLock<Mutex<()>> = LazyLock::new(|| Mutex::new(()));

async fn db_guard() -> MutexGuard<'static, ()> {
    DB_TEST_LOCK.lock().await
}

async fn post_document(
    router: axum::Router,
    payload: serde_json::Value,
) -> anyhow::Result<http::Response<Body>> {
    Ok(router
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/documents")
                .header("content-type", "application/json")
                .header("x-api-key", SHARED_KEY)
                .body(Body::from(payload.to_string()))?,
        )
        .await?)
}

#[tokio::test]
async fn create_and_fetch_document() -> anyhow::Result<()> {
    let _guard = db_guard().await;
    let Some(router) = common::maybe_router().await? else {
        return Ok(());
    };

    let response = router
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/documents")
                .header("content-type", "application/json")
                .header("x-api-key", "test-secret-key")
                .body(Body::from(
                    r##"{"title":"Hello World","bodyMarkdown":"# Hi"}"##,
                ))?,
        )
        .await?;
    assert_eq!(response.status(), StatusCode::CREATED);

    let body = to_bytes(response.into_body(), usize::MAX).await?;
    let json: serde_json::Value = serde_json::from_slice(&body)?;
    assert_eq!(json["slug"], "hello-world");

    let response = router
        .oneshot(
            Request::builder()
                .method(Method::GET)
                .uri("/documents/hello-world")
                .header("x-api-key", "test-secret-key")
                .body(Body::empty())?,
        )
        .await?;
    assert_eq!(response.status(), StatusCode::OK);
    Ok(())
}

#[tokio::test]
async fn create_exposes_growth_in_the_envelope_defaulting_to_seedling() -> anyhow::Result<()> {
    let _guard = db_guard().await;
    let Some(router) = common::maybe_router().await? else {
        return Ok(());
    };

    // No explicit growth → the envelope reports the default stage.
    let response = router
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/documents")
                .header("content-type", "application/json")
                .header("x-api-key", "test-secret-key")
                .body(Body::from(
                    r##"{"title":"Seed Note","bodyMarkdown":"# Hi"}"##,
                ))?,
        )
        .await?;
    assert_eq!(response.status(), StatusCode::CREATED);
    let body = to_bytes(response.into_body(), usize::MAX).await?;
    let json: serde_json::Value = serde_json::from_slice(&body)?;
    assert_eq!(json["growth"], "seedling");

    // Explicit growth on create round-trips through the envelope.
    let response = router
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/documents")
                .header("content-type", "application/json")
                .header("x-api-key", "test-secret-key")
                .body(Body::from(
                    r##"{"title":"Grown Note","bodyMarkdown":"# Hi","growth":"evergreen"}"##,
                ))?,
        )
        .await?;
    assert_eq!(response.status(), StatusCode::CREATED);
    let body = to_bytes(response.into_body(), usize::MAX).await?;
    let json: serde_json::Value = serde_json::from_slice(&body)?;
    assert_eq!(json["growth"], "evergreen");

    Ok(())
}

#[tokio::test]
async fn create_rejects_duplicate_slug_with_conflict() -> anyhow::Result<()> {
    let _guard = db_guard().await;
    let Some(router) = common::maybe_router().await? else {
        return Ok(());
    };

    let first = post_document(
        router.clone(),
        json!({"title": "First", "slug": "dupe", "bodyMarkdown": "x"}),
    )
    .await?;
    assert_eq!(first.status(), StatusCode::CREATED);

    let duplicate = post_document(
        router,
        json!({"title": "Second", "slug": "dupe", "bodyMarkdown": "y"}),
    )
    .await?;
    assert_eq!(duplicate.status(), StatusCode::CONFLICT);

    Ok(())
}

#[tokio::test]
async fn create_rejects_oversize_title_with_bad_request() -> anyhow::Result<()> {
    let _guard = db_guard().await;
    let Some(router) = common::maybe_router().await? else {
        return Ok(());
    };

    let response = post_document(
        router,
        json!({"title": "a".repeat(501), "bodyMarkdown": "x"}),
    )
    .await?;

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    Ok(())
}

#[tokio::test]
async fn create_rejects_oversize_body_with_bad_request() -> anyhow::Result<()> {
    let _guard = db_guard().await;
    let Some(router) = common::maybe_router().await? else {
        return Ok(());
    };

    let response = post_document(
        router,
        json!({"title": "Long Body", "bodyMarkdown": "a".repeat(300_000)}),
    )
    .await?;

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    Ok(())
}

#[tokio::test]
async fn create_rejects_invalid_growth_with_bad_request() -> anyhow::Result<()> {
    let _guard = db_guard().await;
    let Some(router) = common::maybe_router().await? else {
        return Ok(());
    };

    let response = post_document(
        router,
        json!({"title": "G", "bodyMarkdown": "x", "growth": "cursed"}),
    )
    .await?;

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    Ok(())
}

#[tokio::test]
async fn create_rejects_whitespace_slug_with_bad_request() -> anyhow::Result<()> {
    let _guard = db_guard().await;
    let Some(router) = common::maybe_router().await? else {
        return Ok(());
    };

    let response = post_document(
        router,
        json!({"title": "E", "slug": "   ", "bodyMarkdown": "x"}),
    )
    .await?;

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    Ok(())
}

#[tokio::test]
async fn graph_route_hides_drafts_from_public_callers() -> anyhow::Result<()> {
    let _guard = db_guard().await;
    let Some(router) = common::maybe_router().await? else {
        return Ok(());
    };

    // Create a published note and a draft note.
    for (title, published) in [("Public Note", true), ("Draft Note", false)] {
        let response = router
            .clone()
            .oneshot(
                Request::builder()
                    .method(Method::POST)
                    .uri("/documents")
                    .header("content-type", "application/json")
                    .header("x-api-key", "test-secret-key")
                    .body(Body::from(format!(
                        r##"{{"title":"{title}","bodyMarkdown":"# {title}"}}"##
                    )))?,
            )
            .await?;
        assert_eq!(response.status(), StatusCode::CREATED);
        if published {
            let response = router
                .clone()
                .oneshot(
                    Request::builder()
                        .method(Method::POST)
                        .uri("/documents/public-note/publish")
                        .header("x-api-key", "test-secret-key")
                        .body(Body::empty())?,
                )
                .await?;
            assert_eq!(response.status(), StatusCode::OK);
        }
    }

    // Anonymous GET /graph: only the published node, never the draft.
    let response = router
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::GET)
                .uri("/graph")
                .body(Body::empty())?,
        )
        .await?;
    assert_eq!(response.status(), StatusCode::OK);
    let body = to_bytes(response.into_body(), usize::MAX).await?;
    let json: serde_json::Value = serde_json::from_slice(&body)?;
    let slugs: Vec<&str> = json["nodes"]
        .as_array()
        .unwrap()
        .iter()
        .map(|n| n["slug"].as_str().unwrap())
        .collect();
    assert!(slugs.contains(&"public-note"), "published node is present");
    assert!(
        !slugs.contains(&"draft-note"),
        "a draft node must never appear in the public graph"
    );

    // Authenticated GET /graph: the draft node is visible to the owner.
    let response = router
        .oneshot(
            Request::builder()
                .method(Method::GET)
                .uri("/graph")
                .header("x-api-key", "test-secret-key")
                .body(Body::empty())?,
        )
        .await?;
    assert_eq!(response.status(), StatusCode::OK);
    let body = to_bytes(response.into_body(), usize::MAX).await?;
    let json: serde_json::Value = serde_json::from_slice(&body)?;
    let slugs: Vec<&str> = json["nodes"]
        .as_array()
        .unwrap()
        .iter()
        .map(|n| n["slug"].as_str().unwrap())
        .collect();
    assert!(
        slugs.contains(&"draft-note"),
        "owner visibility includes the draft node"
    );

    Ok(())
}
