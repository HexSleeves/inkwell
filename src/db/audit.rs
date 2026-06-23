//! Write-audit trail inserts (ADR 0009, plan 023, slice 1).
//!
//! One row per successful mutating action. The insert is **best-effort** at the
//! call site (mirroring `garden::persist_source_edges`): a failure is logged and
//! never changes the handler's response. This module just owns the SQL + the
//! small action vocabulary; the handlers decide actor/label.

use sqlx::PgPool;
use uuid::Uuid;

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
