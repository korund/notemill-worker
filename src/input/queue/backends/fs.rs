//! Filesystem `BlobStore` backend.
//!
//! Maps `blob_key` directly under a configured root directory. Used for the
//! local docker-compose deployment where bot and worker share a volume.
//! Behaviour matches contract section 5.2.

use std::future::Future;
use std::io::ErrorKind;
use std::path::{Component, Path, PathBuf};

use sha2::{Digest, Sha256};
use tokio::fs;
use tokio::io::AsyncWriteExt;

use crate::input::queue::blob::{BlobMeta, BlobStore};
use crate::{Error, Result};

#[derive(Clone)]
pub struct FsBlobStore {
    root: PathBuf,
}

impl FsBlobStore {
    /// Bind to a directory. Created if missing.
    pub fn open(root: impl Into<PathBuf>) -> Result<Self> {
        let root = root.into();
        std::fs::create_dir_all(&root)
            .map_err(|e| Error::Blob(format!("create root {}: {e}", root.display())))?;
        Ok(Self { root })
    }

    fn resolve(&self, key: &str) -> Result<PathBuf> {
        validate_key(key)?;
        let mut p = self.root.clone();
        for seg in key.split('/') {
            p.push(seg);
        }
        Ok(p)
    }
}

fn validate_key(key: &str) -> Result<()> {
    if key.is_empty() {
        return Err(Error::Blob("empty key".into()));
    }
    if key.starts_with('/') || key.contains('\\') {
        return Err(Error::Blob(format!("invalid key: {key:?}")));
    }
    let p = Path::new(key);
    for c in p.components() {
        match c {
            Component::Normal(_) => {}
            // RootDir / CurDir / ParentDir / Prefix are all rejected.
            _ => return Err(Error::Blob(format!("invalid key: {key:?}"))),
        }
    }
    Ok(())
}

fn map_io(e: std::io::Error, ctx: &str) -> Error {
    Error::Blob(format!("{ctx}: {e}"))
}

impl BlobStore for FsBlobStore {
    fn put(&self, key: &str, bytes: &[u8]) -> impl Future<Output = Result<()>> + Send {
        // Capture by value before crossing await boundaries.
        let path = self.resolve(key);
        let bytes = bytes.to_vec();
        async move {
            let path = path?;
            if fs::try_exists(&path)
                .await
                .map_err(|e| map_io(e, "stat"))?
            {
                return Err(Error::Blob(format!(
                    "already_exists: {}",
                    path.display()
                )));
            }
            if let Some(parent) = path.parent() {
                fs::create_dir_all(parent)
                    .await
                    .map_err(|e| map_io(e, "create dir"))?;
            }
            // Random-suffixed tmp file to avoid clashes with concurrent puts on
            // unrelated keys that share the parent dir.
            let tmp = path.with_extension(format!(
                "tmp.{}.{}",
                std::process::id(),
                rand_suffix()
            ));
            {
                let mut f = fs::File::create(&tmp)
                    .await
                    .map_err(|e| map_io(e, "create tmp"))?;
                f.write_all(&bytes).await.map_err(|e| map_io(e, "write"))?;
                f.sync_all().await.map_err(|e| map_io(e, "fsync"))?;
            }
            // rename overwrites on POSIX; the earlier exists-check makes this
            // a no-clobber operation in practice (single bot, single worker).
            fs::rename(&tmp, &path)
                .await
                .map_err(|e| map_io(e, "rename"))?;
            Ok(())
        }
    }

    fn get(&self, key: &str) -> impl Future<Output = Result<Option<Vec<u8>>>> + Send {
        let path = self.resolve(key);
        async move {
            let path = path?;
            match fs::read(&path).await {
                Ok(b) => Ok(Some(b)),
                Err(e) if e.kind() == ErrorKind::NotFound => Ok(None),
                Err(e) => Err(map_io(e, "read")),
            }
        }
    }

    fn delete(&self, key: &str) -> impl Future<Output = Result<()>> + Send {
        let path = self.resolve(key);
        async move {
            let path = path?;
            match fs::remove_file(&path).await {
                Ok(()) => Ok(()),
                Err(e) if e.kind() == ErrorKind::NotFound => Ok(()),
                Err(e) => Err(map_io(e, "remove")),
            }
        }
    }

    fn head(&self, key: &str) -> impl Future<Output = Result<Option<BlobMeta>>> + Send {
        let path = self.resolve(key);
        async move {
            let path = path?;
            let meta = match fs::metadata(&path).await {
                Ok(m) => m,
                Err(e) if e.kind() == ErrorKind::NotFound => return Ok(None),
                Err(e) => return Err(map_io(e, "stat")),
            };
            // ETag is sha256 over current file contents. Cheap for short
            // audio blobs; if this becomes hot, cache to xattr.
            let bytes = fs::read(&path).await.map_err(|e| map_io(e, "read"))?;
            let mut h = Sha256::new();
            h.update(&bytes);
            let etag = hex::encode(h.finalize());
            Ok(Some(BlobMeta {
                size: meta.len(),
                etag,
            }))
        }
    }
}

/// Cheap unique-ish suffix for tmp files. Not cryptographic.
fn rand_suffix() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.subsec_nanos())
        .unwrap_or(0);
    format!("{nanos:08x}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::runtime::Runtime;

    fn rt() -> Runtime {
        Runtime::new().unwrap()
    }

    fn tmp_root() -> PathBuf {
        let mut p = std::env::temp_dir();
        p.push(format!(
            "voice2text-fsblob-{}-{}",
            std::process::id(),
            rand_suffix()
        ));
        p
    }

    #[test]
    fn key_validation() {
        assert!(validate_key("audio/2026/05/07/abc.oga").is_ok());
        assert!(validate_key("a").is_ok());
        assert!(validate_key("").is_err());
        assert!(validate_key("/abs").is_err());
        assert!(validate_key("a/../b").is_err());
        assert!(validate_key("a\\b").is_err());
        assert!(validate_key("./x").is_err());
    }

    #[test]
    fn put_get_delete_head() {
        let rt = rt();
        let root = tmp_root();
        rt.block_on(async {
            let store = FsBlobStore::open(&root).unwrap();
            let key = "audio/2026/05/07/abc.oga";

            assert!(store.get(key).await.unwrap().is_none());
            assert!(store.head(key).await.unwrap().is_none());

            store.put(key, b"hello").await.unwrap();
            let got = store.get(key).await.unwrap().unwrap();
            assert_eq!(got, b"hello");

            let meta = store.head(key).await.unwrap().unwrap();
            assert_eq!(meta.size, 5);
            assert_eq!(meta.etag.len(), 64); // sha256 hex

            // put twice -> AlreadyExists.
            let err = store.put(key, b"again").await.unwrap_err();
            assert!(matches!(err, Error::Blob(ref m) if m.starts_with("already_exists:")));

            store.delete(key).await.unwrap();
            assert!(store.get(key).await.unwrap().is_none());
            // delete missing -> ok.
            store.delete(key).await.unwrap();
        });
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn rejects_traversal() {
        let rt = rt();
        let root = tmp_root();
        rt.block_on(async {
            let store = FsBlobStore::open(&root).unwrap();
            assert!(store.put("../escape", b"x").await.is_err());
            assert!(store.get("/abs").await.is_err());
        });
        let _ = std::fs::remove_dir_all(&root);
    }
}
