//! Output sinks.
//!
//! Currently stdout or a text file. Others may be added later.
//! All implementations are behind a common trait.

mod concat;
pub mod couchdb;
mod file;
pub mod frontmatter;
mod stdout;

pub use couchdb::CouchdbSink;
pub use file::FileSink;
pub use stdout::StdoutSink;

use crate::Result;

pub trait OutputSink {
    fn write(&mut self, text: &str) -> Result<()>;
}
