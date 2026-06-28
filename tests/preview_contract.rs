//! Contract tests for shareable draft preview links (CIL-129).
//!
//! Tests cover positive access, negative/failure cases, expiry, revocation,
//! auth gates on the management surface, and the guarantee that normal
//! public routes never expose drafts even when a preview token exists.
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

/// Create a draft document and return its slug.
async fn create_draft(router: &axum::Router, slug: &str) -> anyhow::Result<String> {
    let payload = serde_json::json!({
        "title": format!("Draft {slug}"),
        "bodyMarkdown": "# Draft\nThis is a draft.",
        "slug": slug,
    });
    let response = router
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
    assert_eq!(
        response.status(),
        StatusCode::CREATED,
        "create draft failed"
    );
    let json = body_json(response).await?;
    Ok(json["slug"].as_str().unwrap().to_string())
}

/// Mint a preview token for `slug` and return the full token string.
async fn mint_preview_token(router: &axum::Router, slug: &str) -> anyhow::Result<(String, String)> {
    let response = router
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri(format!("/documents/{slug}/preview-tokens"))
                .header("x-api-key", SHARED_KEY)
                .body(Body::empty())?,
        )
        .await?;
    assert_eq!(
        response.status(),
        StatusCode::CREATED,
        "mint preview token failed"
    );
    let json = body_json(response).await?;
    let token = json["token"].as_str().unwrap().to_string();
    let prefix = json["prefix"].as_str().unwrap().to_string();
    Ok((token, prefix))
}

// --------------------------------------------------------------------------
// Positive: valid token grants access to draft
// --------------------------------------------------------------------------

#[tokio::test]
async fn preview_token_grants_draft_access() -> anyhow::Result<()> {
    let _guard = db_guard().await;
    let Some(pool) = common::maybe_pool().await? else {
        return Ok(());
    };
    let router = common::router_for(pool);

    create_draft(&router, "my-draft").await?;
    let (token, _) = mint_preview_token(&router, "my-draft").await?;

    assert!(token.starts_with("pvw_"), "token must start with pvw_");

    // Anonymous GET still hides the draft.
    let anon = router
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::GET)
                .uri("/documents/my-draft")
                .body(Body::empty())?,
        )
        .await?;
    assert_eq!(anon.status(), StatusCode::NOT_FOUND);

    // Preview route with the token returns the draft.
    let preview = router
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::GET)
                .uri(format!("/documents/my-draft/preview?token={token}"))
                .body(Body::empty())?,
        )
        .await?;
    assert_eq!(preview.status(), StatusCode::OK, "preview should succeed");
    let body = body_json(preview).await?;
    assert_eq!(body["slug"], "my-draft");
    assert_eq!(body["status"], "draft");
    Ok(())
}

// --------------------------------------------------------------------------
// Negative: bad/missing/malformed tokens → 401
// --------------------------------------------------------------------------

#[tokio::test]
async fn preview_without_token_returns_401() -> anyhow::Result<()> {
    let _guard = db_guard().await;
    let Some(pool) = common::maybe_pool().await? else {
        return Ok(());
    };
    let router = common::router_for(pool);
    create_draft(&router, "tok-needed").await?;

    let resp = router
        .oneshot(
            Request::builder()
                .method(Method::GET)
                .uri("/documents/tok-needed/preview")
                .body(Body::empty())?,
        )
        .await?;
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    Ok(())
}

#[tokio::test]
async fn preview_with_malformed_token_returns_401() -> anyhow::Result<()> {
    let _guard = db_guard().await;
    let Some(pool) = common::maybe_pool().await? else {
        return Ok(());
    };
    let router = common::router_for(pool);
    create_draft(&router, "bad-tok").await?;

    let resp = router
        .oneshot(
            Request::builder()
                .method(Method::GET)
                .uri("/documents/bad-tok/preview?token=notapreviewtoken")
                .body(Body::empty())?,
        )
        .await?;
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    Ok(())
}

#[tokio::test]
async fn preview_with_nonexistent_token_returns_401() -> anyhow::Result<()> {
    let _guard = db_guard().await;
    let Some(pool) = common::maybe_pool().await? else {
        return Ok(());
    };
    let router = common::router_for(pool);
    create_draft(&router, "ghost-tok").await?;

    // Well-formed token prefix/secret, but not in the DB.
    let resp = router
        .oneshot(
            Request::builder()
                .method(Method::GET)
                .uri("/documents/ghost-tok/preview?token=pvw_abcdef123456_0000000000000000000000000000000000000000000000000000000000000000")
                .body(Body::empty())?,
        )
        .await?;
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    Ok(())
}

// --------------------------------------------------------------------------
// Cross-document: token for doc-A cannot access doc-B
// --------------------------------------------------------------------------

