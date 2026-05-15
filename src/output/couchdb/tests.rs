
use super::*;

#[test]
fn chunk_id_matches_livesync_known_sample() {
    // Verified against a real chunk from a Self-hosted LiveSync DB.
    let data = "Tunic\nOuter wilds\nThe witness ";
    assert_eq!(chunk_id_for(data), "h:1u7m29jltqpqd");
}

// ---------- doc_path() ----------

#[test]
fn doc_path_joins_prefix_and_stem() {
    assert_eq!(doc_path("Inbox", "note"), "Inbox/note.md");
}

#[test]
fn doc_path_trims_single_trailing_slash() {
    assert_eq!(doc_path("Inbox/", "note"), "Inbox/note.md");
}

#[test]
fn doc_path_trims_repeated_trailing_slashes() {
    // `trim_end_matches('/')` collapses any number of trailing slashes; users
    // sometimes paste paths with extra slashes from other tools.
    assert_eq!(doc_path("Inbox///", "note"), "Inbox/note.md");
}

#[test]
fn doc_path_preserves_internal_slashes_in_prefix() {
    assert_eq!(doc_path("notes/voice", "tg-42-1715"), "notes/voice/tg-42-1715.md");
}

#[test]
fn doc_path_preserves_internal_slashes_with_trailing() {
    assert_eq!(doc_path("notes/voice/", "x"), "notes/voice/x.md");
}

#[test]
fn doc_path_with_empty_prefix() {
    // Edge case: empty prefix yields a root-relative path.
    assert_eq!(doc_path("", "x"), "/x.md");
}

#[test]
fn doc_path_stem_with_dashes_passthrough() {
    // Stem is expected pre-sanitized; doc_path does not modify it.
    assert_eq!(doc_path("Inbox", "tg-123-456"), "Inbox/tg-123-456.md");
}

// ---------- to_base36() ----------

#[test]
fn to_base36_zero_renders_as_single_digit() {
    assert_eq!(to_base36(0), "0");
}

#[test]
fn to_base36_small_numerals() {
    assert_eq!(to_base36(1), "1");
    assert_eq!(to_base36(9), "9");
}

#[test]
fn to_base36_crosses_into_letters_at_ten() {
    assert_eq!(to_base36(10), "a");
    assert_eq!(to_base36(35), "z");
}

#[test]
fn to_base36_two_digit_wrap() {
    // 36 -> "10", 71 -> "1z", 72 -> "20"
    assert_eq!(to_base36(36), "10");
    assert_eq!(to_base36(71), "1z");
    assert_eq!(to_base36(72), "20");
}

#[test]
fn to_base36_max_u64() {
    // u64::MAX = 18446744073709551615; base36 = "3w5e11264sgsf"
    assert_eq!(to_base36(u64::MAX), "3w5e11264sgsf");
    assert_eq!(to_base36(u64::MAX).len(), 13);
}

#[test]
fn to_base36_is_lowercase_only() {
    // Critical for case-insensitive doc-id comparison: never produce uppercase.
    for n in [0u64, 1, 35, 36, 1234567890, u64::MAX] {
        let s = to_base36(n);
        assert!(
            s.chars().all(|c| c.is_ascii_digit() || c.is_ascii_lowercase()),
            "non-lowercase output for {n}: {s:?}"
        );
    }
}

// ---------- encode_id() ----------

#[test]
fn encode_id_passes_alphanumerics_unchanged() {
    assert_eq!(encode_id("abc123XYZ"), "abc123XYZ");
}

#[test]
fn encode_id_preserves_path_safe_unreserved_chars() {
    // These RFC 3986 unreserved + sub-delim chars are intentionally NOT escaped,
    // including '/' which is a legitimate path separator in CouchDB doc ids.
    let safe = "-._~!$&'()*+,;=:@";
    assert_eq!(encode_id(safe), safe);
}

#[test]
fn encode_id_escapes_slash_with_percent_encoding() {
    // '/' separates URL path segments, so it MUST be escaped when it appears
    // inside the doc id.
    assert_eq!(encode_id("Inbox/note"), "Inbox%2Fnote");
}

