//! El dibujo del espectro estéreo (la ventana `tui --window-spectrum`), en
//! puntos Braille (2×4 por celda) o, sin Unicode, con una rampa de ASCII.
//! Dos modos (`[interfaz] espectro_modo`, `m` en la ventana):
//!
//! - **campo**: la frecuencia en vertical (logarítmica, los graves abajo) y
//!   la posición estéreo en horizontal, con el centro en la mitad: cada
//!   banda va donde dice su paneo ([`crate::stereo::pan`]), así que lo
//!   centrado se pega a la línea del centro y lo panoteado se va hacia su
//!   lado. Cuanta más energía, más ancha la mancha, más puntos y otro color.
//! - **espejo**: el canal izquierdo a la izquierda del centro y el derecho a
//!   la derecha, con la frecuencia a lo ancho (los graves junto al centro,
//!   los agudos hacia fuera) y el nivel en vertical, de abajo arriba.
//!
//! Solo dibuja lo que llega (ya con su caída: no parpadea).

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Color, Style};
use ratatui::widgets::{Block, Widget};

use super::master::{AXIS, BIT, DITHER, axis_marks, level_at};
use super::spectrum::edges;
use crate::stereo::StereoFrame;
use crate::ui::{accent, braille, dim, warning_color, wave_color};

/// Cómo se dibuja.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Mode {
    /// Frecuencia en vertical y paneo en horizontal.
    #[default]
    Field,
    /// Izquierda y derecha en espejo, los graves en el centro.
    Mirror,
}

impl Mode {
    pub fn name(self) -> &'static str {
        match self {
            Mode::Field => "campo",
            Mode::Mirror => "espejo",
        }
    }

    pub fn from_name(name: &str) -> Option<Mode> {
        [Mode::Field, Mode::Mirror]
            .into_iter()
            .find(|m| m.name() == name)
    }

    /// El otro.
    pub fn next(self) -> Mode {
        match self {
            Mode::Field => Mode::Mirror,
            Mode::Mirror => Mode::Field,
        }
    }
}

/// Lo más pequeño que se dibuja (con el borde): por debajo, un aviso.
pub const MIN_WIDTH: u16 = 24;
pub const MIN_HEIGHT: u16 = 8;
/// Columnas del eje de frecuencias del modo campo.
const AXIS_WIDTH: u16 = 4;

/// El panel: el cuadro (si ha llegado alguno), el modo y el título.
pub struct StereoPanel<'a> {
    pub frame: Option<&'a StereoFrame>,
    pub mode: Mode,
    pub title: &'a str,
}

/// Los puntos del modo campo en `width`×`rows` celdas (fila a fila) y la
/// intensidad más alta de cada celda.
pub fn field_dots(frame: &StereoFrame, width: u16, rows: u16) -> (Vec<u8>, Vec<f32>) {
    let (w, h) = (width as usize, rows as usize);
    let mut cells = vec![0u8; w * h];
    let mut peak = vec![0.0f32; w * h];
    let bands = frame.bands();
    if bands == 0 || w == 0 || h == 0 {
        return (cells, peak);
    }
    let (dots_w, dots_h) = (2 * w, 4 * h);
    for d in 0..dots_h {
        // Las bandas de esta fila de puntos (la de arriba, lo más agudo):
        // la más fuerte.
        let t0 = 1.0 - (d + 1) as f32 / dots_h as f32;
        let t1 = 1.0 - d as f32 / dots_h as f32;
        let b0 = ((t0 * bands as f32).floor() as usize).min(bands - 1);
        let b1 = ((t1 * bands as f32).ceil() as usize).clamp(b0 + 1, bands);
        let Some(band) = (b0..b1).max_by(|&a, &b| frame.energy(a).total_cmp(&frame.energy(b)))
        else {
            continue;
        };
        let e = frame.energy(band).clamp(0.0, 1.0);
        if e <= 0.0 {
            continue;
        }
        let p = frame.pan.get(band).copied().unwrap_or(0.0).clamp(-1.0, 1.0);
        let centre = (p + 1.0) * 0.5 * (dots_w - 1) as f32;
        // Más energía, más ancha (hasta una décima del ancho a cada lado).
        let spread = 1.0 + e * e * dots_w as f32 / 10.0;
        let from = (centre - spread).floor().max(0.0) as usize;
        let to = ((centre + spread).ceil() as usize).min(dots_w - 1);
        for x in from..=to {
            let v = e * (1.0 - (x as f32 - centre).abs() / (spread + 0.5)).max(0.0);
            let cell = (d / 4) * w + x / 2;
            peak[cell] = peak[cell].max(v);
            if v > DITHER[x % 2][d % 4] * 0.9 {
                cells[cell] |= BIT[x % 2][d % 4];
            }
        }
    }
    (cells, peak)
}

