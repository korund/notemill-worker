use std::path::PathBuf;

use chrono::DateTime;

use crate::cli::{CommonRunArgs, Sink};
use crate::config::{Config, NamingConfig};
use crate::input::queue::backends::fs::FsBucket;
use crate::input::queue::backends::sqlite::SqliteBackend;
use crate::input::queue::job::{NoSpeechReason, NotifyResult, TranscribeJob};
use crate::input::queue::{JobProcessor, ProcessOutcome, QueueDriver, QueueDriverConfig};
use crate::pipeline::RunOutcome;
use crate::input::{AudioSource, InputDriver};
use crate::pipeline::{ModelGuard, Pipeline};
use crate::{decode, engine, models, output, Error, Result};
use models::{ModelRegistry, ModelStatus};

use crate::commands::resolve;

enum QueueSink {
    /// Shared persistent sink (file or stdout). The sink itself handles any
    /// inter-job separator (see `output::FileSink::with_separator`).
    Shared {
        sink: Box<dyn output::OutputSink>,
        output_ref: String,
    },
    /// Per-job CouchDB sink; path derived from job metadata per NamingConfig.
    Couchdb {
        cdb: crate::config::CouchdbConfig,
        pwd: String,
        prefix: String,
        naming: NamingConfig,
    },
}

/// Derive the note file stem (no extension) from a job according to naming config.
fn note_stem(job: &TranscribeJob, naming: &NamingConfig) -> String {
    match naming {
        NamingConfig::MessageId => job.dedup_key.replace(':', "-"),
        NamingConfig::Datetime { format } => DateTime::parse_from_rfc3339(&job.source.received_at)
            .map(|dt| dt.format(format).to_string())
            .unwrap_or_else(|_| job.dedup_key.replace(':', "-")),
    }
}

/// Find a collision-free `prefix/stem[-N].md` path.
/// `exists` is called with the lowercase candidate id; returns true if taken.
fn collision_free_path(
    prefix: &str,
    stem: &str,
    exists: impl Fn(&str) -> Result<bool>,
) -> Result<String> {
    let candidate = output::couchdb::doc_path(prefix, stem);
    if !exists(&candidate.to_lowercase())? {
        return Ok(candidate);
    }
    let mut n = 1u32;
    loop {
        let c = output::couchdb::doc_path(prefix, &format!("{stem}-{n}"));
        if !exists(&c.to_lowercase())? {
            return Ok(c);
        }
        n += 1;
    }
}

struct QueueProcessor {
    queue_sink: QueueSink,
}

impl JobProcessor for QueueProcessor {
    fn process(
        &mut self,
        pipeline: &mut Pipeline,
        source: &dyn AudioSource,
        job: &TranscribeJob,
    ) -> Result<ProcessOutcome> {
        match &mut self.queue_sink {
            QueueSink::Shared { sink, output_ref } => {
                match pipeline.run_one(source, sink.as_mut(), None)? {
                    RunOutcome::Written => Ok(ProcessOutcome::Written(output_ref.clone())),
                    // Pipeline only produces NoSpeech for Speech::None today;
                    // hard-code Silent here until more reasons surface.
                    RunOutcome::NoSpeech => Ok(ProcessOutcome::NoSpeech(NoSpeechReason::Silent)),
                }
            }
            QueueSink::Couchdb {
                cdb,
                pwd,
                prefix,
                naming,
            } => {
                let stem = note_stem(job, naming);
                let path = if matches!(naming, NamingConfig::Datetime { .. }) {
                    collision_free_path(prefix, &stem, |id| {
                        output::couchdb::doc_exists(cdb, pwd, id)
                    })?
                } else {
                    output::couchdb::doc_path(prefix, &stem)
                };
                let mut sink = output::CouchdbSink::new(cdb.clone(), pwd.clone(), path.clone());
                match pipeline.run_one(source, &mut sink, None)? {
                    RunOutcome::Written => Ok(ProcessOutcome::Written(format!("couchdb://{path}"))),
                    RunOutcome::NoSpeech => Ok(ProcessOutcome::NoSpeech(NoSpeechReason::Silent)),
                }
            }
        }
    }
}

