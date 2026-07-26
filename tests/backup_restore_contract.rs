//! Database-backed contract tests for `inkwell backup` / `inkwell restore`
//! (CYP-49).
//!
//! The headline test is a real disaster-recovery rehearsal: seed a garden with
//! documents, tags, wikilinks, embedded media, and embedding chunks; publish some
//! of it; take a backup; restore into a *different, freshly created* database
//! (our stand-in for "wipe to a clean volume"); then re-issue the same HTTP
//! requests against the restored deployment and assert the responses are
//! byte-identical.
//!
//! The other tests pin the safety rails: no silent clobber, empty round-trips,
//! and refusal of a bundle from a newer schema.
//!
//! Skipped unless `DATABASE_URL` is set (or forced via `INKWELL_REQUIRE_DB_TESTS=1`).

mod common;

use anyhow::Result;
use axum::body::{Body, to_bytes};
use http::{Method, Request, StatusCode};
use inkwell::backup;
use inkwell::db::pool::create_pool;
use inkwell::media::local::LocalFsStore;
use pretty_assertions::assert_eq;
use sqlx::PgPool;
use std::path::{Path, PathBuf};
use std::sync::LazyLock;
use tokio::sync::{Mutex, MutexGuard};
use tower::ServiceExt;

/// Serialise db-backed tests in this binary; `maybe_pool` truncates on entry.
static DB_TEST_LOCK: LazyLock<Mutex<()>> = LazyLock::new(|| Mutex::new(()));

async fn db_guard() -> MutexGuard<'static, ()> {
    DB_TEST_LOCK.lock().await
}

const SHARED_KEY: &str = "test-secret-key";

/// A 1×1 red PNG, same fixture the media contract test uses.
fn tiny_png() -> Vec<u8> {
    vec![
        0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a, 0x00, 0x00, 0x00, 0x0d, 0x49, 0x48, 0x44,
        0x52, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01, 0x08, 0x02, 0x00, 0x00, 0x00, 0x90,
        0x77, 0x53, 0xde, 0x00, 0x00, 0x00, 0x0c, 0x49, 0x44, 0x41, 0x54, 0x08, 0xd7, 0x63, 0xf8,
        0xcf, 0xc0, 0x00, 0x00, 0x00, 0x02, 0x00, 0x01, 0xe2, 0x21, 0xbc, 0x33, 0x00, 0x00, 0x00,
        0x00, 0x49, 0x45, 0x4e, 0x44, 0xae, 0x42, 0x60, 0x82,
    ]
}

// ---------------------------------------------------------------------------
// HTTP helpers
// ---------------------------------------------------------------------------

async fn get(router: &axum::Router, uri: &str) -> Result<(StatusCode, Option<String>, Vec<u8>)> {
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
    let status = response.status();
    let content_type = response
        .headers()
        .get(http::header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .map(str::to_string);
    let bytes = to_bytes(response.into_body(), usize::MAX).await?.to_vec();
    Ok((status, content_type, bytes))
}

async fn post_json(
    router: &axum::Router,
    uri: &str,
    payload: serde_json::Value,
) -> Result<(StatusCode, serde_json::Value)> {
    let response = router
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri(uri)
                .header("x-api-key", SHARED_KEY)
                .header("content-type", "application/json")
                .body(Body::from(serde_json::to_vec(&payload)?))?,
        )
        .await?;
    let status = response.status();
    let bytes = to_bytes(response.into_body(), usize::MAX).await?;
    let json = if bytes.is_empty() {
        serde_json::Value::Null
    } else {
        serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null)
    };
    Ok((status, json))
}

async fn upload_png(router: &axum::Router) -> Result<String> {
    upload_bytes(router, "image/png", tiny_png()).await
}

/// A minimal valid 1×1 transparent GIF — distinct bytes from [`tiny_png`], so a
/// test can create a *second* blob that a bundle does not contain.
fn tiny_gif() -> Vec<u8> {
    vec![
        0x47, 0x49, 0x46, 0x38, 0x39, 0x61, 0x01, 0x00, 0x01, 0x00, 0x80, 0x00, 0x00, 0x00, 0x00,
        0x00, 0xff, 0xff, 0xff, 0x21, 0xf9, 0x04, 0x01, 0x00, 0x00, 0x00, 0x00, 0x2c, 0x00, 0x00,
        0x00, 0x00, 0x01, 0x00, 0x01, 0x00, 0x00, 0x02, 0x02, 0x44, 0x01, 0x00, 0x3b,
    ]
}

