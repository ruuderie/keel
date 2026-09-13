//! Filesystem blob store. SQLite schema lives in `/sql/schema.sql` (v0).

use async_trait::async_trait;
use keel_types::{ArtifactStore, ContentId};
use std::path::{Path, PathBuf};
use tokio::fs;

#[derive(Clone, Debug)]
pub struct FsArtifactStore {
    root: PathBuf,
}

impl FsArtifactStore {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    pub fn path_for(&self, cid: &ContentId) -> PathBuf {
        let hex = hex::encode(cid.0);
        self.root
            .join("blobs")
            .join("sha256")
            .join(&hex[..2])
            .join(&hex)
    }
}

#[async_trait]
impl ArtifactStore for FsArtifactStore {
    async fn put(&self, bytes: &[u8]) -> Result<ContentId, String> {
        let cid = ContentId::of_bytes(bytes);
        let path = self.path_for(&cid);
        if let Some(dir) = path.parent() {
            fs::create_dir_all(dir)
                .await
                .map_err(|e| e.to_string())?;
        }
        if !path.exists() {
            fs::write(&path, bytes).await.map_err(|e| e.to_string())?;
        }
        Ok(cid)
    }

    async fn get(&self, cid: &ContentId) -> Result<Vec<u8>, String> {
        fs::read(self.path_for(cid))
            .await
            .map_err(|e| e.to_string())
    }

    async fn has(&self, cid: &ContentId) -> Result<bool, String> {
        Ok(Path::new(&self.path_for(cid)).exists())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn put_get_roundtrip() {
        let dir = std::env::temp_dir().join(format!("keel-blob-{}", std::process::id()));
        let store = FsArtifactStore::new(&dir);
        let cid = store.put(b"gguf-bytes").await.unwrap();
        assert!(store.has(&cid).await.unwrap());
        assert_eq!(store.get(&cid).await.unwrap(), b"gguf-bytes");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
