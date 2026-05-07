//! Wire-format types for the queue-driven input mode.
//!
//! Mirrors `docs/contract.md` section 3 verbatim. Both producer (bot) and
//! consumer (worker) must agree on the JSON shape; this module is the Rust
//! side of that agreement.

use serde::{Deserialize, Serialize};

/// Current wire-format version. Mismatched payloads are routed to DLQ by the
/// consumer (see contract section 9).
pub const WIRE_VERSION: u32 = 1;

// --- TranscribeJob (queue `transcribe`) ----------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TranscribeJob {
    pub v: u32,
    #[serde(rename = "type")]
    pub kind: TranscribeKind,
    pub dedup_key: String,
    pub blob_key: String,
    pub source: TelegramSource,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hints: Option<TranscribeHints>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TranscribeKind {
    #[serde(rename = "transcribe")]
    Transcribe,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TelegramSource {
    #[serde(rename = "kind")]
    pub kind: TelegramKind,
    pub chat_id: i64,
    pub message_id: i64,
    pub update_id: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub user_id: Option<i64>,
    /// RFC3339 UTC, e.g. "2026-05-07T10:15:30Z".
    pub received_at: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TelegramKind {
    #[serde(rename = "telegram")]
    Telegram,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct TranscribeHints {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mime: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub duration_sec: Option<u32>,
    /// BCP-47 language tag (e.g. "ru", "en"). If absent, worker decides.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lang: Option<String>,
}

// --- NotifyResult (queue `notifications`) --------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NotifyResult {
    pub v: u32,
    #[serde(rename = "type")]
    pub kind: NotifyKind,
    pub dedup_key: String,
    pub source: SourceRef,
    pub result: JobResult,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum NotifyKind {
    #[serde(rename = "notify_result")]
    NotifyResult,
}

/// Trimmed source reference attached to a notification.
///
/// Carries only what the bot needs to update the original message reaction;
/// `user_id` and `received_at` from the originating job are not echoed back.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SourceRef {
    pub kind: TelegramKind,
    pub chat_id: i64,
    pub message_id: i64,
    pub update_id: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "lowercase")]
pub enum JobResult {
    Ok {
        /// Reference to the produced artefact (e.g.
        /// "couchdb://notes/2026-05-07T10-15-30Z-abc123").
        output_ref: String,
        duration_ms: u64,
    },
    Error {
        error_code: ErrorCode,
        error_msg: String,
        duration_ms: u64,
    },
}

/// Error taxonomy returned to the bot for UX (reaction selection).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ErrorCode {
    /// Blob not found in the BlobStore.
    BlobMissing,
    /// ffmpeg (or other decoder) failed.
    DecodeFailed,
    /// Transcription engine failed.
    EngineFailed,
    /// Sink write failed (CouchDB, file, etc.).
    OutputFailed,
    /// Anything else.
    Internal,
}

// --- Helpers -------------------------------------------------------------

impl TranscribeJob {
    /// Build a dedup key for a Telegram message.
    /// Format: "tg:{chat_id}:{message_id}".
    pub fn dedup_key_for(chat_id: i64, message_id: i64) -> String {
        format!("tg:{chat_id}:{message_id}")
    }
}

impl SourceRef {
    pub fn from_job(src: &TelegramSource) -> Self {
        Self {
            kind: src.kind,
            chat_id: src.chat_id,
            message_id: src.message_id,
            update_id: src.update_id,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn transcribe_job_roundtrip_matches_contract() {
        let raw = r#"{
            "v": 1,
            "type": "transcribe",
            "dedup_key": "tg:123456789:42",
            "blob_key": "audio/2026/05/07/abc123.oga",
            "source": {
                "kind": "telegram",
                "chat_id": 123456789,
                "message_id": 42,
                "update_id": 987654321,
                "user_id": 111222333,
                "received_at": "2026-05-07T10:15:30Z"
            },
            "hints": {
                "mime": "audio/ogg",
                "duration_sec": 47,
                "lang": "ru"
            }
        }"#;
        let job: TranscribeJob = serde_json::from_str(raw).unwrap();
        assert_eq!(job.v, WIRE_VERSION);
        assert_eq!(job.kind, TranscribeKind::Transcribe);
        assert_eq!(job.dedup_key, "tg:123456789:42");
        assert_eq!(job.source.chat_id, 123456789);
        let back = serde_json::to_string(&job).unwrap();
        let job2: TranscribeJob = serde_json::from_str(&back).unwrap();
        assert_eq!(job2.blob_key, job.blob_key);
    }

    #[test]
    fn notify_result_ok_roundtrip() {
        let r = NotifyResult {
            v: WIRE_VERSION,
            kind: NotifyKind::NotifyResult,
            dedup_key: "tg:1:2".into(),
            source: SourceRef {
                kind: TelegramKind::Telegram,
                chat_id: 1,
                message_id: 2,
                update_id: 3,
            },
            result: JobResult::Ok {
                output_ref: "couchdb://notes/x".into(),
                duration_ms: 8421,
            },
        };
        let s = serde_json::to_string(&r).unwrap();
        assert!(s.contains("\"status\":\"ok\""));
        assert!(s.contains("\"type\":\"notify_result\""));
        let _back: NotifyResult = serde_json::from_str(&s).unwrap();
    }

    #[test]
    fn notify_result_error_roundtrip() {
        let r = NotifyResult {
            v: WIRE_VERSION,
            kind: NotifyKind::NotifyResult,
            dedup_key: "tg:1:2".into(),
            source: SourceRef {
                kind: TelegramKind::Telegram,
                chat_id: 1,
                message_id: 2,
                update_id: 3,
            },
            result: JobResult::Error {
                error_code: ErrorCode::DecodeFailed,
                error_msg: "ffmpeg exited with code 1".into(),
                duration_ms: 312,
            },
        };
        let s = serde_json::to_string(&r).unwrap();
        assert!(s.contains("\"status\":\"error\""));
        assert!(s.contains("\"error_code\":\"decode_failed\""));
        let _back: NotifyResult = serde_json::from_str(&s).unwrap();
    }

    #[test]
    fn dedup_key_format() {
        assert_eq!(TranscribeJob::dedup_key_for(123, 45), "tg:123:45");
    }
}
