//! Classify a segmenter run into a Speech verdict.
//!
//! Segmenters return per-window speech probabilities and an assembled
//! list of speech segments. This module turns that pair into one of three
//! outcomes used by the pipeline:
//!
//!   * `Detected` -- at least one speech segment crossed the user
//!     threshold. Normal transcription path.
//!   * `Faint`    -- no segments crossed the threshold, but the peak
//!     per-window probability is above `SILENCE_MAX_PROB`. The model
//!     heard *something* too quiet to clear the bar; the pipeline falls
//!     back to the full PCM rather than declaring silence.
//!   * `None`     -- no segments and peak probability below
//!     `SILENCE_MAX_PROB`. The recording is genuinely silent and the
//!     pipeline should short-circuit with a "no speech" outcome.
//!
//! The threshold is intentionally generous: silero on full silence sits
//! at 0.001..0.005, so 0.05 leaves a wide margin for noise floors while
//! still keeping whisper-territory recordings (>= 0.05) on the fallback
//! path.

use super::Segment;

/// Peak per-window probability below which a segment-less run is
/// classified as `Speech::None` rather than `Speech::Faint`.
pub const SILENCE_MAX_PROB: f32 = 0.05;

/// Verdict for a single segmenter run.
#[derive(Debug)]
pub enum Speech {
    /// One or more speech segments produced by the segmenter.
    Detected(Vec<Segment>),
    /// No segments crossed the threshold, but the model heard something
    /// (peak probability >= `SILENCE_MAX_PROB`).
    Faint,
    /// No segments and peak probability below `SILENCE_MAX_PROB`.
    None,
}

/// Classify segmenter output into a `Speech` verdict.
///
/// `max_prob` is the maximum per-window speech probability observed
/// across the entire input. Callers compute it once and pass it in.
pub fn classify(segments: Vec<Segment>, max_prob: f32) -> Speech {
    if !segments.is_empty() {
        return Speech::Detected(segments);
    }
    if max_prob < SILENCE_MAX_PROB {
        Speech::None
    } else {
        Speech::Faint
    }
}

#[cfg(test)]
mod tests;
