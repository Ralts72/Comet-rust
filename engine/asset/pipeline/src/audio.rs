//! WAV is decoded only by the producer; consumers load the engine PCM protocol.
use anyhow::{Context, Result, ensure};
use comet_asset_data::{
    AudioData,
    audio::{AUDIO_VERSION, MAX_AUDIO_SAMPLES},
};
use std::{
    fs,
    io::{Cursor, Read},
    path::Path,
};

// PCM16 payload plus bounded RIFF headers and metadata chunks.
const MAX_WAV_BYTES: usize = MAX_AUDIO_SAMPLES * 2 + 64 * 1024;

pub(super) fn read(path: &Path) -> Result<Vec<u8>> {
    let file = fs::File::open(path).with_context(|| format!("read {}", path.display()))?;
    ensure!(
        file.metadata()?.len() <= MAX_WAV_BYTES as u64,
        "WAV source exceeds its byte budget: {}",
        path.display()
    );
    let mut bytes = Vec::new();
    file.take(MAX_WAV_BYTES as u64 + 1)
        .read_to_end(&mut bytes)?;
    ensure!(
        bytes.len() <= MAX_WAV_BYTES,
        "WAV source grew beyond its byte budget"
    );
    Ok(bytes)
}

pub(super) fn import(bytes: &[u8]) -> Result<AudioData> {
    ensure!(
        bytes.len() <= MAX_WAV_BYTES,
        "WAV source exceeds its byte budget"
    );
    let mut reader = hound::WavReader::new(Cursor::new(bytes)).context("parse WAV source")?;
    let spec = reader.spec();
    ensure!(
        spec.sample_format == hound::SampleFormat::Int && spec.bits_per_sample == 16,
        "only PCM16 WAV audio is supported"
    );
    ensure!(
        matches!(spec.channels, 1 | 2),
        "WAV requires one or two channels"
    );
    ensure!(
        (8_000..=192_000).contains(&spec.sample_rate),
        "WAV sample rate must be between 8000 and 192000 Hz"
    );
    let count = reader.len() as usize;
    ensure!(
        count > 0 && count <= MAX_AUDIO_SAMPLES && count.is_multiple_of(spec.channels as usize),
        "WAV requires complete frames within its sample budget"
    );
    ensure!(
        count <= bytes.len() / 2,
        "WAV sample count exceeds its source payload"
    );
    let samples: Vec<i16> = reader
        .samples::<i16>()
        .collect::<Result<_, _>>()
        .context("decode PCM16 WAV samples")?;
    ensure!(
        samples.len() == count,
        "WAV sample count changed during decoding"
    );
    let audio = AudioData {
        version: AUDIO_VERSION,
        sample_rate: spec.sample_rate,
        channels: spec.channels,
        samples: samples.into(),
    };
    audio.validate()?;
    Ok(audio)
}
