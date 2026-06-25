//! Database-backed contract tests for scoped-token issuance + resolution
//! (ADR 0009, plan 023, **slice 2**).
//!
//! Slice 2 adds: minting tokens via the admin surface, resolving a token
//! presented as `x-api-key` to a [`Principal`], attributing writes to the owning
//! author in the audit trail, and admin-gating the token-management routes. It
//! does NOT yet enforce scope/ownership on document routes (slice 3) — so a
//! `write`-only token can still create *and* publish here; only the admin
//! surface is gated.
//!
//! Skipped unless `DATABASE_URL` is set (or forced via `INKWELL_REQUIRE_DB_TESTS=1`).

mod common;

use axum::body::{Body, to_bytes};
use http::{Method, Request, StatusCode};
use std::sync::LazyLock;
use tokio::sync::{Mutex, MutexGuard};
use tower::ServiceExt;
use uuid::Uuid;

/// Serialize the db-backed tests in this binary; `maybe_pool` truncates on entry.
static DB_TEST_LOCK: LazyLock<Mutex<()>> = LazyLock::new(|| Mutex::new(()));

async fn db_guard() -> MutexGuard<'static, ()> {
    DB_TEST_LOCK.lock().await
}

const SHARED_KEY: &str = "test-secret-key";
const BOOTSTRAP_ADMIN: &str = "00000000-0000-0000-0000-000000000001";

async fn body_json(response: axum::response::Response) -> anyhow::Result<serde_json::Value> {
    let bytes = to_bytes(response.into_body(), usize::MAX).await?;
    Ok(serde_json::from_slice(&bytes)?)
}

/// Mint a token via `POST /admin/tokens` with the shared (admin) key. Returns
/// the full token secret and its public prefix.
async fn mint_token(
    router: &axum::Router,
    name: &str,
    scopes: &[&str],
) -> anyhow::Result<(String, String)> {
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
    let json = body_json(response).await?;
    let token = json["token"]
        .as_str()
        .expect("response carries the token once")
        .to_string();
    let prefix = json["prefix"]
        .as_str()
        .expect("response carries prefix")
        .to_string();
    Ok((token, prefix))
}

/// Create a document presenting `key` as `x-api-key`. Returns the HTTP status.
async fn create_doc_with_key(
    router: &axum::Router,
    title: &str,
    key: &str,
) -> anyhow::Result<StatusCode> {
    let payload = serde_json::json!({ "title": title, "bodyMarkdown": "# Hi" });
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
    Ok(response.status())
}

/// A minted token authenticates writes, and the write is audited against the
/// owning author (not the bootstrap admin / shared key).
#[tokio::test]
async fn token_authenticates_writes_and_audits_by_author() -> anyhow::Result<()> {
    let _guard = db_guard().await;
    let Some(pool) = common::maybe_pool().await? else {
        return Ok(());
    };
    let router = common::router_for(pool.clone());

    let (token, _prefix) = mint_token(&router, "Ada", &["write", "publish"]).await?;
    assert!(token.starts_with("ink_"));

    assert_eq!(
        create_doc_with_key(&router, "By Ada", &token).await?,
        StatusCode::CREATED,
        "a live token should authenticate a create"
    );

    // The audit row is attributed to the author "Ada", not the shared key.
    let (actor_label, actor_id): (String, Option<Uuid>) = sqlx::query_as(
        "SELECT actor_label, actor_author_id FROM write_audit \
         WHERE slug = 'by-ada' AND action = 'create' ORDER BY at DESC LIMIT 1",
    )
    .fetch_one(&pool)
    .await?;
    assert_eq!(actor_label, "Ada");

    let ada_id: Uuid = sqlx::query_scalar("SELECT id FROM authors WHERE name = 'Ada'")
        .fetch_one(&pool)
        .await?;
    assert_eq!(actor_id, Some(ada_id));
    assert_ne!(
        actor_id.map(|id| id.to_string()),
        Some(BOOTSTRAP_ADMIN.to_string()),
        "a token write must NOT be attributed to the bootstrap admin"
    );

    // last_used_at is stamped on resolution.
    let last_used: Option<time::OffsetDateTime> = sqlx::query_scalar(
        "SELECT last_used_at FROM author_tokens t JOIN authors a ON a.id = t.author_id \
         WHERE a.name = 'Ada'",
    )
    .fetch_one(&pool)
    .await?;
    assert!(last_used.is_some(), "resolving a token stamps last_used_at");

    Ok(())
}