async fn upload_bytes(router: &axum::Router, content_type: &str, bytes: Vec<u8>) -> Result<String> {
    let response = router
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/media")
                .header("x-api-key", SHARED_KEY)
                .header("content-type", content_type)
                .body(Body::from(bytes))?,
        )
        .await?;
    assert_eq!(response.status(), StatusCode::CREATED);
    let bytes = to_bytes(response.into_body(), usize::MAX).await?;
    let json: serde_json::Value = serde_json::from_slice(&bytes)?;
    Ok(json
        .get("url")
        .and_then(|url| url.as_str())
        .expect("upload response must carry a url")
        .to_string())
}

// ---------------------------------------------------------------------------
// Seeding + verification
// ---------------------------------------------------------------------------

/// Everything the restored deployment must reproduce.
struct Fixture {
    media_url: String,
}

/// Seed a small garden: two published notes that wikilink to each other, one
/// draft, tags, and an embedded image. Uses the mock-AI router so `note_chunks`
/// (pgvector `vector(1024)` values) are written too — those exercise the one
/// column type whose JSON round-trip is not obvious.
async fn seed_garden(pool: PgPool, media_dir: &Path) -> Result<Fixture> {
    let router = common::router_for_with_ai_and_media_dir(pool, media_dir);

    let media_url = upload_png(&router).await?;

    let (status, _) = post_json(
        &router,
        "/documents",
        serde_json::json!({
            "title": "Ferns and Fronds",
            "bodyMarkdown": format!(
                "# Ferns and Fronds\n\nShade plants for a damp corner. See [[Moss]].\n\n![a fern](<{media_url}>)\n"
            ),
            "tags": ["garden", "plants"],
        }),
    )
    .await?;
    assert_eq!(status, StatusCode::CREATED);

    let (status, _) = post_json(
        &router,
        "/documents",
        serde_json::json!({
            "title": "Moss",
            "bodyMarkdown": "# Moss\n\nGrows where ferns will not. Back to [[Ferns and Fronds]].\n",
            "tags": ["garden"],
        }),
    )
    .await?;
    assert_eq!(status, StatusCode::CREATED);

    // A draft, so the backup has to preserve unpublished state too.
    let (status, _) = post_json(
        &router,
        "/documents",
        serde_json::json!({
            "title": "Compost Notes",
            "bodyMarkdown": "# Compost Notes\n\nStill turning this one over.\n",
            "tags": ["garden", "soil"],
        }),
    )
    .await?;
    assert_eq!(status, StatusCode::CREATED);

    for slug in ["ferns-and-fronds", "moss"] {
        let (status, _) = post_json(
            &router,
            &format!("/documents/{slug}/publish"),
            serde_json::Value::Null,
        )
        .await?;
        assert_eq!(status, StatusCode::OK, "publishing {slug}");
    }

    Ok(Fixture { media_url })
}

/// Every read surface the acceptance criteria name: published pages, tags,
/// collections (archive/index), the search index, media URLs, and the graph.
fn verification_uris(fixture: &Fixture) -> Vec<String> {
    let mut uris: Vec<String> = [
        "/",
        "/ferns-and-fronds",
        "/moss",
        "/tags",
        "/tags/garden",
        "/tags/plants",
        "/archive",
        "/notes",
        "/search?q=ferns&format=json",
        "/search?q=moss&format=json",
        "/search?q=compost&format=json",
        "/graph",
        "/documents",
        "/documents/ferns-and-fronds",
        "/documents/compost-notes",
        "/documents/ferns-and-fronds/related",
        "/feed.xml",
        "/sitemap.xml",
    ]
    .iter()
    .map(|uri| (*uri).to_string())
    .collect();
    uris.push(fixture.media_url.clone());
    uris
}

/// Per-table row counts, the generic fidelity check: it covers every table in
/// the backup set, including ones this test does not exercise over HTTP.
async fn table_counts(pool: &PgPool) -> Result<Vec<(String, i64)>> {
    let mut counts = Vec::new();
    for table in backup::TABLES {
        let count: i64 = sqlx::query_scalar(sqlx::AssertSqlSafe(format!(
            "SELECT count(*) FROM public.\"{table}\""
        )))
        .fetch_one(pool)
        .await?;
        counts.push((table.to_string(), count));
    }
    Ok(counts)
}

