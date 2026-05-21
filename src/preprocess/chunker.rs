//! Chunking layer: turns VAD speech segments into bounded-length PCM
//! chunks before each one is fed to the transcriber.
//!
//! Two cases:
//!   * Adjacent short segments are concatenated until the running sum
//!     would exceed `max_seconds`. The chunk boundary then falls on a
//!     natural pause (VAD already removed the silence in between), so
//!     no overlap or boundary stitching is needed.
//!   * A single segment longer than `max_seconds` is sliced with
//!     `overlap_seconds` of audio repeated between consecutive chunks
//!     so a word straddling the boundary is decoded twice. The flag
//!     `has_overlap_with_next` on the producer side tells the consumer
//!     to dedup the overlapping prefix/suffix when joining transcripts.

use super::Segment;

const SAMPLE_RATE: u32 = 16_000;

#[derive(Debug, Clone)]
pub struct Chunk {
    pub pcm: Vec<f32>,
    /// True when the next chunk starts with `overlap_seconds` of audio
    /// already heard at the end of this chunk. Consumers should dedup
    /// the corresponding text overlap on join.
    pub has_overlap_with_next: bool,
}

#[derive(Debug, Clone, Copy)]
pub struct ChunkerParams {
    pub max_seconds: f32,
    pub overlap_seconds: f32,
}

impl Default for ChunkerParams {
    fn default() -> Self {
        Self {
            max_seconds: 20.0,
            overlap_seconds: 0.5,
        }
    }
}

pub trait Chunker {
    fn chunk(&self, segments: Vec<Segment>) -> Vec<Chunk>;
}

pub struct SegmentChunker {
    params: ChunkerParams,
}

impl SegmentChunker {
    pub fn new(params: ChunkerParams) -> Self {
        Self { params }
    }

    fn max_samples(&self) -> usize {
        (self.params.max_seconds * SAMPLE_RATE as f32) as usize
    }

    fn overlap_samples(&self) -> usize {
        (self.params.overlap_seconds * SAMPLE_RATE as f32) as usize
    }
}

impl Chunker for SegmentChunker {
    fn chunk(&self, segments: Vec<Segment>) -> Vec<Chunk> {
        let max = self.max_samples();
        let overlap = self.overlap_samples();

        let mut out: Vec<Chunk> = Vec::new();
        let mut current: Vec<f32> = Vec::new();

        for seg in segments {
            if seg.pcm.len() > max {
                // Flush accumulator first; the long segment owns its own
                // chunk(s) with overlap.
                if !current.is_empty() {
                    out.push(Chunk {
                        pcm: std::mem::take(&mut current),
                        has_overlap_with_next: false,
                    });
                }
                for c in split_with_overlap(seg.pcm, max, overlap) {
                    out.push(c);
                }
                continue;
            }

            if current.len() + seg.pcm.len() > max && !current.is_empty() {
                out.push(Chunk {
                    pcm: std::mem::take(&mut current),
                    has_overlap_with_next: false,
                });
            }
            current.extend_from_slice(&seg.pcm);
        }

        if !current.is_empty() {
            out.push(Chunk {
                pcm: current,
                has_overlap_with_next: false,
            });
        }
        out
    }
}

/// Slice a long PCM into `max`-sample chunks with `overlap` samples
/// repeated between consecutive chunks. All produced chunks except the
/// last have `has_overlap_with_next = true`.
fn split_with_overlap(pcm: Vec<f32>, max: usize, overlap: usize) -> Vec<Chunk> {
    debug_assert!(max > 0);
    let step = max.saturating_sub(overlap).max(1);
    let mut out = Vec::new();
    let mut start = 0;
    while start < pcm.len() {
        let end = (start + max).min(pcm.len());
        let last = end == pcm.len();
        out.push(Chunk {
            pcm: pcm[start..end].to_vec(),
            has_overlap_with_next: !last,
        });
        if last {
            break;
        }
        start += step;
    }
    out
}


#[cfg(test)]
mod tests;
