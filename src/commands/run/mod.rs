//! `worker run ...` -- transcribe from a file or from a queue.

use crate::cli::RunCommand;
use crate::Result;

mod file;
mod queue;

pub fn run(cmd: RunCommand) -> Result<()> {
    match cmd {
        RunCommand::File {
            common,
            input,
            frontmatter,
        } => file::run(common, input, frontmatter),
        RunCommand::Queue { common } => queue::run(common),
    }
}
