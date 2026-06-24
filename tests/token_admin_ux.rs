//! Database-backed contract tests for token-list filtering and prune UX
//! (feat: hide revoked from list (--all) + prune revoked).
//!
//! Exercises:
//! - `GET /admin/tokens` (default) hides revoked tokens.
//! - `GET /admin/tokens?all=true` includes revoked tokens.
//! - `POST /admin/tokens/prune` hard-deletes revoked tokens (count returned).
//! - A second prune is a safe no-op (returns 0).
//! - The live token still authenticates after prune.
//!
//! Skipped unless `DATABASE_URL` is set (or forced via `INKWELL_REQUIRE_DB_TESTS=1`).

mod common;

use axum::body::{Body, to_bytes};
use http::{Method, Request, StatusCode};
use std::sync::LazyLock;
use tokio::sync::{Mutex, MutexGuard};
use tower::ServiceExt;

/// Serialize the db-backed tests in this binary; `maybe_pool` truncates on entry.
static DB_TEST_LOCK: LazyLock<Mutex<()>> = LazyLock::new(|| Mutex::new(()));

async fn db_guard() -> MutexGuard<'static, ()> {
    DB_TEST_LOCK.lock().await
}

const SHARED_KEY: &str = "test-secret-key";

async fn body_json(response: axum::response::Response) -> anyhow::Result<serde_json::Value> {
    let bytes = to_bytes(response.into_body(), usize::MAX).await?;
    Ok(serde_json::from_slice(&bytes)?)
}

/// Mint a token via `POST /admin/tokens` with the shared (admin) key.
/// Returns `(full_token_secret, public_prefix)`.
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

/// Revoke a token via `POST /admin/tokens/{prefix}/revoke`.
async fn revoke_token(router: &axum::Router, prefix: &str) -> anyhow::Result<()> {
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
    assert_eq!(response.status(), StatusCode::OK, "revoke should succeed");
    Ok(())
}

/// GET /admin/tokens with optional `?all=true`.
async fn list_tokens(
    router: &axum::Router,
    include_revoked: bool,
) -> anyhow::Result<serde_json::Value> {
    let uri = if include_revoked {
        "/admin/tokens?all=true"
    } else {
        "/admin/tokens"
    };
    let response = router
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::GET)
                .uri(uri)
                .header("x-api-key", SHARED_KEY)
                .body(Body::empty())?,
        )
        .await?;
    assert_eq!(response.status(), StatusCode::OK, "list should succeed");
    body_json(response).await
}

/// POST /admin/tokens/prune and return the pruned count.
async fn prune_tokens(router: &axum::Router) -> anyhow::Result<u64> {
    let response = router
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/admin/tokens/prune")
                .header("x-api-key", SHARED_KEY)
                .body(Body::empty())?,
        )
        .await?;
    assert_eq!(response.status(), StatusCode::OK, "prune should succeed");
    let json = body_json(response).await?;
    Ok(json["pruned"].as_u64().expect("pruned field is a u64"))
}

/// Default listing hides revoked tokens; `?all=true` shows them.
/// After prune, the revoked token is gone and the live token still works.
#[tokio::test]
async fn list_hides_revoked_by_default_and_all_shows_them() -> anyhow::Result<()> {
    let _guard = db_guard().await;
    let Some(pool) = common::maybe_pool().await? else {
        return Ok(());
    };
    let router = common::router_for(pool.clone());

    // Mint two tokens: one will stay live, the other will be revoked.
    let (_live_token, live_prefix) = mint_token(&router, "Alice", &["write"]).await?;
    let (_revoked_token, revoked_prefix) = mint_token(&router, "Bob", &["write"]).await?;

    // Revoke Bob's token.
    revoke_token(&router, &revoked_prefix).await?;

    // Default listing (no ?all): only the live token appears.
    let json = list_tokens(&router, false).await?;
    let tokens = json["tokens"].as_array().expect("tokens array");
    let prefixes: Vec<&str> = tokens.iter().filter_map(|t| t["prefix"].as_str()).collect();
    assert!(
        prefixes.contains(&live_prefix.as_str()),
        "live token must appear in default listing"
    );
    assert!(
        !prefixes.contains(&revoked_prefix.as_str()),
        "revoked token must NOT appear in default listing"
    );

    // ?all=true: both tokens appear.
    let json_all = list_tokens(&router, true).await?;
    let tokens_all = json_all["tokens"].as_array().expect("tokens array");
    let prefixes_all: Vec<&str> = tokens_all
        .iter()
        .filter_map(|t| t["prefix"].as_str())
        .collect();
    assert!(
        prefixes_all.contains(&live_prefix.as_str()),
        "live token must appear in ?all=true listing"
    );
    assert!(
        prefixes_all.contains(&revoked_prefix.as_str()),
        "revoked token must appear in ?all=true listing"
    );

    Ok(())
}

/// Prune deletes revoked tokens, live tokens survive, and a second prune is safe.
#[tokio::test]
async fn prune_removes_revoked_and_is_idempotent() -> anyhow::Result<()> {
    let _guard = db_guard().await;
    let Some(pool) = common::maybe_pool().await? else {
        return Ok(());
    };
    let router = common::router_for(pool.clone());

    // Mint and revoke one token; keep one live.
    let (live_token, live_prefix) = mint_token(&router, "Carol", &["write"]).await?;
    let (_revoked_token, revoked_prefix) = mint_token(&router, "Dave", &["write"]).await?;
    revoke_token(&router, &revoked_prefix).await?;

    // First prune: should delete exactly 1 revoked token.
    let pruned = prune_tokens(&router).await?;
    assert_eq!(pruned, 1, "first prune should delete the one revoked token");

    // Second prune: nothing left to delete.
    let pruned2 = prune_tokens(&router).await?;
    assert_eq!(pruned2, 0, "second prune must be a safe no-op");

    // The live token still authenticates after the prune.
    let payload = serde_json::json!({ "title": "After Prune", "bodyMarkdown": "# Hi" });
    let response = router
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/documents")
                .header("content-type", "application/json")
                .header("x-api-key", &live_token)
                .body(Body::from(serde_json::to_vec(&payload)?))?,
        )
        .await?;
    assert_eq!(
        response.status(),
        StatusCode::CREATED,
        "live token must still authenticate after prune"
    );

    // The live token still appears in the default listing (not pruned).
    let json = list_tokens(&router, false).await?;
    let tokens = json["tokens"].as_array().expect("tokens array");
    let prefixes: Vec<&str> = tokens.iter().filter_map(|t| t["prefix"].as_str()).collect();
    assert!(
        prefixes.contains(&live_prefix.as_str()),
        "live token must still appear in listing after prune"
    );

    Ok(())
}

/// Prune route is admin-gated: non-admin token gets 403, anonymous gets 401.
#[tokio::test]
async fn prune_requires_admin_scope() -> anyhow::Result<()> {
    let _guard = db_guard().await;
    let Some(pool) = common::maybe_pool().await? else {
        return Ok(());
    };
    let router = common::router_for(pool.clone());

    let (write_token, _prefix) = mint_token(&router, "Eve", &["write"]).await?;

    // A write-only token is forbidden.
    let response = router
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/admin/tokens/prune")
                .header("x-api-key", &write_token)
                .body(Body::empty())?,
        )
        .await?;
    assert_eq!(response.status(), StatusCode::FORBIDDEN);

    // Anonymous is unauthorized.
    let response = router
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/admin/tokens/prune")
                .body(Body::empty())?,
        )
        .await?;
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);

    Ok(())
}
