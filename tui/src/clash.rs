//! Solapes de frecuencia entre pistas, en vivo y del análisis offline: la
//! matriz pista × banda (los 30 tercios de octava de `engine::masking`) con
//! la energía de cada celda y los choques resaltados, y la lista de choques
//! agrupados por rangos («kick × bass: 63-125 Hz»), por gravedad.
//!
//! En vivo ([`Live`]), cada cuadro analiza una ventana de lo último que ha
//! sonado de cada pista (las colas de las visualizaciones, que el motor
//! solo llena mientras el panel se ve) con el mismo [`WindowAnalyzer`] que
//! el análisis offline, guarda las de los últimos [`LIVE_SECONDS`] y
//! puntúa con [`MaskingReport::from_spectra`]: el mismo criterio, los
//! mismos grupos y las mismas sugerencias que `--analyze-masking`.

use std::collections::VecDeque;

use engine::masking::{BAND_NAMES, BANDS, BandSpectrum, MaskingOptions, WindowAnalyzer};
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::widgets::{Block, Widget};
use script::masking::MaskingReport;

use crate::ui::{accent, dim, error_color, warning_color};

/// Segundos de lo último que puntúa el análisis en vivo.
pub const LIVE_SECONDS: f32 = 4.0;

/// El análisis en vivo: las ventanas de cada pista de los últimos
/// [`LIVE_SECONDS`] y el informe de la última.
pub struct Live {
    analyzer: WindowAnalyzer,
    options: MaskingOptions,
    /// Ventanas que se guardan por pista.
    keep: usize,
    tracks: Vec<(String, VecDeque<[f32; BANDS]>)>,
    report: Option<MaskingReport>,
    /// Ventanas analizadas (para comprobar que cerrado no se calcula nada).
    runs: u64,
}

impl Live {
    /// A `rate`, con una ventana por cuadro a `hz` cuadros por segundo.
    pub fn new(rate: u32, hz: u32) -> Live {
        let options = MaskingOptions::default();
        Live {
            analyzer: WindowAnalyzer::new(rate, &options),
            options,
            keep: ((LIVE_SECONDS * hz.max(1) as f32).round() as usize).max(2),
            tracks: Vec::new(),
            report: None,
            runs: 0,
        }
    }

    /// Una ventana más de cada pista de `sources` (su nombre y lo último
    /// que ha sonado de ella; las de antes que ya no están, fuera) y el
    /// informe de lo guardado, con `priority`.
    pub fn frame<'a>(
        &mut self,
        sources: impl IntoIterator<Item = (&'a str, &'a VecDeque<f32>)>,
        priority: &[String],
    ) {
        let window = self.analyzer.window_len();
        let mut tracks = Vec::new();
        for (name, history) in sources {
            let mut rows = self
                .tracks
                .iter()
                .position(|(n, _)| n == name)
                .map(|i| std::mem::take(&mut self.tracks[i].1))
                .unwrap_or_default();
            let from = history.len().saturating_sub(window);
            rows.push_back(self.analyzer.analyze(history.range(from..).copied()));
            self.runs += 1;
            while rows.len() > self.keep {
                rows.pop_front();
            }
            tracks.push((name.to_string(), rows));
        }
        self.tracks = tracks;
        self.report = (self.tracks.len() >= 2).then(|| self.build(priority));
    }

    fn build(&self, priority: &[String]) -> MaskingReport {
        let spectra: Vec<BandSpectrum> = self
            .tracks
            .iter()
            .map(|(_, rows)| {
                let mut s = BandSpectrum::empty(&self.analyzer);
                s.power = rows.iter().copied().collect();
                s
            })
            .collect();
        MaskingReport::from_spectra(
            0,
            self.tracks.iter().map(|(n, _)| n.clone()).collect(),
            &spectra,
            &self.options,
            priority.to_vec(),
        )
    }

    /// El informe de lo último (`None` con menos de dos pistas o antes de
    /// la primera ventana).
    pub fn report(&self) -> Option<&MaskingReport> {
        self.report.as_ref()
    }

    pub fn report_mut(&mut self) -> Option<&mut MaskingReport> {
        self.report.as_mut()
    }

    /// Olvida lo guardado (al dejar de verse: al volver, no sale lo de
    /// antes).
    pub fn clear(&mut self) {
        self.tracks.clear();
        self.report = None;
    }

    /// Ventanas analizadas desde que se creó.
    #[cfg(test)]
    pub fn runs(&self) -> u64 {
        self.runs
    }

    /// Segundos de lo guardado (lo de la pista que más tiene).
    pub fn seconds(&self, hz: u32) -> f32 {
        let most = self.tracks.iter().map(|(_, r)| r.len()).max().unwrap_or(0);
        most as f32 / hz.max(1) as f32
    }
}

