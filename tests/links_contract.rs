//! Database-backed contract tests for the P0 link-graph foundation.
//!
//! Skipped unless `DATABASE_URL` is set (or forced via `INKWELL_REQUIRE_DB_TESTS=1`),
//! matching the rest of the db-backed suite.

mod common;

use inkwell::db::documents::create_document;
use inkwell::db::links::{LinkType, NewLink, TargetKind, insert_link, notes_to_rerender};
use inkwell::domain::document::{DocumentStatus, NewDocument};
use sqlx::Postgres;

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
async fn documents_carry_a_version_defaulting_to_one() -> anyhow::Result<()> {
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
