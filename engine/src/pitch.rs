//! Detección de la nota de un trozo de sample (un corte de una melodía, una
//! nota suelta de un bajo...), para saber qué toca cada corte al componer.
//!
//! Es YIN (de Cheveigné y Kawahara, 2002) por ventanas: para cada ventana,
//! la función de diferencia `d(τ) = Σ (x_j − x_{j+τ})²` (calculada con la
//! FFT), normalizada por su media acumulada, y el primer mínimo por debajo
//! de `THRESHOLD` da el periodo (afinado con una parábola). Las ventanas sin
//! periodo claro (ruido, el golpe de una caja) no votan. La nota es la
//! mediana de las de las ventanas con periodo, pesada por su nivel y su
//! claridad; si votan pocas (un golpe de batería), no hay nota.
//!
//! No corre en el hilo de audio: reserva memoria y se llama al cargar.

use std::ops::Range;

use crate::Sample;

/// Frecuencias que se buscan: de un mi grave de bajo (E1, 41 Hz) a un do
/// agudo (C7, 2093 Hz).
pub const MIN_HZ: f32 = 40.0;
pub const MAX_HZ: f32 = 2100.0;
/// Umbral de YIN sobre la diferencia normalizada: por debajo, hay periodo.
const THRESHOLD: f32 = 0.15;
/// Ventanas que se miran como mucho en un trozo.
const MAX_WINDOWS: usize = 48;
/// Se salta el principio del trozo (el ataque no tiene tono claro), si cabe.
const SKIP_SECONDS: f32 = 0.015;
/// Las ventanas con menos nivel que esta fracción de la más fuerte no
/// cuentan (la cola que se apaga, el silencio).
const QUIET: f32 = 0.1;
/// Fracción mínima del peso (nivel) de las ventanas que suenan que tiene
/// que tener periodo para que haya nota.
const MIN_VOICED: f32 = 0.5;

/// Nombres de las notas en cifrado americano, con sostenidos.
const NAMES: [&str; 12] = [
    "C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "A#", "B",
];

/// Nombre de una nota MIDI en cifrado americano: `60` → `"C4"`, `61` →
/// `"C#4"` (C4 = 60, como en `melody`).
pub fn note_name(midi: i32) -> String {
    format!(
        "{}{}",
        NAMES[midi.rem_euclid(12) as usize],
        midi.div_euclid(12) - 1
    )
}

/// Nota detectada.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct NoteGuess {
    /// Frecuencia fundamental, en Hz.
    pub hz: f32,
    /// Nota MIDI con decimales (69 = A4 = 440 Hz).
    pub midi: f32,
    /// De 0 a 1: cuánto del trozo tiene ese tono claro.
    pub clarity: f32,
}

impl NoteGuess {
    /// La nota MIDI más cercana.
    pub fn note(&self) -> i32 {
        self.midi.round() as i32
    }

    /// Desafinación respecto a [`NoteGuess::note`], en cents (−50 a 50).
    pub fn cents(&self) -> i32 {
        ((self.midi - self.midi.round()) * 100.0).round() as i32
    }

    /// Nombre de la nota más cercana: `"C#4"`.
    pub fn name(&self) -> String {
        note_name(self.note())
    }
}

/// Nota MIDI (con decimales) de una frecuencia.
pub fn hz_to_midi(hz: f32) -> f32 {
    69.0 + 12.0 * (hz / 440.0).log2()
}

/// Nota del trozo `range` (en frames) del sample, o `None` si no tiene un
/// tono claro (un golpe de batería, ruido, silencio) o es demasiado corto.
pub fn detect_note(sample: &Sample, range: Range<usize>) -> Option<NoteGuess> {
    let end = range.end.min(sample.frames());
    let mono: Vec<f32> = (range.start.min(end)..end)
        .map(|i| {
            let (l, r) = sample.frame(i);
            0.5 * (l + r)
        })
        .collect();
    detect_mono(&mono, sample.sample_rate())
}

/// La nota de cada corte, en orden.
pub fn detect_notes(sample: &Sample, slices: &crate::Slices) -> Vec<Option<NoteGuess>> {
    slices.iter().map(|r| detect_note(sample, r)).collect()
}