#[test]
fn encode_id_escapes_space_and_hash() {
    assert_eq!(encode_id("a b#c"), "a%20b%23c");
}

#[test]
fn encode_id_escapes_question_mark_and_percent() {
    // '?' separates path from query; '%' is the encoding marker itself.
    assert_eq!(encode_id("a?b%c"), "a%3Fb%25c");
}

#[test]
fn encode_id_emits_uppercase_hex() {
    // RFC 3986: percent-encoded triplets SHOULD be uppercase.
    let out = encode_id(" ");
    assert_eq!(out, "%20");
    assert!(out.chars().filter(|c| c.is_ascii_hexdigit() && c.is_ascii_alphabetic()).all(|c| c.is_uppercase()));
}

#[test]
fn encode_id_handles_utf8_multibyte() {
    // Cyrillic 'я' = 0xD1 0x8F in UTF-8.
    assert_eq!(encode_id("я"), "%D1%8F");
}

#[test]
fn encode_id_empty_string() {
    assert_eq!(encode_id(""), "");
}

// ---------- basic_auth() ----------

#[test]
fn basic_auth_rfc_example() {
    // Classic example from RFC 7617: "Aladdin:open sesame" -> base64.
    assert_eq!(
        basic_auth("Aladdin", "open sesame"),
        "Basic QWxhZGRpbjpvcGVuIHNlc2FtZQ=="
    );
}

#[test]
fn basic_auth_empty_password() {
    // "user:" must still emit a colon -- otherwise the server sees a one-token auth.
    assert_eq!(basic_auth("user", ""), "Basic dXNlcjo=");
}

#[test]
fn basic_auth_includes_password_with_special_chars() {
    // The colon inside the password is NOT escaped at this layer (RFC 7617
    // allows it; the server splits on the first colon only). Pin behavior.
    let out = basic_auth("u", "p:q");
    assert!(out.starts_with("Basic "));
    let token = out.strip_prefix("Basic ").unwrap();
    let decoded = base64::Engine::decode(&base64::engine::general_purpose::STANDARD, token).unwrap();
    assert_eq!(String::from_utf8(decoded).unwrap(), "u:p:q");
}

// ---------- chunk_id_for() ----------

#[test]
fn chunk_id_changes_when_text_changes() {
    let a = chunk_id_for("hello");
    let b = chunk_id_for("hello ");
    assert_ne!(a, b);
}

#[test]
fn chunk_id_is_deterministic() {
    assert_eq!(chunk_id_for("abc"), chunk_id_for("abc"));
}

#[test]
fn chunk_id_has_h_prefix_and_base36_tail() {
    let id = chunk_id_for("anything");
    let tail = id.strip_prefix("h:").expect("must start with `h:`");
    assert!(!tail.is_empty());
    assert!(tail.chars().all(|c| c.is_ascii_digit() || c.is_ascii_lowercase()));
}

// ---------- classify() ----------
//
// `classify` is the gate that determines whether the connected CouchDB is a
// supported Obsidian LiveSync database. It is the main protection against
// silently writing into the wrong schema (encrypted DB, obfuscated paths,
// legacy LiveSync) -- so each rejection branch needs a regression test.

use serde_json::json;

fn probe(docs: Vec<serde_json::Value>) -> ProbeResult {
    ProbeResult {
        db_url: "http://x".into(),
        database: "db".into(),
        doc_count: docs.len() as u64,
        samples: docs,
        chunks: Vec::new(),
        chunk_source: None,
    }
}

fn plaintext_modern(id: &str, path: &str) -> serde_json::Value {
    json!({
        "_id": id,
        "path": path,
        "type": "plain",
        "children": ["h:abc"],
        "eden": {},
    })
}