#[tokio::test]
async fn preview_with_wrong_slug_returns_401() -> anyhow::Result<()> {
    let _guard = db_guard().await;
    let Some(pool) = common::maybe_pool().await? else {
        return Ok(());
    };
    let router = common::router_for(pool);
    create_draft(&router, "doc-a").await?;
    create_draft(&router, "doc-b").await?;

    let (token, _) = mint_preview_token(&router, "doc-a").await?;

    // Use doc-a token on doc-b URL.
    let resp = router
        .oneshot(
            Request::builder()
                .method(Method::GET)
                .uri(format!("/documents/doc-b/preview?token={token}"))
                .body(Body::empty())?,
        )
        .await?;
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    Ok(())
}

// --------------------------------------------------------------------------
// Revocation
// --------------------------------------------------------------------------

#[tokio::test]
async fn revoked_token_returns_401() -> anyhow::Result<()> {
    let _guard = db_guard().await;
    let Some(pool) = common::maybe_pool().await? else {
        return Ok(());
    };
    let router = common::router_for(pool);
    create_draft(&router, "revoke-me").await?;
    let (token, prefix) = mint_preview_token(&router, "revoke-me").await?;

    // Works before revocation.
    let before = router
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::GET)
                .uri(format!("/documents/revoke-me/preview?token={token}"))
                .body(Body::empty())?,
        )
        .await?;
    assert_eq!(before.status(), StatusCode::OK);

    // Revoke.
    let revoke = router
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::DELETE)
                .uri(format!("/documents/revoke-me/preview-tokens/{prefix}"))
                .header("x-api-key", SHARED_KEY)
                .body(Body::empty())?,
        )
        .await?;
    assert_eq!(revoke.status(), StatusCode::OK);
    let revoke_body = body_json(revoke).await?;
    assert_eq!(revoke_body["revoked"], true);

    // No longer works after revocation.
    let after = router
        .oneshot(
            Request::builder()
                .method(Method::GET)
                .uri(format!("/documents/revoke-me/preview?token={token}"))
                .body(Body::empty())?,
        )
        .await?;
    assert_eq!(after.status(), StatusCode::UNAUTHORIZED);
    Ok(())
}

// --------------------------------------------------------------------------
// Expiry
// --------------------------------------------------------------------------

#[tokio::test]
async fn preview_token_with_past_expiry_rejected_at_creation() -> anyhow::Result<()> {
    let _guard = db_guard().await;
    let Some(pool) = common::maybe_pool().await? else {
        return Ok(());
    };
    let router = common::router_for(pool);
    create_draft(&router, "exp-past").await?;

    let payload = serde_json::json!({ "expiresAt": "2000-01-01T00:00:00Z" });
    let resp = router
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/documents/exp-past/preview-tokens")
                .header("content-type", "application/json")
                .header("x-api-key", SHARED_KEY)
                .body(Body::from(serde_json::to_vec(&payload)?))?,
        )
        .await?;
    assert_eq!(
        resp.status(),
        StatusCode::BAD_REQUEST,
        "past expiry must be rejected at creation time"
    );
    Ok(())
}

#[tokio::test]
async fn preview_token_with_future_expiry_works() -> anyhow::Result<()> {
    let _guard = db_guard().await;
    let Some(pool) = common::maybe_pool().await? else {
        return Ok(());
    };
    let router = common::router_for(pool);
    create_draft(&router, "exp-future").await?;

    let payload = serde_json::json!({ "expiresAt": "2099-01-01T00:00:00Z" });
    let mint = router
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/documents/exp-future/preview-tokens")
                .header("content-type", "application/json")
                .header("x-api-key", SHARED_KEY)
                .body(Body::from(serde_json::to_vec(&payload)?))?,
        )
        .await?;
    assert_eq!(mint.status(), StatusCode::CREATED);
    let mint_body = body_json(mint).await?;
    let token = mint_body["token"].as_str().unwrap();
    // expiresAt is round-tripped in the response.
    assert!(
        mint_body["expiresAt"].as_str().is_some(),
        "expiresAt must appear in response"
    );

    let preview = router
        .oneshot(
            Request::builder()
                .method(Method::GET)
                .uri(format!("/documents/exp-future/preview?token={token}"))
                .body(Body::empty())?,
        )
        .await?;
    assert_eq!(preview.status(), StatusCode::OK);
    Ok(())
}

#[tokio::test]
async fn preview_token_expired_after_mint_returns_401() -> anyhow::Result<()> {
    let _guard = db_guard().await;
    let Some(pool) = common::maybe_pool().await? else {
        return Ok(());
    };
    let router = common::router_for(pool.clone());
    create_draft(&router, "exp-after").await?;
    let (token, prefix) = mint_preview_token(&router, "exp-after").await?;

    let before = router
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::GET)
                .uri(format!("/documents/exp-after/preview?token={token}"))
                .body(Body::empty())?,
        )
        .await?;
    assert_eq!(before.status(), StatusCode::OK);

    sqlx::query(
        "UPDATE preview_tokens SET expires_at = now() - interval '1 hour' WHERE prefix = $1",
    )
    .bind(&prefix)
    .execute(&pool)
    .await?;

    let after = router
        .oneshot(
            Request::builder()
                .method(Method::GET)
                .uri(format!("/documents/exp-after/preview?token={token}"))
                .body(Body::empty())?,
        )
        .await?;
    assert_eq!(after.status(), StatusCode::UNAUTHORIZED);
    Ok(())
}

