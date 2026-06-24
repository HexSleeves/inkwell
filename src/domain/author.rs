//! Author identity, token-scope, and request-principal types (ADR 0009, plan 023).
//!
//! **Slice 2 wires these into request auth.** [`Principal`] is the resolved
//! identity behind a request — produced by `authenticate()` from either the
//! shared admin key or a scoped token — and carries the [`Scope`] set used for
//! authorization. Slice 2 resolves and audits principals but does **not** yet
//! enforce scope/ownership on document routes (deferred to slice 3); the one
//! exception is the admin token-management surface, which is admin-gated from
//! the moment it exists so a `write` token can never mint an `admin` token.
//! Some helpers (e.g. [`Principal::has`]) are only consumed once slice 3 turns
//! on enforcement, so `dead_code` stays allowed until then.
#![allow(dead_code)]

use std::collections::HashSet;

use serde::{Deserialize, Serialize};
use time::OffsetDateTime;
use uuid::Uuid;

/// The fixed uuid of the bootstrap admin author seeded by migration 0015.
///
/// The shared `INKWELL_API_KEY` acts as this principal until per-author tokens
/// exist (slice 2). Existing documents are backfilled to it, and slice 1's
/// write-audit rows are attributed to it (`actor_label = "shared-key"`).
pub const BOOTSTRAP_ADMIN_ID: Uuid = Uuid::from_u128(0x0000_0000_0000_0000_0000_0000_0000_0001);

/// A capability a token can carry (ADR 0009). Stored as `text` in the
/// `author_tokens.scopes` array and decoded directly into this enum — exactly
/// the `text` + closed-vocabulary shape of [`crate::domain::document::GrowthStage`].
///
/// `Admin` implies every other scope; enforcement of that rule lands in slice 3.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash, Serialize, Deserialize, sqlx::Type)]
#[sqlx(type_name = "text", rename_all = "lowercase")]
#[serde(rename_all = "lowercase")]
pub enum Scope {
    /// See drafts/unlisted notes (owner-visibility reads).
    Read,
    /// Create notes; update/delete own notes.
    Write,
    /// Publish/unpublish own notes.
    Publish,
    /// All of the above on any note, plus manage authors/tokens.
    Admin,
}

impl Scope {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Read => "read",
            Self::Write => "write",
            Self::Publish => "publish",
            Self::Admin => "admin",
        }
    }

    /// Parse the stored/wire token into a scope, or `None` if unknown.
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "read" => Some(Self::Read),
            "write" => Some(Self::Write),
            "publish" => Some(Self::Publish),
            "admin" => Some(Self::Admin),
            _ => None,
        }
    }

    /// Every scope — the capability set of an admin principal (the shared key).
    pub fn all() -> HashSet<Self> {
        [Self::Read, Self::Write, Self::Publish, Self::Admin]
            .into_iter()
            .collect()
    }
}

impl std::fmt::Display for Scope {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// A first-class principal that can own documents and hold tokens
/// (migration 0011). Slice 2 reads this row when resolving a token to attribute
/// writes to the owning author.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, sqlx::FromRow)]
#[serde(rename_all = "camelCase")]
pub struct Author {
    pub id: Uuid,
    pub name: String,
    #[serde(with = "crate::domain::document::timestamp")]
    pub created_at: OffsetDateTime,
}

/// The resolved identity behind an authenticated request (ADR 0009, plan 023,
/// slice 2). Produced by `authenticate()` from either the shared admin/MCP key
/// or a scoped token, and consumed for write-audit attribution now and for
/// scope/ownership enforcement in slice 3.
#[derive(Clone, Debug)]
pub struct Principal {
    /// The owning author's id. Always `Some` today — the shared key resolves to
    /// the bootstrap admin and a token always belongs to an author — but kept
    /// `Option` so the audit layer can record `actor_author_id = NULL` if a
    /// non-author principal is ever introduced.
    pub author_id: Option<Uuid>,
    /// Human-readable actor label for the audit trail: `"shared-key"`,
    /// `"mcp-key"`, or the author's name for a scoped token.
    pub label: String,
    /// Capabilities this principal holds.
    pub scopes: HashSet<Scope>,
}

impl Principal {
    /// The all-powerful admin principal backing the shared key (and, until
    /// slice 4 retires it, the MCP key). `label` distinguishes them in the audit.
    pub fn admin(author_id: Uuid, label: impl Into<String>) -> Self {
        Self {
            author_id: Some(author_id),
            label: label.into(),
            scopes: Scope::all(),
        }
    }

    /// Whether this principal holds `scope`. [`Scope::Admin`] implies every
    /// scope. The authorization predicate slice 3 enforces on each route.
    pub fn has(&self, scope: Scope) -> bool {
        self.scopes.contains(&Scope::Admin) || self.scopes.contains(&scope)
    }
}
