//! Input audio sources.
//!
//! The only implementation at this stage is [`LocalFileSource`] (a single local file).
//! Future implementations (directory/queue/HTTP/etc.) plug in via the same trait.

mod file;

pub use file::LocalFileSource;

use crate::Result;

/// Arbitrary audio source. Returns raw bytes in the original format
/// (OGA/WAV/MP3/...). Decoding is handled by the `decode` layer.
pub trait AudioSource {
    /// Human-readable source name (for logging).
    fn name(&self) -> &str;

    /// Read the entire source contents into memory.
    ///
    /// A streaming variant can be added later; for now audio files
    /// fit in RAM and streaming is not required.
    fn read(&self) -> Result<RawAudio>;
}

/// Raw bytes plus an optional format hint (e.g., the file extension).
pub struct RawAudio {
    pub bytes: Vec<u8>,
    pub format_hint: Option<String>,
}
