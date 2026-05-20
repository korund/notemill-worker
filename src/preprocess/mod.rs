//! Audio preprocessing between decode and transcribe:
//! speech segmentation (VAD) and, later, chunking.

use crate::config::{AudioConfig, Config};
use crate::decode::Pcm16kMono;
use crate::Result;

pub mod chunker;
pub mod vad;

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

/// Produces a list of speech segments for a single decoded audio buffer.
///
/// Implementations may return an empty Vec when no speech is detected; the
/// caller decides whether to fall back to the original PCM.
pub trait SpeechSegmenter {
    fn segment(&mut self, pcm: &Pcm16kMono) -> Result<Vec<Segment>>;
}

/// Build a segmenter from a top-level Config (convenience for one-shot
/// callers like `run file`).
pub fn segmenter_from_config(cfg: &Config) -> Result<Option<Box<dyn SpeechSegmenter>>> {
    segmenter_from_audio(cfg.audio.as_ref())
}

/// Build a segmenter from an optional audio block. Returns `None` when VAD
/// is disabled or the audio block is absent.
pub fn segmenter_from_audio(
    audio: Option<&AudioConfig>,
) -> Result<Option<Box<dyn SpeechSegmenter>>> {
    let Some(audio) = audio else {
        return Ok(None);
    };
    let vc = &audio.preprocess.vad;
    if !vc.enabled {
        return Ok(None);
    }
    let params = vad::SileroParams {
        threshold: vc.threshold,
        min_speech_ms: vc.min_speech_ms,
        min_silence_ms: vc.min_silence_ms,
        speech_pad_ms: vc.speech_pad_ms,
    };
    let path = vc.resolve_model_path();
    let seg = vad::SileroSegmenter::new(path, params)?;
    Ok(Some(Box::new(seg)))
}

/// Build a chunker from a top-level Config.
pub fn chunker_from_config(cfg: &Config) -> Option<Box<dyn chunker::Chunker>> {
    chunker_from_audio(cfg.audio.as_ref())
}

/// Build a chunker from an optional audio block. Returns `None` when
/// chunking is disabled or the audio block is absent (caller should then
/// transcribe the full speech stream as a single chunk).
pub fn chunker_from_audio(audio: Option<&AudioConfig>) -> Option<Box<dyn chunker::Chunker>> {
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
