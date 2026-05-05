use std::path::PathBuf;

use super::OutputSink;
use crate::{Error, Result};

pub struct FileSink {
    path: PathBuf,
}

impl FileSink {
    pub fn new(path: PathBuf) -> Self {
        Self { path }
    }
}

impl OutputSink for FileSink {
    fn write(&mut self, text: &str) -> Result<()> {
        if let Some(parent) = self.path.parent() {
            if !parent.as_os_str().is_empty() {
                std::fs::create_dir_all(parent)
                    .map_err(|e| Error::Output(format!("mkdir {}: {e}", parent.display())))?;
            }
        }
        std::fs::write(&self.path, text)
            .map_err(|e| Error::Output(format!("write {}: {e}", self.path.display())))?;
        Ok(())
    }
}
