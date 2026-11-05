//! El espectro del master en su panel fijo: con puntos Braille (dos
//! columnas y cuatro filas de puntos por celda), relleno o en línea, o en
//! barras de bloques (lo que gasta menos), o como espectrograma (el
//! historial que se desplaza hacia la izquierda, con la intensidad en la
//! densidad de los puntos y en el color). La frecuencia va en escala
//! logarítmica, de 30 Hz a 20 kHz, con su eje al pie. Lo que se dibuja ya
//! viene con su caída ([`super::spectrum::fall`]), así que no parpadea.
//! Solo dibuja: el análisis es el de siempre ([`super::spectrum::Analyzer`],
//! una vez por cuadro dibujado).

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Color, Style};
use ratatui::widgets::{Block, Widget};

use super::spectrum::edges;
use crate::ui::{accent, braille, dim, wave_color};

/// Cómo se dibuja (`[interfaz] espectro`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Look {
    /// Puntos, rellenos hasta abajo.
    #[default]
    Fill,
    /// Puntos, solo la línea de arriba.
    Line,
    /// Barras de bloques (el dibujo de antes; el que menos gasta).
    Bars,
    /// El historial: tiempo a lo ancho, frecuencia a lo alto.
    Spectrogram,
}

impl Look {
    pub fn name(self) -> &'static str {
        match self {
            Look::Fill => "relleno",
            Look::Line => "linea",
            Look::Bars => "barras",
            Look::Spectrogram => "espectrograma",
        }
    }

    pub fn from_name(name: &str) -> Option<Look> {
        [Look::Fill, Look::Line, Look::Bars, Look::Spectrogram]
            .into_iter()
            .find(|s| s.name() == name || (name == "línea" && *s == Look::Line))
    }
}

/// Lo que dibuja [`MasterSpectrumPanel`].
#[derive(Debug, Clone, PartialEq, Default)]
pub struct MasterSpectrumView {
    pub style: Look,
    /// Nivel de cada banda, de 0 a 1 (con su caída).
    pub levels: Vec<f32>,
    /// Del espectrograma: los niveles de los últimos cuadros, el más
    /// nuevo al final.
    pub history: Vec<Vec<f32>>,
    pub rate: u32,
}

/// Las marcas del eje de frecuencias.
const AXIS: [(f32, &str); 10] = [
    (50.0, "50"),
    (100.0, "100"),
    (200.0, "200"),
    (500.0, "500"),
    (1000.0, "1k"),
    (2000.0, "2k"),
    (5000.0, "5k"),
    (10_000.0, "10k"),
    (15_000.0, "15k"),
    (20_000.0, "20k"),
];

/// Umbrales de los ocho puntos de una celda (columna, fila) para el
/// espectrograma: un punto se enciende si el nivel pasa del suyo, así la
/// densidad de puntos sigue al nivel.
const DITHER: [[f32; 4]; 2] = [
    [0.0625, 0.5625, 0.3125, 0.8125],
    [0.6875, 0.1875, 0.9375, 0.4375],
];

/// Bit Braille de cada punto: [columna][fila].
const BIT: [[u8; 4]; 2] = [[0x01, 0x02, 0x04, 0x40], [0x08, 0x10, 0x20, 0x80]];

/// El nivel en la fracción `t` (de 0 a 1, de grave a agudo) de `levels`,
/// entre los centros de las bandas.
pub fn level_at(levels: &[f32], t: f32) -> f32 {
    let n = levels.len();
    if n == 0 {
        return 0.0;
    }
    let pos = (t * n as f32 - 0.5).clamp(0.0, (n - 1) as f32);
    let i = pos.floor() as usize;
    let frac = pos - i as f32;
    let next = levels[(i + 1).min(n - 1)];
    levels[i] * (1.0 - frac) + next * frac
}

/// Dónde va cada marca del eje en `width` columnas (las que no se pisan).
pub fn axis_marks(width: u16, rate: u32) -> Vec<(u16, &'static str)> {
    let e = edges(1, rate);
    let (low, high) = (e[0], e[1]);
    let mut out = Vec::new();
    let mut free = 0u16;
    for (hz, label) in AXIS {
        if hz > high {
            break;
        }
        let t = (hz / low).ln() / (high / low).ln();
        let x = (t * width as f32).round() as u16;
        let w = label.len() as u16;
        if x < free || x + w > width {
            continue;
        }
        out.push((x, label));
        free = x + w + 1;
    }
    out
}

