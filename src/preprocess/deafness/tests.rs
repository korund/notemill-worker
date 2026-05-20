use super::*;

fn seg(start_ms: u32) -> Segment {
    Segment {
        start_ms,
        end_ms: start_ms + 100,
        pcm: vec![0.0; 1600],
    }
}

#[test]
fn detected_when_segments_present() {
    let v = classify(vec![seg(0)], 0.9);
    assert!(matches!(v, Speech::Detected(s) if s.len() == 1));
}

#[test]
fn detected_even_with_low_max_prob() {
    // Presence of a segment dominates: max_prob is ignored once
    // segments are non-empty.
    let v = classify(vec![seg(0)], 0.0);
    assert!(matches!(v, Speech::Detected(_)));
}

#[test]
fn none_when_empty_and_max_prob_below_threshold() {
    assert!(matches!(classify(vec![], 0.0), Speech::None));
    assert!(matches!(classify(vec![], 0.001), Speech::None));
    assert!(matches!(
        classify(vec![], SILENCE_MAX_PROB - f32::EPSILON),
        Speech::None
    ));
}

#[test]
fn faint_when_empty_and_max_prob_at_or_above_threshold() {
    // Boundary: exactly at the threshold counts as Faint, not None.
    assert!(matches!(classify(vec![], SILENCE_MAX_PROB), Speech::Faint));
    assert!(matches!(classify(vec![], 0.3), Speech::Faint));
    assert!(matches!(classify(vec![], 0.99), Speech::Faint));
}
