
use super::*;
use tokio::runtime::Runtime;

fn rt() -> Runtime {
    Runtime::new().unwrap()
}

fn open_mem() -> SqliteBackend {
    // ":memory:" gives a fresh DB per Connection. We share it via Arc<Mutex<...>>,
    // so it persists for the lifetime of the SqliteBackend instance.
    SqliteBackend::open(":memory:").unwrap()
}

#[derive(serde::Serialize, serde::Deserialize, Debug, PartialEq)]
struct Msg {
    n: i32,
}

#[test]
fn enqueue_pop_ack() {
    let rt = rt();
    rt.block_on(async {
        let be = open_mem();
        let q: SqliteQueue<Msg> = be.queue("test", 5).unwrap();
        q.enqueue(Msg { n: 1 }).await.unwrap();
        q.enqueue(Msg { n: 2 }).await.unwrap();
        let m1 = q.pop(60).await.unwrap().unwrap();
        assert_eq!(m1.payload, Msg { n: 1 });
        assert_eq!(m1.receive_count, 1);
        q.ack(&m1.receipt).await.unwrap();
        let m2 = q.pop(60).await.unwrap().unwrap();
        assert_eq!(m2.payload, Msg { n: 2 });
        q.ack(&m2.receipt).await.unwrap();
        assert!(q.pop(60).await.unwrap().is_none());
    });
}

#[test]
fn nack_makes_visible_immediately() {
    let rt = rt();
    rt.block_on(async {
        let be = open_mem();
        let q: SqliteQueue<Msg> = be.queue("test", 5).unwrap();
        q.enqueue(Msg { n: 7 }).await.unwrap();
        let m = q.pop(3600).await.unwrap().unwrap();
        // With visibility 1h, a second pop would normally see nothing.
        assert!(q.pop(60).await.unwrap().is_none());
        q.nack(&m.receipt).await.unwrap();
        let m2 = q.pop(60).await.unwrap().unwrap();
        assert_eq!(m2.payload, Msg { n: 7 });
        assert_eq!(m2.receive_count, 2);
    });
}

#[test]
fn dlq_after_max_receive() {
    let rt = rt();
    rt.block_on(async {
        let be = open_mem();
        let q: SqliteQueue<Msg> = be.queue("dlqtest", 2).unwrap();
        q.enqueue(Msg { n: 9 }).await.unwrap();
        // pop with vis=0 -> message becomes immediately visible again,
        // so receive_count climbs each loop iteration.
        for _ in 0..2 {
            let m = q.pop(0).await.unwrap().unwrap();
            // do not ack; let visibility expire (vis=0 => already expired).
            let _ = m;
        }
        // Third pop must promote to DLQ and return None (no other rows).
        assert!(q.pop(0).await.unwrap().is_none());
        // Verify DLQ row landed.
        let conn = be.conn.lock().unwrap();
        let n: i64 = conn
            .query_row("SELECT COUNT(*) FROM queue_dlqtest_dlq", [], |r| r.get(0))
            .unwrap();
        assert_eq!(n, 1);
    });
}

#[test]
fn processed_store_roundtrip() {
    let rt = rt();
    rt.block_on(async {
        let be = open_mem();
        let p = be.processed_store();
        assert!(p.lookup("nope").await.unwrap().is_none());
        let rec = ProcessedRecord {
            dedup_key: "tg:1:2".into(),
            finished_at_ms: 12345,
            status: ProcessedStatus::Ok {
                output_ref: "couchdb://x".into(),
            },
        };
        p.record(&rec).await.unwrap();
        let got = p.lookup("tg:1:2").await.unwrap().unwrap();
        assert_eq!(got.dedup_key, "tg:1:2");
        assert_eq!(got.finished_at_ms, 12345);
        match got.status {
            ProcessedStatus::Ok { output_ref } => assert_eq!(output_ref, "couchdb://x"),
            _ => panic!("expected ok"),
        }
        // Overwrite with an error record.
        p.record(&ProcessedRecord {
            dedup_key: "tg:1:2".into(),
            finished_at_ms: 67890,
            status: ProcessedStatus::Error {
                error_code: ErrorCode::DecodeFailed,
            },
        })
        .await
        .unwrap();
        let got2 = p.lookup("tg:1:2").await.unwrap().unwrap();
        match got2.status {
            ProcessedStatus::Error { error_code } => {
                assert_eq!(error_code, ErrorCode::DecodeFailed);
            }
            _ => panic!("expected error"),
        }
    });
}
