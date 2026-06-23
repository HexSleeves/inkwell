//! Link graph data access for the digital garden.
//!
//! Edges live in the `links` table. P0 establishes three things the rest of the
//! garden builds on:
//!   - [`Visibility`] — the single read-scope predicate every surface that can
//!     expose note content (backlinks, graph, search, MCP reads, embeddings)
//!     must share, so no surface re-derives the rule and leaks a draft.
//!   - [`notes_to_rerender`] — the re-render fan-out chokepoint. The repo stores
//!     `rendered_html` at write time, so creating/renaming/deleting a note can
//!     change how *other* notes link to it. Callers resolve edges atomically in
//!     their write transaction, then re-render this set post-commit, best-effort.
//!   - [`insert_link`] — the single edge writer used by the wikilink resolver.
//!
//! Edge lifecycle:
//! ```text
//!   write note ──parse [[...]]──▶ resolve slug ──┬─ found  ─▶ internal, resolved=true,  target_note_id set
//!                                                └─ missing ─▶ internal, resolved=false, target_text=slug (stub)
//!   later: a note with that slug is created/renamed ─▶ backfill flips the stub to resolved
//!          (then notes_to_rerender(new) re-renders the stub's source)
//! ```

use std::collections::{HashMap, HashSet};

use crate::domain::document::DocumentStatus;
use sqlx::{PgPool, Postgres};
use uuid::Uuid;

/// Read scope for any surface that can expose note content. Centralized so the
/// draft-invisibility invariant is enforced in one place instead of re-derived
/// per surface (the systemic draft-leak fix).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Visibility {
    /// Unauthenticated callers: published notes only.
    Public,
    /// Authenticated owner: drafts and unlisted included.
    All,
}

impl Visibility {
    /// The [`DocumentStatus`] filter this visibility implies, or `None` to mean
    /// "no status restriction" (owner sees everything).
    pub fn status_filter(self) -> Option<DocumentStatus> {
        match self {
            Visibility::Public => Some(DocumentStatus::Published),
            Visibility::All => None,
        }
    }
}

/// Whether an edge points at another note in this garden or an external URL.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TargetKind {
    Internal,
    External,
}

impl TargetKind {
    pub fn as_str(self) -> &'static str {
        match self {
            TargetKind::Internal => "internal",
            TargetKind::External => "external",
        }
    }
}

/// The markup that produced an edge. `Wikilink` is `[[note]]`; `Embed` is the
/// `![[note]]` transclusion form (enabled later).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LinkType {
    Wikilink,
    Embed,
}

impl LinkType {
    pub fn as_str(self) -> &'static str {
        match self {
            LinkType::Wikilink => "wikilink",
            LinkType::Embed => "embed",
        }
    }
}

/// A new edge to persist. For an unresolved internal stub, `target_note_id` is
/// `None`, `resolved` is `false`, and `target_text` carries the raw `[[slug]]`
/// inner text that backfill later matches against.
#[derive(Clone, Debug)]
pub struct NewLink {
    pub source_note_id: Uuid,
    pub target_kind: TargetKind,
    pub target_note_id: Option<Uuid>,
    pub target_url: Option<String>,
    pub target_text: Option<String>,
    pub link_type: LinkType,
    pub context_snippet: Option<String>,
    pub resolved: bool,
}

/// Insert one edge, returning its id.
pub async fn insert_link(pool: &PgPool, link: NewLink) -> Result<Uuid, sqlx::Error> {
    sqlx::query_scalar::<Postgres, Uuid>(
        r#"
        INSERT INTO links (
            source_note_id, target_kind, target_note_id, target_url,
            target_text, link_type, context_snippet, resolved
        )
        VALUES ($1, $2, $3, $4, $5, $6, $7, $8)
        RETURNING id
        "#,
    )
    .bind(link.source_note_id)
    .bind(link.target_kind.as_str())
    .bind(link.target_note_id)
    .bind(&link.target_url)
    .bind(&link.target_text)
    .bind(link.link_type.as_str())
    .bind(&link.context_snippet)
    .bind(link.resolved)
    .fetch_one(pool)
    .await
}

