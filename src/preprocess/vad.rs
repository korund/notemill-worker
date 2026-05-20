//! Silero VAD speech segmenter.
//!
//! Loads silero_vad.onnx (v6.x signature) and runs streaming inference over
//! the decoded PCM, then assembles speech segments by threshold + duration
//! filters.

use std::path::PathBuf;

use ort::session::{builder::GraphOptimizationLevel, Session};
use ort::value::Tensor;
use tracing::{debug, error, info, trace};

use super::{Segment, SpeechSegmenter};
use crate::decode::Pcm16kMono;
use crate::{Error, Result};

const SAMPLE_RATE: u32 = 16_000;
const WINDOW_SAMPLES: usize = 512; // 32 ms at 16 kHz, fixed by silero v6
const CONTEXT_SAMPLES: usize = 64; // 64-sample context prefix (silero v6)
const INPUT_SAMPLES: usize = CONTEXT_SAMPLES + WINDOW_SAMPLES;
const WINDOW_MS: u32 = (WINDOW_SAMPLES as u32 * 1000) / SAMPLE_RATE;
/// Below this peak probability across all windows, we treat the result
/// not as "quiet recording" but as "VAD looks broken" and log an error.
const DEAF_THRESHOLD: f32 = 0.1;

#[derive(Debug, Clone, Copy)]
pub struct SileroParams {
    pub threshold: f32,
    pub min_speech_ms: u32,
    pub min_silence_ms: u32,
    pub speech_pad_ms: u32,
}

impl Default for SileroParams {
    fn default() -> Self {
        Self {
            threshold: 0.5,
            min_speech_ms: 250,
            min_silence_ms: 500,
            speech_pad_ms: 100,
        }
    }
}

pub struct SileroSegmenter {
    session: Session,
    params: SileroParams,
}

impl SileroSegmenter {
    /// Load model from `model_path`. Caller is expected to have called
    /// `ort::init().commit()` earlier so the global env (thread pools,
    /// allocators) is shared with any other ort sessions in the process.
    pub fn new(model_path: PathBuf, params: SileroParams) -> Result<Self> {
        let session = Session::builder()
            .map_err(|e| Error::Engine(format!("silero: session builder: {e}")))?
            .with_optimization_level(GraphOptimizationLevel::Level1)
            .map_err(|e| Error::Engine(format!("silero: opt level: {e}")))?
            .with_intra_threads(1)
            .map_err(|e| Error::Engine(format!("silero: intra threads: {e}")))?
            .commit_from_file(&model_path)
            .map_err(|e| {
                Error::Engine(format!("silero: load {}: {e}", model_path.display()))
            })?;
        info!(model = %model_path.display(), "silero VAD loaded");
        Ok(Self { session, params })
    }

    fn probabilities(&mut self, samples: &[f32]) -> Result<Vec<f32>> {
        let total = (samples.len() + WINDOW_SAMPLES - 1) / WINDOW_SAMPLES;
        let mut probs = Vec::with_capacity(total);
        let mut state = vec![0f32; 2 * 1 * 128];
        // Silero v6 takes a 64-sample context prefix; first chunk's context
        // is zeros, subsequent chunks reuse the tail of the previous window.
        let mut context = vec![0f32; CONTEXT_SAMPLES];
        let mut buf = vec![0f32; INPUT_SAMPLES];

        for w in 0..total {
            let start = w * WINDOW_SAMPLES;
            let end = (start + WINDOW_SAMPLES).min(samples.len());
            let n = end - start;

            buf[..CONTEXT_SAMPLES].copy_from_slice(&context);
            buf[CONTEXT_SAMPLES..CONTEXT_SAMPLES + n].copy_from_slice(&samples[start..end]);
            if n < WINDOW_SAMPLES {
                for x in &mut buf[CONTEXT_SAMPLES + n..] {
                    *x = 0.0;
                }
            }
            // Update context = last CONTEXT_SAMPLES of the just-fed chunk.
            context.copy_from_slice(
                &buf[INPUT_SAMPLES - CONTEXT_SAMPLES..INPUT_SAMPLES],
            );

            let input = Tensor::from_array(([1usize, INPUT_SAMPLES], buf.clone()))
                .map_err(|e| Error::Engine(format!("silero: input tensor: {e}")))?;
            let state_t =
                Tensor::from_array(([2usize, 1usize, 128usize], state.clone()))
                    .map_err(|e| Error::Engine(format!("silero: state tensor: {e}")))?;
            // sr is a scalar int64 (shape []), not a 1-d tensor.
            let sr = Tensor::from_array((vec![] as Vec<usize>, vec![SAMPLE_RATE as i64]))
                .map_err(|e| Error::Engine(format!("silero: sr tensor: {e}")))?;

            let outputs = self
                .session
                .run(ort::inputs![
                    "input" => input,
                    "state" => state_t,
                    "sr" => sr,
                ])
                .map_err(|e| Error::Engine(format!("silero: run: {e}")))?;

            // Outputs by graph order. v6 silero emits two tensors; we
            // identify them by element count rather than assume an order
            // (different exports vary). The probability tensor has 1
            // element; the next-state tensor has 2*1*128 = 256 elements.
            let (ashape, adata) = outputs[0]
                .try_extract_tensor::<f32>()
                .map_err(|e| Error::Engine(format!("silero: extract out0: {e}")))?;
            let (bshape, bdata) = outputs[1]
                .try_extract_tensor::<f32>()
                .map_err(|e| Error::Engine(format!("silero: extract out1: {e}")))?;
            let (prob_data, state_data) = if adata.len() == 1 {
                (adata, bdata)
            } else if bdata.len() == 1 {
                (bdata, adata)
            } else {
                return Err(Error::Engine(format!(
                    "silero: unexpected output shapes a={:?} b={:?}",
                    ashape, bshape
                )));
            };
            probs.push(prob_data[0]);
            state.copy_from_slice(state_data);
        }

        Ok(probs)
    }
}