/// [`detect_note`] sobre una señal mono a `rate` Hz.
pub fn detect_mono(mono: &[f32], rate: u32) -> Option<NoteGuess> {
    use rustfft::FftPlanner;
    use rustfft::num_complex::Complex;

    let rate_f = rate as f32;
    let tau_min = ((rate_f / MAX_HZ).floor() as usize).max(2);
    // La ventana dura el periodo más largo que se busca; si el trozo es
    // corto, se buscan periodos más cortos (dos ventanas tienen que caber).
    let tau_max = ((rate_f / MIN_HZ).ceil() as usize).min(mono.len() / 2);
    if tau_max < 2 * tau_min {
        return None;
    }
    let window = tau_max;
    let span = window + tau_max;
    let skip = ((SKIP_SECONDS * rate_f) as usize).min(mono.len() - span);
    let room = mono.len() - span - skip;
    let count = (room / (window / 2).max(1) + 1).min(MAX_WINDOWS);
    let starts: Vec<usize> = (0..count)
        .map(|k| skip + if count > 1 { k * room / (count - 1) } else { 0 })
        .collect();

    let size = (span + window).next_power_of_two();
    let mut planner = FftPlanner::<f32>::new();
    let forward = planner.plan_fft_forward(size);
    let inverse = planner.plan_fft_inverse(size);
    let mut a = vec![Complex::new(0.0f32, 0.0); size];
    let mut b = vec![Complex::new(0.0f32, 0.0); size];
    let mut prefix = vec![0.0f64; span + 1];
    let mut cmnd = vec![0.0f32; tau_max + 1];

    // (nota MIDI, peso) de las ventanas con periodo y el nivel de todas.
    let mut votes: Vec<(f32, f32, f32)> = Vec::with_capacity(count);
    let mut levels: Vec<f32> = Vec::with_capacity(count);
    for &start in &starts {
        let x = &mono[start..start + span];
        for (i, slot) in a.iter_mut().enumerate() {
            *slot = Complex::new(if i < window { x[i] } else { 0.0 }, 0.0);
        }
        for (i, slot) in b.iter_mut().enumerate() {
            *slot = Complex::new(x.get(i).copied().unwrap_or(0.0), 0.0);
        }
        forward.process(&mut a);
        forward.process(&mut b);
        for (p, q) in a.iter_mut().zip(&b) {
            *p = p.conj() * q;
        }
        // `a[τ]` es ahora `size · Σ_{j<window} x_j · x_{j+τ}`.
        inverse.process(&mut a);
        for (i, &s) in x.iter().enumerate() {
            prefix[i + 1] = prefix[i] + (s as f64) * (s as f64);
        }
        let energy = |from: usize| prefix[from + window] - prefix[from];
        let e0 = energy(0);
        let rms = (e0 / window as f64).sqrt() as f32;
        levels.push(rms);
        if e0 <= 1e-12 {
            continue;
        }
        // Diferencia normalizada por su media acumulada.
        let mut sum = 0.0f64;
        cmnd[0] = 1.0;
        for tau in 1..=tau_max {
            let r = a[tau].re as f64 / size as f64;
            let d = (e0 + energy(tau) - 2.0 * r).max(0.0);
            sum += d;
            cmnd[tau] = if sum > 0.0 {
                (d * tau as f64 / sum) as f32
            } else {
                1.0
            };
        }
        // El primer valle por debajo del umbral, hasta su fondo.
        let Some(mut tau) = (tau_min..tau_max).find(|&t| cmnd[t] < THRESHOLD) else {
            continue;
        };
        while tau + 1 < tau_max && cmnd[tau + 1] < cmnd[tau] {
            tau += 1;
        }
        let (l, c, r) = (cmnd[tau - 1], cmnd[tau], cmnd[tau + 1]);
        let bend = l - 2.0 * c + r;
        let offset = if bend > 0.0 {
            (0.5 * (l - r) / bend).clamp(-0.5, 0.5)
        } else {
            0.0
        };
        let hz = rate_f / (tau as f32 + offset);
        votes.push((hz_to_midi(hz), rms * (1.0 - c), rms));
    }

    let loudest = levels.iter().copied().fold(0.0f32, f32::max);
    if loudest <= 0.0 {
        return None;
    }
    let floor = QUIET * loudest;
    let sounding: f32 = levels.iter().filter(|&&l| l >= floor).sum();
    votes.retain(|v| v.2 >= floor);
    let voiced: f32 = votes.iter().map(|v| v.2).sum();
    if votes.is_empty() || voiced < MIN_VOICED * sounding {
        return None;
    }
    // Mediana pesada: si el trozo tiene dos notas, gana la que más suena.
    votes.sort_by(|p, q| p.0.total_cmp(&q.0));
    let total: f32 = votes.iter().map(|v| v.1).sum();
    let mut acc = 0.0;
    let midi = votes
        .iter()
        .find(|v| {
            acc += v.1;
            acc >= 0.5 * total
        })
        .map_or(votes[votes.len() / 2].0, |v| v.0);
    // La claridad: cuánto suena con periodo y cuán limpio es ese periodo.
    let purity = total / voiced;
    Some(NoteGuess {
        hz: 440.0 * 2f32.powf((midi - 69.0) / 12.0),
        midi,
        clarity: (voiced / sounding * purity).clamp(0.0, 1.0),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_follow_c4_is_60() {
        assert_eq!(note_name(60), "C4");
        assert_eq!(note_name(61), "C#4");
        assert_eq!(note_name(69), "A4");
        assert_eq!(note_name(28), "E1");
        assert_eq!(note_name(0), "C-1");
        assert_eq!(note_name(-1), "B-2");
    }

    #[test]
    fn cents_round_to_the_nearest_note() {
        let g = NoteGuess {
            hz: 0.0,
            midi: 60.3,
            clarity: 1.0,
        };
        assert_eq!((g.note(), g.cents(), g.name().as_str()), (60, 30, "C4"));
        let g = NoteGuess { midi: 60.7, ..g };
        assert_eq!((g.note(), g.cents(), g.name().as_str()), (61, -30, "C#4"));
    }

    #[test]
    fn too_short_or_empty_has_no_note() {
        assert_eq!(detect_mono(&[], 48_000), None);
        assert_eq!(detect_mono(&[0.0; 40], 48_000), None);
        assert_eq!(detect_mono(&vec![0.0; 4800], 48_000), None);
    }
}
