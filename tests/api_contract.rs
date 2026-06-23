mod common;

use axum::body::{Body, to_bytes};
use http::{Method, Request, StatusCode};
use tower::ServiceExt;

#[tokio::test]
async fn create_and_fetch_document() -> anyhow::Result<()> {
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
async fn graph_route_hides_drafts_from_public_callers() -> anyhow::Result<()> {
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
