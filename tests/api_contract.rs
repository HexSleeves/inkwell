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
