//! Filesystem `Bucket` backend.
//!
//! Maps `audio_key` directly under a configured root directory. Used for the
//! local docker-compose deployment where bot and worker share a volume.
//! Behaviour matches contract section 5.2.

use std::future::Future;
use std::io::ErrorKind;
use std::path::{Component, Path, PathBuf};

use sha2::{Digest, Sha256};
use tokio::fs;
use tokio::io::AsyncWriteExt;

use crate::input::queue::bucket::{Bucket, BucketMeta};
use crate::{Error, Result};

#[derive(Clone)]
pub struct FsBucket {
    root: PathBuf,
}

impl FsBucket {
    /// Bind to a directory. Created if missing.
    pub fn open(root: impl Into<PathBuf>) -> Result<Self> {
        let root = root.into();
        std::fs::create_dir_all(&root)
            .map_err(|e| Error::Bucket(format!("create root {}: {e}", root.display())))?;
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
        return Err(Error::Bucket("empty key".into()));
    }
    if key.starts_with('/') || key.contains('\\') {
        return Err(Error::Bucket(format!("invalid key: {key:?}")));
    }
    let p = Path::new(key);
    for c in p.components() {
        match c {
            Component::Normal(_) => {}
            // RootDir / CurDir / ParentDir / Prefix are all rejected.
            _ => return Err(Error::Bucket(format!("invalid key: {key:?}"))),
        }
    }
    Ok(())
}

fn map_io(e: std::io::Error, ctx: &str) -> Error {
    Error::Bucket(format!("{ctx}: {e}"))
}

impl Bucket for FsBucket {
    fn put(&self, key: &str, bytes: &[u8]) -> impl Future<Output = Result<()>> + Send {
        // Capture by value before crossing await boundaries.
        let path = self.resolve(key);
        let bytes = bytes.to_vec();
        async move {
            let path = path?;
            if fs::try_exists(&path).await.map_err(|e| map_io(e, "stat"))? {
                return Err(Error::Bucket(format!("already_exists: {}", path.display())));
            }
            if let Some(parent) = path.parent() {
                fs::create_dir_all(parent)
                    .await
                    .map_err(|e| map_io(e, "create dir"))?;
            }
            // Random-suffixed tmp file to avoid clashes with concurrent puts on
            // unrelated keys that share the parent dir.
            let tmp = path.with_extension(format!("tmp.{}.{}", std::process::id(), rand_suffix()));
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

    fn head(&self, key: &str) -> impl Future<Output = Result<Option<BucketMeta>>> + Send {
        let path = self.resolve(key);
        async move {
            let path = path?;
            let meta = match fs::metadata(&path).await {
                Ok(m) => m,
                Err(e) if e.kind() == ErrorKind::NotFound => return Ok(None),
                Err(e) => return Err(map_io(e, "stat")),
            };
            // ETag is sha256 over current file contents. Cheap for short
            // audio buckets; if this becomes hot, cache to xattr.
            let bytes = fs::read(&path).await.map_err(|e| map_io(e, "read"))?;
            let mut h = Sha256::new();
            h.update(&bytes);
            let etag = hex::encode(h.finalize());
            Ok(Some(BucketMeta {
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
mod tests;