fn bundle_path(label: &str) -> PathBuf {
    std::env::temp_dir().join(format!(
        "inkwell-backup-test-{label}-{}.inkwell.gz",
        std::process::id()
    ))
}

/// Blank out the per-response CSP nonce so two renders of the same document
/// compare equal.
///
/// The nonce is freshly random on every response by design — it is the one part
/// of a page that *must* differ between two identical requests, so comparing it
/// would assert the opposite of what we want. Everything else, timestamps and
/// media ids included, is expected byte-for-byte identical.
fn normalise_nonces(body: &[u8]) -> String {
    let text = String::from_utf8_lossy(body);
    let mut out = String::with_capacity(text.len());
    let mut rest = text.as_ref();
    while let Some(start) = rest.find("nonce=\"") {
        let (head, tail) = rest.split_at(start + "nonce=\"".len());
        out.push_str(head);
        match tail.find('"') {
            Some(end) => {
                out.push_str("<nonce>");
                rest = &tail[end..];
            }
            None => {
                rest = tail;
                break;
            }
        }
    }
    out.push_str(rest);
    out
}

/// Count blob files under a local media root, so a test can prove bytes actually
/// moved rather than trusting that the URL happened to resolve.
fn blob_file_count(root: &Path) -> usize {
    fn walk(dir: &Path, found: &mut usize) {
        let Ok(entries) = std::fs::read_dir(dir) else {
            return;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                walk(&path, found);
            } else if !path
                .file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with(".tmp-"))
            {
                *found += 1;
            }
        }
    }
    let mut found = 0;
    walk(root, &mut found);
    found
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

/// The rehearsal: seed → publish → backup → clean volume → restore → every read
/// surface answers identically, and every table has the same row count.
#[tokio::test]
async fn backup_then_restore_into_a_clean_deployment_reproduces_every_read_surface() -> Result<()> {
    let _guard = db_guard().await;
    let Some(pool) = common::maybe_pool().await? else {
        return Ok(());
    };
    let Some(scratch) = common::ScratchDb::create_migrated("roundtrip").await? else {
        return Ok(());
    };

    // Two separate media roots: the source deployment's, and the "clean volume"
    // the restore has to repopulate. If blobs did not travel in the bundle, the
    // restored `/media/{id}` would 404.
    let source_media = common::media_test_dir();
    let restored_media = common::media_test_dir();

    let fixture = seed_garden(pool.clone(), &source_media).await?;
    let uris = verification_uris(&fixture);

    // Capture the "before" responses from the live deployment.
    let source_router = common::router_for_with_ai_and_media_dir(pool.clone(), &source_media);
    let mut before = Vec::new();
    for uri in &uris {
        let (status, content_type, body) = get(&source_router, uri).await?;
        assert!(
            status.is_success(),
            "source {uri} should be readable, got {status}"
        );
        before.push((uri.clone(), status, content_type, body));
    }
    let source_counts = table_counts(&pool).await?;

    // Back up.
    let path = bundle_path("roundtrip");
    let source_store = LocalFsStore::new(source_media.clone());
    let summary = backup::create::run(&pool, &source_store, Some(path.clone())).await?;
    assert_eq!(
        summary.manifest.bundle_format,
        backup::BUNDLE_FORMAT,
        "manifest must record the format it was written in"
    );
    assert_eq!(
        summary.manifest.schema_version,
        inkwell::db::migrations::latest_known_schema_version(),
        "manifest must record the source schema version"
    );
    assert_eq!(
        summary.manifest.inkwell_version,
        env!("CARGO_PKG_VERSION"),
        "manifest must record the writing binary's version"
    );
    assert!(
        summary.manifest.rows_for("media") >= 1,
        "the media metadata row must be part of the bundle"
    );
    assert_eq!(
        summary.blobs_written, 1,
        "the uploaded image's bytes must be in the bundle, read through the store"
    );
    assert_eq!(summary.manifest.media_backend, "local");
    assert!(
        summary.manifest.rows_for("note_chunks") >= 1,
        "embedding chunks must be part of the bundle so retrieval survives restore"
    );
    let total: i64 = summary.manifest.tables.iter().map(|table| table.rows).sum();
    assert_eq!(
        summary.rows_written, total,
        "rows written must match the manifest's counts"
    );

    // Restore into the clean deployment, with an empty media root.
    assert_eq!(
        blob_file_count(&restored_media),
        0,
        "the restore target must start with no blobs at all"
    );
    let restored_pool = create_pool(&scratch.url)?;
    let restored_store = LocalFsStore::new(restored_media.clone());
    let restore = backup::restore::run(
        &restored_pool,
        &restored_store,
        Some(path.clone()),
        backup::restore::RestoreOptions { overwrite: false },
    )
    .await?;
    assert_eq!(restore.rows_restored, summary.rows_written);
    assert_eq!(restore.blobs_restored, summary.blobs_written);
    assert_eq!(
        restore.blobs_removed, 0,
        "an empty target has nothing to remove"
    );
    assert_eq!(
        restore.warnings,
        Vec::<String>::new(),
        "a same-version restore must not warn about schema drift"
    );
    assert_eq!(
        blob_file_count(&restored_media),
        1,
        "the blob must have been written into the previously empty media root"
    );

    // Every read surface must answer identically.
    let restored_router =
        common::router_for_with_ai_and_media_dir(restored_pool.clone(), &restored_media);
    for (uri, status, content_type, body) in &before {
        let (restored_status, restored_content_type, restored_body) =
            get(&restored_router, uri).await?;
        assert_eq!(&restored_status, status, "status for {uri}");
        assert_eq!(
            &restored_content_type, content_type,
            "content-type for {uri}"
        );
        assert_eq!(
            normalise_nonces(&restored_body),
            normalise_nonces(body),
            "body for {uri}"
        );
    }

    assert_eq!(
        table_counts(&restored_pool).await?,
        source_counts,
        "every backed-up table must hold the same number of rows after restore"
    );

    restored_pool.close().await;
    let _ = std::fs::remove_file(&path);
    let _ = std::fs::remove_dir_all(&source_media);
    let _ = std::fs::remove_dir_all(&restored_media);
    scratch.cleanup().await?;
    Ok(())
}

