-- Migration 0025: pluggable media storage backends (CYP-45, ADR 0012).
--
-- Migration 0019 stored every image blob inline in `media.data`. v0.2 adds a
-- storage trait with two backends (local filesystem, Postgres), so a row now
-- points at a *content-addressed storage key* instead of carrying the bytes.
--
-- Rows written before this migration keep their inline `data` and are still
-- served from it (the serve path falls back to `data` when `storage_key` is
-- NULL), so no blob has to be rewritten to upgrade.
ALTER TABLE media
    ADD COLUMN IF NOT EXISTS checksum_sha256 text,
    ADD COLUMN IF NOT EXISTS storage_key     text,
    ADD COLUMN IF NOT EXISTS storage_backend text;

-- Blobs may now live outside this table.
ALTER TABLE media ALTER COLUMN data DROP NOT NULL;

-- The old check hard-coded the 5 MiB cap. The cap is now configurable
-- (`INKWELL_MEDIA_MAX_BYTES`), so keep the byte_size/data agreement invariant
-- and a generous backstop ceiling instead of the exact HTTP limit.
ALTER TABLE media DROP CONSTRAINT IF EXISTS media_size_check;
ALTER TABLE media ADD CONSTRAINT media_size_check CHECK (
    byte_size BETWEEN 0 AND 268435456
    AND (data IS NULL OR byte_size = octet_length(data))
);

-- Exactly one blob location per row: legacy inline `data` XOR a storage key.
ALTER TABLE media DROP CONSTRAINT IF EXISTS media_blob_location_check;
ALTER TABLE media ADD CONSTRAINT media_blob_location_check CHECK (
    (data IS NOT NULL) <> (storage_key IS NOT NULL)
);

-- Storage keys are derived only from the content checksum and a fixed extension
-- allowlist: two hex shard segments, the full digest, and the extension. The
-- pattern is a defence-in-depth guard so no code path (or ad-hoc SQL) can store
-- a key containing `..`, an absolute path, or any other traversal payload.
ALTER TABLE media DROP CONSTRAINT IF EXISTS media_storage_key_check;
ALTER TABLE media ADD CONSTRAINT media_storage_key_check CHECK (
    storage_key IS NULL
    OR storage_key ~ '^[0-9a-f]{2}/[0-9a-f]{2}/[0-9a-f]{64}\.(png|jpg|gif|webp)$'
);

ALTER TABLE media DROP CONSTRAINT IF EXISTS media_checksum_check;
ALTER TABLE media ADD CONSTRAINT media_checksum_check CHECK (
    checksum_sha256 IS NULL OR checksum_sha256 ~ '^[0-9a-f]{64}$'
);

-- Backfill checksums for pre-existing rows so every row can serve a strong
-- ETag. `sha256(bytea)` is core Postgres (11+), no pgcrypto needed.
UPDATE media
   SET checksum_sha256 = encode(sha256(data), 'hex')
 WHERE checksum_sha256 IS NULL
   AND data IS NOT NULL;

-- Blob table for the Postgres storage backend. Keyed by the same
-- content-addressed key the filesystem backend uses, so switching backends only
-- changes where bytes live — never the public `/media/{id}` URL.
CREATE TABLE IF NOT EXISTS media_blobs (
    storage_key text        PRIMARY KEY,
    data        bytea       NOT NULL,
    created_at  timestamptz NOT NULL DEFAULT now(),
    CONSTRAINT media_blobs_key_check CHECK (
        storage_key ~ '^[0-9a-f]{2}/[0-9a-f]{2}/[0-9a-f]{64}\.(png|jpg|gif|webp)$'
    )
);

-- Dedup lookup on upload (same owner re-uploading identical bytes) and the
-- orphan check on delete (is any other row still pointing at this blob?).
CREATE INDEX IF NOT EXISTS media_owner_checksum_idx ON media (owner_id, checksum_sha256);
CREATE INDEX IF NOT EXISTS media_storage_key_idx ON media (storage_key);
