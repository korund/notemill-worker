use clap::Parser;
use tracing_subscriber::EnvFilter;
use voice2text::cli::Cli;
use voice2text::{commands, Result};

fn main() -> Result<()> {
    init_tracing();
    commands::dispatch(Cli::parse())
}

fn init_tracing() {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")))
        .with_writer(std::io::stderr)
        .init();
}
