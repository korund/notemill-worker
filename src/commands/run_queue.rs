use std::path::PathBuf;

use crate::cli::{CommonRunArgs, Sink};
use crate::config::Config;
use crate::input::queue::backends::fs::FsBucket;
use crate::input::queue::backends::sqlite::SqliteBackend;
use crate::input::queue::job::{NotifyResult, TranscribeJob};
use crate::input::queue::{JobProcessor, QueueDriver, QueueDriverConfig};
use crate::input::{AudioSource, InputDriver};
use crate::pipeline::Pipeline;
use crate::{decode, engine, models, output, Error, Result};

use super::resolve;

enum QueueSink {
    /// Shared persistent sink (file or stdout). The sink itself handles any
    /// inter-job separator (see `output::FileSink::with_separator`).
    Shared {
        sink: Box<dyn output::OutputSink>,
        output_ref: String,
    },
    /// Per-job CouchDB sink; path = prefix/safe_dedup_key.
    Couchdb {
        cdb: crate::config::CouchdbConfig,
        pwd: String,
        prefix: String,
    },
}

struct QueueProcessor {
    pipeline: Pipeline,
    queue_sink: QueueSink,
}

impl JobProcessor for QueueProcessor {
    fn process(&mut self, source: &dyn AudioSource, job: &TranscribeJob) -> Result<String> {
        match &mut self.queue_sink {
            QueueSink::Shared { sink, output_ref } => {
                self.pipeline.run_one(source, sink.as_mut(), None)?;
                Ok(output_ref.clone())
            }
            QueueSink::Couchdb { cdb, pwd, prefix } => {
                let safe = job.dedup_key.replace(':', "-");
                // .md so obsidian-livesync surfaces the doc as a regular note in the vault.
                let path = format!("{}/{}.md", prefix.trim_end_matches('/'), safe);
                let mut sink = output::CouchdbSink::new(cdb.clone(), pwd.clone(), path.clone());
                self.pipeline.run_one(source, &mut sink, None)?;
                Ok(format!("couchdb://{path}"))
            }
        }
    }
}

pub fn run(common: CommonRunArgs) -> Result<()> {
    let overrides = common.parsed_set_overrides().map_err(Error::Config)?;
    let cfg = Config::load_merged(&common.config, &overrides)?;
    cfg.apply_globals();

    let models_dir = resolve::models_dir(&cfg, common.model_dir);
    let catalog = models::Catalog::load()?;
    let manager = models::Manager::new(models_dir, catalog);

    let model_name = resolve::model_name(&cfg, common.model_name)?;
    let family = resolve::family(&cfg, common.model_family)?;
    let model_handle = manager.resolve(&model_name, family)?;

    let (sqlite_path, bucket_root, max_receive, visibility_sec, poll_interval_ms) =
        resolve_queue_infra(&cfg)?;

    let sink_kind = resolve::sink(&cfg, common.output, Sink::Couchdb);

    let queue_sink = build_queue_sink(sink_kind, common.target, common.overwrite, &cfg)?;

    let pipeline = Pipeline {
        decoder: Box::new(decode::DefaultDecoder::new()),
        transcriber: engine::build(&model_handle)?,
    };
    let processor = QueueProcessor { pipeline, queue_sink };

    let sqlite = SqliteBackend::open(&sqlite_path)?;
    let transcribe_q: crate::input::queue::backends::sqlite::SqliteQueue<TranscribeJob> =
        sqlite.queue("transcribe", max_receive)?;
    let notify_q: crate::input::queue::backends::sqlite::SqliteQueue<NotifyResult> =
        sqlite.queue("notifications", max_receive)?;
    let processed = sqlite.processed_store();
    let bucket = FsBucket::open(&bucket_root)?;

    let driver_cfg = QueueDriverConfig {
        visibility_sec,
        poll_interval: std::time::Duration::from_millis(poll_interval_ms),
    };
    let mut driver =
        QueueDriver::new(transcribe_q, notify_q, bucket, processed, processor, driver_cfg);
    driver.run()
}

/// Returns (sqlite_path, bucket_root, max_receive, visibility_sec, poll_interval_ms).
fn resolve_queue_infra(cfg: &Config) -> Result<(PathBuf, PathBuf, u32, u32, u64)> {
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
        infra_q.poll_interval_ms,
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
            sink: Box::new(output::StdoutSink::new().with_separator(resolve::stdout_separator(cfg))),
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
            let _state = output::couchdb::ensure_state(
                &cdb,
                &pwd,
                false,
                output::couchdb::DEFAULT_PROBE_LIMIT,
                output::couchdb::DEFAULT_PROBE_CHUNKS,
            )?;
            let prefix = resolve::couchdb_target(cfg, cli_target)?;
            Ok(QueueSink::Couchdb { cdb, pwd, prefix })
        }
    }
}
