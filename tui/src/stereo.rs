//! El espectro estéreo del master para su ventana (`tui --window-spectrum`):
//! en la principal, con lo que copia la cola estéreo del motor
//! (`Controller::stereo_samples`), una FFT por canal con el analizador de
//! siempre ([`crate::viz::spectrum::Analyzer`]) y, por banda, el nivel de
//! cada canal (con su caída, como el espectro del master) y el paneo de la
//! banda. Solo eso viaja a la ventana: [`BANDS`] valores por lista, cada
//! uno en un byte (dos cifras hexadecimales), nunca audio.
//!
//! El paneo de una banda sale de las amplitudes de los dos canales:
//! `(R − L) / (R + L)`, de −1 (solo a la izquierda) a 1 (solo a la
//! derecha); 0, igual en los dos (al centro). Una banda en silencio se
//! queda donde estaba.

use crate::viz::spectrum::{Analyzer, fall, level};

/// Bandas por canal.
pub const BANDS: usize = 96;
/// Las que se aceptan en un cuadro que llega (más, se descarta).
pub const MAX_BANDS: usize = 256;
/// Por debajo de este nivel (de 0 a 1; ~−80 dB), una banda no mueve su
/// paneo.
const PAN_GATE: f32 = 0.12;
/// Lo que se acerca el paneo que se ve al nuevo en cada cuadro.
const PAN_FOLLOW: f32 = 0.5;

/// Lo que se manda a la ventana en cada cuadro.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct StereoFrame {
    /// Frecuencia del motor (para el eje de frecuencias).
    pub rate: u32,
    /// Nivel de cada banda de cada canal, de 0 a 1 (con su caída).
    pub left: Vec<f32>,
    pub right: Vec<f32>,
    /// Paneo de cada banda, de −1 (izquierda) a 1 (derecha).
    pub pan: Vec<f32>,
}

impl StereoFrame {
    pub fn bands(&self) -> usize {
        self.left.len()
    }

    /// El nivel de la banda `i`: el del canal más fuerte.
    pub fn energy(&self, i: usize) -> f32 {
        self.left
            .get(i)
            .copied()
            .unwrap_or(0.0)
            .max(self.right.get(i).copied().unwrap_or(0.0))
    }

    /// Si no se ve nada (todo caído).
    pub fn silent(&self) -> bool {
        self.left.iter().chain(&self.right).all(|&l| l <= 0.0)
    }
}

/// El paneo de dos amplitudes: `(r − l) / (r + l)`, de −1 a 1; sin señal
/// (o con algo que no es un número), al centro.
pub fn pan(l: f32, r: f32) -> f32 {
    let (l, r) = (finite(l).abs(), finite(r).abs());
    let sum = l + r;
    if sum > 1e-9 {
        ((r - l) / sum).clamp(-1.0, 1.0)
    } else {
        0.0
    }
}

/// `x`, o 0 si es NaN o infinito.
fn finite(x: f32) -> f32 {
    if x.is_finite() { x } else { 0.0 }
}

/// Un analizador por canal y lo que se ve (con su caída).
#[derive(Default)]
pub struct StereoAnalyzer {
    left: Analyzer,
    right: Analyzer,
    shown: StereoFrame,
}

impl StereoAnalyzer {
    pub fn new() -> StereoAnalyzer {
        StereoAnalyzer::default()
    }

