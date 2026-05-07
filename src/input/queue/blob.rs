//! Blob storage: backend-agnostic byte KV with `blob_key` addressing.
//!
//! Mirrors `docs/contract.md` section 5. Implementations live under
//! `backends/`. Currently only the local filesystem backend is provided;
//! a remote backend may be added later.
//!
//! Key format: `audio/{YYYY}/{MM}/{DD}/{ulid}.{ext}`. The trait does not
//! parse keys -- it treats them as opaque strings -- but backends MUST NOT
//! resolve a key into anything outside their configured root/bucket
//! (path traversal protection lives in the backend).
//!
//! Semantics:
//! - `put` is atomic and fails on collision (enforces immutability of a
//!   `blob_key` once written; collisions are a bug, not a normal case).
//! - `get` returns `Ok(None)` on missing key, never an error.
//! - `delete` is idempotent: missing key is `Ok(())`.
//! - `head` returns metadata or `Ok(None)` on missing key.

use std::future::Future;

use crate::{
    input::{AudioSource, RawAudio},
    Error, Result,
};

/// Object metadata returned by `head`.
#[derive(Debug, Clone)]
pub struct BlobMeta {
    pub size: u64,
    /// Backend-specific entity tag. Filesystem backend uses sha256-hex;
    /// a remote backend would typically forward whatever native tag it has.
    pub etag: String,
}

/// Backend-agnostic blob store.
pub trait BlobStore: Send + Sync {
    /// Write bytes under `key`. Fails if the key already exists.
    /// On `AlreadyExists` the returned error is `Error::Blob` with a message
    /// starting with the prefix `"already_exists:"` (see [`is_already_exists`]).
    fn put(&self, key: &str, bytes: &[u8]) -> impl Future<Output = Result<()>> + Send;

    /// Read all bytes for `key`. Returns `Ok(None)` if missing.
    fn get(&self, key: &str) -> impl Future<Output = Result<Option<Vec<u8>>>> + Send;

    /// Delete `key`. Missing key is not an error.
    fn delete(&self, key: &str) -> impl Future<Output = Result<()>> + Send;

    /// Stat `key`. Returns `Ok(None)` if missing.
    fn head(&self, key: &str) -> impl Future<Output = Result<Option<BlobMeta>>> + Send;
}

/// Detect whether an error returned by `BlobStore::put` represents an
/// `AlreadyExists` collision (vs a real I/O error).
pub fn is_already_exists(err: &Error) -> bool {
    matches!(err, Error::Blob(msg) if msg.starts_with("already_exists:"))
}

// --- Adapter to AudioSource --------------------------------------------------

/// In-memory `AudioSource` backed by bytes fetched from a `BlobStore`.
///
/// Bridges the synchronous decode pipeline (`AudioSource::read` is sync) and
/// the async blob layer: bytes are fetched once via `fetch`, then handed out
/// synchronously on `read`.
///
/// `format_hint` is what the decoder sees; populate it from the job hints
/// (`hints.mime`) or by parsing the extension off `blob_key`.
pub struct BlobAudioSource {
    name: String,
    bytes: Vec<u8>,
    format_hint: Option<String>,
}

impl BlobAudioSource {
    /// Fetch the blob now and wrap it as an `AudioSource`.
    ///
    /// Returns `Error::Blob("not_found: ...")` if the key is missing.
    pub async fn fetch<B: BlobStore + ?Sized>(
        store: &B,
        key: &str,
        format_hint: Option<String>,
    ) -> Result<Self> {
        let bytes = store
            .get(key)
            .await?
            .ok_or_else(|| Error::Blob(format!("not_found: {key}")))?;
        Ok(Self {
            name: key.to_string(),
            bytes,
            format_hint: format_hint.or_else(|| extension_of(key)),
        })
    }

    /// Construct from already-loaded bytes (tests, fast paths).
    pub fn from_bytes(name: impl Into<String>, bytes: Vec<u8>, format_hint: Option<String>) -> Self {
        Self {
            name: name.into(),
            bytes,
            format_hint,
        }
    }
}

impl AudioSource for BlobAudioSource {
    fn name(&self) -> &str {
        &self.name
    }

    fn read(&self) -> Result<RawAudio> {
        Ok(RawAudio {
            bytes: self.bytes.clone(),
            format_hint: self.format_hint.clone(),
        })
    }
}

/// Detect whether an error from `BlobAudioSource::fetch` (or `BlobStore::get`
/// callers using the same convention) is a missing-key error.
pub fn is_not_found(err: &Error) -> bool {
    matches!(err, Error::Blob(msg) if msg.starts_with("not_found:"))
}

fn extension_of(key: &str) -> Option<String> {
    key.rsplit('.').next().and_then(|ext| {
        if ext.is_empty() || ext.contains('/') {
            None
        } else {
            Some(ext.to_ascii_lowercase())
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extension_extraction() {
        assert_eq!(extension_of("audio/2026/05/07/abc.oga"), Some("oga".into()));
        assert_eq!(extension_of("audio/2026/05/07/abc"), None);
        assert_eq!(extension_of("audio/2026/05/07/abc."), None);
    }

    #[test]
    fn from_bytes_reads_back() {
        let s = BlobAudioSource::from_bytes("test", vec![1, 2, 3], Some("oga".into()));
        assert_eq!(s.name(), "test");
        let raw = s.read().unwrap();
        assert_eq!(raw.bytes, vec![1, 2, 3]);
        assert_eq!(raw.format_hint.as_deref(), Some("oga"));
    }

    #[test]
    fn error_classification() {
        assert!(is_not_found(&Error::Blob("not_found: x".into())));
        assert!(!is_not_found(&Error::Blob("other".into())));
        assert!(is_already_exists(&Error::Blob("already_exists: x".into())));
        assert!(!is_already_exists(&Error::Blob("not_found: x".into())));
    }
}
