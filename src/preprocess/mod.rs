//! Audio preprocessing between decode and transcribe:
//! speech segmentation (VAD) and, later, chunking.

use std::path::PathBuf;

use crate::config::{AudioConfig, Config};
use crate::decode::Pcm16kMono;
use crate::{Error, Result};

pub mod chunker;
pub mod deafness;
pub mod vad;

pub use deafness::Speech;

/// Bundle of preprocess stages that sit between decode and transcribe.
/// Both stages are optional: when both are `None` the pipeline feeds the
/// decoded PCM straight to the transcriber.
pub struct Preprocess {
    pub segmenter: Option<Box<dyn SpeechSegmenter>>,
    pub chunker: Option<Box<dyn chunker::Chunker>>,
}

impl Preprocess {
    pub fn none() -> Self {
        Self {
            segmenter: None,
            chunker: None,
        }
    }

    /// Build from the top-level Config with a pre-resolved VAD model path.
    ///
    /// The caller is responsible for resolving `vad_model_path` via the
    /// `ModelRegistry` before calling this. Pass `None` when VAD is disabled
    /// or the registry is not available.
    pub fn from_config(cfg: &Config, vad_model_path: Option<PathBuf>) -> Result<Self> {
        Self::from_audio(cfg.audio.as_ref(), vad_model_path)
    }

    /// Build from an optional audio block.
    ///
    /// `vad_model_path` must be `Some` when `vad.enabled` is true; if it is
    /// `None` while VAD is enabled, returns `Error::Config`.
    pub fn from_audio(
        audio: Option<&AudioConfig>,
        vad_model_path: Option<PathBuf>,
    ) -> Result<Self> {
        Ok(Self {
            segmenter: segmenter_from_audio(audio, vad_model_path)?,
            chunker: chunker_from_audio(audio),
        })
    }
}

/// A contiguous speech region extracted from the input PCM.
///
/// `start_ms`/`end_ms` are offsets in the original audio. `pcm` holds the
/// sliced (and optionally padded) samples for downstream processing.
#[derive(Debug, Clone)]
pub struct Segment {
    pub start_ms: u32,
    pub end_ms: u32,
    pub pcm: Vec<f32>,
}

/// Classifies a single decoded audio buffer into a `Speech` verdict.
///
/// Implementations bundle "the segments we found" with "what the model
/// heard overall" so the pipeline can distinguish a silent recording
/// (`Speech::None`) from a near-threshold one (`Speech::Faint`) without
/// recomputing diagnostics. See [`deafness`] for the classifier.
pub trait SpeechSegmenter {
    fn segment(&mut self, pcm: &Pcm16kMono) -> Result<Speech>;
}

/// Build a segmenter from an optional audio block. Returns `None` when VAD
/// is disabled or the audio block is absent.
fn segmenter_from_audio(
    audio: Option<&AudioConfig>,
    vad_model_path: Option<PathBuf>,
) -> Result<Option<Box<dyn SpeechSegmenter>>> {
    let Some(audio) = audio else {
        return Ok(None);
    };
    let vc = &audio.preprocess.vad;
    if !vc.enabled {
        return Ok(None);
    }
    let path = vad_model_path.ok_or_else(|| {
        Error::Config("vad enabled but model path not provided".into())
    })?;
    let params = vad::SileroParams {
        threshold: vc.threshold,
        min_speech_ms: vc.min_speech_ms,
        min_silence_ms: vc.min_silence_ms,
        speech_pad_ms: vc.speech_pad_ms,
    };
    let seg = vad::SileroSegmenter::new(path, params)?;
    Ok(Some(Box::new(seg)))
}

/// Build a chunker from an optional audio block. Returns `None` when
/// chunking is disabled or the audio block is absent (caller should then
/// transcribe the full speech stream as a single chunk).
fn chunker_from_audio(audio: Option<&AudioConfig>) -> Option<Box<dyn chunker::Chunker>> {
    let audio = audio?;
    let cc = &audio.preprocess.chunking;
    if !cc.enabled {
        return None;
    }
    let params = chunker::ChunkerParams {
        max_seconds: cc.max_seconds,
        overlap_seconds: cc.overlap_seconds,
    };
    Some(Box::new(chunker::SegmentChunker::new(params)))
}
