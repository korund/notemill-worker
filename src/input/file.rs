use std::path::PathBuf;

use crate::{decode::AudioDecoder, engine::Transcriber, output::OutputSink, Error, Result};

use super::{AudioSource, InputDriver, RawAudio};

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

/// One-shot driver: process a single local file and exit. Selected when the
/// user passes `--file <path>`; bypasses the queue/bucket layers entirely so
/// the standalone debugging path stays self-contained.
pub struct FileDriver {
    path: PathBuf,
    decoder: Box<dyn AudioDecoder>,
    transcriber: Box<dyn Transcriber>,
    sink: Box<dyn OutputSink>,
    frontmatter: Option<String>,
}

impl FileDriver {
    pub fn new(
        path: PathBuf,
        decoder: Box<dyn AudioDecoder>,
        transcriber: Box<dyn Transcriber>,
        sink: Box<dyn OutputSink>,
        frontmatter: Option<String>,
    ) -> Self {
        Self {
            path,
            decoder,
            transcriber,
            sink,
            frontmatter,
        }
    }
}

impl InputDriver for FileDriver {
    fn run(&mut self) -> Result<()> {
        let source = LocalFileSource::new(self.path.clone());
        let raw = source.read()?;
        let pcm = self.decoder.decode(&raw)?;
        let text = self.transcriber.transcribe(&pcm)?;
        let body = match &self.frontmatter {
            Some(prefix) => format!("{prefix}{text}"),
            None => text,
        };
        self.sink.write(&body)?;
        Ok(())
    }
}
