//! `worker queue ...` -- admin operations on the local SQLite queue.
//!
//! Currently exposes DLQ inspection and one-row requeue. Intended to be
//! invoked via `docker exec` on a running worker; no auth.

use std::path::Path;

use chrono::{DateTime, Utc};

use crate::cli::{DlqCommand, QueueCommand, QueueName};
use crate::config::Config;
use crate::input::queue::backends::sqlite::SqliteBackend;
use crate::{Error, Result};

pub fn run(cmd: QueueCommand) -> Result<()> {
    match cmd {
        QueueCommand::Dlq { cmd } => match cmd {
            DlqCommand::List { config, queue } => list(&config, queue),
            DlqCommand::Requeue { config, queue, id } => requeue(&config, queue, id),
        },
    }
}

fn open_backend(config_path: &Path) -> Result<SqliteBackend> {
    let cfg = Config::load(config_path)?;
    let sqlite_path = cfg
        .input
        .as_ref()
        .and_then(|i| i.queue.as_ref())
        .and_then(|q| q.sqlite.as_ref())
        .map(|s| s.path.clone())
        .ok_or_else(|| Error::Config("input.queue.sqlite.path missing".into()))?;
    SqliteBackend::open(&sqlite_path)
}

fn list(config_path: &Path, queue: QueueName) -> Result<()> {
    let backend = open_backend(config_path)?;
    let rows = backend.dlq_list(queue.as_str())?;
    if rows.is_empty() {
        println!("(empty)");
        return Ok(());
    }
    println!(
        "{:<6}  {:<40}  {:<19}  {:<19}  {:>8}",
        "ID", "DEDUP_KEY", "ENQUEUED_UTC", "MOVED_UTC", "ATTEMPTS"
    );
    for r in rows {
        let dedup = extract_dedup_key(&r.payload).unwrap_or_else(|| "<unparsable>".into());
        println!(
            "{:<6}  {:<40}  {:<19}  {:<19}  {:>8}",
            r.id,
            truncate(&dedup, 40),
            fmt_ts(r.enqueued_at_ms),
            fmt_ts(r.moved_at_ms),
            r.receive_count
        );
    }
    Ok(())
}

fn requeue(config_path: &Path, queue: QueueName, id: i64) -> Result<()> {
    let backend = open_backend(config_path)?;
    backend.dlq_requeue(queue.as_str(), id)?;
    println!("requeued: queue={} id={}", queue.as_str(), id);
    Ok(())
}

fn extract_dedup_key(payload: &str) -> Option<String> {
    let v: serde_json::Value = serde_json::from_str(payload).ok()?;
    v.get("dedup_key")?.as_str().map(|s| s.to_string())
}

fn fmt_ts(ms: i64) -> String {
    DateTime::<Utc>::from_timestamp_millis(ms)
        .map(|dt| dt.format("%Y-%m-%d %H:%M:%S").to_string())
        .unwrap_or_else(|| "?".into())
}

fn truncate(s: &str, n: usize) -> String {
    if s.chars().count() <= n {
        s.to_string()
    } else {
        let mut out: String = s.chars().take(n - 3).collect();
        out.push_str("...");
        out
    }
}
