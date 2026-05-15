//! CouchDB output sink and probe.
//!
//! Targets Self-hosted LiveSync schema with E2EE OFF and path obfuscation OFF
//! (the modern children-array + eden variant). Other variants are not supported.
//!
//! Chunk _id construction (verified empirically):
//!   input = data_utf8 ++ "-" ++ len(data_utf8 in bytes, decimal)
//!   hash  = xxhash64(input)
//!   _id   = "h:" + base36(hash)   // no padding, no leading zeros

use std::hash::Hasher;

use base64::engine::general_purpose::STANDARD as B64;
use base64::Engine as _;
use twox_hash::XxHash64;

use crate::config::CouchdbConfig;
use tracing::info;
use crate::{Error, Result};

const PROBE_TIMEOUT_SECS: u64 = 15;
const HTTP_TIMEOUT_SECS: u64 = 30;

pub struct ProbeResult {
    pub db_url: String,
    pub database: String,
    pub doc_count: u64,
    pub samples: Vec<serde_json::Value>,
    /// Chunk documents referenced by the first non-deleted sample with children.
    pub chunks: Vec<serde_json::Value>,
    /// _id of the sample whose chunks were fetched, if any.
    pub chunk_source: Option<String>,
}

/// Connect to CouchDB, fetch DB info, up to `limit` sample documents,
/// and up to `chunk_limit` chunk documents referenced by the first viable sample.
/// No writes.
pub fn probe(
    cfg: &CouchdbConfig,
    password: &str,
    limit: usize,
    chunk_limit: usize,
) -> Result<ProbeResult> {
    let base = cfg.url.trim_end_matches('/');
    let db_url = format!("{}/{}", base, cfg.database);
    let auth = basic_auth(&cfg.username, password);

    let agent = ureq::AgentBuilder::new()
        .timeout(std::time::Duration::from_secs(PROBE_TIMEOUT_SECS))
        .build();

    let info: serde_json::Value = agent
        .get(&db_url)
        .set("Authorization", &auth)
        .set("Accept", "application/json")
        .call()
        .map_err(|e| Error::Output(format!("couchdb db info: {e}")))?
        .into_json()
        .map_err(|e| Error::Output(format!("couchdb db info parse: {e}")))?;

    let doc_count = info
        .get("doc_count")
        .and_then(|v| v.as_u64())
        .unwrap_or(0);

    let resp: serde_json::Value = agent
        .get(&format!("{}/_all_docs", db_url))
        .query("include_docs", "true")
        .query("limit", &limit.to_string())
        .set("Authorization", &auth)
        .set("Accept", "application/json")
        .call()
        .map_err(|e| Error::Output(format!("couchdb _all_docs: {e}")))?
        .into_json()
        .map_err(|e| Error::Output(format!("couchdb _all_docs parse: {e}")))?;

    let samples: Vec<serde_json::Value> = resp
        .get("rows")
        .and_then(|r| r.as_array())
        .map(|rows| {
            rows.iter()
                .filter_map(|row| row.get("doc").cloned())
                .collect()
        })
        .unwrap_or_default();

    let (chunks, chunk_source) = if chunk_limit > 0 {
        fetch_chunks(&agent, &db_url, &auth, &samples, chunk_limit)?
    } else {
        (Vec::new(), None)
    };

    Ok(ProbeResult {
        db_url,
        database: cfg.database.clone(),
        doc_count,
        samples,
        chunks,
        chunk_source,
    })
}

