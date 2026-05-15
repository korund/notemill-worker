//! Tests for queue-runner pure helpers: `next_visibility_sec` (backoff curve)
//! and `classify` (error -> bucket mapping). These are the rules that decide
//! whether a failed job is retried, ack'd as permanent, or escalated to DLQ
//! -- a high-regression-risk area because the wrong classification either
//! hammers transient failures into DLQ or hot-loops on permanent ones.

use crate::input::queue::job::ErrorCode;
use crate::Error;

use super::*;

// ---------- next_visibility_sec(): exponential backoff ----------

#[test]
fn backoff_first_attempt_is_base_10s() {
    // receive_count == 1: first attempt; visibility = 10 * 2^0 = 10.
    assert_eq!(next_visibility_sec(1), 10);
}

#[test]
fn backoff_doubles_each_attempt() {
    assert_eq!(next_visibility_sec(2), 20);
    assert_eq!(next_visibility_sec(3), 40);
    assert_eq!(next_visibility_sec(4), 80);
}

#[test]
fn backoff_caps_at_six_hours() {
    let cap = 6 * 60 * 60;
    // Once the doubling crosses the cap, every subsequent attempt stays clamped.
    assert!(next_visibility_sec(20) <= cap);
    assert_eq!(next_visibility_sec(20), cap);
    assert_eq!(next_visibility_sec(31), cap);
    assert_eq!(next_visibility_sec(u32::MAX), cap);
}

#[test]
fn backoff_zero_receive_count_treated_as_first_attempt() {
    // saturating_sub handles a 0 input without overflow; treat as base.
    assert_eq!(next_visibility_sec(0), 10);
}

#[test]
fn backoff_never_overflows_on_high_shift() {
    // The shift is clamped to 31 internally; this must never panic regardless
    // of how high receive_count goes. Pin the saturation behavior.
    for n in [32u32, 33, 100, 1000, u32::MAX] {
        let v = next_visibility_sec(n);
        assert!(v <= 6 * 60 * 60, "{n} -> {v} exceeded cap");
    }
}

#[test]
fn backoff_curve_is_monotonically_non_decreasing() {
    let mut prev = 0u32;
    for n in 1..=40 {
        let v = next_visibility_sec(n);
        assert!(v >= prev, "non-monotone at {n}: {prev} -> {v}");
        prev = v;
    }
}

// ---------- classify(): pipeline error -> retry bucket ----------
//
// The matcher uses `matches!` because PipelineError carries an owned String
// payload and we only assert on the variant + code.

#[test]
fn classify_decode_error_is_deterministic_decode_failed() {
    let e = Error::Decode("ffmpeg blew up".into());
    let p = classify(&e, "x".into());
    assert!(matches!(
        p,
        PipelineError::Deterministic(ErrorCode::DecodeFailed, _)
    ));
}

#[test]
fn classify_engine_error_is_deterministic_engine_failed() {
    let e = Error::Engine("oom".into());
    assert!(matches!(
        classify(&e, "x".into()),
        PipelineError::Deterministic(ErrorCode::EngineFailed, _)
    ));
}

#[test]
fn classify_output_error_is_transient_output_failed() {
    // CouchDB hiccups must NOT be permanent -- a 500 from CouchDB during a
    // restart should be retried, not DLQ'd.
    let e = Error::Output("502 bad gateway".into());
    assert!(matches!(
        classify(&e, "x".into()),
        PipelineError::Transient(ErrorCode::OutputFailed, _)
    ));
}

#[test]
fn classify_io_error_is_transient_internal() {
    let e = Error::Io(std::io::Error::new(std::io::ErrorKind::Other, "disk"));
    assert!(matches!(
        classify(&e, "x".into()),
        PipelineError::Transient(ErrorCode::Internal, _)
    ));
}

#[test]
fn classify_bucket_error_is_deterministic_internal() {
    // Bucket errors (missing audio file, traversal rejection) will not be
    // fixed by retrying -- the upload either failed cleanly or never happened.
    let e = Error::Bucket("not_found".into());
    assert!(matches!(
        classify(&e, "x".into()),
        PipelineError::Deterministic(ErrorCode::Internal, _)
    ));
}

#[test]
fn classify_queue_error_is_transient_internal() {
    let e = Error::Queue("sqlite locked".into());
    assert!(matches!(
        classify(&e, "x".into()),
        PipelineError::Transient(ErrorCode::Internal, _)
    ));
}

#[test]
fn classify_config_error_is_deterministic() {
    // Misconfiguration must not hot-loop the queue.
    let e = Error::Config("missing key".into());
    assert!(matches!(
        classify(&e, "x".into()),
        PipelineError::Deterministic(ErrorCode::Internal, _)
    ));
}

#[test]
fn classify_model_error_is_deterministic() {
    let e = Error::Model("model file corrupt".into());
    assert!(matches!(
        classify(&e, "x".into()),
        PipelineError::Deterministic(ErrorCode::Internal, _)
    ));
}

#[test]
fn classify_preserves_message_payload() {
    let p = classify(&Error::Decode("anything".into()), "the-msg".into());
    match p {
        PipelineError::Deterministic(_, m) => assert_eq!(m, "the-msg"),
        _ => panic!("unexpected variant"),
    }
}
