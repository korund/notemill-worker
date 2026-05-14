//! `worker admin ...` -- diagnostics and maintenance commands.

use crate::cli::AdminCommand;
use crate::Result;

mod couchdb;
mod models;
mod queue;

pub fn run(cmd: AdminCommand) -> Result<()> {
    match cmd {
        AdminCommand::Couchdb { cmd } => couchdb::run(cmd),
        AdminCommand::Queue { cmd } => queue::run(cmd),
        AdminCommand::Models { dir, cmd } => models::run(dir, cmd),
    }
}
