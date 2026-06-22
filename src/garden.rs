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

use crate::db::links::{self, LinkType, NewLink, TargetKind, Visibility};
use crate::rendering::wikilink::{extract_wikilinks, render_markdown_with_links};
use sqlx::PgPool;
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
