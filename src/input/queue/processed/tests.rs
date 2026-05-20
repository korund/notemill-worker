
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
        JobResult::Ok {
            output_ref,
            duration_ms,
        } => {
            assert_eq!(output_ref, "couchdb://x");
            assert_eq!(duration_ms, 0);
        }
        _ => panic!("expected ok"),
    }
}

#[test]
fn replay_no_speech() {
    let rec = ProcessedRecord {
        dedup_key: "tg:1:2".into(),
        finished_at_ms: 0,
        status: ProcessedStatus::NoSpeech {
            reason: NoSpeechReason::Silent,
        },
    };
    let n = replay_notify(&rec, src());
    match n.result {
        JobResult::NoSpeech { reason, duration_ms } => {
            assert_eq!(reason, NoSpeechReason::Silent);
            assert_eq!(duration_ms, 0);
        }
        _ => panic!("expected NoSpeech"),
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
        JobResult::Error {
            error_code,
            error_msg,
            ..
        } => {
            assert_eq!(error_code, ErrorCode::DecodeFailed);
            assert_eq!(error_msg, "");
        }
        _ => panic!("expected error"),
    }
}
