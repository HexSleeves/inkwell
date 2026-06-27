use serde::{Deserialize, Serialize};
use time::OffsetDateTime;
use uuid::Uuid;

pub const MAX_SLUG_LENGTH: usize = 200;
pub const MAX_TITLE_LENGTH: usize = 500;
pub const MAX_TAG_LENGTH: usize = 50;
pub const MAX_TAGS: usize = 20;
pub const MAX_BODY_MARKDOWN_LENGTH: usize = 262_144;
/// Upper bound on the raw request body accepted by the authoring API, checked
/// before JSON parsing. Sized above `MAX_BODY_MARKDOWN_LENGTH` to leave room
/// for envelope fields while still bounding per-request memory.
pub const MAX_REQUEST_BODY_BYTES: usize = 1_000_000;
pub const DEFAULT_LIMIT: u32 = 20;
pub const MAX_LIMIT: u32 = 100;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize, sqlx::Type)]
#[sqlx(type_name = "text", rename_all = "lowercase")]
#[serde(rename_all = "lowercase")]
pub enum DocumentStatus {
    Draft,
    Published,
}

impl DocumentStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Draft => "draft",
            Self::Published => "published",
        }
    }
}

impl std::fmt::Display for DocumentStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Digital-garden maturity of a note. A note grows from a rough `Seedling`
/// through `Budding` to a polished `Evergreen`. Stored as `text` with a CHECK
/// constraint (migration 0007), decoded directly into this enum — exactly the
/// shape of [`DocumentStatus`]. New notes default to [`Seedling`](Self::Seedling).
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize, sqlx::Type)]
#[sqlx(type_name = "text", rename_all = "lowercase")]
#[serde(rename_all = "lowercase")]
pub enum GrowthStage {
    #[default]
    Seedling,
    Budding,
    Evergreen,
}

impl GrowthStage {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Seedling => "seedling",
            Self::Budding => "budding",
            Self::Evergreen => "evergreen",
        }
    }

    /// Parse the wire/front-matter token into a stage, or `None` if unknown.
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "seedling" => Some(Self::Seedling),
            "budding" => Some(Self::Budding),
            "evergreen" => Some(Self::Evergreen),
            _ => None,
        }
    }
}

impl std::fmt::Display for GrowthStage {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

#[derive(Clone, Debug, Serialize, sqlx::FromRow)]
#[serde(rename_all = "camelCase")]
pub struct Document {
    pub id: Uuid,
    pub slug: String,
    pub title: String,
    pub body_markdown: String,
    pub rendered_html: String,
    pub status: DocumentStatus,
    /// Digital-garden maturity stage. Defaults to `seedling` (migration 0007).
    pub growth: GrowthStage,
    pub tags: Vec<String>,
    /// Monotonic per-note revision counter, bumped on every content edit.
    /// Powers the MCP `If-Match` optimistic-concurrency check (T6).
    pub version: i64,
    #[serde(with = "timestamp")]
    pub created_at: OffsetDateTime,
    #[serde(with = "timestamp")]
    pub updated_at: OffsetDateTime,
}

impl Document {
    pub fn body_markdown(&self) -> &str {
        &self.body_markdown
    }

