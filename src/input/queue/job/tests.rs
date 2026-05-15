
use super::*;

#[test]
fn transcribe_job_roundtrip_matches_contract() {
    let raw = r#"{
            "v": 1,
            "type": "transcribe",
            "dedup_key": "tg:123456789:42",
            "audio_key": "audio/2026/05/07/abc123.oga",
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
    assert_eq!(job2.audio_key, job.audio_key);
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