/// Los puntos del modo espejo en `width`×`rows` celdas: a la izquierda del
/// centro el canal izquierdo y a la derecha el derecho, cada uno de grave
/// (junto al centro) a agudo (hacia fuera), con su nivel de abajo arriba;
/// y el nivel más alto de cada celda.
pub fn mirror_dots(frame: &StereoFrame, width: u16, rows: u16) -> (Vec<u8>, Vec<f32>) {
    let (w, h) = (width as usize, rows as usize);
    let mut cells = vec![0u8; w * h];
    let mut peak = vec![0.0f32; w * h];
    let dots_w = 2 * w;
    let half = dots_w / 2;
    let dots_h = 4 * h;
    if half == 0 || h == 0 {
        return (cells, peak);
    }
    for k in 0..dots_w {
        let (levels, j) = if k < half {
            (&frame.left, half - 1 - k)
        } else {
            (&frame.right, k - half)
        };
        let t = (j as f32 + 0.5) / half as f32;
        let level = level_at(levels, t).clamp(0.0, 1.0);
        let top = ((1.0 - level) * dots_h as f32).round() as usize;
        for d in top..dots_h {
            let cell = (d / 4) * w + k / 2;
            cells[cell] |= BIT[k % 2][d % 4];
            peak[cell] = peak[cell].max(level);
        }
    }
    (cells, peak)
}

/// El color de una intensidad (de 0 a 1).
fn heat(v: f32) -> Color {
    if v > 0.66 {
        accent()
    } else if v > 0.33 {
        wave_color()
    } else {
        dim()
    }
}

/// Dibuja los puntos (o la rampa de ASCII) en `plot`.
fn paint(cells: &[u8], peak: &[f32], plot: Rect, buf: &mut Buffer) {
    let ascii = crate::theme::ascii();
    let w = plot.width as usize;
    for (i, (&bits, &p)) in cells.iter().zip(peak).enumerate() {
        let at = (plot.x + (i % w) as u16, plot.y + (i / w) as u16);
        if ascii {
            const RAMP: [char; 6] = [' ', '.', ':', '-', '=', '#'];
            let c = RAMP[((p.clamp(0.0, 1.0) * 5.0).round() as usize).min(5)];
            if c != ' ' && bits != 0 {
                buf[at].set_char(c).set_fg(heat(p));
            }
        } else if bits != 0 {
            buf[at].set_char(braille(bits)).set_fg(heat(p));
        }
    }
}

