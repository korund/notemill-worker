//! SQLite backend for `Queue<T>` and `ProcessedStore`.
//!
//! Single DB file, one table per queue (`queue_<name>`), one DLQ table per
//! queue (`queue_<name>_dlq`), plus the worker-internal `processed_jobs`
//! table. WAL journal mode lets concurrent readers/writers coexist; we use
//! a single shared `Connection` behind a `Mutex` and run each operation on
//! `tokio::task::spawn_blocking` so the async caller is not blocked.
//!
//! Visibility-timeout and DLQ semantics are implemented in `pop`.

use std::marker::PhantomData;
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use rusqlite::{params, Connection, OptionalExtension};
use serde::{de::DeserializeOwned, Serialize};
use tokio::task;

use crate::input::queue::job::ErrorCode;
use crate::input::queue::processed::{ProcessedRecord, ProcessedStatus, ProcessedStore};
use crate::input::queue::transport::{Message, Queue, Receipt};
use crate::{Error, Result};

const PROCESSED_DDL: &str = "\
CREATE TABLE IF NOT EXISTS processed_jobs (
    dedup_key   TEXT PRIMARY KEY,
    finished_at INTEGER NOT NULL,
    status      TEXT NOT NULL,
    output_ref  TEXT,
    error_code  TEXT
);
";

/// Shared SQLite handle. Hand out queue and processed-store views over the
/// same underlying connection.
#[derive(Clone)]
pub struct SqliteBackend {
    conn: Arc<Mutex<Connection>>,
}

impl SqliteBackend {
    /// Open (or create) the SQLite file, set required PRAGMAs, and apply
    /// `processed_jobs` migration. Per-queue tables are created lazily by
    /// [`SqliteBackend::queue`].
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| {
                Error::Queue(format!("sqlite: mkdir {}: {e}", parent.display()))
            })?;
        }
        let conn = Connection::open(path).map_err(map_err)?;
        // WAL: concurrent readers + single writer, no blocking on read.
        conn.pragma_update(None, "journal_mode", &"WAL").map_err(map_err)?;
        // Wait up to 5s on a locked DB before erroring; covers brief WAL checkpoints.
        conn.pragma_update(None, "busy_timeout", 5000i64).map_err(map_err)?;
        conn.pragma_update(None, "synchronous", &"NORMAL").map_err(map_err)?;
        conn.execute_batch(PROCESSED_DDL).map_err(map_err)?;
        Ok(Self {
            conn: Arc::new(Mutex::new(conn)),
        })
    }

    /// Get a typed handle to a named queue. Creates the underlying tables on
    /// first call.
    pub fn queue<T>(&self, name: &str, max_receive: u32) -> Result<SqliteQueue<T>>
    where
        T: Serialize + DeserializeOwned + Send + 'static,
    {
        let qn = sanitize_name(name)?;
        let ddl = queue_ddl(&qn);
        self.conn
            .lock()
            .expect("sqlite mutex poisoned")
            .execute_batch(&ddl)
            .map_err(map_err)?;
        Ok(SqliteQueue {
            conn: self.conn.clone(),
            name: qn,
            max_receive,
            _marker: PhantomData,
        })
    }

    pub fn processed_store(&self) -> SqliteProcessedStore {
        SqliteProcessedStore {
            conn: self.conn.clone(),
        }
    }
}

fn queue_ddl(qn: &str) -> String {
    format!(
        "\
CREATE TABLE IF NOT EXISTS queue_{qn} (
    id            INTEGER PRIMARY KEY AUTOINCREMENT,
    payload       TEXT NOT NULL,
    enqueued_at   INTEGER NOT NULL,
    visible_at    INTEGER NOT NULL,
    receive_count INTEGER NOT NULL DEFAULT 0
);
CREATE INDEX IF NOT EXISTS idx_queue_{qn}_visible_at ON queue_{qn}(visible_at);
CREATE TABLE IF NOT EXISTS queue_{qn}_dlq (
    id            INTEGER PRIMARY KEY AUTOINCREMENT,
    payload       TEXT NOT NULL,
    enqueued_at   INTEGER NOT NULL,
    moved_at      INTEGER NOT NULL,
    receive_count INTEGER NOT NULL
);
"
    )
}

fn sanitize_name(name: &str) -> Result<String> {
    let ok = !name.is_empty()
        && name.len() <= 64
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_');
    if !ok {
        return Err(Error::Queue(format!("invalid queue name: {name:?}")));
    }
    Ok(name.to_string())
}

fn map_err(e: rusqlite::Error) -> Error {
    Error::Queue(format!("sqlite: {e}"))
}

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