pub fn run(common: CommonRunArgs) -> Result<()> {
    let overrides = common.parsed_set_overrides().map_err(Error::Config)?;
    let cfg = Config::load_merged(&common.config, &overrides)?;
    cfg.apply_globals();
    cfg.output.name.validate()?;

    let models_dir = resolve::models_dir(&cfg, common.model_dir);
    let catalog = models::Catalog::load()?;
    let manager = std::sync::Arc::new(models::Manager::new(models_dir, catalog));

    let model_name = resolve::model_name(&cfg, common.model_name)?;
    let family = resolve::family(&cfg, common.model_family)?;

    let registry = ModelRegistry::new();
    registry.init_models(
        std::sync::Arc::clone(&manager),
        vec![(model_name.clone(), family)],
    );

    // Register the VAD model when enabled so it follows the same pull-pending
    // semantics as the transcription model.
    let vad_model_name_opt = cfg
        .audio
        .preprocess
        .vad
        .enabled
        .then(|| cfg.audio.preprocess.vad.model_name.clone());
    if let Some(ref vad_name) = vad_model_name_opt {
        registry.init_vad_models(
            std::sync::Arc::clone(&manager),
            vec![vad_name.clone()],
        );
    }

    let (sqlite_path, bucket_root, max_receive, visibility_sec, model_loop) =
        resolve_queue_infra(&cfg)?;

    let sink_kind = resolve::sink(&cfg, common.output, Sink::Couchdb);

    let queue_sink = build_queue_sink(sink_kind, common.target, common.overwrite, &cfg)?;

    let reg_for_guard = registry.clone();
    let guard_model_name = model_name.clone();
    let audio_cfg = cfg.audio.clone();
    let guard = ModelGuard::new(
        Box::new(move || {
            // Resolve transcription model.
            let status = reg_for_guard.get(&guard_model_name).ok_or_else(|| {
                Error::Model(format!("model `{}` not in registry", guard_model_name))
            })?;
            let handle = match status {
                ModelStatus::Ready(h) => h,
                ModelStatus::Pulling => return Err(Error::Model("model still downloading".into())),
                ModelStatus::Failed(msg) => return Err(Error::Model(msg)),
            };

            // Resolve VAD model path when VAD is enabled.
            let vad_path = if let Some(ref vad_name) = vad_model_name_opt {
                let vad_status = reg_for_guard.get(vad_name).ok_or_else(|| {
                    Error::Model(format!("VAD model `{vad_name}` not in registry"))
                })?;
                match vad_status {
                    ModelStatus::Ready(h) => Some(h.path.clone()),
                    ModelStatus::Pulling => {
                        return Err(Error::Model("VAD model still downloading".into()))
                    }
                    ModelStatus::Failed(msg) => return Err(Error::Model(msg)),
                }
            } else {
                None
            };

            Ok(Pipeline {
                decoder: Box::new(decode::DefaultDecoder::new()),
                preprocess: crate::preprocess::Preprocess::from_audio(Some(&audio_cfg), vad_path)?,
                transcriber: engine::build(&handle)?,
            })
        }),
        std::time::Duration::from_millis(model_loop.unload_after_ms),
    );
    let processor = QueueProcessor { queue_sink };

    let sqlite = SqliteBackend::open(&sqlite_path)?;
    let transcribe_q: crate::input::queue::backends::sqlite::SqliteQueue<TranscribeJob> =
        sqlite.queue("transcribe", max_receive)?;
    let notify_q: crate::input::queue::backends::sqlite::SqliteQueue<NotifyResult> =
        sqlite.queue("notifications", max_receive)?;
    let processed = sqlite.processed_store();
    let bucket = FsBucket::open(&bucket_root)?;

    let driver_cfg = QueueDriverConfig {
        visibility_sec,
        loaded_loop: std::time::Duration::from_millis(model_loop.loaded_loop_ms),
        unloaded_loop: std::time::Duration::from_millis(model_loop.unloaded_loop_ms),
    };
    let mut driver = QueueDriver::new(
        transcribe_q,
        notify_q,
        bucket,
        processed,
        processor,
        guard,
        driver_cfg,
        registry,
        model_name,
    );
    driver.run()
}

