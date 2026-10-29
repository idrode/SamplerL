//! Tonalidad de un sample: cromagrama (energía de cada clase de nota, de
//! C a B) y correlación con los perfiles mayor y menor de Krumhansl y
//! Kessler en las 24 tonalidades. Fuera del hilo de audio (se analiza al
//! pedirlo).
//!
//! El cromagrama se saca de FFT de 16384 puntos (2,9 Hz por bin a 48 kHz,
//! menos que el semitono más estrecho que se mira, 3,9 Hz en C2) con
//! ventana de Hann y salto de media ventana. De cada ventana solo cuentan
//! los picos del espectro (máximos locales) entre C2 y C8: así la fuga de
//! la ventana no reparte un tono entre sus vecinos. Cada pico suma su
//! amplitud a la nota más cercana. Una ventana más débil que 60 dB por
//! debajo de la más fuerte no cuenta (silencio y colas).

use rustfft::FftPlanner;
use rustfft::num_complex::Complex;

use crate::sample::Sample;

/// Puntos de la FFT.
const SIZE: usize = 16384;
/// Lo que se analiza como mucho (el resto del sample no cuenta).
const MAX_SECONDS: f32 = 60.0;
/// Notas MIDI que cuentan: de C2 a C8.
const LOWEST: i32 = 36;
const HIGHEST: i32 = 108;
/// Una ventana que queda esto por debajo de la más fuerte no cuenta.
const FLOOR_DB: f32 = -60.0;
/// Por debajo de esta confianza se avisa: el material no tiene una
/// tonalidad clara (percusión, ruido, una sola nota...).
pub const LOW_CONFIDENCE: f32 = 0.5;

/// Perfiles de Krumhansl y Kessler (1982): lo que pesa cada grado
/// cromático desde la tónica en una tonalidad mayor y en una menor.
const MAJOR: [f32; 12] = [
    6.35, 2.23, 3.48, 2.33, 4.38, 4.09, 2.52, 5.19, 2.39, 3.66, 2.29, 2.88,
];
const MINOR: [f32; 12] = [
    6.33, 2.68, 3.52, 5.38, 2.60, 3.53, 2.54, 4.75, 3.98, 2.69, 3.34, 3.17,
];

/// Nombres de las clases de nota (con sostenidos, como `note_name`).
const NAMES: [&str; 12] = [
    "C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "A#", "B",
];

/// Una tonalidad candidata.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct KeyGuess {
    /// Tónica, de 0 (C) a 11 (B).
    pub tonic: u8,
    /// Menor (si no, mayor).
    pub minor: bool,
    /// Correlación del cromagrama con el perfil (de -1 a 1).
    pub score: f32,
}

impl KeyGuess {
    /// Nombre de la tónica (`"A"`, `"F#"`).
    pub fn tonic_name(&self) -> &'static str {
        NAMES[self.tonic as usize % 12]
    }

    /// El modo como lo entiende `key()` y `scale()`.
    pub fn mode(&self) -> &'static str {
        if self.minor { "minor" } else { "major" }
    }

    /// Para leer: «A menor», «C mayor».
    pub fn label(&self) -> String {
        let mode = if self.minor { "menor" } else { "mayor" };
        format!("{} {mode}", self.tonic_name())
    }
}

/// Resultado del análisis.
#[derive(Debug, Clone, PartialEq)]
pub struct KeyAnalysis {
    /// Las tres tonalidades más probables, de más a menos.
    pub candidates: [KeyGuess; 3],
    /// De 0 a 1: la correlación de la mejor, rebajada si la segunda (que
    /// no sea su relativa) se le acerca. Por debajo de [`LOW_CONFIDENCE`],
    /// poco fiable.
    pub confidence: f32,
    /// La tónica en la octava donde más suena (nota MIDI): la raíz que se
    /// propone para tocar el sample como instrumento.
    pub root: i32,
    /// Cromagrama normalizado (suma 1), de C a B.
    pub chroma: [f32; 12],
}

impl KeyAnalysis {
    /// La más probable.
    pub fn best(&self) -> KeyGuess {
        self.candidates[0]
    }

    /// Poco fiable (material percusivo o sin una tonalidad clara).
    pub fn low_confidence(&self) -> bool {
        self.confidence < LOW_CONFIDENCE
    }
}

/// Analiza la tonalidad de `sample`. `None` si no hay nada que analizar
/// (silencio, o más corto que una décima de segundo).
pub fn detect_key(sample: &Sample) -> Option<KeyAnalysis> {
    let rate = sample.sample_rate() as f32;
    let frames = sample.frames().min((MAX_SECONDS * rate) as usize);
    if frames < (rate / 10.0) as usize {
        return None;
    }
    let mono: Vec<f32> = (0..frames)
        .map(|i| {
            let (l, r) = sample.frame(i);
            0.5 * (l + r)
        })
        .collect();
    let notes = note_energy(&mono, rate)?;
    let mut chroma = [0.0f32; 12];
    for (k, &e) in notes.iter().enumerate() {
        chroma[(LOWEST as usize + k) % 12] += e;
    }
    let total: f32 = chroma.iter().sum();
    if total <= 0.0 {
        return None;
    }
    for c in &mut chroma {
        *c /= total;
    }
    let mut guesses: Vec<KeyGuess> = (0..12u8)
        .flat_map(|tonic| {
            [false, true].map(|minor| KeyGuess {
                tonic,
                minor,
                score: correlation(&chroma, if minor { &MINOR } else { &MAJOR }, tonic),
            })
        })
        .collect();
    guesses.sort_by(|a, b| b.score.total_cmp(&a.score));
    let best = guesses[0];
    // La relativa (A menor de C mayor) comparte las notas: que se acerque
    // no quita confianza. La siguiente que no lo sea, sí.
    let relative = |g: &KeyGuess| {
        g.minor != best.minor
            && if best.minor {
                g.tonic == (best.tonic + 3) % 12
            } else {
                g.tonic == (best.tonic + 9) % 12
            }
    };
    let rival = guesses[1..]
        .iter()
        .find(|g| !relative(g))
        .map_or(0.0, |g| g.score);
    // Con la rival a menos de 0,1 de la mejor, la confianza baja en
    // proporción (a la par, la mitad).
    let margin = ((best.score - rival) / 0.1).clamp(0.0, 1.0);
    let confidence = best.score.clamp(0.0, 1.0) * (0.5 + 0.5 * margin);
    // La tónica en la octava donde más suena.
    let root = (0..notes.len())
        .filter(|k| (LOWEST as usize + k) % 12 == best.tonic as usize)
        .max_by(|&a, &b| notes[a].total_cmp(&notes[b]))
        .map_or(60, |k| LOWEST + k as i32);
    Some(KeyAnalysis {
        candidates: [guesses[0], guesses[1], guesses[2]],
        confidence,
        root,
        chroma,
    })
}

