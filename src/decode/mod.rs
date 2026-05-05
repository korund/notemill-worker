//! Decodes arbitrary audio to PCM 16 kHz mono f32 -- the format expected by the engine.
//!
//! Implementation plan: pure-Rust via `symphonia` (containers/codecs) + `rubato` (resampling).
//! This satisfies the "no external system dependencies" requirement:
//! everything links into the binary, no system `ffmpeg` needed.
//! Alternative: `ffmpeg-next` (libav* vendored into the image) if symphonia
//! does not cover the required formats.
//!
//! Current state: trait + stub. Real implementation is enabled via feature `decode-symphonia`.

use crate::input::RawAudio;
use crate::{Error, Result};

/// Target sample rate expected by whisper.cpp and compatible engines.
pub const TARGET_SAMPLE_RATE: u32 = 16_000;

/// PCM normalized to 16 kHz mono f32 in the range [-1.0, 1.0].
pub struct Pcm16kMono {
    pub samples: Vec<f32>,
}

pub trait AudioDecoder {
    fn decode(&self, raw: &RawAudio) -> Result<Pcm16kMono>;
}

/// Default decoder. Currently a stub; real implementation is enabled via feature `decode-symphonia`.
pub struct DefaultDecoder {
    _private: (),
}

impl DefaultDecoder {
    pub fn new() -> Self {
        Self { _private: () }
    }
}

impl Default for DefaultDecoder {
    fn default() -> Self {
        Self::new()
    }
}

impl AudioDecoder for DefaultDecoder {
    #[cfg(not(feature = "decode-symphonia"))]
    fn decode(&self, _raw: &RawAudio) -> Result<Pcm16kMono> {
        Err(Error::NotImplemented(
            "decode: enable feature `decode-symphonia` to provide a real decoder",
        ))
    }

    #[cfg(feature = "decode-symphonia")]
    fn decode(&self, _raw: &RawAudio) -> Result<Pcm16kMono> {
        // TODO: symphonia probe -> decode -> mixdown to mono -> rubato resample -> 16 kHz f32.
        Err(Error::Decode(
            "symphonia-based decoder is wired up but not yet implemented".into(),
        ))
    }
}