async fn blocking<F, R>(f: F) -> Result<R>
where
    F: FnOnce() -> Result<R> + Send + 'static,
    R: Send + 'static,
{
    task::spawn_blocking(f)
        .await
        .map_err(|e| Error::Queue(format!("blocking task join: {e}")))?
}

// --- SqliteQueue<T> ---------------------------------------------------------

pub struct SqliteQueue<T> {
    conn: Arc<Mutex<Connection>>,
    name: String,
    max_receive: u32,
    _marker: PhantomData<fn() -> T>,
}

impl<T> Queue<T> for SqliteQueue<T>
where
    T: Serialize + DeserializeOwned + Send + 'static,
{
    async fn enqueue(&self, payload: T) -> Result<()> {
        let json = serde_json::to_string(&payload)
            .map_err(|e| Error::Queue(format!("serialize: {e}")))?;
        let conn = self.conn.clone();
        let name = self.name.clone();
        blocking(move || {
            let c = conn.lock().expect("sqlite mutex poisoned");
            let sql = format!(
                "INSERT INTO queue_{name} (payload, enqueued_at, visible_at, receive_count) \
                 VALUES (?1, ?2, ?2, 0)"
            );
            let now = now_ms();
            c.execute(&sql, params![json, now]).map_err(map_err)?;
            Ok(())
        })
        .await
    }

    async fn pop(&self, visibility_sec: u32) -> Result<Option<Message<T>>> {
        let conn = self.conn.clone();
        let name = self.name.clone();
        let max_receive = self.max_receive;
        let raw = blocking(move || pop_blocking(&conn, &name, visibility_sec, max_receive)).await?;
        let Some((id, payload_json, receive_count)) = raw else {
            return Ok(None);
        };
        let payload: T = serde_json::from_str(&payload_json)
            .map_err(|e| Error::Queue(format!("deserialize: {e}")))?;
        Ok(Some(Message {
            receipt: Receipt::new(id.to_string()),
            payload,
            receive_count,
        }))
    }

    async fn ack(&self, receipt: &Receipt) -> Result<()> {
        let id = parse_id(receipt)?;
        let conn = self.conn.clone();
        let name = self.name.clone();
        blocking(move || {
            let c = conn.lock().expect("sqlite mutex poisoned");
            let sql = format!("DELETE FROM queue_{name} WHERE id = ?1");
            c.execute(&sql, params![id]).map_err(map_err)?;
            Ok(())
        })
        .await
    }

    async fn nack(&self, receipt: &Receipt) -> Result<()> {
        let id = parse_id(receipt)?;
        let conn = self.conn.clone();
        let name = self.name.clone();
        blocking(move || {
            let c = conn.lock().expect("sqlite mutex poisoned");
            let sql = format!("UPDATE queue_{name} SET visible_at = 0 WHERE id = ?1");
            c.execute(&sql, params![id]).map_err(map_err)?;
            Ok(())
        })
        .await
    }
}

fn parse_id(receipt: &Receipt) -> Result<i64> {
    receipt
        .as_str()
        .parse::<i64>()
        .map_err(|_| Error::Queue(format!("invalid receipt: {}", receipt.as_str())))
}

/// Single-step pop with DLQ promotion. Runs on the blocking pool.
///
/// Algorithm:
/// 1. `BEGIN IMMEDIATE` (acquire write lock; SQLite serializes writers).
/// 2. SELECT next visible row by `id` order.
/// 3. If `receive_count + 1 > max_receive`: copy to DLQ, DELETE from queue, retry from step 2.
/// 4. Otherwise: bump `receive_count`, push `visible_at` forward, COMMIT, return row.
fn pop_blocking(
    conn: &Mutex<Connection>,
    name: &str,
    visibility_sec: u32,
    max_receive: u32,
) -> Result<Option<(i64, String, u32)>> {
    let mut c = conn.lock().expect("sqlite mutex poisoned");
    let tx = c.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
        .map_err(map_err)?;
    let now = now_ms();
    loop {
        let sel = format!(
            "SELECT id, payload, enqueued_at, receive_count FROM queue_{name} \
             WHERE visible_at <= ?1 ORDER BY id LIMIT 1"
        );
        let row: Option<(i64, String, i64, u32)> = tx
            .query_row(&sel, params![now], |r| {
                Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?))
            })
            .optional()
            .map_err(map_err)?;
        let Some((id, payload, enqueued_at, receive_count)) = row else {
            tx.commit().map_err(map_err)?;
            return Ok(None);
        };
        let next_count = receive_count + 1;
        if next_count > max_receive {
            // Promote to DLQ and skip.
            let dlq_ins = format!(
                "INSERT INTO queue_{name}_dlq (payload, enqueued_at, moved_at, receive_count) \
                 VALUES (?1, ?2, ?3, ?4)"
            );
            tx.execute(&dlq_ins, params![payload, enqueued_at, now, receive_count])
                .map_err(map_err)?;
            let del = format!("DELETE FROM queue_{name} WHERE id = ?1");
            tx.execute(&del, params![id]).map_err(map_err)?;
            continue;
        }
        let upd = format!(
            "UPDATE queue_{name} SET visible_at = ?1, receive_count = ?2 WHERE id = ?3"
        );
        let new_visible = now + (visibility_sec as i64) * 1000;
        tx.execute(&upd, params![new_visible, next_count, id])
            .map_err(map_err)?;
        tx.commit().map_err(map_err)?;
        return Ok(Some((id, payload, next_count)));
    }
}

