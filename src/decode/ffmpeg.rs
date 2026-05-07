use std::path::Path;

use ffmpeg_next as ffmpeg;
use ffmpeg_next::format::Sample as AvSample;
use ffmpeg_next::software::resampling;
use ffmpeg_next::util::channel_layout::ChannelLayout;
use ffmpeg_next::util::frame::audio::Audio;

use crate::input::RawAudio;
use crate::{Error, Result};

use super::{Pcm16kMono, TARGET_SAMPLE_RATE};

static FFMPEG_INIT: std::sync::Once = std::sync::Once::new();

fn ensure_init() {
    FFMPEG_INIT.call_once(|| {
        ffmpeg::init().expect("ffmpeg init failed");
    });
}

pub fn decode_to_pcm16k(raw: &RawAudio) -> Result<Pcm16kMono> {
    ensure_init();

    let tmp_dir = std::env::temp_dir();
    let ext = raw.format_hint.as_deref().unwrap_or("bin");
    let tmp_path = tmp_dir.join(format!("nc_decode_{}_{}.{ext}", std::process::id(), format!("{:?}", std::thread::current().id())));
    std::fs::write(&tmp_path, &raw.bytes)
        .map_err(|e| Error::Decode(format!("write temp file: {e}")))?;

    let result = decode_file(&tmp_path);
    let _ = std::fs::remove_file(&tmp_path);
    result
}

fn decode_file(path: &Path) -> Result<Pcm16kMono> {
    let mut ictx = ffmpeg::format::input(path)
        .map_err(|e| Error::Decode(format!("open input: {e}")))?;

    let stream = ictx
        .streams()
        .best(ffmpeg::media::Type::Audio)
        .ok_or_else(|| Error::Decode("no audio stream found".into()))?;
    let stream_index = stream.index();

    let codec_par = stream.parameters();
    let ctx = ffmpeg::codec::context::Context::from_parameters(codec_par)
        .map_err(|e| Error::Decode(format!("codec context: {e}")))?;
    let mut decoder = ctx
        .decoder()
        .audio()
        .map_err(|e| Error::Decode(format!("audio decoder: {e}")))?;

    let mut resampler: Option<resampling::Context> = None;
    let mut all_samples: Vec<f32> = Vec::new();

    let receive_frames =
        |decoder: &mut ffmpeg::decoder::Audio,
         resampler: &mut Option<resampling::Context>,
         all_samples: &mut Vec<f32>|
         -> Result<()> {
            let mut decoded = Audio::empty();
            while decoder.receive_frame(&mut decoded).is_ok() {
                if decoded.channel_layout().is_empty() {
                    decoded.set_channel_layout(ChannelLayout::default(
                        decoded.channels() as i32,
                    ));
                }

                let rs = resampler.get_or_insert_with(|| {
                    resampling::Context::get(
                        decoded.format(),
                        decoded.channel_layout(),
                        decoded.rate(),
                        AvSample::F32(ffmpeg::format::sample::Type::Packed),
                        ChannelLayout::MONO,
                        TARGET_SAMPLE_RATE,
                    )
                    .expect("resampler init failed")
                });

                let mut resampled = Audio::empty();
                rs.run(&decoded, &mut resampled)
                    .map_err(|e| Error::Decode(format!("resample: {e}")))?;
                extend_f32(&resampled, all_samples);
            }
            Ok(())
        };

    for (stream, packet) in ictx.packets() {
        if stream.index() != stream_index {
            continue;
        }
        decoder
            .send_packet(&packet)
            .map_err(|e| Error::Decode(format!("send packet: {e}")))?;
        receive_frames(&mut decoder, &mut resampler, &mut all_samples)?;
    }

    decoder
        .send_eof()
        .map_err(|e| Error::Decode(format!("send eof: {e}")))?;
    receive_frames(&mut decoder, &mut resampler, &mut all_samples)?;

    if let Some(mut rs) = resampler {
        let mut flushed = Audio::empty();
        while rs.flush(&mut flushed).is_ok() {
            if flushed.samples() == 0 {
                break;
            }
            extend_f32(&flushed, &mut all_samples);
        }
    }

    if all_samples.is_empty() {
        return Err(Error::Decode("no audio samples decoded".into()));
    }

    Ok(Pcm16kMono {
        samples: all_samples,
    })
}

fn extend_f32(frame: &Audio, out: &mut Vec<f32>) {
    let n = frame.samples();
    if n == 0 {
        return;
    }
    let data = frame.data(0);
    let byte_len = n * std::mem::size_of::<f32>();
    for chunk in data[..byte_len].chunks_exact(4) {
        out.push(f32::from_ne_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]));
    }
}
