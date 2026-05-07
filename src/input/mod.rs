//! Input layer.
//!
//! Two distinct concerns live here:
//!
//! 1. [`InputDriver`] -- top-level "how the application is fed". Each driver
//!    owns its execution model: [`file::FileDriver`] is one-shot (process one
//!    file, exit); `queue::QueueDriver` is a long-running daemon loop
//!    (pop job -> fetch blob -> pipeline -> ack/notify). Selected at startup
//!    from CLI flag or config.
//!
//! 2. [`AudioSource`] -- low-level "give me bytes of one audio". Used inside
//!    the decode -> engine -> output pipeline. Drivers construct sources per
//!    item and feed them through the pipeline.
//!
//! New input modes (HTTP, watch-dir, etc.) plug in by adding a new driver
//! plus, if needed, a new [`AudioSource`] implementation.

mod file;
pub mod queue;

pub use file::{FileDriver, LocalFileSource};

use crate::Result;

/// Top-level input mode. An implementation owns the full lifecycle of the
/// run: constructing sources, driving them through the pipeline, and
/// handling acknowledgements / retries / notifications where applicable.
///
/// `run` consumes the driver via `&mut self` so it can be held behind
/// `Box<dyn InputDriver>` chosen at startup.
pub trait InputDriver {
    /// Execute the driver until completion.
    ///
    /// - One-shot drivers return after processing their single item.
    /// - Daemon drivers return only on graceful shutdown (SIGTERM) or a
    ///   fatal, non-recoverable error.
    fn run(&mut self) -> Result<()>;
}

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
