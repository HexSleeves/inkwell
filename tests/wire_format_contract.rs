//! Wire-format stability contracts (CIL-136).
//!
//! Asserts the stable observable shapes documented in `docs/COMPATIBILITY.md`:
//!   - `GET /health` response body (`{"status":"ok","db":"up"}`)
//!   - Error envelope: `{"error":{"message":"…","requestId":"…"}}`
//!   - Document JSON envelope: all stable camelCase field names present and typed
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

async fn body_json(response: axum::response::Response) -> anyhow::Result<serde_json::Value> {
    let bytes = to_bytes(response.into_body(), usize::MAX).await?;
    Ok(serde_json::from_slice(&bytes)?)
}

// ---------------------------------------------------------------------------
// Health endpoint
// ---------------------------------------------------------------------------

/// `GET /health` returns 200 with a stable body shape when the DB is up.
///
/// Stable fields (from COMPATIBILITY.md): `status` ("ok") and `db` ("up").
#[tokio::test]
async fn health_returns_stable_json_body_when_db_is_connected() -> anyhow::Result<()> {
    let _guard = db_guard().await;
    let Some(router) = common::maybe_router().await? else {
        return Ok(());
    };

    let response = router
        .oneshot(
            Request::builder()
                .method(Method::GET)
                .uri("/health")
                .body(Body::empty())?,
        )
        .await?;

    assert_eq!(
        response.status(),
        StatusCode::OK,
        "health must return 200 when database is reachable"
    );
    let json = body_json(response).await?;
    assert_eq!(
        json["status"], "ok",
        "health body must carry status:\"ok\" when up"
    );
    assert_eq!(
        json["db"], "up",
        "health body must carry db:\"up\" when reachable"
    );
    Ok(())
}

// ---------------------------------------------------------------------------
// Error envelope shape
// ---------------------------------------------------------------------------

/// Every 4xx response carries `{"error":{"message":"…","requestId":"…"}}`.
///
/// This shape is stable (COMPATIBILITY.md). `error.message` must be a
/// non-empty string; `error.requestId` must be a non-empty string matching
/// the `X-Request-Id` response header.
#[tokio::test]
async fn error_envelope_carries_message_and_request_id() -> anyhow::Result<()> {
    let _guard = db_guard().await;
    let Some(router) = common::maybe_router().await? else {
        return Ok(());
    };

    // Unauthenticated write → 401; a reliable 4xx without DB writes.
    let response = router
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/documents")
                .header("content-type", "application/json")
                .body(Body::from(r##"{"title":"x","bodyMarkdown":"x"}"##))?,
        )
        .await?;

    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);

    let header_request_id = response
        .headers()
        .get("x-request-id")
        .and_then(|v| v.to_str().ok())
        .map(str::to_string)
        .expect("every response carries X-Request-Id");

    let json = body_json(response).await?;

    // `error.message` must be a non-empty string.
    let message = json["error"]["message"]
        .as_str()
        .expect("error.message must be a string");
    assert!(!message.is_empty(), "error.message must not be empty");

    // `error.requestId` must be a non-empty string matching the header.
    let request_id = json["error"]["requestId"]
        .as_str()
        .expect("error.requestId must be a string");
    assert!(!request_id.is_empty(), "error.requestId must not be empty");
    assert_eq!(
        request_id, header_request_id,
        "error.requestId must match the X-Request-Id response header"
    );

    Ok(())
}

