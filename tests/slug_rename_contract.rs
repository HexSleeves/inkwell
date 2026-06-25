//! Database-backed contract tests for slug rename + 301 alias redirect
//! (ADR 0011).
//!
//! A document's slug can be renamed via `PUT /documents/{slug}` with a `"slug"`
//! field. The old slug is recorded as a 301 alias to the document, so old links
//! keep working — on both the JSON route (`/documents/{slug}`) and the HTML page
//! (`/{slug}`). Renames respect scope (`write`) and ownership, and an alias
//! never leaks a draft the caller cannot see.
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

/// Mint a scoped token via `POST /admin/tokens` with the shared (admin) key.
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
    Ok(body_json(response)
        .await?
        .get("token")
        .and_then(|t| t.as_str())
        .expect("token in response")
        .to_string())
}

/// Create a (draft) note and return its slug.
async fn create_note(router: &axum::Router, title: &str, key: &str) -> anyhow::Result<String> {
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
    assert_eq!(
        response.status(),
        StatusCode::CREATED,
        "create should succeed"
    );
    Ok(body_json(response)
        .await?
        .get("slug")
        .and_then(|s| s.as_str())
        .expect("slug in response")
        .to_string())
}

/// Rename a note's slug via `PUT /documents/{slug}` with `{ "slug": new_slug }`.
async fn rename(
    router: &axum::Router,
    slug: &str,
    new_slug: &str,
    key: &str,
) -> anyhow::Result<StatusCode> {
    let payload = serde_json::json!({ "slug": new_slug });
    let response = router
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::PUT)
                .uri(format!("/documents/{slug}"))
                .header("content-type", "application/json")
                .header("x-api-key", key)
                .body(Body::from(serde_json::to_vec(&payload)?))?,
        )
        .await?;
    Ok(response.status())
}

async fn publish(router: &axum::Router, slug: &str, key: &str) -> anyhow::Result<StatusCode> {
    act(
        router,
        Method::POST,
        &format!("/documents/{slug}/publish"),
        Some(key),
    )
    .await
}

async fn act(
    router: &axum::Router,
    method: Method,
    uri: &str,
    key: Option<&str>,
) -> anyhow::Result<StatusCode> {
    let mut builder = Request::builder().method(method).uri(uri);
    if let Some(k) = key {
        builder = builder.header("x-api-key", k);
    }
    let response = router.clone().oneshot(builder.body(Body::empty())?).await?;
    Ok(response.status())
}

/// `GET` a slug and return `(status, Location header)`. Works for both the JSON
/// route (`/documents/{slug}`) and the HTML page (`/{slug}`). oneshot does not
/// follow redirects, so a 301 is observable directly.
async fn get_at(
    router: &axum::Router,
    uri: &str,
    key: Option<&str>,
) -> anyhow::Result<(StatusCode, Option<String>)> {
    let mut builder = Request::builder().method(Method::GET).uri(uri);
    if let Some(k) = key {
        builder = builder.header("x-api-key", k);
    }
    let response = router.clone().oneshot(builder.body(Body::empty())?).await?;
    let status = response.status();
    let location = response
        .headers()
        .get(http::header::LOCATION)
        .and_then(|v| v.to_str().ok())
        .map(str::to_string);
    Ok((status, location))
}

async fn get_json(
    router: &axum::Router,
    slug: &str,
    key: Option<&str>,
) -> anyhow::Result<(StatusCode, Option<String>)> {
    get_at(router, &format!("/documents/{slug}"), key).await
}

