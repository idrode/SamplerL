//! Vista de las lanes de automatización de la pista seleccionada: una
//! franja por lane con su curva dibujada en octavos de celda, sus puntos y
//! el cursor de lo que suena. El modelo ([`LanesView`]) es puro: sale del
//! modelo de la canción (`Song::lanes`, `TrackDef::lanes`) y de lo que
//! publica el motor de cada lane (`Status::lanes`, un atómico por lane).

use engine::{Lane, LaneLive, LaneMode, Status};
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Style, Stylize};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Clear, Widget};
use script::Song;

use crate::ui::{accent, dim};

/// Filas de la curva de cada lane (más la de su título).
pub const CURVE_ROWS: u16 = 4;

/// Lo que dibuja [`LanesPanel`].
#[derive(Debug, Clone, PartialEq)]
pub struct LanesView {
    pub track: String,
    pub lanes: Vec<LaneRow>,
}

/// Una lane de la pista.
#[derive(Debug, Clone, PartialEq)]
pub struct LaneRow {
    /// El parámetro (`"lowpass.cutoff"`).
    pub param: String,
    /// Su índice en la tabla del motor.
    pub index: usize,
    pub lane: Lane,
    /// Pasos de un compás (para decir lo que dura en compases).
    pub bar_steps: usize,
    /// Lo que publica el motor ahora (`None`: parado antes de la primera
    /// mezcla o aún no ha entrado).
    pub live: Option<LaneLive>,
    /// La escala de la vista y de la edición: mínimo, máximo y si es
    /// logarítmica (la del parámetro o la de `range`).
    pub scale: (f32, f32, bool),
    /// Con `range`, los valores escritos van de 0 a 1 (y si en log).
    pub range: Option<(f32, f32)>,
    pub range_log: bool,
    /// Si se puede editar desde el panel (la tabla de puntos está escrita
    /// tal cual) o por qué no.
    pub editable: Result<script::TableSpan, String>,
    /// El punto elegido para editarlo.
    pub selected: Option<usize>,
}

impl LaneRow {
    /// Lo que dura, en compases.
    pub fn bars(&self) -> f64 {
        self.lane.length() / self.bar_steps as f64
    }

    /// El título: parámetro, duración y modo.
    pub fn title(&self) -> String {
        let bars = self.bars();
        let length = if bars.fract() == 0.0 {
            format!(
                "{} {}",
                bars as u64,
                if bars == 1.0 { "compás" } else { "compases" }
            )
        } else {
            format!("{bars:.2} compases")
        };
        let mode = match self.lane.mode() {
            LaneMode::Loop => "loop",
            LaneMode::Once => "once",
            LaneMode::Hold => "hold",
        };
        let offset = self.lane.offset() / self.bar_steps as f64;
        let offset = if offset != 0.0 {
            format!(" · offset {offset}")
        } else {
            String::new()
        };
        format!("{} · {length} · {mode}{offset}", self.param)
    }

    /// Lo alto que va `value` en la franja (0 abajo, 1 arriba), en la
    /// escala de la vista (logarítmica en las frecuencias).
    pub fn level(&self, value: f32) -> f32 {
        let (lo, hi, log) = self.scale;
        if hi <= lo {
            return 0.5;
        }
        if log && lo > 0.0 {
            ((value.max(lo).ln() - lo.ln()) / (hi.ln() - lo.ln())).clamp(0.0, 1.0)
        } else {
            ((value - lo) / (hi - lo)).clamp(0.0, 1.0)
        }
    }

    /// El valor que va a la altura `level` (de 0 a 1) de la franja.
    pub fn value_at(&self, level: f32) -> f32 {
        let (lo, hi, log) = self.scale;
        let level = level.clamp(0.0, 1.0);
        if log && lo > 0.0 {
            (lo.ln() + level * (hi.ln() - lo.ln())).exp()
        } else {
            lo + level * (hi - lo)
        }
    }

    /// Lo que se lee ahora: el valor que publica el motor (con «fijo» fuera
    /// del tramo de una `once`), o nada si aún no ha llegado.
    pub fn reading(&self) -> Option<String> {
        let live = self.live?;
        let value = number(live.value);
        Some(match live.phase {
            Some(_) => value,
            None => format!("{value} fijo"),
        })
    }

    /// La columna (de `width`) de la posición `steps` de la lane.
    pub fn column(&self, steps: f64, width: u16) -> u16 {
        let x = steps / self.lane.length() * width as f64;
        (x.floor().max(0.0) as u16).min(width.saturating_sub(1))
    }
}

impl LanesView {
    /// Las lanes de la pista `track` (índice en `song.tracks`) con lo que
    /// publica el motor.
    pub fn new(song: Option<&Song>, track: usize, status: Option<&Status>) -> Option<LanesView> {
        let song = song?;
        let def = song.tracks.get(track)?;
        let lanes = def
            .lanes
            .iter()
            .filter_map(|(param, index)| {
                let lane_def = song.lanes.get(*index)?.as_ref()?;
                Some(LaneRow {
                    param: param.clone(),
                    index: *index,
                    lane: lane_def.lane,
                    bar_steps: lane_def.bar_steps,
                    live: status.and_then(|s| s.lanes.get(*index).copied().flatten()),
                    scale: lane_def.scale,
                    range: lane_def.range,
                    range_log: lane_def.range_log,
                    editable: lane_def.written.clone(),
                    selected: None,
                })
            })
            .collect();
        Some(LanesView {
            track: def.name.clone(),
            lanes,
        })
    }

