//! Persistent runtime state: detected LiveSync schema cache.
//!
//! File: `.cache/livesync.yaml`. Auto-managed; never edit by hand.

use std::path::Path;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::config::CouchdbConfig;
use crate::{Error, Result};

const STATE_PATH: &str = ".cache/livesync.yaml";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LivesyncState {
    /// sha256 of "url\0database\0username". Used to invalidate cache when
    /// the connection target changes. Password is not part of the fingerprint.
    pub connection_fingerprint: String,
    pub e2ee: bool,
    pub path_obfuscation: bool,
    /// Schema marker, e.g. "livesync-modern-children-eden".
    pub schema: String,
    /// Hash algorithm used for chunk _id, e.g. "xxhash64".
    pub hash_algo: String,
    /// Unix seconds (UTC) when the detection ran.
    pub detected_at_unix: u64,
}

impl LivesyncState {
    /// Load cached state. Returns None if the file is missing or unreadable.
    pub fn load() -> Option<Self> {
        let raw = std::fs::read_to_string(STATE_PATH).ok()?;
        serde_yaml_ng::from_str(&raw).ok()
    }

    /// Atomically save state to `.cache/livesync.yaml`.
    pub fn save(&self) -> Result<()> {
        let path = Path::new(STATE_PATH);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| Error::Output(format!("mkdir {}: {e}", parent.display())))?;
        }
        let raw = serde_yaml_ng::to_string(self)
            .map_err(|e| Error::Output(format!("serialize state: {e}")))?;
        let tmp = path.with_extension("yaml.tmp");
        std::fs::write(&tmp, raw)
            .map_err(|e| Error::Output(format!("write {}: {e}", tmp.display())))?;
        std::fs::rename(&tmp, path).map_err(|e| {
            Error::Output(format!(
                "rename {} -> {}: {e}",
                tmp.display(),
                path.display()
            ))
        })?;
        Ok(())
    }
}

/// Compute connection fingerprint from CouchDB config.
pub fn fingerprint(cfg: &CouchdbConfig) -> String {
    let mut h = Sha256::new();
    h.update(cfg.url.as_bytes());
    h.update(b"\0");
    h.update(cfg.database.as_bytes());
    h.update(b"\0");
    h.update(cfg.username.as_bytes());
    hex::encode(h.finalize())
}

pub fn now_unix_seconds() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}
