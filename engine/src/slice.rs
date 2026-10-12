//! Detección de transitorios y división del sample en cortes.

use std::ops::Range;

use crate::sample::Sample;

/// Tamaño del salto del análisis de energía (~5 ms a 48 kHz).
const HOP: usize = 256;
/// Separación mínima por defecto entre dos transitorios.
pub const DEFAULT_MIN_GAP_SECONDS: f32 = 0.040;
/// Fuerza mínima (subida de energía comprimida) para considerar un transitorio.
const MIN_STRENGTH: f32 = 0.5;
/// Compresión logarítmica de la envolvente: ln(1 + K·rms).
const LOG_K: f32 = 100.0;

/// Parámetros de la detección de transitorios.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DetectOptions {
    /// Separación mínima entre dos transitorios, en segundos. De dos golpes
    /// más cercanos se queda el más fuerte.
    pub min_gap_seconds: f32,
}

impl Default for DetectOptions {
    fn default() -> Self {
        DetectOptions {
            min_gap_seconds: DEFAULT_MIN_GAP_SECONDS,
        }
    }
}

/// Cortes de un sample: rangos de frames contiguos, ordenados.
/// El corte `i` va desde su transitorio hasta el siguiente (o el final del sample).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Slices(Vec<Range<usize>>);

impl Slices {
    /// Busca hasta `n` transitorios (los más fuertes) y corta el sample en ellos.
    /// Si hay menos transitorios que `n`, devuelve menos cortes; si no hay
    /// ninguno, un único corte con el sample entero. Lo que haya antes del
    /// primer transitorio no pertenece a ningún corte.
    pub fn detect(sample: &Sample, n: usize) -> Self {
        Self::detect_with(sample, n, &DetectOptions::default())
    }

    /// Como [`Slices::detect`], con parámetros de detección propios.
    pub fn detect_with(sample: &Sample, n: usize, options: &DetectOptions) -> Self {
        let frames = sample.frames();
        let mut starts = detect_onsets(sample, n, options);
        if starts.is_empty() && frames > 0 && n > 0 {
            starts.push(0);
        }
        let ranges = starts
            .iter()
            .enumerate()
            .map(|(i, &start)| start..starts.get(i + 1).copied().unwrap_or(frames))
            .collect();
        Slices(ranges)
    }

    pub fn len(&self) -> usize {
        self.0.len()
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    pub fn get(&self, index: usize) -> Option<Range<usize>> {
        self.0.get(index).cloned()
    }

    pub fn iter(&self) -> impl Iterator<Item = Range<usize>> + '_ {
        self.0.iter().cloned()
    }
}

/// Posiciones (en frames) de hasta `n` transitorios, ordenadas.
fn detect_onsets(sample: &Sample, n: usize, options: &DetectOptions) -> Vec<usize> {
    let frames = sample.frames();
    if n == 0 || frames == 0 {
        return Vec::new();
    }
    let mono = |i: usize| {
        let (l, r) = sample.frame(i);
        0.5 * (l + r)
    };

    // Envolvente: RMS por salto, comprimida logarítmicamente.
    let hops = frames.div_ceil(HOP);
    let envelope: Vec<f32> = (0..hops)
        .map(|h| {
            let range = h * HOP..((h + 1) * HOP).min(frames);
            let len = range.len() as f32;
            let rms = (range.map(|i| mono(i).powi(2)).sum::<f32>() / len).sqrt();
            (1.0 + LOG_K * rms).ln()
        })
        .collect();

    // Función de detección: subida positiva de la envolvente. Antes del
    // sample se asume silencio, así que un golpe en el frame 0 cuenta.
    let flux: Vec<f32> = (0..hops)
        .map(|h| {
            let prev = if h > 0 { envelope[h - 1] } else { 0.0 };
            (envelope[h] - prev).max(0.0)
        })
        .collect();

    // Candidatos: máximos locales por encima del umbral.
    let mut candidates: Vec<usize> = (0..hops)
        .filter(|&h| {
            flux[h] >= MIN_STRENGTH
                && (h == 0 || flux[h] >= flux[h - 1])
                && (h + 1 == hops || flux[h] > flux[h + 1])
        })
        .collect();
    candidates.sort_by(|&a, &b| flux[b].total_cmp(&flux[a]));

    // Los más fuertes primero, respetando la separación mínima. Se compara
    // la posición ya afinada, en frames, para que la separación sea exacta.
    let min_gap_frames = (options.min_gap_seconds.max(0.0) * sample.sample_rate() as f32) as usize;
    let min_gap_frames = min_gap_frames.max(1);
    let mut onsets: Vec<usize> = Vec::with_capacity(n);
    for h in candidates {
        if onsets.len() == n {
            break;
        }
        let onset = refine(h, frames, &mono);
        if onsets.iter().all(|&o| o.abs_diff(onset) >= min_gap_frames) {
            onsets.push(onset);
        }
    }
    onsets.sort_unstable();
    onsets
}

/// Afina un transitorio detectado en el salto `h` hasta el frame donde empieza
/// el ataque: el primero que supera una fracción del pico local y claramente
/// el nivel previo.
fn refine(h: usize, frames: usize, mono: &impl Fn(usize) -> f32) -> usize {
    let peak_in = |range: Range<usize>| range.map(|i| mono(i).abs()).fold(0.0f32, f32::max);

    // La subida está entre el final del salto anterior y el final de este.
    let start = h.saturating_sub(1) * HOP;
    let end = ((h + 1) * HOP).min(frames);
    let peak = peak_in(start..end);
    let before = if h >= 2 {
        peak_in((h - 2) * HOP..(h - 1) * HOP)
    } else {
        0.0
    };
    let threshold = (0.1 * peak).max(2.0 * before).min(0.5 * peak);

    (start..end)
        .find(|&i| mono(i).abs() >= threshold)
        .unwrap_or(h * HOP)
}
