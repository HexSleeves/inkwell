//! Database-backed contract tests for the scoped-author-tokens foundation
//! (ADR 0009, plan 023, **slice 1**).
//!
//! Slice 1 is the non-enforcing foundation: schema (`authors`, `author_tokens`,
//! `documents.owner_id`, `write_audit`), a seeded bootstrap admin author, an
//! ownership backfill, and best-effort audit-on-write. It changes NOTHING about
//! who can do what, so these tests assert both the new foundation AND that the
//! existing auth/visibility behavior is byte-for-byte unchanged.
//!
//! Skipped unless `DATABASE_URL` is set (or forced via `INKWELL_REQUIRE_DB_TESTS=1`),
//! matching the rest of the db-backed suite.

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

const BOOTSTRAP_ADMIN: &str = "00000000-0000-0000-0000-000000000001";

// ---------------------------------------------------------------------------
// migration shape
// ---------------------------------------------------------------------------

/// The four foundation tables exist with the expected key columns, and
/// `documents.owner_id` is present AND nullable (the `NOT NULL` tightening is
/// deferred to slice 4).
#[tokio::test]
async fn foundation_schema_exists_with_nullable_owner() -> anyhow::Result<()> {
    let _guard = db_guard().await;
    let Some(pool) = common::maybe_pool().await? else {
        return Ok(());
    };

    for table in ["authors", "author_tokens", "write_audit"] {
        let exists: bool = sqlx::query_scalar(
            "SELECT EXISTS (SELECT 1 FROM information_schema.tables \
             WHERE table_schema = 'public' AND table_name = $1)",
        )
        .bind(table)
        .fetch_one(&pool)
        .await?;
        assert!(exists, "table {table} should exist after migrate");
    }

    // documents.owner_id exists and is nullable.
    let is_nullable: String = sqlx::query_scalar(
        "SELECT is_nullable FROM information_schema.columns \
         WHERE table_schema = 'public' AND table_name = 'documents' AND column_name = 'owner_id'",
    )
    .fetch_one(&pool)
    .await?;
    assert_eq!(
        is_nullable, "YES",
        "documents.owner_id must remain nullable in slice 1"
    );

    Ok(())
}

/// The bootstrap admin author is seeded at the fixed uuid by the migration.
#[tokio::test]
async fn bootstrap_admin_author_is_seeded() -> anyhow::Result<()> {
    let _guard = db_guard().await;
    let Some(pool) = common::maybe_pool().await? else {
        return Ok(());
    };

    let name: Option<String> = sqlx::query_scalar("SELECT name FROM authors WHERE id = $1::uuid")
        .bind(BOOTSTRAP_ADMIN)
        .fetch_optional(&pool)
        .await?;
    assert_eq!(name.as_deref(), Some("admin"));

    Ok(())
}

// ---------------------------------------------------------------------------
// backfill + idempotency
// ---------------------------------------------------------------------------

/// Re-running the seed+backfill is a no-op: a document inserted with no owner is
/// claimed by the bootstrap admin, and a second run neither duplicates the
/// author nor re-touches the already-owned row.
#[tokio::test]
async fn seed_and_backfill_are_idempotent() -> anyhow::Result<()> {
    let _guard = db_guard().await;
    let Some(pool) = common::maybe_pool().await? else {
        return Ok(());
    };

    // A pre-existing document with no owner (the state migration 0015 backfills).
    let doc_id: Uuid = sqlx::query_scalar(
        "INSERT INTO documents (slug, title, body_markdown, rendered_html) \
         VALUES ('legacy', 'Legacy', '# Legacy', '<h1>Legacy</h1>') RETURNING id",
    )
    .fetch_one(&pool)
    .await?;

    // The seed+backfill statements, run twice (mirrors migration 0015 re-applied).
    let seed_backfill = async || -> anyhow::Result<()> {
        sqlx::query(
            "INSERT INTO authors (id, name) VALUES ($1::uuid, 'admin') \
             ON CONFLICT (id) DO NOTHING",
        )
        .bind(BOOTSTRAP_ADMIN)
        .execute(&pool)
        .await?;
        sqlx::query("UPDATE documents SET owner_id = $1::uuid WHERE owner_id IS NULL")
            .bind(BOOTSTRAP_ADMIN)
            .execute(&pool)
            .await?;
        Ok(())
    };
    seed_backfill().await?;
    seed_backfill().await?;

    // Exactly one admin author, and the legacy doc is owned by it.
    let admin_count: i64 = sqlx::query_scalar("SELECT count(*) FROM authors WHERE id = $1::uuid")
        .bind(BOOTSTRAP_ADMIN)
        .fetch_one(&pool)
        .await?;
    assert_eq!(
        admin_count, 1,
        "seed must not duplicate the bootstrap admin"
    );

    let owner: Option<Uuid> = sqlx::query_scalar("SELECT owner_id FROM documents WHERE id = $1")
        .bind(doc_id)
        .fetch_one(&pool)
        .await?;
    assert_eq!(
        owner.map(|id| id.to_string()),
        Some(BOOTSTRAP_ADMIN.to_string()),
        "backfill must claim the legacy document for the bootstrap admin"
    );

    Ok(())
}