/// La matriz y la lista que se dibujan (del informe en vivo o del último
/// análisis offline).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct MatrixView {
    pub tracks: Vec<String>,
    /// Nivel de cada pista y banda en dBFS.
    pub levels: Vec<[f32; BANDS]>,
    /// La puntuación del choque más fuerte de cada celda (0: ninguno).
    pub hot: Vec<[f32; BANDS]>,
    /// Los choques agrupados, por gravedad: su línea y su puntuación.
    pub groups: Vec<(String, f32)>,
    /// El elegido con el cursor del panel, y sus celdas (pistas y bandas).
    pub selected: Option<usize>,
    pub selected_cells: Vec<(usize, (usize, usize))>,
    /// Las sugerencias del elegido (líneas de Lua) y cuál se ha elegido.
    pub fixes: Vec<String>,
    pub chosen_fix: usize,
}

impl MatrixView {
    /// La de `report`, con el grupo `selected` elegido (si el cursor está
    /// en el panel) y sus sugerencias `fixes`.
    pub fn of(report: &MaskingReport, selected: Option<usize>, fixes: Vec<String>) -> MatrixView {
        let groups = report.groups();
        let mut hot = vec![[0.0f32; BANDS]; report.tracks.len()];
        for g in &groups {
            let pair = report.pairs.iter().find(|p| (p.a, p.b) == (g.a, g.b));
            let scores: Vec<(usize, f32)> = (g.bands.0..=g.bands.1)
                .map(|band| {
                    let c = pair.and_then(|p| p.bands[band]);
                    (band, c.map_or(g.score, |c| c.score))
                })
                .collect();
            for t in [g.a, g.b] {
                for &(band, score) in &scores {
                    hot[t][band] = hot[t][band].max(score);
                }
            }
        }
        let selected = selected.filter(|&i| i < groups.len());
        let selected_cells = selected.map_or_else(Vec::new, |i| {
            let g = groups[i];
            vec![(g.a, g.bands), (g.b, g.bands)]
        });
        MatrixView {
            tracks: report.tracks.clone(),
            levels: report.levels.clone(),
            hot,
            groups: groups
                .iter()
                .map(|g| (report.group_text(g), g.score))
                .collect(),
            selected,
            selected_cells,
            fixes,
            chosen_fix: 0,
        }
    }

    /// Filas que pide (sin los bordes): la cabecera, una por pista, una
    /// por choque y las sugerencias.
    pub fn height(&self) -> u16 {
        let groups = self.groups.len().max(1);
        (1 + self.tracks.len() + groups + self.fixes.len()) as u16
    }
}

/// Columnas del nombre de la pista en la matriz.
const NAME_WIDTH: u16 = 8;

/// Sombreado de una celda por su nivel (dBFS).
fn shade(db: f32, ascii: bool) -> char {
    const UNICODE: [char; 5] = [' ', '░', '▒', '▓', '█'];
    const ASCII: [char; 5] = [' ', '.', ':', '+', '#'];
    let i = if db < -60.0 {
        0
    } else if db < -45.0 {
        1
    } else if db < -30.0 {
        2
    } else if db < -15.0 {
        3
    } else {
        4
    };
    if ascii { ASCII[i] } else { UNICODE[i] }
}

/// Color de un choque según lo fuerte que es.
pub fn clash_style(score: f32) -> Style {
    match script::masking::strength(score) {
        "fuerte" => Style::new().fg(error_color()).bold(),
        "medio" => Style::new().fg(warning_color()),
        _ => Style::new().fg(accent()),
    }
}

/// Dónde quedó cada cosa del panel (para el ratón).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct MatrixLayout {
    /// Lo de dentro del borde.
    pub inner: Rect,
    /// Fila de la primera pista, columnas de una banda y dónde empiezan.
    pub first_track: u16,
    pub tracks: u16,
    pub cell: u16,
    pub cells_x: u16,
    /// Fila del primer choque que se ve de la lista, cuántos se ven y el
    /// índice del primero (la lista se desplaza para que se vea el elegido).
    pub first_group: u16,
    pub groups: u16,
    pub group_offset: usize,
    /// Fila de la primera sugerencia y cuántas.
    pub first_fix: u16,
    pub fixes: u16,
}

/// Lo que hay bajo el ratón en el panel.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MatrixHit {
    /// Una celda: pista y banda.
    Cell(usize, usize),
    /// Un choque de la lista.
    Group(usize),
    /// Una sugerencia del elegido.
    Fix(usize),
}

