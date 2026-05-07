use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::{Error, Result};

#[derive(Debug, Clone)]
pub struct Catalog {
    entries: Vec<CatalogEntry>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct CatalogEntry {
    pub name: String,
    pub family: ModelFamily,
    pub filename: String,
    pub url: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sha256: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub size_bytes: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default)]
    pub is_directory: bool,
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum ModelFamily {
    Whisper,
    Parakeet,
    GigaAm,
}


#[derive(Debug, Deserialize, Serialize)]
struct CatalogFile {
    #[serde(rename = "model", default)]
    models: Vec<CatalogEntry>,
}

const EMBEDDED: &str = include_str!("../../config/models.toml");
const CATALOG_PATH: &str = "config/models.toml";

impl Catalog {
    /// Load catalog: embedded entries as base, merged with the external file
    /// at `config/models.toml` if it exists. External entries override
    /// embedded ones by name.
    pub fn load() -> Result<Self> {
        let mut entries = Self::parse_toml(EMBEDDED, "embedded catalog")?;

        let external_path = Path::new(CATALOG_PATH);
        if external_path.is_file() {
            let content = std::fs::read_to_string(&external_path)
                .map_err(|e| Error::Config(format!("read {}: {e}", external_path.display())))?;
            let external = Self::parse_toml(&content, &external_path.display().to_string())?;

            for ext in external {
                if let Some(pos) = entries.iter().position(|e| e.name == ext.name) {
                    entries[pos] = ext;
                } else {
                    entries.push(ext);
                }
            }
        }

        Ok(Self { entries })
    }

    fn parse_toml(src: &str, label: &str) -> Result<Vec<CatalogEntry>> {
        let parsed: CatalogFile =
            toml::from_str(src).map_err(|e| Error::Config(format!("{label}: {e}")))?;
        Ok(parsed.models)
    }

    pub fn entries(&self) -> &[CatalogEntry] {
        &self.entries
    }

    pub fn find(&self, name: &str) -> Option<&CatalogEntry> {
        self.entries.iter().find(|e| e.name == name)
    }

    /// Append (or replace by name) an entry in the external catalog file.
    /// Creates the file if it does not exist yet.
    pub fn append_to_file(entry: &CatalogEntry) -> Result<()> {
        let path = Path::new(CATALOG_PATH);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| Error::Config(format!("mkdir {}: {e}", parent.display())))?;
        }

        let mut external = if path.is_file() {
            let content = std::fs::read_to_string(&path)
                .map_err(|e| Error::Config(format!("read {}: {e}", path.display())))?;
            Self::parse_toml(&content, &path.display().to_string())?
        } else {
            Vec::new()
        };

        if let Some(pos) = external.iter().position(|e| e.name == entry.name) {
            external[pos] = entry.clone();
        } else {
            external.push(entry.clone());
        }

        let file = CatalogFile { models: external };
        let content = toml::to_string_pretty(&file)
            .map_err(|e| Error::Config(format!("serialize catalog: {e}")))?;
        std::fs::write(&path, content)
            .map_err(|e| Error::Config(format!("write {}: {e}", path.display())))?;

        Ok(())
    }
}