/// Restoring into a deployment that already holds data must fail loudly and
/// change nothing.
#[tokio::test]
async fn restore_into_a_non_empty_deployment_without_overwrite_fails_and_changes_nothing()
-> Result<()> {
    let _guard = db_guard().await;
    let Some(pool) = common::maybe_pool().await? else {
        return Ok(());
    };
    let Some(scratch) = common::ScratchDb::create_migrated("nonempty").await? else {
        return Ok(());
    };

    // Bundle from the shared deployment...
    let source_media = common::media_test_dir();
    seed_garden(pool.clone(), &source_media).await?;
    let path = bundle_path("nonempty");
    backup::create::run(
        &pool,
        &LocalFsStore::new(source_media.clone()),
        Some(path.clone()),
    )
    .await?;

    // ...and a target that already has *different* content of its own.
    let target_media = common::media_test_dir();
    let target_pool = create_pool(&scratch.url)?;
    let target_router =
        common::router_for_with_ai_and_media_dir(target_pool.clone(), &target_media);
    let (status, _) = post_json(
        &target_router,
        "/documents",
        serde_json::json!({
            "title": "Pre-existing Note",
            "bodyMarkdown": "# Pre-existing Note\n\nDo not clobber me.\n",
            "tags": ["existing"],
        }),
    )
    .await?;
    assert_eq!(status, StatusCode::CREATED);
    let before = table_counts(&target_pool).await?;

    let target_store = LocalFsStore::new(target_media.clone());
    let blobs_before = blob_file_count(&target_media);
    let error = backup::restore::run(
        &target_pool,
        &target_store,
        Some(path.clone()),
        backup::restore::RestoreOptions { overwrite: false },
    )
    .await
    .expect_err("restore into a non-empty deployment must fail without --overwrite");
    let message = error.to_string();
    assert!(
        message.contains("not empty"),
        "error must say the target is not empty: {message}"
    );
    assert!(
        message.contains("--overwrite"),
        "error must name the flag that would allow it: {message}"
    );
    assert!(
        message.contains("Nothing was changed"),
        "error must state that nothing changed: {message}"
    );

    assert_eq!(
        table_counts(&target_pool).await?,
        before,
        "a refused restore must not touch a single row"
    );
    assert_eq!(
        blob_file_count(&target_media),
        blobs_before,
        "a refused restore must not write a single blob either"
    );
    // Read the draft back over the API (the public HTML page is published-only).
    let (status, _, body) = get(&target_router, "/documents/pre-existing-note").await?;
    assert!(
        status.is_success(),
        "the pre-existing draft must still be there"
    );
    assert!(String::from_utf8_lossy(&body).contains("Do not clobber me"));

    target_pool.close().await;
    let _ = std::fs::remove_file(&path);
    let _ = std::fs::remove_dir_all(&source_media);
    let _ = std::fs::remove_dir_all(&target_media);
    scratch.cleanup().await?;
    Ok(())
}