impl MatrixLayout {
    pub fn hit(&self, x: u16, y: u16) -> Option<MatrixHit> {
        if !self.inner.contains(ratatui::layout::Position::new(x, y)) {
            return None;
        }
        if (self.first_track..self.first_track + self.tracks).contains(&y)
            && x >= self.cells_x
            && self.cell > 0
        {
            let band = ((x - self.cells_x) / self.cell) as usize;
            return (band < BANDS)
                .then_some(MatrixHit::Cell((y - self.first_track) as usize, band));
        }
        if (self.first_group..self.first_group + self.groups).contains(&y) {
            return Some(MatrixHit::Group(
                self.group_offset + (y - self.first_group) as usize,
            ));
        }
        if (self.first_fix..self.first_fix + self.fixes).contains(&y) {
            return Some(MatrixHit::Fix((y - self.first_fix) as usize));
        }
        None
    }
}

/// Dibuja la matriz en `inner` (ya sin borde) y dice dónde quedó cada
/// cosa. `selected_track`: la pista elegida (su nombre, con el acento).
pub fn draw_matrix(
    view: &MatrixView,
    selected_track: Option<&str>,
    inner: Rect,
    buf: &mut Buffer,
) -> MatrixLayout {
    let ascii = crate::theme::ascii();
    let mut layout = MatrixLayout {
        inner,
        ..Default::default()
    };
    if inner.width <= NAME_WIDTH || inner.height == 0 {
        return layout;
    }
    // Dos columnas por banda si caben; si no, una.
    let room = inner.width - NAME_WIDTH;
    let cell = if room >= 2 * BANDS as u16 { 2 } else { 1 };
    let cells_x = inner.x + NAME_WIDTH;
    let mut y = inner.y;
    let bottom = inner.y + inner.height;
    // Cabecera: el nombre de una banda de cada cuatro (desde 25 Hz:
    // 25, 63, 160, 400, 1k, 2.5k, 6.3k, 16k), donde cabe.
    let mut free = cells_x;
    for band in (0..BANDS).step_by(4) {
        let x = cells_x + band as u16 * cell;
        let name = BAND_NAMES[band];
        if x >= free && x + name.len() as u16 <= inner.x + inner.width {
            buf.set_string(x, y, name, Style::new().fg(dim()));
            free = x + name.len() as u16 + 1;
        }
    }
    y += 1;
    layout.first_track = y;
    let shown_tracks = (view.tracks.len() as u16).min(bottom.saturating_sub(y));
    layout.tracks = shown_tracks;
    layout.cell = cell;
    layout.cells_x = cells_x;
    for (t, name) in view.tracks.iter().enumerate().take(shown_tracks as usize) {
        let style = if selected_track == Some(name.as_str()) {
            Style::new().fg(accent()).bold()
        } else {
            Style::new()
        };
        buf.set_stringn(inner.x, y, name, (NAME_WIDTH - 1) as usize, style);
        for band in 0..BANDS {
            let x = cells_x + band as u16 * cell;
            if x + cell > inner.x + inner.width {
                break;
            }
            let level = view.levels.get(t).map_or(-120.0, |l| l[band]);
            let score = view.hot.get(t).map_or(0.0, |h| h[band]);
            let mut glyph = shade(level, ascii);
            let mut style = if score > 0.0 {
                // Un choque sin apenas nivel en la celda: que se vea igual.
                if glyph == ' ' {
                    glyph = if ascii { '.' } else { '░' };
                }
                clash_style(score)
            } else {
                Style::new().fg(dim())
            };
            if view
                .selected_cells
                .iter()
                .any(|&(st, (lo, hi))| st == t && (lo..=hi).contains(&band))
            {
                style = style.add_modifier(Modifier::REVERSED);
            }
            let text: String = std::iter::repeat_n(glyph, cell as usize).collect();
            buf.set_string(x, y, text, style);
        }
        y += 1;
    }
    // La lista de choques.
    layout.first_group = y;
    if view.groups.is_empty() {
        if y < bottom {
            buf.set_stringn(
                inner.x,
                y,
                "sin choques",
                inner.width as usize,
                Style::new().fg(dim()),
            );
        }
        return layout;
    }
    let room = bottom.saturating_sub(y) as usize;
    let fixes = view.fixes.len().min(room.saturating_sub(1));
    let room = room - fixes;
    // Los que caben, con el elegido dentro.
    let first = match view.selected {
        Some(s) if s >= room => s + 1 - room,
        _ => 0,
    };
    let marker = if ascii { ">" } else { "▶" };
    for (i, (text, score)) in view.groups.iter().enumerate().skip(first).take(room) {
        let chosen = view.selected == Some(i);
        let lead = if chosen { marker } else { " " };
        let style = if chosen {
            clash_style(*score).add_modifier(Modifier::REVERSED)
        } else {
            clash_style(*score)
        };
        buf.set_stringn(inner.x, y, lead, 1, style);
        buf.set_stringn(inner.x + 2, y, text, inner.width as usize - 2, style);
        y += 1;
    }
    layout.groups = y - layout.first_group;
    layout.group_offset = first;
    layout.first_fix = y;
    for (i, fix) in view.fixes.iter().enumerate().take(fixes) {
        let style = if i == view.chosen_fix {
            Style::new().fg(accent()).add_modifier(Modifier::BOLD)
        } else {
            Style::new().fg(dim())
        };
        let fix = if ascii {
            fix.replace('▸', ">")
        } else {
            fix.clone()
        };
        buf.set_stringn(inner.x + 2, y, fix, inner.width as usize - 2, style);
        y += 1;
    }
    layout.fixes = fixes as u16;
    layout
}