/// Renaming a note records a 301 alias from the old slug to the document's
/// current slug, on both the JSON route and the HTML page.
#[tokio::test]
async fn owner_rename_redirects_old_slug() -> anyhow::Result<()> {
    let _guard = db_guard().await;
    let Some(pool) = common::maybe_pool().await? else {
        return Ok(());
    };
    let router = common::router_for(pool.clone());

    let alice = mint_token(&router, "Alice", &["read", "write", "publish"]).await?;
    let old = create_note(&router, "My First Title", &alice).await?;
    assert_eq!(
        rename(&router, &old, "my-better-title", &alice).await?,
        StatusCode::OK
    );

    // New slug resolves for the owner.
    let (status, _) = get_json(&router, "my-better-title", Some(&alice)).await?;
    assert_eq!(status, StatusCode::OK);

    // Old slug 301s to the new one (owner can see their own draft target).
    let (status, location) = get_json(&router, &old, Some(&alice)).await?;
    assert_eq!(status, StatusCode::MOVED_PERMANENTLY);
    assert_eq!(location.as_deref(), Some("/documents/my-better-title"));

    // Publish, then the HTML page redirects for anyone.
    assert_eq!(
        publish(&router, "my-better-title", &alice).await?,
        StatusCode::OK
    );
    let (status, location) = get_at(&router, &format!("/{old}"), None).await?;
    assert_eq!(status, StatusCode::MOVED_PERMANENTLY);
    assert_eq!(location.as_deref(), Some("/my-better-title"));

    Ok(())
}

/// A chain of renames `a → b → c` resolves both `a` and `b` straight to `c`
/// (aliases store the document id, not a slug, so there are no stale chains).
#[tokio::test]
async fn chained_renames_resolve_to_current_slug() -> anyhow::Result<()> {
    let _guard = db_guard().await;
    let Some(pool) = common::maybe_pool().await? else {
        return Ok(());
    };
    let router = common::router_for(pool.clone());

    let alice = mint_token(&router, "Alice", &["read", "write", "publish"]).await?;
    let a = create_note(&router, "Alpha", &alice).await?;
    assert_eq!(rename(&router, &a, "beta", &alice).await?, StatusCode::OK);
    assert_eq!(
        rename(&router, "beta", "gamma", &alice).await?,
        StatusCode::OK
    );

    let (status, location) = get_json(&router, &a, Some(&alice)).await?;
    assert_eq!(status, StatusCode::MOVED_PERMANENTLY);
    assert_eq!(
        location.as_deref(),
        Some("/documents/gamma"),
        "alpha → gamma"
    );

    let (status, location) = get_json(&router, "beta", Some(&alice)).await?;
    assert_eq!(status, StatusCode::MOVED_PERMANENTLY);
    assert_eq!(
        location.as_deref(),
        Some("/documents/gamma"),
        "beta → gamma"
    );

    Ok(())
}

/// Renaming onto a slug already used by a live document is a 409 Conflict.
#[tokio::test]
async fn rename_to_existing_slug_conflicts() -> anyhow::Result<()> {
    let _guard = db_guard().await;
    let Some(pool) = common::maybe_pool().await? else {
        return Ok(());
    };
    let router = common::router_for(pool.clone());

    let alice = mint_token(&router, "Alice", &["read", "write"]).await?;
    let one = create_note(&router, "One", &alice).await?;
    let _two = create_note(&router, "Two", &alice).await?;
    assert_eq!(
        rename(&router, &one, "two", &alice).await?,
        StatusCode::CONFLICT
    );
    // The original is untouched.
    let (status, _) = get_json(&router, &one, Some(&alice)).await?;
    assert_eq!(status, StatusCode::OK);

    Ok(())
}

/// A malformed slug is a 400 before any DB work.
#[tokio::test]
async fn rename_with_invalid_slug_is_rejected() -> anyhow::Result<()> {
    let _guard = db_guard().await;
    let Some(pool) = common::maybe_pool().await? else {
        return Ok(());
    };
    let router = common::router_for(pool.clone());

    let alice = mint_token(&router, "Alice", &["read", "write"]).await?;
    let slug = create_note(&router, "Valid Note", &alice).await?;
    assert_eq!(
        rename(&router, &slug, "Not A Slug!", &alice).await?,
        StatusCode::BAD_REQUEST
    );

    Ok(())
}