/// Las filas (desde arriba, dentro de `rows`) de las marcas del eje de
/// frecuencias vertical, sin pisarse.
pub fn axis_rows(rows: u16, rate: u32) -> Vec<(u16, &'static str)> {
    let e = edges(1, rate.max(1));
    let (low, high) = (e[0], e[1]);
    let mut out: Vec<(u16, &'static str)> = Vec::new();
    if rows == 0 {
        return out;
    }
    for (hz, label) in AXIS {
        if hz > high {
            break;
        }
        let t = (hz / low).ln() / (high / low).ln();
        let row = ((1.0 - t) * (rows - 1) as f32).round() as u16;
        // Las agudas, arriba; una marca por fila y con una libre entre dos.
        if out.iter().any(|&(r, _)| r.abs_diff(row) < 2) {
            continue;
        }
        out.push((row, label));
    }
    out
}

impl Widget for StereoPanel<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let block = Block::bordered()
            .border_style(Style::new().fg(accent()))
            .title(self.title);
        let inner = block.inner(area);
        block.render(area, buf);
        if area.width < MIN_WIDTH || area.height < MIN_HEIGHT {
            let text = format!("muy pequeña: al menos {MIN_WIDTH}×{MIN_HEIGHT}");
            let y = inner.y + inner.height / 2;
            if inner.height > 0 {
                buf.set_stringn(
                    inner.x,
                    y,
                    text,
                    inner.width as usize,
                    Style::new().fg(warning_color()),
                );
            }
            return;
        }
        let Some(frame) = self.frame else {
            buf.set_stringn(
                inner.x + 1,
                inner.y + inner.height / 2,
                "esperando el sonido del master…",
                inner.width.saturating_sub(1) as usize,
                Style::new().fg(dim()),
            );
            return;
        };
        let label = Style::new().fg(dim());
        let ascii = crate::theme::ascii();
        match self.mode {
            Mode::Field => {
                // El eje de frecuencias a la izquierda y L · C · R al pie.
                let plot = Rect {
                    x: inner.x + AXIS_WIDTH,
                    width: inner.width - AXIS_WIDTH,
                    height: inner.height - 1,
                    ..inner
                };
                for (row, text) in axis_rows(plot.height, frame.rate) {
                    let x = inner.x + AXIS_WIDTH - 1 - text.len() as u16;
                    buf.set_string(x, plot.y + row, text, label);
                }
                let foot = inner.bottom() - 1;
                let centre = plot.x + plot.width / 2;
                buf.set_string(plot.x, foot, "L", label);
                buf.set_string(centre, foot, "C", label);
                buf.set_string(plot.right() - 1, foot, "R", label);
                // La línea del centro, debajo de los puntos.
                for y in plot.y..plot.bottom() {
                    buf[(centre, y)]
                        .set_char(if ascii { '|' } else { '│' })
                        .set_fg(dim());
                }
                let (cells, peak) = field_dots(frame, plot.width, plot.height);
                paint(&cells, &peak, plot, buf);
            }
            Mode::Mirror => {
                // L y R arriba, el eje al pie, en espejo.
                let plot = Rect {
                    y: inner.y + 1,
                    height: inner.height - 2,
                    ..inner
                };
                let centre = plot.x + plot.width / 2;
                buf.set_string(plot.x, inner.y, "L", label);
                buf.set_string(plot.right() - 1, inner.y, "R", label);
                let half = plot.width / 2;
                let foot = inner.bottom() - 1;
                for (x, text) in axis_marks(half, frame.rate.max(1)) {
                    // Las graves, junto al centro (el «30» del eje no se
                    // pone: es el centro).
                    let w = text.len() as u16;
                    buf.set_string(centre + x, foot, text, label);
                    if let Some(left) = (centre).checked_sub(x + w) {
                        buf.set_string(left.max(plot.x), foot, text, label);
                    }
                }
                for y in plot.y..plot.bottom() {
                    buf[(centre, y)]
                        .set_char(if ascii { '|' } else { '│' })
                        .set_fg(dim());
                }
                let (cells, peak) = mirror_dots(frame, plot.width, plot.height);
                paint(&cells, &peak, plot, buf);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::stereo::{BANDS, StereoAnalyzer};
    use crate::viz::spectrum::SIZE;

    const RATE: u32 = 48_000;

    fn sine(hz: f32) -> Vec<f32> {
        (0..SIZE)
            .map(|i| 0.5 * (std::f32::consts::TAU * hz * i as f32 / RATE as f32).sin())
            .collect()
    }

    /// El cuadro, ya asentado, de un tono en `l` y `r` (veces su amplitud).
    fn frame(hz: f32, l: f32, r: f32) -> StereoFrame {
        let tone = sine(hz);
        let mut a = StereoAnalyzer::new();
        for _ in 0..12 {
            a.analyze(tone.iter().map(|s| s * l), tone.iter().map(|s| s * r), RATE);
        }
        a.analyze(tone.iter().map(|s| s * l), tone.iter().map(|s| s * r), RATE)
            .clone()
    }

    /// La columna de puntos media (ponderada) de lo encendido, de 0 a 1.
    fn centroid(cells: &[u8], width: u16) -> f32 {
        let w = width as usize;
        let (mut sum, mut n) = (0.0, 0.0);
        for (i, &bits) in cells.iter().enumerate() {
            for (col, rows) in BIT.iter().enumerate() {
                for &bit in rows {
                    if bits & bit != 0 {
                        sum += (2 * (i % w) + col) as f32;
                        n += 1.0;
                    }
                }
            }
        }
        sum / n / (2 * w - 1) as f32
    }

    #[test]
    fn in_the_field_left_goes_left_right_goes_right_and_both_stay_centred() {
        let (w, h) = (40, 12);
        let left = field_dots(&frame(1000.0, 1.0, 0.0), w, h).0;
        let right = field_dots(&frame(1000.0, 0.0, 1.0), w, h).0;
        let both = field_dots(&frame(1000.0, 1.0, 1.0), w, h).0;
        assert!(centroid(&left, w) < 0.15, "{}", centroid(&left, w));
        assert!(centroid(&right, w) > 0.85, "{}", centroid(&right, w));
        assert!(
            (centroid(&both, w) - 0.5).abs() < 0.05,
            "{}",
            centroid(&both, w)
        );
    }

    #[test]
    fn in_the_field_the_bass_is_low_and_the_treble_high() {
        let (w, h) = (40, 12);
        let row = |cells: &[u8]| {
            let first = cells.iter().position(|&b| b != 0).unwrap() / w as usize;
            let last = cells.iter().rposition(|&b| b != 0).unwrap() / w as usize;
            (first + last) / 2
        };
        let bass = row(&field_dots(&frame(80.0, 1.0, 1.0), w, h).0);
        let treble = row(&field_dots(&frame(8000.0, 1.0, 1.0), w, h).0);
        assert!(bass > 8 && treble < 4, "graves {bass}, agudos {treble}");
        // Silencio: nada.
        let quiet = StereoFrame {
            rate: RATE,
            left: vec![0.0; BANDS],
            right: vec![0.0; BANDS],
            pan: vec![0.0; BANDS],
        };
        assert!(field_dots(&quiet, w, h).0.iter().all(|&b| b == 0));
    }

    #[test]
    fn the_mirror_puts_each_channel_on_its_side_with_the_bass_in_the_middle() {
        let (w, h) = (40, 8);
        let lit = |cells: &[u8], range: std::ops::Range<usize>| {
            cells
                .iter()
                .enumerate()
                .filter(|(i, b)| **b != 0 && range.contains(&(i % w as usize)))
                .count()
        };
        // Solo a la izquierda: nada a la derecha del centro.
        let (cells, _) = mirror_dots(&frame(200.0, 1.0, 0.0), w, h);
        assert!(lit(&cells, 0..20) > 0 && lit(&cells, 20..40) == 0);
        let (cells, _) = mirror_dots(&frame(200.0, 0.0, 1.0), w, h);
        assert!(lit(&cells, 0..20) == 0 && lit(&cells, 20..40) > 0);
        // Un grave, junto al centro; un agudo, hacia fuera.
        let tallest = |cells: &[u8]| {
            (20..40usize)
                .max_by_key(|&x| (0..h as usize).filter(|y| cells[y * 40 + x] != 0).count())
                .unwrap()
        };
        let bass = tallest(&mirror_dots(&frame(60.0, 1.0, 1.0), w, h).0);
        let treble = tallest(&mirror_dots(&frame(10_000.0, 1.0, 1.0), w, h).0);
        assert!(bass < 26 && treble > 32, "graves {bass}, agudos {treble}");
    }

    #[test]
    fn the_vertical_axis_is_logarithmic_and_its_marks_do_not_touch() {
        let rows = axis_rows(20, RATE);
        let at = |l: &str| rows.iter().find(|r| r.1 == l).map(|r| r.0);
        assert!(at("100").unwrap() > at("1k").unwrap());
        assert!(at("1k").unwrap() > at("10k").unwrap());
        for a in &rows {
            for b in &rows {
                assert!(a == b || a.0.abs_diff(b.0) >= 2, "{rows:?}");
            }
        }
        assert!(axis_rows(0, RATE).is_empty());
    }

    #[test]
    fn modes_by_name() {
        for m in [Mode::Field, Mode::Mirror] {
            assert_eq!(Mode::from_name(m.name()), Some(m));
            assert_eq!(m.next().next(), m);
        }
        assert_eq!(Mode::from_name("x"), None);
    }
}
