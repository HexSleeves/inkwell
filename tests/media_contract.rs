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
use inkwell::config::MediaBackend;
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

/// `POST /media` with an explicit declared type, body, and credential.
async fn post_media(
    router: &axum::Router,
    content_type: &str,
    body: Vec<u8>,
    key: &str,
) -> anyhow::Result<axum::response::Response> {
    Ok(router
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/media")
                .header("content-type", content_type)
                .header("x-api-key", key)
                .body(Body::from(body))?,
        )
        .await?)
}

/// Upload a PNG and return its `/media/{id}` URL.
async fn upload_png(router: &axum::Router, png: &[u8], key: &str) -> anyhow::Result<String> {
    let response = post_media(router, "image/png", png.to_vec(), key).await?;
    assert_eq!(response.status(), StatusCode::CREATED, "upload should 201");
    Ok(body_json(response).await?["url"]
        .as_str()
        .expect("url in response")
        .to_string())
}

async fn get(
    router: &axum::Router,
    uri: &str,
    headers: &[(&str, &str)],
) -> anyhow::Result<axum::response::Response> {
    let mut request = Request::builder().method(Method::GET).uri(uri);
    for (name, value) in headers {
        request = request.header(*name, *value);
    }
    Ok(router.clone().oneshot(request.body(Body::empty())?).await?)
}

async fn delete_media(
    router: &axum::Router,
    uri: &str,
    key: Option<&str>,
) -> anyhow::Result<StatusCode> {
    let mut request = Request::builder().method(Method::DELETE).uri(uri);
    if let Some(key) = key {
        request = request.header("x-api-key", key);
    }
    Ok(router
        .clone()
        .oneshot(request.body(Body::empty())?)
        .await?
        .status())
}

/// Mint a scoped token (creating the named author) via the admin surface.
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
    assert_eq!(
        response.status(),
        StatusCode::CREATED,
        "mint should succeed"
    );
    Ok(body_json(response).await?["token"]
        .as_str()
        .expect("token in response")
        .to_string())
}

fn header_value(response: &axum::response::Response, name: &str) -> String {
    response
        .headers()
        .get(name)
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default()
        .to_string()
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
                .body(Body::from(tiny_png()))?,
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

/// Serving carries an explicit `Content-Length` and a strong `ETag` (the content
/// SHA-256), and a conditional re-request is answered 304 with no body.
#[tokio::test]
async fn serve_sets_content_length_and_etag_and_answers_304() -> anyhow::Result<()> {
    let _guard = db_guard().await;
    let Some(pool) = common::maybe_pool().await? else {
        return Ok(());
    };
    let router = common::router_for(pool);
    let png = tiny_png();
    let url = upload_png(&router, &png, SHARED_KEY).await?;

    let response = get(&router, &url, &[]).await?;
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        header_value(&response, "content-length"),
        png.len().to_string(),
        "content-length must be the exact blob size"
    );
    let etag = header_value(&response, "etag");
    assert_eq!(
        etag,
        format!("\"{}\"", inkwell::media::checksum_hex(&png)),
        "etag must be the strong content sha256"
    );

    // Same etag back → 304, no body, cache directive preserved.
    let conditional = get(&router, &url, &[("if-none-match", etag.as_str())]).await?;
    assert_eq!(conditional.status(), StatusCode::NOT_MODIFIED);
    assert_eq!(header_value(&conditional, "etag"), etag);
    assert!(
        header_value(&conditional, "cache-control").contains("immutable"),
        "304 should still advertise the immutable cache policy"
    );
    let body = to_bytes(conditional.into_body(), usize::MAX).await?;
    assert!(body.is_empty(), "304 must not carry a body");

    // A stale etag re-sends the bytes.
    let stale = get(&router, &url, &[("if-none-match", "\"stale\"")]).await?;
    assert_eq!(stale.status(), StatusCode::OK);
    Ok(())
}

