use crate::decode::Pcm16kMono;
#[cfg(feature = "engine-transcribe")]
use crate::models::ModelFamily;
use crate::models::ResolvedModel;
use crate::{Error, Result};

pub trait Transcriber {
    fn transcribe(&mut self, pcm: &Pcm16kMono) -> Result<String>;
}

pub fn build(model: &ResolvedModel) -> Result<Box<dyn Transcriber>> {
    #[cfg(not(feature = "engine-transcribe"))]
    {
        let _ = model;
        return Err(Error::NotImplemented(
            "engine: enable feature `engine-transcribe` to use transcribe-rs",
        ));
    }

    #[cfg(feature = "engine-transcribe")]
    {
        use transcribe_rs::SpeechModel;

        let family = model.family.ok_or_else(|| {
            Error::Engine("transcribe family expected, got VAD-only model".to_string())
        })?;

        let boxed: Box<dyn SpeechModel> = match family {
            ModelFamily::Whisper => {
                let engine = transcribe_rs::whisper_cpp::WhisperEngine::load(&model.path)
                    .map_err(|e| Error::Engine(format!("whisper load: {e}")))?;
                Box::new(engine)
            }
            ModelFamily::Parakeet => {
                let q = detect_quantization(&model.path);
                let engine = transcribe_rs::onnx::parakeet::ParakeetModel::load(&model.path, &q)
                    .map_err(|e| Error::Engine(format!("parakeet load: {e}")))?;
                Box::new(engine)
            }
            ModelFamily::GigaAm => {
                let q = detect_quantization(&model.path);
                let engine = transcribe_rs::onnx::gigaam::GigaAMModel::load(&model.path, &q)
                    .map_err(|e| Error::Engine(format!("gigaam load: {e}")))?;
                Box::new(engine)
            }
            ModelFamily::SenseVoice => {
                let engine = transcribe_rs::onnx::sense_voice::SenseVoiceModel::load(
                    &model.path,
                    &transcribe_rs::onnx::Quantization::Int8,
                )
                .map_err(|e| Error::Engine(format!("sense_voice load: {e}")))?;
                Box::new(engine)
            }
            ModelFamily::Canary => {
                let engine = transcribe_rs::onnx::canary::CanaryModel::load(
                    &model.path,
                    &transcribe_rs::onnx::Quantization::Int8,
                )
                .map_err(|e| Error::Engine(format!("canary load: {e}")))?;
                Box::new(engine)
            }
            ModelFamily::Cohere => {
                let engine = transcribe_rs::onnx::cohere::CohereModel::load(
                    &model.path,
                    &transcribe_rs::onnx::Quantization::Int8,
                )
                .map_err(|e| Error::Engine(format!("cohere load: {e}")))?;
                Box::new(engine)
            }
        };

        Ok(Box::new(TranscribeRsAdapter { inner: boxed }))
    }
}

#[cfg(feature = "engine-transcribe")]
fn detect_quantization(dir: &std::path::Path) -> transcribe_rs::onnx::Quantization {
    use transcribe_rs::onnx::Quantization;
    for (suffix, q) in [
        ("int4", Quantization::Int4),
        ("int8", Quantization::Int8),
        ("fp16", Quantization::FP16),
    ] {
        if dir.join(format!("model.{suffix}.onnx")).exists() {
            return q;
        }
    }
    Quantization::FP32
}

#[cfg(feature = "engine-transcribe")]
struct TranscribeRsAdapter {
    inner: Box<dyn transcribe_rs::SpeechModel>,
}

#[cfg(feature = "engine-transcribe")]
impl Transcriber for TranscribeRsAdapter {
    fn transcribe(&mut self, pcm: &Pcm16kMono) -> Result<String> {
        let opts = transcribe_rs::TranscribeOptions::default();
        let result = self
            .inner
            .transcribe(&pcm.samples, &opts)
            .map_err(|e| Error::Engine(format!("transcribe: {e}")))?;
        Ok(result.text)
    }
}
