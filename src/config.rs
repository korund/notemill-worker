//! Application configuration loaded from a YAML file (default: config/config.yaml).

use std::path::{Path, PathBuf};

use serde::Deserialize;

use crate::{Error, Result};

#[derive(Debug, Deserialize)]
pub struct Config {
    #[serde(default)]
    pub model: Option<ModelConfig>,
    pub output: OutputConfig,
    #[serde(default)]
    pub input: Option<InputConfig>,
    #[serde(default)]
    pub ffmpeg: Option<FfmpegConfig>,
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct ModelConfig {
    /// Model name from catalog or path to model file.
    #[serde(default)]
    pub name: Option<String>,
    /// Engine family. Required only when name is a direct path.
    #[serde(default)]
    pub family: Option<String>,
    /// Models directory. Default: ./models.
    #[serde(default)]
    pub dir: Option<PathBuf>,
}

#[derive(Debug, Deserialize)]
pub struct InputConfig {
    /// Currently only "queue" is supported. Reserved for future drivers
    /// (http, watch_dir, ...). Selects which per-driver block below is used.
    pub driver: String,
    #[serde(default)]
    pub queue: Option<QueueConfig>,
}

#[derive(Debug, Deserialize)]
pub struct QueueConfig {
    /// "sqlite" (local). Remote backends are out of scope for this build.
    pub backend: String,
    #[serde(default)]
    pub sqlite: Option<SqliteQueueConfig>,
    #[serde(default = "default_visibility_sec")]
    pub visibility_timeout_sec: u32,
    #[serde(default = "default_max_receive")]
    pub max_receive: u32,
    #[serde(default = "default_poll_ms")]
    pub poll_interval_ms: u64,

    /// Storage for the actual audio bytes the queue points to (claim-check).
    /// Specific to the queue driver; other drivers carry payload differently.
    #[serde(default)]
    pub bucket: Option<BucketConfig>,
}

#[derive(Debug, Deserialize)]
pub struct SqliteQueueConfig {
    pub path: PathBuf,
}

#[derive(Debug, Deserialize)]
pub struct BucketConfig {
    /// "fs" (local). Remote backends are out of scope for this build.
    pub backend: String,
    #[serde(default)]
    pub fs: Option<FsBucketConfig>,
}

#[derive(Debug, Deserialize)]
pub struct FsBucketConfig {
    pub root: PathBuf,
}

fn default_visibility_sec() -> u32 {
    300
}
fn default_max_receive() -> u32 {
    5
}
fn default_poll_ms() -> u64 {
    1000
}

#[derive(Debug, Deserialize)]
pub struct OutputConfig {
    /// Active sink. Accepted: "stdout", "file", "couchdb".
    /// Selects which per-sink config block below is actually used.
    /// CLI `--output` overrides this.
    #[serde(default)]
    pub sink: Option<String>,

    #[serde(default)]
    pub couchdb: Option<CouchdbConfig>,
    #[serde(default)]
    pub file: Option<FileSinkConfig>,
    #[serde(default)]
    pub stdout: Option<StdoutSinkConfig>,

    /// How to derive the note name when writing to a sink that uses a path
    /// per note (currently: couchdb). Default: MessageId.
    #[serde(default)]
    pub name: NamingConfig,
}

/// Strategy for deriving the note file name.
#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum NamingConfig {
    /// `tg-{chat_id}-{message_id}` -- the original behaviour.
    MessageId,
    /// Format `received_at` (RFC3339 UTC) with a chrono strftime string.
    /// On collision a `-1`, `-2`, ... suffix is appended before the extension.
    Datetime { format: String },
}

impl Default for NamingConfig {
    fn default() -> Self {
        NamingConfig::MessageId
    }
}

