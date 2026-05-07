//! Bridge between `Config` + CLI flags and the concrete values used by commands.
//!
//! All "CLI overrides YAML" precedence rules live here so command handlers
//! stay focused on orchestration.

use std::path::PathBuf;

use crate::cli::{FamilyArg, Sink};
use crate::config::{Config, CouchdbConfig};
use crate::models::ModelFamily;
use crate::{Error, Result};

pub fn parse_family_str(s: &str) -> Result<ModelFamily> {
    match s {
        "whisper" => Ok(ModelFamily::Whisper),
        "parakeet" => Ok(ModelFamily::Parakeet),
        "giga-am" => Ok(ModelFamily::GigaAm),
        other => Err(Error::Config(format!("unknown family: {other:?}"))),
    }
}

pub fn load_couchdb_config(cfg: &Config) -> Result<(CouchdbConfig, String)> {
    let cdb = cfg
        .output
        .couchdb
        .clone()
        .ok_or_else(|| Error::Config("output.couchdb section missing".into()))?;
    let pwd = cdb.resolve_password()?.ok_or_else(|| {
        Error::Config("password not configured (set password_env or password_file)".into())
    })?;
    Ok((cdb, pwd))
}

pub fn models_dir(cfg: &Config, cli_flag: Option<PathBuf>) -> PathBuf {
    cli_flag
        .or_else(|| cfg.model.as_ref().and_then(|m| m.dir.clone()))
        .unwrap_or_else(|| PathBuf::from("models"))
}

pub fn model_name(cfg: &Config, cli_flag: Option<String>) -> Result<String> {
    cli_flag
        .or_else(|| cfg.model.as_ref().and_then(|m| m.name.clone()))
        .ok_or_else(|| {
            Error::Config("model not set; use --model-name or set `model.name:` in config".into())
        })
}

pub fn family(cfg: &Config, cli_flag: Option<FamilyArg>) -> Result<Option<ModelFamily>> {
    if let Some(f) = cli_flag {
        return Ok(Some(f.into()));
    }
    cfg.model
        .as_ref()
        .and_then(|m| m.family.as_deref())
        .map(parse_family_str)
        .transpose()
}

/// Resolve the active sink kind: CLI flag wins, else `output.sink`, else `default`.
pub fn sink(cfg: &Config, cli: Option<Sink>, default: Sink) -> Sink {
    cli.or_else(|| {
        cfg.output.sink.as_deref().and_then(|s| match s {
            "stdout" => Some(Sink::Stdout),
            "file" => Some(Sink::File),
            "couchdb" => Some(Sink::Couchdb),
            _ => None,
        })
    })
    .unwrap_or(default)
}

/// CouchDB doc-path prefix: CLI `--target` wins, else `output.couchdb.target`.
/// `output.couchdb.target` always has a default ("Inbox") via serde, so this can
/// only fail when there is no `output.couchdb` block at all.
pub fn couchdb_target(cfg: &Config, cli: Option<String>) -> Result<String> {
    cli.filter(|s| !s.is_empty())
        .or_else(|| cfg.output.couchdb.as_ref().map(|c| c.target.clone()))
        .ok_or_else(|| {
            Error::Config(
                "couchdb sink requires output.couchdb section in config (or --target on CLI)"
                    .into(),
            )
        })
}

/// File sink path: CLI `--target` wins, else `output.file.path`.
pub fn file_path(cfg: &Config, cli: Option<String>) -> Result<String> {
    cli.filter(|s| !s.is_empty())
        .or_else(|| {
            cfg.output
                .file
                .as_ref()
                .map(|f| f.path.to_string_lossy().into_owned())
        })
        .ok_or_else(|| {
            Error::Config("file sink requires --target or output.file.path in config".into())
        })
}

/// File sink overwrite flag: CLI `--overwrite` forces true, else `output.file.overwrite`.
pub fn file_overwrite(cfg: &Config, cli: bool) -> bool {
    cli || cfg.output.file.as_ref().map(|f| f.overwrite).unwrap_or(false)
}

/// File sink separator marker, taken from `output.file.separator`.
/// `None` means no separator is inserted between writes.
pub fn file_separator(cfg: &Config) -> Option<String> {
    cfg.output.file.as_ref().and_then(|f| f.separator.clone())
}

/// Stdout sink separator marker, taken from `output.stdout.separator`.
/// `None` means no separator is inserted between writes.
pub fn stdout_separator(cfg: &Config) -> Option<String> {
    cfg.output.stdout.as_ref().and_then(|s| s.separator.clone())
}