/// A revoked token no longer authenticates: the write is rejected 401.
#[tokio::test]
async fn revoked_token_is_rejected() -> anyhow::Result<()> {
    let _guard = db_guard().await;
    let Some(pool) = common::maybe_pool().await? else {
        return Ok(());
    };
    let router = common::router_for(pool.clone());

    let (token, prefix) = mint_token(&router, "Grace", &["write"]).await?;

    // Works before revocation.
    assert_eq!(
        create_doc_with_key(&router, "Before Revoke", &token).await?,
        StatusCode::CREATED
    );

    // Revoke via the admin surface.
    let response = router
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri(format!("/admin/tokens/{prefix}/revoke"))
                .header("x-api-key", SHARED_KEY)
                .body(Body::empty())?,
        )
        .await?;
    assert_eq!(response.status(), StatusCode::OK);

    // Rejected after revocation.
    assert_eq!(
        create_doc_with_key(&router, "After Revoke", &token).await?,
        StatusCode::UNAUTHORIZED,
        "a revoked token must not authenticate"
    );

    // Revoking again is an idempotent 404 (no live token).
    let response = router
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri(format!("/admin/tokens/{prefix}/revoke"))
                .header("x-api-key", SHARED_KEY)
                .body(Body::empty())?,
        )
        .await?;
    assert_eq!(response.status(), StatusCode::NOT_FOUND);

    Ok(())
}

/// The admin surface is admin-gated: a non-admin (write-only) token cannot mint
/// or list tokens (403), and an anonymous request is 401. This guard exists in
/// slice 2 even though document-route scope enforcement is deferred to slice 3.
#[tokio::test]
async fn admin_surface_requires_admin_scope() -> anyhow::Result<()> {
    let _guard = db_guard().await;
    let Some(pool) = common::maybe_pool().await? else {
        return Ok(());
    };
    let router = common::router_for(pool.clone());

    let (write_token, _prefix) = mint_token(&router, "Edsger", &["write"]).await?;

    // A write-only token cannot mint another token (no privilege escalation).
    let response = router
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/admin/tokens")
                .header("content-type", "application/json")
                .header("x-api-key", &write_token)
                .body(Body::from(serde_json::to_vec(
                    &serde_json::json!({ "name": "evil", "scopes": ["admin"] }),
                )?))?,
        )
        .await?;
    assert_eq!(response.status(), StatusCode::FORBIDDEN);

    // ...nor list tokens.
    let response = router
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::GET)
                .uri("/admin/tokens")
                .header("x-api-key", &write_token)
                .body(Body::empty())?,
        )
        .await?;
    assert_eq!(response.status(), StatusCode::FORBIDDEN);

    // An anonymous request is 401 (no credential at all).
    let response = router
        .oneshot(
            Request::builder()
                .method(Method::GET)
                .uri("/admin/tokens")
                .body(Body::empty())?,
        )
        .await?;
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);

    Ok(())
}

/// `GET /admin/tokens` lists minted tokens by prefix and never leaks the secret.
#[tokio::test]
async fn token_list_shows_prefix_without_secret() -> anyhow::Result<()> {
    let _guard = db_guard().await;
    let Some(router) = common::maybe_router().await? else {
        return Ok(());
    };

    let (token, prefix) = mint_token(&router, "Linus", &["read", "write"]).await?;

    let response = router
        .oneshot(
            Request::builder()
                .method(Method::GET)
                .uri("/admin/tokens")
                .header("x-api-key", SHARED_KEY)
                .body(Body::empty())?,
        )
        .await?;
    assert_eq!(response.status(), StatusCode::OK);
    let json = body_json(response).await?;
    let tokens = json["tokens"].as_array().expect("tokens array");
    let entry = tokens
        .iter()
        .find(|t| t["prefix"] == prefix.as_str())
        .expect("minted token appears in the list");
    assert_eq!(entry["authorName"], "Linus");
    // The list response must never carry the secret.
    let rendered = serde_json::to_string(&json)?;
    assert!(
        !rendered.contains(&token),
        "token list must not leak the secret"
    );
    Ok(())
}

