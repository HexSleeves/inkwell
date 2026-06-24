//! Database-backed contract tests for the scoped-token TIGHTENING
//! (ADR 0009, plan 023, **slice 4**).
//!
//! Slice 4: `documents.owner_id` is `NOT NULL` (the DB default is kept as a
//! safety net so non-API insert paths still attribute to the bootstrap admin),
//! and the separate `INKWELL_MCP_KEY` is retired — the MCP server now
//! authenticates with `INKWELL_API_KEY`, which operators set to a scoped token.
//! This test proves a scoped token drives the MCP client surface end-to-end.
//!
//! Skipped unless `DATABASE_URL` is set (or forced via `INKWELL_REQUIRE_DB_TESTS=1`).

mod common;

use axum::body::{Body, to_bytes};
use http::{Method, Request, StatusCode};
use inkwell::client::InkwellClient;
use std::sync::LazyLock;
use tokio::sync::{Mutex, MutexGuard};
use tower::ServiceExt;
use uuid::Uuid;

static DB_TEST_LOCK: LazyLock<Mutex<()>> = LazyLock::new(|| Mutex::new(()));

async fn db_guard() -> MutexGuard<'static, ()> {
    DB_TEST_LOCK.lock().await
}

const SHARED_KEY: &str = "test-secret-key";
const BOOTSTRAP_ADMIN: &str = "00000000-0000-0000-0000-000000000001";

/// `owner_id` is `NOT NULL`, yet an insert path that omits it (raw SQL, seed,
/// maintenance) still succeeds — the kept DB default attributes it to the
/// bootstrap admin rather than violating the constraint.
#[tokio::test]
async fn owner_id_not_null_with_admin_default() -> anyhow::Result<()> {
    let _guard = db_guard().await;
    let Some(pool) = common::maybe_pool().await? else {
        return Ok(());
    };

    // The column is NOT NULL after migration 0017.
    let is_nullable: String = sqlx::query_scalar(
        "SELECT is_nullable FROM information_schema.columns \
         WHERE table_name = 'documents' AND column_name = 'owner_id'",
    )
    .fetch_one(&pool)
    .await?;
    assert_eq!(is_nullable, "NO");

    // A raw insert that omits owner_id still works: the default fills it.
    let owner: Uuid = sqlx::query_scalar(
        "INSERT INTO documents (slug, title, body_markdown, rendered_html) \
         VALUES ('raw-insert', 'Raw', '# Raw', '<h1>Raw</h1>') RETURNING owner_id",
    )
    .fetch_one(&pool)
    .await?;
    assert_eq!(
        owner.to_string(),
        BOOTSTRAP_ADMIN,
        "omitted owner_id defaults to the bootstrap admin (no NOT NULL violation)"
    );

    Ok(())
}

/// The MCP path is `InkwellClient` over HTTP authenticating with `INKWELL_API_KEY`.
/// A scoped `read,write` token minted via the admin surface drives create + read,
/// proving `INKWELL_MCP_KEY`'s retirement: MCP runs on a scoped token.
#[tokio::test]
async fn scoped_token_drives_the_mcp_client_surface() -> anyhow::Result<()> {
    let _guard = db_guard().await;
    let Some(pool) = common::maybe_pool().await? else {
        return Ok(());
    };
    let router = common::router_for(pool.clone());

    // Mint a scoped token over the admin surface (shared key).
    let mint = router
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/admin/tokens")
                .header("content-type", "application/json")
                .header("x-api-key", SHARED_KEY)
                .body(Body::from(serde_json::to_vec(
                    &serde_json::json!({ "name": "mcp-agent", "scopes": ["read", "write"] }),
                )?))?,
        )
        .await?;
    assert_eq!(mint.status(), StatusCode::CREATED);
    let token = {
        let bytes = to_bytes(mint.into_body(), usize::MAX).await?;
        let json: serde_json::Value = serde_json::from_slice(&bytes)?;
        json["token"].as_str().expect("token").to_string()
    };

    // Bind the real router on an ephemeral port; point an InkwellClient (the MCP
    // transport) at it, authenticating with the scoped TOKEN — no MCP key exists.
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    let addr = listener.local_addr()?;
    let _server = tokio::spawn(async move {
        axum::serve(listener, router).await.unwrap();
    });
    let client = InkwellClient::new(format!("http://{addr}"), token)?;

    // create + read round-trip succeeds on the scoped token.
    let created = client
        .create_note(&inkwell::client::DocumentInput {
            title: "Agent Note".to_string(),
            slug: "agent-note".to_string(),
            body: "# Agent".to_string(),
            tags: vec![],
            growth: None,
        })
        .await?;
    assert_eq!(created.slug, "agent-note");

    let read = client.read_note("agent-note").await?.expect("note exists");
    assert_eq!(read.title, "Agent Note");

    // The note is owned by the token's author, not the bootstrap admin.
    let owner: Uuid =
        sqlx::query_scalar("SELECT owner_id FROM documents WHERE slug = 'agent-note'")
            .fetch_one(&pool)
            .await?;
    let agent_id: Uuid = sqlx::query_scalar("SELECT id FROM authors WHERE name = 'mcp-agent'")
        .fetch_one(&pool)
        .await?;
    assert_eq!(owner, agent_id);

    Ok(())
}
