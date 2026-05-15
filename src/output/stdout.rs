use std::io::Write;

use super::concat::SeparatorState;
use super::OutputSink;
use crate::{Error, Result};

pub struct StdoutSink {
    state: SeparatorState,
}

impl StdoutSink {
    pub fn new() -> Self {
        Self {
            state: SeparatorState::new(None, false),
        }
    }

    pub fn with_separator(mut self, separator: Option<String>) -> Self {
        self.state = SeparatorState::new(separator, false);
        self
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
        if let Some(p) = self.state.next_prefix() {
            h.write_all(p.as_bytes())
                .map_err(|e| Error::Output(format!("stdout: {e}")))?;
        }
        h.write_all(text.as_bytes())
            .map_err(|e| Error::Output(format!("stdout: {e}")))?;
        if !text.ends_with('\n') {
            h.write_all(b"\n")
                .map_err(|e| Error::Output(format!("stdout: {e}")))?;
        }
        Ok(())
    }
}
