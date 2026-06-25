//! Database-backed contract tests for media upload and serving
//! (`POST /media`, `GET /media/{id}`).
//!
//! Tests:
//! - Anonymous upload → 401
//! - Authed PNG upload → 201 with `{ id, url }`; GET returns 200, correct
//!   content-type, bytes round-trip intact.
//! - Oversized body → 413.
//! - Disallowed content-type (e.g. `text/plain`) → 400.
//! - GET unknown id → 404.
//!
//! Skipped unless `DATABASE_URL` is set (or forced via `INKWELL_REQUIRE_DB_TESTS=1`).

mod common;

use axum::body::{Body, to_bytes};
use http::{Method, Request, StatusCode};
use std::sync::LazyLock;
use tokio::sync::{Mutex, MutexGuard};
use tower::ServiceExt;

/// Serialise db-backed tests in this binary; `maybe_pool` truncates on entry.
static DB_TEST_LOCK: LazyLock<Mutex<()>> = LazyLock::new(|| Mutex::new(()));

async fn db_guard() -> MutexGuard<'static, ()> {
    DB_TEST_LOCK.lock().await
}

const SHARED_KEY: &str = "test-secret-key";

/// Parse a response body as a JSON value.
async fn body_json(response: axum::response::Response) -> anyhow::Result<serde_json::Value> {
    let bytes = to_bytes(response.into_body(), usize::MAX).await?;
    Ok(serde_json::from_slice(&bytes)?)
}

/// Minimal PNG (8×8 px, 1-byte palette, valid IDAT). Enough to round-trip
/// as raw bytes without pulling in an image-encode crate.
fn tiny_png() -> Vec<u8> {
    // A well-formed 1×1 red PNG (67 bytes), commonly used in contract tests.
    vec![
        0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a, // PNG signature
        0x00, 0x00, 0x00, 0x0d, 0x49, 0x48, 0x44, 0x52, // IHDR length + type
        0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01, // 1×1
        0x08, 0x02, 0x00, 0x00, 0x00, 0x90, 0x77, 0x53, // bit depth, colour type, crc
        0xde, 0x00, 0x00, 0x00, 0x0c, 0x49, 0x44, 0x41, // IDAT length + type
        0x54, 0x08, 0xd7, 0x63, 0xf8, 0xcf, 0xc0, 0x00, // IDAT data
        0x00, 0x00, 0x02, 0x00, 0x01, 0xe2, 0x21, 0xbc, // IDAT data cont.
        0x33, 0x00, 0x00, 0x00, 0x00, 0x49, 0x45, 0x4e, // IEND length + type
        0x44, 0xae, 0x42, 0x60, 0x82, // IEND data + crc
    ]
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

/// An anonymous (unauthenticated) upload is rejected 401.
#[tokio::test]
async fn anonymous_upload_is_unauthorized() -> anyhow::Result<()> {
    let _guard = db_guard().await;
    let Some(pool) = common::maybe_pool().await? else {
        return Ok(());
    };
    let router = common::router_for(pool);

    let response = router
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/media")
                .header("content-type", "image/png")
                .body(Body::from(tiny_png()))?,
        )
        .await?;

    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    Ok(())
}

/// An authenticated upload of a PNG returns 201 with `{ id, url }`.
/// Fetching the returned URL returns 200 with the correct content-type
/// and the exact bytes that were uploaded (round-trip).
#[tokio::test]
async fn authed_png_upload_round_trips() -> anyhow::Result<()> {
    let _guard = db_guard().await;
    let Some(pool) = common::maybe_pool().await? else {
        return Ok(());
    };
    let router = common::router_for(pool);
    let png_bytes = tiny_png();

    // Upload.
    let response = router
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/media")
                .header("content-type", "image/png")
                .header("x-api-key", SHARED_KEY)
                .body(Body::from(png_bytes.clone()))?,
        )
        .await?;

    assert_eq!(
        response.status(),
        StatusCode::CREATED,
        "upload should return 201"
    );
    let json = body_json(response).await?;
    let id = json["id"].as_str().expect("response carries id");
    let url = json["url"].as_str().expect("response carries url");
    assert!(
        url.starts_with("/media/"),
        "url should be /media/<id>, got {url}"
    );
    assert!(url.ends_with(id), "url should contain the id");

    // Serve — GET the returned URL.
    let get_response = router
        .oneshot(
            Request::builder()
                .method(Method::GET)
                .uri(url)
                .body(Body::empty())?,
        )
        .await?;

    assert_eq!(
        get_response.status(),
        StatusCode::OK,
        "GET /media/<id> should return 200"
    );

    let ct = get_response
        .headers()
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    assert_eq!(ct, "image/png", "content-type should match the upload");

    let cache = get_response
        .headers()
        .get("cache-control")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    assert!(
        cache.contains("immutable"),
        "cache-control should include immutable"
    );

    let nosniff = get_response
        .headers()
        .get("x-content-type-options")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    assert_eq!(
        nosniff, "nosniff",
        "served media must carry X-Content-Type-Options: nosniff"
    );

    let body_bytes = to_bytes(get_response.into_body(), usize::MAX).await?;
    assert_eq!(
        body_bytes.as_ref(),
        png_bytes.as_slice(),
        "body bytes must round-trip exactly"
    );

    Ok(())
}

