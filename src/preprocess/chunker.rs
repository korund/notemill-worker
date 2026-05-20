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

/// Join transcript fragments produced from chunks. When the producing
/// chunk had `has_overlap_with_next == true`, the tail of `prev` and the
/// head of `next` likely repeat the same words. We dedup by finding the
/// longest suffix of `prev` (up to ~10 words) that is a prefix of `next`
/// and dropping it from `next`.
pub fn join_texts(parts: &[(String, bool)]) -> String {
    let mut result = String::new();
    for (i, (text, has_overlap)) in parts.iter().enumerate() {
        if i == 0 {
            result.push_str(text.trim());
            continue;
        }
        let prev_has_overlap = parts[i - 1].1;
        if prev_has_overlap {
            let trimmed = dedup_overlap(&result, text);
            if !trimmed.is_empty() {
                if !result.ends_with(' ') {
                    result.push(' ');
                }
                result.push_str(&trimmed);
            }
        } else {
            if !text.trim().is_empty() {
                if !result.ends_with('\n') && !result.is_empty() {
                    result.push(' ');
                }
                result.push_str(text.trim());
            }
        }
        let _ = has_overlap; // sentinel: only the producing side's flag matters
    }
    result
}

/// Drop from `next` the longest leading sequence of words (up to
/// `MAX_WORDS`) that also appears at the very end of `prev`.
fn dedup_overlap(prev: &str, next: &str) -> String {
    const MAX_WORDS: usize = 10;
    let prev_words: Vec<&str> = prev.split_whitespace().collect();
    let next_words: Vec<&str> = next.split_whitespace().collect();
    if prev_words.is_empty() || next_words.is_empty() {
        return next.trim().to_string();
    }
    let max_k = MAX_WORDS.min(prev_words.len()).min(next_words.len());
    let mut best = 0;
    for k in (1..=max_k).rev() {
        let prev_tail = &prev_words[prev_words.len() - k..];
        let next_head = &next_words[..k];
        if eq_ci(prev_tail, next_head) {
            best = k;
            break;
        }
    }
    next_words[best..].join(" ")
}

fn eq_ci(a: &[&str], b: &[&str]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    a.iter()
        .zip(b.iter())
        .all(|(x, y)| x.to_lowercase() == y.to_lowercase())
}

#[cfg(test)]
mod tests;
