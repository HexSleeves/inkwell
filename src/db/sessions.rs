//! Browser session persistence (ADR 0010).
//!
//! Only the SHA-256 hash of the session token is stored — never the raw token.
//! This mirrors the scoped-token pattern in [`crate::db::tokens`].
//!
//! Sessions are created by [`crate::http::auth_session::login`] (when
//! `INKWELL_BROWSER_LOGIN=true`) and deleted on logout or expiry. The migration
//! that creates the `sessions` table is `migrations/0020_create_sessions.sql`.

use sqlx::PgPool;
use time::OffsetDateTime;
use uuid::Uuid;

/// A resolved session row, with the author name joined in for `Principal` construction.
#[derive(Debug, Clone)]
pub struct SessionRow {
    pub author_id: Uuid,
    /// The owning author's name — becomes the session principal's audit label.
    pub author_name: String,
    /// Scopes inherited from the minting token — the session must grant exactly
    /// these (never more). Decoded to `Scope` by the caller.
    pub scopes: Vec<String>,
    /// UTC expiry instant; the caller must reject expired sessions.
    pub expires_at: OffsetDateTime,
}

/// Insert a new session row. Only the `session_token_hash` is stored; the raw
/// token is set in the `Set-Cookie` response and never persisted. `scopes` are
/// the originating token's scopes — the session never grants more.
pub async fn create_session(
    pool: &PgPool,
    author_id: Uuid,
    session_token_hash: &str,
    scopes: &[String],
    expires_at: OffsetDateTime,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "INSERT INTO sessions (session_token_hash, author_id, scopes, expires_at) VALUES ($1, $2, $3, $4)",
    )
    .bind(session_token_hash)
    .bind(author_id)
    .bind(scopes)
    .bind(expires_at)
    .execute(pool)
    .await?;
    Ok(())
}

/// Look up a session by its token hash, joining the author name so the caller
/// can build a [`crate::domain::author::Principal`]. Returns `None` when no
/// matching row exists; expiry is checked by the caller.
pub async fn find_session_by_hash(
    pool: &PgPool,
    hash: &str,
) -> Result<Option<SessionRow>, sqlx::Error> {
    let row = sqlx::query_as::<_, (Uuid, String, Vec<String>, OffsetDateTime)>(
        r#"
        SELECT s.author_id, a.name, s.scopes, s.expires_at
        FROM   sessions s
        JOIN   authors  a ON a.id = s.author_id
        WHERE  s.session_token_hash = $1
        "#,
    )
    .bind(hash)
    .fetch_optional(pool)
    .await?;
    Ok(
        row.map(|(author_id, author_name, scopes, expires_at)| SessionRow {
            author_id,
            author_name,
            scopes,
            expires_at,
        }),
    )
}

/// Delete the session row identified by its token hash. Used by `POST
/// /auth/logout`. A no-op if the hash is not found (idempotent).
pub async fn delete_session_by_hash(pool: &PgPool, hash: &str) -> Result<(), sqlx::Error> {
    sqlx::query("DELETE FROM sessions WHERE session_token_hash = $1")
        .bind(hash)
        .execute(pool)
        .await?;
    Ok(())
}
