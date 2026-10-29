//! Vista del sidechain: por cada efecto de dinámica de una pista (EQ
//! dinámico, con o sin sidechain, y compresor), quién lo dispara, cuánto
//! reduce ahora y las curvas de los últimos segundos (el nivel del que
//! dispara y la reducción). El motor publica la reducción de cada ranura
//! con atómicos (`TrackStatus::reduction`); aquí se guarda el historial,
//! una muestra por `tick`, y se dibuja.

use std::collections::VecDeque;

use engine::{Fx, Status};
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Style, Stylize};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Clear, Paragraph, Widget};
use script::Song;

use crate::ui::{accent, dim, warning_color};

/// Muestras de historial que se guardan (unos 5 s a 20 por segundo).
pub const HISTORY: usize = 100;
/// Reducción a fondo de escala del compresor (el EQ dinámico usa su
/// `range`).
const COMPRESSOR_SCALE_DB: f32 = 24.0;

/// El historial de cada efecto de dinámica que se ha visto.
#[derive(Debug, Default)]
pub struct Sidechain {
    rows: Vec<History>,
}

#[derive(Debug)]
struct History {
    track: String,
    slot: usize,
    /// Reducción (dB) y nivel del que dispara (pico de 0 a 1), de la más
    /// vieja a la más nueva.
    reduction: VecDeque<f32>,
    key: VecDeque<f32>,
}

impl Sidechain {
    /// Apunta una muestra de cada efecto de dinámica de `song`, con la
    /// instantánea del motor `status` (`slot` da la pista del motor de un
    /// nombre). Los que ya no están se olvidan.
    pub fn observe(
        &mut self,
        song: Option<&Song>,
        slot: impl Fn(&str) -> Option<usize>,
        status: &Status,
    ) {
        let mut seen = Vec::new();
        for track in song.map_or(&[][..], |s| &s.tracks[..]) {
            let Some(t) = slot(&track.name) else { continue };
            for (i, fx) in track.fx.iter().enumerate() {
                if !matches!(fx, Fx::DynEq(_) | Fx::Compressor { .. }) {
                    continue;
                }
                let source = trigger(track, i);
                let key_slot = source.as_deref().map_or(Some(t), &slot);
                let key = key_slot.map_or(0.0, |k| status.tracks[k].peak.min(1.0));
                let reduction = status.tracks[t].reduction[i];
                let row = match self
                    .rows
                    .iter_mut()
                    .position(|r| r.track == track.name && r.slot == i)
                {
                    Some(p) => &mut self.rows[p],
                    None => {
                        self.rows.push(History {
                            track: track.name.clone(),
                            slot: i,
                            reduction: VecDeque::with_capacity(HISTORY),
                            key: VecDeque::with_capacity(HISTORY),
                        });
                        self.rows.last_mut().expect("recién puesta")
                    }
                };
                for (queue, value) in [(&mut row.reduction, reduction), (&mut row.key, key)] {
                    if queue.len() == HISTORY {
                        queue.pop_front();
                    }
                    queue.push_back(value);
                }
                seen.push((track.name.clone(), i));
            }
        }
        self.rows
            .retain(|r| seen.iter().any(|(n, i)| *n == r.track && *i == r.slot));
    }

    /// Lo que se enseña, en el orden de las pistas de `song`.
    pub fn view(&self, song: Option<&Song>) -> SidechainView {
        let mut rows = Vec::new();
        for track in song.map_or(&[][..], |s| &s.tracks[..]) {
            for (i, fx) in track.fx.iter().enumerate() {
                let (effect, scale) = match fx {
                    Fx::DynEq(p) => (
                        format!("eq dinámico {}", hz(p.freq_hz)),
                        p.range_db.max(1.0),
                    ),
                    Fx::Compressor { .. } => ("compresor".to_string(), COMPRESSOR_SCALE_DB),
                    _ => continue,
                };
                let history = self
                    .rows
                    .iter()
                    .find(|r| r.track == track.name && r.slot == i);
                let collect = |q: Option<&VecDeque<f32>>| -> Vec<f32> {
                    q.map_or_else(Vec::new, |q| q.iter().copied().collect())
                };
                let reduction = collect(history.map(|h| &h.reduction));
                rows.push(SidechainRow {
                    track: track.name.clone(),
                    effect,
                    source: trigger(track, i),
                    now_db: reduction.last().copied().unwrap_or(0.0),
                    scale_db: scale,
                    reduction,
                    key: collect(history.map(|h| &h.key)),
                });
            }
        }
        SidechainView { rows }
    }
}

/// La pista que dispara el efecto `i` de `track` por sidechain (`None`: se
/// escucha a sí misma).
fn trigger(track: &script::TrackDef, i: usize) -> Option<String> {
    track
        .sidechains
        .iter()
        .find(|(slot, _)| *slot == i)
        .map(|(_, name)| name.clone())
}

