use clap::Parser;
use notes_capture::cli::{Cli, Command, ModelsCommand};
use notes_capture::{decode, engine, input, models, output, Result};
use tracing_subscriber::EnvFilter;

fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")))
        .with_writer(std::io::stderr)
        .init();

    let cli = Cli::parse();

    let models_dir = cli.models_dir();
    let manager = models::Manager::new(models_dir, models::Catalog::embedded()?);

    match cli.command {
        Command::Models { cmd } => match cmd {
            ModelsCommand::List => {
                manager.print_list();
                Ok(())
            }
            ModelsCommand::Pull { name } => manager.pull(&name),
        },
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
