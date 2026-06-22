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
