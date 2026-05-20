use clap::Parser;
use notemill_worker::cli::Cli;
use notemill_worker::{commands, Result};
use tracing_subscriber::EnvFilter;

fn main() -> Result<()> {
    init_tracing();
    init_ort();
    commands::dispatch(Cli::parse())
}

/// Initialize the global ORT environment up-front so that every Session
/// created later in the process (transcribe-rs engines and the silero VAD
/// segmenter) shares thread pools and allocators.
#[cfg(feature = "engine-transcribe")]
fn init_ort() {
    let _ = ort::init().commit();
}

#[cfg(not(feature = "engine-transcribe"))]
fn init_ort() {}

fn init_tracing() {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")))
        .with_writer(std::io::stderr)
        .init();
}
