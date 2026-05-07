//! Idempotency table for the queue worker (contract section 6).
//!
//! Worker-internal state, distinct from the queue itself: it remembers which
//! `dedup_key`s have already been finalised so that re-deliveries (caused by
//! crashes, missed acks, or visibility-timeout expirations) are absorbed
//! without re-running the pipeline.
//!
//! On a duplicate delivery the worker rebuilds a `NotifyResult` from a
//! `ProcessedRecord` and replays it; `error_msg` is not preserved in storage
//! and is replayed as empty.

use std::future::Future;

use crate::Result;

use super::job::{ErrorCode, JobResult, NotifyKind, NotifyResult, SourceRef, WIRE_VERSION};

/// One row of the `processed_jobs` table.
#[derive(Debug, Clone)]
pub struct ProcessedRecord {
    pub dedup_key: String,
    /// Unix epoch milliseconds at the moment the job was finalised.
    pub finished_at_ms: i64,
    pub status: ProcessedStatus,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProcessedStatus {
    Ok { output_ref: String },
    Error { error_code: ErrorCode },
}

/// Backend-agnostic idempotency store. SQLite impl lives in `backends`.
pub trait ProcessedStore: Send + Sync {
    fn lookup(
        &self,
        dedup_key: &str,
    ) -> impl Future<Output = Result<Option<ProcessedRecord>>> + Send;

    fn record(&self, record: &ProcessedRecord) -> impl Future<Output = Result<()>> + Send;
}

/// Rebuild a `NotifyResult` from a stored record + source ref, for replay
/// when a duplicate delivery is observed. `error_msg` is not stored, so the
/// replayed value is empty -- this is acceptable per contract section 6
/// (the bot uses only `status` and `error_code` for the reaction UX).
pub fn replay_notify(record: &ProcessedRecord, source: SourceRef) -> NotifyResult {
    let result = match &record.status {
        ProcessedStatus::Ok { output_ref } => JobResult::Ok {
            output_ref: output_ref.clone(),
            duration_ms: 0,
        },
        ProcessedStatus::Error { error_code } => JobResult::Error {
            error_code: *error_code,
            error_msg: String::new(),
            duration_ms: 0,
        },
    };
    NotifyResult {
        v: WIRE_VERSION,
        kind: NotifyKind::NotifyResult,
        dedup_key: record.dedup_key.clone(),
        source,
        result,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::input::queue::job::TelegramKind;

    fn src() -> SourceRef {
        SourceRef {
            kind: TelegramKind::Telegram,
            chat_id: 1,
            message_id: 2,
            update_id: 3,
        }
    }

    #[test]
    fn replay_ok() {
        let rec = ProcessedRecord {
            dedup_key: "tg:1:2".into(),
            finished_at_ms: 0,
            status: ProcessedStatus::Ok {
                output_ref: "couchdb://x".into(),
            },
        };
        let n = replay_notify(&rec, src());
        match n.result {
            JobResult::Ok { output_ref, duration_ms } => {
                assert_eq!(output_ref, "couchdb://x");
                assert_eq!(duration_ms, 0);
            }
            _ => panic!("expected ok"),
        }
    }

    #[test]
    fn replay_error_drops_msg() {
        let rec = ProcessedRecord {
            dedup_key: "tg:1:2".into(),
            finished_at_ms: 0,
            status: ProcessedStatus::Error {
                error_code: ErrorCode::DecodeFailed,
            },
        };
        let n = replay_notify(&rec, src());
        match n.result {
            JobResult::Error { error_code, error_msg, .. } => {
                assert_eq!(error_code, ErrorCode::DecodeFailed);
                assert_eq!(error_msg, "");
            }
            _ => panic!("expected error"),
        }
    }
}
