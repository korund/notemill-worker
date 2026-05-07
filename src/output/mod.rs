//! Output sinks.
//!
//! Currently stdout or a text file. Others may be added later.
//! All implementations are behind a common trait.

mod file;
pub mod couchdb;
mod stdout;

pub use file::FileSink;
pub use stdout::StdoutSink;
pub use couchdb::CouchdbSink;

use crate::Result;

pub trait OutputSink {
    fn write(&mut self, text: &str) -> Result<()>;
}
