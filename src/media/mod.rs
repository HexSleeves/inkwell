//! Pluggable media blob storage (CYP-45, ADR 0013).
//!
//! Uploaded image bytes live behind the [`MediaStore`] trait so the storage
//! target is an operator choice, not an API contract. Two backends ship in v0.2:
//!
//! - [`local::LocalFsStore`] — the default. Bytes land under
//!   `INKWELL_MEDIA_DIR` on the local filesystem (a Docker volume in compose).
//! - [`pg::PgBlobStore`] — bytes in the `media_blobs` Postgres table. Suits
//!   platforms with an ephemeral filesystem (e.g. Railway) where a volume is
//!   inconvenient; costs DB storage and WAL traffic.
//!
//! An object-store backend (S3/R2) can be added by implementing the same trait:
//! keys are already content-addressed and opaque to the HTTP surface, so
//! `/media/{id}` never changes shape.
//!
//! # Naming
//! A blob's key is derived **only** from the SHA-256 of its bytes plus a fixed
//! extension from the MIME allowlist:
//!
//! ```text
//! <hex[0..2]>/<hex[2..4]>/<hex>.<ext>      e.g. 3f/a9/3fa9…c1.png
//! ```
//!
//! Nothing client-controlled (filename, declared type after validation, header
//! text) reaches the path, so traversal is impossible by construction rather
//! than by escaping. The two shard segments keep directory fan-out sane on the
//! filesystem backend. Content addressing also makes uploads idempotent: the
//! same bytes always map to the same key.

pub mod local;
pub mod pg;
pub mod sniff;

use async_trait::async_trait;
use sha2::{Digest, Sha256};

/// MIME types accepted on upload.
///
/// SVG is intentionally excluded: it is XML that browsers execute as active
/// content (`<script>`, event handlers) when served as `image/svg+xml`, which
/// would make any upload a stored-XSS vector.
pub const ALLOWED_MIME_TYPES: &[&str] = &["image/png", "image/jpeg", "image/gif", "image/webp"];

/// Failures a storage backend can report.
#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    /// The key does not match the content-addressed shape. Only reachable from a
    /// bug or from data that bypassed the HTTP layer — never from user input.
    #[error("invalid media storage key \"{0}\"")]
    InvalidKey(String),
    #[error("media filesystem error: {0}")]
    Io(#[from] std::io::Error),
    #[error("media database error: {0}")]
    Database(#[from] sqlx::Error),
}

/// A content-addressed blob store for uploaded media.
///
/// Implementations must treat keys as opaque and MUST reject any key that fails
/// [`is_valid_storage_key`] rather than trying to sanitise it.
#[async_trait]
pub trait MediaStore: Send + Sync {
    /// Stable identifier persisted in `media.storage_backend`, so a row records
    /// where its bytes were written even after the config changes.
    fn backend(&self) -> &'static str;

    /// Write `bytes` at `key`. Idempotent: writing identical bytes to an
    /// existing key succeeds (content addressing means the bytes must match).
    async fn put(&self, key: &str, bytes: &[u8]) -> Result<(), StoreError>;

    /// Read the bytes at `key`, or `None` when the blob is absent.
    async fn get(&self, key: &str) -> Result<Option<Vec<u8>>, StoreError>;

    /// Remove the blob at `key`. Absent blobs are not an error (delete is
    /// idempotent, so a retried delete converges).
    async fn delete(&self, key: &str) -> Result<(), StoreError>;
}

/// Lowercase hex SHA-256 of `bytes` — the content address and the strong ETag.
pub fn checksum_hex(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    let mut out = String::with_capacity(digest.len() * 2);
    for byte in digest {
        use std::fmt::Write as _;
        let _ = write!(out, "{byte:02x}");
    }
    out
}

/// File extension for an allowlisted MIME type, or `None` when not allowlisted.
pub fn extension_for_mime(mime: &str) -> Option<&'static str> {
    match mime {
        "image/png" => Some("png"),
        "image/jpeg" => Some("jpg"),
        "image/gif" => Some("gif"),
        "image/webp" => Some("webp"),
        _ => None,
    }
}

