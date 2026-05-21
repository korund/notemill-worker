use std::path::PathBuf;

use crate::cli::{CommonRunArgs, Sink};
use crate::config::Config;
use crate::pipeline::Pipeline;
use crate::{decode, engine, input, models, output, Error, Result};

use crate::commands::resolve;

pub fn run(
    common: CommonRunArgs,
    input_path: PathBuf,
    frontmatter: Option<String>,
) -> Result<()> {
    let overrides = common.parsed_set_overrides().map_err(Error::Config)?;
    let cfg = Config::load_merged(&common.config, &overrides)?;
    cfg.apply_globals();

    let models_dir = resolve::models_dir(&cfg, common.model_dir);
    let catalog = models::Catalog::load()?;
    let manager = models::Manager::new(models_dir, catalog);

    let model_name = resolve::model_name(&cfg, common.model_name)?;
    let family = resolve::family(&cfg, common.model_family)?;
    let model_handle = manager.resolve(&model_name, family)?;

    // Resolve VAD model path when VAD is enabled (one-shot: pull synchronously).
    let vad_path = if cfg.audio.preprocess.vad.enabled {
        let vad_name = cfg.audio.preprocess.vad.model_name.clone();
        match manager.resolve_vad(&vad_name) {
            Ok(h) => Some(h.path),
            Err(_) => {
                manager.pull_vad(&vad_name)?;
                Some(manager.resolve_vad(&vad_name)?.path)
            }
        }
    } else {
        None
    };

    let sink_kind = resolve::sink(&cfg, common.output, Sink::Stdout);
    let mut sink: Box<dyn output::OutputSink> = match sink_kind {
        Sink::Stdout => {
            if common.target.as_deref().is_some_and(|s| !s.is_empty()) {
                return Err(Error::Config(
                    "--target must not be set with --output stdout".into(),
                ));
            }
            Box::new(output::StdoutSink::new().with_separator(resolve::stdout_separator(&cfg)))
        }
        Sink::File => {
            let p = resolve::file_path(&cfg, common.target)?;
            let overwrite = resolve::file_overwrite(&cfg, common.overwrite);
            let separator = resolve::file_separator(&cfg);
            Box::new(
                output::FileSink::new(PathBuf::from(p))
                    .with_overwrite(overwrite)
                    .with_separator(separator),
            )
        }
        Sink::Couchdb => {
            let (cdb, pwd) = resolve::load_couchdb_config(&cfg)?;
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
            let target = resolve::couchdb_target(&cfg, common.target)?;
            Box::new(output::CouchdbSink::new(cdb, pwd, target))
        }
    };

    let fm = frontmatter
        .as_deref()
        .filter(|s| !s.is_empty())
        .and_then(crate::output::frontmatter::render_from_spec);

    let source = input::LocalFileSource::new(input_path);
    let mut pipeline = Pipeline {
        decoder: Box::new(decode::DefaultDecoder::new()),
        preprocess: crate::preprocess::Preprocess::from_config(&cfg, vad_path)?,
        transcriber: engine::build(&model_handle)?,
    };
    pipeline.run_one(&source, sink.as_mut(), fm.as_deref())?;
    Ok(())
}
