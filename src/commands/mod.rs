//! CLI command handlers. Each submodule owns one top-level command path.
//!
//! `dispatch` is the single entry point used by `main`.

use crate::cli::{Cli, Command};
use crate::Result;

mod admin;
mod decode;
mod resolve;
mod run;

pub fn dispatch(cli: Cli) -> Result<()> {
    match cli.command {
        Command::Run { cmd } => run::run(cmd),
        Command::Admin { cmd } => admin::run(cmd),
        Command::Decode { input, output } => decode::run(input, output),
    }
}