    pub fn rendered_html(&self) -> &str {
        &self.rendered_html
    }
}

#[derive(Clone, Debug)]
pub struct NewDocument {
    pub slug: String,
    pub title: String,
    pub body_markdown: String,
    pub rendered_html: String,
    pub status: Option<DocumentStatus>,
    /// Maturity stage; `None` lets the column default to `seedling`.
    pub growth: Option<GrowthStage>,
    pub tags: Vec<String>,
    /// Owning author (ADR 0009 slice 3). `None` falls back to the bootstrap
    /// admin via the column default, preserving pre-token behavior.
    pub owner_id: Option<Uuid>,
}

#[derive(Clone, Debug, Default)]
pub struct DocumentPatch {
    pub title: Option<String>,
    pub body_markdown: Option<String>,
    pub rendered_html: Option<String>,
    pub growth: Option<GrowthStage>,
    pub tags: Option<Vec<String>>,
    /// A new slug to rename the document to (ADR 0011). When `Some` and different
    /// from the current slug, the update records the old slug as a 301 alias and
    /// changes `documents.slug` atomically. `None` leaves the slug untouched.
    pub new_slug: Option<String>,
}

#[derive(Clone, Debug, Default)]
pub struct StatusFilter {
    pub status: Option<DocumentStatus>,
}

#[derive(Clone, Debug, Default)]
pub struct ListOptions {
    pub limit: Option<u32>,
    pub offset: Option<u32>,
    pub status: Option<DocumentStatus>,
}

#[derive(Clone, Debug, Default)]
pub struct ListByTagOptions {
    pub limit: Option<u32>,
    pub offset: Option<u32>,
    pub status: Option<DocumentStatus>,
}

#[derive(Clone, Debug, Default)]
pub struct SearchOptions {
    pub limit: Option<u32>,
    pub offset: Option<u32>,
}

#[derive(Clone, Debug, Serialize, sqlx::FromRow)]
pub struct TagCount {
    pub tag: String,
    pub count: i64,
}

/// A year/month bucket of published documents, returned by `list_archive_months`.
#[derive(Clone, Debug, sqlx::FromRow)]
pub struct ArchiveMonth {
    pub year: i32,
    pub month: i32,
    pub count: i64,
}

/// Lightweight slug+title pair used for previous/next document navigation.
#[derive(Clone, Debug, sqlx::FromRow)]
pub struct AdjacentDoc {
    pub slug: String,
    pub title: String,
}

pub mod timestamp {
    use serde::{Deserialize, Deserializer, Serializer};
    use time::OffsetDateTime;
    use time::format_description::well_known::Rfc3339;

    pub fn serialize<S>(value: &OffsetDateTime, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let text = serialize_to_string(value);
        serializer.serialize_str(&text)
    }

    pub fn deserialize<'de, D>(deserializer: D) -> Result<OffsetDateTime, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        OffsetDateTime::parse(&value, &Rfc3339).map_err(serde::de::Error::custom)
    }

    pub fn serialize_to_string(value: &OffsetDateTime) -> String {
        let value = value.to_offset(time::UtcOffset::UTC);
        let value = value
            .replace_nanosecond((value.nanosecond() / 1_000_000) * 1_000_000)
            .unwrap_or(value);
        let base = value.format(&Rfc3339).unwrap_or_else(|_| value.to_string());
        if base.contains('.') {
            base.replace("+00:00", "Z")
        } else if let Some(stripped) = base.strip_suffix('Z') {
            format!("{stripped}.000Z")
        } else if let Some(stripped) = base.strip_suffix("+00:00") {
            format!("{stripped}.000Z")
        } else {
            base
        }
    }

    /// `Option<OffsetDateTime>` flavour for nullable timestamps (e.g. a token's
    /// `last_used_at` / `revoked_at`): `null` when absent, else the same RFC3339
    /// string this module's scalar form emits.
    pub mod option {
        use serde::{Deserialize, Deserializer, Serializer};
        use time::OffsetDateTime;
        use time::format_description::well_known::Rfc3339;

        pub fn serialize<S>(
            value: &Option<OffsetDateTime>,
            serializer: S,
        ) -> Result<S::Ok, S::Error>
        where
            S: Serializer,
        {
            match value {
                Some(value) => serializer.serialize_some(&super::serialize_to_string(value)),
                None => serializer.serialize_none(),
            }
        }

        pub fn deserialize<'de, D>(deserializer: D) -> Result<Option<OffsetDateTime>, D::Error>
        where
            D: Deserializer<'de>,
        {
            let value = Option::<String>::deserialize(deserializer)?;
            match value {
                Some(text) => OffsetDateTime::parse(&text, &Rfc3339)
                    .map(Some)
                    .map_err(serde::de::Error::custom),
                None => Ok(None),
            }
        }
    }
}
