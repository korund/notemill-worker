#[cfg(feature = "decode-ffmpeg")]
mod decode_ffmpeg {
    use notemill_worker::decode::{AudioDecoder, DefaultDecoder, TARGET_SAMPLE_RATE};
    use notemill_worker::input::RawAudio;

    fn generate_wav_sine(
        sample_rate: u32,
        channels: u16,
        freq_hz: f32,
        duration_secs: f32,
    ) -> Vec<u8> {
        let num_samples = (sample_rate as f32 * duration_secs) as usize;
        let bits_per_sample: u16 = 16;
        let block_align = channels * (bits_per_sample / 8);
        let byte_rate = sample_rate * block_align as u32;
        let data_size = (num_samples * channels as usize * 2) as u32;

        let mut buf: Vec<u8> = Vec::new();

        // RIFF header
        buf.extend_from_slice(b"RIFF");
        buf.extend_from_slice(&(36 + data_size).to_le_bytes());
        buf.extend_from_slice(b"WAVE");

        // fmt chunk
        buf.extend_from_slice(b"fmt ");
        buf.extend_from_slice(&16u32.to_le_bytes());
        buf.extend_from_slice(&1u16.to_le_bytes()); // PCM
        buf.extend_from_slice(&channels.to_le_bytes());
        buf.extend_from_slice(&sample_rate.to_le_bytes());
        buf.extend_from_slice(&byte_rate.to_le_bytes());
        buf.extend_from_slice(&block_align.to_le_bytes());
        buf.extend_from_slice(&bits_per_sample.to_le_bytes());

        // data chunk
        buf.extend_from_slice(b"data");
        buf.extend_from_slice(&data_size.to_le_bytes());

        for i in 0..num_samples {
            let t = i as f32 / sample_rate as f32;
            let sample = (2.0 * std::f32::consts::PI * freq_hz * t).sin();
            let s16 = (sample * 32767.0) as i16;
            for _ in 0..channels {
                buf.extend_from_slice(&s16.to_le_bytes());
            }
        }

        buf
    }

    #[test]
    fn decode_mono_wav_44100() {
        let wav = generate_wav_sine(44_100, 1, 440.0, 1.0);
        let raw = RawAudio {
            bytes: wav,
            format_hint: Some("wav".into()),
        };

        let decoder = DefaultDecoder::new();
        let pcm = decoder.decode(&raw).expect("decode failed");

        let expected_samples = TARGET_SAMPLE_RATE as usize; // 1 second
        let tolerance = (expected_samples as f32 * 0.02) as usize;
        assert!(
            (pcm.samples.len() as i64 - expected_samples as i64).unsigned_abs() < tolerance as u64,
            "expected ~{expected_samples} samples, got {}",
            pcm.samples.len()
        );

        assert!(
            pcm.samples.iter().any(|&s| s.abs() > 0.5),
            "signal should contain non-trivial amplitude"
        );

        assert!(
            pcm.samples.iter().all(|&s| s >= -1.01 && s <= 1.01),
            "samples should be in [-1, 1] range"
        );
    }

    #[test]
    fn decode_stereo_wav_48000() {
        let wav = generate_wav_sine(48_000, 2, 440.0, 2.0);
        let raw = RawAudio {
            bytes: wav,
            format_hint: Some("wav".into()),
        };

        let decoder = DefaultDecoder::new();
        let pcm = decoder.decode(&raw).expect("decode failed");

        let expected_samples = TARGET_SAMPLE_RATE as usize * 2; // 2 sec
        let tolerance = (expected_samples as f32 * 0.02) as usize;
        assert!(
            (pcm.samples.len() as i64 - expected_samples as i64).unsigned_abs() < tolerance as u64,
            "expected ~{expected_samples} samples, got {}",
            pcm.samples.len()
        );
    }

    #[test]
    fn decode_wav_already_16k() {
        let wav = generate_wav_sine(16_000, 1, 440.0, 0.25);
        let raw = RawAudio {
            bytes: wav,
            format_hint: Some("wav".into()),
        };

        let decoder = DefaultDecoder::new();
        let pcm = decoder.decode(&raw).expect("decode failed");

        // No resampling -- exact sample count
        assert_eq!(pcm.samples.len(), 4_000, "16kHz * 0.25s = 4000 samples");
    }
}