    /// Un cuadro con las últimas muestras de cada canal (las primeras
    /// [`crate::viz::spectrum::SIZE`] de cada iterador). Lo que no es un
    /// número cuenta como silencio.
    pub fn analyze(
        &mut self,
        left: impl IntoIterator<Item = f32>,
        right: impl IntoIterator<Item = f32>,
        rate: u32,
    ) -> &StereoFrame {
        let pl = self.left.peaks(left.into_iter().map(finite), rate, BANDS);
        let pr = self.right.peaks(right.into_iter().map(finite), rate, BANDS);
        let s = &mut self.shown;
        if s.pan.len() != BANDS {
            s.pan = vec![0.0; BANDS];
        }
        let mut fresh_l = [0.0f32; BANDS];
        let mut fresh_r = [0.0f32; BANDS];
        for i in 0..BANDS {
            let (a, b) = (pl[i], pr[i]);
            fresh_l[i] = level(a);
            fresh_r[i] = level(b);
            if fresh_l[i].max(fresh_r[i]) > PAN_GATE {
                let target = pan(a, b);
                s.pan[i] += (target - s.pan[i]) * PAN_FOLLOW;
            }
        }
        fall(&mut s.left, &fresh_l);
        fall(&mut s.right, &fresh_r);
        s.rate = rate;
        &self.shown
    }

    /// FFT hechas (las dos).
    #[cfg(test)]
    pub fn runs(&self) -> u64 {
        self.left.runs() + self.right.runs()
    }
}

/// Los niveles (de 0 a 1) en hexadecimal, un byte cada uno; lo que no es
/// un número, 0.
pub fn encode_levels(values: &[f32]) -> String {
    hex(values.iter().map(|&v| finite(v).clamp(0.0, 1.0)))
}

/// Los paneos (de −1 a 1) en hexadecimal: 0 la izquierda, 80 el centro y
/// ff la derecha.
pub fn encode_pan(values: &[f32]) -> String {
    hex(values
        .iter()
        .map(|&v| (finite(v).clamp(-1.0, 1.0) + 1.0) * 0.5))
}

fn hex(values: impl Iterator<Item = f32>) -> String {
    use std::fmt::Write;
    let mut out = String::new();
    for v in values {
        let _ = write!(out, "{:02x}", (v * 255.0).round() as u8);
    }
    out
}

/// Lo contrario de [`encode_levels`]: `bands` valores de 0 a 1, o error si
/// el texto no tiene ese largo o no es hexadecimal.
pub fn decode_levels(text: &str, bands: usize) -> Result<Vec<f32>, String> {
    unhex(text, bands)
}

/// Lo contrario de [`encode_pan`].
pub fn decode_pan(text: &str, bands: usize) -> Result<Vec<f32>, String> {
    Ok(unhex(text, bands)?
        .into_iter()
        // 0x80 es el centro exacto (128/255 se queda a medio paso).
        .map(|v| {
            let p = v * 2.0 - 1.0;
            if p.abs() < 1.5 / 255.0 { 0.0 } else { p }
        })
        .collect())
}

