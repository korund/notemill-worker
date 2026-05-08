//! Bucket storage: backend-agnostic byte KV with `audio_key` addressing.
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
//!   `audio_key` once written; collisions are a bug, not a normal case).
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
pub struct BucketMeta {
    pub size: u64,
    /// Backend-specific entity tag. Filesystem backend uses sha256-hex;
    /// a remote backend would typically forward whatever native tag it has.
    pub etag: String,
}

/// Backend-agnostic bucket store.
pub trait Bucket: Send + Sync {
    /// Write bytes under `key`. Fails if the key already exists.
    /// On `AlreadyExists` the returned error is `Error::Bucket` with a message
    /// starting with the prefix `"already_exists:"` (see [`is_already_exists`]).
    fn put(&self, key: &str, bytes: &[u8]) -> impl Future<Output = Result<()>> + Send;

    /// Read all bytes for `key`. Returns `Ok(None)` if missing.
    fn get(&self, key: &str) -> impl Future<Output = Result<Option<Vec<u8>>>> + Send;

    /// Delete `key`. Missing key is not an error.
    fn delete(&self, key: &str) -> impl Future<Output = Result<()>> + Send;

    /// Stat `key`. Returns `Ok(None)` if missing.
    fn head(&self, key: &str) -> impl Future<Output = Result<Option<BucketMeta>>> + Send;
}

/// Detect whether an error returned by `Bucket::put` represents an
/// `AlreadyExists` collision (vs a real I/O error).
pub fn is_already_exists(err: &Error) -> bool {
    matches!(err, Error::Bucket(msg) if msg.starts_with("already_exists:"))
}

// --- Adapter to AudioSource --------------------------------------------------

/// In-memory `AudioSource` backed by bytes fetched from a `Bucket`.
///
/// Bridges the synchronous decode pipeline (`AudioSource::read` is sync) and
/// the async bucket layer: bytes are fetched once via `fetch`, then handed out
/// synchronously on `read`.
///
/// `format_hint` is what the decoder sees; populate it from the job hints
/// (`hints.mime`) or by parsing the extension off `audio_key`.
pub struct BucketAudioSource {
    name: String,
    bytes: Vec<u8>,
    format_hint: Option<String>,
}

impl BucketAudioSource {
    /// Fetch the bucket now and wrap it as an `AudioSource`.
    ///
    /// Returns `Error::Bucket("not_found: ...")` if the key is missing.
    pub async fn fetch<B: Bucket + ?Sized>(
        store: &B,
        key: &str,
        format_hint: Option<String>,
    ) -> Result<Self> {
        let bytes = store
            .get(key)
            .await?
            .ok_or_else(|| Error::Bucket(format!("not_found: {key}")))?;
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

impl AudioSource for BucketAudioSource {
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

/// Detect whether an error from `BucketAudioSource::fetch` (or `Bucket::get`
/// callers using the same convention) is a missing-key error.
pub fn is_not_found(err: &Error) -> bool {
    matches!(err, Error::Bucket(msg) if msg.starts_with("not_found:"))
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
        let s = BucketAudioSource::from_bytes("test", vec![1, 2, 3], Some("oga".into()));
        assert_eq!(s.name(), "test");
        let raw = s.read().unwrap();
        assert_eq!(raw.bytes, vec![1, 2, 3]);
        assert_eq!(raw.format_hint.as_deref(), Some("oga"));
    }

    #[test]
    fn error_classification() {
        assert!(is_not_found(&Error::Bucket("not_found: x".into())));
        assert!(!is_not_found(&Error::Bucket("other".into())));
        assert!(is_already_exists(&Error::Bucket("already_exists: x".into())));
        assert!(!is_already_exists(&Error::Bucket("not_found: x".into())));
    }
}
