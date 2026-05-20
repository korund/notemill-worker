//! Pure-helper tests for queue runner (`note_stem`, `collision_free_path`)
//! plus `QueueProcessor::process` mapping from `RunOutcome` to
//! `ProcessOutcome` against a stubbed pipeline.

use std::cell::{Cell, RefCell};
use std::collections::HashSet;
use std::rc::Rc;

use crate::config::NamingConfig;
use crate::decode::{AudioDecoder, Pcm16kMono};
use crate::engine::Transcriber;
use crate::input::queue::job::{
    NoSpeechReason, TelegramKind, TelegramSource, TranscribeJob, TranscribeKind,
};
use crate::input::queue::ProcessOutcome;
use crate::input::{AudioSource, RawAudio};
use crate::output::OutputSink;
use crate::pipeline::Pipeline;
use crate::preprocess::{Preprocess, Segment, Speech, SpeechSegmenter};

use super::*;

fn job(chat_id: i64, message_id: i64, received_at: &str) -> TranscribeJob {
    TranscribeJob {
        v: 1,
        kind: TranscribeKind::Transcribe,
        dedup_key: TranscribeJob::dedup_key_for(chat_id, message_id),
        audio_key: format!("buckets/voice/{chat_id}-{message_id}.ogg"),
        source: TelegramSource {
            kind: TelegramKind::Telegram,
            chat_id,
            message_id,
            update_id: 1,
            user_id: None,
            received_at: received_at.to_string(),
        },
        hints: None,
    }
}

// ---------- note_stem() ----------

#[test]
fn note_stem_message_id_uses_dedup_key_with_dashes() {
    let j = job(123, 45, "2026-05-07T10:15:30Z");
    assert_eq!(note_stem(&j, &NamingConfig::MessageId), "tg-123-45");
}

#[test]
fn note_stem_message_id_handles_negative_chat_id() {
    // Group chats have negative chat ids; the colon replacement still works.
    let j = job(-1001234567890, 7, "2026-05-07T10:15:30Z");
    assert_eq!(note_stem(&j, &NamingConfig::MessageId), "tg--1001234567890-7");
}

#[test]
fn note_stem_datetime_formats_received_at() {
    let j = job(1, 1, "2026-05-07T10:15:30Z");
    let naming = NamingConfig::Datetime {
        format: "%Y-%m-%d_%H-%M-%S".into(),
    };
    assert_eq!(note_stem(&j, &naming), "2026-05-07_10-15-30");
}

#[test]
fn note_stem_datetime_preserves_timezone_offset() {
    // chrono parses with offset; formatting uses the parsed local-clock fields.
    let j = job(1, 1, "2026-05-07T13:15:30+03:00");
    let naming = NamingConfig::Datetime {
        format: "%H-%M-%S".into(),
    };
    assert_eq!(note_stem(&j, &naming), "13-15-30");
}

#[test]
fn note_stem_datetime_falls_back_to_message_id_on_bad_date() {
    // If received_at is malformed, the stem must not panic -- it falls back to
    // the message-id form so a job can still be processed (and the bug is
    // visible in the filename instead of as a crash).
    let j = job(7, 8, "not-a-date");
    let naming = NamingConfig::Datetime {
        format: "%Y".into(),
    };
    assert_eq!(note_stem(&j, &naming), "tg-7-8");
}

#[test]
fn note_stem_datetime_with_zero_offset_z_suffix() {
    let j = job(1, 1, "2026-01-01T00:00:00Z");
    let naming = NamingConfig::Datetime {
        format: "%Y%m%dT%H%M%SZ".into(),
    };
    assert_eq!(note_stem(&j, &naming), "20260101T000000Z");
}

// ---------- collision_free_path() ----------

fn taken(ids: &[&str]) -> RefCell<HashSet<String>> {
    RefCell::new(ids.iter().map(|s| s.to_string()).collect())
}

#[test]
fn collision_free_returns_base_when_unused() {
    let store = taken(&[]);
    let p = collision_free_path("Inbox", "note", |id| {
        Ok(store.borrow().contains(id))
    })
    .unwrap();
    assert_eq!(p, "Inbox/note.md");
}

#[test]
fn collision_free_appends_one_on_first_collision() {
    // exists() is queried with the lowercase candidate; mirror that here.
    let store = taken(&["inbox/note.md"]);
    let p = collision_free_path("Inbox", "note", |id| {
        Ok(store.borrow().contains(id))
    })
    .unwrap();
    assert_eq!(p, "Inbox/note-1.md");
}

#[test]
fn collision_free_walks_until_free_suffix() {
    let store = taken(&["inbox/note.md", "inbox/note-1.md", "inbox/note-2.md"]);
    let p = collision_free_path("Inbox", "note", |id| {
        Ok(store.borrow().contains(id))
    })
    .unwrap();
    assert_eq!(p, "Inbox/note-3.md");
}