fn hz(f: f32) -> String {
    if f >= 1000.0 {
        format!("{:.1} kHz", f / 1000.0)
    } else {
        format!("{f:.0} Hz")
    }
}

/// Lo que dibuja [`SidechainPanel`].
#[derive(Debug, Clone, PartialEq, Default)]
pub struct SidechainView {
    pub rows: Vec<SidechainRow>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct SidechainRow {
    pub track: String,
    /// «eq dinámico 120 Hz», «compresor».
    pub effect: String,
    /// Quién lo dispara (`None`: la propia pista).
    pub source: Option<String>,
    /// Reducción de ahora (la última muestra), en dB.
    pub now_db: f32,
    /// Reducción a fondo de escala (el `range` del EQ dinámico).
    pub scale_db: f32,
    /// Historial (de la más vieja a la más nueva): reducción en dB y nivel
    /// del que dispara (0 a 1).
    pub reduction: Vec<f32>,
    pub key: Vec<f32>,
}

impl SidechainView {
    /// Filas que pide con tres líneas por efecto (más los bordes).
    pub fn height(&self) -> u16 {
        (3 * self.rows.len().max(1) + 2) as u16
    }
}

/// El panel: por efecto, una línea con la pista, el efecto, quién lo
/// dispara y la reducción de ahora con su barra; y, si cabe, debajo las dos
/// curvas (nivel del disparo y reducción). Si no caben tres líneas por
/// efecto, una con la curva de la reducción al lado.
pub struct SidechainPanel<'a>(pub &'a SidechainView);

impl Widget for SidechainPanel<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let view = self.0;
        Clear.render(area, buf);
        let block = Block::bordered()
            .border_style(Style::new().fg(accent()))
            .title(" sidechain ");
        let inner = block.inner(area);
        let width = inner.width as usize;
        let mut lines: Vec<Line> = Vec::new();
        if view.rows.is_empty() {
            lines.push(Line::from("ninguna pista tiene un EQ dinámico ni un compresor").fg(dim()));
        }
        let full = inner.height as usize >= 3 * view.rows.len();
        for row in &view.rows {
            let source = match &row.source {
                Some(s) => format!("← {s}"),
                None => "← sí misma".into(),
            };
            let head = format!("{} · {} {source}", row.track, row.effect);
            let db = format!(" −{:.1} dB ", row.now_db);
            let color = if row.now_db > 0.05 {
                warning_color()
            } else {
                dim()
            };
            if full {
                let bar = width.saturating_sub(head.chars().count() + db.len() + 1);
                lines.push(Line::from(vec![
                    Span::from(head).bold(),
                    Span::from(db).fg(color),
                    Span::from(crate::ui::meter(
                        (row.now_db / row.scale_db).clamp(0.0, 1.0),
                        bar,
                    ))
                    .fg(color),
                ]));
                let curve = width.saturating_sub(12);
                lines.push(Line::from(vec![
                    Span::from("  disparo   ").fg(dim()),
                    Span::from(sparkline(&row.key, 1.0, curve)).fg(accent()),
                ]));
                lines.push(Line::from(vec![
                    Span::from("  reducción ").fg(dim()),
                    Span::from(sparkline(&row.reduction, row.scale_db, curve)).fg(color),
                ]));
            } else {
                let curve = width.saturating_sub(head.chars().count() + db.len() + 1);
                lines.push(Line::from(vec![
                    Span::from(head).bold(),
                    Span::from(db).fg(color),
                    Span::from(sparkline(&row.reduction, row.scale_db, curve)).fg(color),
                ]));
            }
        }
        Paragraph::new(lines).block(block).render(area, buf);
    }
}

/// Las últimas `width` muestras de `values` como barras de octavos de
/// celda (`full` es una barra entera); a la izquierda, lo que falta, en
/// blanco.
pub fn sparkline(values: &[f32], full: f32, width: usize) -> String {
    const BARS: [char; 9] = [' ', '▁', '▂', '▃', '▄', '▅', '▆', '▇', '█'];
    let shown = &values[values.len().saturating_sub(width)..];
    let mut out = " ".repeat(width - shown.len());
    for &v in shown {
        let level = (v / full).clamp(0.0, 1.0);
        // Algo distinto de cero se ve siempre (al menos un octavo).
        let i = if level > 0.0 {
            ((level * 8.0).round() as usize).max(1)
        } else {
            0
        };
        out.push(BARS[i]);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_sparkline_keeps_the_last_values_and_shows_any_reduction() {
        assert_eq!(sparkline(&[0.0, 0.5, 1.0, 2.0], 1.0, 6), "   ▄██");
        assert_eq!(sparkline(&[0.0, 0.01, 0.25], 1.0, 2), "▁▂");
        assert_eq!(sparkline(&[], 1.0, 3), "   ");
    }
}