fn fetch_chunks(
    agent: &ureq::Agent,
    db_url: &str,
    auth: &str,
    samples: &[serde_json::Value],
    chunk_limit: usize,
) -> Result<(Vec<serde_json::Value>, Option<String>)> {
    let viable = samples.iter().find(|d| {
        let not_deleted = d.get("deleted").and_then(|v| v.as_bool()) != Some(true);
        let has_children = d
            .get("children")
            .and_then(|v| v.as_array())
            .map(|a| !a.is_empty())
            .unwrap_or(false);
        not_deleted && has_children
    });
    let Some(doc) = viable else {
        return Ok((Vec::new(), None));
    };
    let source_id = doc
        .get("_id")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let child_ids: Vec<String> = doc
        .get("children")
        .and_then(|v| v.as_array())
        .map(|a| {
            a.iter()
                .filter_map(|x| x.as_str().map(|s| s.to_string()))
                .take(chunk_limit)
                .collect()
        })
        .unwrap_or_default();

    let mut out = Vec::with_capacity(child_ids.len());
    for id in child_ids {
        let url = format!("{}/{}", db_url, encode_id(&id));
        let doc: serde_json::Value = agent
            .get(&url)
            .set("Authorization", auth)
            .set("Accept", "application/json")
            .call()
            .map_err(|e| Error::Output(format!("couchdb get {id}: {e}")))?
            .into_json()
            .map_err(|e| Error::Output(format!("couchdb get {id} parse: {e}")))?;
        out.push(doc);
    }
    Ok((out, Some(source_id)))
}

/// Minimal percent-encoding for a CouchDB document id used in the URL path.
/// Encodes characters that are problematic in path segments.
fn encode_id(id: &str) -> String {
    let mut out = String::with_capacity(id.len());
    for b in id.bytes() {
        let unreserved = b.is_ascii_alphanumeric()
            || matches!(b, b'-' | b'_' | b'.' | b'~' | b'!' | b'$' | b'&' | b'\'' | b'(' | b')' | b'*' | b'+' | b',' | b';' | b'=' | b':' | b'@');
        if unreserved {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{:02X}", b));
        }
    }
    out
}

fn basic_auth(user: &str, pass: &str) -> String {
    let token = B64.encode(format!("{}:{}", user, pass));
    format!("Basic {}", token)
}

/// Write a single text note to CouchDB in LiveSync schema.
/// `vault_path` is the original-case path inside the Obsidian vault, e.g.
/// "Transcripts/2026-05-07 14-30.md".
/// On success returns the main document _id.
pub fn write_note(cfg: &CouchdbConfig, password: &str, vault_path: &str, text: &str) -> Result<String> {
    let base = cfg.url.trim_end_matches('/');
    let db_url = format!("{}/{}", base, cfg.database);
    let auth = basic_auth(&cfg.username, password);

    let agent = ureq::AgentBuilder::new()
        .timeout(std::time::Duration::from_secs(HTTP_TIMEOUT_SECS))
        .build();

    let chunk_id = chunk_id_for(text);
    put_chunk_if_absent(&agent, &db_url, &auth, &chunk_id, text)?;

    let now_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0);
    let main_id = vault_path.to_lowercase();
    let size = text.as_bytes().len() as u64;

    let main = serde_json::json!({
        "path": vault_path,
        "type": "plain",
        "ctime": now_ms,
        "mtime": now_ms,
        "size": size,
        "children": [chunk_id],
        "eden": {},
    });
    put_main(&agent, &db_url, &auth, &main_id, main)?;
    Ok(main_id)
}

fn chunk_id_for(data: &str) -> String {
    let mut h = XxHash64::with_seed(0);
    h.write(data.as_bytes());
    h.write(b"-");
    h.write(data.as_bytes().len().to_string().as_bytes());
    format!("h:{}", to_base36(h.finish()))
}

fn to_base36(mut n: u64) -> String {
    if n == 0 {
        return "0".to_string();
    }
    const ALPHA: &[u8] = b"0123456789abcdefghijklmnopqrstuvwxyz";
    let mut buf = Vec::with_capacity(13);
    while n > 0 {
        buf.push(ALPHA[(n % 36) as usize]);
        n /= 36;
    }
    buf.reverse();
    String::from_utf8(buf).expect("base36 alphabet is ascii")
}

fn put_chunk_if_absent(
    agent: &ureq::Agent,
    db_url: &str,
    auth: &str,
    id: &str,
    data: &str,
) -> Result<()> {
    let body = serde_json::json!({
        "_id": id,
        "type": "leaf",
        "data": data,
    });
    let url = format!("{}/{}", db_url, encode_id(id));
    let resp = agent
        .put(&url)
        .set("Authorization", auth)
        .set("Content-Type", "application/json")
        .send_json(body);
    match resp {
        Ok(_) => Ok(()),
        Err(ureq::Error::Status(409, _)) => Ok(()), // chunk already exists -- dedup hit
        Err(e) => Err(Error::Output(format!("couchdb put chunk {id}: {e}"))),
    }
}

