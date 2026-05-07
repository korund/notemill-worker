use clap::Parser;
use voice2text::cli::{Cli, Command, ModelsCommand};
use voice2text::{decode, engine, input, models, output, Result};
use tracing_subscriber::EnvFilter;

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
            output: output_path,
        } => {
            let model_handle = manager.resolve(&model, family.map(Into::into))?;

            let source: Box<dyn input::AudioSource> =
                Box::new(input::LocalFileSource::new(input_path));
            let decoder: Box<dyn decode::AudioDecoder> = Box::new(decode::DefaultDecoder::new());
            let mut transcriber: Box<dyn engine::Transcriber> =
                engine::build(&model_handle)?;
            let mut sink: Box<dyn output::OutputSink> = match output_path {
                Some(path) => Box::new(output::FileSink::new(path)),
                None => Box::new(output::StdoutSink::new()),
            };

            let raw = source.read()?;
            let pcm = decoder.decode(&raw)?;
            let text = transcriber.transcribe(&pcm)?;
            sink.write(&text)?;
            Ok(())
        }
    }
}
