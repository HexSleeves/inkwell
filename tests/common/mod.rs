// Shared test helpers: not every integration-test binary uses every helper,
// so silence dead-code warnings for the ones a given binary skips.
#![allow(dead_code)]

use anyhow::{Result, anyhow};
use async_trait::async_trait;
use inkwell::ai::{Embedder, Llm, MockEmbedder, MockLlm};
use inkwell::config::{Config, MediaBackend};
use inkwell::db::migrations;
use inkwell::db::pool::create_pool;
use inkwell::http::router::{build_router, build_router_with_providers};
use sqlx::PgPool;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use uuid::Uuid;

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
        "TRUNCATE TABLE documents, links, write_audit, author_tokens, media, media_blobs, sessions, slug_aliases RESTART IDENTITY CASCADE",
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
        min_similarity: 0.0,
        // Webmention send stays OFF in tests: the receive path and SSRF guard are
        // what we exercise; send is asserted inert separately.
        webmention_send: false,
        // Browser login stays OFF by default: the flag-on surface is exercised
        // separately in tests/browser_login.rs.
        browser_login: false,
        // Rate limiting OFF by default so existing contract tests can fire many
        // writes under one key without 429s. The rate-limit contract test opts
        // into a low limit via `router_for_with_rate_limit`.
        write_rate_limit: 0,
        // Forwarded-header trust off in tests; IP keying uses the peer address.
        trust_forwarded_headers: false,
        site_title: inkwell::config::DEFAULT_SITE_TITLE.to_string(),
        site_description: None,
        site_author: None,
        custom_css_url: None,
        theme: None,
        // `/metrics` stays unregistered by default, matching production. The
        // observability contract test opts in via `router_for_with_metrics`.
        metrics_enabled: false,
        metrics_token: None,
        // Media: the default local backend, rooted in a throwaway directory so
        // uploads from one test can never be seen (or deleted) by another.
        media_backend: MediaBackend::Local,
        media_dir: media_test_dir().to_string_lossy().into_owned(),
        media_max_bytes: inkwell::config::DEFAULT_MEDIA_MAX_BYTES,
        // Outbound webhooks stay OFF, matching production. Every publish/unpublish
        // test therefore also asserts the default-inert contract; the webhook
        // contract test opts in via `router_for_with_webhooks`.
        webhooks_enabled: false,
        webhook_urls: Vec::new(),
        webhook_secret: None,
    })
}

/// A unique, never-reused temp directory for a test's local media backend.
/// Nothing creates it up front — the store makes shard directories on demand, so
/// a test that uploads nothing leaves nothing behind.
pub fn media_test_dir() -> PathBuf {
    std::env::temp_dir().join(format!("inkwell-test-media-{}", Uuid::new_v4()))
}

/// Build a router whose local media backend is rooted at `media_dir`, so a test
/// can assert what actually landed on disk.
pub fn router_for_with_media_dir(pool: PgPool, media_dir: &Path) -> axum::Router {
    let database_url = std::env::var("DATABASE_URL").unwrap_or_default();
    let mut config = (*test_config(database_url)).clone();
    config.media_backend = MediaBackend::Local;
    config.media_dir = media_dir.to_string_lossy().into_owned();
    build_router(Arc::new(config), pool)
}

/// Build a router with an explicit media backend, mirroring an operator setting
/// `INKWELL_MEDIA_BACKEND`. Used to prove both backends satisfy the same
/// upload/serve contract.
pub fn router_for_with_media_backend(pool: PgPool, media_backend: MediaBackend) -> axum::Router {
    let database_url = std::env::var("DATABASE_URL").unwrap_or_default();
    let mut config = (*test_config(database_url)).clone();
    config.media_backend = media_backend;
    build_router(Arc::new(config), pool)
}

