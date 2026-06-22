//! Database-backed contract tests for the P0 link-graph foundation.
//!
//! Skipped unless `DATABASE_URL` is set (or forced via `INKWELL_REQUIRE_DB_TESTS=1`),
//! matching the rest of the db-backed suite.

mod common;

use inkwell::db::documents::{create_document, update_document_by_slug};
use inkwell::db::links::{
    LinkType, NewLink, TargetKind, Visibility, insert_link, notes_to_rerender,
    resolve_existing_slugs,
};
use inkwell::domain::document::{DocumentPatch, DocumentStatus, NewDocument};
use inkwell::garden::{persist_source_edges, render_and_resolve};
use sqlx::Postgres;
use std::sync::LazyLock;
use tokio::sync::{Mutex, MutexGuard};

/// These tests share one database and `maybe_pool` truncates it on entry, so
/// they must not run concurrently (libtest runs a binary's tests on parallel
/// threads). Hold this lock for the whole test to serialize them. Cargo already
/// runs separate test binaries sequentially, so this is sufficient.
static DB_TEST_LOCK: LazyLock<Mutex<()>> = LazyLock::new(|| Mutex::new(()));

async fn db_guard() -> MutexGuard<'static, ()> {
    DB_TEST_LOCK.lock().await
}

fn new_doc(slug: &str) -> NewDocument {
    NewDocument {
        slug: slug.to_string(),
        title: format!("Title {slug}"),
        body_markdown: format!("# {slug}"),
        rendered_html: format!("<h1>{slug}</h1>"),
        status: Some(DocumentStatus::Published),
        tags: Vec::new(),
    }
}

#[tokio::test]
async fn notes_to_rerender_returns_resolved_and_matching_stub_sources() -> anyhow::Result<()> {
    let _guard = db_guard().await;
    let Some(pool) = common::maybe_pool().await? else {
        return Ok(());
    };

    // Target note plus three potential sources.
    let target = create_document(&pool, new_doc("target")).await?;
    let resolved_source = create_document(&pool, new_doc("resolved-source")).await?;
    let stub_source = create_document(&pool, new_doc("stub-source")).await?;
    let unrelated_source = create_document(&pool, new_doc("unrelated-source")).await?;

    // resolved-source resolves directly to target.
    insert_link(
        &pool,
        NewLink {
            source_note_id: resolved_source.id,
            target_kind: TargetKind::Internal,
            target_note_id: Some(target.id),
            target_url: None,
            target_text: Some("target".to_string()),
            link_type: LinkType::Wikilink,
            context_snippet: Some("see [[target]]".to_string()),
            resolved: true,
        },
    )
    .await?;

    // stub-source has an unresolved [[target]] stub: a future create/rename of
    // "target" must re-render it.
    insert_link(
        &pool,
        NewLink {
            source_note_id: stub_source.id,
            target_kind: TargetKind::Internal,
            target_note_id: None,
            target_url: None,
            target_text: Some("target".to_string()),
            link_type: LinkType::Wikilink,
            context_snippet: Some("a [[target]] that does not exist yet".to_string()),
            resolved: false,
        },
    )
    .await?;

    // unrelated-source points at a different note and must NOT be in the set.
    insert_link(
        &pool,
        NewLink {
            source_note_id: unrelated_source.id,
            target_kind: TargetKind::Internal,
            target_note_id: Some(resolved_source.id),
            target_url: None,
            target_text: Some("resolved-source".to_string()),
            link_type: LinkType::Wikilink,
            context_snippet: None,
            resolved: true,
        },
    )
    .await?;

    let mut affected = notes_to_rerender(&pool, target.id, "target").await?;
    affected.sort();
    let mut expected = vec![resolved_source.id, stub_source.id];
    expected.sort();

    assert_eq!(
        affected, expected,
        "re-render set must be exactly the resolved source and the matching stub source"
    );
    assert!(
        !affected.contains(&unrelated_source.id),
        "an unrelated source must never be re-rendered"
    );

    Ok(())
}

#[tokio::test]
async fn deleting_a_note_cascades_to_its_outbound_edges() -> anyhow::Result<()> {
    let _guard = db_guard().await;
    let Some(pool) = common::maybe_pool().await? else {
        return Ok(());
    };

    let source = create_document(&pool, new_doc("cascade-source")).await?;
    let target = create_document(&pool, new_doc("cascade-target")).await?;
    insert_link(
        &pool,
        NewLink {
            source_note_id: source.id,
            target_kind: TargetKind::Internal,
            target_note_id: Some(target.id),
            target_url: None,
            target_text: Some("cascade-target".to_string()),
            link_type: LinkType::Wikilink,
            context_snippet: None,
            resolved: true,
        },
    )
    .await?;

    sqlx::query("DELETE FROM documents WHERE id = $1")
        .bind(source.id)
        .execute(&pool)
        .await?;

    let remaining: i64 = sqlx::query_scalar::<Postgres, i64>(
        "SELECT count(*)::bigint FROM links WHERE source_note_id = $1",
    )
    .bind(source.id)
    .fetch_one(&pool)
    .await?;
    assert_eq!(
        remaining, 0,
        "deleting a source note must cascade-delete its edges"
    );

    Ok(())
}