/// Of `slugs`, return the subset that currently exist as documents visible at
/// `visibility`. This is the batch wikilink resolver: the renderer collects
/// every `[[slug]]` target from one parse pass and resolves them all in a single
/// query (no N+1). A slug absent from the result renders as a stub.
pub async fn resolve_existing_slugs(
    pool: &PgPool,
    slugs: &[String],
    visibility: Visibility,
) -> Result<HashSet<String>, sqlx::Error> {
    if slugs.is_empty() {
        return Ok(HashSet::new());
    }
    let found: Vec<String> = match visibility.status_filter() {
        Some(status) => {
            sqlx::query_scalar::<Postgres, String>(
                "SELECT slug FROM documents WHERE slug = ANY($1) AND status = $2",
            )
            .bind(slugs)
            .bind(status.as_str())
            .fetch_all(pool)
            .await?
        }
        None => {
            sqlx::query_scalar::<Postgres, String>(
                "SELECT slug FROM documents WHERE slug = ANY($1)",
            )
            .bind(slugs)
            .fetch_all(pool)
            .await?
        }
    };
    Ok(found.into_iter().collect())
}

/// Like [`resolve_existing_slugs`] but returns a `slug -> note id` map, so the
/// write path can both decide which links render resolved AND record the
/// `target_note_id` on each persisted edge in one query.
pub async fn resolve_slug_ids(
    pool: &PgPool,
    slugs: &[String],
    visibility: Visibility,
) -> Result<HashMap<String, Uuid>, sqlx::Error> {
    if slugs.is_empty() {
        return Ok(HashMap::new());
    }
    let rows: Vec<(String, Uuid)> = match visibility.status_filter() {
        Some(status) => {
            sqlx::query_as::<Postgres, (String, Uuid)>(
                "SELECT slug, id FROM documents WHERE slug = ANY($1) AND status = $2",
            )
            .bind(slugs)
            .bind(status.as_str())
            .fetch_all(pool)
            .await?
        }
        None => {
            sqlx::query_as::<Postgres, (String, Uuid)>(
                "SELECT slug, id FROM documents WHERE slug = ANY($1)",
            )
            .bind(slugs)
            .fetch_all(pool)
            .await?
        }
    };
    Ok(rows.into_iter().collect())
}

/// Replace every outbound edge of `source_id` with `edges`, atomically. Called
/// after a note's markdown is (re)rendered so its link graph matches its current
/// content. Existing rows are deleted and the new set inserted in one transaction.
///
/// Every edge is inserted under `source_id` regardless of its own
/// `source_note_id` field — this function owns exactly one note's outbound set,
/// so the delete and the inserts cannot diverge.
pub async fn replace_source_edges(
    pool: &PgPool,
    source_id: Uuid,
    edges: &[NewLink],
) -> Result<(), sqlx::Error> {
    let mut tx = pool.begin().await?;
    sqlx::query("DELETE FROM links WHERE source_note_id = $1")
        .bind(source_id)
        .execute(&mut *tx)
        .await?;
    for edge in edges {
        sqlx::query(
            r#"
            INSERT INTO links (
                source_note_id, target_kind, target_note_id, target_url,
                target_text, link_type, context_snippet, resolved
            )
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8)
            "#,
        )
        .bind(source_id)
        .bind(edge.target_kind.as_str())
        .bind(edge.target_note_id)
        .bind(&edge.target_url)
        .bind(&edge.target_text)
        .bind(edge.link_type.as_str())
        .bind(&edge.context_snippet)
        .bind(edge.resolved)
        .execute(&mut *tx)
        .await?;
    }
    tx.commit().await?;
    Ok(())
}

/// An inbound edge to a note: another note (`source_slug` / `source_title`) that
/// links to it, plus the `[[...]]` context the link appeared in. Produced by
/// [`backlinks`] for the "linked from" surfaces (HTML panel + JSON).
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Backlink {
    pub source_slug: String,
    pub source_title: String,
    pub context_snippet: Option<String>,
}