impl SpeechSegmenter for SileroSegmenter {
    fn segment(&mut self, pcm: &Pcm16kMono) -> Result<Vec<Segment>> {
        if pcm.samples.is_empty() {
            return Ok(Vec::new());
        }
        let probs = self.probabilities(&pcm.samples)?;
        let sample = probs.iter().take(10).copied().collect::<Vec<_>>();
        let max_prob = probs.iter().copied().fold(0.0f32, f32::max);
        trace!(?sample, max_prob, "vad prob sample");
        let segments = assemble_segments(&pcm.samples, &probs, &self.params);
        debug!(
            n_windows = probs.len(),
            n_segments = segments.len(),
            max_prob,
            "silero segmentation done"
        );
        // Distinguish a genuinely-quiet input from a broken VAD: a
        // healthy run on speech tops max_prob > 0.99, on full silence
        // it sits at 0.001..0.005. Anything in between is whisper-
        // territory and should not raise alarm; anything under 0.1 with
        // zero segments means the model itself produced no signal --
        // likely a code/model regression rather than a quiet recording.
        if is_deaf(segments.is_empty(), max_prob) {
            error!(
                max_prob,
                threshold = DEAF_THRESHOLD,
                "silero produced no speech signal at all -- model may be broken"
            );
        }
        Ok(segments)
    }
}

/// Pure function: turn per-window speech probabilities into PCM segments.
///
/// Steps:
///   1. mark windows with prob >= threshold as speech;
///   2. merge runs separated by < min_silence_ms of silence;
///   3. drop runs shorter than min_speech_ms;
///   4. expand each surviving run by speech_pad_ms (clamped).
pub fn assemble_segments(samples: &[f32], probs: &[f32], p: &SileroParams) -> Vec<Segment> {
    if probs.is_empty() {
        return Vec::new();
    }
    let min_speech_windows = ms_to_windows(p.min_speech_ms).max(1);
    let min_silence_windows = ms_to_windows(p.min_silence_ms);
    let pad_samples = (p.speech_pad_ms as usize * SAMPLE_RATE as usize) / 1000;

    let mut runs: Vec<(usize, usize)> = Vec::new();
    let mut i = 0;
    while i < probs.len() {
        if probs[i] >= p.threshold {
            let start = i;
            while i < probs.len() && probs[i] >= p.threshold {
                i += 1;
            }
            runs.push((start, i));
        } else {
            i += 1;
        }
    }

    let mut merged: Vec<(usize, usize)> = Vec::new();
    for run in runs {
        if let Some(last) = merged.last_mut() {
            if run.0 - last.1 < min_silence_windows {
                last.1 = run.1;
                continue;
            }
        }
        merged.push(run);
    }

    merged
        .into_iter()
        .filter(|(s, e)| e - s >= min_speech_windows)
        .map(|(ws, we)| {
            let s_start = (ws * WINDOW_SAMPLES).saturating_sub(pad_samples);
            let s_end = ((we * WINDOW_SAMPLES) + pad_samples).min(samples.len());
            let start_ms = (s_start as u64 * 1000 / SAMPLE_RATE as u64) as u32;
            let end_ms = (s_end as u64 * 1000 / SAMPLE_RATE as u64) as u32;
            Segment {
                start_ms,
                end_ms,
                pcm: samples[s_start..s_end].to_vec(),
            }
        })
        .collect()
}

fn ms_to_windows(ms: u32) -> usize {
    (ms / WINDOW_MS) as usize
}

/// True when the VAD output looks pathological (no segments AND the
/// peak per-window probability never rose above `DEAF_THRESHOLD`).
/// Extracted so the trip-wire stays reachable from tests.
fn is_deaf(segments_empty: bool, max_prob: f32) -> bool {
    segments_empty && max_prob < DEAF_THRESHOLD
}

#[cfg(test)]
mod tests;