fn resolve_queue_infra(
    cfg: &Config,
) -> Result<(PathBuf, PathBuf, u32, u32, crate::config::ModelLoopConfig)> {
    let input_cfg = cfg
        .input
        .as_ref()
        .ok_or_else(|| Error::Config("input section missing".into()))?;
    if input_cfg.driver != "queue" {
        return Err(Error::Config(format!(
            "input.driver={:?}; only \"queue\" is supported",
            input_cfg.driver
        )));
    }
    let infra_q = input_cfg
        .queue
        .as_ref()
        .ok_or_else(|| Error::Config("input.queue section missing".into()))?;
    let bucket = infra_q
        .bucket
        .as_ref()
        .ok_or_else(|| Error::Config("input.queue.bucket section missing".into()))?;
    if infra_q.backend != "sqlite" {
        return Err(Error::Config(format!(
            "input.queue.backend={:?}; only \"sqlite\" is supported",
            infra_q.backend
        )));
    }
    if bucket.backend != "fs" {
        return Err(Error::Config(format!(
            "input.queue.bucket.backend={:?}; only \"fs\" is supported",
            bucket.backend
        )));
    }
    let sqlite_path = infra_q
        .sqlite
        .as_ref()
        .ok_or_else(|| Error::Config("input.queue.sqlite missing".into()))?
        .path
        .clone();
    let bucket_root = bucket
        .fs
        .as_ref()
        .ok_or_else(|| Error::Config("input.queue.bucket.fs missing".into()))?
        .root
        .clone();
    Ok((
        sqlite_path,
        bucket_root,
        infra_q.max_receive,
        infra_q.visibility_timeout_sec,
        infra_q.model,
    ))
}

fn build_queue_sink(
    sink_kind: Sink,
    cli_target: Option<String>,
    cli_overwrite: bool,
    cfg: &Config,
) -> Result<QueueSink> {
    match sink_kind {
        Sink::Stdout => Ok(QueueSink::Shared {
            sink: Box::new(
                output::StdoutSink::new().with_separator(resolve::stdout_separator(cfg)),
            ),
            output_ref: "stdout://".to_string(),
        }),
        Sink::File => {
            let p = resolve::file_path(cfg, cli_target)?;
            let overwrite = resolve::file_overwrite(cfg, cli_overwrite);
            let separator = resolve::file_separator(cfg);
            Ok(QueueSink::Shared {
                output_ref: format!("file://{p}"),
                sink: Box::new(
                    output::FileSink::new(PathBuf::from(p))
                        .with_overwrite(overwrite)
                        .with_separator(separator),
                ),
            })
        }
        Sink::Couchdb => {
            let (cdb, pwd) = resolve::load_couchdb_config(cfg)?;
            let state = output::couchdb::ensure_state(
                &cdb,
                &pwd,
                false,
                output::couchdb::DEFAULT_PROBE_LIMIT,
                output::couchdb::DEFAULT_PROBE_CHUNKS,
            )?;
            tracing::info!(
                schema = %state.schema,
                hash_algo = %state.hash_algo,
                e2ee = state.e2ee,
                obfuscated = state.path_obfuscation,
                "couchdb livesync state ensured"
            );
            let prefix = resolve::couchdb_target(cfg, cli_target)?;
            let naming = cfg.output.name.clone();
            Ok(QueueSink::Couchdb {
                cdb,
                pwd,
                prefix,
                naming,
            })
        }
    }
}

#[cfg(test)]
mod tests;