fn put_main(
    agent: &ureq::Agent,
    db_url: &str,
    auth: &str,
    id: &str,
    mut body: serde_json::Value,
) -> Result<()> {
    let url = format!("{}/{}", db_url, encode_id(id));
    for attempt in 0..3 {
        if attempt > 0 {
            // Refresh _rev on conflict.
            match agent
                .get(&url)
                .set("Authorization", auth)
                .call()
            {
                Ok(r) => {
                    let existing: serde_json::Value = r
                        .into_json()
                        .map_err(|e| Error::Output(format!("couchdb get main parse: {e}")))?;
                    if let Some(rev) = existing.get("_rev").and_then(|v| v.as_str()) {
                        body["_rev"] = serde_json::Value::String(rev.to_string());
                    }
                }
                Err(ureq::Error::Status(404, _)) => {
                    body.as_object_mut().map(|o| o.remove("_rev"));
                }
                Err(e) => return Err(Error::Output(format!("couchdb get main {id}: {e}"))),
            }
        }
        let resp = agent
            .put(&url)
            .set("Authorization", auth)
            .set("Content-Type", "application/json")
            .send_json(body.clone());
        match resp {
            Ok(_) => return Ok(()),
            Err(ureq::Error::Status(409, _)) => continue,
            Err(e) => return Err(Error::Output(format!("couchdb put main {id}: {e}"))),
        }
    }
    Err(Error::Output(format!(
        "couchdb put main {id}: too many _rev conflicts"
    )))
}

/// Build the CouchDB doc path for a note: `<prefix>/<stem>.md`, with any
/// trailing slash on `prefix` collapsed so users may write either
/// `target: Notes/Voice` or `target: Notes/Voice/` in YAML without effect.
/// Pure helper; `stem` is expected to already be safe for use in a path.
pub fn doc_path(prefix: &str, stem: &str) -> String {
    format!("{}/{stem}.md", prefix.trim_end_matches('/'))
}

