use std::io::Write;

use super::OutputSink;
use crate::{Error, Result};

pub struct StdoutSink;

impl StdoutSink {
    pub fn new() -> Self {
        Self
    }
}

impl Default for StdoutSink {
    fn default() -> Self {
        Self::new()
    }
}

impl OutputSink for StdoutSink {
    fn write(&mut self, text: &str) -> Result<()> {
        let stdout = std::io::stdout();
        let mut h = stdout.lock();
        h.write_all(text.as_bytes())
            .map_err(|e| Error::Output(format!("stdout: {e}")))?;
        if !text.ends_with('\n') {
            h.write_all(b"\n")
                .map_err(|e| Error::Output(format!("stdout: {e}")))?;
        }
        Ok(())
    }
}
