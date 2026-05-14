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
#[command(name = env!("CARGO_PKG_NAME"), about = env!("CARGO_PKG_DESCRIPTION"))]
pub struct Cli {
    #[command(subcommand)]
    pub command: Command,
}

#[derive(Debug, Subcommand)]
pub enum Command {
    /// Transcribe from a file or a queue.
    Run {
        #[command(subcommand)]
        cmd: RunCommand,
    },

    /// Diagnostics and maintenance commands.
    Admin {
        #[command(subcommand)]
        cmd: AdminCommand,
    },

    /// Decode an audio file to PCM 16 kHz mono and print stats (no engine needed).
    #[command(hide = true)]
    Decode {
        #[arg(long)]
        input: PathBuf,
        #[arg(long)]
        output: Option<PathBuf>,
    },
}

#[derive(Debug, Subcommand)]
pub enum AdminCommand {
    /// CouchDB output diagnostics and operations.
    Couchdb {
        #[command(subcommand)]
        cmd: CouchdbCommand,
    },

    /// Queue maintenance: DLQ inspection and requeue.
    Queue {
        #[command(subcommand)]
        cmd: QueueCommand,
    },

    /// Model management: list / pull / add.
    Models {
        /// Models directory. Default: ./models.
        #[arg(long)]
        dir: Option<PathBuf>,
        #[command(subcommand)]
        cmd: ModelsCommand,
    },
}

/// Flags shared by `run file` and `run queue`.
#[derive(Debug, clap::Args)]
pub struct CommonRunArgs {
    /// Path to config file.
    #[arg(long, default_value = "config/config.yaml")]
    pub config: PathBuf,

    /// Model name from catalog or path to model file. Overrides YAML `model.name`.
    #[arg(long)]
    pub model_name: Option<String>,

    /// Engine family. Required only when --model-name is a direct path. Overrides YAML `model.family`.
    #[arg(long, value_enum)]
    pub model_family: Option<FamilyArg>,

    /// Models directory. Overrides YAML `model.dir`.
    #[arg(long)]
    pub model_dir: Option<PathBuf>,

    /// Output sink.
    #[arg(long, value_enum)]
    pub output: Option<Sink>,

    /// Output target path. For file sink: filesystem path. For couchdb sink: doc path / prefix.
    /// Forbidden for stdout.
    #[arg(long)]
    pub target: Option<String>,

    /// Truncate the output file instead of appending. FileSink only.
    #[arg(long)]
    pub overwrite: bool,

    /// Override a YAML config key. Format: key=value (dotted keys, YAML scalars). Repeatable.
    #[arg(long = "set", value_name = "KEY=VALUE")]
    pub set_overrides: Vec<String>,
}

impl CommonRunArgs {
    /// Parse --set KEY=VALUE pairs into (key, value) tuples.
    pub fn parsed_set_overrides(&self) -> Result<Vec<(String, String)>, String> {
        self.set_overrides
            .iter()
            .map(|s| {
                let (k, v) = s
                    .split_once('=')
                    .ok_or_else(|| format!("--set {s:?}: expected KEY=VALUE"))?;
                Ok((k.to_string(), v.to_string()))
            })
            .collect()
    }
}

#[derive(Debug, Subcommand)]
pub enum RunCommand {
    /// Transcribe a single audio file.
    File {
        #[command(flatten)]
        common: CommonRunArgs,

        /// Path to the input audio file.
        input: PathBuf,

        /// Prepend YAML frontmatter. Format: "key1: value1, key2: value2".
        #[arg(long)]
        frontmatter: Option<String>,
    },

    /// Run the queue-driven worker. Configured via config.yaml and --set.
    Queue {
        #[command(flatten)]
        common: CommonRunArgs,
    },
}

#[derive(Debug, Clone, Copy, ValueEnum)]
pub enum Sink {
    Stdout,
    File,
    Couchdb,
}

#[derive(Debug, Subcommand)]
pub enum ModelsCommand {
    /// Show the catalog and files physically present in the models directory.
    List,
    /// Download a model by name from the catalog into the models directory.
    Pull {
        name: String,
    },
    /// Add a model by URL: download, compute sha256/size, register in catalog.
    Add {
        url: String,
        #[arg(long, value_enum)]
        family: FamilyArg,
        #[arg(long)]
        name: Option<String>,
    },
}

#[derive(Debug, Subcommand)]
pub enum CouchdbCommand {
    /// Probe the configured database: print metadata and a few sample documents.
    Probe {
        #[arg(long)]
        config: Option<PathBuf>,
        #[arg(long, default_value_t = 10)]
        limit: usize,
        #[arg(long, default_value_t = 3)]
        chunks: usize,
    },
}

#[derive(Debug, Subcommand)]
pub enum QueueCommand {
    /// Dead-letter queue operations.
    Dlq {
        #[command(subcommand)]
        cmd: DlqCommand,
    },
}

#[derive(Debug, Clone, Copy, ValueEnum, Default)]
pub enum QueueName {
    #[default]
    Transcribe,
    Notifications,
}

impl QueueName {
    pub fn as_str(self) -> &'static str {
        match self {
            QueueName::Transcribe => "transcribe",
            QueueName::Notifications => "notifications",
        }
    }
}

#[derive(Debug, Subcommand)]
pub enum DlqCommand {
    /// List rows currently in the DLQ.
    List {
        #[arg(long, default_value = "config/config.yaml")]
        config: PathBuf,
        #[arg(long, value_enum, default_value_t = QueueName::Transcribe)]
        queue: QueueName,
    },
    /// Move a DLQ row back to the main queue. Resets receive_count to 0.
    Requeue {
        #[arg(long, default_value = "config/config.yaml")]
        config: PathBuf,
        #[arg(long, value_enum, default_value_t = QueueName::Transcribe)]
        queue: QueueName,
        /// DLQ row id (see `dlq list`).
        id: i64,
    },
}