impl NamingConfig {
    /// Validate that the naming config is self-consistent.
    /// For Datetime, rejects format strings containing characters that are
    /// forbidden in file names on common platforms (Windows + Unix).
    pub fn validate(&self) -> crate::Result<()> {
        if let NamingConfig::Datetime { format } = self {
            // Characters forbidden in Windows file names (also covers Unix '/')
            const FORBIDDEN: &[char] = &['\\', '/', ':', '*', '?', '"', '<', '>', '|', '\0'];
            if let Some(bad) = format.chars().find(|c| FORBIDDEN.contains(c)) {
                return Err(crate::Error::Config(format!(
                    "output.name.format contains forbidden file-name character {:?}; \
                     use chrono strftime tokens that produce safe characters only \
                     (e.g. use %H-%M-%S instead of %H:%M:%S)",
                    bad
                )));
            }
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct StdoutSinkConfig {
    /// Marker written between consecutive chunks as `\n<separator>\n`.
    /// Default: "---". Set to `null` to disable.
    #[serde(default = "default_stdout_separator")]
    pub separator: Option<String>,
}

fn default_stdout_separator() -> Option<String> {
    Some("---".into())
}

fn default_couchdb_target() -> String {
    "Inbox".into()
}

#[derive(Debug, Clone, Deserialize)]
pub struct CouchdbConfig {
    pub url: String,
    pub database: String,
    pub username: String,

    #[serde(default)]
    pub password_env: Option<String>,
    #[serde(default)]
    pub password_file: Option<PathBuf>,

    /// Doc-path prefix; each queue job writes to `<target>/<safe dedup_key>.md`.
    /// CLI `--target` overrides this.
    #[serde(default = "default_couchdb_target")]
    pub target: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct FileSinkConfig {
    /// Filesystem path of the output file. CLI `--target` overrides this.
    pub path: PathBuf,
    /// Truncate the file on open instead of appending. Default: false.
    /// CLI `--overwrite` forces true.
    #[serde(default)]
    pub overwrite: bool,
    /// Marker written between consecutive chunks as `\n<separator>\n`.
    /// Default: "---" (markdown horizontal rule). Set to `null` to disable.
    #[serde(default = "default_file_separator")]
    pub separator: Option<String>,
}

fn default_file_separator() -> Option<String> {
    Some("---".into())
}

impl Config {
    pub fn load(path: &Path) -> Result<Self> {
        Self::load_merged(path, &[])
    }

    /// Load config from YAML, then apply --set key=value overrides (dotted keys, YAML scalars).
    pub fn load_merged(path: &Path, overrides: &[(String, String)]) -> Result<Self> {
        let raw = std::fs::read_to_string(path).map_err(|e| {
            if e.kind() == std::io::ErrorKind::NotFound {
                Error::Config(format!(
                    "{} not found; copy config/config.example.yaml to {} and fill it in",
                    path.display(),
                    path.display()
                ))
            } else {
                Error::Config(format!("read {}: {e}", path.display()))
            }
        })?;
        let mut value: serde_yaml::Value = serde_yaml::from_str(&raw)
            .map_err(|e| Error::Config(format!("parse {}: {e}", path.display())))?;
        for (key, val_str) in overrides {
            set_dotted_key(&mut value, key, val_str)?;
        }
        serde_yaml::from_value(value)
            .map_err(|e| Error::Config(format!("parse {}: {e}", path.display())))
    }
}

fn set_dotted_key(root: &mut serde_yaml::Value, key: &str, val_str: &str) -> Result<()> {
    let val: serde_yaml::Value = serde_yaml::from_str(val_str)
        .map_err(|e| Error::Config(format!("--set {key}={val_str}: {e}")))?;
    let parts: Vec<&str> = key.split('.').collect();
    let mut cur = root;
    for part in &parts[..parts.len() - 1] {
        if !cur.is_mapping() {
            *cur = serde_yaml::Value::Mapping(serde_yaml::Mapping::new());
        }
        let k = serde_yaml::Value::String((*part).to_string());
        cur = cur
            .as_mapping_mut()
            .unwrap()
            .entry(k)
            .or_insert_with(|| serde_yaml::Value::Mapping(serde_yaml::Mapping::new()));
    }
    let last = serde_yaml::Value::String(parts.last().unwrap().to_string());
    if let Some(m) = cur.as_mapping_mut() {
        m.insert(last, val);
    }
    Ok(())
}

impl CouchdbConfig {
    /// Resolve the password using priority: file -> env.
    /// Returns None only if both sources are unset or empty.
    pub fn resolve_password(&self) -> Result<Option<String>> {
        if let Some(path) = self.password_file.as_ref() {
            if !path.as_os_str().is_empty() {
                match std::fs::read_to_string(path) {
                    Ok(s) => {
                        let trimmed = s.trim_end_matches(['\n', '\r']).to_string();
                        if !trimmed.is_empty() {
                            return Ok(Some(trimmed));
                        }
                    }
                    Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                    Err(e) => {
                        return Err(Error::Config(format!(
                            "read password_file {}: {e}",
                            path.display()
                        )));
                    }
                }
            }
        }
        if let Some(name) = self.password_env.as_deref().filter(|s| !s.is_empty()) {
            if let Ok(val) = std::env::var(name) {
                if !val.is_empty() {
                    return Ok(Some(val));
                }
            }
        }
        Ok(None)
    }
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct FfmpegConfig {
    /// FFmpeg log level: quiet|panic|fatal|error|warning|info|verbose|debug|trace.
    /// Default: error (silences common ogg/opus warnings on Telegram voice notes).
    #[serde(default)]
    pub log_level: Option<String>,
}

impl Config {
    /// Apply globals derived from config (FFmpeg log level, etc).
    /// Call once at startup after Config::load_merged.
    pub fn apply_globals(&self) {
        let level = self
            .ffmpeg
            .as_ref()
            .and_then(|f| f.log_level.as_deref())
            .unwrap_or("error");
        crate::decode::set_ffmpeg_log_level(level);
    }
}
