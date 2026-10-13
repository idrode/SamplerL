use std::fmt;
use std::path::Path;

use rubato::audioadapter_buffers::owned::InterleavedOwned;
use rubato::{Fft, FixedSync, Resampler};

/// Sample decodificado en memoria: estéreo entrelazado (L, R, L, R...) en f32,
/// ya resampleado a la frecuencia del motor.
pub struct Sample {
    data: Vec<f32>,
    sample_rate: u32,
}

#[derive(Debug)]
pub enum LoadError {
    Wav(hound::Error),
    Resample(String),
    Empty,
}

impl fmt::Display for LoadError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            LoadError::Wav(e) => write!(f, "error leyendo WAV: {e}"),
            LoadError::Resample(e) => write!(f, "error resampleando: {e}"),
            LoadError::Empty => write!(f, "el archivo no tiene audio"),
        }
    }
}

impl std::error::Error for LoadError {}

impl From<hound::Error> for LoadError {
    fn from(e: hound::Error) -> Self {
        LoadError::Wav(e)
    }
}

impl Sample {
    /// Carga un WAV y lo convierte a estéreo f32 a `target_rate`.
    pub fn load_wav(path: impl AsRef<Path>, target_rate: u32) -> Result<Self, LoadError> {
        let mut reader = hound::WavReader::open(path)?;
        let spec = reader.spec();
        let samples: Vec<f32> = match spec.sample_format {
            hound::SampleFormat::Float => reader.samples::<f32>().collect::<Result<_, _>>()?,
            hound::SampleFormat::Int => {
                let scale = 1.0 / (1u64 << (spec.bits_per_sample - 1)) as f32;
                reader
                    .samples::<i32>()
                    .map(|s| s.map(|v| v as f32 * scale))
                    .collect::<Result<_, _>>()?
            }
        };
        Self::from_interleaved(
            &samples,
            spec.channels as usize,
            spec.sample_rate,
            target_rate,
        )
    }

    /// Construye un sample desde audio entrelazado con cualquier número de canales.
    /// Mono se duplica a ambos lados; con más de dos canales se usan los dos primeros.
    pub fn from_interleaved(
        samples: &[f32],
        channels: usize,
        source_rate: u32,
        target_rate: u32,
    ) -> Result<Self, LoadError> {
        if channels == 0 || samples.len() < channels {
            return Err(LoadError::Empty);
        }
        let frames = samples.len() / channels;
        let mut stereo = Vec::with_capacity(frames * 2);
        for frame in samples.chunks_exact(channels) {
            let l = frame[0];
            let r = if channels > 1 { frame[1] } else { l };
            stereo.push(l);
            stereo.push(r);
        }

        let data = if source_rate == target_rate {
            stereo
        } else {
            resample(stereo, frames, source_rate, target_rate)?
        };
        Ok(Sample {
            data,
            sample_rate: target_rate,
        })
    }

    /// Sample desde estéreo entrelazado ya a la frecuencia del motor.
    pub(crate) fn from_raw(data: Vec<f32>, sample_rate: u32) -> Self {
        Sample { data, sample_rate }
    }

    pub fn frames(&self) -> usize {
        self.data.len() / 2
    }

    pub fn sample_rate(&self) -> u32 {
        self.sample_rate
    }

    /// Frame `i` como (izquierda, derecha).
    #[inline]
    pub fn frame(&self, i: usize) -> (f32, f32) {
        (self.data[2 * i], self.data[2 * i + 1])
    }
}

fn resample(stereo: Vec<f32>, frames: usize, from: u32, to: u32) -> Result<Vec<f32>, LoadError> {
    let err = |e: &dyn fmt::Display| LoadError::Resample(e.to_string());
    let mut resampler = Fft::<f32>::new(from as usize, to as usize, 1024, 2, FixedSync::Input)
        .map_err(|e| err(&e))?;
    let input = InterleavedOwned::new_from(stereo, 2, frames).map_err(|e| err(&e))?;
    let output = resampler
        .process_all(&input, frames, None)
        .map_err(|e| err(&e))?;
    // rubato calcula la longitud con un ratio en coma flotante y `ceil`, y puede
    // sobrar un frame; la longitud exacta se calcula con enteros.
    let expected = ((frames as u64 * to as u64 + from as u64 / 2) / from as u64) as usize;
    let mut data = output.take_data();
    data.resize(expected * 2, 0.0);
    Ok(data)
}
