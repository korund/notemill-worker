use clap::Parser;
use voice2text::cli::{Cli, Command, CouchdbCommand, ModelsCommand, OutputKind};
use std::path::PathBuf;
use voice2text::{decode, engine, input, models, output, Result};
use tracing_subscriber::EnvFilter;

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
