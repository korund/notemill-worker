use serde::Deserialize;

use crate::engine::EngineKind;
use crate::{Error, Result};

/// Built-in model catalog. Format: TOML, embedded from
/// `assets/models_catalog.toml` via `include_str!`.
#[derive(Debug, Clone)]
pub struct Catalog {
    entries: Vec<CatalogEntry>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct CatalogEntry {
    pub name: String,
    pub family: ModelFamily,
    pub filename: String,
    pub url: String,
    pub sha256: Option<String>,
    pub size_bytes: Option<u64>,
    pub description: Option<String>,
}

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ModelFamily {
    Whisper,
    Parakeet,
    GigaAm,
}

impl ModelFamily {
    pub fn engine_kind(self) -> EngineKind {
        match self {
            ModelFamily::Whisper => EngineKind::Whisper,
            ModelFamily::Parakeet => EngineKind::Parakeet,
            ModelFamily::GigaAm => EngineKind::GigaAm,
        }
    }
}

#[derive(Debug, Deserialize)]
struct CatalogFile {
    #[serde(rename = "model", default)]
    models: Vec<CatalogEntry>,
}

const EMBEDDED: &str = include_str!("../../assets/models_catalog.toml");

impl Catalog {
    pub fn embedded() -> Result<Self> {
        let parsed: CatalogFile = toml::from_str(EMBEDDED)
            .map_err(|e| Error::Config(format!("models catalog: {e}")))?;
        Ok(Self {
            entries: parsed.models,
        })
    }

    pub fn entries(&self) -> &[CatalogEntry] {
        &self.entries
    }

    pub fn find(&self, name: &str) -> Option<&CatalogEntry> {
        self.entries.iter().find(|e| e.name == name)
    }
}
