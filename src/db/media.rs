//! Database layer for the `media` table (migrations 0019, 0025).
//!
//! The table holds *metadata* — id, filename, mime type, byte size, SHA-256
//! checksum, owner, and the content-addressed `storage_key` — while the bytes
//! themselves live in whichever [`MediaStore`](crate::media::MediaStore) backend
//! the operator configured.
//!
//! Rows written before migration 0025 have `storage_key IS NULL` and carry their
//! bytes inline in `data`; [`get_media`] returns both so the serve path can fall
//! back to the legacy column without a data migration.

use sqlx::PgPool;
use uuid::Uuid;

/// Row shape shared by the serve, delete, and dedup paths.
type MediaTuple = (
    Uuid,
    String,
    i32,
    Option<String>,
    Option<String>,
    Option<Vec<u8>>,
    Uuid,
);

/// Row projection used by the serve, delete, and dedup paths.
pub struct MediaRow {
    pub id: Uuid,
    pub content_type: String,
    pub byte_size: i64,
    /// SHA-256 hex of the bytes. Always set for rows written after migration
    /// 0025 (and backfilled for older rows), so the serve path can emit a strong
    /// ETag; `None` only if a row somehow predates the backfill.
    pub checksum_sha256: Option<String>,
    /// Content-addressed key in the configured backend, or `None` for legacy
    /// rows whose bytes are in [`MediaRow::data`].
    pub storage_key: Option<String>,
    /// Legacy inline bytes (pre-0025 uploads). `None` for storage-backed rows.
    pub data: Option<Vec<u8>>,
    pub owner_id: Uuid,
}

/// Metadata for a newly uploaded blob.
pub struct NewMedia<'a> {
    pub filename: Option<&'a str>,
    pub content_type: &'a str,
    pub byte_size: i64,
    pub checksum_sha256: &'a str,
    pub storage_key: &'a str,
    pub storage_backend: &'a str,
    pub owner_id: Uuid,
}

/// Insert a media row pointing at an already-stored blob, returning its `id`.
///
/// The caller writes bytes to the store *first*, so a committed row always has
/// its blob available and `GET /media/{id}` can never 404 on a live row.
pub async fn insert_media(pool: &PgPool, new: NewMedia<'_>) -> Result<Uuid, sqlx::Error> {
    let id: Uuid = sqlx::query_scalar(
        r#"
        INSERT INTO media (filename, content_type, byte_size, checksum_sha256,
                           storage_key, storage_backend, owner_id)
        VALUES ($1, $2, $3, $4, $5, $6, $7)
        RETURNING id
        "#,
    )
    .bind(new.filename)
    .bind(new.content_type)
    // `byte_size` is `integer` in the schema. The HTTP upload cap is validated
    // before this call and is far below i32::MAX, so the cast cannot wrap.
    .bind(new.byte_size as i32)
    .bind(new.checksum_sha256)
    .bind(new.storage_key)
    .bind(new.storage_backend)
    .bind(new.owner_id)
    .fetch_one(pool)
    .await?;
    Ok(id)
}

/// Fetch a media row by id, or `None` when unknown (a 404 at the HTTP layer).
pub async fn get_media(pool: &PgPool, id: Uuid) -> Result<Option<MediaRow>, sqlx::Error> {
    sqlx::query_as::<_, MediaTuple>(
        "SELECT id, content_type, byte_size, checksum_sha256, storage_key, data, owner_id
           FROM media WHERE id = $1",
    )
    .bind(id)
    .fetch_optional(pool)
    .await
    .map(|row| row.map(into_media_row))
}

/// Find an existing row for the same owner and content checksum.
///
/// Upload dedup: re-uploading identical bytes returns the existing id instead of
/// creating a second row for the same blob. Scoped per owner so one author can't
/// probe another's library by uploading candidate bytes.
pub async fn find_by_owner_and_checksum(
    pool: &PgPool,
    owner_id: Uuid,
    checksum_sha256: &str,
) -> Result<Option<MediaRow>, sqlx::Error> {
    sqlx::query_as::<_, MediaTuple>(
        "SELECT id, content_type, byte_size, checksum_sha256, storage_key, data, owner_id
           FROM media
          WHERE owner_id = $1 AND checksum_sha256 = $2
          ORDER BY created_at
          LIMIT 1",
    )
    .bind(owner_id)
    .bind(checksum_sha256)
    .fetch_optional(pool)
    .await
    .map(|row| row.map(into_media_row))
}

/// Delete the row for `id`, returning `true` when a row was removed.
///
/// The blob is *not* touched here: the caller removes it from the store only
/// after checking that no other row still references the same key (see
/// [`count_rows_for_storage_key`]).
pub async fn delete_media(pool: &PgPool, id: Uuid) -> Result<bool, sqlx::Error> {
    let result = sqlx::query("DELETE FROM media WHERE id = $1")
        .bind(id)
        .execute(pool)
        .await?;
    Ok(result.rows_affected() > 0)
}

/// How many rows still point at `storage_key`.
///
/// Content addressing means two owners uploading identical bytes share one blob,
/// so the blob may only be removed when this reaches zero.
pub async fn count_rows_for_storage_key(
    pool: &PgPool,
    storage_key: &str,
) -> Result<i64, sqlx::Error> {
    sqlx::query_scalar("SELECT count(*) FROM media WHERE storage_key = $1")
        .bind(storage_key)
        .fetch_one(pool)
        .await
}

fn into_media_row(row: MediaTuple) -> MediaRow {
    let (id, content_type, byte_size, checksum_sha256, storage_key, data, owner_id) = row;
    MediaRow {
        id,
        content_type,
        byte_size: i64::from(byte_size),
        checksum_sha256,
        storage_key,
        data,
        owner_id,
    }
}
