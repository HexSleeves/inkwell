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
use crate::domain::document::DocumentStatus;
use crate::rendering::wikilink::{EmbedResolution, extract_wikilinks, render_markdown_with_embeds};
use sqlx::{PgPool, Postgres};
use std::collections::{HashMap, HashSet};
use uuid::Uuid;

/// Maximum transclusion nesting depth. An embed chain `A → B → C → …` expands at
/// most this many levels before further embeds collapse to a placeholder. A
/// small hard const so a deep (or accidentally deep) chain can never blow the
/// stack or the rendered-HTML size — bounded exactly like every other surface.
const MAX_EMBED_DEPTH: u32 = 3;

/// Maximum total number of embed targets expanded for a single render. Depth
/// alone bounds nesting, but a wide fan-out (an "index" note embedding many
/// notes, each embedding many more) is `O(b^depth)` in the branching factor and
/// would balloon both the query count and the persisted HTML. A global budget
/// collapses every further embed to a placeholder once the cap is hit, bounding
/// total work exactly like the graph surface caps total nodes/edges.
const MAX_EMBED_EXPANSIONS: u32 = 256;

/// A wikilink/embed reference after resolution against the live garden.
#[derive(Clone, Debug)]
pub struct ResolvedRef {
    pub target_slug: String,
    pub target_note_id: Option<Uuid>,
    pub is_embed: bool,
    pub context_snippet: String,
}

/// Render `markdown` to the HTML to store (wikilinks resolved against the public
/// garden, `![[embeds]]` transcluded) and return the resolved references so the
/// caller can persist edges once the source note's id is known.
///
/// Writes resolve at [`Visibility::Public`], so a wikilink to a draft renders as
/// a stub and an embed of a draft renders only a neutral placeholder — never the
/// draft's content (the systemic no-draft-leak invariant). Transclusion is
/// bounded by [`MAX_EMBED_DEPTH`] and a cycle guard, so a self/transitive embed
/// terminates with a placeholder instead of looping.
pub async fn render_and_resolve(
    pool: &PgPool,
    markdown: &str,
) -> Result<(String, Vec<ResolvedRef>), sqlx::Error> {
    let refs = extract_wikilinks(markdown);
    let slugs: Vec<String> = refs.iter().map(|r| r.target_slug.clone()).collect();
    let resolved = links::resolve_slug_ids(pool, &slugs, Visibility::Public).await?;
    let existing: HashSet<String> = resolved.keys().cloned().collect();

    // Expand `![[embeds]]` recursively (depth- and cycle-bounded) at the same
    // public visibility the write path uses everywhere else.
    let embed_slugs: HashSet<String> = refs
        .iter()
        .filter(|r| r.is_embed)
        .map(|r| r.target_slug.clone())
        .collect();
    let mut visited: HashSet<String> = HashSet::new();
    let mut budget: u32 = MAX_EMBED_EXPANSIONS;
    let embeds = resolve_embeds(
        pool,
        &embed_slugs,
        Visibility::Public,
        0,
        &mut visited,
        &mut budget,
    )
    .await?;

    let html = render_markdown_with_embeds(markdown, &existing, &embeds);

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

/// Resolve a set of embed target slugs to their [`EmbedResolution`], recursing
/// into each published target's own embeds.
///
/// Guards (every path terminates):
///   - DEPTH: at `depth >= MAX_EMBED_DEPTH` every embed collapses to a
///     placeholder, so the recursion can never exceed the cap.
///   - BUDGET: a shared `budget` counter caps the total number of targets
///     expanded across the whole render; once it hits zero every remaining
///     embed collapses to a placeholder, so a wide fan-out can't explode work.
///   - CYCLE: a target already on the current expansion path (`visited`) renders
///     a placeholder instead of recursing, so `A → B → A` (and a direct
///     self-embed) terminate.
///   - NO-LEAK: a target that is not visible at `visibility` (a draft, or
///     missing) renders a neutral placeholder — never its content.
async fn resolve_embeds(
    pool: &PgPool,
    slugs: &HashSet<String>,
    visibility: Visibility,
    depth: u32,
    visited: &mut HashSet<String>,
    budget: &mut u32,
) -> Result<HashMap<String, EmbedResolution>, sqlx::Error> {
    let mut out: HashMap<String, EmbedResolution> = HashMap::new();
    if slugs.is_empty() {
        return Ok(out);
    }
    // Depth cap: stop expanding; every embed at this level is a placeholder.
    if depth >= MAX_EMBED_DEPTH {
        for slug in slugs {
            out.insert(slug.clone(), EmbedResolution::Placeholder);
        }
        return Ok(out);
    }

    for slug in slugs {
        // Cycle: this slug is already being expanded on the path to here.
        if visited.contains(slug) {
            out.insert(slug.clone(), EmbedResolution::Placeholder);
            continue;
        }
        // Budget cap: once the global expansion budget is exhausted, every
        // remaining target collapses to a placeholder so total work stays bounded
        // regardless of fan-out.
        if *budget == 0 {
            out.insert(slug.clone(), EmbedResolution::Placeholder);
            continue;
        }
        *budget -= 1;
        // Fetch the target's status + body. A draft (under Public) or a missing
        // note yields a placeholder — never any of the target's content.
        let Some((status, body)) = fetch_embed_target(pool, slug, visibility).await? else {
            out.insert(slug.clone(), EmbedResolution::Placeholder);
            continue;
        };
        if visibility == Visibility::Public && status != DocumentStatus::Published {
            out.insert(slug.clone(), EmbedResolution::Placeholder);
            continue;
        }

        // Recurse into the target's own content with this slug on the path.
        visited.insert(slug.clone());
        let child_refs = extract_wikilinks(&body);
        let child_link_slugs: Vec<String> =
            child_refs.iter().map(|r| r.target_slug.clone()).collect();
        let child_resolved = links::resolve_slug_ids(pool, &child_link_slugs, visibility).await?;
        let child_existing: HashSet<String> = child_resolved.keys().cloned().collect();
        let child_embed_slugs: HashSet<String> = child_refs
            .iter()
            .filter(|r| r.is_embed)
            .map(|r| r.target_slug.clone())
            .collect();
        let child_embeds = Box::pin(resolve_embeds(
            pool,
            &child_embed_slugs,
            visibility,
            depth + 1,
            visited,
            budget,
        ))
        .await?;
        let content = render_markdown_with_embeds(&body, &child_existing, &child_embeds);
        visited.remove(slug);

        out.insert(slug.clone(), EmbedResolution::Content(content));
    }
    Ok(out)
}

/// Fetch an embed target's `(status, body_markdown)` by slug at `visibility`.
/// Under [`Visibility::Public`] a draft target is filtered out at the SQL level
/// (returns `None`), so a draft can never reach the renderer as content.
async fn fetch_embed_target(
    pool: &PgPool,
    slug: &str,
    visibility: Visibility,
) -> Result<Option<(DocumentStatus, String)>, sqlx::Error> {
    match visibility.status_filter() {
        Some(status) => {
            sqlx::query_as::<Postgres, (DocumentStatus, String)>(
                "SELECT status, body_markdown FROM documents WHERE slug = $1 AND status = $2",
            )
            .bind(slug)
            .bind(status.as_str())
            .fetch_optional(pool)
            .await
        }
        None => {
            sqlx::query_as::<Postgres, (DocumentStatus, String)>(
                "SELECT status, body_markdown FROM documents WHERE slug = $1",
            )
            .bind(slug)
            .fetch_optional(pool)
            .await
        }
    }
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