/// A 403 Forbidden response (insufficient scope) also carries the stable
/// error envelope with a non-empty message and request id.
#[tokio::test]
async fn error_envelope_present_on_403_insufficient_scope() -> anyhow::Result<()> {
    let _guard = db_guard().await;
    let Some(pool) = common::maybe_pool().await? else {
        return Ok(());
    };
    let router = common::router_for(pool);

    // Mint a read-only token.
    let mint_resp = router
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/admin/tokens")
                .header("content-type", "application/json")
                .header("x-api-key", "test-secret-key")
                .body(Body::from(r#"{"name":"reader","scopes":["read"]}"#))?,
        )
        .await?;
    assert_eq!(mint_resp.status(), StatusCode::CREATED);
    let token = body_json(mint_resp).await?["token"]
        .as_str()
        .expect("token in mint response")
        .to_string();

    // A read-only token attempting a write returns 403.
    let response = router
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/documents")
                .header("content-type", "application/json")
                .header("authorization", format!("Bearer {token}"))
                .body(Body::from(r##"{"title":"x","bodyMarkdown":"x"}"##))?,
        )
        .await?;

    assert_eq!(response.status(), StatusCode::FORBIDDEN);
    let json = body_json(response).await?;
    assert!(
        json["error"]["message"].is_string(),
        "error.message must be a string on 403"
    );
    assert!(
        !json["error"]["message"].as_str().unwrap().is_empty(),
        "error.message must not be empty on 403"
    );
    assert!(
        json["error"]["requestId"].is_string(),
        "error.requestId must be a string on 403"
    );
    Ok(())
}

// ---------------------------------------------------------------------------
// Document JSON envelope stable field names
// ---------------------------------------------------------------------------

/// Creating a document returns all stable camelCase fields in the envelope.
///
/// Stable fields (COMPATIBILITY.md): id, slug, title, bodyMarkdown,
/// renderedHtml, status, growth, tags, version, createdAt, updatedAt.
#[tokio::test]
async fn document_envelope_contains_all_stable_camel_case_fields() -> anyhow::Result<()> {
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
                    r##"{"title":"Wire Format Test","bodyMarkdown":"# Hi","tags":["rust"],"growth":"budding"}"##,
                ))?,
        )
        .await?;

    assert_eq!(response.status(), StatusCode::CREATED);
    let json = body_json(response).await?;

    // All stable fields must be present and correctly typed.
    assert!(
        uuid::Uuid::parse_str(json["id"].as_str().unwrap_or("")).is_ok(),
        "id must be a valid UUID string"
    );
    assert_eq!(
        json["slug"], "wire-format-test",
        "slug must be the slugified title"
    );
    assert_eq!(json["title"], "Wire Format Test");
    assert!(
        json["bodyMarkdown"].is_string(),
        "bodyMarkdown must be a string"
    );
    assert!(
        json["renderedHtml"].is_string(),
        "renderedHtml must be a string"
    );
    assert_eq!(
        json["status"], "draft",
        "new documents are created as draft"
    );
    assert_eq!(
        json["growth"], "budding",
        "explicit growth stage round-trips"
    );
    assert_eq!(
        json["tags"].as_array().unwrap(),
        &vec![serde_json::Value::String("rust".to_string())],
        "tags must be a string array"
    );
    assert!(
        json["version"].is_i64() || json["version"].is_u64(),
        "version must be an integer"
    );
    assert_eq!(json["version"], 1, "first version is 1");
    assert!(
        json["createdAt"].is_string(),
        "createdAt must be an ISO-8601 string"
    );
    assert!(
        json["updatedAt"].is_string(),
        "updatedAt must be an ISO-8601 string"
    );

    // Verify createdAt is a parseable RFC 3339 / ISO-8601 timestamp.
    let created_at_str = json["createdAt"].as_str().unwrap();
    time::OffsetDateTime::parse(
        created_at_str,
        &time::format_description::well_known::Rfc3339,
    )
    .unwrap_or_else(|_| {
        panic!("createdAt must be a valid RFC 3339 timestamp, got {created_at_str}")
    });

    Ok(())
}

/// Fetching a document via GET returns the same stable field set.
#[tokio::test]
async fn get_document_envelope_contains_all_stable_fields() -> anyhow::Result<()> {
    let _guard = db_guard().await;
    let Some(router) = common::maybe_router().await? else {
        return Ok(());
    };

    // Create.
    let create_resp = router
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/documents")
                .header("content-type", "application/json")
                .header("x-api-key", "test-secret-key")
                .body(Body::from(
                    r##"{"title":"Get Field Test","bodyMarkdown":"# Get"}"##,
                ))?,
        )
        .await?;
    assert_eq!(create_resp.status(), StatusCode::CREATED);
    let created = body_json(create_resp).await?;
    let slug = created["slug"].as_str().unwrap().to_string();

    // Fetch.
    let get_resp = router
        .oneshot(
            Request::builder()
                .method(Method::GET)
                .uri(format!("/documents/{slug}"))
                .header("x-api-key", "test-secret-key")
                .body(Body::empty())?,
        )
        .await?;
    assert_eq!(get_resp.status(), StatusCode::OK);
    let json = body_json(get_resp).await?;

    // All stable fields present on GET.
    for field in &[
        "id",
        "slug",
        "title",
        "bodyMarkdown",
        "renderedHtml",
        "status",
        "growth",
        "tags",
        "version",
        "createdAt",
        "updatedAt",
    ] {
        assert!(
            !json[field].is_null(),
            "stable field {field} must be present in GET /documents/:slug response"
        );
    }
    Ok(())
}

/// `GET /documents` returns the stable list envelope shape.
#[tokio::test]
async fn list_envelope_contains_stable_pagination_fields() -> anyhow::Result<()> {
    let _guard = db_guard().await;
    let Some(router) = common::maybe_router().await? else {
        return Ok(());
    };

    let response = router
        .oneshot(
            Request::builder()
                .method(Method::GET)
                .uri("/documents")
                .header("x-api-key", "test-secret-key")
                .body(Body::empty())?,
        )
        .await?;
    assert_eq!(response.status(), StatusCode::OK);
    let json = body_json(response).await?;

    assert!(json["documents"].is_array(), "documents must be an array");
    assert!(
        json["total"].is_i64() || json["total"].is_u64(),
        "total must be an integer"
    );
    assert!(
        json["limit"].is_i64() || json["limit"].is_u64(),
        "limit must be an integer"
    );
    assert!(
        json["offset"].is_i64() || json["offset"].is_u64(),
        "offset must be an integer"
    );
    Ok(())
}
