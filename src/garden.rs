//! Write-path orchestration for the link graph.
//!
//! When a note is created or its body changes, the garden does two coupled
//! things: renders the markdown with wikilinks resolved, and records the note's
//! outbound edges in the `links` table.
//!
//! ```text
//!   markdown ─▶ extract_wikilinks ─▶ resolve slugs (PUBLIC) ─┬─▶ render html (resolved vs stub)
//!                                                            └─▶ persist edges (replace source set)
//! ```
//!
//! Wikilinks resolve against PUBLIC visibility so a link to a draft renders as a
//! stub and never leaks the draft's existence; it lights up once the target is
//! published (via the re-render fan-out — a later slice). Slugs are immutable in
//! the current write API, so there is no rename trigger yet.

use crate::db::documents;
use crate::db::links::{self, LinkType, NewLink, TargetKind, Visibility};
use crate::rendering::wikilink::{extract_wikilinks, render_markdown_with_links};
use sqlx::{PgPool, Postgres};
use std::collections::HashSet;
use uuid::Uuid;

/// A wikilink/embed reference after resolution against the live garden.
#[derive(Clone, Debug)]
pub struct ResolvedRef {
    pub target_slug: String,
    pub target_note_id: Option<Uuid>,
    pub is_embed: bool,
    pub context_snippet: String,
}

/// Render `markdown` to the HTML to store (wikilinks resolved against the public
/// garden) and return the resolved references so the caller can persist edges
/// once the source note's id is known.
pub async fn render_and_resolve(
    pool: &PgPool,
    markdown: &str,
) -> Result<(String, Vec<ResolvedRef>), sqlx::Error> {
    let refs = extract_wikilinks(markdown);
    let slugs: Vec<String> = refs.iter().map(|r| r.target_slug.clone()).collect();
    let resolved = links::resolve_slug_ids(pool, &slugs, Visibility::Public).await?;
    let existing: HashSet<String> = resolved.keys().cloned().collect();
    let html = render_markdown_with_links(markdown, &existing);

    let out = refs
        .into_iter()
        .map(|r| ResolvedRef {
            target_note_id: resolved.get(&r.target_slug).copied(),
            target_slug: r.target_slug,
            is_embed: r.is_embed,
            context_snippet: r.context_snippet,
        })
        .collect();

    Ok((html, out))
}

/// Replace `source_id`'s outbound edges with the given resolved references.
pub async fn persist_source_edges(
    pool: &PgPool,
    source_id: Uuid,
    refs: &[ResolvedRef],
) -> Result<(), sqlx::Error> {
    let edges: Vec<NewLink> = refs
        .iter()
        .map(|r| NewLink {
            source_note_id: source_id,
            target_kind: TargetKind::Internal,
            target_note_id: r.target_note_id,
            target_url: None,
            target_text: Some(r.target_slug.clone()),
            link_type: if r.is_embed {
                LinkType::Embed
            } else {
                LinkType::Wikilink
            },
            context_snippet: Some(r.context_snippet.clone()),
            resolved: r.target_note_id.is_some(),
        })
        .collect();
    links::replace_source_edges(pool, source_id, &edges).await
}

/// Source notes whose stored HTML may need re-rendering because the note
/// (`note_id`, `slug`) was created, published, unpublished, or deleted. Returns
/// an empty set on error (best-effort; the fan-out never fails the request).
///
/// For deletes, call this BEFORE removing the row: once gone, resolved inbound
/// edges have had `target_note_id` nulled and would no longer match by id.
pub async fn affected_sources(pool: &PgPool, note_id: Uuid, slug: &str) -> Vec<Uuid> {
    match links::notes_to_rerender(pool, note_id, slug).await {
        Ok(ids) => ids,
        Err(error) => {
            tracing::warn!(%error, "notes_to_rerender failed; skipping re-render fan-out");
            Vec::new()
        }
    }
}

/// Re-render each source note against the current garden and replace its edges,
/// so stubs light up (target appeared/published) or drop back to stubs (target
/// unpublished/deleted). Best-effort: a single note's failure is logged and the
/// rest proceed — a stale stub self-heals on that note's next save.
pub async fn rerender_sources(pool: &PgPool, ids: &[Uuid]) {
    for &id in ids {
        if let Err(error) = rerender_one(pool, id).await {
            tracing::warn!(note_id = %id, %error, "re-render failed; stub may be stale until next save");
        }
    }
}

async fn rerender_one(pool: &PgPool, id: Uuid) -> Result<(), sqlx::Error> {
    let Some((_slug, body_markdown)) = sqlx::query_as::<Postgres, (String, String)>(
        "SELECT slug, body_markdown FROM documents WHERE id = $1",
    )
    .bind(id)
    .fetch_optional(pool)
    .await?
    else {
        return Ok(());
    };
    let (html, refs) = render_and_resolve(pool, &body_markdown).await?;
    documents::set_rendered_html(pool, id, &html).await?;
    persist_source_edges(pool, id, &refs).await
}

/// Convenience for create/publish/unpublish: compute the affected sources and
/// re-render them. (Delete must split this — capture sources before the row is
/// removed, then `rerender_sources` after.)
pub async fn backfill_after_change(pool: &PgPool, note_id: Uuid, slug: &str) {
    let affected = affected_sources(pool, note_id, slug).await;
    rerender_sources(pool, &affected).await;
}
