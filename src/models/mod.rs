//! Model manager.
//!
//! Analogous to the handy UI model picker: a built-in catalog of known models
//! (name, URL, sha256, size, engine family). Two usage modes:
//!   1) `models pull <name>` -- download a model into the local directory.
//!   2) Manual file placement in the same directory -- `run --model <name>` picks
//!      it up without network, checking name and (optionally) sha256.
//!
//! The models directory is set via `--models-dir` or `$NOTES_CAPTURE_MODELS_DIR`,
//! defaulting to `./models`.

mod catalog;
mod manager;

pub use catalog::{Catalog, CatalogEntry, ModelFamily};
pub use manager::{Manager, ResolvedModel};
