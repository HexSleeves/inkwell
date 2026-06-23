//! Author identity and token-scope types (ADR 0009, plan 023).
//!
//! **Slice 1 is non-enforcing foundation.** These types anchor the scoped-token
//! work but are not yet wired into request auth: there is no token resolution,
//! no `Principal`, and no scope/ownership enforcement in this slice (those are
//! slices 2–3). They are public-but-unused for now, so `dead_code` is allowed
//! until a later slice consumes them.
#![allow(dead_code)]

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
}

impl std::fmt::Display for Scope {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// A first-class principal that can own documents and hold tokens
/// (migration 0011). Foundation only in slice 1: no surface reads this row yet.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, sqlx::FromRow)]
#[serde(rename_all = "camelCase")]
pub struct Author {
    pub id: Uuid,
    pub name: String,
    #[serde(with = "crate::domain::document::timestamp")]
    pub created_at: OffsetDateTime,
}
