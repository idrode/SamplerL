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
    let mut out = WavOut::create(path, channels, sample_rate, WavFormat::Int16, false)?;
    out.write(samples)?;
    out.finalize()
}

/// Formato de un WAV exportado (ver [`WavOut`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum WavFormat {
    /// PCM de 16 bits, con recorte a [-1, 1] (el de siempre).
    #[default]
    Int16,
    /// PCM de 24 bits, con recorte a [-1, 1].
    Int24,
    /// 32 bits en coma flotante: las muestras tal cual.
    Float32,
}

impl WavFormat {
    /// `16`, `24` o `32f` (como en `--bits` y en `exportar`).
    pub fn parse(text: &str) -> Option<WavFormat> {
        match text {
            "16" => Some(WavFormat::Int16),
            "24" => Some(WavFormat::Int24),
            "32f" => Some(WavFormat::Float32),
            _ => None,
        }
    }

    /// «16 bits», «24 bits», «32 bits en coma flotante».
    pub fn label(self) -> &'static str {
        match self {
            WavFormat::Int16 => "16 bits",
            WavFormat::Int24 => "24 bits",
            WavFormat::Float32 => "32 bits en coma flotante",
        }
    }

    /// Bytes por segundo de audio estéreo a `sample_rate`.
    pub fn stereo_bytes_per_second(self, sample_rate: u32) -> u64 {
        let bytes = match self {
            WavFormat::Int16 => 2,
            WavFormat::Int24 => 3,
            WavFormat::Float32 => 4,
        };
        2 * bytes * sample_rate as u64
    }
}

/// Un WAV que se escribe a trozos (el render no necesita tener toda la
/// mezcla en memoria). Con `dither`, ruido TPDF de un escalón (LSB) antes
/// de redondear, en 16 y 24 bits; determinista (la misma semilla siempre),
/// así dos renders iguales dan el mismo archivo.
pub struct WavOut {
    writer: hound::WavWriter<std::io::BufWriter<std::fs::File>>,
    format: WavFormat,
    dither: Option<u64>,
}

impl WavOut {
    pub fn create(
        path: impl AsRef<Path>,
        channels: u16,
        sample_rate: u32,
        format: WavFormat,
        dither: bool,
    ) -> Result<WavOut, hound::Error> {
        let (bits_per_sample, sample_format) = match format {
            WavFormat::Int16 => (16, hound::SampleFormat::Int),
            WavFormat::Int24 => (24, hound::SampleFormat::Int),
            WavFormat::Float32 => (32, hound::SampleFormat::Float),
        };
        let spec = hound::WavSpec {
            channels,
            sample_rate,
            bits_per_sample,
            sample_format,
        };
        Ok(WavOut {
            writer: hound::WavWriter::create(path, spec)?,
            format,
            dither: (dither && format != WavFormat::Float32).then_some(0x9E37_79B9_7F4A_7C15),
        })
    }

    /// Un número de 0 a 1 del generador del dither (xorshift).
    fn uniform(state: &mut u64) -> f32 {
        *state ^= *state << 13;
        *state ^= *state >> 7;
        *state ^= *state << 17;
        (*state >> 40) as f32 / (1u64 << 24) as f32
    }

    pub fn write(&mut self, samples: &[f32]) -> Result<(), hound::Error> {
        for &s in samples {
            match self.format {
                WavFormat::Float32 => self.writer.write_sample(s)?,
                WavFormat::Int16 | WavFormat::Int24 => {
                    let full = if self.format == WavFormat::Int16 {
                        i16::MAX as f32
                    } else {
                        8_388_607.0
                    };
                    let mut x = s.clamp(-1.0, 1.0) * full;
                    if let Some(state) = &mut self.dither {
                        // Triangular de ±1 escalón: la resta de dos uniformes.
                        x += Self::uniform(state) - Self::uniform(state);
                    }
                    let x = x.round().clamp(-full, full);
                    if self.format == WavFormat::Int16 {
                        self.writer.write_sample(x as i16)?;
                    } else {
                        self.writer.write_sample(x as i32)?;
                    }
                }
            }
        }
        Ok(())
    }

    pub fn finalize(self) -> Result<(), hound::Error> {
        self.writer.finalize()
    }
}

/// Renderiza `frames` frames estéreo y los escribe en `out` bloque a bloque
/// (los mismos bloques que [`render`]: el mismo audio), sin guardar la
/// mezcla entera en memoria.
pub fn render_into(
    engine: &mut Engine,
    frames: usize,
    out: &mut WavOut,
) -> Result<(), hound::Error> {
    let mut block = vec![0.0f32; BLOCK_FRAMES * 2];
    let mut left = frames;
    while left > 0 {
        let n = left.min(BLOCK_FRAMES);
        let block = &mut block[..2 * n];
        engine.process(block, 2);
        out.write(block)?;
        left -= n;
    }
    Ok(())
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