#[tokio::test]
async fn resolve_existing_slugs_respects_visibility() -> anyhow::Result<()> {
    let _guard = db_guard().await;
    let Some(pool) = common::maybe_pool().await? else {
        return Ok(());
    };

    create_document(&pool, new_doc("pub-note")).await?;
    let mut draft = new_doc("draft-note");
    draft.status = Some(DocumentStatus::Draft);
    create_document(&pool, draft).await?;

    let requested = vec![
        "pub-note".to_string(),
        "draft-note".to_string(),
        "missing".to_string(),
    ];

    let public = resolve_existing_slugs(&pool, &requested, Visibility::Public).await?;
    assert!(public.contains("pub-note"), "published note resolves");
    assert!(
        !public.contains("draft-note"),
        "drafts must not resolve for public callers"
    );
    assert!(!public.contains("missing"), "absent slug never resolves");

    let all = resolve_existing_slugs(&pool, &requested, Visibility::All).await?;
    assert!(
        all.contains("pub-note") && all.contains("draft-note"),
        "owner visibility resolves drafts too"
    );
    assert!(!all.contains("missing"));

    Ok(())
}

#[tokio::test]
async fn render_and_resolve_renders_links_and_persists_edges() -> anyhow::Result<()> {
    let _guard = db_guard().await;
    let Some(pool) = common::maybe_pool().await? else {
        return Ok(());
    };

    let target = create_document(&pool, new_doc("target")).await?;
    let source = create_document(&pool, new_doc("source")).await?;

    let (html, refs) = render_and_resolve(&pool, "see [[target]] and [[Missing One]] here").await?;

    // Resolved target renders a plain anchor; the missing one renders a stub.
    assert!(html.contains("href=\"/target\""), "resolved link renders");
    assert!(
        html.contains("class=\"stub\""),
        "missing link renders a stub"
    );
    assert!(
        html.contains("href=\"/missing-one\""),
        "stub slug is normalized"
    );
    assert_eq!(refs.len(), 2);

    persist_source_edges(&pool, source.id, &refs).await?;

    let rows: Vec<(String, bool)> = sqlx::query_as::<Postgres, (String, bool)>(
        "SELECT target_text, resolved FROM links WHERE source_note_id = $1 ORDER BY target_text",
    )
    .bind(source.id)
    .fetch_all(&pool)
    .await?;
    assert_eq!(
        rows,
        vec![
            ("missing-one".to_string(), false),
            ("target".to_string(), true),
        ]
    );

    // The source now resolves to the target, so renaming/changing target must
    // re-render the source.
    let affected = notes_to_rerender(&pool, target.id, "target").await?;
    assert!(affected.contains(&source.id));

    Ok(())
}

#[tokio::test]
async fn updating_a_document_bumps_its_version() -> anyhow::Result<()> {
    let _guard = db_guard().await;
    let Some(pool) = common::maybe_pool().await? else {
        return Ok(());
    };

    let doc = create_document(&pool, new_doc("bump")).await?;
    update_document_by_slug(
        &pool,
        "bump",
        DocumentPatch {
            title: Some("Bumped".to_string()),
            ..Default::default()
        },
    )
    .await?
    .expect("document exists");

    let version: i64 =
        sqlx::query_scalar::<Postgres, i64>("SELECT version FROM documents WHERE id = $1")
            .bind(doc.id)
            .fetch_one(&pool)
            .await?;
    assert_eq!(version, 2, "an update bumps version from 1 to 2");

    Ok(())
}

#[tokio::test]
async fn documents_carry_a_version_defaulting_to_one() -> anyhow::Result<()> {
    let _guard = db_guard().await;
    let Some(pool) = common::maybe_pool().await? else {
        return Ok(());
    };

    let doc = create_document(&pool, new_doc("versioned")).await?;
    let version: i64 =
        sqlx::query_scalar::<Postgres, i64>("SELECT version FROM documents WHERE id = $1")
            .bind(doc.id)
            .fetch_one(&pool)
            .await?;
    assert_eq!(version, 1, "new documents start at version 1");

    Ok(())
}
