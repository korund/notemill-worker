use clap::Parser;
use std::path::PathBuf;
use tracing_subscriber::EnvFilter;
use voice2text::cli::{Cli, Command, CouchdbCommand, ModelsCommand, OutputKind};
use voice2text::input::queue::backends::fs::FsBlobStore;
use voice2text::input::queue::backends::sqlite::SqliteBackend;
use voice2text::input::queue::job::{NotifyResult, TranscribeJob};
use voice2text::input::queue::{JobProcessor, QueueDriver, QueueDriverConfig};
use voice2text::input::{AudioSource, InputDriver};
use voice2text::output::OutputSink;
use voice2text::{decode, engine, input, models, output, Error, Result};

/// Pipeline glue used by the queue daemon. One instance per worker, reused
/// across jobs (model stays in RAM). Each call:
/// 1. decode -> 16 kHz mono f32 PCM
/// 2. engine.transcribe -> text
/// 3. write to a per-job CouchDB doc whose path is `path_prefix/<dedup_key>`
/// 4. return the `couchdb://...` reference for the NotifyResult.
struct DaemonProcessor {
    decoder: Box<dyn decode::AudioDecoder>,
    transcriber: Box<dyn engine::Transcriber>,
    cdb: voice2text::config::CouchdbConfig,
    pwd: String,
    path_prefix: String,
}

fn parse_family_str(s: &str) -> Result<voice2text::models::ModelFamily> {
    match s {
        "whisper" => Ok(voice2text::models::ModelFamily::Whisper),
        "parakeet" => Ok(voice2text::models::ModelFamily::Parakeet),
        "giga-am" => Ok(voice2text::models::ModelFamily::GigaAm),
        other => Err(Error::Config(format!("unknown family: {other:?}"))),
    }
}

impl JobProcessor for DaemonProcessor {
    fn process(&mut self, source: &dyn AudioSource, job: &TranscribeJob) -> Result<String> {
        let raw = source.read()?;
        let pcm = self.decoder.decode(&raw)?;
        let text = self.transcriber.transcribe(&pcm)?;
        // Frontmatter is sourced via the bot/queue (job hints), not configured
        // on the daemon -- daemon mode keeps the body as the engine produced it.
        let body = text;
        // Sanitize dedup_key for use as a path segment ("tg:1:2" -> "tg-1-2").
        let safe = job.dedup_key.replace(':', "-");
        let path = format!("{}/{}", self.path_prefix.trim_end_matches('/'), safe);
        let mut sink = output::CouchdbSink::new(self.cdb.clone(), self.pwd.clone(), path.clone());
        sink.write(&body)?;
        Ok(format!("couchdb://{path}"))
    }
}

fn load_couchdb_config(
    config: Option<PathBuf>,
) -> Result<(voice2text::config::CouchdbConfig, String)> {
    let cfg_path = config.unwrap_or_else(|| PathBuf::from("config/config.yaml"));
    let cfg = voice2text::config::Config::load(&cfg_path)?;
    let cdb = cfg
        .output
        .couchdb
        .ok_or_else(|| voice2text::Error::Config("output.couchdb section missing".into()))?;
    let pwd = cdb.resolve_password()?.ok_or_else(|| {
        voice2text::Error::Config(
            "password not configured (set password_env or password_file)".into(),
        )
    })?;
    Ok((cdb, pwd))
}