/// With `--overwrite`, the same restore replaces the target's content wholesale.
#[tokio::test]
async fn restore_with_overwrite_replaces_existing_data() -> Result<()> {
    let _guard = db_guard().await;
    let Some(pool) = common::maybe_pool().await? else {
        return Ok(());
    };
    let Some(scratch) = common::ScratchDb::create_migrated("overwrite").await? else {
        return Ok(());
    };

    let source_media = common::media_test_dir();
    seed_garden(pool.clone(), &source_media).await?;
    let path = bundle_path("overwrite");
    let summary = backup::create::run(
        &pool,
        &LocalFsStore::new(source_media.clone()),
        Some(path.clone()),
    )
    .await?;
    let source_counts = table_counts(&pool).await?;

    let target_media = common::media_test_dir();
    let target_pool = create_pool(&scratch.url)?;
    let target_router =
        common::router_for_with_ai_and_media_dir(target_pool.clone(), &target_media);
    let (status, _) = post_json(
        &target_router,
        "/documents",
        serde_json::json!({
            "title": "Doomed Note",
            "bodyMarkdown": "# Doomed Note\n\nThis is about to be replaced.\n",
            "tags": ["doomed"],
        }),
    )
    .await?;
    assert_eq!(status, StatusCode::CREATED);
    // A blob the bundle does not contain, so overwrite has something to reap.
    let doomed_url = upload_bytes(&target_router, "image/gif", tiny_gif()).await?;
    assert_eq!(blob_file_count(&target_media), 1);

    let restore = backup::restore::run(
        &target_pool,
        &LocalFsStore::new(target_media.clone()),
        Some(path.clone()),
        backup::restore::RestoreOptions { overwrite: true },
    )
    .await?;
    assert_eq!(restore.rows_restored, summary.rows_written);
    assert_eq!(restore.blobs_restored, summary.blobs_written);
    assert_eq!(
        restore.blobs_removed, 1,
        "the superseded deployment's blob must not be left readable on disk"
    );
    let (status, _, _) = get(&target_router, &doomed_url).await?;
    assert_eq!(
        status,
        StatusCode::NOT_FOUND,
        "the superseded media URL must stop resolving"
    );

    assert_eq!(
        table_counts(&target_pool).await?,
        source_counts,
        "overwrite must leave exactly the bundle's contents, not a merge"
    );
    let (status, _, _) = get(&target_router, "/doomed-note").await?;
    assert_eq!(
        status,
        StatusCode::NOT_FOUND,
        "the overwritten note must be gone, not merged"
    );
    let (status, _, _) = get(&target_router, "/ferns-and-fronds").await?;
    assert!(status.is_success(), "the bundle's notes must be present");

    target_pool.close().await;
    let _ = std::fs::remove_file(&path);
    let _ = std::fs::remove_dir_all(&source_media);
    let _ = std::fs::remove_dir_all(&target_media);
    scratch.cleanup().await?;
    Ok(())
}

