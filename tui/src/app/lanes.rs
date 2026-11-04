//! Editar las lanes desde su panel: con el ratón (clic en un hueco añade un
//! punto, arrastrar uno lo mueve, clic derecho lo borra) y, con un punto
//! elegido, con las teclas (`c` cambia su curva, Supr lo borra, las flechas
//! lo afinan, Esc lo suelta). Cada edición reescribe la tabla de puntos de
//! su `auto()` en el texto, en un paso de deshacer, y carga; si la tabla no
//! está escrita tal cual, no se toca nada y se dice por qué.

use ratatui::layout::{Position, Rect};
use ratatui_textarea::{Input, Key};

use super::mouse::MouseAction;
use super::{App, Flow};
use crate::lanes_edit::{self, LaneEdit};
use crate::lanes_panel::{CURVE_ROWS, LaneRow, LanesView};

/// El punto elegido: de qué pista y lane (por su parámetro) y cuál.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct LaneFocus {
    pub track: String,
    pub param: String,
    pub point: usize,
}

/// Un punto que se arrastra: a dónde va (en pasos y su valor). Se escribe
/// al soltar.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct LaneDrag {
    pub at: f64,
    pub value: f32,
}

/// Columnas a cada lado de un punto en las que un clic lo agarra.
const GRAB: u16 = 1;

impl App {
    /// Dónde se ha dibujado el panel de las lanes (para el ratón).
    pub fn set_lanes_area(&mut self, area: Option<Rect>) {
        self.lanes_area = area;
    }

    /// Dónde se dibujó el panel de las lanes (para los tests).
    #[cfg(test)]
    pub fn lanes_area_for_test(&self) -> Option<Rect> {
        self.lanes_area
    }

    /// Las lanes de la pista seleccionada, con el punto elegido y lo que se
    /// arrastra.
    pub(super) fn lanes_view(&self) -> Option<LanesView> {
        let mut view = LanesView::new(self.host.song(), self.selected, self.status.as_ref())?;
        if let Some(focus) = &self.lanes_focus
            && focus.track == view.track
            && let Some(row) = view.lanes.iter_mut().find(|r| r.param == focus.param)
        {
            row.selected = Some(focus.point);
            // Lo que se arrastra se ve antes de escribirlo.
            if let Some(drag) = self.lane_drag
                && let Ok(edited) = lanes_edit::apply(
                    row,
                    LaneEdit::Move {
                        index: focus.point,
                        at: drag.at,
                        value: drag.value,
                    },
                )
                && let Some(lane) = preview(row, &edited.points)
            {
                row.lane = lane;
            }
        }
        Some(view)
    }

    /// El ratón en el panel de las lanes (o arrastrando un punto): `true`
    /// si era para él.
    pub(super) fn lanes_mouse(&mut self, action: MouseAction, x: u16, y: u16) -> bool {
        let Some(area) = self.lanes_area.filter(|_| self.lanes_open) else {
            return false;
        };
        let dragging = self.lane_drag.is_some();
        if !dragging && !area.contains(Position::new(x, y)) {
            if action == MouseAction::Down {
                self.lanes_focus = None;
            }
            return false;
        }
        let Some(view) = self.lanes_view() else {
            return true;
        };
        let inner = Rect {
            x: area.x + 1,
            y: area.y + 1,
            width: area.width.saturating_sub(2),
            height: area.height.saturating_sub(2),
        };
        match action {
            MouseAction::Drag if dragging => {
                let Some(focus) = self.lanes_focus.clone() else {
                    return true;
                };
                if let Some((i, row)) = view
                    .lanes
                    .iter()
                    .enumerate()
                    .find(|(_, r)| r.param == focus.param)
                {
                    let curve = curve_area(inner, i);
                    let (at, value) = position(row, curve, x, y);
                    self.lane_drag = Some(LaneDrag { at, value });
                }
                true
            }
            MouseAction::Up if dragging => {
                let drag = self.lane_drag.take();
                if let (Some(drag), Some(focus)) = (drag, self.lanes_focus.clone())
                    && let Some(row) = view.lanes.iter().find(|r| r.param == focus.param)
                {
                    self.lane_edit(
                        row,
                        LaneEdit::Move {
                            index: focus.point,
                            at: drag.at,
                            value: drag.value,
                        },
                    );
                }
                true
            }
            MouseAction::Down | MouseAction::RightDown => {
                let Some((i, row)) = view.lanes.iter().enumerate().find(|(i, _)| {
                    let curve = curve_area(inner, *i);
                    curve.contains(Position::new(x, y))
                }) else {
                    return true;
                };
                let curve = curve_area(inner, i);
                let near = row
                    .lane
                    .points()
                    .iter()
                    .position(|p| row.column(p.at, curve.width).abs_diff(x - curve.x) <= GRAB);
                let track = view.track.clone();
                match (action, near) {
                    (MouseAction::RightDown, Some(point)) => {
                        self.lane_edit(row, LaneEdit::Delete(point));
                    }
                    (MouseAction::RightDown, None) => {}
                    (_, Some(point)) => {
                        // Lo elige y empieza a arrastrarlo.
                        self.lanes_focus = Some(LaneFocus {
                            track,
                            param: row.param.clone(),
                            point,
                        });
                        let p = row.lane.points()[point];
                        self.lane_drag = Some(LaneDrag {
                            at: p.at,
                            value: p.value,
                        });
                        self.message = lanes_help(row);
                    }
                    (_, None) => {
                        let (at, value) = position(row, curve, x, y);
                        self.lane_edit(row, LaneEdit::Add { at, value });
                    }
                }
                true
            }
            _ => area.contains(Position::new(x, y)),
        }
    }

