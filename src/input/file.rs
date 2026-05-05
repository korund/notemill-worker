use std::path::PathBuf;

use crate::{Error, Result};

use super::{AudioSource, RawAudio};

pub struct LocalFileSource {
    path: PathBuf,
    display: String,
}

impl LocalFileSource {
    pub fn new(path: PathBuf) -> Self {
        let display = path.display().to_string();
        Self { path, display }
    }
}

impl AudioSource for LocalFileSource {
    fn name(&self) -> &str {
        &self.display
    }

    fn read(&self) -> Result<RawAudio> {
        let bytes = std::fs::read(&self.path)
            .map_err(|e| Error::Input(format!("read {}: {e}", self.path.display())))?;
        let format_hint = self
            .path
            .extension()
            .and_then(|s| s.to_str())
            .map(|s| s.to_ascii_lowercase());
        Ok(RawAudio { bytes, format_hint })
    }
}
