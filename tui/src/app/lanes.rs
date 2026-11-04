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
        let dragging = self.lane_drag.is_some()
            || self
                .touch
                .as_ref()
                .is_some_and(|t| t.takes.iter().any(|k| k.mouse));
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
        // Grabando: el ratón toca el valor en vez de editar los puntos.
        if self.touch_mouse(action, x, y, inner, &view) {
            return true;
        }
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
        match lanes_edit::apply(row, edit) {
            Ok(edited) => self.write_lane(row, &edited),
            Err(why) => self.message = format!("lanes: {why}"),
        }
    }

    /// Escribe los puntos de `edited` en la tabla de la lane de `row`, en un
    /// paso de deshacer, y carga.
    pub(super) fn write_lane(&mut self, row: &LaneRow, edited: &lanes_edit::Edited) {
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

/// La grabación de lanes («touch»), armada: las tomas abiertas.
#[derive(Debug, Default)]
pub(crate) struct Touch {
    takes: Vec<Take>,
}

/// Lo que se ha tocado de un parámetro: (fase de la lane, valor), desde
/// el ratón o desde un mando MIDI, y cuándo se tocó por última vez.
#[derive(Debug)]
struct Take {
    track: String,
    param: String,
    samples: Vec<(f64, f32)>,
    last: std::time::Instant,
    mouse: bool,
}

/// Lo que un mando tiene que estar quieto para cerrar su toma.
const KNOB_IDLE: std::time::Duration = std::time::Duration::from_millis(600);

impl App {
    /// «Grabar lanes (touch)»: la arma o la desarma (y cierra lo que se
    /// estaba grabando).
    pub(super) fn toggle_touch(&mut self) {
        match self.touch.take() {
            Some(touch) => {
                let wrote = !touch.takes.is_empty();
                for take in touch.takes {
                    self.finish_take(take);
                }
                self.message = if wrote {
                    format!("{} · grabar lanes: desarmado", self.message)
                } else {
                    "grabar lanes: desarmado".into()
                };
            }
            None => {
                self.touch = Some(Touch::default());
                self.message = "grabar lanes: armado · con el transporte en marcha, mueve un mando MIDI \
                                asignado a un parámetro con lane, o arrastra en vertical sobre su curva"
                    .into();
            }
        }
    }

    /// La lane `param` de la pista `track`, con lo que publica el motor.
    fn lane_row(&self, track: &str, param: &str) -> Option<LaneRow> {
        let song = self.host.song()?;
        let index = song.tracks.iter().position(|t| t.name == track)?;
        LanesView::new(Some(song), index, self.status.as_ref())?
            .lanes
            .into_iter()
            .find(|r| r.param == param)
    }

    /// Apunta en la toma de `track`/`param` el valor `value` en la fase de
    /// ahora de su lane (con el transporte en marcha y la grabación
    /// armada). `true` si había lane.
    fn touch_record(&mut self, track: &str, param: &str, value: f32, mouse: bool) -> bool {
        if self.touch.is_none() || !self.status.is_some_and(|s| s.running) {
            return false;
        }
        let Some(phase) = self
            .lane_row(track, param)
            .and_then(|r| r.live)
            .and_then(|l| l.phase)
        else {
            return false;
        };
        let Some(touch) = &mut self.touch else {
            return false;
        };
        let now = std::time::Instant::now();
        match touch
            .takes
            .iter_mut()
            .find(|t| t.track == track && t.param == param)
        {
            Some(take) => {
                take.samples.push((phase as f64, value));
                take.last = now;
            }
            None => touch.takes.push(Take {
                track: track.to_string(),
                param: param.to_string(),
                samples: vec![(phase as f64, value)],
                last: now,
                mouse,
            }),
        }
        self.message = format!(
            "grabando {track} · {param}: {}",
            crate::lanes_panel::number(value)
        );
        true
    }

    /// Cierra una toma: la simplifica, la pone en su tramo de la lane y la
    /// escribe en el código.
    fn finish_take(&mut self, take: Take) {
        let Some(row) = self.lane_row(&take.track, &take.param) else {
            return;
        };
        let points: Vec<(f64, f32, engine::LaneCurve)> = row
            .lane
            .points()
            .iter()
            .map(|p| (p.at, p.value, p.curve))
            .collect();
        let merged = crate::touch::merge(
            &points,
            &take.samples,
            row.lane.length(),
            |v| row.level(v),
            |l| row.value_at(l),
            engine::MAX_LANE_POINTS,
        );
        let edited = lanes_edit::Edited {
            points: lanes_edit::written_points(&row, &merged),
            selected: 0,
            what: "toma grabada",
        };
        self.write_lane(&row, &edited);
    }

    /// Cierra las tomas de los mandos que llevan un rato quietos (en cada
    /// `tick`).
    pub(super) fn tick_touch(&mut self) {
        let Some(touch) = &mut self.touch else { return };
        let now = std::time::Instant::now();
        let (done, open): (Vec<Take>, Vec<Take>) = std::mem::take(&mut touch.takes)
            .into_iter()
            .partition(|t| !t.mouse && now.duration_since(t.last) >= KNOB_IDLE);
        touch.takes = open;
        for take in done {
            self.finish_take(take);
        }
    }

    /// Un mando MIDI se ha movido: con la grabación armada, apunta su valor
    /// en la lane de su parámetro, si la tiene.
    pub(super) fn touch_knob(&mut self, channel: u8, cc: u8, value: u8) {
        if self.touch.is_none() {
            return;
        }
        let moved: Vec<(String, String, f32)> = self
            .learned
            .iter()
            .filter(|l| l.listens(channel, cc))
            .map(|l| {
                let map = engine::KnobMap {
                    target: engine::KnobTarget::Bus { param: 0 },
                    min: l.min,
                    max: l.max,
                    curve: l.curve,
                    latch: false,
                };
                (
                    l.track.clone(),
                    l.param.clone(),
                    map.value(value as f32 / 127.0),
                )
            })
            .collect();
        for (track, param, v) in moved {
            self.touch_record(&track, &param, v, false);
        }
    }

    /// El ratón sobre la curva de una lane con la grabación armada y en
    /// marcha: graba el valor de esa altura (en la fase del cursor); al
    /// soltar, cierra la toma. `true` si era para esto.
    fn touch_mouse(
        &mut self,
        action: MouseAction,
        x: u16,
        y: u16,
        inner: Rect,
        view: &LanesView,
    ) -> bool {
        if self.touch.is_none() || !self.status.is_some_and(|s| s.running) {
            return false;
        }
        match action {
            MouseAction::Down | MouseAction::Drag => {
                let hit = view.lanes.iter().enumerate().find(|(i, _)| {
                    let curve = curve_area(inner, *i);
                    let column = Rect {
                        y: curve.y,
                        height: curve.height,
                        ..curve
                    };
                    // Al arrastrar vale salirse un poco por arriba o abajo.
                    action == MouseAction::Drag && x >= column.x && x < column.right()
                        || column.contains(Position::new(x, y))
                });
                let Some((i, row)) = hit else { return true };
                let curve = curve_area(inner, i);
                let (_, value) = position(row, curve, x, y);
                self.touch_record(&view.track, &row.param.clone(), value, true);
                true
            }
            MouseAction::Up => {
                let takes: Vec<Take> = match &mut self.touch {
                    Some(touch) => {
                        let (mouse, rest) = std::mem::take(&mut touch.takes)
                            .into_iter()
                            .partition(|t| t.mouse);
                        touch.takes = rest;
                        mouse
                    }
                    None => Vec::new(),
                };
                for take in takes {
                    self.finish_take(take);
                }
                true
            }
            _ => false,
        }
    }
}

impl App {
    /// Lo que vale ahora cada parámetro con lane de la pista `track` (lo
    /// que publica el motor, en las unidades de Lua), para enseñarlo junto
    /// al parámetro.
    pub(super) fn live_params(&self, track: &script::TrackDef) -> Vec<crate::model::LiveParam> {
        let (Some(song), Some(status)) = (self.host.song(), self.status.as_ref()) else {
            return Vec::new();
        };
        track
            .lanes
            .iter()
            .filter_map(|(name, index)| {
                let def = song.lanes.get(*index)?.as_ref()?;
                let live = status.lanes.get(*index).copied().flatten()?;
                let k = if def.unit_scale != 0.0 {
                    def.unit_scale
                } else {
                    1.0
                };
                let value = live.value / k;
                let (lo, hi, log) = def.scale;
                let level = if hi <= lo {
                    0.5
                } else if log && lo > 0.0 && value > 0.0 {
                    (value.ln() - lo.ln()) / (hi.ln() - lo.ln())
                } else {
                    (value - lo) / (hi - lo)
                };
                Some(crate::model::LiveParam {
                    target: def.target.clone()?,
                    name: name.clone(),
                    value,
                    level: level.clamp(0.0, 1.0),
                })
            })
            .collect()
    }
}