/// Notes that link *to* `target_note_id` — the "linked from" set — filtered by
/// `visibility` so a draft source is NEVER exposed to a public caller (the
/// no-draft-leak invariant). Only resolved internal edges count; a source that
/// links more than once appears once (`DISTINCT ON` the source slug), ordered by
/// slug for deterministic output. Mirrors the two-arm `status_filter()` pattern
/// used by [`resolve_existing_slugs`]/[`resolve_slug_ids`]: `Public` ⇒ the source
/// document must be `published`; `All` ⇒ no status restriction (owner scope).
pub async fn backlinks(
    pool: &PgPool,
    target_note_id: Uuid,
    visibility: Visibility,
) -> Result<Vec<Backlink>, sqlx::Error> {
    let rows: Vec<(String, String, Option<String>)> = match visibility.status_filter() {
        Some(status) => {
            sqlx::query_as::<Postgres, (String, String, Option<String>)>(
                r#"
                SELECT DISTINCT ON (documents.slug)
                       documents.slug, documents.title, links.context_snippet
                FROM links
                JOIN documents ON documents.id = links.source_note_id
                WHERE links.target_note_id = $1
                  AND links.target_kind = 'internal'
                  AND links.resolved = true
                  AND documents.status = $2
                ORDER BY documents.slug, links.created_at, links.id
                "#,
            )
            .bind(target_note_id)
            .bind(status.as_str())
            .fetch_all(pool)
            .await?
        }
        None => {
            sqlx::query_as::<Postgres, (String, String, Option<String>)>(
                r#"
                SELECT DISTINCT ON (documents.slug)
                       documents.slug, documents.title, links.context_snippet
                FROM links
                JOIN documents ON documents.id = links.source_note_id
                WHERE links.target_note_id = $1
                  AND links.target_kind = 'internal'
                  AND links.resolved = true
                ORDER BY documents.slug, links.created_at, links.id
                "#,
            )
            .bind(target_note_id)
            .fetch_all(pool)
            .await?
        }
    };
    Ok(rows
        .into_iter()
        .map(|(source_slug, source_title, context_snippet)| Backlink {
            source_slug,
            source_title,
            context_snippet,
        })
        .collect())
}

/// Hard upper bound on the number of nodes a single graph query returns. The
/// graph is a bounded surface (never an unbounded crawl): both the global graph
/// and a per-note neighborhood cap their node set at this many notes so a large
/// garden can never produce an unbounded payload.
pub const MAX_GRAPH_NODES: i64 = 500;

/// Hard upper bound on the number of edges a single graph query returns. Edges
/// are capped independently of nodes (a dense garden has many more edges than
/// notes) so the payload stays bounded regardless of link density.
pub const MAX_GRAPH_EDGES: i64 = 2_000;

/// Hard cap on the neighborhood depth a per-note graph will expand. The
/// neighborhood is a small local view, never a transitive crawl of the whole
/// garden, so the depth is fixed at one hop.
pub const MAX_GRAPH_DEPTH: u32 = 1;

/// One node in the link graph: a note exposed at the rendering visibility.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GraphNode {
    pub slug: String,
    pub title: String,
}

/// One edge in the link graph: a resolved internal link from `source_slug` to
/// `target_slug`. Both endpoints are guaranteed visible at the query's
/// visibility (a public graph never includes an edge touching a draft).
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GraphEdge {
    pub source_slug: String,
    pub target_slug: String,
}

/// A bounded slice of the link graph: the visible nodes and the resolved
/// internal edges whose BOTH endpoints are in `nodes`.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct Graph {
    pub nodes: Vec<GraphNode>,
    pub edges: Vec<GraphEdge>,
}

