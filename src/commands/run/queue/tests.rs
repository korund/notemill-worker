//! Pure-helper tests for queue runner: `note_stem` and `collision_free_path`.
//!
//! `QueueProcessor::process` itself is not covered here -- it requires a live
//! Pipeline + AudioSource and is better suited to an integration test once
//! lightweight stubs exist.

use std::cell::RefCell;
use std::collections::HashSet;

use crate::config::NamingConfig;
use crate::input::queue::job::{TelegramKind, TelegramSource, TranscribeJob, TranscribeKind};

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