// --------------------------------------------------------------------------
// List tokens
// --------------------------------------------------------------------------

#[tokio::test]
async fn list_preview_tokens_reflects_minted_count() -> anyhow::Result<()> {
    let _guard = db_guard().await;
    let Some(pool) = common::maybe_pool().await? else {
        return Ok(());
    };
    let router = common::router_for(pool);
    create_draft(&router, "list-me").await?;

    // Initially empty.
    let empty = router
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::GET)
                .uri("/documents/list-me/preview-tokens")
                .header("x-api-key", SHARED_KEY)
                .body(Body::empty())?,
        )
        .await?;
    assert_eq!(empty.status(), StatusCode::OK);
    let empty_body = body_json(empty).await?;
    assert_eq!(empty_body["previewTokens"].as_array().unwrap().len(), 0);

    // Mint two tokens.
    mint_preview_token(&router, "list-me").await?;
    mint_preview_token(&router, "list-me").await?;

    let list = router
        .oneshot(
            Request::builder()
                .method(Method::GET)
                .uri("/documents/list-me/preview-tokens")
                .header("x-api-key", SHARED_KEY)
                .body(Body::empty())?,
        )
        .await?;
    assert_eq!(list.status(), StatusCode::OK);
    let list_body = body_json(list).await?;
    assert_eq!(list_body["previewTokens"].as_array().unwrap().len(), 2);
    Ok(())
}

// --------------------------------------------------------------------------
// Auth gates on management routes
// --------------------------------------------------------------------------

#[tokio::test]
async fn create_token_requires_auth() -> anyhow::Result<()> {
    let _guard = db_guard().await;
    let Some(pool) = common::maybe_pool().await? else {
        return Ok(());
    };
    let router = common::router_for(pool);
    create_draft(&router, "create-needs-auth").await?;

    let resp = router
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/documents/create-needs-auth/preview-tokens")
                .body(Body::empty())?,
        )
        .await?;
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    Ok(())
}

#[tokio::test]
async fn list_tokens_requires_auth() -> anyhow::Result<()> {
    let _guard = db_guard().await;
    let Some(pool) = common::maybe_pool().await? else {
        return Ok(());
    };
    let router = common::router_for(pool);
    create_draft(&router, "list-needs-auth").await?;

    let resp = router
        .oneshot(
            Request::builder()
                .method(Method::GET)
                .uri("/documents/list-needs-auth/preview-tokens")
                .body(Body::empty())?,
        )
        .await?;
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    Ok(())
}

#[tokio::test]
async fn revoke_token_requires_auth() -> anyhow::Result<()> {
    let _guard = db_guard().await;
    let Some(pool) = common::maybe_pool().await? else {
        return Ok(());
    };
    let router = common::router_for(pool);
    create_draft(&router, "revoke-needs-auth").await?;

    let resp = router
        .oneshot(
            Request::builder()
                .method(Method::DELETE)
                .uri("/documents/revoke-needs-auth/preview-tokens/someprefix")
                .body(Body::empty())?,
        )
        .await?;
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    Ok(())
}

// --------------------------------------------------------------------------
// Normal GET never exposes drafts
// --------------------------------------------------------------------------

#[tokio::test]
async fn normal_get_still_hides_draft_after_token_minted() -> anyhow::Result<()> {
    let _guard = db_guard().await;
    let Some(pool) = common::maybe_pool().await? else {
        return Ok(());
    };
    let router = common::router_for(pool);
    create_draft(&router, "hidden-draft").await?;

    // Mint proves the document exists.
    mint_preview_token(&router, "hidden-draft").await?;

    // Normal anonymous GET still returns 404.
    let resp = router
        .oneshot(
            Request::builder()
                .method(Method::GET)
                .uri("/documents/hidden-draft")
                .body(Body::empty())?,
        )
        .await?;
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    Ok(())
}

// --------------------------------------------------------------------------
// Wrong HTTP method → 405
// --------------------------------------------------------------------------

#[tokio::test]
async fn preview_document_post_returns_405() -> anyhow::Result<()> {
    let _guard = db_guard().await;
    let Some(pool) = common::maybe_pool().await? else {
        return Ok(());
    };
    let router = common::router_for(pool);
    create_draft(&router, "method-guard").await?;

    let resp = router
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/documents/method-guard/preview?token=pvw_abc123def456_0000000000000000000000000000000000000000000000000000000000000000")
                .header("x-api-key", SHARED_KEY)
                .body(Body::empty())?,
        )
        .await?;
    assert_eq!(resp.status(), StatusCode::METHOD_NOT_ALLOWED);
    Ok(())
}
