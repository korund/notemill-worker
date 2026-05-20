use std::time::Instant;

use tracing::{info, warn};

use crate::preprocess::chunker::Chunk;
use crate::preprocess::{Preprocess, Segment, Speech};
use crate::{decode, engine, input, output, Result};

const SAMPLE_RATE: u32 = 16_000;

/// Result of a single pipeline run, surfaced to the job runner so it
/// can pick the right user-facing notification.
#[derive(Debug, PartialEq, Eq)]
pub enum RunOutcome {
    /// Transcript was produced and written to the sink.
    Written,
    /// Segmenter reported `Speech::None` (genuinely silent input); no
    /// transcription was attempted and nothing was written.
    NoSpeech,
}

pub struct Pipeline {
    pub decoder: Box<dyn decode::AudioDecoder>,
    pub preprocess: Preprocess,
    pub transcriber: Box<dyn engine::Transcriber>,
}

impl Pipeline {
    pub fn run_one(
        &mut self,
        source: &dyn input::AudioSource,
        sink: &mut dyn output::OutputSink,
        fm: Option<&str>,
    ) -> Result<RunOutcome> {
        let raw = source.read()?;
        let pcm = self.decoder.decode(&raw)?;
        let segments = match self.segment(pcm)? {
            Some(s) => s,
            None => return Ok(RunOutcome::NoSpeech),
        };
        let chunks = self.chunk(segments);

        let mut parts: Vec<(String, bool)> = Vec::with_capacity(chunks.len());
        for (i, c) in chunks.iter().enumerate() {
            let pcm = decode::Pcm16kMono {
                samples: c.pcm.clone(),
            };
            let text = self.transcriber.transcribe(&pcm)?;
            info!(
                chunk = i + 1,
                of = chunks.len(),
                samples = c.pcm.len(),
                "chunk transcribed"
            );
            parts.push((text, c.has_overlap_with_next));
        }
        let text = crate::preprocess::chunker::join_texts(&parts);

        let body = match fm {
            Some(prefix) => format!("{}{}\n", prefix, text),
            None => format!("{}\n", text),
        };
        sink.write(&body)?;
        Ok(RunOutcome::Written)
    }

    /// Returns `Some(segments)` to transcribe, or `None` when the
    /// segmenter classified the input as `Speech::None` and the caller
    /// should short-circuit.
    fn segment(&mut self, pcm: decode::Pcm16kMono) -> Result<Option<Vec<Segment>>> {
        let Some(seg) = self.preprocess.segmenter.as_mut() else {
            return Ok(Some(vec![full_segment(pcm)]));
        };
        let segments = match seg.segment(&pcm)? {
            Speech::Detected(segments) => segments,
            Speech::Faint => {
                warn!("vad found no speech above threshold, falling back to full audio");
                return Ok(Some(vec![full_segment(pcm)]));
            }
            Speech::None => {
                info!("vad classified input as silent, skipping transcription");
                return Ok(None);
            }
        };
        let total_ms: u32 = segments.last().map(|s| s.end_ms).unwrap_or(0);
        let kept_samples: usize = segments.iter().map(|s| s.pcm.len()).sum();
        let original_ms = (pcm.samples.len() as u64 * 1000) / SAMPLE_RATE as u64;
        let kept_ms = (kept_samples as u64 * 1000) / SAMPLE_RATE as u64;
        info!(
            n_segments = segments.len(),
            original_ms, kept_ms, total_ms, "vad applied"
        );
        Ok(Some(segments))
    }

    fn chunk(&self, segments: Vec<Segment>) -> Vec<Chunk> {
        if let Some(chunker) = self.preprocess.chunker.as_ref() {
            let chunks = chunker.chunk(segments);
            info!(n_chunks = chunks.len(), "chunking applied");
            chunks
        } else {
            // No chunker: concatenate every segment into a single chunk so
            // the encoder sees one continuous speech stream.
            let mut combined: Vec<f32> = Vec::new();
            for s in segments {
                combined.extend_from_slice(&s.pcm);
            }
            vec![Chunk {
                pcm: combined,
                has_overlap_with_next: false,
            }]
        }
    }
}

fn full_segment(pcm: decode::Pcm16kMono) -> Segment {
    let end_ms = ((pcm.samples.len() as u64 * 1000) / SAMPLE_RATE as u64) as u32;
    Segment {
        start_ms: 0,
        end_ms,
        pcm: pcm.samples,
    }
}

// ---------------------------------------------------------------------------
// ModelGuard -- lazy load / idle unload for the pipeline's heavy resources.
// ---------------------------------------------------------------------------

pub type PipelineFactory = Box<dyn FnMut() -> Result<Pipeline>>;

pub struct ModelGuard {
    pipeline: Option<Pipeline>,
    factory: PipelineFactory,
    idle_timeout: std::time::Duration,
    last_used: Option<Instant>,
}

impl ModelGuard {
    pub fn new(factory: PipelineFactory, idle_timeout: std::time::Duration) -> Self {
        Self {
            pipeline: None,
            factory,
            idle_timeout,
            last_used: None,
        }
    }

    pub fn acquire(&mut self) -> Result<&mut Pipeline> {
        if self.pipeline.is_none() {
            info!("loading model");
            self.pipeline = Some((self.factory)()?);
        }
        self.last_used = Some(Instant::now());
        Ok(self.pipeline.as_mut().unwrap())
    }

    pub fn try_unload(&mut self) {
        let Some(last) = self.last_used else { return };
        if last.elapsed() >= self.idle_timeout {
            if self.pipeline.is_some() {
                info!(
                    idle_ms = last.elapsed().as_millis() as u64,
                    "unloading model"
                );
                self.pipeline = None;
                self.last_used = None;
            }
        }
    }

    pub fn is_loaded(&self) -> bool {
        self.pipeline.is_some()
    }
}

#[cfg(test)]
mod tests;

