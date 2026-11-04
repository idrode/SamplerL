//! `punchcard` y `pianoroll`: el ciclo de la pista en una rejilla, del
//! modelo del piano roll (lo que sonará, con `every`, `chance`... ya
//! resueltos), con el cursor de lo que suena. `punchcard` solo tiene las
//! filas que suenan (los cortes o las notas que hay), compacto;
//! `pianoroll`, todas las del rango, con el nombre de cada nota.

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::widgets::{Block, Clear, Widget};

use crate::roll::Roll;
use crate::ui::{accent, dim};

/// Lo que dibuja [`GridPanel`].
#[derive(Debug, Clone, PartialEq)]
pub struct GridView {
    pub title: String,
    /// Filas de dentro (sin los bordes).
    pub size: u16,
    /// Las filas, de arriba abajo: su nombre y, por columna del ciclo, si
    /// suena ahí (una nota que empieza `Head`, una que sigue `Hold`).
    pub rows: Vec<(String, Vec<Cell>)>,
    /// Columnas del ciclo.
    pub steps: usize,
    /// La columna que suena.
    pub playhead: Option<usize>,
    /// Con el nombre de cada fila a la izquierda (`pianoroll`).
    pub labels: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Cell {
    Empty,
    Head,
    Hold,
}

/// La rejilla de `roll`: las filas que suenan (`compact`) o todas las del
/// rango, de la más alta a la más baja.
pub fn build(
    roll: &Roll,
    title: String,
    size: u16,
    playhead: Option<usize>,
    compact: bool,
) -> GridView {
    let steps = roll.steps().max(1);
    let mut pitches: Vec<i32> = if compact {
        let mut p: Vec<i32> = roll.notes.iter().map(|n| n.pitch).collect();
        p.sort_unstable();
        p.dedup();
        p
    } else {
        let (lo, hi) = roll.range().unwrap_or((60, 60));
        (lo..=hi).collect()
    };
    pitches.reverse();
    let rows = pitches
        .iter()
        .map(|&pitch| {
            let mut cells = vec![Cell::Empty; steps];
            for n in roll.notes.iter().filter(|n| n.pitch == pitch) {
                for k in 0..n.len.max(1) {
                    let col = (n.col + k) % steps;
                    cells[col] = if k == 0 { Cell::Head } else { Cell::Hold };
                }
            }
            (roll.pitch_name(pitch), cells)
        })
        .collect();
    GridView {
        title,
        size,
        rows,
        steps,
        playhead,
        labels: !compact,
    }
}

pub struct GridPanel<'a>(pub &'a GridView);

impl Widget for GridPanel<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let g = self.0;
        Clear.render(area, buf);
        let block = Block::bordered()
            .border_style(Style::new().fg(accent()))
            .title(g.title.as_str());
        let inner = block.inner(area);
        block.render(area, buf);
        if inner.width == 0 || inner.height == 0 {
            return;
        }
        let label = if g.labels {
            g.rows
                .iter()
                .map(|(n, _)| n.chars().count())
                .max()
                .unwrap_or(0) as u16
                + 1
        } else {
            0
        };
        let width = inner.width.saturating_sub(label).max(1);
        let ascii = crate::theme::ascii();
        // Si hay más filas que alto, las de en medio (lo más común).
        let skip = g.rows.len().saturating_sub(inner.height as usize) / 2;
        for (r, (name, cells)) in g
            .rows
            .iter()
            .skip(skip)
            .take(inner.height as usize)
            .enumerate()
        {
            let y = inner.y + r as u16;
            if g.labels {
                buf.set_string(inner.x, y, name, Style::new().fg(dim()));
            }
            for x in 0..width {
                // La columna del ciclo que cae en esta celda.
                let col = (x as usize * g.steps) / width as usize;
                let cell = cells[col.min(g.steps - 1)];
                let here = g.playhead.is_some_and(|p| {
                    let start = (p * width as usize) / g.steps;
                    x as usize == start
                });
                let (ch, color) = match cell {
                    Cell::Head => (if ascii { '#' } else { '█' }, accent()),
                    Cell::Hold => (if ascii { '=' } else { '▒' }, accent()),
                    Cell::Empty if here => (if ascii { '|' } else { '│' }, accent()),
                    Cell::Empty => (if ascii { '.' } else { '·' }, dim()),
                };
                let mut style = Style::new().fg(color);
                if here {
                    style = style.add_modifier(ratatui::style::Modifier::REVERSED);
                }
                buf.set_string(inner.x + label + x, y, ch.to_string(), style);
            }
        }
    }
}
