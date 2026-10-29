//! Renderizado offline: el motor se llama por bloques igual que en el
//! callback, pero desde un hilo normal y sin dispositivo de audio.

use std::path::Path;

use crate::engine::{Controller, Engine};

/// Tamaño de bloque del render offline, parecido al de un callback real.
pub const BLOCK_FRAMES: usize = 512;

/// Renderiza `frames` frames (entrelazados, `channels` canales).
pub fn render(engine: &mut Engine, frames: usize, channels: usize) -> Vec<f32> {
    let mut out = vec![0.0f32; frames * channels];
    for block in out.chunks_mut(BLOCK_FRAMES * channels) {
        engine.process(block, channels);
    }
    out
}

/// Renderiza hasta que el motor deja de reproducir, con `max_frames` como límite.
/// La longitud es múltiplo del bloque; lo que sobra tras el final es silencio.
pub fn render_until_stopped(
    controller: &Controller,
    engine: &mut Engine,
    channels: usize,
    max_frames: usize,
) -> Vec<f32> {
    let mut out = Vec::new();
    let mut block = vec![0.0f32; BLOCK_FRAMES * channels];
    while controller.is_playing() && out.len() < max_frames * channels {
        engine.process(&mut block, channels);
        out.extend_from_slice(&block);
    }
    out
}

/// Escribe audio entrelazado en un WAV PCM de 16 bits (con recorte a [-1, 1]).
pub fn write_wav(
    path: impl AsRef<Path>,
    samples: &[f32],
    channels: u16,
    sample_rate: u32,
) -> Result<(), hound::Error> {
    let spec = hound::WavSpec {
        channels,
        sample_rate,
        bits_per_sample: 16,
        sample_format: hound::SampleFormat::Int,
    };
    let mut writer = hound::WavWriter::create(path, spec)?;
    for &s in samples {
        writer.write_sample((s.clamp(-1.0, 1.0) * i16::MAX as f32).round() as i16)?;
    }
    writer.finalize()
}

/// Escribe un WAV de 32 bits en coma flotante: las muestras tal cual, sin
/// recortar ni redondear (al leerlo con [`crate::Sample::load_wav`] a la
/// misma frecuencia vuelven bit a bit). Para congelar pistas.
pub fn write_wav_float(
    path: impl AsRef<Path>,
    samples: &[f32],
    channels: u16,
    sample_rate: u32,
) -> Result<(), hound::Error> {
    let spec = hound::WavSpec {
        channels,
        sample_rate,
        bits_per_sample: 32,
        sample_format: hound::SampleFormat::Float,
    };
    let mut writer = hound::WavWriter::create(path, spec)?;
    for &s in samples {
        writer.write_sample(s)?;
    }
    writer.finalize()
}