/// `HEAD /media/{id}` returns the same headers as GET with no body (axum
/// answers HEAD automatically because the route is registered with `get(...)`).
#[tokio::test]
async fn head_media_returns_headers_without_body() -> anyhow::Result<()> {
    let _guard = db_guard().await;
    let Some(pool) = common::maybe_pool().await? else {
        return Ok(());
    };
    let router = common::router_for(pool);

    let upload = router
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/media")
                .header("content-type", "image/png")
                .header("x-api-key", SHARED_KEY)
                .body(Body::from(vec![1u8, 2, 3, 4]))?,
        )
        .await?;
    assert_eq!(upload.status(), StatusCode::CREATED);
    let url = body_json(upload).await?["url"]
        .as_str()
        .expect("url")
        .to_string();

    let head = router
        .oneshot(
            Request::builder()
                .method(Method::HEAD)
                .uri(&url)
                .body(Body::empty())?,
        )
        .await?;
    assert_eq!(head.status(), StatusCode::OK, "HEAD should 200");
    assert_eq!(
        head.headers()
            .get("content-type")
            .and_then(|v| v.to_str().ok()),
        Some("image/png")
    );
    let body = to_bytes(head.into_body(), usize::MAX).await?;
    assert!(body.is_empty(), "HEAD must not return a body");
    Ok(())
}

/// An oversized request body returns 413.
#[tokio::test]
async fn oversized_upload_is_413() -> anyhow::Result<()> {
    let _guard = db_guard().await;
    let Some(pool) = common::maybe_pool().await? else {
        return Ok(());
    };
    let router = common::router_for(pool);

    // 5 MiB + 1 byte — just over the cap.
    let big: Vec<u8> = vec![0u8; 5 * 1024 * 1024 + 1];

    let response = router
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/media")
                .header("content-type", "image/png")
                .header("x-api-key", SHARED_KEY)
                .body(Body::from(big))?,
        )
        .await?;

    assert_eq!(
        response.status(),
        StatusCode::PAYLOAD_TOO_LARGE,
        "oversized upload should return 413"
    );
    Ok(())
}

/// A disallowed content-type (e.g. `text/plain`) returns 400.
#[tokio::test]
async fn disallowed_content_type_is_400() -> anyhow::Result<()> {
    let _guard = db_guard().await;
    let Some(pool) = common::maybe_pool().await? else {
        return Ok(());
    };
    let router = common::router_for(pool);

    let response = router
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/media")
                .header("content-type", "text/plain")
                .header("x-api-key", SHARED_KEY)
                .body(Body::from(b"hello".as_ref()))?,
        )
        .await?;

    assert_eq!(
        response.status(),
        StatusCode::BAD_REQUEST,
        "text/plain should be rejected 400"
    );

    // SVG is also excluded (script injection risk).
    let response = router
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/media")
                .header("content-type", "image/svg+xml")
                .header("x-api-key", SHARED_KEY)
                .body(Body::from(b"<svg/>".as_ref()))?,
        )
        .await?;

    assert_eq!(
        response.status(),
        StatusCode::BAD_REQUEST,
        "image/svg+xml should be rejected 400 (SVG excluded for v1)"
    );
    Ok(())
}

/// GET with an unknown id returns 404.
#[tokio::test]
async fn get_unknown_media_is_404() -> anyhow::Result<()> {
    let _guard = db_guard().await;
    let Some(pool) = common::maybe_pool().await? else {
        return Ok(());
    };
    let router = common::router_for(pool);

    let response = router
        .oneshot(
            Request::builder()
                .method(Method::GET)
                .uri("/media/00000000-0000-0000-0000-000000000000")
                .body(Body::empty())?,
        )
        .await?;

    assert_eq!(
        response.status(),
        StatusCode::NOT_FOUND,
        "unknown media id should return 404"
    );
    Ok(())
}
