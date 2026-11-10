use std::fmt;
use std::path::Path;

use rubato::audioadapter_buffers::owned::InterleavedOwned;
use rubato::{Fft, FixedSync, Resampler};

/// Sample decodificado en memoria: estéreo entrelazado (L, R, L, R...) en f32,
/// ya resampleado a la frecuencia del motor.
pub struct Sample {
    data: Vec<f32>,
    sample_rate: u32,
    repairs: Repairs,
}

/// Nivel más alto que se acepta en un sample (+36 dBFS): por encima, un WAV
/// en coma flotante trae basura (`1e30`), que desbordaría el resampleo o
/// los filtros. Se recorta a este valor.
pub const MAX_SAMPLE_LEVEL: f32 = 64.0;

/// Lo que hubo que arreglar al cargar un sample (ver
/// [`Sample::from_interleaved`]). Con un archivo normal, todo a cero.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Repairs {
    /// Muestras NaN o infinitas, puestas a 0.
    pub non_finite: usize,
    /// Muestras por encima de [`MAX_SAMPLE_LEVEL`], recortadas.
    pub clamped: usize,
}

impl Repairs {
    pub fn any(&self) -> bool {
        self.non_finite > 0 || self.clamped > 0
    }
}

impl fmt::Display for Repairs {
    /// «3 muestras no válidas (NaN o infinito) puestas a 0», para un aviso.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let plural = |n: usize| if n == 1 { "" } else { "s" };
        let mut parts = Vec::new();
        if self.non_finite > 0 {
            let n = self.non_finite;
            parts.push(format!(
                "{n} muestra{} no válida{} (NaN o infinito) puesta{} a 0",
                plural(n),
                plural(n),
                plural(n)
            ));
        }
        if self.clamped > 0 {
            let n = self.clamped;
            parts.push(format!(
                "{n} muestra{} fuera de rango (más de {MAX_SAMPLE_LEVEL}) recortada{}",
                plural(n),
                plural(n)
            ));
        }
        f.write_str(&parts.join(" y "))
    }
}

/// Deja `x` en algo que el motor puede tocar: lo no finito a 0, lo enorme
/// recortado y los subnormales a 0 (inaudibles, y lentos en los filtros).
/// Lo normal no cambia.
fn sanitize(x: f32, repairs: &mut Repairs) -> f32 {
    if !x.is_finite() {
        repairs.non_finite += 1;
        0.0
    } else if x.abs() > MAX_SAMPLE_LEVEL {
        repairs.clamped += 1;
        MAX_SAMPLE_LEVEL.copysign(x)
    } else if x.is_subnormal() {
        0.0
    } else {
        x
    }
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
                // 0 bits desbordaría el desplazamiento (y no es audio).
                if !(1..=32).contains(&spec.bits_per_sample) {
                    return Err(LoadError::Wav(hound::Error::Unsupported));
                }
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
    ///
    /// Antes de resamplear se sanea: NaN e infinitos a 0, lo que pasa de
    /// [`MAX_SAMPLE_LEVEL`] recortado y los subnormales a 0. Un NaN rompía
    /// el resampleo (pánico de `rubato`) o, sin resampleo, se quedaba en
    /// los filtros y dejaba muda la mezcla. Cuántas se arreglaron, en
    /// [`Sample::repairs`] (la carga lo avisa).
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
        let mut repairs = Repairs::default();
        for frame in samples.chunks_exact(channels) {
            let l = sanitize(frame[0], &mut repairs);
            let r = if channels > 1 {
                sanitize(frame[1], &mut repairs)
            } else {
                l
            };
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
            repairs,
        })
    }

    /// Lo que hubo que arreglar al cargarlo (ver [`Sample::from_interleaved`]).
    pub fn repairs(&self) -> Repairs {
        self.repairs
    }

    /// Un frame de silencio: el sample de una pista que no toca samples
    /// (un sintetizador; ver `Pattern::with_synth`).
    pub fn silence(sample_rate: u32) -> Self {
        Sample {
            data: vec![0.0; 2],
            sample_rate,
            repairs: Repairs::default(),
        }
    }

    /// Sample desde estéreo entrelazado ya a la frecuencia del motor.
    pub(crate) fn from_raw(data: Vec<f32>, sample_rate: u32) -> Self {
        Sample {
            data,
            sample_rate,
            repairs: Repairs::default(),
        }
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

/// Cómo se prepara un sample al importarlo (ver [`Sample::processed`]).
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct ImportOptions {
    /// Recortar el silencio del principio y del final: lo que queda por
    /// debajo de este nivel (dBFS).
    pub trim_db: Option<f32>,
    /// Llevar el pico a este nivel (dBFS).
    pub normalize_db: Option<f32>,
    /// Fundidos de entrada y de salida, en ms (0: ninguno).
    pub fade_in_ms: f32,
    pub fade_out_ms: f32,
}

impl ImportOptions {
    /// Nivel por defecto de `trim` y de `normalize` cuando solo se pide.
    pub const TRIM_DB: f32 = -60.0;
    pub const NORMALIZE_DB: f32 = 0.0;

    pub fn is_empty(&self) -> bool {
        *self == ImportOptions::default()
    }

    pub fn validate(&self) -> Result<(), &'static str> {
        if self.trim_db.is_some_and(|db| !(-120.0..=0.0).contains(&db)) {
            return Err("trim fuera de rango (-120 a 0 dB)");
        }
        if self
            .normalize_db
            .is_some_and(|db| !(-60.0..=0.0).contains(&db))
        {
            return Err("normalize fuera de rango (-60 a 0 dB)");
        }
        if !(0.0..=10_000.0).contains(&self.fade_in_ms)
            || !(0.0..=10_000.0).contains(&self.fade_out_ms)
        {
            return Err("fundido fuera de rango (0 a 10000 ms)");
        }
        Ok(())
    }
}