/// El panel entero: borde con su título y la matriz.
pub struct MatrixPanel<'a> {
    pub view: &'a MatrixView,
    pub title: String,
    pub stale: bool,
    pub selected_track: Option<&'a str>,
}

impl MatrixPanel<'_> {
    pub fn render_with_layout(self, area: Rect, buf: &mut Buffer) -> MatrixLayout {
        let border = if self.stale {
            Style::new().fg(warning_color())
        } else {
            Style::new()
        };
        let block = Block::bordered().border_style(border).title(self.title);
        let mut inner = block.inner(area);
        block.render(area, buf);
        if self.stale && inner.height > 0 {
            buf.set_stringn(
                inner.x,
                inner.y + inner.height - 1,
                "⚠ de una carga anterior",
                inner.width as usize,
                Style::new().fg(warning_color()),
            );
            inner.height -= 1;
        }
        draw_matrix(self.view, self.selected_track, inner, buf)
    }
}

impl Widget for MatrixPanel<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        self.render_with_layout(area, buf);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sine(freq: f32, amp: f32, n: usize, rate: u32) -> VecDeque<f32> {
        (0..n)
            .map(|i| amp * (std::f32::consts::TAU * freq * i as f32 / rate as f32).sin())
            .collect()
    }

    #[test]
    fn two_low_sines_clash_live_and_a_high_one_does_not() {
        let rate = 48_000;
        let mut live = Live::new(rate, 15);
        let kick = sine(80.0, 0.5, 16_384, rate);
        let bass = sine(80.0, 0.4, 16_384, rate);
        let hat = sine(6000.0, 0.3, 16_384, rate);
        for _ in 0..10 {
            live.frame([("kick", &kick), ("bass", &bass), ("hat", &hat)], &[]);
        }
        let report = live.report().unwrap();
        let groups = report.groups();
        assert!(!groups.is_empty());
        let g = groups[0];
        assert_eq!((g.a, g.b), (0, 1));
        assert!(BAND_NAMES[g.bands.0..=g.bands.1].contains(&"80"), "{g:?}");
        assert!(groups.iter().all(|g| g.a != 2 && g.b != 2), "{groups:?}");
        // El nivel de la celda, en dBFS: un seno de 0,5 da unos −6 dB
        // entre su banda y las vecinas.
        let band = BAND_NAMES.iter().position(|b| *b == "80").unwrap();
        let e: f32 = (band - 1..=band + 1)
            .map(|b| 10f32.powf(report.levels[0][b] / 10.0))
            .sum();
        assert!(
            (10.0 * e.log10() + 6.02).abs() < 0.5,
            "{}",
            10.0 * e.log10()
        );
        assert_eq!(live.runs(), 30);
        // Lo que se guarda es lo de los últimos segundos.
        assert!(live.seconds(15) <= LIVE_SECONDS + 0.1);
    }

    #[test]
    fn the_matrix_marks_the_clash_and_lists_it() {
        let rate = 48_000;
        let mut live = Live::new(rate, 15);
        let a = sine(100.0, 0.5, 8192, rate);
        let b = sine(100.0, 0.5, 8192, rate);
        live.frame([("kick", &a), ("bass", &b)], &[]);
        let view = MatrixView::of(live.report().unwrap(), Some(0), Vec::new());
        let band = BAND_NAMES.iter().position(|b| *b == "100").unwrap();
        assert!(view.hot[0][band] > 0.6 && view.hot[1][band] > 0.6);
        assert!(
            view.groups[0].0.starts_with("kick × bass: "),
            "{:?}",
            view.groups
        );
        let area = Rect::new(0, 0, 70, 8);
        let mut buf = Buffer::empty(area);
        let layout = draw_matrix(&view, None, area, &mut buf);
        // La celda del choque, en rojo e invertida (es el elegido).
        let x = layout.cells_x + band as u16 * layout.cell;
        let cell = &buf[(x, layout.first_track)];
        assert_eq!(cell.fg, error_color());
        assert!(cell.modifier.contains(Modifier::REVERSED));
        assert_eq!(
            layout.hit(x, layout.first_track + 1),
            Some(MatrixHit::Cell(1, band))
        );
        assert_eq!(layout.hit(3, layout.first_group), Some(MatrixHit::Group(0)));
    }
}