#[test]
fn collision_free_case_insensitive_lookup() {
    // CouchDB doc ids are case-insensitive in practice; the helper lowercases
    // before checking so that "Inbox/Note.md" and "inbox/note.md" collide.
    let store = taken(&["inbox/note.md"]);
    let p = collision_free_path("Inbox", "Note", |id| {
        Ok(store.borrow().contains(id))
    })
    .unwrap();
    assert_eq!(p, "Inbox/Note-1.md");
}

#[test]
fn collision_free_propagates_exists_error() {
    let p = collision_free_path("Inbox", "note", |_id| {
        Err(Error::Output("backend offline".into()))
    });
    assert!(p.is_err());
}

// ---------- QueueProcessor::process mapping ----------

struct StubSource;
impl AudioSource for StubSource {
    fn name(&self) -> &str {
        "stub"
    }
    fn read(&self) -> Result<RawAudio> {
        Ok(RawAudio {
            bytes: vec![0u8; 4],
            format_hint: None,
        })
    }
}

struct StubDecoder;
impl AudioDecoder for StubDecoder {
    fn decode(&self, _raw: &RawAudio) -> Result<Pcm16kMono> {
        Ok(Pcm16kMono {
            samples: vec![0.0; 16_000],
        })
    }
}

struct StubSegmenter(Cell<Option<Speech>>);
impl SpeechSegmenter for StubSegmenter {
    fn segment(&mut self, _pcm: &Pcm16kMono) -> Result<Speech> {
        Ok(self.0.take().expect("segmenter called twice"))
    }
}

struct StubTranscriber {
    calls: Rc<Cell<usize>>,
}
impl Transcriber for StubTranscriber {
    fn transcribe(&mut self, _pcm: &Pcm16kMono) -> Result<String> {
        self.calls.set(self.calls.get() + 1);
        Ok("ok".to_string())
    }
}

#[derive(Default)]
struct CountingSink {
    writes: Rc<Cell<usize>>,
}
impl OutputSink for CountingSink {
    fn write(&mut self, _text: &str) -> Result<()> {
        self.writes.set(self.writes.get() + 1);
        Ok(())
    }
}

fn build_pipeline(verdict: Speech) -> (Pipeline, Rc<Cell<usize>>) {
    let calls = Rc::new(Cell::new(0));
    let pre = Preprocess {
        segmenter: Some(Box::new(StubSegmenter(Cell::new(Some(verdict))))),
        chunker: None,
    };
    let p = Pipeline {
        decoder: Box::new(StubDecoder),
        preprocess: pre,
        transcriber: Box::new(StubTranscriber {
            calls: calls.clone(),
        }),
    };
    (p, calls)
}

fn shared_sink(output_ref: &str) -> (QueueSink, Rc<Cell<usize>>) {
    let writes = Rc::new(Cell::new(0));
    let sink: Box<dyn OutputSink> = Box::new(CountingSink {
        writes: writes.clone(),
    });
    let qs = QueueSink::Shared {
        sink,
        output_ref: output_ref.to_string(),
    };
    (qs, writes)
}

#[test]
fn process_written_returns_output_ref_and_writes() {
    let (mut pipeline, transcribes) =
        build_pipeline(Speech::Detected(vec![Segment {
            start_ms: 0,
            end_ms: 100,
            pcm: vec![0.0; 1600],
        }]));
    let (qs, writes) = shared_sink("stub://x");
    let mut proc = QueueProcessor { queue_sink: qs };
    let j = job(1, 2, "2026-05-07T10:15:30Z");
    let out = proc.process(&mut pipeline, &StubSource, &j).unwrap();
    match out {
        ProcessOutcome::Written(r) => assert_eq!(r, "stub://x"),
        other => panic!("expected Written, got {other:?}"),
    }
    assert_eq!(transcribes.get(), 1);
    assert_eq!(writes.get(), 1);
}

#[test]
fn process_no_speech_returns_silent_and_skips_write() {
    let (mut pipeline, transcribes) = build_pipeline(Speech::None);
    let (qs, writes) = shared_sink("stub://x");
    let mut proc = QueueProcessor { queue_sink: qs };
    let j = job(1, 2, "2026-05-07T10:15:30Z");
    let out = proc.process(&mut pipeline, &StubSource, &j).unwrap();
    match out {
        ProcessOutcome::NoSpeech(r) => assert_eq!(r, NoSpeechReason::Silent),
        other => panic!("expected NoSpeech, got {other:?}"),
    }
    assert_eq!(transcribes.get(), 0);
    assert_eq!(writes.get(), 0);
}

#[test]
fn collision_free_honors_doc_path_slash_trim() {
    // Caller may pass prefix with a trailing slash; result must not double up.
    let store = taken(&[]);
    let p = collision_free_path("Inbox/", "note", |id| {
        Ok(store.borrow().contains(id))
    })
    .unwrap();
    assert_eq!(p, "Inbox/note.md");
}
