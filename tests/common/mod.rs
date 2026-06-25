// Shared test helpers: not every integration-test binary uses every helper,
// so silence dead-code warnings for the ones a given binary skips.
#![allow(dead_code)]

use anyhow::{Result, anyhow};
use async_trait::async_trait;
use inkwell::ai::{Embedder, Llm, MockEmbedder, MockLlm};
use inkwell::config::Config;
use inkwell::db::migrations;
use inkwell::db::pool::create_pool;
use inkwell::http::router::{build_router, build_router_with_providers};
use sqlx::PgPool;
use std::sync::Arc;

/// Test-only embedder that returns an error if `embed()` is called. Use to
/// prove that a route does NOT invoke the embedder (e.g. `/documents/{slug}/related`
/// after the stored-chunk refactor). With the old body-embedding implementation,
/// this would cause `/related` to return `related: []`; with the new stored-chunk
/// implementation it is never called and the route returns real results.
///
/// Reports the same provider/model as [`MockEmbedder`] so that chunks seeded by
/// a mock-AI router (via [`router_for_with_ai`]) match the provenance filter in
/// retrieval functions — this lets the test verify that stored chunks ARE
/// returned without ever calling the embedder.
struct FailingEmbedder;

#[async_trait]
impl Embedder for FailingEmbedder {
    fn provider(&self) -> &'static str {
        "mock"
    }

    fn model(&self) -> &str {
        "mock-hash-v1"
    }

    async fn embed(&self, _texts: &[String]) -> anyhow::Result<Vec<Vec<f32>>> {
        Err(anyhow::anyhow!(
            "FailingEmbedder: embed() must not be called on this route"
        ))
    }
}

pub async fn maybe_pool() -> Result<Option<PgPool>> {
    let Ok(database_url) = std::env::var("DATABASE_URL") else {
        if std::env::var("INKWELL_REQUIRE_DB_TESTS").as_deref() == Ok("1") {
            return Err(anyhow!(
                "DATABASE_URL is required for database-backed contract tests when INKWELL_REQUIRE_DB_TESTS=1"
            ));
        }
        eprintln!("Skipping database-backed contract tests: set DATABASE_URL to run them locally.");
        return Ok(None);
    };
    let pool = create_pool(&database_url)?;
    migrations::migrate(&pool).await?;
    // `links` (and now `write_audit.document_id`/`documents.owner_id`) reference
    // `documents`, so truncate them together with CASCADE or the referencing
    // constraints error. `authors` is deliberately NOT truncated: it holds the
    // seeded bootstrap admin that owns backfilled docs and is the audit actor.
    // `author_tokens` cascades from `authors` but is cleared explicitly so a test
    // that mints a token starts clean without disturbing the seeded author.
    sqlx::query(
        "TRUNCATE TABLE documents, links, write_audit, author_tokens, media, sessions, slug_aliases RESTART IDENTITY CASCADE",
    )
    .execute(&pool)
    .await?;
    Ok(Some(pool))
}

pub fn test_config(database_url: String) -> Arc<Config> {
    Arc::new(Config {
        database_url,
        host: "127.0.0.1".to_string(),
        port: 3000,
        api_key: Some("test-secret-key".to_string()),
        site_url: Some("https://blog.example.com".to_string()),
        // AI features unconfigured by default: the router falls back to the
        // deterministic mock embedder for retrieval and reports "AI features not
        // configured" for synthesis. Tests that exercise the AI surfaces build
        // their own AppState with the mock LLM wired in.
        voyage_api_key: None,
        anthropic_api_key: None,
        llm_model: inkwell::config::DEFAULT_LLM_MODEL.to_string(),
        // Webmention send stays OFF in tests: the receive path and SSRF guard are
        // what we exercise; send is asserted inert separately.
        webmention_send: false,
        // Browser login stays OFF by default: the flag-on surface is exercised
        // separately in tests/browser_login.rs.
        browser_login: false,
    })
}

pub async fn maybe_router() -> Result<Option<axum::Router>> {
    let Some(pool) = maybe_pool().await? else {
        return Ok(None);
    };
    Ok(Some(build_router(
        test_config(std::env::var("DATABASE_URL")?),
        pool,
    )))
}

/// Build a router from an already-acquired pool, reusing the shared
/// [`test_config`]. Lets a test do its own setup against `pool` (e.g. seed
/// documents/webmentions) and then exercise the HTTP surface over the SAME pool.
pub fn router_for(pool: PgPool) -> axum::Router {
    let database_url = std::env::var("DATABASE_URL").unwrap_or_default();
    build_router(test_config(database_url), pool)
}

/// Router wired with the deterministic mock embedder AND mock LLM, so the eval
/// suite can exercise the full RAG path (embedding on write, vector retrieval,
/// answer synthesis) with no API keys. Mirrors [`maybe_router`] otherwise.
pub async fn maybe_router_with_ai() -> Result<Option<axum::Router>> {
    let Some(pool) = maybe_pool().await? else {
        return Ok(None);
    };
    let embedder: Arc<dyn Embedder> = Arc::new(MockEmbedder);
    let llm: Option<Arc<dyn Llm>> = Some(Arc::new(MockLlm));
    Ok(Some(build_router_with_providers(
        test_config(std::env::var("DATABASE_URL")?),
        pool,
        embedder,
        llm,
    )))
}

/// Build a router with the mock AI providers against an already-acquired pool.
/// Lets tests seed documents through a normal mock-AI router, then hand the
/// SAME pool to a second router (e.g. [`router_for_with_failing_embedder`]) to
/// exercise a route's behavior over the populated database.
pub fn router_for_with_ai(pool: PgPool) -> axum::Router {
    let database_url = std::env::var("DATABASE_URL").unwrap_or_default();
    let embedder: Arc<dyn Embedder> = Arc::new(MockEmbedder);
    let llm: Option<Arc<dyn Llm>> = Some(Arc::new(MockLlm));
    build_router_with_providers(test_config(database_url), pool, embedder, llm)
}

/// Build a router whose embedder returns an error on any call. Pass the SAME
/// pool used by a seeding router (built with [`router_for_with_ai`]) so the
/// route operates over real chunk data while the embedder is provably unused.
pub fn router_for_with_failing_embedder(pool: PgPool) -> axum::Router {
    let database_url = std::env::var("DATABASE_URL").unwrap_or_default();
    let embedder: Arc<dyn Embedder> = Arc::new(FailingEmbedder);
    build_router_with_providers(test_config(database_url), pool, embedder, None)
}
