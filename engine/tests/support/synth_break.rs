//! Generador de golpes de batería sintéticos, compartido por los tests y por
//! el ejemplo `synth_break`. Determinista: mismo resultado en cada ejecución.

#![allow(dead_code)]

use std::f32::consts::TAU;

#[derive(Clone, Copy, Debug)]
pub enum Hit {
    /// Seno con barrido de 120 a 45 Hz y cola larga.
    Kick,
    /// Ruido más tono de 190 Hz.
    Snare,
    /// Ruido con paso alto simple, débil y corto.
    Hat,
}

/// Frecuencia del break sintético (distinta de la del motor a propósito,
/// para que la carga pase por el resampleo).
pub const RATE: u32 = 44_100;
/// Un compás en corcheas a 140 BPM.
pub const BPM: f32 = 140.0;
pub const BAR: [Hit; 8] = [
    Hit::Kick,
    Hit::Hat,
    Hit::Snare,
    Hit::Hat,
    Hit::Kick,
    Hit::Kick,
    Hit::Snare,
    Hit::Hat,
];

/// Frames entre dos corcheas del break.
pub fn eighth() -> usize {
    (RATE as f32 * 60.0 / BPM / 2.0) as usize
}

/// El break sintético (mono, a `RATE`) y la posición de cada golpe en frames.
pub fn synth_break() -> (Vec<f32>, Vec<usize>) {
    let hits: Vec<(Hit, usize, f32)> = BAR
        .iter()
        .enumerate()
        .map(|(n, &hit)| (hit, n * eighth(), 1.0))
        .collect();
    let audio = render_hits(RATE, &hits, eighth() * BAR.len());
    (audio, hits.iter().map(|&(_, start, _)| start).collect())
}

/// Suma los golpes `(tipo, frame de inicio, ganancia)` en un buffer mono de
/// `frames`.
pub fn render_hits(rate: u32, hits: &[(Hit, usize, f32)], frames: usize) -> Vec<f32> {
    let mut audio = vec![0.0f32; frames];
    let mut noise = Noise(0x5eed_1234);
    for &(hit, start, gain) in hits {
        let mut phase = 0.0f32;
        let mut previous = 0.0f32;
        for (i, s) in audio[start..].iter_mut().enumerate() {
            let t = i as f32 / rate as f32;
            let attack = (t / 0.0005).min(1.0);
            let white = noise.next();
            let voice = match hit {
                Hit::Kick => {
                    phase += TAU * (45.0 + 75.0 * (-t / 0.03).exp()) / rate as f32;
                    0.9 * (-t / 0.15).exp() * phase.sin()
                }
                Hit::Snare => {
                    let tone = (TAU * 190.0 * t).sin();
                    (-t / 0.09).exp() * (0.55 * white + 0.25 * tone)
                }
                Hit::Hat => {
                    let high = white - previous;
                    previous = white;
                    0.18 * (-t / 0.03).exp() * high
                }
            };
            *s += gain * attack * voice;
        }
    }
    audio
}

/// Generador pseudoaleatorio determinista (xorshift32) en [-1, 1).
struct Noise(u32);

impl Noise {
    fn next(&mut self) -> f32 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 17;
        self.0 ^= self.0 << 5;
        (self.0 as f32 / u32::MAX as f32) * 2.0 - 1.0
    }
}
