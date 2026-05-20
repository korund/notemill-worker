use super::*;
use crate::decode::AudioDecoder;
use crate::engine::Transcriber;
use crate::input::{AudioSource, RawAudio};
use crate::output::OutputSink;
use crate::preprocess::chunker::Chunker;
use crate::preprocess::{Preprocess, Segment, Speech, SpeechSegmenter};
use crate::{decode, Result};
use std::cell::Cell;

// ---------- stubs ----------------------------------------------------------

struct StubSource;
impl AudioSource for StubSource {
    fn name(&self) -> &str {
        "stub"
    }
    fn read(&self) -> Result<RawAudio> {
        Ok(RawAudio {
            bytes: vec![0u8; 4],
            format_hint: None,
        })
    }
}

struct StubDecoder;
impl AudioDecoder for StubDecoder {
    fn decode(&self, _raw: &RawAudio) -> Result<decode::Pcm16kMono> {
        // 1 second of zeros at 16 kHz; content does not matter because the
        // segmenter is also stubbed.
        Ok(decode::Pcm16kMono {
            samples: vec![0.0; 16_000],
        })
    }
}

struct StubSegmenter {
    verdict: Cell<Option<Speech>>,
}
impl StubSegmenter {
    fn new(v: Speech) -> Self {
        Self {
            verdict: Cell::new(Some(v)),
        }
    }
}
impl SpeechSegmenter for StubSegmenter {
    fn segment(&mut self, _pcm: &decode::Pcm16kMono) -> Result<Speech> {
        Ok(self.verdict.take().expect("segmenter called twice"))
    }
}

#[derive(Default)]
struct StubTranscriber {
    calls: std::rc::Rc<Cell<usize>>,
}
impl Transcriber for StubTranscriber {
    fn transcribe(&mut self, _pcm: &decode::Pcm16kMono) -> Result<String> {
        self.calls.set(self.calls.get() + 1);
        Ok("hello".to_string())
    }
}

#[derive(Default)]
struct StubSink {
    writes: std::rc::Rc<Cell<usize>>,
}
impl OutputSink for StubSink {
    fn write(&mut self, _text: &str) -> Result<()> {
        self.writes.set(self.writes.get() + 1);
        Ok(())
    }
}

// Passthrough chunker that emits one chunk per segment without overlap.
struct PassthroughChunker;
impl Chunker for PassthroughChunker {
    fn chunk(&self, segments: Vec<Segment>) -> Vec<crate::preprocess::chunker::Chunk> {
        segments
            .into_iter()
            .map(|s| crate::preprocess::chunker::Chunk {
                pcm: s.pcm,
                has_overlap_with_next: false,
            })
            .collect()
    }
}

// ---------- helpers --------------------------------------------------------

fn build_pipeline(verdict: Option<Speech>) -> (Pipeline, std::rc::Rc<Cell<usize>>, std::rc::Rc<Cell<usize>>) {
    let transcribe_calls = std::rc::Rc::new(Cell::new(0));
    let write_calls = std::rc::Rc::new(Cell::new(0));
    let preprocess = Preprocess {
        segmenter: verdict
            .map(|v| Box::new(StubSegmenter::new(v)) as Box<dyn SpeechSegmenter>),
        chunker: Some(Box::new(PassthroughChunker)),
    };
    let pipeline = Pipeline {
        decoder: Box::new(StubDecoder),
        preprocess,
        transcriber: Box::new(StubTranscriber {
            calls: transcribe_calls.clone(),
        }),
    };
    (pipeline, transcribe_calls, write_calls)
}

fn run(pipeline: &mut Pipeline, sink_writes: std::rc::Rc<Cell<usize>>) -> Result<RunOutcome> {
    let mut sink = StubSink {
        writes: sink_writes,
    };
    pipeline.run_one(&StubSource, &mut sink, None)
}

fn segment(start: u32) -> Segment {
    Segment {
        start_ms: start,
        end_ms: start + 100,
        pcm: vec![0.0; 1600],
    }
}

// ---------- tests ----------------------------------------------------------

#[test]
fn detected_runs_transcription_and_writes() {
    let (mut p, transcribes, writes) = build_pipeline(Some(Speech::Detected(vec![segment(0)])));
    let outcome = run(&mut p, writes.clone()).unwrap();
    assert_eq!(outcome, RunOutcome::Written);
    assert_eq!(transcribes.get(), 1);
    assert_eq!(writes.get(), 1);
}

#[test]
fn faint_falls_back_to_full_pcm_and_writes() {
    let (mut p, transcribes, writes) = build_pipeline(Some(Speech::Faint));
    let outcome = run(&mut p, writes.clone()).unwrap();
    assert_eq!(outcome, RunOutcome::Written);
    assert_eq!(transcribes.get(), 1, "faint must still transcribe (fallback)");
    assert_eq!(writes.get(), 1);
}

#[test]
fn none_short_circuits_without_transcribing_or_writing() {
    let (mut p, transcribes, writes) = build_pipeline(Some(Speech::None));
    let outcome = run(&mut p, writes.clone()).unwrap();
    assert_eq!(outcome, RunOutcome::NoSpeech);
    assert_eq!(transcribes.get(), 0, "no speech must not invoke transcriber");
    assert_eq!(writes.get(), 0, "no speech must not write to sink");
}

#[test]
fn no_segmenter_runs_transcription_and_writes() {
    let (mut p, transcribes, writes) = build_pipeline(None);
    let outcome = run(&mut p, writes.clone()).unwrap();
    assert_eq!(outcome, RunOutcome::Written);
    assert_eq!(transcribes.get(), 1);
    assert_eq!(writes.get(), 1);
}
