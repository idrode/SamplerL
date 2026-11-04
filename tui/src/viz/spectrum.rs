//! `spectrum`: el espectro en barras de bloques (`▁▂▃▄▅▆▇█`). De lo último
//! de su fuente, 2048 muestras con ventana de Hann, una FFT (radix-2,
//! propia: Rust puro y sin dependencias) y bandas en escala logarítmica de
//! 30 Hz a 20 kHz (o al Nyquist); cada banda, lo más fuerte de sus bins en
//! dB, de −90 a 0. Para que no parpadee, cada barra baja despacio
//! ([`FALL`] por cuadro) y sube de golpe.

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::widgets::{Block, Clear, Widget};

use crate::ui::{accent, dim};

/// Puntos de la FFT.
pub const SIZE: usize = 2048;
/// Frecuencias de la primera y la última banda.
const LOW_HZ: f32 = 30.0;
const HIGH_HZ: f32 = 20_000.0;
/// dB de la barra vacía (y 0, la llena).
const FLOOR_DB: f32 = -90.0;
/// Lo que baja una barra por cuadro (de 0 a 1): a 30 cuadros por segundo,
/// de arriba abajo en algo más de un segundo.
pub const FALL: f32 = 0.025;

/// FFT compleja en el sitio (radix-2, `re.len()` potencia de dos).
pub fn fft(re: &mut [f32], im: &mut [f32]) {
    let n = re.len();
    // Reordenar por bits invertidos.
    let mut j = 0;
    for i in 1..n {
        let mut bit = n >> 1;
        while j & bit != 0 {
            j ^= bit;
            bit >>= 1;
        }
        j |= bit;
        if i < j {
            re.swap(i, j);
            im.swap(i, j);
        }
    }
    let mut len = 2;
    while len <= n {
        let angle = -std::f64::consts::TAU / len as f64;
        for start in (0..n).step_by(len) {
            for k in 0..len / 2 {
                let (s, c) = (angle * k as f64).sin_cos();
                let (wr, wi) = (c as f32, s as f32);
                let (a, b) = (start + k, start + k + len / 2);
                let (xr, xi) = (re[b] * wr - im[b] * wi, re[b] * wi + im[b] * wr);
                re[b] = re[a] - xr;
                im[b] = im[a] - xi;
                re[a] += xr;
                im[a] += xi;
            }
        }
        len <<= 1;
    }
}

/// Las frecuencias de los bordes de `bands` bandas logarítmicas a `rate`.
pub fn edges(bands: usize, rate: u32) -> Vec<f32> {
    let high = HIGH_HZ.min(rate as f32 / 2.0);
    let bands = bands.max(1);
    (0..=bands)
        .map(|i| LOW_HZ * (high / LOW_HZ).powf(i as f32 / bands as f32))
        .collect()
}

/// El nivel de cada banda (de 0 a 1) de las últimas [`SIZE`] muestras de
/// `samples` a `rate`.
pub fn analyze(samples: &[f32], rate: u32, bands: usize) -> Vec<f32> {
    let mut re = vec![0.0f32; SIZE];
    let mut im = vec![0.0f32; SIZE];
    let tail = &samples[samples.len().saturating_sub(SIZE)..];
    for (i, &x) in tail.iter().enumerate() {
        let w = 0.5 - 0.5 * (std::f32::consts::TAU * i as f32 / (SIZE - 1) as f32).cos();
        re[i] = x * w;
    }
    fft(&mut re, &mut im);
    // Amplitud de cada bin, normalizada (un seno de amplitud 1 da ~1).
    let bin_hz = rate as f32 / SIZE as f32;
    let magnitude = |k: usize| (re[k] * re[k] + im[k] * im[k]).sqrt() * 4.0 / SIZE as f32;
    let edges = edges(bands, rate);
    edges
        .windows(2)
        .map(|e| {
            // Los bins cuyo centro cae en la banda; una banda más estrecha
            // que un bin (las graves), el más cercano a su centro.
            let from = ((e[0] / bin_hz).ceil() as usize).max(1);
            let to = ((e[1] / bin_hz).ceil() as usize).min(SIZE / 2);
            let peak = if to > from {
                (from..to).map(magnitude).fold(0.0f32, f32::max)
            } else {
                let center = (e[0] * e[1]).sqrt() / bin_hz;
                magnitude((center.round() as usize).clamp(1, SIZE / 2 - 1))
            };
            let db = 20.0 * peak.max(1e-9).log10();
            ((db - FLOOR_DB) / -FLOOR_DB).clamp(0.0, 1.0)
        })
        .collect()
}