/// Amplitud acumulada en cada nota de `LOWEST` a `HIGHEST` (solo los picos
/// del espectro). `None` si todo es silencio.
fn note_energy(mono: &[f32], rate: f32) -> Option<Vec<f32>> {
    let fft = FftPlanner::<f32>::new().plan_fft_forward(SIZE);
    let window: Vec<f32> = (0..SIZE)
        .map(|i| 0.5 - 0.5 * (std::f32::consts::TAU * i as f32 / SIZE as f32).cos())
        .collect();
    let hop = SIZE / 2;
    // Al menos una ventana, aunque el sample sea más corto (con ceros).
    let starts: Vec<usize> = (0..mono.len().saturating_sub(SIZE).max(1))
        .step_by(hop)
        .collect();
    let mut spectra: Vec<(f32, Vec<f32>)> = Vec::with_capacity(starts.len());
    let mut buf = vec![Complex::new(0.0f32, 0.0); SIZE];
    for &start in &starts {
        for (i, c) in buf.iter_mut().enumerate() {
            let x = mono.get(start + i).copied().unwrap_or(0.0);
            *c = Complex::new(x * window[i], 0.0);
        }
        fft.process(&mut buf);
        let mags: Vec<f32> = buf[..SIZE / 2].iter().map(|c| c.norm()).collect();
        let energy: f32 = mags.iter().map(|m| m * m).sum();
        spectra.push((energy, mags));
    }
    let loudest = spectra.iter().map(|(e, _)| *e).fold(0.0f32, f32::max);
    if loudest <= 1e-12 {
        return None;
    }
    let floor = loudest * 10f32.powf(FLOOR_DB / 10.0);
    let bin_hz = rate / SIZE as f32;
    let mut notes = vec![0.0f32; (HIGHEST - LOWEST + 1) as usize];
    for (_, mags) in spectra.iter().filter(|(e, _)| *e >= floor) {
        let low = (midi_hz(LOWEST as f32 - 0.5) / bin_hz).floor().max(1.0) as usize;
        let high = ((midi_hz(HIGHEST as f32 + 0.5) / bin_hz).ceil() as usize).min(mags.len() - 2);
        // Lo más fuerte de la ventana: los picos 60 dB por debajo, fuera.
        let top = mags[low..=high].iter().copied().fold(0.0f32, f32::max);
        for b in low..=high {
            let m = mags[b];
            if m <= mags[b - 1] || m < mags[b + 1] || m < top * 1e-3 {
                continue;
            }
            // Interpolación parabólica del pico (en dB) para afinar la
            // frecuencia entre bins.
            let (a, c) = (db(mags[b - 1]), db(mags[b + 1]));
            let d = 0.5 * (a - c) / (a - 2.0 * db(m) + c);
            let hz = (b as f32 + if d.is_finite() { d } else { 0.0 }) * bin_hz;
            let midi = hz_midi(hz).round() as i32;
            if (LOWEST..=HIGHEST).contains(&midi) {
                notes[(midi - LOWEST) as usize] += m;
            }
        }
    }
    Some(notes)
}

fn db(m: f32) -> f32 {
    20.0 * m.max(1e-12).log10()
}

fn midi_hz(midi: f32) -> f32 {
    440.0 * 2f32.powf((midi - 69.0) / 12.0)
}

fn hz_midi(hz: f32) -> f32 {
    69.0 + 12.0 * (hz.max(1e-6) / 440.0).log2()
}

/// Correlación de Pearson del cromagrama con el perfil girado a `tonic`.
fn correlation(chroma: &[f32; 12], profile: &[f32; 12], tonic: u8) -> f32 {
    let shifted: [f32; 12] = std::array::from_fn(|pc| profile[(pc + 12 - tonic as usize) % 12]);
    let mean = |v: &[f32; 12]| v.iter().sum::<f32>() / 12.0;
    let (ma, mb) = (mean(chroma), mean(&shifted));
    let (mut num, mut va, mut vb) = (0.0f32, 0.0f32, 0.0f32);
    for i in 0..12 {
        let (a, b) = (chroma[i] - ma, shifted[i] - mb);
        num += a * b;
        va += a * a;
        vb += b * b;
    }
    if va <= 0.0 || vb <= 0.0 {
        return 0.0;
    }
    num / (va.sqrt() * vb.sqrt())
}
