//! Database-backed contract tests for scoped-token ENFORCEMENT
//! (ADR 0009, plan 023, **slice 3**).
//!
//! Slice 3 turns on authorization on the document routes:
//! - mutations require the right scope (`write` for create/update/delete,
//!   `publish` for publish/unpublish);
//! - non-admin principals may only mutate notes they OWN (403 otherwise);
//! - `create` stamps `owner_id` from the principal;
//! - draft READ visibility requires the `read` scope (admin implies it).
//!
//! Per-owner draft read ISOLATION (a read-scoped author seeing only their own
//! drafts) is deferred to slice 3b; here a `read` token sees all drafts.
//!
//! Skipped unless `DATABASE_URL` is set (or forced via `INKWELL_REQUIRE_DB_TESTS=1`).

mod common;

use axum::body::{Body, to_bytes};
use http::{Method, Request, StatusCode};
use std::sync::LazyLock;
use tokio::sync::{Mutex, MutexGuard};
use tower::ServiceExt;
use uuid::Uuid;

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

async fn create_doc(router: &axum::Router, title: &str, key: &str) -> anyhow::Result<StatusCode> {
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

async fn update_doc(router: &axum::Router, slug: &str, key: &str) -> anyhow::Result<StatusCode> {
    let payload = serde_json::json!({ "title": "Edited", "bodyMarkdown": "# Edited" });
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

async fn act(
    router: &axum::Router,
    method: Method,
    uri: &str,
    key: &str,
) -> anyhow::Result<StatusCode> {
    let response = router
        .clone()
        .oneshot(
            Request::builder()
                .method(method)
                .uri(uri)
                .header("x-api-key", key)
                .body(Body::empty())?,
        )
        .await?;
    Ok(response.status())
}

/// `create` requires `write` and stamps `owner_id`; a non-admin may update/delete
/// only its own notes, and another author's note is 403.
#[tokio::test]
async fn write_scope_and_ownership_are_enforced() -> anyhow::Result<()> {
    let _guard = db_guard().await;
    let Some(pool) = common::maybe_pool().await? else {
        return Ok(());
    };
    let router = common::router_for(pool.clone());

    let ada = mint_token(&router, "Ada", &["write", "read"]).await?;
    let bob = mint_token(&router, "Bob", &["write", "read"]).await?;

    assert_eq!(
        create_doc(&router, "Ada Note", &ada).await?,
        StatusCode::CREATED
    );

    // owner_id was stamped to Ada (not the bootstrap admin).
    let (owner, ada_id): (Option<Uuid>, Uuid) = {
        let owner = sqlx::query_scalar::<_, Option<Uuid>>(
            "SELECT owner_id FROM documents WHERE slug = 'ada-note'",
        )
        .fetch_one(&pool)
        .await?;
        let ada_id = sqlx::query_scalar::<_, Uuid>("SELECT id FROM authors WHERE name = 'Ada'")
            .fetch_one(&pool)
            .await?;
        (owner, ada_id)
    };
    assert_eq!(owner, Some(ada_id), "create stamps owner_id to the author");

    // Ada owns it → can update.
    assert_eq!(update_doc(&router, "ada-note", &ada).await?, StatusCode::OK);

    // Bob has `write` scope but does NOT own ada-note. Ownership is enforced
    // atomically in the UPDATE/DELETE (owner-scoped WHERE), so the write matches
    // no row and surfaces as 404 — which also hides the note's existence from a
    // non-owner rather than confirming it with a 403.
    assert_eq!(
        update_doc(&router, "ada-note", &bob).await?,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        act(&router, Method::DELETE, "/documents/ada-note", &bob).await?,
        StatusCode::NOT_FOUND
    );
    // ...and ada-note is untouched (still present, owned by Ada).
    assert_eq!(
        act(&router, Method::GET, "/documents/ada-note", &ada).await?,
        StatusCode::OK
    );

    Ok(())
}

/// `publish`/`unpublish` require the `publish` scope AND ownership.
#[tokio::test]
async fn publish_scope_and_ownership_are_enforced() -> anyhow::Result<()> {
    let _guard = db_guard().await;
    let Some(pool) = common::maybe_pool().await? else {
        return Ok(());
    };
    let router = common::router_for(pool.clone());

    let writer = mint_token(&router, "Writer", &["write"]).await?;
    let publisher = mint_token(&router, "Publisher", &["write", "publish"]).await?;

    assert_eq!(
        create_doc(&router, "W Note", &writer).await?,
        StatusCode::CREATED
    );
    assert_eq!(
        create_doc(&router, "P Note", &publisher).await?,
        StatusCode::CREATED
    );

    // A write-only token cannot publish even its OWN note (missing publish scope).
    assert_eq!(
        act(&router, Method::POST, "/documents/w-note/publish", &writer).await?,
        StatusCode::FORBIDDEN
    );

    // The publisher publishes its OWN note.
    assert_eq!(
        act(
            &router,
            Method::POST,
            "/documents/p-note/publish",
            &publisher
        )
        .await?,
        StatusCode::OK
    );

    // The publisher has the scope but NOT ownership of the writer's note.
    // Ownership is enforced atomically (owner-scoped UPDATE matches no row), so a
    // non-owner publish surfaces as 404 (also hiding existence), not 403.
    assert_eq!(
        act(
            &router,
            Method::POST,
            "/documents/w-note/publish",
            &publisher
        )
        .await?,
        StatusCode::NOT_FOUND
    );

    // --- /unpublish carries the SAME scope+ownership boundary as /publish ---

    // Owner + publish scope → can unpublish their own (now-published) note.
    assert_eq!(
        act(
            &router,
            Method::POST,
            "/documents/p-note/unpublish",
            &publisher
        )
        .await?,
        StatusCode::OK
    );
    // Missing publish scope → 403 (capability check runs before any DB write).
    assert_eq!(
        act(
            &router,
            Method::POST,
            "/documents/p-note/unpublish",
            &writer
        )
        .await?,
        StatusCode::FORBIDDEN
    );
    // Has publish scope but not owner → 404 (atomic owner-scoped write, no leak).
    assert_eq!(
        act(
            &router,
            Method::POST,
            "/documents/w-note/unpublish",
            &publisher
        )
        .await?,
        StatusCode::NOT_FOUND
    );

    Ok(())
}

/// A `read`-only token cannot write (403) but can see drafts. A `write`-only
/// token can write but is a public reader (no draft visibility).
#[tokio::test]
async fn read_scope_gates_writes_and_draft_visibility() -> anyhow::Result<()> {
    let _guard = db_guard().await;
    let Some(pool) = common::maybe_pool().await? else {
        return Ok(());
    };
    let router = common::router_for(pool.clone());

    // Admin seeds a draft.
    assert_eq!(
        create_doc(&router, "Seed Draft", SHARED_KEY).await?,
        StatusCode::CREATED
    );

    let reader = mint_token(&router, "Reader", &["read"]).await?;
    let writer = mint_token(&router, "Writer2", &["write"]).await?;

    // read-only: cannot create (no write scope) → 403.
    assert_eq!(
        create_doc(&router, "Nope", &reader).await?,
        StatusCode::FORBIDDEN
    );
    // read-only: sees the draft (read scope ⇒ All visibility).
    assert_eq!(
        act(&router, Method::GET, "/documents/seed-draft", &reader).await?,
        StatusCode::OK
    );

    // write-only: creates its own draft...
    assert_eq!(
        create_doc(&router, "Writer Draft", &writer).await?,
        StatusCode::CREATED
    );
    // ...but, lacking `read`, is a public reader and cannot see ANY draft —
    // including its own (the coarse slice-3 read gate; isolation is slice 3b).
    assert_eq!(
        act(&router, Method::GET, "/documents/writer-draft", &writer).await?,
        StatusCode::NOT_FOUND
    );
    // The admin shared key can see it.
    assert_eq!(
        act(&router, Method::GET, "/documents/writer-draft", SHARED_KEY).await?,
        StatusCode::OK
    );

    Ok(())
}

/// The admin shared key bypasses ownership: it can edit and delete any note.
#[tokio::test]
async fn admin_bypasses_ownership() -> anyhow::Result<()> {
    let _guard = db_guard().await;
    let Some(pool) = common::maybe_pool().await? else {
        return Ok(());
    };
    let router = common::router_for(pool.clone());

    let ada = mint_token(&router, "Ada", &["write"]).await?;
    assert_eq!(
        create_doc(&router, "Ada Owned", &ada).await?,
        StatusCode::CREATED
    );

    // Admin edits and deletes Ada's note despite not owning it.
    assert_eq!(
        update_doc(&router, "ada-owned", SHARED_KEY).await?,
        StatusCode::OK
    );
    assert_eq!(
        act(
            &router,
            Method::POST,
            "/documents/ada-owned/publish",
            SHARED_KEY
        )
        .await?,
        StatusCode::OK
    );
    assert_eq!(
        act(&router, Method::DELETE, "/documents/ada-owned", SHARED_KEY).await?,
        StatusCode::NO_CONTENT
    );

    Ok(())
}