impl Sample {
    /// El sample preparado: primero se recorta el silencio de los extremos
    /// (`trim_db`), luego se normaliza el pico (`normalize_db`) y al final
    /// se aplican los fundidos (lineales). Sin opciones, una copia exacta.
    /// Si todo es silencio, el recorte no quita nada.
    pub fn processed(&self, options: &ImportOptions) -> Sample {
        let level = |i: usize| {
            let (l, r) = self.frame(i);
            l.abs().max(r.abs())
        };
        let frames = self.frames();
        let (mut start, mut end) = (0, frames);
        if let Some(db) = options.trim_db {
            let threshold = 10f32.powf(db / 20.0);
            if let Some(first) = (0..frames).find(|&i| level(i) > threshold) {
                let last = (0..frames).rev().find(|&i| level(i) > threshold).unwrap();
                (start, end) = (first, last + 1);
            }
        }
        let mut data = self.data[2 * start..2 * end].to_vec();
        if let Some(db) = options.normalize_db {
            let peak = data.iter().fold(0.0f32, |m, x| m.max(x.abs()));
            if peak > 0.0 {
                let gain = 10f32.powf(db / 20.0) / peak;
                for x in &mut data {
                    *x *= gain;
                }
            }
        }
        let len = data.len() / 2;
        let ms = |ms: f32| ((ms / 1000.0 * self.sample_rate as f32) as usize).min(len);
        let (fade_in, fade_out) = (ms(options.fade_in_ms), ms(options.fade_out_ms));
        for (i, frame) in data.as_chunks_mut::<2>().0.iter_mut().enumerate() {
            let mut gain = 1.0f32;
            if i < fade_in {
                gain *= i as f32 / fade_in as f32;
            }
            if len - i <= fade_out {
                gain *= (len - 1 - i) as f32 / fade_out as f32;
            }
            if gain != 1.0 {
                frame[0] *= gain;
                frame[1] *= gain;
            }
        }
        Sample {
            data,
            sample_rate: self.sample_rate,
            repairs: self.repairs,
        }
    }
}