// ---------------------------------------------------------------------------
// audit-on-write
// ---------------------------------------------------------------------------

async fn create_doc(router: &axum::Router, title: &str) -> anyhow::Result<()> {
    let response = router
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/documents")
                .header("content-type", "application/json")
                .header("x-api-key", "test-secret-key")
                .body(Body::from(format!(
                    r##"{{"title":"{title}","bodyMarkdown":"# Hi"}}"##
                )))?,
        )
        .await?;
    assert_eq!(response.status(), StatusCode::CREATED);
    Ok(())
}

/// Create / publish / delete with the shared key each write exactly one
/// `write_audit` row with the right action and `actor_label = 'shared-key'`.
#[tokio::test]
async fn mutations_emit_one_audit_row_each() -> anyhow::Result<()> {
    let _guard = db_guard().await;
    let Some(pool) = common::maybe_pool().await? else {
        return Ok(());
    };
    let router = common::router_for(pool.clone());

    let audit_count = async |action: &str, slug: &str| -> anyhow::Result<i64> {
        Ok(sqlx::query_scalar(
            "SELECT count(*) FROM write_audit \
             WHERE action = $1 AND slug = $2 AND actor_label = 'shared-key'",
        )
        .bind(action)
        .bind(slug)
        .fetch_one(&pool)
        .await?)
    };

    // create
    create_doc(&router, "Audit Me").await?;
    assert_eq!(audit_count("create", "audit-me").await?, 1);

    // publish
    let response = router
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/documents/audit-me/publish")
                .header("x-api-key", "test-secret-key")
                .body(Body::empty())?,
        )
        .await?;
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(audit_count("publish", "audit-me").await?, 1);

    // delete
    let response = router
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::DELETE)
                .uri("/documents/audit-me")
                .header("x-api-key", "test-secret-key")
                .body(Body::empty())?,
        )
        .await?;
    assert_eq!(response.status(), StatusCode::NO_CONTENT);
    // The audit row survives the document's deletion (document_id is not an FK).
    assert_eq!(audit_count("delete", "audit-me").await?, 1);

    // Every audit row is attributed to the bootstrap admin in slice 1.
    let actor: Option<Uuid> = sqlx::query_scalar(
        "SELECT actor_author_id FROM write_audit WHERE slug = 'audit-me' LIMIT 1",
    )
    .fetch_one(&pool)
    .await?;
    assert_eq!(
        actor.map(|id| id.to_string()),
        Some(BOOTSTRAP_ADMIN.to_string())
    );

    Ok(())
}

// ---------------------------------------------------------------------------
// no regression: auth + visibility unchanged
// ---------------------------------------------------------------------------

/// Slice 1 is inert: shared-key writes still succeed (201), unauthenticated
/// writes are still 401, and a public read still sees only published notes —
/// the existing behavior is byte-for-byte unchanged.
#[tokio::test]
async fn auth_and_visibility_are_unchanged() -> anyhow::Result<()> {
    let _guard = db_guard().await;
    let Some(router) = common::maybe_router().await? else {
        return Ok(());
    };

    // shared key still creates (201).
    create_doc(&router, "Still Works").await?;

    // unauthenticated write still 401.
    let response = router
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/documents")
                .header("content-type", "application/json")
                .body(Body::from(r##"{"title":"No Key","bodyMarkdown":"# Hi"}"##))?,
        )
        .await?;
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);

    // A draft is NOT visible to a public (unauthenticated) reader.
    let response = router
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::GET)
                .uri("/documents/still-works")
                .body(Body::empty())?,
        )
        .await?;
    assert_eq!(
        response.status(),
        StatusCode::NOT_FOUND,
        "public reads must stay published-only (no draft leak)"
    );

    // The author (shared key) can still see the draft.
    let response = router
        .oneshot(
            Request::builder()
                .method(Method::GET)
                .uri("/documents/still-works")
                .header("x-api-key", "test-secret-key")
                .body(Body::empty())?,
        )
        .await?;
    assert_eq!(response.status(), StatusCode::OK);
    let body = to_bytes(response.into_body(), usize::MAX).await?;
    let json: serde_json::Value = serde_json::from_slice(&body)?;
    assert_eq!(json["status"], "draft");

    Ok(())
}