/// The declared `Content-Type` must agree with the sniffed magic bytes: neither
/// active content dressed as an image nor an image mislabelled as another image
/// is accepted.
#[tokio::test]
async fn declared_type_must_match_sniffed_bytes() -> anyhow::Result<()> {
    let _guard = db_guard().await;
    let Some(pool) = common::maybe_pool().await? else {
        return Ok(());
    };
    let router = common::router_for(pool);

    // HTML declared as PNG — the stored-XSS shape this check exists for.
    let response = post_media(
        &router,
        "image/png",
        b"<html><script>alert(1)</script></html>".to_vec(),
        SHARED_KEY,
    )
    .await?;
    assert_eq!(
        response.status(),
        StatusCode::BAD_REQUEST,
        "HTML declared as image/png must be rejected"
    );

    // A real PNG declared as JPEG — allowlisted type, wrong bytes.
    let response = post_media(&router, "image/jpeg", tiny_png(), SHARED_KEY).await?;
    assert_eq!(
        response.status(),
        StatusCode::BAD_REQUEST,
        "declared/sniffed mismatch must be rejected"
    );
    let message = body_json(response).await?["error"]["message"]
        .as_str()
        .unwrap_or_default()
        .to_string();
    assert!(
        message.contains("does not match"),
        "error should explain the mismatch, got {message:?}"
    );

    // Truncated/garbage bytes with an allowlisted declaration are rejected too.
    let response = post_media(&router, "image/png", b"\x89PNGnope".to_vec(), SHARED_KEY).await?;
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    Ok(())
}

/// Re-uploading identical bytes returns the existing row (200) instead of
/// creating a second row over the same content-addressed blob.
#[tokio::test]
async fn reuploading_identical_bytes_reuses_the_existing_row() -> anyhow::Result<()> {
    let _guard = db_guard().await;
    let Some(pool) = common::maybe_pool().await? else {
        return Ok(());
    };
    let router = common::router_for(pool.clone());
    let png = tiny_png();

    let first = post_media(&router, "image/png", png.clone(), SHARED_KEY).await?;
    assert_eq!(first.status(), StatusCode::CREATED);
    let first_id = body_json(first).await?["id"]
        .as_str()
        .expect("id")
        .to_string();

    let second = post_media(&router, "image/png", png.clone(), SHARED_KEY).await?;
    assert_eq!(
        second.status(),
        StatusCode::OK,
        "a duplicate upload is not a new creation"
    );
    let second_id = body_json(second).await?["id"]
        .as_str()
        .expect("id")
        .to_string();
    assert_eq!(first_id, second_id, "duplicate upload must reuse the row");

    let rows: i64 = sqlx::query_scalar("SELECT count(*) FROM media")
        .fetch_one(&pool)
        .await?;
    assert_eq!(rows, 1, "only one row should exist for one blob");
    Ok(())
}

/// The local backend writes the blob at its content-addressed, sharded path —
/// derived only from the digest, so nothing client-supplied reaches the path.
#[tokio::test]
async fn local_backend_writes_a_content_addressed_file() -> anyhow::Result<()> {
    let _guard = db_guard().await;
    let Some(pool) = common::maybe_pool().await? else {
        return Ok(());
    };
    let media_dir = common::media_test_dir();
    let router = common::router_for_with_media_dir(pool, &media_dir);
    let png = tiny_png();

    let url = upload_png(&router, &png, SHARED_KEY).await?;
    let checksum = inkwell::media::checksum_hex(&png);
    let key = inkwell::media::storage_key_for(&checksum, "image/png").expect("key");
    let path = media_dir.join(&key);
    assert!(
        path.is_file(),
        "blob should be stored at {key} under the media dir"
    );
    assert_eq!(std::fs::read(&path)?, png, "stored bytes must be verbatim");
    // And it serves from there.
    assert_eq!(get(&router, &url, &[]).await?.status(), StatusCode::OK);

    let _ = std::fs::remove_dir_all(&media_dir);
    Ok(())
}

