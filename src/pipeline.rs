use std::time::Instant;

use tracing::info;

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
            Some(prefix) => format!("{}{}\n", prefix, text),
            None => format!("{}\n", text),
        };
        sink.write(&body)
    }
}

// ---------------------------------------------------------------------------
// ModelGuard -- lazy load / idle unload for the pipeline's heavy resources.
// ---------------------------------------------------------------------------

pub type PipelineFactory = Box<dyn FnMut() -> Result<Pipeline>>;

pub struct ModelGuard {
    pipeline: Option<Pipeline>,
    factory: PipelineFactory,
    idle_timeout: std::time::Duration,
    last_used: Option<Instant>,
}

impl ModelGuard {
    pub fn new(factory: PipelineFactory, idle_timeout: std::time::Duration) -> Self {
        Self {
            pipeline: None,
            factory,
            idle_timeout,
            last_used: None,
        }
    }

    pub fn acquire(&mut self) -> Result<&mut Pipeline> {
        if self.pipeline.is_none() {
            info!("loading model");
            self.pipeline = Some((self.factory)()?);
        }
        self.last_used = Some(Instant::now());
        Ok(self.pipeline.as_mut().unwrap())
    }

    pub fn try_unload(&mut self) {
        let Some(last) = self.last_used else { return };
        if last.elapsed() >= self.idle_timeout {
            if self.pipeline.is_some() {
                info!(idle_ms = last.elapsed().as_millis() as u64, "unloading model");
                self.pipeline = None;
                self.last_used = None;
            }
        }
    }

    pub fn is_loaded(&self) -> bool {
        self.pipeline.is_some()
    }
}
