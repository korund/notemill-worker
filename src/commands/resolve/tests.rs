//! Precedence rule under test: CLI > YAML > default.
//! Configs are built in-memory; the YAML parser is exercised separately.

use super::*;
use crate::config::{FileSinkConfig, ModelConfig, NamingConfig, OutputConfig, StdoutSinkConfig};

fn empty_config() -> Config {
    Config {
        model: None,
        output: OutputConfig {
            sink: None,
            couchdb: None,
            file: None,
            stdout: None,
            name: NamingConfig::default(),
        },
        input: None,
        audio: None,
    }
}

fn file_sink(path: &str, overwrite: bool, separator: Option<&str>) -> FileSinkConfig {
    FileSinkConfig {
        path: PathBuf::from(path),
        overwrite,
        separator: separator.map(String::from),
    }
}

fn couchdb_block(target: &str) -> CouchdbConfig {
    CouchdbConfig {
        url: "http://x".into(),
        database: "db".into(),
        username: "u".into(),
        password_env: None,
        password_file: None,
        target: target.into(),
    }
}

// `Sink` and `ModelFamily` do not derive PartialEq, so use `matches!`.

// ---------- sink() ----------

#[test]
fn sink_cli_wins_over_yaml_and_default() {
    let mut cfg = empty_config();
    cfg.output.sink = Some("couchdb".into());
    let s = sink(&cfg, Some(Sink::File), Sink::Stdout);
    assert!(matches!(s, Sink::File));
}

#[test]
fn sink_yaml_used_when_cli_absent() {
    let mut cfg = empty_config();
    cfg.output.sink = Some("couchdb".into());
    assert!(matches!(sink(&cfg, None, Sink::Stdout), Sink::Couchdb));
}

#[test]
fn sink_default_used_when_neither_set() {
    assert!(matches!(
        sink(&empty_config(), None, Sink::Stdout),
        Sink::Stdout
    ));
}

#[test]
fn sink_yaml_unknown_value_falls_through_to_default() {
    let mut cfg = empty_config();
    cfg.output.sink = Some("bogus".into());
    assert!(matches!(sink(&cfg, None, Sink::File), Sink::File));
}

// ---------- couchdb_target() ----------

#[test]
fn couchdb_target_cli_overrides_yaml() {
    let mut cfg = empty_config();
    cfg.output.couchdb = Some(couchdb_block("Inbox"));
    assert_eq!(
        couchdb_target(&cfg, Some("Custom".into())).unwrap(),
        "Custom"
    );
}

#[test]
fn couchdb_target_cli_empty_string_falls_back_to_yaml() {
    let mut cfg = empty_config();
    cfg.output.couchdb = Some(couchdb_block("Inbox"));
    assert_eq!(couchdb_target(&cfg, Some(String::new())).unwrap(), "Inbox");
}

#[test]
fn couchdb_target_yaml_used_when_no_cli() {
    let mut cfg = empty_config();
    cfg.output.couchdb = Some(couchdb_block("Notes"));
    assert_eq!(couchdb_target(&cfg, None).unwrap(), "Notes");
}

#[test]
fn couchdb_target_errors_when_no_block_and_no_cli() {
    assert!(couchdb_target(&empty_config(), None).is_err());
}

#[test]
fn couchdb_target_cli_works_without_block() {
    assert_eq!(
        couchdb_target(&empty_config(), Some("X".into())).unwrap(),
        "X"
    );
}

// ---------- file_path() ----------

#[test]
fn file_path_cli_overrides_yaml() {
    let mut cfg = empty_config();
    cfg.output.file = Some(file_sink("/yaml.md", false, None));
    assert_eq!(file_path(&cfg, Some("/cli.md".into())).unwrap(), "/cli.md");
}

#[test]
fn file_path_cli_empty_falls_back_to_yaml() {
    let mut cfg = empty_config();
    cfg.output.file = Some(file_sink("/yaml.md", false, None));
    assert_eq!(file_path(&cfg, Some(String::new())).unwrap(), "/yaml.md");
}

#[test]
fn file_path_yaml_used_when_no_cli() {
    let mut cfg = empty_config();
    cfg.output.file = Some(file_sink("/yaml.md", false, None));
    assert_eq!(file_path(&cfg, None).unwrap(), "/yaml.md");
}

#[test]
fn file_path_errors_when_neither_set() {
    assert!(file_path(&empty_config(), None).is_err());
}

// ---------- file_overwrite() ----------

#[test]
fn file_overwrite_cli_true_forces_true() {
    let mut cfg = empty_config();
    cfg.output.file = Some(file_sink("/x", false, None));
    assert!(file_overwrite(&cfg, true));
}

#[test]
fn file_overwrite_cli_false_yields_yaml_value() {
    let mut cfg = empty_config();
    cfg.output.file = Some(file_sink("/x", true, None));
    assert!(file_overwrite(&cfg, false));
}

