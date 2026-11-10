//! El cargador de WAV (`engine::Sample::load_wav`) con archivos mutados.

use std::time::Instant;

use crate::{Cuenta, Rng, carpeta};

/// WAV válidos de partida, de varios formatos.
fn semillas(dir: &std::path::Path) -> Vec<Vec<u8>> {
    let mut out = Vec::new();
    let formatos: [(u16, u32, u16, hound::SampleFormat); 8] = [
        (1, 44_100, 16, hound::SampleFormat::Int),
        (2, 48_000, 16, hound::SampleFormat::Int),
        (2, 96_000, 24, hound::SampleFormat::Int),
        (1, 8_000, 8, hound::SampleFormat::Int),
        (2, 44_100, 32, hound::SampleFormat::Int),
        (1, 48_000, 32, hound::SampleFormat::Float),
        (6, 48_000, 16, hound::SampleFormat::Int),
        (2, 22_050, 32, hound::SampleFormat::Float),
    ];
    for (i, (channels, rate, bits, format)) in formatos.into_iter().enumerate() {
        let path = dir.join(format!("semilla-{i}.wav"));
        let spec = hound::WavSpec { channels, sample_rate: rate, bits_per_sample: bits, sample_format: format };
        let mut w = hound::WavWriter::create(&path, spec).unwrap();
        for n in 0..(400 * channels as usize) {
            let x = ((n as f32) * 0.05).sin() * 0.7;
            match format {
                hound::SampleFormat::Float => w.write_sample(x).unwrap(),
                hound::SampleFormat::Int => {
                    let max = (1i64 << (bits - 1)) - 1;
                    w.write_sample((x * max as f32) as i32).unwrap()
                }
            }
        }
        w.finalize().unwrap();
        out.push(std::fs::read(&path).unwrap());
    }
    if let Ok(amen) = std::fs::read(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../scripts/ejemplos/amen.wav")) {
        // Solo el principio: lo que importa es la cabecera.
        out.push(amen[..amen.len().min(6000)].to_vec());
    }
    out
}

const INTERESANTES: [u32; 12] =
    [0, 1, 2, 3, 0x7f, 0xff, 0x7fff, 0xffff, 0x7fff_ffff, 0x8000_0000, 0xffff_ffff, 0xffff_fffe];

fn mutar(rng: &mut Rng, base: &[u8]) -> Vec<u8> {
    let mut b = base.to_vec();
    for _ in 0..1 + rng.below(4) {
        if b.is_empty() {
            break;
        }
        match rng.below(8) {
            // Un campo de la cabecera (los primeros 48 bytes) con un valor límite.
            0 | 1 | 2 => {
                let at = rng.below(b.len().min(48).saturating_sub(4).max(1));
                let v = *rng.pick(&INTERESANTES);
                let width = *rng.pick(&[1usize, 2, 4]);
                for (k, byte) in v.to_le_bytes().iter().take(width).enumerate() {
                    if at + k < b.len() {
                        b[at + k] = *byte;
                    }
                }
            }
            3 => {
                let at = rng.below(b.len());
                b[at] ^= 1 << rng.below(8);
            }
            4 => {
                let at = rng.below(b.len());
                b.truncate(at);
            }
            5 => {
                let at = rng.below(b.len());
                let n = rng.below(64);
                let extra: Vec<u8> = (0..n).map(|_| rng.next() as u8).collect();
                b.splice(at..at, extra);
            }
            6 => {
                let at = rng.below(b.len());
                let n = rng.below(16).min(b.len() - at);
                b.drain(at..at + n);
            }
            _ => {
                let at = rng.below(b.len());
                b[at] = rng.next() as u8;
            }
        }
    }
    b
}

pub fn correr(rng: &mut Rng, cuenta: &mut Cuenta, fin: Instant) {
    let dir = carpeta("tmp");
    let semillas = semillas(&dir);
    let path = dir.join("fuzz.wav");
    let rates = [8_000u32, 44_100, 48_000, 96_000, 192_000];
    while Instant::now() < fin {
        let base = rng.pick(&semillas).clone();
        let bytes = mutar(rng, &base);
        std::fs::write(&path, &bytes).unwrap();
        let rate = *rng.pick(&rates);
        cuenta.correr("wav", &bytes, || match engine::Sample::load_wav(&path, rate) {
            Ok(sample) => {
                // Lo cargado tiene que poder usarse: leer el primer y el
                // último frame y cortarlo.
                let n = sample.frames();
                if n > 0 {
                    let _ = sample.frame(0);
                    let _ = sample.frame(n - 1);
                }
                let _ = engine::Slices::detect(&sample, 8);
                let _ = engine::Slices::grid(&sample, 8);
                let _ = engine::detect_bpm(&sample);
                true
            }
            Err(_) => false,
        });
    }
}