/// An unknown/malformed token is unauthorized for writes (no DB row, bad hash).
#[tokio::test]
async fn unknown_token_is_unauthorized() -> anyhow::Result<()> {
    let _guard = db_guard().await;
    let Some(router) = common::maybe_router().await? else {
        return Ok(());
    };

    // Well-formed shape but no matching row.
    assert_eq!(
        create_doc_with_key(&router, "Nope", "ink_deadbeef0000_cafef00d").await?,
        StatusCode::UNAUTHORIZED
    );
    // Not a token and not the shared key.
    assert_eq!(
        create_doc_with_key(&router, "Nope2", "totally-bogus").await?,
        StatusCode::UNAUTHORIZED
    );
    Ok(())
}

/// A read-scoped token sees its OWN drafts (owner visibility, slice 3b), but NOT
/// another author's draft. An anonymous reader sees no drafts at all.
#[tokio::test]
async fn token_grants_draft_read_visibility() -> anyhow::Result<()> {
    let _guard = db_guard().await;
    let Some(pool) = common::maybe_pool().await? else {
        return Ok(());
    };
    let router = common::router_for(pool.clone());

    // Create a draft owned by the admin (shared key → bootstrap admin).
    assert_eq!(
        create_doc_with_key(&router, "Admin Secret Draft", SHARED_KEY).await?,
        StatusCode::CREATED
    );

    // Mint a read+write token so Reader can create their own draft.
    let (rw_token, _prefix) = mint_token(&router, "Reader", &["read", "write"]).await?;

    // Reader creates their OWN draft.
    assert_eq!(
        create_doc_with_key(&router, "Reader Own Draft", &rw_token).await?,
        StatusCode::CREATED
    );

    // Anonymous reader: both drafts are invisible (404).
    let response = router
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::GET)
                .uri("/documents/admin-secret-draft")
                .body(Body::empty())?,
        )
        .await?;
    assert_eq!(
        response.status(),
        StatusCode::NOT_FOUND,
        "anonymous must not see admin draft"
    );

    let response = router
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::GET)
                .uri("/documents/reader-own-draft")
                .body(Body::empty())?,
        )
        .await?;
    assert_eq!(
        response.status(),
        StatusCode::NOT_FOUND,
        "anonymous must not see reader draft"
    );

    // Reader (read+write) sees their OWN draft but NOT the admin's draft.
    let response = router
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::GET)
                .uri("/documents/reader-own-draft")
                .header("x-api-key", &rw_token)
                .body(Body::empty())?,
        )
        .await?;
    assert_eq!(
        response.status(),
        StatusCode::OK,
        "reader must see own draft"
    );
    let json = body_json(response).await?;
    assert_eq!(json["status"], "draft");

    // Reader CANNOT see the admin's draft (different owner — cross-author leak guard).
    let response = router
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::GET)
                .uri("/documents/admin-secret-draft")
                .header("x-api-key", &rw_token)
                .body(Body::empty())?,
        )
        .await?;
    assert_eq!(
        response.status(),
        StatusCode::NOT_FOUND,
        "reader must NOT see admin's draft (slice 3b owner isolation)"
    );

    // Admin (shared key) sees both drafts.
    let response = router
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::GET)
                .uri("/documents/admin-secret-draft")
                .header("x-api-key", SHARED_KEY)
                .body(Body::empty())?,
        )
        .await?;
    assert_eq!(
        response.status(),
        StatusCode::OK,
        "admin must see admin's own draft"
    );

    let response = router
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::GET)
                .uri("/documents/reader-own-draft")
                .header("x-api-key", SHARED_KEY)
                .body(Body::empty())?,
        )
        .await?;
    assert_eq!(
        response.status(),
        StatusCode::OK,
        "admin must see reader's draft"
    );

    Ok(())
}
