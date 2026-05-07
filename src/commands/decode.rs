use std::path::PathBuf;

use crate::{decode, input, Error, Result};

pub fn run(input_path: PathBuf, output_path: Option<PathBuf>) -> Result<()> {
    let source = input::LocalFileSource::new(input_path);
    let decoder = decode::DefaultDecoder::new();
    let raw = <input::LocalFileSource as input::AudioSource>::read(&source)?;
    let pcm = <decode::DefaultDecoder as decode::AudioDecoder>::decode(&decoder, &raw)?;

    let duration_secs = pcm.samples.len() as f64 / decode::TARGET_SAMPLE_RATE as f64;
    let (min, max) = pcm
        .samples
        .iter()
        .fold((f32::MAX, f32::MIN), |(lo, hi), &s| (lo.min(s), hi.max(s)));
    let rms = (pcm.samples.iter().map(|s| s * s).sum::<f32>() / pcm.samples.len() as f32).sqrt();

    println!("Samples : {}", pcm.samples.len());
    println!("Rate    : {} Hz", decode::TARGET_SAMPLE_RATE);
    println!("Duration: {:.3} s", duration_secs);
    println!("Range   : [{:.4}, {:.4}]", min, max);
    println!("RMS     : {:.4}", rms);

    if let Some(path) = output_path {
        let bytes: Vec<u8> = pcm.samples.iter().flat_map(|s| s.to_le_bytes()).collect();
        std::fs::write(&path, &bytes)
            .map_err(|e| Error::Output(format!("write {}: {e}", path.display())))?;
        println!("PCM f32 written to {}", path.display());
    }
    Ok(())
}
