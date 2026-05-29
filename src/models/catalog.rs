use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::{Error, Result};

#[derive(Debug, Clone)]
pub struct Catalog {
    transcribe: Vec<CatalogEntry>,
    vad: Vec<CatalogEntry>,
}

/// A single entry in the model catalog.
///
/// `family` is `Some` for transcription models; `None` for VAD models
/// (which belong to the `[[model.vad]]` section and are role-identified
/// by that section, not by a family tag).
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct CatalogEntry {
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub family: Option<ModelFamily>,
    pub filename: String,
    pub url: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sha256: Option<String>,
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

/// Sectioned catalog file: `[[model.transcribe]]` and `[[model.vad]]`.
/// The external override file (if present) uses the same layout.
#[derive(Debug, Deserialize, Serialize, Default)]
struct ModelSections {
    #[serde(default)]
    transcribe: Vec<CatalogEntry>,
    #[serde(default)]
    vad: Vec<CatalogEntry>,
}

#[derive(Debug, Deserialize, Serialize)]
struct CatalogFile {
    #[serde(rename = "model")]
    model: ModelSections,
}

const EMBEDDED: &str = include_str!("../../config/models.toml");
const CATALOG_PATH: &str = "config/models.toml";

impl Catalog {
    /// Load catalog: embedded entries as base, merged with the external file
    /// at `config/models.toml` if it exists. External entries override
    /// embedded ones by name within each section.
    pub fn load() -> Result<Self> {
        let (mut transcribe, mut vad) = Self::parse_toml(EMBEDDED, "embedded catalog")?;

        let external_path = Path::new(CATALOG_PATH);
        if external_path.is_file() {
            let content = std::fs::read_to_string(&external_path)
                .map_err(|e| Error::Config(format!("read {}: {e}", external_path.display())))?;
            let (ext_t, ext_v) =
                Self::parse_toml(&content, &external_path.display().to_string())?;

            for ext in ext_t {
                if let Some(pos) = transcribe.iter().position(|e| e.name == ext.name) {
                    transcribe[pos] = ext;
                } else {
                    transcribe.push(ext);
                }
            }
            for ext in ext_v {
                if let Some(pos) = vad.iter().position(|e| e.name == ext.name) {
                    vad[pos] = ext;
                } else {
                    vad.push(ext);
                }
            }
        }

        Ok(Self { transcribe, vad })
    }

    fn parse_toml(src: &str, label: &str) -> Result<(Vec<CatalogEntry>, Vec<CatalogEntry>)> {
        let parsed: CatalogFile =
            toml::from_str(src).map_err(|e| Error::Config(format!("{label}: {e}")))?;
        Ok((parsed.model.transcribe, parsed.model.vad))
    }

    /// Public alias for use in unit tests within sibling modules.
    #[cfg(test)]
    pub fn parse_toml_pub(
        src: &str,
        label: &str,
    ) -> Result<(Vec<CatalogEntry>, Vec<CatalogEntry>)> {
        Self::parse_toml(src, label)
    }

    /// Construct a Catalog directly from already-parsed section vecs.
    /// Used in tests to avoid touching the filesystem.
    #[cfg(test)]
    pub fn from_parts(transcribe: Vec<CatalogEntry>, vad: Vec<CatalogEntry>) -> Self {
        Self { transcribe, vad }
    }

    /// All entries (transcribe + vad) in one flat slice.
    pub fn entries(&self) -> impl Iterator<Item = &CatalogEntry> {
        self.transcribe.iter().chain(self.vad.iter())
    }

    /// Transcription-section entries only.
    pub fn transcribe_entries(&self) -> &[CatalogEntry] {
        &self.transcribe
    }

    /// VAD-section entries only.
    pub fn vad_entries(&self) -> &[CatalogEntry] {
        &self.vad
    }

    /// Look up a transcription model by name.
    pub fn find(&self, name: &str) -> Option<&CatalogEntry> {
        self.transcribe.iter().find(|e| e.name == name)
    }

    /// Look up a VAD model by name.
    pub fn find_vad(&self, name: &str) -> Option<&CatalogEntry> {
        self.vad.iter().find(|e| e.name == name)
    }

    /// Append (or replace by name) a transcription entry in the external
    /// catalog file.  Creates the file if it does not exist yet.
    pub fn append_to_file(entry: &CatalogEntry) -> Result<()> {
        let path = Path::new(CATALOG_PATH);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| Error::Config(format!("mkdir {}: {e}", parent.display())))?;
        }

        let mut sections = if path.is_file() {
            let content = std::fs::read_to_string(&path)
                .map_err(|e| Error::Config(format!("read {}: {e}", path.display())))?;
            let (t, v) = Self::parse_toml(&content, &path.display().to_string())?;
            ModelSections { transcribe: t, vad: v }
        } else {
            ModelSections::default()
        };

        if let Some(pos) = sections.transcribe.iter().position(|e| e.name == entry.name) {
            sections.transcribe[pos] = entry.clone();
        } else {
            sections.transcribe.push(entry.clone());
        }

        let file = CatalogFile { model: sections };
        let content = toml::to_string_pretty(&file)
            .map_err(|e| Error::Config(format!("serialize catalog: {e}")))?;
        std::fs::write(&path, content)
            .map_err(|e| Error::Config(format!("write {}: {e}", path.display())))?;

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"
[model]
[[model.transcribe]]
name = "whisper-medium"
family = "whisper"
filename = "whisper-medium-q4_1.bin"
url = "https://example.com/whisper.bin"
is_directory = false

[[model.vad]]
name = "silero-vad-v6"
filename = "silero_vad.onnx"
url = "https://example.com/silero_vad.onnx"
sha256 = "abc123"
is_directory = false
"#;

    #[test]
    fn catalog_parses_both_sections() {
        let (transcribe, vad) = Catalog::parse_toml(SAMPLE, "test").unwrap();
        assert_eq!(transcribe.len(), 1);
        assert_eq!(transcribe[0].name, "whisper-medium");
        assert!(transcribe[0].family.is_some());
        assert_eq!(vad.len(), 1);
        assert_eq!(vad[0].name, "silero-vad-v6");
        assert!(vad[0].family.is_none());
    }

    #[test]
    fn catalog_find_transcribe_and_vad() {
        let (transcribe, vad) = Catalog::parse_toml(SAMPLE, "test").unwrap();
        let cat = Catalog { transcribe, vad };
        assert!(cat.find("whisper-medium").is_some());
        assert!(cat.find("silero-vad-v6").is_none()); // not in transcribe section
        assert!(cat.find_vad("silero-vad-v6").is_some());
        assert!(cat.find_vad("whisper-medium").is_none()); // not in vad section
    }
}