/// Returns true if a CouchDB document with the given id already exists.
/// Used by the queue worker to detect datetime name collisions before writing.
pub fn doc_exists(cfg: &CouchdbConfig, password: &str, doc_id: &str) -> Result<bool> {
    let base = cfg.url.trim_end_matches('/');
    let db_url = format!("{}/{}", base, cfg.database);
    let auth = basic_auth(&cfg.username, password);
    let agent = ureq::AgentBuilder::new()
        .timeout(std::time::Duration::from_secs(HTTP_TIMEOUT_SECS))
        .build();
    let url = format!("{}/{}", db_url, encode_id(doc_id));
    match agent.head(&url).set("Authorization", &auth).call() {
        Ok(_) => Ok(true),
        Err(ureq::Error::Status(404, _)) => Ok(false),
        Err(e) => Err(Error::Output(format!("couchdb head {doc_id}: {e}"))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chunk_id_matches_livesync_known_sample() {
        // Verified against a real chunk from a Self-hosted LiveSync DB.
        let data = "Tunic\nOuter wilds\nThe witness ";
        assert_eq!(chunk_id_for(data), "h:1u7m29jltqpqd");
    }
}

/// Validate a probe against expected schema (LiveSync, plaintext, modern
/// children-array + eden) and return a fresh state record. Errors out with
/// a clear message when the schema is not recognized.
pub fn classify(p: &ProbeResult, fingerprint: String) -> Result<crate::state::LivesyncState> {
    if p.doc_count == 0 || p.samples.is_empty() {
        return Err(Error::Output(
            "couchdb: database is empty -- create at least one note in Obsidian first".into(),
        ));
    }
    let plaintext_modern = p.samples.iter().any(|d| {
        d.get("type").and_then(|v| v.as_str()) == Some("plain")
            && d.get("children").and_then(|v| v.as_array()).is_some()
            && d.get("eden").and_then(|v| v.as_object()).is_some()
            && d.get("path").and_then(|v| v.as_str()).is_some()
    });
    if !plaintext_modern {
        return Err(Error::Output(
            "couchdb: schema not recognized (no plaintext modern doc with type/children/eden/path). \
             DB may be encrypted or run an unsupported LiveSync version.".into(),
        ));
    }
    let path_obfuscated = p
        .samples
        .iter()
        .filter(|d| d.get("type").and_then(|v| v.as_str()) == Some("plain"))
        .any(|d| {
            let id = d.get("_id").and_then(|v| v.as_str()).unwrap_or("");
            let path = d.get("path").and_then(|v| v.as_str()).unwrap_or("");
            !path.is_empty() && id != path.to_lowercase()
        });
    if path_obfuscated {
        return Err(Error::Output(
            "couchdb: path obfuscation detected; unsupported".into(),
        ));
    }
    Ok(crate::state::LivesyncState {
        connection_fingerprint: fingerprint,
        e2ee: false,
        path_obfuscation: false,
        schema: "livesync-modern-children-eden".to_string(),
        hash_algo: "xxhash64".to_string(),
        detected_at_unix: crate::state::now_unix_seconds(),
    })
}

/// Print probe results to stdout in the same human-readable form the
/// `couchdb probe` subcommand produces.
pub fn print_probe(p: &ProbeResult) {
    println!("URL       : {}", p.db_url);
    println!("Database  : {}", p.database);
    println!("doc_count : {}", p.doc_count);
    println!("samples   : {} fetched", p.samples.len());
    println!("---");
    for (i, doc) in p.samples.iter().enumerate() {
        println!("# Sample {}", i + 1);
        match serde_json::to_string_pretty(doc) {
            Ok(s) => println!("{}", s),
            Err(_) => println!("<unprintable>"),
        }
        println!();
    }
    if let Some(src) = p.chunk_source.as_deref() {
        println!("---");
        println!("Chunks of: {}", src);
        println!("chunks    : {} fetched", p.chunks.len());
        println!();
        for (i, doc) in p.chunks.iter().enumerate() {
            println!("# Chunk {}", i + 1);
            match serde_json::to_string_pretty(doc) {
                Ok(s) => println!("{}", s),
                Err(_) => println!("<unprintable>"),
            }
            println!();
        }
    }
}

/// Default probe knobs used when probe is invoked implicitly (e.g. by `write`
/// when the cache is missing or stale).
pub const DEFAULT_PROBE_LIMIT: usize = 10;
pub const DEFAULT_PROBE_CHUNKS: usize = 3;

/// Ensure we have a valid LivesyncState matching the current connection.
/// If `force` is true, always re-probe. Otherwise, use cached state when
/// the connection fingerprint matches.
///
/// On a cache miss / mismatch / `force`, runs a probe, classifies the schema,
/// and writes the state file. Does not print: callers wanting a human-readable
/// dump should invoke `probe` + `print_probe` + `classify` + `save` directly.
pub fn ensure_state(
    cfg: &CouchdbConfig,
    password: &str,
    force: bool,
    sample_limit: usize,
    chunk_limit: usize,
) -> Result<crate::state::LivesyncState> {
    let fp = crate::state::fingerprint(cfg);
    if !force {
        if let Some(s) = crate::state::LivesyncState::load() {
            if s.connection_fingerprint == fp {
                return Ok(s);
            }
        }
    }
    let p = probe(cfg, password, sample_limit, chunk_limit)?;
    let state = classify(&p, fp)?;
    state.save()?;
    Ok(state)
}

/// OutputSink that writes the transcribed text as a LiveSync note.
/// `ensure_state` MUST be called before constructing this sink so the schema
/// has been validated and cached.
pub struct CouchdbSink {
    cfg: CouchdbConfig,
    password: String,
    vault_path: String,
}

impl CouchdbSink {
    pub fn new(cfg: CouchdbConfig, password: String, vault_path: String) -> Self {
        Self { cfg, password, vault_path }
    }
}

impl super::OutputSink for CouchdbSink {
    fn write(&mut self, text: &str) -> Result<()> {
        let id = write_note(&self.cfg, &self.password, &self.vault_path, text)?;
        info!(path = %id, "wrote");
        Ok(())
    }
}
