//! Database-backed contract tests for `inkwell seed` (card T8).
//!
//! Skipped unless `DATABASE_URL` is set (or forced via `INKWELL_REQUIRE_DB_TESTS=1`),
//! matching the rest of the db-backed suite. They assert the two properties the
//! one-command demo relies on:
//!   - seeded notes are **published** (so they are publicly visible), and
//!   - seeding is **idempotent** — running it twice does not duplicate content.

mod common;

use inkwell::cli::seed;
use inkwell::db::documents::{count_documents, get_document_by_slug, list_documents};
use inkwell::domain::document::{DocumentStatus, ListOptions, StatusFilter};
use std::path::PathBuf;
use std::sync::LazyLock;
use tokio::sync::{Mutex, MutexGuard};

/// These tests share one database and `maybe_pool` truncates it on entry, so
/// they must not run concurrently. Cargo runs separate test binaries
/// sequentially; this serializes the tests within this binary.
static DB_TEST_LOCK: LazyLock<Mutex<()>> = LazyLock::new(|| Mutex::new(()));

async fn db_guard() -> MutexGuard<'static, ()> {
    DB_TEST_LOCK.lock().await
}

/// The bundled sample vault shipped in the repo. Tests seed from this exact
/// directory the Docker image copies into the runtime.
fn sample_vault() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("examples/garden")
}

#[tokio::test]
async fn seed_creates_published_notes_with_backlinks() -> anyhow::Result<()> {
    let _guard = db_guard().await;
    let Some(pool) = common::maybe_pool().await? else {
        return Ok(());
    };

    seed::seed(&pool, &sample_vault()).await?;

    // The sample vault is non-trivial and every note is published.
    let total = count_documents(&pool, StatusFilter::default()).await?;
    assert!(total >= 5, "expected the bundled sample notes, got {total}");
    let published = count_documents(
        &pool,
        StatusFilter {
            status: Some(DocumentStatus::Published),
        },
    )
    .await?;
    assert_eq!(
        published, total,
        "every seeded note must be published so the demo garden is visible"
    );

    // A known seeded note exists, is published, and renders a real (resolved)
    // wikilink to a sibling — not a stub — so the demo has live links.
    let welcome = get_document_by_slug(&pool, "welcome", StatusFilter::default())
        .await?
        .expect("welcome note should be seeded");
    assert_eq!(welcome.status, DocumentStatus::Published);
    assert!(
        welcome
            .rendered_html
            .contains("href=\"/what-is-a-digital-garden\""),
        "wikilink between seeded notes should resolve to a real link, got: {}",
        welcome.rendered_html
    );

    Ok(())
}

#[tokio::test]
async fn seed_is_idempotent_and_does_not_duplicate() -> anyhow::Result<()> {
    let _guard = db_guard().await;
    let Some(pool) = common::maybe_pool().await? else {
        return Ok(());
    };

    seed::seed(&pool, &sample_vault()).await?;
    let first = list_documents(&pool, ListOptions::default()).await?;
    assert!(!first.is_empty(), "first seed should create notes");

    // Seeding again must no-op: the documents table is non-empty.
    seed::seed(&pool, &sample_vault()).await?;
    let second = list_documents(&pool, ListOptions::default()).await?;

    assert_eq!(
        first.len(),
        second.len(),
        "re-seeding must not duplicate notes"
    );

    // Slugs are unique and unchanged across the two runs.
    let mut first_slugs: Vec<&str> = first.iter().map(|d| d.slug.as_str()).collect();
    let mut second_slugs: Vec<&str> = second.iter().map(|d| d.slug.as_str()).collect();
    first_slugs.sort_unstable();
    second_slugs.sort_unstable();
    assert_eq!(first_slugs, second_slugs);

    Ok(())
}
