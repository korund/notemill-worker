use std::fs::OpenOptions;
use std::io::Write as IoWrite;
use std::path::PathBuf;

use super::concat::SeparatorState;
use super::OutputSink;
use crate::{Error, Result};

pub struct FileSink {
    path: PathBuf,
    overwrite: bool,
    separator: Option<String>,
    state: Option<SeparatorState>,
}

impl FileSink {
    pub fn new(path: PathBuf) -> Self {
        Self {
            path,
            overwrite: false,
            separator: None,
            state: None,
        }
    }

    pub fn with_overwrite(mut self, overwrite: bool) -> Self {
        self.overwrite = overwrite;
        self
    }

    pub fn with_separator(mut self, separator: Option<String>) -> Self {
        self.separator = separator;
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
        let truncate = self.overwrite && self.state.is_none();
        if self.state.is_none() {
            let initially_primed = if truncate {
                false
            } else {
                std::fs::metadata(&self.path)
                    .map(|m| m.len() > 0)
                    .unwrap_or(false)
            };
            self.state = Some(SeparatorState::new(
                self.separator.clone(),
                initially_primed,
            ));
        }
        let prefix = self.state.as_mut().unwrap().next_prefix();

        if let Some(parent) = self.path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| Error::Output(format!("mkdir {}: {e}", parent.display())))?;
        }
        let mut file = OpenOptions::new()
            .create(true)
            .write(true)
            .append(!truncate)
            .truncate(truncate)
            .open(&self.path)
            .map_err(|e| Error::Output(format!("open {}: {e}", self.path.display())))?;
        if let Some(p) = prefix {
            file.write_all(p.as_bytes())
                .map_err(|e| Error::Output(format!("write {}: {e}", self.path.display())))?;
        }
        file.write_all(text.as_bytes())
            .map_err(|e| Error::Output(format!("write {}: {e}", self.path.display())))?;
        Ok(())
    }
}
