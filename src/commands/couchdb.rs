use std::path::PathBuf;

use crate::cli::CouchdbCommand;
use crate::config::Config;
use crate::{output, Result};

use super::resolve;

pub fn run(cmd: CouchdbCommand) -> Result<()> {
    match cmd {
        CouchdbCommand::Probe { config, limit, chunks } => probe(config, limit, chunks),
    }
}

fn probe(config: Option<PathBuf>, limit: usize, chunks: usize) -> Result<()> {
    let cfg_path = config.unwrap_or_else(|| PathBuf::from("config/config.yaml"));
    let cfg = Config::load(&cfg_path)?;
    let (cdb, pwd) = resolve::load_couchdb_config(&cfg)?;
    let state = output::couchdb::ensure_state(&cdb, &pwd, true, limit, chunks)?;
    println!("---");
    println!("schema    : {}", state.schema);
    println!("hash_algo : {}", state.hash_algo);
    println!("e2ee      : {}", state.e2ee);
    println!("obfuscated: {}", state.path_obfuscation);
    println!("cached at .cache/livesync.yaml");
    Ok(())
}
