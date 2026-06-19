use serde::{Deserialize, Serialize};
use time::OffsetDateTime;
use uuid::Uuid;

pub const MAX_SLUG_LENGTH: usize = 200;
pub const MAX_TITLE_LENGTH: usize = 500;
pub const MAX_TAG_LENGTH: usize = 50;
pub const MAX_TAGS: usize = 20;
pub const MAX_BODY_MARKDOWN_LENGTH: usize = 262_144;
pub const DEFAULT_LIMIT: u32 = 20;
pub const MAX_LIMIT: u32 = 100;

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, sqlx::Type)]
#[sqlx(type_name = "text")]
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

#[derive(Clone, Debug, Serialize, sqlx::FromRow)]
#[serde(rename_all = "camelCase")]
pub struct Document {
    pub id: Uuid,
    pub slug: String,
    pub title: String,
    pub body_markdown: String,
    pub rendered_html: String,
    pub status: DocumentStatus,
    pub tags: Vec<String>,
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
    pub tags: Vec<String>,
}

#[derive(Clone, Debug, Default)]
pub struct DocumentPatch {
    pub title: Option<String>,
    pub body_markdown: Option<String>,
    pub rendered_html: Option<String>,
    pub tags: Option<Vec<String>>,
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
}