#[test]
fn classify_accepts_modern_plaintext_schema() {
    let p = probe(vec![plaintext_modern("notes/voice/x.md", "Notes/Voice/X.md")]);
    let s = classify(&p, "fp-1".into()).expect("modern plaintext is supported");
    assert_eq!(s.connection_fingerprint, "fp-1");
    assert!(!s.e2ee);
    assert!(!s.path_obfuscation);
    assert_eq!(s.schema, "livesync-modern-children-eden");
    assert_eq!(s.hash_algo, "xxhash64");
}

#[test]
fn classify_rejects_empty_database() {
    let p = probe(vec![]);
    let err = classify(&p, "fp".into()).unwrap_err();
    assert!(format!("{err}").contains("empty"), "got: {err}");
}

#[test]
fn classify_rejects_when_doc_count_zero_even_if_samples_present() {
    // Defensive: doc_count == 0 alone is enough to reject; do not classify on
    // stale or partial samples.
    let mut p = probe(vec![plaintext_modern("a", "A")]);
    p.doc_count = 0;
    assert!(classify(&p, "fp".into()).is_err());
}

#[test]
fn classify_rejects_when_no_plaintext_doc() {
    // E2EE-encrypted DB: type is something other than "plain".
    let encrypted = json!({
        "_id": "abc",
        "type": "leaf",
        "children": ["h:x"],
        "eden": {},
        "path": "Notes/x.md",
    });
    let p = probe(vec![encrypted]);
    let err = classify(&p, "fp".into()).unwrap_err();
    assert!(format!("{err}").contains("schema not recognized"), "got: {err}");
}

#[test]
fn classify_rejects_legacy_schema_without_eden() {
    // Plaintext but missing `eden` (older LiveSync layouts).
    let legacy = json!({
        "_id": "notes/x.md",
        "type": "plain",
        "children": ["h:x"],
        "path": "Notes/x.md",
    });
    let p = probe(vec![legacy]);
    assert!(classify(&p, "fp".into()).is_err());
}

#[test]
fn classify_rejects_when_children_is_not_array() {
    let bad = json!({
        "_id": "notes/x.md",
        "type": "plain",
        "children": "h:x",
        "eden": {},
        "path": "Notes/x.md",
    });
    let p = probe(vec![bad]);
    assert!(classify(&p, "fp".into()).is_err());
}

#[test]
fn classify_rejects_when_path_missing() {
    let no_path = json!({
        "_id": "notes/x.md",
        "type": "plain",
        "children": [],
        "eden": {},
    });
    let p = probe(vec![no_path]);
    assert!(classify(&p, "fp".into()).is_err());
}

#[test]
fn classify_rejects_path_obfuscation() {
    // Path obfuscation: doc _id is not the lowercased path. LiveSync clients
    // with path-obfuscation enabled hash the path; we cannot map writes back.
    let obf = json!({
        "_id": "h:9z9z9z9z",
        "type": "plain",
        "children": [],
        "eden": {},
        "path": "Notes/Voice/X.md",
    });
    let p = probe(vec![obf]);
    let err = classify(&p, "fp".into()).unwrap_err();
    assert!(format!("{err}").contains("obfuscation"), "got: {err}");
}

#[test]
fn classify_accepts_when_one_of_many_docs_is_modern_plaintext() {
    // Realistic DB: mix of plaintext notes and other doc kinds. As long as one
    // sample matches the schema, classification proceeds.
    let other = json!({"_id": "_design/x", "type": "something_else"});
    let good = plaintext_modern("notes/x.md", "Notes/X.md");
    let p = probe(vec![other, good]);
    assert!(classify(&p, "fp".into()).is_ok());
}

#[test]
fn classify_ignores_obfuscation_check_for_non_plain_docs() {
    // A non-plain doc with weird _id must not be flagged as path obfuscation;
    // the obfuscation rule applies only to type=="plain" entries.
    let weird_design = json!({
        "_id": "_design/auth",
        "type": "design",
        "path": "ignored",
    });
    let good = plaintext_modern("notes/x.md", "Notes/X.md");
    let p = probe(vec![weird_design, good]);
    assert!(classify(&p, "fp".into()).is_ok());
}