/// Backing up an empty deployment succeeds, and the bundle restores into another
/// empty deployment. The seeded bootstrap admin is the only row either side.
#[tokio::test]
async fn empty_deployment_backs_up_and_restores() -> Result<()> {
    let _guard = db_guard().await;
    let Some(source) = common::ScratchDb::create_migrated("empty_src").await? else {
        return Ok(());
    };
    let Some(target) = common::ScratchDb::create_migrated("empty_dst").await? else {
        source.cleanup().await?;
        return Ok(());
    };

    let source_media = common::media_test_dir();
    let target_media = common::media_test_dir();
    let source_pool = create_pool(&source.url)?;
    let path = bundle_path("empty");
    let summary = backup::create::run(
        &source_pool,
        &LocalFsStore::new(source_media.clone()),
        Some(path.clone()),
    )
    .await?;
    assert_eq!(
        summary.manifest.rows_for("documents"),
        0,
        "an empty deployment has no documents"
    );
    assert_eq!(summary.blobs_written, 0, "an empty deployment has no blobs");
    assert_eq!(
        summary.manifest.rows_for("authors"),
        1,
        "migration 0015 seeds exactly one bootstrap admin"
    );

    let target_pool = create_pool(&target.url)?;
    let restore = backup::restore::run(
        &target_pool,
        &LocalFsStore::new(target_media.clone()),
        Some(path.clone()),
        backup::restore::RestoreOptions { overwrite: false },
    )
    .await?;
    assert_eq!(
        restore.rows_restored, 1,
        "only the bootstrap admin is copied"
    );

    let router = common::router_for_with_ai_and_media_dir(target_pool.clone(), &target_media);
    let (status, _, body) = get(&router, "/documents").await?;
    assert_eq!(status, StatusCode::OK);
    let listing: serde_json::Value = serde_json::from_slice(&body)?;
    assert_eq!(
        listing.get("total").and_then(|total| total.as_i64()),
        Some(0),
        "restored empty deployment must list zero documents"
    );

    source_pool.close().await;
    target_pool.close().await;
    let _ = std::fs::remove_file(&path);
    source.cleanup().await?;
    target.cleanup().await?;
    Ok(())
}

/// A bundle claiming a schema version this binary does not know is refused, and
/// the target is left untouched — including its migration state.
#[tokio::test]
async fn restore_refuses_a_bundle_from_a_newer_schema_and_changes_nothing() -> Result<()> {
    let _guard = db_guard().await;
    let Some(scratch) = common::ScratchDb::create_migrated("future").await? else {
        return Ok(());
    };
    let target_pool = create_pool(&scratch.url)?;

    // Take a real backup of the empty target, then rewrite the manifest's
    // schema version to one past what this binary knows. Everything else about
    // the bundle stays valid, so the refusal is provably about the version.
    let media_dir = common::media_test_dir();
    let store = LocalFsStore::new(media_dir.clone());
    let path = bundle_path("future");
    backup::create::run(&target_pool, &store, Some(path.clone())).await?;
    let future_version = inkwell::db::migrations::latest_known_schema_version() + 1;
    rewrite_manifest_schema_version(&path, future_version)?;

    let before = table_counts(&target_pool).await?;
    let migrations_before = inkwell::db::migrations::status(&target_pool).await?.len();

    let error = backup::restore::run(
        &target_pool,
        &store,
        Some(path.clone()),
        backup::restore::RestoreOptions { overwrite: true },
    )
    .await
    .expect_err("a bundle from a newer schema must be refused");
    let message = error.to_string();
    assert!(
        message.contains(&future_version.to_string()),
        "error must state the bundle's schema version: {message}"
    );
    assert!(
        message.contains(&inkwell::db::migrations::latest_known_schema_version().to_string()),
        "error must state the version this binary knows: {message}"
    );
    assert!(
        message.contains("Nothing was changed"),
        "error must state that nothing changed: {message}"
    );

    assert_eq!(table_counts(&target_pool).await?, before);
    assert_eq!(
        inkwell::db::migrations::status(&target_pool).await?.len(),
        migrations_before
    );

    target_pool.close().await;
    let _ = std::fs::remove_file(&path);
    scratch.cleanup().await?;
    Ok(())
}

/// Rewrite only the `schemaVersion` field of a bundle's manifest line, leaving
/// the rest of the bundle byte-identical.
fn rewrite_manifest_schema_version(path: &std::path::Path, schema_version: i64) -> Result<()> {
    use std::io::{BufRead, BufReader, Write};

    let file = std::fs::File::open(path)?;
    let mut lines = BufReader::new(flate2::read::GzDecoder::new(file))
        .lines()
        .collect::<std::result::Result<Vec<_>, _>>()?;
    let mut manifest: serde_json::Value = serde_json::from_str(&lines[0])?;
    manifest["schemaVersion"] = serde_json::json!(schema_version);
    lines[0] = serde_json::to_string(&manifest)?;

    let mut encoder =
        flate2::write::GzEncoder::new(std::fs::File::create(path)?, flate2::Compression::default());
    for line in lines {
        encoder.write_all(line.as_bytes())?;
        encoder.write_all(b"\n")?;
    }
    encoder.finish()?.flush()?;
    Ok(())
}
