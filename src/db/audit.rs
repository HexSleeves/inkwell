//! Write-audit trail reads and inserts (ADR 0009, plan 023, slice 1).
//!
//! One row per successful mutating action. The insert is **best-effort** at the
//! call site (mirroring `garden::persist_source_edges`): a failure is logged and
//! never changes the handler's response. This module just owns the SQL + the
//! small action vocabulary; the handlers decide actor/label.

use sqlx::PgPool;
use time::OffsetDateTime;
use uuid::Uuid;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AuditEntry {
    pub action: String,
    pub actor_label: String,
    pub at: OffsetDateTime,
}

/// A mutating action recorded in `write_audit`. The string forms match the
/// `write_audit_action_check` constraint (migration 0014).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AuditAction {
    Create,
    Update,
    Delete,
    Publish,
    Unpublish,
}

impl AuditAction {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Create => "create",
            Self::Update => "update",
            Self::Delete => "delete",
            Self::Publish => "publish",
            Self::Unpublish => "unpublish",
        }
    }
}

/// Insert one `write_audit` row. Slice 1 always attributes writes to the
/// bootstrap admin (`actor_author_id`) with `actor_label = "shared-key"`, since
/// there is no per-request principal yet. `document_id` is stored without an FK
/// so the row survives the document's deletion.
pub async fn record_write(
    pool: &PgPool,
    actor_author_id: Option<Uuid>,
    actor_label: &str,
    action: AuditAction,
    document_id: Option<Uuid>,
    slug: Option<&str>,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        r#"
        INSERT INTO write_audit (actor_author_id, actor_label, action, document_id, slug)
        VALUES ($1, $2, $3, $4, $5)
        "#,
    )
    .bind(actor_author_id)
    .bind(actor_label)
    .bind(action.as_str())
    .bind(document_id)
    .bind(slug)
    .execute(pool)
    .await?;
    Ok(())
}

/// Resolve a document for the history surface. `owner = None` is the admin path;
/// `Some(id)` restricts the lookup to documents owned by that author.
pub async fn resolve_history_document_id(
    pool: &PgPool,
    slug: &str,
    owner: Option<Uuid>,
) -> Result<Option<Uuid>, sqlx::Error> {
    sqlx::query_scalar::<_, Uuid>(
        "SELECT id FROM documents WHERE slug = $1 AND ($2::uuid IS NULL OR owner_id = $2)",
    )
    .bind(slug)
    .bind(owner)
    .fetch_optional(pool)
    .await
}

pub async fn list_audit_for_document(
    pool: &PgPool,
    document_id: Uuid,
    limit: i64,
    offset: i64,
) -> Result<Vec<AuditEntry>, sqlx::Error> {
    sqlx::query_as::<_, (String, String, OffsetDateTime)>(
        "SELECT action, actor_label, at FROM write_audit \
         WHERE document_id = $1 ORDER BY at DESC LIMIT $2 OFFSET $3",
    )
    .bind(document_id)
    .bind(limit)
    .bind(offset)
    .fetch_all(pool)
    .await
    .map(|rows| {
        rows.into_iter()
            .map(|(action, actor_label, at)| AuditEntry {
                action,
                actor_label,
                at,
            })
            .collect()
    })
}
