// Shared test helpers: not every integration-test binary uses every helper,
// so silence dead-code warnings for the ones a given binary skips.
#![allow(dead_code)]

use anyhow::{Result, anyhow};
use inkwell::ai::{Embedder, Llm, MockEmbedder, MockLlm};
use inkwell::config::Config;
use inkwell::db::migrations;
use inkwell::db::pool::create_pool;
use inkwell::http::router::{build_router, build_router_with_providers};
use sqlx::PgPool;
use std::sync::Arc;

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
        "TRUNCATE TABLE documents, links, write_audit, author_tokens RESTART IDENTITY CASCADE",
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