/// Build the content-addressed storage key for `bytes` of type `mime`.
///
/// Returns `None` when `mime` is not allowlisted, so a caller cannot
/// accidentally mint a key for content the serve path would refuse.
pub fn storage_key_for(checksum: &str, mime: &str) -> Option<String> {
    let ext = extension_for_mime(mime)?;
    if checksum.len() != 64 || !checksum.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    let checksum = checksum.to_ascii_lowercase();
    Some(format!(
        "{}/{}/{checksum}.{ext}",
        &checksum[0..2],
        &checksum[2..4]
    ))
}

/// Whether `key` is a well-formed content-addressed key.
///
/// The predicate every backend applies before touching storage: three segments,
/// two hex shard bytes matching the digest prefix, a 64-char lowercase hex
/// digest, and an allowlisted extension. `..`, absolute paths, backslashes,
/// NULs, and unicode lookalikes all fail — there is no unescaping step to get
/// wrong.
pub fn is_valid_storage_key(key: &str) -> bool {
    let mut parts = key.split('/');
    let (Some(shard_a), Some(shard_b), Some(file), None) =
        (parts.next(), parts.next(), parts.next(), parts.next())
    else {
        return false;
    };
    let is_lower_hex = |s: &str| {
        s.bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    };
    if shard_a.len() != 2 || shard_b.len() != 2 || !is_lower_hex(shard_a) || !is_lower_hex(shard_b)
    {
        return false;
    }
    let Some((digest, ext)) = file.split_once('.') else {
        return false;
    };
    if digest.len() != 64 || !is_lower_hex(digest) {
        return false;
    }
    if !ALLOWED_MIME_TYPES
        .iter()
        .filter_map(|mime| extension_for_mime(mime))
        .any(|allowed| allowed == ext)
    {
        return false;
    }
    // Shards must be the digest's own prefix, so a key cannot scatter one blob
    // across directories (and two keys can't disagree about where bytes live).
    &digest[0..2] == shard_a && &digest[2..4] == shard_b
}

#[cfg(test)]
mod tests {
    use super::*;

    const PNG_DIGEST: &str = "3fa9c1000000000000000000000000000000000000000000000000000000abcd";

    #[test]
    fn checksum_is_lowercase_hex_sha256() {
        let hex = checksum_hex(b"hello");
        assert_eq!(
            hex,
            "2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824"
        );
        assert_eq!(hex.len(), 64);
    }

    #[test]
    fn storage_key_shards_on_the_digest_prefix() {
        let key = storage_key_for(PNG_DIGEST, "image/png").expect("allowlisted mime");
        assert_eq!(key, format!("3f/a9/{PNG_DIGEST}.png"));
        assert!(is_valid_storage_key(&key));
    }

    #[test]
    fn storage_key_rejects_non_allowlisted_mime_and_bad_digests() {
        assert!(storage_key_for(PNG_DIGEST, "image/svg+xml").is_none());
        assert!(storage_key_for(PNG_DIGEST, "text/html").is_none());
        assert!(storage_key_for("short", "image/png").is_none());
        assert!(storage_key_for(&"z".repeat(64), "image/png").is_none());
    }

    #[test]
    fn jpeg_and_webp_extensions_are_stable() {
        assert!(
            storage_key_for(PNG_DIGEST, "image/jpeg")
                .expect("jpeg")
                .ends_with(".jpg")
        );
        assert!(
            storage_key_for(PNG_DIGEST, "image/webp")
                .expect("webp")
                .ends_with(".webp")
        );
    }

    #[test]
    fn traversal_and_malformed_keys_are_invalid() {
        for key in [
            "../../etc/passwd",
            "3f/a9/../../../etc/passwd",
            "3f/a9/..%2f..%2fetc%2fpasswd.png",
            "/etc/passwd",
            "3f\\a9\\file.png",
            &format!("3f/a9/{PNG_DIGEST}.png/../../x"),
            // Right shape, wrong extension (would let active content be stored).
            &format!("3f/a9/{PNG_DIGEST}.svg"),
            &format!("3f/a9/{PNG_DIGEST}.html"),
            // Shards that disagree with the digest prefix.
            &format!("aa/bb/{PNG_DIGEST}.png"),
            // Uppercase hex is not the canonical form we mint.
            &format!("3F/A9/{}.png", PNG_DIGEST.to_uppercase()),
            // Missing shard levels / extra levels.
            &format!("{PNG_DIGEST}.png"),
            &format!("3f/a9/c1/{PNG_DIGEST}.png"),
            "",
        ] {
            assert!(!is_valid_storage_key(key), "must reject key {key:?}");
        }
    }
}
