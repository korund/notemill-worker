use crate::{decode, engine, input, output, Result};

pub struct Pipeline {
    pub decoder: Box<dyn decode::AudioDecoder>,
    pub transcriber: Box<dyn engine::Transcriber>,
}

impl Pipeline {
    pub fn run_one(
        &mut self,
        source: &dyn input::AudioSource,
        sink: &mut dyn output::OutputSink,
        fm: Option<&str>,
    ) -> Result<()> {
        let raw = source.read()?;
        let pcm = self.decoder.decode(&raw)?;
        let text = self.transcriber.transcribe(&pcm)?;
        let body = match fm {
            Some(prefix) => format!("{}{}", prefix, text),
            None => text,
        };
        sink.write(&body)
    }
}
