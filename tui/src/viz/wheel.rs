//! `pitchwheel` y `spiral`, de las muestras de su fuente.
//!
//! `pitchwheel`: un cromagrama (FFT de [`CHROMA_SIZE`] puntos con ventana de
//! Hann: cada bin de 55 Hz a 5 kHz suma su energía a su clase de nota) en
//! una rueda de doce radios con do arriba, cada uno tan largo como suena su
//! nota; la más fuerte, nombrada. `spiral`: la onda enrollada en una
//! espiral cuyo periodo es el de la nota que se oye (YIN,
//! `engine::detect_mono`): con una nota estable, la espiral se queda quieta.
//! Las dos con Braille en el `Canvas` de ratatui; en ASCII, en texto.

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::symbols::Marker;
use ratatui::text::Line as TextLine;
use ratatui::widgets::canvas::{Canvas, Circle, Line as CanvasLine, Points};
use ratatui::widgets::{Block, Clear, Widget};

use crate::ui::{accent, dim};

/// Puntos de la FFT del cromagrama (5,9 Hz por bin a 48 kHz: separa las
/// notas graves).
pub const CHROMA_SIZE: usize = 8192;
const LOW_HZ: f32 = 55.0;
const HIGH_HZ: f32 = 5000.0;

pub const NOTES: [&str; 12] = [
    "C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "A#", "B",
];

/// La energía de cada clase de nota (do primero), de 0 a 1 (la más fuerte,
/// 1); sin señal, todo 0.
pub fn chroma(samples: &[f32], rate: u32) -> [f32; 12] {
    let mut re = vec![0.0f32; CHROMA_SIZE];
    let mut im = vec![0.0f32; CHROMA_SIZE];
    let tail = &samples[samples.len().saturating_sub(CHROMA_SIZE)..];
    for (i, &x) in tail.iter().enumerate() {
        let w = 0.5 - 0.5 * (std::f32::consts::TAU * i as f32 / (CHROMA_SIZE - 1) as f32).cos();
        re[i] = x * w;
    }
    super::spectrum::fft(&mut re, &mut im);
    let bin_hz = rate as f32 / CHROMA_SIZE as f32;
    let mut out = [0.0f32; 12];
    let from = (LOW_HZ / bin_hz).ceil() as usize;
    let to = ((HIGH_HZ.min(rate as f32 / 2.0)) / bin_hz) as usize;
    for k in from.max(1)..to.min(CHROMA_SIZE / 2) {
        let f = k as f32 * bin_hz;
        let midi = 69.0 + 12.0 * (f / 440.0).log2();
        let class = (midi.round() as i32).rem_euclid(12) as usize;
        out[class] += re[k] * re[k] + im[k] * im[k];
    }
    let max = out.iter().copied().fold(0.0f32, f32::max);
    if max > 1e-12 {
        for v in &mut out {
            *v /= max;
        }
    } else {
        out = [0.0; 12];
    }
    out
}

/// Lo que dibuja [`WheelPanel`].
#[derive(Debug, Clone, PartialEq)]
pub struct WheelView {
    pub title: String,
    pub size: u16,
    pub chroma: [f32; 12],
}

/// La vista de `pitchwheel` de `history`.
pub fn wheel(history: &[f32], rate: u32, title: String, size: u16) -> WheelView {
    WheelView {
        title,
        size,
        chroma: chroma(history, rate),
    }
}

impl WheelView {
    /// La nota más fuerte, si suena algo.
    pub fn strongest(&self) -> Option<usize> {
        let (i, v) = self
            .chroma
            .iter()
            .enumerate()
            .max_by(|a, b| a.1.total_cmp(b.1))?;
        (*v > 0.0).then_some(i)
    }
}

fn frame(title: &str, area: Rect, buf: &mut Buffer) -> Rect {
    Clear.render(area, buf);
    let block = Block::bordered()
        .border_style(Style::new().fg(accent()))
        .title(title);
    let inner = block.inner(area);
    block.render(area, buf);
    inner
}

pub struct WheelPanel<'a>(pub &'a WheelView);

impl Widget for WheelPanel<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let w = self.0;
        let inner = frame(&w.title, area, buf);
        if inner.width == 0 || inner.height == 0 {
            return;
        }
        let strongest = w.strongest();
        if crate::theme::ascii() {
            // Una línea: cada nota con tantas `#` como suena.
            let text: String = (0..12)
                .map(|i| {
                    let n = (w.chroma[i] * 4.0).round() as usize;
                    format!("{}{:<4} ", NOTES[i], "#".repeat(n))
                })
                .collect();
            TextLine::from(text).render(inner, buf);
            return;
        }
        // Una rueda redonda: las celdas son el doble de altas que anchas.
        let aspect = inner.width as f64 / (2.0 * inner.height as f64);
        Canvas::default()
            .marker(Marker::Braille)
            .x_bounds([-aspect, aspect])
            .y_bounds([-1.0, 1.0])
            .paint(|ctx| {
                ctx.draw(&Circle {
                    x: 0.0,
                    y: 0.0,
                    radius: 0.95,
                    color: dim(),
                });
                for i in 0..12 {
                    let angle =
                        std::f64::consts::FRAC_PI_2 - std::f64::consts::TAU * i as f64 / 12.0;
                    let r = 0.9 * w.chroma[i] as f64;
                    ctx.draw(&CanvasLine {
                        x1: 0.0,
                        y1: 0.0,
                        x2: r * angle.cos(),
                        y2: r * angle.sin(),
                        color: if Some(i) == strongest {
                            accent()
                        } else {
                            dim()
                        },
                    });
                }
                if let Some(i) = strongest {
                    let angle =
                        std::f64::consts::FRAC_PI_2 - std::f64::consts::TAU * i as f64 / 12.0;
                    ctx.print(
                        0.6 * angle.cos(),
                        0.6 * angle.sin(),
                        TextLine::from(NOTES[i]).style(Style::new().fg(accent())),
                    );
                }
            })
            .render(inner, buf);
    }
}