    /// Las teclas con un punto elegido: `true` si eran para él.
    pub(super) fn lanes_key(&mut self, input: &Input) -> Option<Flow> {
        let focus = self.lanes_focus.clone().filter(|_| self.lanes_open)?;
        let view = self.lanes_view()?;
        let Some(row) = view
            .lanes
            .iter()
            .find(|r| r.param == focus.param && view.track == focus.track)
        else {
            self.lanes_focus = None;
            return None;
        };
        let point = focus.point.min(row.lane.points().len() - 1);
        let edit = match input.key {
            Key::Esc => {
                self.lanes_focus = None;
                self.message = "lanes: punto suelto".into();
                return Some(Flow::Continue);
            }
            Key::Delete | Key::Backspace => LaneEdit::Delete(point),
            Key::Char('c') if !input.ctrl && !input.alt => LaneEdit::Cycle(point),
            Key::Left | Key::Right | Key::Up | Key::Down if !input.ctrl && !input.alt => {
                let (steps, levels) = match input.key {
                    Key::Left => (-1, 0),
                    Key::Right => (1, 0),
                    Key::Up => (0, 1),
                    _ => (0, -1),
                };
                LaneEdit::Nudge {
                    index: point,
                    steps,
                    levels,
                }
            }
            Key::Tab => {
                let count = row.lane.points().len();
                let next = (point + 1) % count;
                self.lanes_focus = Some(LaneFocus {
                    point: next,
                    ..focus
                });
                return Some(Flow::Continue);
            }
            _ => return None,
        };
        self.lane_edit(row, edit);
        Some(Flow::Continue)
    }

    /// Aplica `edit` a la lane de `row` y la escribe en el código.
    fn lane_edit(&mut self, row: &LaneRow, edit: LaneEdit) {
        let edited = match lanes_edit::apply(row, edit) {
            Ok(edited) => edited,
            Err(why) => {
                self.message = format!("lanes: {why}");
                return;
            }
        };
        let span = match &row.editable {
            Ok(span) => *span,
            Err(why) => {
                self.message = format!("lanes: no se puede editar: {why}");
                return;
            }
        };
        // La posición es la del texto cargado: si ha cambiado, no vale.
        if self.loaded_text.as_deref() != Some(self.text().as_str()) {
            self.message = match &self.error {
                Some(e) => format!(
                    "lanes: la última carga falló ({}); arréglalo y carga",
                    e.message
                ),
                None => {
                    "lanes: el texto ha cambiado desde la última carga; carga antes (Ctrl-S)".into()
                }
            };
            return;
        }
        let lines: Vec<String> = self.editor.lines().to_vec();
        let Some(original) = span_text(&lines, span) else {
            self.message = "lanes: la tabla de puntos ya no está donde estaba".into();
            return;
        };
        let written = script::points_text(&edited.points, &original);
        let text = replace_span(&lines, span, &written);
        let track = self
            .tracks()
            .get(self.selected)
            .map_or(String::new(), |t| t.name.clone());
        self.completion = None;
        self.marks.clear();
        self.replace_text(&text);
        self.load();
        self.lanes_focus = Some(LaneFocus {
            track,
            param: row.param.clone(),
            point: edited.selected,
        });
        if self.error.is_none() {
            self.message = format!(
                "lane {}: {} · entra en el compás siguiente (Ctrl-Z lo deshace) · {}",
                row.param,
                edited.what,
                lanes_help(row)
            );
        }
    }
}

