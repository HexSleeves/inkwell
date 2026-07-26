//! Local-filesystem media backend (the v0.2 default).
//!
//! Blobs are written under a configured root (`INKWELL_MEDIA_DIR`) at the
//! content-addressed key, e.g. `<root>/3f/a9/3fa9….png`. Writes are atomic:
//! bytes go to a unique temp file in the same shard directory and are then
//! renamed, so a crash mid-write can never leave a truncated blob that would be
//! served as a valid image (and `rename(2)` within one directory is atomic on
//! every filesystem we support).

use std::path::{Path, PathBuf};

use async_trait::async_trait;
use uuid::Uuid;

use super::{MediaStore, StoreError, is_valid_storage_key};

/// Filesystem-backed [`MediaStore`] rooted at a single directory.
pub struct LocalFsStore {
    root: PathBuf,
}

impl LocalFsStore {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    /// Resolve `key` to an absolute path under the root.
    ///
    /// Validation happens here, once, for every operation: an invalid key is an
    /// error, never something to sanitise. Because valid keys are pure hex plus a
    /// fixed extension, the join cannot escape the root.
    fn path_for(&self, key: &str) -> Result<PathBuf, StoreError> {
        if !is_valid_storage_key(key) {
            return Err(StoreError::InvalidKey(key.to_string()));
        }
        Ok(self.root.join(key))
    }
}

#[async_trait]
impl MediaStore for LocalFsStore {
    fn backend(&self) -> &'static str {
        "local"
    }

    async fn put(&self, key: &str, bytes: &[u8]) -> Result<(), StoreError> {
        let path = self.path_for(key)?;
        let parent = path
            .parent()
            .map(Path::to_path_buf)
            .unwrap_or_else(|| self.root.clone());
        tokio::fs::create_dir_all(&parent).await?;

        // Unique temp name: two concurrent uploads of the same bytes must not
        // write the same temp file and truncate each other's data.
        let temp = parent.join(format!(".tmp-{}", Uuid::new_v4()));
        match tokio::fs::write(&temp, bytes).await {
            Ok(()) => {}
            Err(error) => {
                let _ = tokio::fs::remove_file(&temp).await;
                return Err(error.into());
            }
        }
        if let Err(error) = tokio::fs::rename(&temp, &path).await {
            let _ = tokio::fs::remove_file(&temp).await;
            return Err(error.into());
        }
        Ok(())
    }

    async fn get(&self, key: &str) -> Result<Option<Vec<u8>>, StoreError> {
        let path = self.path_for(key)?;
        match tokio::fs::read(&path).await {
            Ok(bytes) => Ok(Some(bytes)),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(error) => Err(error.into()),
        }
    }

    async fn delete(&self, key: &str) -> Result<(), StoreError> {
        let path = self.path_for(key)?;
        match tokio::fs::remove_file(&path).await {
            Ok(()) => Ok(()),
            // Already gone: delete is idempotent so a retry converges.
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(error.into()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::media::{checksum_hex, storage_key_for};

    fn temp_root(label: &str) -> PathBuf {
        std::env::temp_dir().join(format!("inkwell-media-{label}-{}", Uuid::new_v4()))
    }

    #[tokio::test]
    async fn round_trips_and_deletes_a_blob() {
        let root = temp_root("roundtrip");
        let store = LocalFsStore::new(&root);
        let bytes = b"\x89PNG\r\n\x1a\nnot-a-real-image".to_vec();
        let key = storage_key_for(&checksum_hex(&bytes), "image/png").expect("key");

        assert_eq!(store.get(&key).await.expect("get"), None);
        store.put(&key, &bytes).await.expect("put");
        assert_eq!(store.get(&key).await.expect("get"), Some(bytes.clone()));

        // Sharded layout: <root>/<2>/<2>/<digest>.png
        assert!(root.join(&key).is_file());

        store.delete(&key).await.expect("delete");
        assert_eq!(store.get(&key).await.expect("get"), None);
        // Deleting again is a no-op, not an error.
        store.delete(&key).await.expect("idempotent delete");

        let _ = std::fs::remove_dir_all(&root);
    }

    #[tokio::test]
    async fn put_is_idempotent_for_identical_content() {
        let root = temp_root("idempotent");
        let store = LocalFsStore::new(&root);
        let bytes = b"GIF89a-bytes".to_vec();
        let key = storage_key_for(&checksum_hex(&bytes), "image/gif").expect("key");

        store.put(&key, &bytes).await.expect("first put");
        store.put(&key, &bytes).await.expect("second put");
        assert_eq!(store.get(&key).await.expect("get"), Some(bytes));

        // No temp files left behind.
        let shard = root.join(&key[0..5]);
        let leftovers: Vec<_> = std::fs::read_dir(&shard)
            .expect("shard dir")
            .filter_map(Result::ok)
            .filter(|e| e.file_name().to_string_lossy().starts_with(".tmp-"))
            .collect();
        assert!(leftovers.is_empty(), "temp files must be cleaned up");

        let _ = std::fs::remove_dir_all(&root);
    }

    #[tokio::test]
    async fn traversal_keys_are_refused_on_every_operation() {
        let root = temp_root("traversal");
        let store = LocalFsStore::new(&root);

        for key in ["../../etc/passwd", "/etc/passwd", "3f/a9/evil.png"] {
            assert!(matches!(
                store.put(key, b"x").await,
                Err(StoreError::InvalidKey(_))
            ));
            assert!(matches!(
                store.get(key).await,
                Err(StoreError::InvalidKey(_))
            ));
            assert!(matches!(
                store.delete(key).await,
                Err(StoreError::InvalidKey(_))
            ));
        }
        // Nothing was created outside (or inside) the root.
        assert!(!root.exists(), "refused keys must not create directories");
    }
}
