use crate::input::RawAudio;
use crate::Result;

#[cfg(not(feature = "decode-ffmpeg"))]
use crate::Error;

#[cfg(feature = "decode-ffmpeg")]
mod ffmpeg;

pub const TARGET_SAMPLE_RATE: u32 = 16_000;

pub struct Pcm16kMono {
    pub samples: Vec<f32>,
}

pub trait AudioDecoder {
    fn decode(&self, raw: &RawAudio) -> Result<Pcm16kMono>;
}

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
    #[cfg(not(feature = "decode-ffmpeg"))]
    fn decode(&self, _raw: &RawAudio) -> Result<Pcm16kMono> {
        Err(Error::NotImplemented(
            "decode: enable feature `decode-ffmpeg` to provide a real decoder",
        ))
    }

    #[cfg(feature = "decode-ffmpeg")]
    fn decode(&self, raw: &RawAudio) -> Result<Pcm16kMono> {
        ffmpeg::decode_to_pcm16k(raw)
    }
}
