//! Database layer for the `media` table (migration 0019).
//!
//! Two operations:
//! - [`insert_media`] — persist raw image bytes + metadata, returning the new
//!   row's `id` for URL construction.
//! - [`get_media`] — fetch the `content_type` and `data` for serving; returns
//!   `None` when the id is unknown (surfaced as a 404 by the handler).

use sqlx::PgPool;
use uuid::Uuid;

/// Minimal projection returned by [`get_media`]. Only the serving-required
/// fields are fetched — we never need to re-read the full blob for anything
/// other than serving.
pub struct MediaRow {
    pub content_type: String,
    pub data: Vec<u8>,
}

/// Insert a media blob and return its generated `id`.
///
/// `filename` is optional — the raw-byte upload path does not require one, but
/// it is stored for future multipart/form-data work. `byte_size` is derived
/// from `data.len()` here so callers don't have to duplicate the cast.
pub async fn insert_media(
    pool: &PgPool,
    filename: Option<&str>,
    content_type: &str,
    data: &[u8],
    owner_id: Uuid,
) -> Result<Uuid, sqlx::Error> {
    // `byte_size` is `integer` (4-byte signed) in the schema. The upload cap
    // (5 MiB = 5_242_880) fits safely in i32 (max ~2.1 GiB).
    let byte_size = data.len() as i32;
    let id: Uuid = sqlx::query_scalar(
        r#"
        INSERT INTO media (filename, content_type, byte_size, data, owner_id)
        VALUES ($1, $2, $3, $4, $5)
        RETURNING id
        "#,
    )
    .bind(filename)
    .bind(content_type)
    .bind(byte_size)
    .bind(data)
    .bind(owner_id)
    .fetch_one(pool)
    .await?;
    Ok(id)
}

/// Fetch `content_type` and `data` for a media row by its `id`.
///
/// Returns `None` when no row exists; the handler converts that to a 404.
pub async fn get_media(pool: &PgPool, id: Uuid) -> Result<Option<MediaRow>, sqlx::Error> {
    let row: Option<(String, Vec<u8>)> =
        sqlx::query_as("SELECT content_type, data FROM media WHERE id = $1")
            .bind(id)
            .fetch_optional(pool)
            .await?;
    Ok(row.map(|(content_type, data)| MediaRow { content_type, data }))
}