/// The Postgres backend satisfies the same upload/serve contract with the same
/// URLs — the storage choice is invisible to clients.
#[tokio::test]
async fn postgres_backend_round_trips_through_the_same_api() -> anyhow::Result<()> {
    let _guard = db_guard().await;
    let Some(pool) = common::maybe_pool().await? else {
        return Ok(());
    };
    let router = common::router_for_with_media_backend(pool.clone(), MediaBackend::Postgres);
    let png = tiny_png();

    let url = upload_png(&router, &png, SHARED_KEY).await?;
    let response = get(&router, &url, &[]).await?;
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(header_value(&response, "content-type"), "image/png");
    assert_eq!(
        to_bytes(response.into_body(), usize::MAX).await?.as_ref(),
        png.as_slice()
    );

    let blobs: i64 = sqlx::query_scalar("SELECT count(*) FROM media_blobs")
        .fetch_one(&pool)
        .await?;
    assert_eq!(blobs, 1, "bytes should live in media_blobs");
    let backend: String = sqlx::query_scalar("SELECT storage_backend FROM media LIMIT 1")
        .fetch_one(&pool)
        .await?;
    assert_eq!(backend, "postgres", "row records where its bytes went");
    Ok(())
}

/// Deleting removes the row and the (now unreferenced) blob; the URL then 404s.
#[tokio::test]
async fn delete_removes_the_row_and_the_blob() -> anyhow::Result<()> {
    let _guard = db_guard().await;
    let Some(pool) = common::maybe_pool().await? else {
        return Ok(());
    };
    let media_dir = common::media_test_dir();
    let router = common::router_for_with_media_dir(pool, &media_dir);
    let png = tiny_png();
    let url = upload_png(&router, &png, SHARED_KEY).await?;
    let key = inkwell::media::storage_key_for(&inkwell::media::checksum_hex(&png), "image/png")
        .expect("key");

    let response = router
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::DELETE)
                .uri(&url)
                .header("x-api-key", SHARED_KEY)
                .body(Body::empty())?,
        )
        .await?;
    assert_eq!(
        response.status(),
        StatusCode::NO_CONTENT,
        "delete returns 204"
    );

    assert_eq!(
        get(&router, &url, &[]).await?.status(),
        StatusCode::NOT_FOUND,
        "deleted media must 404"
    );
    assert!(
        !media_dir.join(&key).exists(),
        "unreferenced blob should be removed from disk"
    );

    // Deleting again reports the row is gone.
    let repeat = router
        .oneshot(
            Request::builder()
                .method(Method::DELETE)
                .uri(&url)
                .header("x-api-key", SHARED_KEY)
                .body(Body::empty())?,
        )
        .await?;
    assert_eq!(repeat.status(), StatusCode::NOT_FOUND);

    let _ = std::fs::remove_dir_all(&media_dir);
    Ok(())
}

/// Delete is credentialled, needs `write`, and is owner-scoped.
#[tokio::test]
async fn delete_requires_auth_write_scope_and_ownership() -> anyhow::Result<()> {
    let _guard = db_guard().await;
    let Some(pool) = common::maybe_pool().await? else {
        return Ok(());
    };
    let router = common::router_for(pool);
    let owner_token = mint_token(&router, "media-owner", &["read", "write"]).await?;
    let other_token = mint_token(&router, "media-stranger", &["read", "write"]).await?;
    let reader_token = mint_token(&router, "media-reader", &["read"]).await?;

    let url = upload_png(&router, &tiny_png(), &owner_token).await?;

    // Anonymous.
    assert_eq!(
        delete_media(&router, &url, None).await?,
        StatusCode::UNAUTHORIZED
    );
    // Authenticated but without `write`.
    assert_eq!(
        delete_media(&router, &url, Some(&reader_token)).await?,
        StatusCode::FORBIDDEN
    );
    // A different author's `write` token cannot delete someone else's upload.
    assert_eq!(
        delete_media(&router, &url, Some(&other_token)).await?,
        StatusCode::FORBIDDEN
    );
    // The owner can.
    assert_eq!(
        delete_media(&router, &url, Some(&owner_token)).await?,
        StatusCode::NO_CONTENT
    );
    Ok(())
}

/// A read-only token cannot upload (the `write` scope is required).
#[tokio::test]
async fn upload_requires_the_write_scope() -> anyhow::Result<()> {
    let _guard = db_guard().await;
    let Some(pool) = common::maybe_pool().await? else {
        return Ok(());
    };
    let router = common::router_for(pool);
    let reader_token = mint_token(&router, "media-read-only", &["read"]).await?;

    let response = post_media(&router, "image/png", tiny_png(), &reader_token).await?;
    assert_eq!(response.status(), StatusCode::FORBIDDEN);
    Ok(())
}