/// Los puntos del espectro en `width`×`rows` celdas (uno por celda, fila a
/// fila) y, por celda, si tiene el borde de arriba (va en el acento).
pub fn dots(levels: &[f32], width: u16, rows: u16, fill: bool) -> (Vec<u8>, Vec<bool>) {
    let (w, h) = (width as usize, rows as usize);
    let mut cells = vec![0u8; w * h];
    let mut edge = vec![false; w * h];
    let dots_h = h * 4;
    let mut prev: Option<usize> = None;
    for k in 0..2 * w {
        let t = (k as f32 + 0.5) / (2 * w) as f32;
        let level = level_at(levels, t).clamp(0.0, 1.0);
        let top = ((1.0 - level) * dots_h as f32).round() as usize;
        if top >= dots_h {
            prev = None;
            continue;
        }
        let (from, to) = if fill {
            (top, dots_h)
        } else {
            let p = prev.unwrap_or(top);
            (p.min(top), p.max(top) + 1)
        };
        for d in from..to.min(dots_h) {
            cells[(d / 4) * w + k / 2] |= BIT[k % 2][d % 4];
        }
        edge[(top / 4) * w + k / 2] = true;
        prev = Some(top);
    }
    (cells, edge)
}

/// El espectrograma en `width`×`rows` celdas: dos cuadros por celda a lo
/// ancho (el más nuevo a la derecha) y cuatro alturas de frecuencia por
/// celda (los graves abajo); y el nivel más alto de cada celda.
pub fn spectrogram(history: &[Vec<f32>], width: u16, rows: u16) -> (Vec<u8>, Vec<f32>) {
    let (w, h) = (width as usize, rows as usize);
    let mut cells = vec![0u8; w * h];
    let mut peak = vec![0.0f32; w * h];
    let columns = 2 * w;
    let skip = history.len().saturating_sub(columns);
    let shown = &history[skip..];
    // El más nuevo en la última columna de puntos.
    let first = columns - shown.len();
    let dots_h = h * 4;
    for (i, levels) in shown.iter().enumerate() {
        let k = first + i;
        for d in 0..dots_h {
            let t = 1.0 - (d as f32 + 0.5) / dots_h as f32;
            let level = level_at(levels, t);
            let cell = (d / 4) * w + k / 2;
            peak[cell] = peak[cell].max(level);
            if level > DITHER[k % 2][d % 4] {
                cells[cell] |= BIT[k % 2][d % 4];
            }
        }
    }
    (cells, peak)
}

/// El panel del espectro del master con su borde.
pub struct MasterSpectrumPanel<'a> {
    pub view: &'a MasterSpectrumView,
    pub block: Block<'a>,
}

impl Widget for MasterSpectrumPanel<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let s = self.view;
        let inner = self.block.inner(area);
        self.block.render(area, buf);
        if inner.width == 0 || inner.height == 0 {
            return;
        }
        let ascii = crate::theme::ascii();
        // El eje al pie, si hay sitio.
        let plot = if inner.height >= 3 {
            let axis_y = inner.bottom() - 1;
            for (x, label) in axis_marks(inner.width, s.rate.max(1)) {
                buf.set_string(inner.x + x, axis_y, label, Style::new().fg(dim()));
            }
            Rect {
                height: inner.height - 1,
                ..inner
            }
        } else {
            inner
        };
        // Con menos de tres filas, el espectrograma no se lee: relleno.
        let style = if s.style == Look::Spectrogram && plot.height < 3 {
            Look::Fill
        } else {
            s.style
        };
        let cell = |buf: &mut Buffer, i: usize, c: char, color: Color| {
            let w = plot.width as usize;
            buf[(plot.x + (i % w) as u16, plot.y + (i / w) as u16)]
                .set_char(c)
                .set_fg(color);
        };
        match style {
            Look::Spectrogram => {
                let (cells, peak) = spectrogram(&s.history, plot.width, plot.height);
                for (i, (&bits, &p)) in cells.iter().zip(&peak).enumerate() {
                    let color = if p > 0.66 {
                        accent()
                    } else if p > 0.33 {
                        wave_color()
                    } else {
                        dim()
                    };
                    if ascii {
                        const RAMP: [char; 6] = [' ', '.', ':', '-', '=', '#'];
                        let c = RAMP[((p * 5.0).round() as usize).min(5)];
                        if c != ' ' {
                            cell(buf, i, c, color);
                        }
                    } else if bits != 0 {
                        cell(buf, i, braille(bits), color);
                    }
                }
            }
            Look::Bars => bars(&s.levels, plot, buf, ascii),
            Look::Fill | Look::Line if ascii => bars(&s.levels, plot, buf, true),
            Look::Fill | Look::Line => {
                let (cells, edge) = dots(&s.levels, plot.width, plot.height, style == Look::Fill);
                for (i, (&bits, &top)) in cells.iter().zip(&edge).enumerate() {
                    if bits != 0 {
                        let color = if top || style == Look::Line {
                            accent()
                        } else {
                            wave_color()
                        };
                        cell(buf, i, braille(bits), color);
                    }
                }
            }
        }
    }
}

