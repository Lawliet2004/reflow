pub fn wav_or_pcm_to_f32(bytes: &[u8]) -> Result<Vec<f32>, String> {
    if bytes.len() > 12 && &bytes[0..4] == b"RIFF" {
        let cursor = std::io::Cursor::new(bytes.to_vec());
        let mut reader = hound::WavReader::new(cursor).map_err(|e| e.to_string())?;
        let spec = reader.spec();
        crate::session::validate_sample_rate(spec.sample_rate).map_err(|e| e.message)?;
        if spec.channels == 0 || spec.channels > 8 {
            return Err("WAV audio must contain 1 to 8 channels".into());
        }

        let mut samples: Vec<f32> = Vec::new();
        match spec.sample_format {
            hound::SampleFormat::Int => {
                for s in reader.samples::<i16>() {
                    samples.push(s.map_err(|e| e.to_string())? as f32 / 32768.0);
                }
            }
            hound::SampleFormat::Float => {
                for s in reader.samples::<f32>() {
                    let sample = s.map_err(|e| e.to_string())?;
                    if !sample.is_finite() {
                        return Err("WAV samples must be finite".into());
                    }
                    samples.push(sample);
                }
            }
        }
        if spec.channels > 1 {
            let ch = spec.channels as usize;
            samples = samples
                .chunks_exact(ch)
                .map(|c| c.iter().sum::<f32>() / ch as f32)
                .collect();
        }
        if spec.sample_rate != 16000 {
            let mut resampler = crate::audio::AudioResampler::new(spec.sample_rate, 1);
            samples = resampler.resample_f32(&samples);
        }
        return Ok(samples);
    }
    Ok(crate::audio::AudioResampler::pcm16_bytes_to_f32(bytes))
}

use std::path::Path;
use symphonia::core::{
    audio::SampleBuffer, codecs::DecoderOptions, errors::Error, formats::FormatOptions,
    io::MediaSourceStream, meta::MetadataOptions, probe::Hint,
};

pub const MAX_FILE_SAMPLES: usize = 16_000 * 60 * 60 * 2;
pub const CONFIRM_FILE_BYTES: u64 = 100 * 1024 * 1024;

/// Decode packets incrementally; audio stays on this computer.
pub fn decode_file(path: &Path, allow_large: bool) -> Result<Vec<f32>, String> {
    let metadata = std::fs::metadata(path).map_err(|e| e.to_string())?;
    if !metadata.is_file() {
        return Err("Choose an audio file".into());
    }
    if metadata.len() > CONFIRM_FILE_BYTES && !allow_large {
        return Err("This file exceeds 100 MB. Confirm the large-file import first.".into());
    }
    let file = std::fs::File::open(path).map_err(|e| e.to_string())?;
    let mut hint = Hint::new();
    if let Some(extension) = path.extension().and_then(|e| e.to_str()) {
        hint.with_extension(extension);
    }
    let source = MediaSourceStream::new(Box::new(file), Default::default());
    let probe = symphonia::default::get_probe()
        .format(
            &hint,
            source,
            &FormatOptions::default(),
            &MetadataOptions::default(),
        )
        .map_err(|e| e.to_string())?;
    let mut format = probe.format;
    let track = format.default_track().ok_or("No audio track found")?;
    let id = track.id;
    let mut decoder = symphonia::default::get_codecs()
        .make(&track.codec_params, &DecoderOptions::default())
        .map_err(|e| e.to_string())?;
    let mut resampler = None;
    let mut stream_spec = None;
    let mut output = Vec::new();
    loop {
        let packet = match format.next_packet() {
            Ok(packet) => packet,
            Err(Error::IoError(e)) if e.kind() == std::io::ErrorKind::UnexpectedEof => break,
            Err(e) => return Err(format!("Audio container error: {e}")),
        };
        if packet.track_id() != id {
            continue;
        }
        let decoded = decoder
            .decode(&packet)
            .map_err(|e| format!("Audio decoding failed: {e}"))?;
        let spec = *decoded.spec();
        let channels = spec.channels.count();
        crate::session::validate_sample_rate(spec.rate).map_err(|e| e.message)?;
        if !(1..=8).contains(&channels) {
            return Err("Audio must contain 1 to 8 channels".into());
        }
        if stream_spec.is_some_and(|previous| previous != spec) {
            return Err("Changing audio formats are unsupported".into());
        }
        stream_spec = Some(spec);
        let converter = resampler
            .get_or_insert_with(|| crate::audio::AudioResampler::new(spec.rate, channels as u16));
        let mut buffer = SampleBuffer::<f32>::new(decoded.capacity() as u64, spec);
        buffer.copy_interleaved_ref(decoded);
        if buffer.samples().iter().any(|s| !s.is_finite()) {
            return Err("Audio samples must be finite".into());
        }
        output.extend(converter.resample_f32(buffer.samples()));
        if output.len() > MAX_FILE_SAMPLES {
            return Err("Audio files must be at most two hours long".into());
        }
    }
    if output.is_empty() {
        return Err("The file contains no audio".into());
    }
    Ok(output)
}