/// The whole garden's link graph, visibility-filtered and hard-bounded.
///
/// Nodes are the notes visible at `visibility` (public ⇒ published only),
/// capped at [`MAX_GRAPH_NODES`] and ordered by slug for determinism. Edges are
/// resolved internal links whose source AND target are both in the node set —
/// so a public graph never leaks a draft node, nor an edge that would reveal a
/// draft's existence (the no-draft-leak invariant), and never an edge dangling
/// to a note dropped by the node cap. Edges are capped at [`MAX_GRAPH_EDGES`].
pub async fn garden_graph(pool: &PgPool, visibility: Visibility) -> Result<Graph, sqlx::Error> {
    // Nodes: the visible notes, deterministically ordered and bounded.
    let node_rows: Vec<(String, String)> = match visibility.status_filter() {
        Some(status) => {
            sqlx::query_as::<Postgres, (String, String)>(
                "SELECT slug, title FROM documents WHERE status = $1 ORDER BY slug LIMIT $2",
            )
            .bind(status.as_str())
            .bind(MAX_GRAPH_NODES)
            .fetch_all(pool)
            .await?
        }
        None => {
            sqlx::query_as::<Postgres, (String, String)>(
                "SELECT slug, title FROM documents ORDER BY slug LIMIT $1",
            )
            .bind(MAX_GRAPH_NODES)
            .fetch_all(pool)
            .await?
        }
    };
    let nodes: Vec<GraphNode> = node_rows
        .into_iter()
        .map(|(slug, title)| GraphNode { slug, title })
        .collect();
    let visible: HashSet<&str> = nodes.iter().map(|n| n.slug.as_str()).collect();

    // Edges: resolved internal links where BOTH endpoints are visible. The join
    // is filtered to the same visibility on both ends in SQL; we additionally
    // intersect with the (capped) node set so an edge can never dangle to a
    // node the LIMIT dropped.
    let edge_rows: Vec<(String, String)> = match visibility.status_filter() {
        Some(status) => {
            sqlx::query_as::<Postgres, (String, String)>(
                r#"
                SELECT src.slug, tgt.slug
                FROM links
                JOIN documents AS src ON src.id = links.source_note_id
                JOIN documents AS tgt ON tgt.id = links.target_note_id
                WHERE links.target_kind = 'internal'
                  AND links.resolved = true
                  AND src.status = $1
                  AND tgt.status = $1
                ORDER BY src.slug, tgt.slug
                LIMIT $2
                "#,
            )
            .bind(status.as_str())
            .bind(MAX_GRAPH_EDGES)
            .fetch_all(pool)
            .await?
        }
        None => {
            sqlx::query_as::<Postgres, (String, String)>(
                r#"
                SELECT src.slug, tgt.slug
                FROM links
                JOIN documents AS src ON src.id = links.source_note_id
                JOIN documents AS tgt ON tgt.id = links.target_note_id
                WHERE links.target_kind = 'internal'
                  AND links.resolved = true
                ORDER BY src.slug, tgt.slug
                LIMIT $1
                "#,
            )
            .bind(MAX_GRAPH_EDGES)
            .fetch_all(pool)
            .await?
        }
    };
    let edges: Vec<GraphEdge> = edge_rows
        .into_iter()
        .filter(|(src, tgt)| visible.contains(src.as_str()) && visible.contains(tgt.as_str()))
        .map(|(source_slug, target_slug)| GraphEdge {
            source_slug,
            target_slug,
        })
        .collect();

    Ok(Graph { nodes, edges })
}

