use clap::{Parser, Subcommand, ValueEnum};
use std::path::PathBuf;

use crate::models::ModelFamily;

#[derive(Debug, Clone, Copy, ValueEnum)]
pub enum FamilyArg {
    Whisper,
    Parakeet,
    GigaAm,
}

impl From<FamilyArg> for ModelFamily {
    fn from(v: FamilyArg) -> Self {
        match v {
            FamilyArg::Whisper => ModelFamily::Whisper,
            FamilyArg::Parakeet => ModelFamily::Parakeet,
            FamilyArg::GigaAm => ModelFamily::GigaAm,
        }
    }
}

#[derive(Debug, Parser)]
#[command(name = "notes-capture", about = "Audio file transcription via transcribe-rs (Whisper / Parakeet / GigaAM, CPU)")]
pub struct Cli {
    /// Models directory. Defaults to ./models, overridden by $NOTES_CAPTURE_MODELS_DIR.
    #[arg(long, global = true, env = "NOTES_CAPTURE_MODELS_DIR")]
    pub models_dir: Option<PathBuf>,

    #[command(subcommand)]
    pub command: Command,
}

impl Cli {
    pub fn models_dir(&self) -> PathBuf {
        self.models_dir
            .clone()
            .unwrap_or_else(|| PathBuf::from("models"))
    }
}

#[derive(Debug, Subcommand)]
pub enum Command {
    /// Model management: list / pull.
    Models {
        #[command(subcommand)]
        cmd: ModelsCommand,
    },

    /// Transcribe a single audio file with the chosen model.
    Run {
        /// Model name from the built-in catalog OR path to a model file.
        #[arg(long)]
        model: String,

        /// Engine family for a direct path (`--model <path>`). Not required and ignored
        /// when a catalog name is used.
        #[arg(long, value_enum)]
        family: Option<FamilyArg>,

        /// Path to the input audio file.
        #[arg(long)]
        input: PathBuf,

        /// Path to the output file. Omit to write to stdout.
        #[arg(long)]
        output: Option<PathBuf>,
    },
}

#[derive(Debug, Subcommand)]
pub enum ModelsCommand {
    /// Show the built-in catalog and files physically present in the models directory.
    List,
    /// Download a model by name from the built-in catalog into the models directory.
    Pull {
        /// Model name (see `models list`).
        name: String,
    },
}
