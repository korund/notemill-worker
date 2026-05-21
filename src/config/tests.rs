//! Parser and default-value tests. Build configs from YAML strings via
//! `Config::from_str`, not the filesystem.

use super::*;

fn parse(yaml: &str) -> Config {
    Config::from_str(yaml, &[]).expect("config should parse")
}

fn parse_with(yaml: &str, overrides: &[(&str, &str)]) -> Config {
    let owned: Vec<(String, String)> = overrides
        .iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect();
    Config::from_str(yaml, &owned).expect("config should parse")
}

// ---------- minimal / required shape ----------

#[test]
fn minimal_config_only_output_section() {
    let cfg = parse("output: {}\n");
    assert!(cfg.model.is_none());
    assert!(cfg.input.is_none());
    // Missing `audio:` section materializes the default block so VAD and
    // chunking are on out of the box (prevents OOM on long recordings).
    assert!(cfg.audio.preprocess.vad.enabled);
    assert!(cfg.audio.preprocess.chunking.enabled);
    assert!(cfg.output.sink.is_none());
    assert!(cfg.output.couchdb.is_none());
    assert!(cfg.output.file.is_none());
    assert!(cfg.output.stdout.is_none());
    assert!(matches!(cfg.output.name, NamingConfig::MessageId));
}

#[test]
fn missing_output_section_is_error() {
    // OutputConfig is required (not Option).
    assert!(Config::from_str("model: {}\n", &[]).is_err());
}

// ---------- per-sink defaults ----------

#[test]
fn couchdb_target_defaults_to_inbox_when_omitted() {
    let cfg = parse("output:\n  couchdb:\n    url: http://x\n    database: db\n    username: u\n");
    let cdb = cfg.output.couchdb.expect("couchdb block present");
    assert_eq!(cdb.target, "Inbox");
}

#[test]
fn couchdb_target_explicit_value_preserved() {
    let cfg = parse(
        "output:\n  couchdb:\n    url: http://x\n    database: db\n    username: u\n    target: Notes/Voice\n",
    );
    assert_eq!(cfg.output.couchdb.unwrap().target, "Notes/Voice");
}

#[test]
fn couchdb_target_null_is_rejected() {
    // `target: String`, not Option<String>. Explicit null is a sensible parse error.
    let result = Config::from_str(
        "output:\n  couchdb:\n    url: http://x\n    database: db\n    username: u\n    target: null\n",
        &[],
    );
    assert!(result.is_err());
}

#[test]
fn file_separator_defaults_to_dashes() {
    let cfg = parse("output:\n  file:\n    path: /tmp/out.md\n");
    let f = cfg.output.file.expect("file block present");
    assert_eq!(f.separator, Some("---".to_string()));
    assert!(!f.overwrite); // overwrite default false
}

#[test]
fn file_separator_explicit_null_disables_it() {
    let cfg = parse("output:\n  file:\n    path: /tmp/out.md\n    separator: null\n");
    assert_eq!(cfg.output.file.unwrap().separator, None);
}

#[test]
fn file_separator_custom_value_preserved() {
    let cfg = parse("output:\n  file:\n    path: /tmp/out.md\n    separator: \"=====\"\n");
    assert_eq!(
        cfg.output.file.unwrap().separator,
        Some("=====".to_string())
    );
}

#[test]
fn stdout_separator_defaults_to_dashes() {
    let cfg = parse("output:\n  stdout: {}\n");
    let s = cfg.output.stdout.expect("stdout block present");
    assert_eq!(s.separator, Some("---".to_string()));
}

#[test]
fn stdout_separator_explicit_null_disables_it() {
    let cfg = parse("output:\n  stdout:\n    separator: null\n");
    assert_eq!(cfg.output.stdout.unwrap().separator, None);
}

// ---------- model section: every field is optional ----------

#[test]
fn model_section_all_fields_optional() {
    let cfg = parse("output: {}\nmodel: {}\n");
    let m = cfg.model.expect("model block present");
    assert!(m.name.is_none());
    assert!(m.family.is_none());
    assert!(m.dir.is_none());
}

#[test]
fn model_section_can_be_omitted_entirely() {
    let cfg = parse("output: {}\n");
    assert!(cfg.model.is_none());
}

// ---------- naming strategy ----------

#[test]
fn naming_defaults_to_message_id_when_block_absent() {
    let cfg = parse("output: {}\n");
    assert!(matches!(cfg.output.name, NamingConfig::MessageId));
}

#[test]
fn naming_message_id_explicit() {
    let cfg = parse("output:\n  name:\n    type: message_id\n");
    assert!(matches!(cfg.output.name, NamingConfig::MessageId));
}

#[test]
fn naming_datetime_with_format() {
    let cfg = parse("output:\n  name:\n    type: datetime\n    format: \"%Y-%m-%d_%H-%M-%S\"\n");
    match cfg.output.name {
        NamingConfig::Datetime { format } => assert_eq!(format, "%Y-%m-%d_%H-%M-%S"),
        _ => panic!("expected Datetime variant"),
    }
}

