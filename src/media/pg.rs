//! Postgres media backend: blobs in the `media_blobs` table (migration 0025).
//!
//! Chosen by `INKWELL_MEDIA_BACKEND=postgres`. Useful where the filesystem is
//! ephemeral and mounting a volume is awkward (e.g. Railway), at the cost of DB
//! storage, WAL traffic, and backup size. Keys are the same content-addressed
//! keys the filesystem backend uses, so switching backends does not change any
//! public `/media/{id}` URL — only where the bytes live.

use async_trait::async_trait;
use sqlx::PgPool;

use super::{MediaStore, StoreError, is_valid_storage_key};

/// Postgres-backed [`MediaStore`] over the `media_blobs` table.
pub struct PgBlobStore {
    pool: PgPool,
}

impl PgBlobStore {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    fn check_key(key: &str) -> Result<(), StoreError> {
        if is_valid_storage_key(key) {
            Ok(())
        } else {
            Err(StoreError::InvalidKey(key.to_string()))
        }
    }
}

#[async_trait]
impl MediaStore for PgBlobStore {
    fn backend(&self) -> &'static str {
        "postgres"
    }

    async fn put(&self, key: &str, bytes: &[u8]) -> Result<(), StoreError> {
        Self::check_key(key)?;
        // Content-addressed, so an existing row already holds these exact bytes:
        // DO NOTHING keeps `put` idempotent without a pointless rewrite.
        sqlx::query(
            "INSERT INTO media_blobs (storage_key, data) VALUES ($1, $2)
             ON CONFLICT (storage_key) DO NOTHING",
        )
        .bind(key)
        .bind(bytes)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    async fn get(&self, key: &str) -> Result<Option<Vec<u8>>, StoreError> {
        Self::check_key(key)?;
        let data: Option<Vec<u8>> =
            sqlx::query_scalar("SELECT data FROM media_blobs WHERE storage_key = $1")
                .bind(key)
                .fetch_optional(&self.pool)
                .await?;
        Ok(data)
    }

    async fn delete(&self, key: &str) -> Result<(), StoreError> {
        Self::check_key(key)?;
        sqlx::query("DELETE FROM media_blobs WHERE storage_key = $1")
            .bind(key)
            .execute(&self.pool)
            .await?;
        Ok(())
    }
}