/// A one-hop neighborhood graph around the note `slug`: the note itself plus
/// every visible note one resolved internal link away (in either direction),
/// and the edges among that set. Visibility-filtered exactly like
/// [`garden_graph`] — a public neighborhood never includes a draft neighbor or
/// an edge touching one. Returns an empty graph when the center note is not
/// visible. Hard depth cap of [`MAX_GRAPH_DEPTH`] (one hop). Built center-first
/// (the center's own one-hop edges), so it is bounded by [`MAX_GRAPH_EDGES`] /
/// [`MAX_GRAPH_NODES`] without depending on the global graph's node-cap window —
/// the center and its neighbors are never dropped in a large garden.
pub async fn note_neighborhood(
    pool: &PgPool,
    slug: &str,
    visibility: Visibility,
) -> Result<Graph, sqlx::Error> {
    // The center note must itself be visible, or there is no neighborhood.
    let center_exists: Option<String> = match visibility.status_filter() {
        Some(status) => {
            sqlx::query_scalar::<Postgres, String>(
                "SELECT slug FROM documents WHERE slug = $1 AND status = $2",
            )
            .bind(slug)
            .bind(status.as_str())
            .fetch_optional(pool)
            .await?
        }
        None => {
            sqlx::query_scalar::<Postgres, String>("SELECT slug FROM documents WHERE slug = $1")
                .bind(slug)
                .fetch_optional(pool)
                .await?
        }
    };
    let Some(center_slug) = center_exists else {
        return Ok(Graph::default());
    };

    // Center-first: pull only the center's one-hop resolved internal edges
    // (in either direction), visibility-filtered on BOTH endpoints, bounded by
    // the edge cap. This is independent of the global node cap, so the center
    // and its neighbors are never silently dropped in a large garden.
    let edge_rows: Vec<(String, String)> = match visibility.status_filter() {
        Some(status) => {
            sqlx::query_as::<Postgres, (String, String)>(
                r#"
                SELECT src.slug, tgt.slug
                FROM links
                JOIN documents AS src ON src.id = links.source_note_id
                JOIN documents AS tgt ON tgt.id = links.target_note_id
                WHERE links.target_kind = 'internal'
                  AND links.resolved = true
                  AND src.status = $1
                  AND tgt.status = $1
                  AND (src.slug = $2 OR tgt.slug = $2)
                ORDER BY src.slug, tgt.slug
                LIMIT $3
                "#,
            )
            .bind(status.as_str())
            .bind(&center_slug)
            .bind(MAX_GRAPH_EDGES)
            .fetch_all(pool)
            .await?
        }
        None => {
            sqlx::query_as::<Postgres, (String, String)>(
                r#"
                SELECT src.slug, tgt.slug
                FROM links
                JOIN documents AS src ON src.id = links.source_note_id
                JOIN documents AS tgt ON tgt.id = links.target_note_id
                WHERE links.target_kind = 'internal'
                  AND links.resolved = true
                  AND (src.slug = $1 OR tgt.slug = $1)
                ORDER BY src.slug, tgt.slug
                LIMIT $2
                "#,
            )
            .bind(&center_slug)
            .bind(MAX_GRAPH_EDGES)
            .fetch_all(pool)
            .await?
        }
    };

    // The kept set is the center plus each one-hop neighbor.
    let mut keep: HashSet<String> = HashSet::new();
    keep.insert(center_slug.clone());
    for (src, tgt) in &edge_rows {
        keep.insert(src.clone());
        keep.insert(tgt.clone());
    }
    let edges: Vec<GraphEdge> = edge_rows
        .into_iter()
        .map(|(source_slug, target_slug)| GraphEdge {
            source_slug,
            target_slug,
        })
        .collect();

    // Fetch titles for exactly the kept slugs (the center always exists). The
    // set is at most edge-cap-sized, so this stays bounded; cap defensively.
    let keep_slugs: Vec<String> = keep.iter().cloned().collect();
    let node_rows: Vec<(String, String)> = match visibility.status_filter() {
        Some(status) => {
            sqlx::query_as::<Postgres, (String, String)>(
                "SELECT slug, title FROM documents \
                 WHERE slug = ANY($1) AND status = $2 ORDER BY slug LIMIT $3",
            )
            .bind(&keep_slugs)
            .bind(status.as_str())
            .bind(MAX_GRAPH_NODES)
            .fetch_all(pool)
            .await?
        }
        None => {
            sqlx::query_as::<Postgres, (String, String)>(
                "SELECT slug, title FROM documents \
                 WHERE slug = ANY($1) ORDER BY slug LIMIT $2",
            )
            .bind(&keep_slugs)
            .bind(MAX_GRAPH_NODES)
            .fetch_all(pool)
            .await?
        }
    };
    let nodes: Vec<GraphNode> = node_rows
        .into_iter()
        .map(|(slug, title)| GraphNode { slug, title })
        .collect();

    Ok(Graph { nodes, edges })
}

/// Note ids whose stored `rendered_html` may depend on the note identified by
/// (`changed_id`, `changed_slug`) and therefore must be re-rendered after that
/// note is created, renamed, or deleted.
///
/// A source depends on the changed note when it either already resolves to it
/// (`target_note_id = changed_id`) or holds an unresolved internal stub whose
/// text matches the slug (`resolved = false AND target_text = changed_slug`),
/// which a create/rename will upgrade to a real link.
///
/// This is the single fan-out chokepoint (the re-render of these sources runs
/// post-commit, best-effort, and never blocks the triggering write).
pub async fn notes_to_rerender(
    pool: &PgPool,
    changed_id: Uuid,
    changed_slug: &str,
) -> Result<Vec<Uuid>, sqlx::Error> {
    sqlx::query_scalar::<Postgres, Uuid>(
        r#"
        SELECT DISTINCT source_note_id
        FROM links
        WHERE target_kind = 'internal'
          AND (
                target_note_id = $1
             OR (resolved = false AND target_text = $2)
              )
        "#,
    )
    .bind(changed_id)
    .bind(changed_slug)
    .fetch_all(pool)
    .await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::document::DocumentStatus;

    #[test]
    fn public_visibility_filters_to_published() {
        assert_eq!(
            Visibility::Public.status_filter(),
            Some(DocumentStatus::Published)
        );
    }

    #[test]
    fn all_visibility_applies_no_status_filter() {
        assert_eq!(Visibility::All.status_filter(), None);
    }

    #[test]
    fn target_kind_and_link_type_round_trip_their_sql_text() {
        assert_eq!(TargetKind::Internal.as_str(), "internal");
        assert_eq!(TargetKind::External.as_str(), "external");
        assert_eq!(LinkType::Wikilink.as_str(), "wikilink");
        assert_eq!(LinkType::Embed.as_str(), "embed");
    }
}