/// Barras de bloques (octavos de celda), una por columna.
fn bars(levels: &[f32], plot: Rect, buf: &mut Buffer, ascii: bool) {
    const BLOCKS: [char; 9] = [' ', '▁', '▂', '▃', '▄', '▅', '▆', '▇', '█'];
    let rows = plot.height;
    for x in 0..plot.width {
        let t = (x as f32 + 0.5) / plot.width as f32;
        let eighths = (level_at(levels, t).clamp(0.0, 1.0) * rows as f32 * 8.0).round() as u16;
        for r in 0..rows {
            let fill = eighths.saturating_sub(r * 8).min(8) as usize;
            if fill == 0 {
                continue;
            }
            let ch = if ascii {
                if fill >= 4 { '#' } else { '.' }
            } else {
                BLOCKS[fill]
            };
            buf[(plot.x + x, plot.bottom() - 1 - r)]
                .set_char(ch)
                .set_fg(accent());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn levels_interpolate_between_band_centres() {
        let levels = [0.0, 1.0];
        assert_eq!(level_at(&levels, 0.0), 0.0);
        assert_eq!(level_at(&levels, 0.25), 0.0);
        assert!((level_at(&levels, 0.5) - 0.5).abs() < 1e-6);
        assert_eq!(level_at(&levels, 1.0), 1.0);
        assert_eq!(level_at(&[], 0.5), 0.0);
    }

    #[test]
    fn the_axis_is_logarithmic_and_its_marks_do_not_overlap() {
        let marks = axis_marks(60, 48_000);
        let at = |label: &str| marks.iter().find(|m| m.1 == label).map(|m| m.0);
        // 1 kHz cae algo más allá de la mitad (30 Hz a 20 kHz).
        let k = at("1k").unwrap();
        assert!((30..40).contains(&k), "{marks:?}");
        assert!(at("100").unwrap() < at("1k").unwrap());
        for pair in marks.windows(2) {
            assert!(
                pair[0].0 + (pair[0].1.len() as u16) < pair[1].0,
                "{marks:?}"
            );
        }
        // A 22,05 kHz de Nyquist no pasa de 20k; con poco ancho, menos marcas.
        assert!(axis_marks(12, 44_100).len() < marks.len());
    }

    #[test]
    fn fill_lights_everything_below_the_line_and_line_only_the_edge() {
        let flat = vec![0.5; 16];
        let (cells, edge) = dots(&flat, 4, 2, true);
        // Mitad: la fila de abajo entera, la de arriba vacía.
        assert_eq!(&cells[..4], &[0, 0, 0, 0]);
        assert_eq!(&cells[4..], &[0xFF; 4]);
        assert!(edge[4..].iter().all(|&e| e));
        let (cells, _) = dots(&flat, 4, 2, false);
        // Solo la fila de puntos de arriba de la celda de abajo.
        assert_eq!(&cells[4..], &[0x09; 4]);
        // En silencio, nada.
        let (cells, _) = dots(&[0.0; 16], 4, 2, true);
        assert!(cells.iter().all(|&b| b == 0));
    }

    #[test]
    fn the_spectrogram_puts_the_newest_on_the_right_and_more_dots_when_louder() {
        let quiet = vec![0.1; 8];
        let loud = vec![0.95; 8];
        let history = vec![quiet.clone(), quiet, loud.clone(), loud];
        let (cells, peak) = spectrogram(&history, 4, 1);
        // 8 columnas de puntos, 4 cuadros: los dos primeros vacíos.
        assert_eq!(cells[0], 0);
        let count = |b: u8| b.count_ones();
        assert!(count(cells[3]) > count(cells[2]), "{cells:?}");
        assert!(count(cells[3]) >= 7);
        assert!(peak[3] > 0.9 && peak[2] < 0.2);
    }

    #[test]
    fn styles_by_name() {
        for s in [Look::Fill, Look::Line, Look::Bars, Look::Spectrogram] {
            assert_eq!(Look::from_name(s.name()), Some(s));
        }
        assert_eq!(Look::from_name("línea"), Some(Look::Line));
        assert_eq!(Look::from_name("x"), None);
    }
}