    /// Filas que pide: título y curva por lane, más los bordes.
    pub fn height(&self) -> u16 {
        let lanes = self.lanes.len().max(1) as u16;
        lanes * (CURVE_ROWS + 1) + 2
    }
}

/// Un valor como se lee: sin decimales los grandes, con dos los pequeños.
pub fn number(value: f32) -> String {
    if value.abs() >= 100.0 {
        format!("{value:.0}")
    } else if value.abs() >= 10.0 {
        format!("{value:.1}")
    } else {
        format!("{value:.2}")
    }
}

/// Columnas de la barra del valor.
const BAR: usize = 12;

/// El panel: por lane, su título y su curva (rellena por debajo en octavos
/// de celda), con sus puntos (`●`) y el cursor de lo que suena.
pub struct LanesPanel<'a>(pub &'a LanesView);

impl Widget for LanesPanel<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let view = self.0;
        Clear.render(area, buf);
        let block = Block::bordered()
            .border_style(Style::new().fg(accent()))
            .title(format!(" lanes · {} ", view.track));
        let inner = block.inner(area);
        block.render(area, buf);
        if view.lanes.is_empty() {
            Line::from("la pista no tiene lanes: pista:auto(\"cutoff\", { {0, 200}, {1, 2000} })")
                .fg(dim())
                .render(inner, buf);
            return;
        }
        let mut y = inner.y;
        for row in &view.lanes {
            if y >= inner.bottom() {
                break;
            }
            let title = Rect {
                y,
                height: 1,
                ..inner
            };
            // El título y, a la derecha, lo que se lee: el valor y su barra.
            let head = match &row.editable {
                Ok(_) => row.title(),
                Err(_) => format!("{} · solo lectura", row.title()),
            };
            let mut spans = vec![Span::from(head.clone()).bold()];
            if let Some(reading) = row.reading() {
                let level = row.live.map_or(0.0, |l| row.level(l.value));
                let used = head.chars().count() + reading.chars().count() + BAR + 4;
                let gap = (inner.width as usize).saturating_sub(used).max(1);
                spans.push(Span::from(" ".repeat(gap)));
                spans.push(Span::from(format!("▸ {reading} ")).fg(accent()).bold());
                spans.push(Span::from(crate::ui::meter(level, BAR)).fg(accent()));
            }
            Line::from(spans).render(title, buf);
            y += 1;
            let rows = CURVE_ROWS.min(inner.bottom().saturating_sub(y));
            if rows == 0 {
                break;
            }
            let curve = Rect {
                y,
                height: rows,
                ..inner
            };
            draw_curve(row, curve, buf);
            y += rows;
        }
    }
}

/// Octavos de celda, de vacía a llena.
const BARS: [char; 9] = [' ', '▁', '▂', '▃', '▄', '▅', '▆', '▇', '█'];

/// La curva de `row` en `area`: cada columna, rellena hasta el valor de la
/// lane en su centro; los puntos, `●` en su altura; el cursor, la columna
/// de lo que suena resaltada.
fn draw_curve(row: &LaneRow, area: Rect, buf: &mut Buffer) {
    let (width, height) = (area.width, area.height);
    if width == 0 || height == 0 {
        return;
    }
    let levels = height as f32 * 8.0;
    let length = row.lane.length();
    let cursor = row
        .live
        .and_then(|l| l.phase)
        .map(|p| row.column(p as f64, width));
    for x in 0..width {
        let t = (x as f64 + 0.5) / width as f64 * length;
        let fill = (row.level(row.lane.at(t)) * levels).round() as u16;
        let here = cursor == Some(x);
        for r in 0..height {
            // Desde abajo: la fila `r` tiene los octavos de `8r` a `8r + 8`.
            let below = r * 8;
            let eighths = fill.saturating_sub(below).min(8) as usize;
            // En la columna del cursor, lo vacío es una raya.
            let char = if here && eighths == 0 {
                '│'
            } else {
                BARS[eighths]
            };
            buf[(area.x + x, area.bottom() - 1 - r)]
                .set_char(char)
                .set_fg(if here { accent() } else { dim() });
        }
    }
    // Los puntos, encima, en la fila de su valor (el elegido, `◉` y en
    // negrita).
    for (i, point) in row.lane.points().iter().enumerate() {
        let x = row.column(point.at, width);
        let level = (row.level(point.value) * (height as f32 - 1.0)).round() as u16;
        let chosen = row.selected == Some(i);
        let cell = buf[(area.x + x, area.bottom() - 1 - level)]
            .set_char(if chosen { '◉' } else { '●' })
            .set_fg(accent());
        if chosen {
            cell.modifier.insert(ratatui::style::Modifier::BOLD);
        }
    }
    // Lo que suena: un rombo sobre la curva, en la columna del cursor y a
    // la altura del valor que publica el motor.
    if let (Some(x), Some(live)) = (cursor, row.live) {
        let level = (row.level(live.value) * (height as f32 - 1.0)).round() as u16;
        buf[(area.x + x, area.bottom() - 1 - level)]
            .set_char('◆')
            .set_fg(accent());
    }
}