#[test]
fn naming_datetime_forbidden_chars_rejected_by_validate() {
    // Parse succeeds; validate() is the gate. Format contains ':' which is forbidden on Windows.
    let cfg = parse("output:\n  name:\n    type: datetime\n    format: \"%H:%M:%S\"\n");
    assert!(cfg.output.name.validate().is_err());
}

#[test]
fn naming_datetime_safe_format_passes_validate() {
    let cfg = parse("output:\n  name:\n    type: datetime\n    format: \"%Y-%m-%dT%H-%M-%S\"\n");
    assert!(cfg.output.name.validate().is_ok());
}

// ---------- input.queue + bucket parsing ----------

#[test]
fn input_queue_bucket_parses_as_nested_block() {
    let yaml = "\
output: {}
input:
  driver: queue
  queue:
    backend: sqlite
    sqlite:
      path: /tmp/queue.db
    bucket:
      backend: fs
      fs:
        root: /tmp/buckets
";
    let cfg = parse(yaml);
    let input = cfg.input.expect("input block present");
    assert_eq!(input.driver, "queue");
    let q = input.queue.expect("queue block present");
    assert_eq!(q.backend, "sqlite");
    assert_eq!(q.sqlite.unwrap().path, PathBuf::from("/tmp/queue.db"));
    let b = q.bucket.expect("bucket block present");
    assert_eq!(b.backend, "fs");
    assert_eq!(b.fs.unwrap().root, PathBuf::from("/tmp/buckets"));
}

#[test]
fn input_queue_defaults_visibility_and_max_receive() {
    let yaml = "\
output: {}
input:
  driver: queue
  queue:
    backend: sqlite
    sqlite:
      path: /tmp/q.db
";
    let cfg = parse(yaml);
    let q = cfg.input.unwrap().queue.unwrap();
    assert_eq!(q.visibility_timeout_sec, 300);
    assert_eq!(q.max_receive, 23);
    // ModelLoopConfig defaults
    assert_eq!(q.model.loaded_loop_ms, 1000);
    assert_eq!(q.model.unloaded_loop_ms, 60_000);
    assert_eq!(q.model.unload_after_ms, 300_000);
}

#[test]
fn input_driver_unknown_value_parses_ok_validation_is_runtime() {
    // Parser does not gate on driver value; run_queue is responsible for rejecting.
    let yaml = "\
output: {}
input:
  driver: experimental-driver
";
    let cfg = parse(yaml);
    assert_eq!(cfg.input.unwrap().driver, "experimental-driver");
}

// ---------- --set overrides via dotted path ----------

#[test]
fn override_sets_leaf_value_in_existing_block() {
    let cfg = parse_with(
        "output:\n  file:\n    path: /yaml.md\n",
        &[("output.file.path", "/cli.md")],
    );
    assert_eq!(cfg.output.file.unwrap().path, PathBuf::from("/cli.md"));
}

#[test]
fn override_creates_missing_intermediate_blocks() {
    let cfg = parse_with(
        "output: {}\n",
        &[
            ("output.file.path", "/new.md"),
            ("output.file.overwrite", "true"),
        ],
    );
    let f = cfg
        .output
        .file
        .expect("file block was created via overrides");
    assert_eq!(f.path, PathBuf::from("/new.md"));
    assert!(f.overwrite);
}

#[test]
fn override_yaml_scalar_typing_is_parsed_not_stringified() {
    // "true" must become a bool, not a string, because overwrite: bool.
    let cfg = parse_with(
        "output:\n  file:\n    path: /x\n",
        &[("output.file.overwrite", "true")],
    );
    assert!(cfg.output.file.unwrap().overwrite);
}

#[test]
fn override_invalid_yaml_value_returns_error() {
    // Override value must parse as YAML scalar; raw `not: yaml: at all` fails.
    let result = Config::from_str(
        "output: {}\n",
        &[("output.file.path".to_string(), "{ unterminated".to_string())],
    );
    assert!(result.is_err());
}

// ---------- VadConfig defaults ----------

#[test]
fn vad_config_model_name_defaults_to_silero_vad_v6() {
    let cfg = parse("output: {}\naudio:\n  preprocess:\n    vad: {}\n");
    let vad = cfg.audio.preprocess.vad;
    assert_eq!(vad.model_name, "silero-vad-v6");
}

#[test]
fn vad_config_model_name_explicit_value_preserved() {
    let cfg = parse(
        "output: {}\naudio:\n  preprocess:\n    vad:\n      model_name: my-custom-vad\n",
    );
    let vad = cfg.audio.preprocess.vad;
    assert_eq!(vad.model_name, "my-custom-vad");
}

#[test]
fn vad_config_default_instance_has_model_name() {
    let v = VadConfig::default();
    assert_eq!(v.model_name, "silero-vad-v6");
}