/// Build a router with a custom media size cap (`INKWELL_MEDIA_MAX_BYTES`).
pub fn router_for_with_media_max_bytes(pool: PgPool, media_max_bytes: usize) -> axum::Router {
    let database_url = std::env::var("DATABASE_URL").unwrap_or_default();
    let mut config = (*test_config(database_url)).clone();
    config.media_max_bytes = media_max_bytes;
    build_router(Arc::new(config), pool)
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

/// Build a router with a custom write rate limit (requests/minute) over an
/// already-acquired pool. The shared [`test_config`] disables rate limiting
/// (limit 0); this opts into a low limit so the rate-limit contract test can
/// drive a burst into a 429 without firing dozens of requests.
pub fn router_for_with_rate_limit(pool: PgPool, write_rate_limit: u32) -> axum::Router {
    let database_url = std::env::var("DATABASE_URL").unwrap_or_default();
    let mut config = (*test_config(database_url)).clone();
    config.write_rate_limit = write_rate_limit;
    build_router(Arc::new(config), pool)
}

/// Build a router with `/metrics` registered, optionally behind a scrape token.
/// Mirrors an operator setting `INKWELL_METRICS_ENABLED=true` (plus
/// `INKWELL_METRICS_TOKEN`) without touching process env.
pub fn router_for_with_metrics(pool: PgPool, metrics_token: Option<&str>) -> axum::Router {
    let database_url = std::env::var("DATABASE_URL").unwrap_or_default();
    let mut config = (*test_config(database_url)).clone();
    config.metrics_enabled = true;
    config.metrics_token = metrics_token.map(str::to_string);
    build_router(Arc::new(config), pool)
}

/// Build a router with outbound webhooks enabled, delivering to `urls` and
/// signing with `secret`. Mirrors an operator setting `INKWELL_WEBHOOKS_ENABLED`,
/// `INKWELL_WEBHOOK_URLS`, and `INKWELL_WEBHOOK_SECRET`.
///
/// `/metrics` is registered (unauthenticated) alongside, so a webhook test can
/// assert the delivery counters the same scrape an operator would see. Webhooks
/// stay OFF and `/metrics` unregistered in the shared [`test_config`], so the
/// default-off contract is still asserted by every other test.
pub fn router_for_with_webhooks(pool: PgPool, urls: &[&str], secret: &str) -> axum::Router {
    let database_url = std::env::var("DATABASE_URL").unwrap_or_default();
    let mut config = (*test_config(database_url)).clone();
    config.webhooks_enabled = true;
    config.webhook_urls = urls.iter().map(|url| (*url).to_string()).collect();
    config.webhook_secret = Some(secret.to_string());
    config.metrics_enabled = true;
    build_router(Arc::new(config), pool)
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

/// Mock-AI router pinned to an explicit local media directory.
///
/// [`test_config`] mints a *fresh* media dir per call, which is right for a test
/// that only needs isolation but wrong for one that must read back what an
/// earlier router wrote (backup/restore). Pass the same dir to both routers to
/// share a store, or different dirs to prove blobs actually moved.
pub fn router_for_with_ai_and_media_dir(pool: PgPool, media_dir: &Path) -> axum::Router {
    let database_url = std::env::var("DATABASE_URL").unwrap_or_default();
    let mut config = (*test_config(database_url)).clone();
    config.media_backend = MediaBackend::Local;
    config.media_dir = media_dir.to_string_lossy().into_owned();
    let embedder: Arc<dyn Embedder> = Arc::new(MockEmbedder);
    let llm: Option<Arc<dyn Llm>> = Some(Arc::new(MockLlm));
    build_router_with_providers(Arc::new(config), pool, embedder, llm)
}

/// Build a router whose embedder returns an error on any call. Pass the SAME
/// pool used by a seeding router (built with [`router_for_with_ai`]) so the
/// route operates over real chunk data while the embedder is provably unused.
pub fn router_for_with_failing_embedder(pool: PgPool) -> axum::Router {
    let database_url = std::env::var("DATABASE_URL").unwrap_or_default();
    let embedder: Arc<dyn Embedder> = Arc::new(FailingEmbedder);
    build_router_with_providers(test_config(database_url), pool, embedder, None)
}

// ---------------------------------------------------------------------------
// Scratch databases (backup/restore contract tests)
// ---------------------------------------------------------------------------

/// A throwaway database that stands in for "wipe to a clean volume".
///
/// The backup/restore contract needs a *second* deployment to restore into —
/// truncating the shared test database would only prove that restore can undo
/// its own delete, and would fight the `maybe_pool()` truncation other tests
/// rely on. So each test creates its own database, migrates it (or not, when the
/// point is an unmigrated target), and drops it at the end.
///
/// Call [`ScratchDb::cleanup`] when done. A leaked scratch database is harmless
/// — names are unique per process and test — but leaving them behind on a
/// developer machine is rude.
pub struct ScratchDb {
    pub url: String,
    name: String,
    maintenance_url: String,
}

impl ScratchDb {
    /// Create and migrate an empty deployment. Returns `Ok(None)` when
    /// `DATABASE_URL` is unset, matching [`maybe_pool`]'s skip behaviour.
    pub async fn create_migrated(label: &str) -> Result<Option<Self>> {
        let Some(scratch) = Self::create(label).await? else {
            return Ok(None);
        };
        let pool = create_pool(&scratch.url)?;
        migrations::migrate(&pool).await?;
        pool.close().await;
        Ok(Some(scratch))
    }

    /// Create an empty database with no schema at all.
    pub async fn create(label: &str) -> Result<Option<Self>> {
        let Ok(database_url) = std::env::var("DATABASE_URL") else {
            if std::env::var("INKWELL_REQUIRE_DB_TESTS").as_deref() == Ok("1") {
                return Err(anyhow!(
                    "DATABASE_URL is required when INKWELL_REQUIRE_DB_TESTS=1"
                ));
            }
            eprintln!("Skipping scratch-database test: set DATABASE_URL to run it locally.");
            return Ok(None);
        };

        // Unique per process and per call so parallel test binaries can't collide.
        static COUNTER: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
        let seq = COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let name = format!("inkwell_scratch_{label}_{}_{seq}", std::process::id());

        let mut url = url::Url::parse(&database_url)?;
        // `CREATE DATABASE` cannot run from inside the database being created,
        // so issue it against the `postgres` maintenance database.
        url.set_path("/postgres");
        let maintenance_url = url.to_string();

        let admin = create_pool(&maintenance_url)?;
        // Identifier is built from a fixed prefix + pid + counter, so it cannot
        // contain a quote; guard anyway since this is concatenated SQL.
        assert!(
            name.chars()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_'),
            "scratch database name must be a bare identifier: {name}"
        );
        sqlx::query(sqlx::AssertSqlSafe(format!(
            "DROP DATABASE IF EXISTS \"{name}\" WITH (FORCE)"
        )))
        .execute(&admin)
        .await?;
        sqlx::query(sqlx::AssertSqlSafe(format!("CREATE DATABASE \"{name}\"")))
            .execute(&admin)
            .await?;
        admin.close().await;

        let mut scratch_url = url::Url::parse(&database_url)?;
        scratch_url.set_path(&format!("/{name}"));

        Ok(Some(Self {
            url: scratch_url.to_string(),
            name,
            maintenance_url,
        }))
    }

    pub async fn cleanup(self) -> Result<()> {
        let admin = create_pool(&self.maintenance_url)?;
        sqlx::query(sqlx::AssertSqlSafe(format!(
            "DROP DATABASE IF EXISTS \"{}\" WITH (FORCE)",
            self.name
        )))
        .execute(&admin)
        .await?;
        admin.close().await;
        Ok(())
    }
}
