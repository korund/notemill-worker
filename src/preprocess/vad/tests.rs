use super::*;

const SR: u32 = 16_000;
const WIN: usize = WINDOW_SAMPLES;

fn pcm_of_windows(n: usize) -> Vec<f32> {
    vec![0.0; n * WIN]
}

fn params(min_speech_ms: u32, min_silence_ms: u32, pad_ms: u32) -> SileroParams {
    SileroParams {
        threshold: 0.5,
        min_speech_ms,
        min_silence_ms,
        speech_pad_ms: pad_ms,
    }
}

#[test]
fn assemble_empty_probs_yields_empty() {
    let segs = assemble_segments(&[], &[], &params(0, 0, 0));
    assert!(segs.is_empty());
}

#[test]
fn assemble_all_silence_yields_empty() {
    let pcm = pcm_of_windows(10);
    let probs = vec![0.1; 10];
    let segs = assemble_segments(&pcm, &probs, &params(0, 0, 0));
    assert!(segs.is_empty());
}

#[test]
fn assemble_one_long_run_yields_one_segment() {
    let pcm = pcm_of_windows(20);
    let probs = vec![0.9; 20];
    let segs = assemble_segments(&pcm, &probs, &params(0, 0, 0));
    assert_eq!(segs.len(), 1);
    assert_eq!(segs[0].pcm.len(), 20 * WIN);
    assert_eq!(segs[0].start_ms, 0);
}

#[test]
fn min_speech_ms_drops_short_blips() {
    // Each window is 32 ms. A 2-window blip is 64 ms; min_speech_ms=200
    // requires at least ceil(200/32)=6 windows.
    let pcm = pcm_of_windows(20);
    let mut probs = vec![0.1f32; 20];
    probs[5] = 0.9;
    probs[6] = 0.9;
    let segs = assemble_segments(&pcm, &probs, &params(200, 0, 0));
    assert!(segs.is_empty(), "short blip should be dropped");
}

#[test]
fn min_silence_ms_merges_close_runs() {
    // Two runs of 6 windows each separated by 2 windows of silence
    // (~64 ms). min_silence_ms = 200 should merge them.
    let pcm = pcm_of_windows(20);
    let mut probs = vec![0.1f32; 20];
    for i in 0..6 {
        probs[i] = 0.9;
    }
    for i in 8..14 {
        probs[i] = 0.9;
    }
    let segs = assemble_segments(&pcm, &probs, &params(0, 200, 0));
    assert_eq!(segs.len(), 1, "close runs must merge with min_silence_ms");
    assert_eq!(segs[0].start_ms, 0);
}

#[test]
fn long_silence_does_not_merge() {
    // Same as above but separated by 8 windows (~256 ms). With
    // min_silence_ms=200 the gap exceeds the merge window.
    let pcm = pcm_of_windows(30);
    let mut probs = vec![0.1f32; 30];
    for i in 0..6 {
        probs[i] = 0.9;
    }
    for i in 14..20 {
        probs[i] = 0.9;
    }
    let segs = assemble_segments(&pcm, &probs, &params(0, 200, 0));
    assert_eq!(segs.len(), 2);
}

#[test]
fn speech_pad_ms_expands_and_clamps_to_buffer() {
    // 5 windows of speech in the middle of a 20-window buffer, pad 100ms.
    let pcm = pcm_of_windows(20);
    let mut probs = vec![0.1f32; 20];
    for i in 8..13 {
        probs[i] = 0.9;
    }
    let segs = assemble_segments(&pcm, &probs, &params(0, 0, 100));
    assert_eq!(segs.len(), 1);
    // 100ms = 1600 samples, 3 windows ~ 1536. Pad should add ~1600
    // samples on each side.
    let pad_samples = (100 * SR as usize) / 1000;
    let expected_start = (8 * WIN).saturating_sub(pad_samples);
    let expected_end = ((13 * WIN) + pad_samples).min(20 * WIN);
    assert_eq!(segs[0].pcm.len(), expected_end - expected_start);
}

#[test]
fn deaf_when_no_segments_and_max_prob_below_threshold() {
    assert!(is_deaf(true, 0.0));
    assert!(is_deaf(true, 0.05));
    assert!(is_deaf(true, DEAF_THRESHOLD - f32::EPSILON));
}

#[test]
fn not_deaf_when_segments_present() {
    // Even with low max_prob, the presence of any segment means the
    // model produced usable output -- not deaf.
    assert!(!is_deaf(false, 0.0));
    assert!(!is_deaf(false, 0.99));
}

#[test]
fn not_deaf_when_max_prob_above_threshold() {
    // No segments crossed the user threshold, but the model clearly
    // heard something near it -- whisper territory, not a broken VAD.
    assert!(!is_deaf(true, DEAF_THRESHOLD));
    assert!(!is_deaf(true, 0.3));
    assert!(!is_deaf(true, 0.99));
}

#[test]
fn speech_pad_clamps_at_buffer_start() {
    let pcm = pcm_of_windows(10);
    let probs = vec![0.9; 10];
    let segs = assemble_segments(&pcm, &probs, &params(0, 0, 500));
    assert_eq!(segs.len(), 1);
    // The whole buffer is speech, pad would extend past both ends but
    // must clamp.
    assert_eq!(segs[0].pcm.len(), 10 * WIN);
    assert_eq!(segs[0].start_ms, 0);
}