/// The configured cap (`INKWELL_MEDIA_MAX_BYTES`) is what is enforced, not a
/// hard-coded 5 MiB.
#[tokio::test]
async fn configured_size_cap_is_enforced() -> anyhow::Result<()> {
    let _guard = db_guard().await;
    let Some(pool) = common::maybe_pool().await? else {
        return Ok(());
    };
    // Cap just under the size of a padded PNG so the request is legal in every
    // other respect and only the cap can reject it.
    let mut padded = tiny_png();
    padded.extend(std::iter::repeat_n(0u8, 4096));
    let router = common::router_for_with_media_max_bytes(pool, padded.len() - 1);

    let response = post_media(&router, "image/png", padded.clone(), SHARED_KEY).await?;
    assert_eq!(
        response.status(),
        StatusCode::PAYLOAD_TOO_LARGE,
        "over the configured cap must be 413"
    );

    // One byte under the cap is accepted.
    padded.truncate(padded.len() - 2);
    let response = post_media(&router, "image/png", padded, SHARED_KEY).await?;
    assert_eq!(response.status(), StatusCode::CREATED);
    Ok(())
}

/// Path-traversal payloads in the serve route resolve to nothing: the route takes
/// a uuid, and stored keys are derived from the content digest alone.
#[tokio::test]
async fn traversal_payloads_never_reach_the_store() -> anyhow::Result<()> {
    let _guard = db_guard().await;
    let Some(pool) = common::maybe_pool().await? else {
        return Ok(());
    };
    let router = common::router_for(pool);

    for uri in [
        "/media/..%2f..%2fetc%2fpasswd",
        "/media/....//....//etc/passwd",
        "/media/not-a-uuid",
    ] {
        let status = get(&router, uri, &[]).await?.status();
        assert!(
            status == StatusCode::BAD_REQUEST || status == StatusCode::NOT_FOUND,
            "{uri} must not resolve to content (got {status})"
        );
    }
    Ok(())
}

/// The acceptance path end to end: upload an image, reference it in a document
/// body, publish, and see the `<img>` rendered on the public page — then fetch
/// that image URL anonymously, as a reader's browser would.
#[tokio::test]
async fn uploaded_image_renders_on_the_public_page() -> anyhow::Result<()> {
    let _guard = db_guard().await;
    let Some(pool) = common::maybe_pool().await? else {
        return Ok(());
    };
    let media_dir = common::media_test_dir();
    let router = common::router_for_with_media_dir(pool, &media_dir);

    // 1. Upload, exactly as the editor's image control does.
    let url = upload_png(&router, &tiny_png(), SHARED_KEY).await?;

    // 2. Create a document whose body embeds the returned URL.
    let payload = serde_json::json!({
        "title": "Note with an image",
        "bodyMarkdown": format!("Here it is:\n\n![A tiny red dot]({url})\n"),
    });
    let created = router
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/documents")
                .header("content-type", "application/json")
                .header("x-api-key", SHARED_KEY)
                .body(Body::from(serde_json::to_vec(&payload)?))?,
        )
        .await?;
    assert_eq!(created.status(), StatusCode::CREATED);
    let slug = body_json(created).await?["slug"]
        .as_str()
        .expect("slug")
        .to_string();

    // 3. Publish it.
    let published = router
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri(format!("/documents/{slug}/publish"))
                .header("x-api-key", SHARED_KEY)
                .body(Body::empty())?,
        )
        .await?;
    assert_eq!(published.status(), StatusCode::OK);

    // 4. The public page carries the image, surviving HTML sanitisation.
    let page = get(&router, &format!("/{slug}"), &[]).await?;
    assert_eq!(page.status(), StatusCode::OK);
    let html = String::from_utf8(to_bytes(page.into_body(), usize::MAX).await?.to_vec())?;
    assert!(
        html.contains(&format!("src=\"{url}\"")),
        "public page should embed the uploaded image, got: {}",
        &html[..html.len().min(400)]
    );
    assert!(html.contains("A tiny red dot"), "alt text should survive");

    // 5. An anonymous reader can fetch the image itself.
    let image = get(&router, &url, &[]).await?;
    assert_eq!(image.status(), StatusCode::OK);
    assert_eq!(header_value(&image, "content-type"), "image/png");

    let _ = std::fs::remove_dir_all(&media_dir);
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
