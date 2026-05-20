use super::*;
use crate::preprocess::Segment;

const SR: usize = SAMPLE_RATE as usize;

fn seg(seconds: f32) -> Segment {
    let n = (seconds * SR as f32) as usize;
    Segment {
        start_ms: 0,
        end_ms: (seconds * 1000.0) as u32,
        pcm: vec![0.1f32; n],
    }
}

fn chunker(max: f32, overlap: f32) -> SegmentChunker {
    SegmentChunker::new(ChunkerParams {
        max_seconds: max,
        overlap_seconds: overlap,
    })
}

#[test]
fn two_short_segments_form_one_chunk() {
    // 5s + 5s under a 20s cap fits in one chunk.
    let c = chunker(20.0, 0.5);
    let chunks = c.chunk(vec![seg(5.0), seg(5.0)]);
    assert_eq!(chunks.len(), 1);
    assert_eq!(chunks[0].pcm.len(), 10 * SR);
    assert!(!chunks[0].has_overlap_with_next);
}

#[test]
fn segments_split_into_separate_chunks_when_cap_exceeded() {
    // 15s + 10s under a 20s cap = first chunk holds the 15s segment,
    // adding the 10s would exceed; flush and start fresh.
    let c = chunker(20.0, 0.5);
    let chunks = c.chunk(vec![seg(15.0), seg(10.0)]);
    assert_eq!(chunks.len(), 2);
    assert_eq!(chunks[0].pcm.len(), 15 * SR);
    assert_eq!(chunks[1].pcm.len(), 10 * SR);
    assert!(!chunks[0].has_overlap_with_next);
    assert!(!chunks[1].has_overlap_with_next);
}

#[test]
fn long_segment_split_with_overlap_marks_intermediate_chunks() {
    // 50s segment under a 20s cap with 0.5s overlap.
    //   chunk 0: [0..20), has_overlap_with_next=true
    //   chunk 1: starts at 20-0.5=19.5 -> [19.5..39.5), has_overlap_with_next=true
    //   chunk 2: starts at 39.5-0.5=39.0 -> [39.0..50.0), last, no flag
    let c = chunker(20.0, 0.5);
    let chunks = c.chunk(vec![seg(50.0)]);
    assert_eq!(chunks.len(), 3);
    assert!(chunks[0].has_overlap_with_next);
    assert!(chunks[1].has_overlap_with_next);
    assert!(!chunks[2].has_overlap_with_next);
    // First chunk is exactly the cap.
    assert_eq!(chunks[0].pcm.len(), 20 * SR);
    // Middle chunk is also exactly the cap.
    assert_eq!(chunks[1].pcm.len(), 20 * SR);
}

#[test]
fn long_segment_after_short_flushes_accumulator() {
    let c = chunker(20.0, 0.5);
    let chunks = c.chunk(vec![seg(5.0), seg(50.0)]);
    // The 5s sits in its own chunk; the 50s then gets its own splits.
    assert!(chunks.len() >= 2);
    assert_eq!(chunks[0].pcm.len(), 5 * SR);
    assert!(!chunks[0].has_overlap_with_next);
    // Subsequent chunks belong to the long-segment split.
    assert!(chunks[1].has_overlap_with_next);
}

#[test]
fn empty_segments_yield_no_chunks() {
    let c = chunker(20.0, 0.5);
    let chunks = c.chunk(vec![]);
    assert!(chunks.is_empty());
}

#[test]
fn join_texts_concatenates_with_space_when_no_overlap() {
    let parts = vec![
        ("hello world".to_string(), false),
        ("foo bar".to_string(), false),
    ];
    assert_eq!(join_texts(&parts), "hello world foo bar");
}

#[test]
fn join_texts_dedups_overlap_at_boundary() {
    // First chunk ends "the quick brown fox"; second starts "brown fox
    // jumps over"; with overlap flag we expect dedup of two words.
    let parts = vec![
        ("the quick brown fox".to_string(), true),
        ("brown fox jumps over".to_string(), false),
    ];
    assert_eq!(join_texts(&parts), "the quick brown fox jumps over");
}

#[test]
fn join_texts_case_insensitive_dedup() {
    let parts = vec![
        ("Hello World".to_string(), true),
        ("world is round".to_string(), false),
    ];
    assert_eq!(join_texts(&parts), "Hello World is round");
}

#[test]
fn join_texts_no_overlap_match_keeps_full_next() {
    let parts = vec![
        ("alpha beta gamma".to_string(), true),
        ("xyz qux".to_string(), false),
    ];
    assert_eq!(join_texts(&parts), "alpha beta gamma xyz qux");
}

#[test]
fn join_texts_empty_inputs_safe() {
    assert_eq!(join_texts(&[]), "");
    let parts = vec![("only".to_string(), false)];
    assert_eq!(join_texts(&parts), "only");
}
