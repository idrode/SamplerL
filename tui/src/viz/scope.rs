//! `scope`: un osciloscopio. De lo último que ha llegado de su fuente se
//! toma una ventana que empieza en un cruce por cero de subida (el
//! disparo: con una señal periódica, cada cuadro empieza en la misma fase y
//! la onda se queda quieta) y se dibuja con Braille (2×4 puntos por celda)
//! en el `Canvas` de ratatui; en ASCII, con `*`.

use std::collections::VecDeque;

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::symbols::Marker;
use ratatui::widgets::canvas::{Canvas, Line as CanvasLine};
use ratatui::widgets::{Block, Clear, Widget};

use crate::ui::{accent, dim};

/// Muestras de la ventana (unos 43 ms a 48 kHz: dos periodos de 50 Hz).
pub const WINDOW: usize = 2048;
/// Lo que tiene que bajar la señal antes de subir por el cero para contar
/// como disparo (sin esto, el ruido dispara en cualquier sitio).
const HYSTERESIS: f32 = 0.01;

/// Dónde empieza la ventana de `window` muestras de `a` seguido de `b`
/// (las dos mitades de un `VecDeque`, sin copiarlas a un búfer contiguo):
/// el último cruce por cero de subida (de menos de −`HYSTERESIS` a 0 o
/// más) que deja la ventana entera detrás; sin ninguno, `None`.
fn trigger_in(a: &[f32], b: &[f32], window: usize) -> Option<usize> {
    let len = a.len() + b.len();
    if len < window + 2 {
        return None;
    }
    let last = len - window;
    let (mut before, tail_a, tail_b) = match a.split_first() {
        Some((first, rest)) => (*first, rest, b),
        None => {
            let (first, rest) = b.split_first()?;
            (*first, &[][..], rest)
        }
    };
    let mut armed = false;
    let mut found = None;
    let mut i = 1;
    // Desde el principio, para saber si venía de abajo; el último vale.
    'scan: for part in [tail_a, tail_b] {
        for &now in part {
            if i > last {
                break 'scan;
            }
            if before < -HYSTERESIS {
                armed = true;
            }
            if armed && before < 0.0 && now >= 0.0 {
                found = Some(i);
                armed = false;
            }
            before = now;
            i += 1;
        }
    }
    found
}

/// Lo que dibuja [`ScopePanel`].
#[derive(Debug, Clone, PartialEq)]
pub struct ScopeView {
    pub title: String,
    pub size: u16,
    /// La ventana (desde el disparo, o lo último si no lo hay).
    pub samples: Vec<f32>,
    pub triggered: bool,
}

/// La vista de `history` (lo último de su fuente). Sin copiar el
/// historial: solo la ventana que se dibuja.
pub fn build(history: &VecDeque<f32>, title: String, size: u16) -> ScopeView {
    let (a, b) = history.as_slices();
    let (start, triggered) = match trigger_in(a, b, WINDOW) {
        Some(i) => (i, true),
        None => (history.len().saturating_sub(WINDOW), false),
    };
    let end = (start + WINDOW).min(history.len());
    ScopeView {
        title,
        size,
        samples: history.range(start..end).copied().collect(),
        triggered,
    }
}

pub struct ScopePanel<'a>(pub &'a ScopeView);

impl Widget for ScopePanel<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let s = self.0;
        Clear.render(area, buf);
        let title = if s.triggered {
            s.title.clone()
        } else {
            format!("{}(sin disparo) ", s.title)
        };
        let block = Block::bordered()
            .border_style(Style::new().fg(accent()))
            .title(title);
        let inner = block.inner(area);
        block.render(area, buf);
        if inner.width == 0 || inner.height == 0 || s.samples.is_empty() {
            return;
        }
        if crate::theme::plain_viz() {
            ascii(s, inner, buf);
            return;
        }
        let n = s.samples.len();
        // Dos puntos por columna (Braille): un punto por cada media celda.
        let points = 2 * inner.width as usize;
        let at = |k: usize| s.samples[(k * n / points).min(n - 1)] as f64;
        Canvas::default()
            .marker(Marker::Braille)
            .x_bounds([0.0, points as f64])
            .y_bounds([-1.0, 1.0])
            .paint(|ctx| {
                ctx.draw(&CanvasLine {
                    x1: 0.0,
                    y1: 0.0,
                    x2: points as f64,
                    y2: 0.0,
                    color: dim(),
                });
                for k in 1..points {
                    ctx.draw(&CanvasLine {
                        x1: (k - 1) as f64,
                        y1: at(k - 1).clamp(-1.0, 1.0),
                        x2: k as f64,
                        y2: at(k).clamp(-1.0, 1.0),
                        color: accent(),
                    });
                }
            })
            .render(inner, buf);
    }
}

/// En ASCII: un `*` por columna a la altura de su muestra.
fn ascii(s: &ScopeView, inner: Rect, buf: &mut Buffer) {
    let n = s.samples.len();
    let rows = inner.height as f32;
    for x in 0..inner.width {
        let v = s.samples[(x as usize * n / inner.width as usize).min(n - 1)].clamp(-1.0, 1.0);
        let row = (((1.0 - v) / 2.0) * (rows - 1.0)).round() as u16;
        buf.set_string(inner.x + x, inner.y + row, "*", Style::new().fg(accent()));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sine(period: f32, len: usize, from: usize) -> Vec<f32> {
        (from..from + len)
            .map(|i| (std::f32::consts::TAU * i as f32 / period).sin() * 0.8)
            .collect()
    }

    #[test]
    fn the_trigger_keeps_a_periodic_signal_still() {
        // Cuadros consecutivos de la misma señal, cada uno con lo último
        // que ha llegado (desplazado lo que sea): la ventana empieza siempre
        // en la misma fase.
        let period = 437.3;
        let mut firsts = Vec::new();
        for frame in 0..10 {
            let history = sine(period, 8192, frame * 801 + 13);
            let view = build(&history.into(), String::new(), 6);
            assert!(view.triggered);
            firsts.push(view.samples[..8].to_vec());
        }
        for f in &firsts[1..] {
            for (a, b) in f.iter().zip(&firsts[0]) {
                // Un cruce cae entre dos muestras: a menos de una muestra.
                assert!(
                    (a - b).abs() < 0.8 * std::f32::consts::TAU / period * 1.01,
                    "{a} {b}"
                );
            }
        }
        // Y empieza subiendo por el cero.
        assert!(firsts[0][0] >= 0.0 && firsts[0][0] < 0.02 && firsts[0][3] > firsts[0][0]);
    }

    #[test]
    fn without_a_crossing_it_shows_the_latest() {
        let flat = vec![0.3f32; 5000];
        let view = build(&flat.clone().into(), String::new(), 6);
        assert!(!view.triggered);
        assert_eq!(view.samples.len(), WINDOW);
        assert!(trigger_in(&flat[..100], &[], WINDOW).is_none());
        // Partido en dos mitades, lo mismo que entero.
        let sine = sine(437.3, 8192, 0);
        let whole = trigger_in(&sine, &[], WINDOW);
        assert!(whole.is_some());
        for cut in [0, 1, 3000, 8191] {
            assert_eq!(trigger_in(&sine[..cut], &sine[cut..], WINDOW), whole);
        }
    }
}
