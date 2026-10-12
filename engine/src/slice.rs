//! Detección de transitorios y división del sample en cortes.

use std::ops::Range;

use crate::sample::Sample;

/// Tamaño del salto del análisis de energía (~5 ms a 48 kHz).
const HOP: usize = 256;
/// Distancia mínima entre dos transitorios.
const MIN_GAP_SECONDS: f32 = 0.040;
/// Fuerza mínima (subida de energía comprimida) para considerar un transitorio.
const MIN_STRENGTH: f32 = 0.5;
/// Compresión logarítmica de la envolvente: ln(1 + K·rms).
const LOG_K: f32 = 100.0;

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
        let frames = sample.frames();
        let mut starts = detect_onsets(sample, n);
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
fn detect_onsets(sample: &Sample, n: usize) -> Vec<usize> {
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

    // Los más fuertes primero, respetando la distancia mínima.
    let min_gap = ((MIN_GAP_SECONDS * sample.sample_rate() as f32) as usize).div_ceil(HOP);
    let mut chosen: Vec<usize> = Vec::with_capacity(n);
    for h in candidates {
        if chosen.len() == n {
            break;
        }
        if chosen.iter().all(|&c| c.abs_diff(h) >= min_gap) {
            chosen.push(h);
        }
    }

    let mut onsets: Vec<usize> = chosen
        .into_iter()
        .map(|h| refine(h, frames, &mono))
        .collect();
    onsets.sort_unstable();
    onsets.dedup();
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