fn build_run_sink(
    kind: OutputKind,
    path: Option<String>,
) -> Result<Box<dyn voice2text::output::OutputSink>> {
    match kind {
        OutputKind::Stdout => {
            if path.is_some() {
                return Err(voice2text::Error::Config(
                    "--path must not be set when --output stdout".into(),
                ));
            }
            Ok(Box::new(voice2text::output::StdoutSink::new()))
        }
        OutputKind::File => {
            let p = path.ok_or_else(|| {
                voice2text::Error::Config("--path is required for --output file".into())
            })?;
            Ok(Box::new(voice2text::output::FileSink::new(PathBuf::from(p))))
        }
        OutputKind::Couchdb => {
            let p = path.ok_or_else(|| {
                voice2text::Error::Config("--path is required for --output couchdb".into())
            })?;
            let (cdb, pwd) = load_couchdb_config(None)?;
            let _state = voice2text::output::couchdb::ensure_state(
                &cdb,
                &pwd,
                false,
                voice2text::output::couchdb::DEFAULT_PROBE_LIMIT,
                voice2text::output::couchdb::DEFAULT_PROBE_CHUNKS,
            )?;
            Ok(Box::new(voice2text::output::CouchdbSink::new(cdb, pwd, p)))
        }
    }
}

fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")))
        .with_writer(std::io::stderr)
        .init();

    let cli = Cli::parse();

    let models_dir = cli.models_dir();
    let catalog = models::Catalog::load()?;
    let manager = models::Manager::new(models_dir, catalog);

    match cli.command {
        Command::Models { cmd } => match cmd {
            ModelsCommand::List => {
                manager.print_list();
                Ok(())
            }
            ModelsCommand::Pull { name } => manager.pull(&name),
            ModelsCommand::Add { url, family, name } => {
                manager.add(&url, family.into(), name.as_deref())
            }
        },
        Command::Couchdb { cmd } => match cmd {
            CouchdbCommand::Probe { config, limit, chunks } => {
                let (cdb, pwd) = load_couchdb_config(config)?;
                let state = output::couchdb::ensure_state(&cdb, &pwd, true, limit, chunks)?;
                println!("---");
                println!("schema    : {}", state.schema);
                println!("hash_algo : {}", state.hash_algo);
                println!("e2ee      : {}", state.e2ee);
                println!("obfuscated: {}", state.path_obfuscation);
                println!("cached at config/.cache/livesync.yaml");
                Ok(())
            }
        },
        Command::Decode {
            input: input_path,
            output: output_path,
        } => {
            let source = input::LocalFileSource::new(input_path);
            let decoder = decode::DefaultDecoder::new();

            let raw = <input::LocalFileSource as input::AudioSource>::read(&source)?;
            let pcm = <decode::DefaultDecoder as decode::AudioDecoder>::decode(&decoder, &raw)?;

            let duration_secs = pcm.samples.len() as f64 / decode::TARGET_SAMPLE_RATE as f64;
            let (min, max) = pcm.samples.iter().fold((f32::MAX, f32::MIN), |(lo, hi), &s| {
                (lo.min(s), hi.max(s))
            });
            let rms = (pcm.samples.iter().map(|s| s * s).sum::<f32>() / pcm.samples.len() as f32).sqrt();

            println!("Samples : {}", pcm.samples.len());
            println!("Rate    : {} Hz", decode::TARGET_SAMPLE_RATE);
            println!("Duration: {:.3} s", duration_secs);
            println!("Range   : [{:.4}, {:.4}]", min, max);
            println!("RMS     : {:.4}", rms);

            if let Some(path) = output_path {
                let bytes: Vec<u8> = pcm
                    .samples
                    .iter()
                    .flat_map(|s| s.to_le_bytes())
                    .collect();
                std::fs::write(&path, &bytes)
                    .map_err(|e| voice2text::Error::Output(format!("write {}: {e}", path.display())))?;
                println!("PCM f32 written to {}", path.display());
            }

            Ok(())
        }
        Command::Daemon { config } => {
            let cfg_path = config.unwrap_or_else(|| PathBuf::from("config/config.yaml"));
            let cfg = voice2text::config::Config::load(&cfg_path)?;
            let dcfg = cfg
                .daemon
                .as_ref()
                .ok_or_else(|| Error::Config("daemon section missing".into()))?;
            let model = dcfg.model.clone();
            let family = dcfg
                .family
                .as_deref()
                .map(parse_family_str)
                .transpose()?;
            let path_prefix = dcfg.path_prefix.clone();
            let input_cfg = cfg
                .input
                .as_ref()
                .ok_or_else(|| Error::Config("input section missing".into()))?;
            if input_cfg.mode != "queue" {
                return Err(Error::Config(format!(
                    "input.mode={:?}; only \"queue\" is supported",
                    input_cfg.mode
                )));
            }
            let qcfg = input_cfg
                .queue
                .as_ref()
                .ok_or_else(|| Error::Config("input.queue section missing".into()))?;
            let bcfg = input_cfg
                .blob
                .as_ref()
                .ok_or_else(|| Error::Config("input.blob section missing".into()))?;
            if qcfg.backend != "sqlite" {
                return Err(Error::Config(format!(
                    "queue.backend={:?}; only \"sqlite\" is supported in this build",
                    qcfg.backend
                )));
            }
            if bcfg.backend != "fs" {
                return Err(Error::Config(format!(
                    "blob.backend={:?}; only \"fs\" is supported in this build",
                    bcfg.backend
                )));
            }
            let sqlite_path = qcfg
                .sqlite
                .as_ref()
                .ok_or_else(|| Error::Config("input.queue.sqlite missing".into()))?
                .path
                .clone();
            let blob_root = bcfg
                .fs
                .as_ref()
                .ok_or_else(|| Error::Config("input.blob.fs missing".into()))?
                .root
                .clone();

            let sqlite = SqliteBackend::open(&sqlite_path)?;
            let transcribe_q: voice2text::input::queue::backends::sqlite::SqliteQueue<
                TranscribeJob,
            > = sqlite.queue("transcribe", qcfg.max_receive)?;
            let notify_q: voice2text::input::queue::backends::sqlite::SqliteQueue<NotifyResult> =
                sqlite.queue("notifications", qcfg.max_receive)?;
            let processed = sqlite.processed_store();
            let blob = FsBlobStore::open(&blob_root)?;

            let (cdb, pwd) = load_couchdb_config(Some(cfg_path.clone()))?;
            let _state = output::couchdb::ensure_state(
                &cdb,
                &pwd,
                false,
                output::couchdb::DEFAULT_PROBE_LIMIT,
                output::couchdb::DEFAULT_PROBE_CHUNKS,
            )?;

            // TODO(deferred): lazy-load model on first job + idle TTL eviction.
            // Today the model is loaded eagerly here and held until shutdown.
            let model_handle = manager.resolve(&model, family)?;
            let processor = DaemonProcessor {
                decoder: Box::new(decode::DefaultDecoder::new()),
                transcriber: engine::build(&model_handle)?,
                cdb,
                pwd,
                path_prefix,
            };

            let driver_cfg = QueueDriverConfig {
                visibility_sec: qcfg.visibility_timeout_sec,
                poll_interval: std::time::Duration::from_millis(qcfg.poll_interval_ms),
            };
            let mut driver =
                QueueDriver::new(transcribe_q, notify_q, blob, processed, processor, driver_cfg);
            driver.run()
        }
        Command::Run {
            model,
            family,
            input: input_path,
            output,
            path,
            frontmatter,
        } => {
            let model_handle = manager.resolve(&model, family.map(Into::into))?;

            let mut sink: Box<dyn output::OutputSink> = build_run_sink(output, path)?;

            let fm = frontmatter
                .as_deref()
                .filter(|s| !s.is_empty())
                .and_then(voice2text::output::frontmatter::render_from_spec);

            let source: Box<dyn input::AudioSource> =
                Box::new(input::LocalFileSource::new(input_path));
            let decoder: Box<dyn decode::AudioDecoder> = Box::new(decode::DefaultDecoder::new());
            let mut transcriber: Box<dyn engine::Transcriber> =
                engine::build(&model_handle)?;

            let raw = source.read()?;
            let pcm = decoder.decode(&raw)?;
            let text = transcriber.transcribe(&pcm)?;
            let body = match fm {
                Some(prefix) => format!("{}{}", prefix, text),
                None => text,
            };
            sink.write(&body)?;
            Ok(())
        }
    }
}
