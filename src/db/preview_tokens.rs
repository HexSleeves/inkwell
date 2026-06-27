//! Preview token persistence (CIL-129).
//!
//! A preview token is `pvw_<prefix>_<secret>`. Only the `prefix` (indexed
//! lookup handle) and a SHA-256 hash of the full token are stored — the secret
//! is never persisted. Resolution: find by prefix → constant-time hash compare
//! → check expiry + revocation → verify the token's `document_id` matches the
//! requested slug.

use sqlx::{PgPool, Postgres};
use time::OffsetDateTime;
use uuid::Uuid;

/// The columns needed to authenticate a preview token request.
#[derive(Debug, Clone)]
pub struct PreviewTokenRow {
    pub document_id: Uuid,
    pub token_hash: String,
    pub expires_at: Option<OffsetDateTime>,
    pub revoked_at: Option<OffsetDateTime>,
}

/// Token metadata for the management surface. The secret is never recoverable;
/// only the public `prefix` and bookkeeping fields are exposed.
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PreviewTokenInfo {
    pub prefix: String,
    pub document_id: Uuid,
    #[serde(with = "crate::domain::document::timestamp")]
    pub created_at: OffsetDateTime,
    #[serde(with = "crate::domain::document::timestamp::option")]
    pub expires_at: Option<OffsetDateTime>,
    #[serde(with = "crate::domain::document::timestamp::option")]
    pub revoked_at: Option<OffsetDateTime>,
}

/// Insert a freshly minted preview token. Only the `prefix` and `token_hash`
/// (SHA-256 of the full token) are stored; the secret never touches the DB.
pub async fn insert_preview_token(
    pool: &PgPool,
    document_id: Uuid,
    prefix: &str,
    token_hash: &str,
    expires_at: Option<OffsetDateTime>,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        r#"
        INSERT INTO preview_tokens (document_id, prefix, token_hash, expires_at)
        VALUES ($1, $2, $3, $4)
        "#,
    )
    .bind(document_id)
    .bind(prefix)
    .bind(token_hash)
    .bind(expires_at)
    .execute(pool)
    .await?;
    Ok(())
}

/// Resolve a preview token by its public `prefix`. Returns `None` when no row
/// carries that prefix. A revoked or expired row is returned with the
/// corresponding field set so the caller can reject it after the hash compare.
pub async fn find_preview_token_by_prefix(
    pool: &PgPool,
    prefix: &str,
) -> Result<Option<PreviewTokenRow>, sqlx::Error> {
    let row =
        sqlx::query_as::<Postgres, (Uuid, String, Option<OffsetDateTime>, Option<OffsetDateTime>)>(
            r#"
        SELECT document_id, token_hash, expires_at, revoked_at
        FROM preview_tokens
        WHERE prefix = $1
        "#,
        )
        .bind(prefix)
        .fetch_optional(pool)
        .await?;
    Ok(row.map(
        |(document_id, token_hash, expires_at, revoked_at)| PreviewTokenRow {
            document_id,
            token_hash,
            expires_at,
            revoked_at,
        },
    ))
}

/// List all preview tokens for a document, newest first. Both live and revoked
/// tokens are returned so the author can see the full history.
pub async fn list_preview_tokens_for_document(
    pool: &PgPool,
    document_id: Uuid,
) -> Result<Vec<PreviewTokenInfo>, sqlx::Error> {
    let rows = sqlx::query_as::<
        Postgres,
        (
            String,
            Uuid,
            OffsetDateTime,
            Option<OffsetDateTime>,
            Option<OffsetDateTime>,
        ),
    >(
        r#"
        SELECT prefix, document_id, created_at, expires_at, revoked_at
        FROM preview_tokens
        WHERE document_id = $1
        ORDER BY created_at DESC
        "#,
    )
    .bind(document_id)
    .fetch_all(pool)
    .await?;
    Ok(rows
        .into_iter()
        .map(
            |(prefix, document_id, created_at, expires_at, revoked_at)| PreviewTokenInfo {
                prefix,
                document_id,
                created_at,
                expires_at,
                revoked_at,
            },
        )
        .collect())
}

/// Revoke a preview token by `prefix`, scoped to `document_id` so an author
/// cannot revoke another document's tokens. Returns `true` when a live token
/// was revoked, `false` when no matching live token was found.
pub async fn revoke_preview_token(
    pool: &PgPool,
    document_id: Uuid,
    prefix: &str,
) -> Result<bool, sqlx::Error> {
    let affected = sqlx::query(
        r#"
        UPDATE preview_tokens
        SET revoked_at = now()
        WHERE prefix = $1
          AND document_id = $2
          AND revoked_at IS NULL
        "#,
    )
    .bind(prefix)
    .bind(document_id)
    .execute(pool)
    .await?
    .rows_affected();
    Ok(affected > 0)
}
