use std::fs::OpenOptions;
use std::io::Write as IoWrite;
use std::path::PathBuf;

use super::OutputSink;
use crate::{Error, Result};

pub struct FileSink {
    path: PathBuf,
    overwrite: bool,
    written: bool,
}

impl FileSink {
    pub fn new(path: PathBuf) -> Self {
        Self { path, overwrite: false, written: false }
    }

    pub fn with_overwrite(mut self, overwrite: bool) -> Self {
        self.overwrite = overwrite;
        self
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
        let truncate = self.overwrite && !self.written;
        let mut file = OpenOptions::new()
            .create(true)
            .write(true)
            .append(!truncate)
            .truncate(truncate)
            .open(&self.path)
            .map_err(|e| Error::Output(format!("open {}: {e}", self.path.display())))?;
        file.write_all(text.as_bytes())
            .map_err(|e| Error::Output(format!("write {}: {e}", self.path.display())))?;
        self.written = true;
        Ok(())
    }
}
