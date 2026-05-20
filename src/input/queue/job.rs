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
    pub audio_key: String,
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
#[serde(tag = "status", rename_all = "snake_case")]
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
    /// Pipeline finished without producing a transcript because the
    /// segmenter classified the input as silent. Not an error: the bot
    /// should send the user a friendly "no speech heard" reply.
    NoSpeech {
        reason: NoSpeechReason,
        duration_ms: u64,
    },
}

/// Why the worker decided the input had no speech. Kept as an enum so
/// future categories (too noisy, too short, empty transcript) extend
/// the surface without renaming the existing variant.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NoSpeechReason {
    /// Silero VAD produced no segments and the peak per-window
    /// probability stayed below the silence threshold.
    Silent,
}

/// Error taxonomy returned to the bot for UX (reaction selection).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ErrorCode {
    /// Object referenced by `audio_key` not found.
    AudioMissing,
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
mod tests;
