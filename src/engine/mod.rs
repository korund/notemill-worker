//! Transcription via `transcribe-rs` (same engine as handy).
//!
//! Supported model families: Whisper (whisper.cpp, GGUF) and
//! Parakeet/GigaAM (ONNX via the same wrapper). All on CPU.
//!
//! Current state: trait + factory stub. Real integration is enabled via feature `engine-transcribe`.

use crate::decode::Pcm16kMono;
use crate::models::ResolvedModel;
use crate::{Error, Result};

pub trait Transcriber {
    fn transcribe(&mut self, pcm: &Pcm16kMono) -> Result<String>;
}

/// Engine family inferred from model metadata or an explicit flag.
#[derive(Debug, Clone, Copy)]
pub enum EngineKind {
    Whisper,
    Parakeet,
    GigaAm,
}

/// Build a concrete engine for the selected model.
pub fn build(model: &ResolvedModel) -> Result<Box<dyn Transcriber>> {
    let kind = model.engine_kind;
    #[cfg(not(feature = "engine-transcribe"))]
    {
        let _ = kind;
        let _ = model;
        Err(Error::NotImplemented(
            "engine: enable feature `engine-transcribe` to use transcribe-rs",
        ))
    }

    #[cfg(feature = "engine-transcribe")]
    {
        let _ = kind;
        let _ = model;
        // TODO: instantiate transcribe_rs::WhisperEngine / ParakeetEngine,
        // load model from model.path, pass params (language, threads, etc.).
        Err(Error::Engine(
            "transcribe-rs adapter is wired up but not yet implemented".into(),
        ))
    }
}
