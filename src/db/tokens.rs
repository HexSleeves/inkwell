//! Author and scoped-token persistence (ADR 0009, plan 023, slice 2).
//!
//! Token resolution is a single indexed lookup by the public `prefix`, after
//! which `auth::authenticate` constant-time compares the recomputed hash against
//! [`ResolvedToken::token_hash`]. The full token is never stored — see
//! [`crate::domain::token`]. Scopes cross the DB boundary as `Vec<String>`
//! (the `text[]` column, validated by the migration 0012 CHECK constraint) and
//! are decoded into [`Scope`](crate::domain::author::Scope) by the caller.

use sqlx::{PgPool, Postgres};
use time::OffsetDateTime;
use uuid::Uuid;

use crate::domain::author::Author;

/// A token row resolved by its `prefix`, with just the fields auth needs.
#[derive(Debug, Clone)]
pub struct ResolvedToken {
    pub author_id: Uuid,
    /// The owning author's name — becomes the principal's audit label.
    pub author_name: String,
    /// SHA-256 hex of the whole token, compared in constant time by the caller.
    pub token_hash: String,
    /// Raw scope strings; decoded to `Scope` by the caller.
    pub scopes: Vec<String>,
    /// `true` once the token has been revoked — a revoked token never authenticates.
    pub revoked: bool,
}

/// A token's metadata for the `token list` admin surface. The secret is never
/// recoverable, so only the public `prefix` and bookkeeping fields are returned.
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TokenInfo {
    pub prefix: String,
    pub author_name: String,
    pub scopes: Vec<String>,
    #[serde(with = "crate::domain::document::timestamp")]
    pub created_at: OffsetDateTime,
    #[serde(with = "crate::domain::document::timestamp::option")]
    pub last_used_at: Option<OffsetDateTime>,
    #[serde(with = "crate::domain::document::timestamp::option")]
    pub revoked_at: Option<OffsetDateTime>,
}

/// Find an author by exact name (`authors.name` is unique).
pub async fn find_author_by_name(pool: &PgPool, name: &str) -> Result<Option<Author>, sqlx::Error> {
    sqlx::query_as::<Postgres, Author>("SELECT id, name, created_at FROM authors WHERE name = $1")
        .bind(name)
        .fetch_optional(pool)
        .await
}

/// Find an author by name, creating one if absent. The `ON CONFLICT` makes this
/// safe under the unique-name constraint even if two creates race.
pub async fn find_or_create_author(pool: &PgPool, name: &str) -> Result<Author, sqlx::Error> {
    sqlx::query_as::<Postgres, Author>(
        r#"
        INSERT INTO authors (name) VALUES ($1)
        ON CONFLICT (name) DO UPDATE SET name = EXCLUDED.name
        RETURNING id, name, created_at
        "#,
    )
    .bind(name)
    .fetch_one(pool)
    .await
}

/// Insert a freshly minted token for `author_id`. Only the `prefix` and
/// `token_hash` are stored; the secret lives only in the response shown once.
pub async fn insert_token(
    pool: &PgPool,
    author_id: Uuid,
    prefix: &str,
    token_hash: &str,
    scopes: &[String],
) -> Result<(), sqlx::Error> {
    sqlx::query(
        r#"
        INSERT INTO author_tokens (author_id, token_hash, prefix, scopes)
        VALUES ($1, $2, $3, $4)
        "#,
    )
    .bind(author_id)
    .bind(token_hash)
    .bind(prefix)
    .bind(scopes)
    .execute(pool)
    .await?;
    Ok(())
}