// --- SqliteProcessedStore ---------------------------------------------------

pub struct SqliteProcessedStore {
    conn: Arc<Mutex<Connection>>,
}

impl ProcessedStore for SqliteProcessedStore {
    async fn lookup(&self, dedup_key: &str) -> Result<Option<ProcessedRecord>> {
        let conn = self.conn.clone();
        let dk = dedup_key.to_string();
        blocking(move || {
            let c = conn.lock().expect("sqlite mutex poisoned");
            let row: Option<(String, i64, String, Option<String>, Option<String>)> = c
                .query_row(
                    "SELECT dedup_key, finished_at, status, output_ref, error_code \
                     FROM processed_jobs WHERE dedup_key = ?1",
                    params![dk],
                    |r| {
                        Ok((
                            r.get(0)?,
                            r.get(1)?,
                            r.get(2)?,
                            r.get(3)?,
                            r.get(4)?,
                        ))
                    },
                )
                .optional()
                .map_err(map_err)?;
            let Some((dedup_key, finished_at_ms, status, output_ref, error_code)) = row else {
                return Ok(None);
            };
            let status = match status.as_str() {
                "ok" => ProcessedStatus::Ok {
                    output_ref: output_ref
                        .ok_or_else(|| Error::Queue("ok row missing output_ref".into()))?,
                },
                "error" => ProcessedStatus::Error {
                    error_code: parse_error_code(
                        error_code
                            .as_deref()
                            .ok_or_else(|| Error::Queue("error row missing error_code".into()))?,
                    )?,
                },
                other => return Err(Error::Queue(format!("unknown status: {other}"))),
            };
            Ok(Some(ProcessedRecord {
                dedup_key,
                finished_at_ms,
                status,
            }))
        })
        .await
    }

    async fn record(&self, record: &ProcessedRecord) -> Result<()> {
        let conn = self.conn.clone();
        let rec = record.clone();
        blocking(move || {
            let c = conn.lock().expect("sqlite mutex poisoned");
            let (status, output_ref, error_code): (&str, Option<String>, Option<&str>) =
                match &rec.status {
                    ProcessedStatus::Ok { output_ref } => ("ok", Some(output_ref.clone()), None),
                    ProcessedStatus::Error { error_code } => {
                        ("error", None, Some(error_code_str(*error_code)))
                    }
                };
            c.execute(
                "INSERT OR REPLACE INTO processed_jobs \
                 (dedup_key, finished_at, status, output_ref, error_code) \
                 VALUES (?1, ?2, ?3, ?4, ?5)",
                params![
                    rec.dedup_key,
                    rec.finished_at_ms,
                    status,
                    output_ref,
                    error_code
                ],
            )
            .map_err(map_err)?;
            Ok(())
        })
        .await
    }
}

fn error_code_str(c: ErrorCode) -> &'static str {
    match c {
        ErrorCode::AudioMissing => "audio_missing",
        ErrorCode::DecodeFailed => "decode_failed",
        ErrorCode::EngineFailed => "engine_failed",
        ErrorCode::OutputFailed => "output_failed",
        ErrorCode::Internal => "internal",
    }
}

fn parse_error_code(s: &str) -> Result<ErrorCode> {
    Ok(match s {
        "audio_missing" => ErrorCode::AudioMissing,
        "decode_failed" => ErrorCode::DecodeFailed,
        "engine_failed" => ErrorCode::EngineFailed,
        "output_failed" => ErrorCode::OutputFailed,
        "internal" => ErrorCode::Internal,
        other => return Err(Error::Queue(format!("unknown error_code: {other}"))),
    })
}

#[cfg(test)]
mod tests {
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
}