/// A non-owner cannot rename another author's note (404, no existence leak), and
/// the slug is unchanged.
#[tokio::test]
async fn non_owner_cannot_rename() -> anyhow::Result<()> {
    let _guard = db_guard().await;
    let Some(pool) = common::maybe_pool().await? else {
        return Ok(());
    };
    let router = common::router_for(pool.clone());

    let alice = mint_token(&router, "Alice", &["read", "write"]).await?;
    let bob = mint_token(&router, "Bob", &["read", "write"]).await?;
    let slug = create_note(&router, "Alice Note", &alice).await?;

    assert_eq!(
        rename(&router, &slug, "bob-grab", &bob).await?,
        StatusCode::NOT_FOUND
    );
    // Unchanged: still resolves at its original slug for Alice; the attempted
    // target does not exist.
    let (status, _) = get_json(&router, &slug, Some(&alice)).await?;
    assert_eq!(status, StatusCode::OK);
    let (status, _) = get_json(&router, "bob-grab", Some(&alice)).await?;
    assert_eq!(status, StatusCode::NOT_FOUND);

    Ok(())
}

/// A read-only token cannot rename (no `write` scope → 403, before ownership).
#[tokio::test]
async fn read_only_token_cannot_rename() -> anyhow::Result<()> {
    let _guard = db_guard().await;
    let Some(pool) = common::maybe_pool().await? else {
        return Ok(());
    };
    let router = common::router_for(pool.clone());

    let alice = mint_token(&router, "Alice", &["read", "write"]).await?;
    let reader = mint_token(&router, "Reader", &["read"]).await?;
    let slug = create_note(&router, "Some Note", &alice).await?;

    assert_eq!(
        rename(&router, &slug, "reader-rename", &reader).await?,
        StatusCode::FORBIDDEN
    );

    Ok(())
}

/// An alias to a still-draft note does not leak it to anonymous callers: the old
/// slug 404s (not 301) until the note is published.
#[tokio::test]
async fn alias_does_not_leak_draft_to_anonymous() -> anyhow::Result<()> {
    let _guard = db_guard().await;
    let Some(pool) = common::maybe_pool().await? else {
        return Ok(());
    };
    let router = common::router_for(pool.clone());

    let alice = mint_token(&router, "Alice", &["read", "write", "publish"]).await?;
    let old = create_note(&router, "Secret Draft", &alice).await?;
    assert_eq!(
        rename(&router, &old, "renamed-secret", &alice).await?,
        StatusCode::OK
    );

    // Draft: anonymous must NOT be redirected (that would confirm it exists).
    let (status, location) = get_json(&router, &old, None).await?;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(location, None);

    // Once published, the redirect becomes public.
    assert_eq!(
        publish(&router, "renamed-secret", &alice).await?,
        StatusCode::OK
    );
    let (status, location) = get_json(&router, &old, None).await?;
    assert_eq!(status, StatusCode::MOVED_PERMANENTLY);
    assert_eq!(location.as_deref(), Some("/documents/renamed-secret"));

    Ok(())
}

/// Deleting a document drops its aliases (FK `ON DELETE CASCADE`): the old slug
/// 404s afterward instead of redirecting to a dangling id.
#[tokio::test]
async fn delete_cascades_aliases() -> anyhow::Result<()> {
    let _guard = db_guard().await;
    let Some(pool) = common::maybe_pool().await? else {
        return Ok(());
    };
    let router = common::router_for(pool.clone());

    let alice = mint_token(&router, "Alice", &["read", "write", "publish"]).await?;
    let old = create_note(&router, "Doomed Note", &alice).await?;
    assert_eq!(
        rename(&router, &old, "doomed-renamed", &alice).await?,
        StatusCode::OK
    );
    assert_eq!(
        publish(&router, "doomed-renamed", &alice).await?,
        StatusCode::OK
    );

    // Alias works before delete.
    let (status, _) = get_json(&router, &old, None).await?;
    assert_eq!(status, StatusCode::MOVED_PERMANENTLY);

    // Delete the document; its alias must be gone with it.
    assert_eq!(
        act(
            &router,
            Method::DELETE,
            "/documents/doomed-renamed",
            Some(&alice)
        )
        .await?,
        StatusCode::NO_CONTENT
    );
    let (status, _) = get_json(&router, &old, None).await?;
    assert_eq!(status, StatusCode::NOT_FOUND);
    let (status, _) = get_json(&router, "doomed-renamed", None).await?;
    assert_eq!(status, StatusCode::NOT_FOUND);

    Ok(())
}