fn unhex(text: &str, bands: usize) -> Result<Vec<f32>, String> {
    if bands > MAX_BANDS || text.len() != 2 * bands || !text.is_ascii() {
        return Err(format!(
            "{} cifras para {bands} bandas (como mucho {MAX_BANDS})",
            text.len()
        ));
    }
    (0..bands)
        .map(|i| {
            u8::from_str_radix(&text[2 * i..2 * i + 2], 16)
                .map(|b| b as f32 / 255.0)
                .map_err(|_| format!("«{}» no es hexadecimal", &text[2 * i..2 * i + 2]))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::viz::spectrum::{SIZE, edges};

    const RATE: u32 = 48_000;

    fn sine(hz: f32, amp: f32) -> Vec<f32> {
        (0..SIZE)
            .map(|i| amp * (std::f32::consts::TAU * hz * i as f32 / RATE as f32).sin())
            .collect()
    }

    /// La banda más fuerte de un cuadro.
    fn loudest(frame: &StereoFrame) -> usize {
        (0..frame.bands())
            .max_by(|&a, &b| frame.energy(a).total_cmp(&frame.energy(b)))
            .unwrap()
    }

    #[test]
    fn a_sine_only_left_pans_left_only_right_right_and_both_centre() {
        let tone = sine(1000.0, 0.5);
        let silence = vec![0.0; SIZE];
        for (l, r, want) in [
            (&tone, &silence, -1.0f32),
            (&silence, &tone, 1.0),
            (&tone, &tone, 0.0),
        ] {
            let mut a = StereoAnalyzer::new();
            // Unos cuadros, para que el paneo que se ve llegue.
            for _ in 0..12 {
                a.analyze(l.iter().copied(), r.iter().copied(), RATE);
            }
            let frame = a.analyze(l.iter().copied(), r.iter().copied(), RATE);
            let band = loudest(frame);
            assert!(
                (frame.pan[band] - want).abs() < 0.01,
                "{want}: {}",
                frame.pan[band]
            );
            // Y lo mismo una vez codificado y leído.
            let back = decode_pan(&encode_pan(&frame.pan), BANDS).unwrap();
            assert!((back[band] - want).abs() < 0.01, "{}", back[band]);
        }
        // A medias (el doble a la derecha): a un tercio de la derecha.
        assert!((pan(0.25, 0.5) - 1.0 / 3.0).abs() < 1e-6);
    }

    #[test]
    fn a_sine_falls_in_its_band() {
        let e = edges(BANDS, RATE);
        for hz in [60.0f32, 440.0, 3000.0, 12_000.0] {
            let tone = sine(hz, 0.5);
            let mut a = StereoAnalyzer::new();
            let frame = a.analyze(tone.iter().copied(), tone.iter().copied(), RATE);
            let band = loudest(frame);
            // En los graves, las bandas son más estrechas que un bin de la
            // FFT (48 000 / 2048 ≈ 23 Hz): como mucho, un bin de margen.
            let bin = RATE as f32 / SIZE as f32;
            assert!(
                e[band] - bin <= hz && hz <= e[band + 1] + bin,
                "{hz} Hz en la banda {band} ({}–{})",
                e[band],
                e[band + 1]
            );
            assert!(frame.left[band] > 0.85 && frame.right[band] > 0.85);
        }
    }

    #[test]
    fn nan_and_infinities_are_silence_and_levels_fall_slowly() {
        let mut a = StereoAnalyzer::new();
        let tone = sine(440.0, 0.5);
        let band = loudest(a.analyze(tone.iter().copied(), tone.iter().copied(), RATE));
        let bad = [f32::NAN, f32::INFINITY, f32::NEG_INFINITY, 1.0];
        let frame = a.analyze(bad.iter().copied(), bad.iter().copied(), RATE);
        assert!(frame.left.iter().chain(&frame.pan).all(|v| v.is_finite()));
        // El tono no se va de golpe: cae.
        assert!(frame.left[band] > 0.8);
        assert!(!frame.silent());
        assert_eq!(pan(f32::NAN, 1.0), 1.0);
        assert_eq!(pan(f32::INFINITY, f32::NAN), 0.0);
        assert_eq!(a.runs(), 4);
    }

    #[test]
    fn levels_and_pans_survive_the_trip_and_bad_text_is_refused() {
        let levels = [0.0, 0.5, 1.0, f32::NAN, 7.0, -3.0];
        let text = encode_levels(&levels);
        assert_eq!(text.len(), 12);
        let back = decode_levels(&text, 6).unwrap();
        assert_eq!(back[0], 0.0);
        assert!((back[1] - 0.5).abs() < 0.003);
        assert_eq!(&back[2..], &[1.0, 0.0, 1.0, 0.0]);
        assert_eq!(decode_pan(&encode_pan(&[0.0]), 1).unwrap(), [0.0]);
        assert_eq!(decode_pan("00ff", 2).unwrap(), [-1.0, 1.0]);
        assert!(decode_levels("0", 1).is_err());
        assert!(decode_levels("zz", 1).is_err());
        assert!(decode_levels("ñ0", 1).is_err());
        assert!(decode_levels(&"00".repeat(MAX_BANDS + 1), MAX_BANDS + 1).is_err());
    }
}
