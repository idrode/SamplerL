//! Genera un break sintético de un compás (8 golpes en corcheas a 140 BPM:
//! bombo, charles, caja, charles, bombo, bombo, caja, charles) a 44,1 kHz,
//! mono. Determinista: siempre produce el mismo archivo.
//!
//! Uso: cargo run -p engine --example synth_break -- <salida.wav>

use std::f32::consts::TAU;

const RATE: u32 = 44_100;
const BPM: f32 = 140.0;

#[derive(Clone, Copy)]
enum Hit {
    Kick,
    Snare,
    Hat,
}

const BAR: [Hit; 8] = [
    Hit::Kick,
    Hit::Hat,
    Hit::Snare,
    Hit::Hat,
    Hit::Kick,
    Hit::Kick,
    Hit::Snare,
    Hit::Hat,
];

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

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let out = std::env::args()
        .nth(1)
        .ok_or("uso: synth_break <salida.wav>")?;

    let eighth = (RATE as f32 * 60.0 / BPM / 2.0) as usize;
    let mut audio = vec![0.0f32; eighth * BAR.len()];
    let mut noise = Noise(0x5eed_1234);

    for (n, hit) in BAR.iter().enumerate() {
        let start = n * eighth;
        let mut phase = 0.0f32;
        let mut previous = 0.0f32;
        for (i, s) in audio[start..].iter_mut().enumerate() {
            let t = i as f32 / RATE as f32;
            let attack = (t / 0.0005).min(1.0);
            let white = noise.next();
            *s += attack
                * match hit {
                    Hit::Kick => {
                        // Seno con barrido de 120 a 45 Hz.
                        phase += TAU * (45.0 + 75.0 * (-t / 0.03).exp()) / RATE as f32;
                        0.9 * (-t / 0.15).exp() * phase.sin()
                    }
                    Hit::Snare => {
                        let tone = (TAU * 190.0 * t).sin();
                        (-t / 0.09).exp() * (0.55 * white + 0.25 * tone)
                    }
                    Hit::Hat => {
                        // Ruido con paso alto simple (diferencia de muestras).
                        let high = white - previous;
                        previous = white;
                        0.18 * (-t / 0.03).exp() * high
                    }
                };
        }
    }

    engine::offline::write_wav(&out, &audio, 1, RATE)?;
    println!(
        "{out}: {} golpes, {} frames a {RATE} Hz ({:.2} s)",
        BAR.len(),
        audio.len(),
        audio.len() as f32 / RATE as f32
    );
    Ok(())
}
