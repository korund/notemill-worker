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
    /// Model management: list / pull / add.
    Models {
        #[command(subcommand)]
        cmd: ModelsCommand,
    },

    /// Decode an audio file to PCM 16 kHz mono and print stats (no engine needed).
    #[command(hide = true)]
    Decode {
        /// Path to the input audio file.
        #[arg(long)]
        input: PathBuf,

        /// Write raw f32 PCM to this file instead of just printing stats.
        #[arg(long)]
        output: Option<PathBuf>,
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
    /// Show the catalog and files physically present in the models directory.
    List,
    /// Download a model by name from the catalog into the models directory.
    Pull {
        /// Model name (see `models list`).
        name: String,
    },
    /// Add a model by URL: download, compute sha256/size, register in catalog.
    Add {
        /// Download URL of the model file or .tar.gz archive.
        url: String,

        /// Engine family (whisper, parakeet, giga-am).
        #[arg(long, value_enum)]
        family: FamilyArg,

        /// Override the auto-derived model name.
        #[arg(long)]
        name: Option<String>,
    },
}
