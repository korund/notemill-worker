//! CLI command handlers. Each submodule owns one top-level command path.
//!
//! `dispatch` is the single entry point used by `main`.

use crate::cli::{Cli, Command, RunCommand};
use crate::Result;

mod couchdb;
mod decode;
mod models;
mod resolve;
mod run_file;
mod run_queue;

pub fn dispatch(cli: Cli) -> Result<()> {
    match cli.command {
        Command::Models { dir, cmd } => models::run(dir, cmd),
        Command::Couchdb { cmd } => couchdb::run(cmd),
        Command::Decode { input, output } => decode::run(input, output),
        Command::Run { cmd } => match cmd {
            RunCommand::File { common, input, frontmatter } => {
                run_file::run(common, input, frontmatter)
            }
            RunCommand::Queue { common } => run_queue::run(common),
        },
    }
}
