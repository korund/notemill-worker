//! Application configuration loaded from a YAML file (default: config/config.yaml).

use std::path::{Path, PathBuf};

use serde::Deserialize;

use crate::{Error, Result};

#[derive(Debug, Deserialize)]
pub struct Config {
    pub output: OutputConfig,
}

#[derive(Debug, Deserialize)]
pub struct OutputConfig {
    #[serde(default)]
    pub couchdb: Option<CouchdbConfig>,
}

#[derive(Debug, Deserialize)]
pub struct CouchdbConfig {
    pub url: String,
    pub database: String,
    pub username: String,

    #[serde(default)]
    pub password_env: Option<String>,
    #[serde(default)]
    pub password_file: Option<PathBuf>,
}

impl Config {
    pub fn load(path: &Path) -> Result<Self> {
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
        serde_yaml::from_str(&raw)
            .map_err(|e| Error::Config(format!("parse {}: {e}", path.display())))
    }
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
