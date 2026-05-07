//! Modular CLI on top of transcribe-rs.
//!
//! Layers are designed as traits so future input sources, output sinks,
//! and (if needed) alternative engines can be plugged in without touching the rest.

pub mod cli;
pub mod config;
pub mod decode;
pub mod engine;
pub mod error;
pub mod input;
pub mod models;
pub mod output;
pub mod state;

pub use error::{Error, Result};