#[test]
fn file_overwrite_default_false_when_no_yaml_block() {
    assert!(!file_overwrite(&empty_config(), false));
}

// ---------- file_separator() / stdout_separator() ----------

#[test]
fn file_separator_from_yaml_block() {
    let mut cfg = empty_config();
    cfg.output.file = Some(file_sink("/x", false, Some("===")));
    assert_eq!(file_separator(&cfg), Some("===".into()));
}

#[test]
fn file_separator_none_when_no_block() {
    assert_eq!(file_separator(&empty_config()), None);
}

#[test]
fn file_separator_none_when_block_disables_it() {
    let mut cfg = empty_config();
    cfg.output.file = Some(file_sink("/x", false, None));
    assert_eq!(file_separator(&cfg), None);
}

#[test]
fn stdout_separator_from_yaml_block() {
    let mut cfg = empty_config();
    cfg.output.stdout = Some(StdoutSinkConfig {
        separator: Some("###".into()),
    });
    assert_eq!(stdout_separator(&cfg), Some("###".into()));
}

#[test]
fn stdout_separator_none_when_no_block() {
    assert_eq!(stdout_separator(&empty_config()), None);
}

// ---------- models_dir() ----------

#[test]
fn models_dir_cli_wins() {
    let mut cfg = empty_config();
    cfg.model = Some(ModelConfig {
        name: None,
        family: None,
        dir: Some(PathBuf::from("/yaml/models")),
    });
    assert_eq!(
        models_dir(&cfg, Some(PathBuf::from("/cli/models"))),
        PathBuf::from("/cli/models")
    );
}

#[test]
fn models_dir_yaml_used_when_no_cli() {
    let mut cfg = empty_config();
    cfg.model = Some(ModelConfig {
        name: None,
        family: None,
        dir: Some(PathBuf::from("/yaml/models")),
    });
    assert_eq!(models_dir(&cfg, None), PathBuf::from("/yaml/models"));
}

#[test]
fn models_dir_default_when_neither_set() {
    assert_eq!(models_dir(&empty_config(), None), PathBuf::from("models"));
}

// ---------- model_name() ----------

#[test]
fn model_name_cli_wins() {
    let mut cfg = empty_config();
    cfg.model = Some(ModelConfig {
        name: Some("yaml-model".into()),
        family: None,
        dir: None,
    });
    assert_eq!(
        model_name(&cfg, Some("cli-model".into())).unwrap(),
        "cli-model"
    );
}

#[test]
fn model_name_yaml_used_when_no_cli() {
    let mut cfg = empty_config();
    cfg.model = Some(ModelConfig {
        name: Some("yaml-model".into()),
        family: None,
        dir: None,
    });
    assert_eq!(model_name(&cfg, None).unwrap(), "yaml-model");
}

#[test]
fn model_name_errors_when_neither_set() {
    assert!(model_name(&empty_config(), None).is_err());
}

// ---------- family() ----------

#[test]
fn family_cli_wins_over_yaml() {
    let mut cfg = empty_config();
    cfg.model = Some(ModelConfig {
        name: None,
        family: Some("whisper".into()),
        dir: None,
    });
    let f = family(&cfg, Some(FamilyArg::Parakeet)).unwrap();
    assert!(matches!(f, Some(ModelFamily::Parakeet)));
}

#[test]
fn family_yaml_used_when_no_cli() {
    let mut cfg = empty_config();
    cfg.model = Some(ModelConfig {
        name: None,
        family: Some("giga-am".into()),
        dir: None,
    });
    let f = family(&cfg, None).unwrap();
    assert!(matches!(f, Some(ModelFamily::GigaAm)));
}

#[test]
fn family_none_when_neither_set() {
    assert!(family(&empty_config(), None).unwrap().is_none());
}

#[test]
fn family_yaml_unknown_value_errors() {
    let mut cfg = empty_config();
    cfg.model = Some(ModelConfig {
        name: None,
        family: Some("bogus".into()),
        dir: None,
    });
    assert!(family(&cfg, None).is_err());
}

// ---------- parse_family_str() ----------

#[test]
fn parse_family_known_values() {
    assert!(matches!(
        parse_family_str("whisper").unwrap(),
        ModelFamily::Whisper
    ));
    assert!(matches!(
        parse_family_str("parakeet").unwrap(),
        ModelFamily::Parakeet
    ));
    assert!(matches!(
        parse_family_str("giga-am").unwrap(),
        ModelFamily::GigaAm
    ));
}

#[test]
fn parse_family_unknown_errors() {
    assert!(parse_family_str("Whisper").is_err()); // case-sensitive
    assert!(parse_family_str("").is_err());
    assert!(parse_family_str("gigaam").is_err());
}