/// La ayuda con un punto elegido.
fn lanes_help(row: &LaneRow) -> String {
    match &row.editable {
        Ok(_) => "c curva · Supr borra · ←→ paso · ↑↓ valor · Tab siguiente · Esc suelta".into(),
        Err(why) => format!("solo lectura: {why}"),
    }
}

/// Las filas de la curva de la lane `i` del panel (la de su título, encima).
fn curve_area(inner: Rect, i: usize) -> Rect {
    let y = inner.y + i as u16 * (CURVE_ROWS + 1) + 1;
    Rect {
        y,
        height: CURVE_ROWS.min(inner.bottom().saturating_sub(y)),
        ..inner
    }
}

/// La posición (pasos de la lane) y el valor de la celda `x`, `y` de la
/// curva: el centro de la columna y de la fila.
fn position(row: &LaneRow, curve: Rect, x: u16, y: u16) -> (f64, f32) {
    let x = x.clamp(curve.x, curve.right().saturating_sub(1)) - curve.x;
    let y = y.clamp(curve.y, curve.bottom().saturating_sub(1));
    let at = (x as f64 + 0.5) / curve.width.max(1) as f64 * row.lane.length();
    let from_bottom = (curve.bottom() - 1 - y) as f32;
    let level = if curve.height > 1 {
        from_bottom / (curve.height - 1) as f32
    } else {
        0.5
    };
    (at, row.value_at(level))
}

/// La lane de `row` con estos puntos (escritos: en compases y, con
/// `range`, de 0 a 1), para verla mientras se arrastra.
fn preview(row: &LaneRow, points: &[(f64, f32, engine::LaneCurve)]) -> Option<engine::Lane> {
    let real = |v: f32| match row.range {
        None => v,
        Some((lo, hi)) if row.range_log && lo > 0.0 && hi > 0.0 => lo * (hi / lo).powf(v),
        Some((lo, hi)) => lo + v * (hi - lo),
    };
    let points: Vec<engine::LanePoint> = points
        .iter()
        .map(|&(at, value, curve)| engine::LanePoint {
            at: at * row.bar_steps as f64,
            value: real(value),
            curve,
        })
        .collect();
    engine::Lane::new(&points, row.lane.length(), row.lane.is_log())
        .and_then(|l| l.with_timing(row.lane.mode(), row.lane.offset(), row.lane.rest()))
        .ok()
}

/// El texto de `span` en `lines`.
fn span_text(lines: &[String], span: script::TableSpan) -> Option<String> {
    let mut out = String::new();
    for r in span.start.0..=span.end.0 {
        let line: Vec<char> = lines.get(r)?.chars().collect();
        let from = if r == span.start.0 { span.start.1 } else { 0 };
        let to = if r == span.end.0 {
            span.end.1
        } else {
            line.len()
        };
        out.extend(line.get(from..to)?);
        if r != span.end.0 {
            out.push('\n');
        }
    }
    Some(out)
}

/// `lines` con `span` cambiado por `text`, como un solo texto.
fn replace_span(lines: &[String], span: script::TableSpan, text: &str) -> String {
    let mut out = String::new();
    for (r, line) in lines.iter().enumerate() {
        let chars: Vec<char> = line.chars().collect();
        if r < span.start.0 || r > span.end.0 {
            out.push_str(line);
            out.push('\n');
            continue;
        }
        if r == span.start.0 {
            out.extend(&chars[..span.start.1.min(chars.len())]);
            out.push_str(text);
        }
        if r == span.end.0 {
            out.extend(&chars[span.end.1.min(chars.len())..]);
            out.push('\n');
        }
    }
    out
}