/// Resolve a token by its public `prefix` for authentication. Returns `None`
/// when no row carries that prefix; a revoked row is returned with
/// `revoked = true` so the caller can reject it after the hash compare.
pub async fn find_token_by_prefix(
    pool: &PgPool,
    prefix: &str,
) -> Result<Option<ResolvedToken>, sqlx::Error> {
    let row =
        sqlx::query_as::<Postgres, (Uuid, String, String, Vec<String>, Option<OffsetDateTime>)>(
            r#"
        SELECT t.author_id, a.name, t.token_hash, t.scopes, t.revoked_at
        FROM author_tokens t
        JOIN authors a ON a.id = t.author_id
        WHERE t.prefix = $1
        "#,
        )
        .bind(prefix)
        .fetch_optional(pool)
        .await?;
    Ok(row.map(
        |(author_id, author_name, token_hash, scopes, revoked_at)| ResolvedToken {
            author_id,
            author_name,
            token_hash,
            scopes,
            revoked: revoked_at.is_some(),
        },
    ))
}

/// Stamp `last_used_at = now()` on a live token. Best-effort: a stale timestamp
/// never affects auth, so callers ignore the error.
pub async fn touch_last_used(pool: &PgPool, prefix: &str) -> Result<(), sqlx::Error> {
    sqlx::query("UPDATE author_tokens SET last_used_at = now() WHERE prefix = $1")
        .bind(prefix)
        .execute(pool)
        .await?;
    Ok(())
}

/// List tokens (newest first) with their owning author and bookkeeping.
///
/// When `include_revoked` is `false` (the default), only live tokens
/// (`revoked_at IS NULL`) are returned. Pass `true` to include revoked rows
/// as well (e.g. for admin audit or the `--all` flag).
pub async fn list_tokens(
    pool: &PgPool,
    include_revoked: bool,
) -> Result<Vec<TokenInfo>, sqlx::Error> {
    let sql = if include_revoked {
        r#"
        SELECT t.prefix, a.name, t.scopes, t.created_at, t.last_used_at, t.revoked_at
        FROM author_tokens t
        JOIN authors a ON a.id = t.author_id
        ORDER BY t.created_at DESC
        "#
    } else {
        r#"
        SELECT t.prefix, a.name, t.scopes, t.created_at, t.last_used_at, t.revoked_at
        FROM author_tokens t
        JOIN authors a ON a.id = t.author_id
        WHERE t.revoked_at IS NULL
        ORDER BY t.created_at DESC
        "#
    };
    let rows = sqlx::query_as::<
        Postgres,
        (
            String,
            String,
            Vec<String>,
            OffsetDateTime,
            Option<OffsetDateTime>,
            Option<OffsetDateTime>,
        ),
    >(sql)
    .fetch_all(pool)
    .await?;
    Ok(rows
        .into_iter()
        .map(
            |(prefix, author_name, scopes, created_at, last_used_at, revoked_at)| TokenInfo {
                prefix,
                author_name,
                scopes,
                created_at,
                last_used_at,
                revoked_at,
            },
        )
        .collect())
}

/// Hard-delete all revoked tokens. Returns the number of rows deleted.
///
/// Only tokens with `revoked_at IS NOT NULL` are removed, so live tokens are
/// never touched. Because `write_audit` rows reference `authors` (not tokens)
/// this loses no audit history.
pub async fn prune_revoked_tokens(pool: &PgPool) -> Result<u64, sqlx::Error> {
    let affected = sqlx::query("DELETE FROM author_tokens WHERE revoked_at IS NOT NULL")
        .execute(pool)
        .await?
        .rows_affected();
    Ok(affected)
}

/// Revoke a token by `prefix`. Returns `true` when a still-live token was
/// revoked, `false` when no such token exists or it was already revoked
/// (idempotent: a second revoke is a no-op `false`).
pub async fn revoke_token(pool: &PgPool, prefix: &str) -> Result<bool, sqlx::Error> {
    let affected = sqlx::query(
        "UPDATE author_tokens SET revoked_at = now() WHERE prefix = $1 AND revoked_at IS NULL",
    )
    .bind(prefix)
    .execute(pool)
    .await?
    .rows_affected();
    Ok(affected > 0)
}