/// Lo que dibuja [`SpiralPanel`].
#[derive(Debug, Clone, PartialEq)]
pub struct SpiralView {
    pub title: String,
    pub size: u16,
    /// El periodo de la vuelta, en muestras (el de la nota que se oye, o
    /// uno fijo si no hay nota).
    pub period: usize,
    /// La nota, si se ve.
    pub note: Option<String>,
    /// Las últimas `TURNS · period` muestras.
    pub samples: Vec<f32>,
}

/// Vueltas de la espiral.
pub const TURNS: usize = 6;
/// El periodo sin nota.
const DEFAULT_PERIOD: usize = 1024;

/// La vista de `spiral` de `history`.
pub fn spiral(history: &[f32], rate: u32, title: String, size: u16) -> SpiralView {
    let tail = &history[history.len().saturating_sub(4096)..];
    let guess = if tail.len() >= 2048 {
        engine::detect_mono(tail, rate)
    } else {
        None
    };
    let period = guess
        .map(|g| (rate as f32 / g.hz).round() as usize)
        .filter(|&p| p >= 8)
        .unwrap_or(DEFAULT_PERIOD);
    let want = (TURNS * period).min(history.len());
    // Desde una vuelta entera contada desde el final: con la misma nota, la
    // espiral empieza en la misma fase.
    let start = history.len() - want;
    SpiralView {
        title,
        size,
        period,
        note: guess.map(|g| g.name()),
        samples: history[start..].to_vec(),
    }
}

pub struct SpiralPanel<'a>(pub &'a SpiralView);

impl Widget for SpiralPanel<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let s = self.0;
        let title = match &s.note {
            Some(n) => format!("{}{n} ", s.title),
            None => s.title.clone(),
        };
        let inner = frame(&title, area, buf);
        if inner.width == 0 || inner.height == 0 || s.samples.is_empty() {
            return;
        }
        let n = s.samples.len();
        let period = s.period.max(1) as f64;
        let point = |i: usize| {
            let turn = i as f64 / period;
            let angle = std::f64::consts::TAU * turn;
            // El radio crece con cada vuelta y la muestra lo empuja.
            let r = 0.15 + 0.75 * turn / TURNS as f64 + 0.12 * s.samples[i] as f64;
            (r * angle.cos(), r * angle.sin())
        };
        if crate::theme::ascii() {
            let (w, h) = (inner.width as f64, inner.height as f64);
            for i in (0..n).step_by((n / 400).max(1)) {
                let (x, y) = point(i);
                let cx = ((x + 1.0) / 2.0 * (w - 1.0)).round() as u16;
                let cy = ((1.0 - y) / 2.0 * (h - 1.0)).round() as u16;
                if cx < inner.width && cy < inner.height {
                    buf.set_string(inner.x + cx, inner.y + cy, "*", Style::new().fg(accent()));
                }
            }
            return;
        }
        let aspect = inner.width as f64 / (2.0 * inner.height as f64);
        let coords: Vec<(f64, f64)> = (0..n).step_by((n / 2000).max(1)).map(point).collect();
        Canvas::default()
            .marker(Marker::Braille)
            .x_bounds([-aspect, aspect])
            .y_bounds([-1.0, 1.0])
            .paint(|ctx| {
                ctx.draw(&Points {
                    coords: &coords,
                    color: accent(),
                });
            })
            .render(inner, buf);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sine(hz: f32) -> Vec<f32> {
        (0..16_384)
            .map(|i| 0.5 * (std::f32::consts::TAU * hz * i as f32 / 48_000.0).sin())
            .collect()
    }

    #[test]
    fn a_note_lights_up_its_class() {
        for (hz, class) in [
            (440.0, 9),
            (261.63, 0),
            (110.0, 9),
            (82.41, 4),
            (1567.98, 7),
        ] {
            let c = chroma(&sine(hz), 48_000);
            let strongest = (0..12).max_by(|&a, &b| c[a].total_cmp(&c[b])).unwrap();
            assert_eq!(strongest, class, "{hz} Hz: {c:?}");
            assert_eq!(c[class], 1.0);
        }
        assert_eq!(chroma(&[0.0; 9000], 48_000), [0.0; 12]);
    }

    #[test]
    fn the_spiral_turns_with_the_note() {
        let view = spiral(&sine(220.0), 48_000, String::new(), 8);
        // 48 000 / 220 = 218,2 muestras.
        assert!(view.period.abs_diff(218) <= 2, "{}", view.period);
        assert_eq!(view.note.as_deref(), Some("A3"));
        assert_eq!(view.samples.len(), TURNS * view.period);
        let silent = spiral(&[0.0; 8000], 48_000, String::new(), 8);
        assert_eq!(silent.note, None);
        assert_eq!(silent.period, DEFAULT_PERIOD);
    }
}
