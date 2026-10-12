//! Detección del tempo de un break: supone que el sample es un loop de
//! compases enteros de 4/4 (lo normal en un break cortado para samplear).
//!
//! Para 1, 2, 4 y 8 compases, el tempo candidato es `240 · compases /
//! duración` (entre `MIN_DETECTED_BPM` y `MAX_DETECTED_BPM`). Cada candidato
//! se puntúa por lo bien que caen los transitorios del sample en su rejilla
//! de semicorcheas (1 = todos justo en la rejilla). Gana el mejor; si dos
//! empatan (un break en corcheas encaja igual a 86 que a 172), el más
//! cercano a 130 BPM, y el otro queda como alternativa.

use crate::{Sample, Slices};

/// Rango de tempos que se proponen.
pub const MIN_DETECTED_BPM: f64 = 60.0;
pub const MAX_DETECTED_BPM: f64 = 200.0;
/// Diferencia de puntuación por debajo de la cual dos candidatos empatan.
const TIE: f64 = 0.05;
/// Transitorios que se miran.
const ONSETS: usize = 32;

/// Tempo detectado.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BpmGuess {
    pub bpm: f64,
    /// Compases que dura el sample a ese tempo.
    pub bars: u32,
    /// De 0 a 1: cuánto encajan los golpes en la rejilla de semicorcheas.
    pub score: f64,
    /// Otro tempo que encaja casi igual (tempo y compases), si lo hay.
    pub alternative: Option<(f64, u32)>,
}

impl BpmGuess {
    /// El tempo como se escribiría en `bpm()`: entero si está a menos de
    /// 0,05 de uno, si no con un decimal.
    pub fn rounded(&self) -> f64 {
        round_bpm(self.bpm)
    }
}

/// Tempo con un decimal, o entero si está muy cerca.
pub fn round_bpm(bpm: f64) -> f64 {
    if (bpm - bpm.round()).abs() < 0.05 {
        bpm.round()
    } else {
        (bpm * 10.0).round() / 10.0
    }
}

/// Detecta el tempo de un sample que es un loop de compases enteros.
/// `None` si no hay golpes o ningún número de compases da un tempo en rango.
pub fn detect_bpm(sample: &Sample) -> Option<BpmGuess> {
    let frames = sample.frames();
    if frames == 0 {
        return None;
    }
    let seconds = frames as f64 / sample.sample_rate() as f64;
    let onsets: Vec<usize> = Slices::detect(sample, ONSETS)
        .iter()
        .map(|r| r.start)
        .collect();
    if onsets.is_empty() {
        return None;
    }
    let mut candidates: Vec<(f64, u32, f64)> = [1u32, 2, 4, 8]
        .into_iter()
        .map(|bars| (240.0 * bars as f64 / seconds, bars))
        .filter(|(bpm, _)| (MIN_DETECTED_BPM..=MAX_DETECTED_BPM).contains(bpm))
        .map(|(bpm, bars)| {
            let grid = frames as f64 / (16.0 * bars as f64);
            let error: f64 = onsets
                .iter()
                .map(|&o| {
                    let x = o as f64 / grid;
                    (x - x.round()).abs()
                })
                .sum::<f64>()
                / onsets.len() as f64;
            (bpm, bars, 1.0 - 2.0 * error)
        })
        .collect();
    let best = candidates.iter().map(|c| c.2).fold(f64::MIN, f64::max);
    // Entre los que empatan con el mejor, el más cercano a 130 BPM.
    let closeness = |bpm: f64| (bpm / 130.0).ln().abs();
    candidates.retain(|c| best - c.2 < TIE);
    candidates.sort_by(|a, b| closeness(a.0).total_cmp(&closeness(b.0)));
    let (bpm, bars, score) = *candidates.first()?;
    Some(BpmGuess {
        bpm,
        bars,
        score,
        alternative: candidates.get(1).map(|c| (c.0, c.1)),
    })
}