/// Lo que se ve con caída: sube de golpe y baja [`FALL`] por cuadro.
pub fn fall(shown: &mut Vec<f32>, fresh: &[f32]) {
    if shown.len() != fresh.len() {
        *shown = fresh.to_vec();
        return;
    }
    for (s, &f) in shown.iter_mut().zip(fresh) {
        *s = f.max(*s - FALL);
    }
}

/// Lo que dibuja [`SpectrumPanel`].
#[derive(Debug, Clone, PartialEq)]
pub struct SpectrumView {
    pub title: String,
    pub size: u16,
    /// Nivel de cada banda, de 0 a 1 (con su caída).
    pub levels: Vec<f32>,
}

pub struct SpectrumPanel<'a>(pub &'a SpectrumView);

const BLOCKS: [char; 9] = [' ', '▁', '▂', '▃', '▄', '▅', '▆', '▇', '█'];

impl Widget for SpectrumPanel<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let s = self.0;
        Clear.render(area, buf);
        let block = Block::bordered()
            .border_style(Style::new().fg(accent()))
            .title(s.title.as_str());
        let inner = block.inner(area);
        block.render(area, buf);
        if inner.width == 0 || inner.height == 0 || s.levels.is_empty() {
            return;
        }
        let ascii = crate::theme::ascii();
        let bands = s.levels.len();
        let rows = inner.height;
        for x in 0..inner.width {
            let band = (x as usize * bands / inner.width as usize).min(bands - 1);
            let eighths = (s.levels[band] * rows as f32 * 8.0).round() as u16;
            for r in 0..rows {
                let fill = eighths.saturating_sub(r * 8).min(8) as usize;
                let ch = if ascii {
                    if fill >= 4 { '#' } else { ' ' }
                } else {
                    BLOCKS[fill]
                };
                let color = if r + 1 == rows { accent() } else { dim() };
                let style = Style::new().fg(if fill > 0 { accent() } else { color });
                buf.set_string(inner.x + x, inner.bottom() - 1 - r, ch.to_string(), style);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sine(hz: f32, rate: u32) -> Vec<f32> {
        (0..4096)
            .map(|i| 0.5 * (std::f32::consts::TAU * hz * i as f32 / rate as f32).sin())
            .collect()
    }

    #[test]
    fn a_sine_peaks_in_its_band() {
        let rate = 48_000;
        for hz in [80.0f32, 440.0, 1000.0, 5000.0] {
            let levels = analyze(&sine(hz, rate), rate, 32);
            let edges = edges(32, rate);
            let peak = levels
                .iter()
                .enumerate()
                .max_by(|a, b| a.1.total_cmp(b.1))
                .unwrap()
                .0;
            assert!(
                edges[peak] <= hz && hz < edges[peak + 1],
                "{hz} Hz en la banda {peak} ({}–{})",
                edges[peak],
                edges[peak + 1]
            );
            // Fuerte (0.5 de amplitud: unos −6 dB) y lejos, bajo.
            assert!(levels[peak] > 0.85, "{}", levels[peak]);
            let far = if peak > 16 { 0 } else { 31 };
            assert!(levels[far] < 0.5, "{}", levels[far]);
        }
    }

    #[test]
    fn the_fft_of_an_impulse_is_flat() {
        let mut re = vec![0.0f32; 16];
        let mut im = vec![0.0f32; 16];
        re[0] = 1.0;
        fft(&mut re, &mut im);
        assert!(re.iter().all(|x| (x - 1.0).abs() < 1e-6));
        assert!(im.iter().all(|x| x.abs() < 1e-6));
    }

    #[test]
    fn bars_rise_at_once_and_fall_slowly() {
        let mut shown = vec![0.0, 0.5];
        fall(&mut shown, &[1.0, 0.0]);
        assert_eq!(shown, [1.0, 0.5 - FALL]);
        fall(&mut shown, &[0.0, 0.0]);
        assert_eq!(shown, [1.0 - FALL, 0.5 - 2.0 * FALL]);
    }
}
