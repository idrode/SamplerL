//! El ratón en el editor (fase R): un clic lleva el cursor a donde se
//! pulsa (con el desplazamiento, los números de línea y las líneas largas,
//! con `TextArea::screen_to_data`), arrastrar selecciona y la rueda
//! desplaza. Solo en el modo editor y sin nada abierto encima (la paleta,
//! la línea de órdenes...); también en la ventana del editor de `--split`.
//! Con la captura del ratón, la terminal ya no selecciona sola: Shift y
//! arrastrar lo hace (casi todas las terminales).
//!
//! En la vista del sample (en los dos modos y en la ventana principal sin
//! editor), pulsar cerca de la marca de un corte la agarra y arrastrarla
//! cambia dónde empieza (y dónde acaba el anterior): mientras se arrastra
//! solo cambia la vista; al soltar, se pone como con `,`/`.`
//! ([`App::place_slice`]: al proyecto y recarga, sin sonar). Pulsar en
//! otro sitio de la onda selecciona el corte que hay ahí y lo preescucha,
//! como agarrar una marca y soltarla sin moverla. El clic derecho
//! pone un corte nuevo donde se pulsa, o quita el de la marca pulsada; la
//! rueda acerca y aleja la vista alrededor del ratón (ver `zoom.rs`) y la
//! rueda horizontal (o Shift y la rueda) la desplaza.

use ratatui_textarea::{CursorMove, Scrolling};

use super::slices::{self, Edge};
use super::{App, Mode};
use crate::ui::{WaveHit, WaveLayout};

/// Lo que hace el ratón (sin depender de crossterm: así se prueba).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MouseAction {
    /// Botón izquierdo pulsado.
    Down,
    /// Movido con el botón izquierdo pulsado.
    Drag,
    /// Botón izquierdo soltado.
    Up,
    /// Botón derecho pulsado.
    RightDown,
    ScrollUp,
    ScrollDown,
    ScrollLeft,
    ScrollRight,
    /// Ctrl y la rueda (el zoom del piano roll).
    ZoomIn,
    ZoomOut,
}

/// Líneas que mueve cada paso de la rueda.
const WHEEL_LINES: i16 = 3;

/// Lo que se deja como mínimo de largo a un corte al arrastrar, en ms:
/// algo más que el mínimo de 1 ms del script, para que el redondeo a
/// 0,1 ms no lo pise.
pub(super) const DRAG_GAP_MS: f64 = 1.2;

/// Un corte agarrado con el ratón (en frames del sample que suena,
/// estirado si la pista tiene stretch).
#[derive(Debug, Clone)]
pub(crate) struct SliceDrag {
    /// La pista (por nombre: si cambia la seleccionada, no se aplica).
    pub(crate) track: String,
    pub(crate) index: usize,
    /// Qué se arrastra: un borde, el corte entero o la marca compartida.
    edge: Edge,
    /// Columna de la terminal donde se pulsó.
    column: u16,
    /// Los cortes de la pista al agarrar y donde los deja el arrastre (lo
    /// que enseña la vista hasta soltar).
    start: Vec<std::ops::Range<usize>>,
    pub(crate) preview: Vec<std::ops::Range<usize>>,
    /// Se ha movido (si no, al soltar es un clic: suena).
    moved: bool,
    gap: usize,
    total: usize,
}
impl App {
    /// El ratón en la columna `column` y la fila `row` de la terminal, sin
    /// Alt ni Ctrl.
    #[cfg(test)]
    pub fn mouse(&mut self, action: MouseAction, column: u16, row: u16) {
        self.mouse_with(action, column, row, false);
    }

    /// Como [`App::mouse`]; `held`: con Alt o Ctrl pulsado (en la onda,
    /// agarrar la marca de un corte la mueve con los vecinos que la
    /// comparten). Shift no vale: la terminal lo usa para seleccionar.
    pub fn mouse_with(&mut self, action: MouseAction, column: u16, row: u16, held: bool) {
        if !self.mouse_enabled {
            return;
        }
        if action == MouseAction::Down {
            self.mouse_held = held;
        }
        // Un clic nuevo suelta el arrastre que se quedó a medias (soltado
        // fuera de la terminal, sin `Up`).
        if action == MouseAction::Down {
            self.slice_drag = None;
        }
        // La barra de menús y el menú abierto, antes que nada.
        if self.menu_mouse(action, column, row) {
            return;
        }
        // Los bordes que se arrastran (el del editor) y el explorador de
        // archivos (y el arrastre de su borde).
        if self.nothing_on_top()
            && (self.dock_mouse(action, column, row) || self.files_mouse(action, column, row))
        {
            return;
        }
        // Sobre el editor flotante, el ratón es solo suyo: no llega a lo
        // que tapa (el primer clic, sin el foco, solo se lo da).
        let on_float = self.grid.as_ref().is_some_and(|g| {
            g.floating
                && g.editor
                    .is_some_and(|r| r.contains(ratatui::layout::Position::new(column, row)))
        });
        if on_float
            && action == MouseAction::Down
            && self.nothing_on_top()
            && self.focus() != crate::focus::Panel::Code
        {
            self.set_focus(crate::focus::Panel::Code);
            return;
        }
        // (Arrastrar y soltar siguen llegando a lo que se arrastraba.)
        if !on_float || matches!(action, MouseAction::Drag | MouseAction::Up) {
            // Las pestañas del dock, y el foco al panel que se pulsa.
            if action == MouseAction::Down && self.nothing_on_top() && self.focus_mouse(column, row)
            {
                return;
            }
            // El piano roll, si se ve aquí y el ratón está encima (o
            // arrastra).
            if let Some(area) = self.roll.area.filter(|_| self.local_roll_view_shown())
                && self.roll_mouse(action, column, row, area)
            {
                return;
            }
            // El panel de las lanes, si se ve y el ratón está encima (o
            // arrastra un punto).
            if self.lanes_mouse(action, column, row) {
                return;
            }
            // El tracker, la vista de ocupación y el mezclador.
            if self.slice_drag.is_none() && self.panels_mouse(action, column, row) {
                return;
            }
            if self.slice_drag.is_some() || self.on_sample(action, column, row) {
                self.mouse_sample(action, column);
                return;
            }
        }
        if self.mode != Mode::Edit || !self.pastes_in_editor() {
            self.drag_from = None;
            return;
        }
        let Some(area) = self.editor_area else {
            return;
        };
        match action {
            MouseAction::RightDown
            | MouseAction::ScrollLeft
            | MouseAction::ScrollRight
            | MouseAction::ZoomIn
            | MouseAction::ZoomOut => {}
            MouseAction::ScrollUp | MouseAction::ScrollDown => {
                let inside = area.contains(ratatui::layout::Position::new(column, row));
                if inside {
                    let rows = if action == MouseAction::ScrollUp {
                        -WHEEL_LINES
                    } else {
                        WHEEL_LINES
                    };
                    self.editor.scroll(Scrolling::Delta { rows, cols: 0 });
                }
            }
            MouseAction::Down => {
                if !area.contains(ratatui::layout::Position::new(column, row)) {
                    return;
                }
                // Como al moverse con las teclas: se cierra el autocompletado
                // y se olvidan los campos de la plantilla.
                self.completion = None;
                self.marks.clear();
                self.mark_selected = false;
                self.quit_armed = false;
                let at = self.text_at(column, row);
                self.editor.cancel_selection();
                self.jump(at);
                self.drag_from = Some(at);
            }
            MouseAction::Drag => {
                let Some(from) = self.drag_from else { return };
                // Fuera del texto, arriba o abajo: desplaza y sigue.
                if row < area.y {
                    self.editor.scroll(Scrolling::Delta { rows: -1, cols: 0 });
                } else if row >= area.bottom() {
                    self.editor.scroll(Scrolling::Delta { rows: 1, cols: 0 });
                }
                let at = self.text_at(column, row);
                if self.editor.selection_range().is_none() {
                    if at == from {
                        return;
                    }
                    self.jump(from);
                    self.editor.start_selection();
                }
                self.jump(at);
            }
            MouseAction::Up => self.drag_from = None,
        }
    }

    /// La fila y la columna del texto que se ven en esa celda de la
    /// terminal (fuera del texto, lo más cerca: la columna 0 sobre los
    /// números de línea, el final de la línea más allá).
    fn text_at(&self, column: u16, row: u16) -> (usize, usize) {
        let Some(area) = self.editor_area else {
            return (0, 0);
        };
        let (top, left) = self.editor.scroll_offset();
        let text_x = area.x + self.editor.line_number_width();
        let row = row.clamp(area.y, area.bottom().saturating_sub(1)) - area.y;
        let col = column.saturating_sub(text_x);
        let at = self
            .editor
            .screen_to_data(top as usize + row as usize, left as usize + col as usize);
        (at.0, at.1)
    }

    fn jump(&mut self, (row, col): (usize, usize)) {
        let clamp = |n: usize| n.min(u16::MAX as usize) as u16;
        self.editor
            .move_cursor(CursorMove::Jump(clamp(row), clamp(col)));
    }
}

impl App {
    /// La geometría de la onda tal como se dibujó la última vez.
    #[cfg(test)]
    pub(crate) fn wave_layout(&self) -> Option<WaveLayout> {
        self.wave_layout.clone()
    }

    /// Pulsa (o mueve la rueda) en la forma de onda de una pista con
    /// sample, sin nada encima.
    fn on_sample(&self, action: MouseAction, column: u16, row: u16) -> bool {
        !matches!(action, MouseAction::Drag | MouseAction::Up)
            && self.nothing_on_top()
            && self
                .wave_layout
                .as_ref()
                .is_some_and(|l| l.contains(column, row))
            && self.tracks().get(self.selected).is_some_and(|t| {
                t.instrument.and_then(|i| i.synth).is_none() && !t.slices.is_empty()
            })
    }

    /// Agarra, arrastra o suelta un corte en la vista del sample (un borde,
    /// el cuerpo entero o, con Alt o Ctrl, la marca compartida); con el
    /// botón derecho, pone o quita un corte; con la rueda, zoom.
    fn mouse_sample(&mut self, action: MouseAction, column: u16) {
        let Some(layout) = self.wave_layout.clone() else {
            return;
        };
        match action {
            MouseAction::Down => {
                // Con zoom, la vista se queda donde está aunque cambie el
                // corte seleccionado.
                self.pin_wave_view();
                self.grab_slice(&layout, column);
            }
            MouseAction::RightDown => {
                self.pin_wave_view();
                self.right_click_sample(&layout, column);
            }
            MouseAction::ScrollUp | MouseAction::ScrollDown => {
                let x = column.saturating_sub(layout.area.x) as f64 + 0.5;
                let anchor = x / layout.area.width.max(1) as f64;
                self.zoom_wave(action == MouseAction::ScrollUp, Some(anchor));
            }
            MouseAction::ScrollLeft | MouseAction::ScrollRight => {
                self.pan_wave(action == MouseAction::ScrollRight);
            }
            MouseAction::ZoomIn | MouseAction::ZoomOut => {}
            MouseAction::Drag => {
                let per_column = layout.frames_per_column();
                let Some(drag) = self.slice_drag.as_mut() else {
                    return;
                };
                // Lo que se mueve el ratón, en frames (sin saltar al pulsar
                // un poco al lado del borde).
                let delta = ((column as f64 - drag.column as f64) * per_column).round() as i64;
                let from = drag.edge.of(&drag.start[drag.index]) as i64;
                let (preview, limit) = slices::moved(
                    &drag.start,
                    drag.index,
                    drag.edge,
                    from + delta,
                    drag.total,
                    drag.gap,
                );
                drag.moved |= preview != drag.start;
                drag.preview = preview;
                let (index, edge, r) = (drag.index, drag.edge, drag.preview[drag.index].clone());
                let (a, b) = (self.frame_ms(r.start), self.frame_ms(r.end));
                let what = match edge {
                    Edge::End => format!("acaba en {b:.1} ms"),
                    Edge::Whole => format!("de {a:.1} a {b:.1} ms"),
                    Edge::Start | Edge::Shared => format!("empieza en {a:.1} ms"),
                };
                let limit = match limit {
                    None => String::new(),
                    Some(slices::Limit::SampleStart) => " (tope: el principio del sample)".into(),
                    Some(slices::Limit::SampleEnd) => " (tope: el final del sample)".into(),
                    Some(slices::Limit::Neighbour(j)) => format!(" (tope: el corte {})", j + 1),
                    Some(slices::Limit::Own) if edge == Edge::End => " (tope: su inicio)".into(),
                    Some(slices::Limit::Own) => " (tope: su final)".into(),
                };
                self.message = format!("corte {}: {what}{limit} (suelta para fijarlo)", index + 1);
            }
            MouseAction::Up => {
                let Some(drag) = self.slice_drag.take() else {
                    return;
                };
                let same = self
                    .tracks()
                    .get(self.selected)
                    .is_some_and(|t| t.name == drag.track);
                if !same || !drag.moved {
                    // Un clic, sin arrastrar: suena.
                    if same {
                        self.preview_selected();
                    }
                    return;
                }
                let ms = self.frame_ms(drag.edge.of(&drag.preview[drag.index]));
                self.move_slice_edge(drag.index, drag.edge, slices::Target::At(ms));
            }
        }
    }

    /// Clic derecho en la onda: en un borde, quita ese corte; si no, pone
    /// uno nuevo donde se pulsa.
    fn right_click_sample(&mut self, layout: &WaveLayout, column: u16) {
        let Some(track) = self.tracks().get(self.selected) else {
            return;
        };
        let ranges: Vec<_> = track.slices.iter().collect();
        match layout.hit(column, &ranges, None) {
            WaveHit::Edge { slice, end: false } => self.delete_slice(slice),
            _ => self.split_at(layout.frame_at(column).round() as usize),
        }
    }

    /// Clic en la onda: en un borde de un corte, lo agarra (con Alt o
    /// Ctrl, la marca compartida); dentro de un corte, lo elige y lo agarra
    /// entero; en un hueco, nada.
    fn grab_slice(&mut self, layout: &WaveLayout, column: u16) {
        let Some(track) = self.tracks().get(self.selected).cloned() else {
            return;
        };
        let selected = Some(
            self.selected_slice
                .min(track.slices.len().saturating_sub(1)),
        );
        let ranges: Vec<_> = track.slices.iter().collect();
        let (index, edge) = match layout.hit(column, &ranges, selected) {
            WaveHit::Gap => {
                self.message = "aquí no hay ningún corte (un hueco): clic derecho pone uno".into();
                return;
            }
            WaveHit::Body { slice } => (slice, Edge::Whole),
            WaveHit::Edge { slice, end } => {
                let r = &ranges[slice];
                match (end, self.mouse_held) {
                    (false, false) => (slice, Edge::Start),
                    (true, false) => (slice, Edge::End),
                    (false, true) => (slice, Edge::Shared),
                    // El final con Alt: la marca del que empieza ahí, si hay.
                    (true, true) => match ranges.iter().position(|o| o.start == r.end) {
                        Some(next) => (next, Edge::Shared),
                        None => (slice, Edge::End),
                    },
                }
            }
        };
        self.selected_slice = index;
        if track.origin.indices.get(index).copied().flatten().is_none() {
            self.message = format!(
                "el corte {} de {} lo añade el script con :add(): muévelo en el script",
                index + 1,
                track.name
            );
            return;
        }
        let rate = track.sample.sample_rate() as f64;
        let ratio = track.stretch.map_or(1.0, |s| s.ratio);
        let gap = (DRAG_GAP_MS / 1000.0 * rate * ratio).ceil() as usize;
        self.slice_drag = Some(SliceDrag {
            track: track.name.clone(),
            index,
            edge,
            column,
            start: ranges.clone(),
            preview: ranges,
            moved: false,
            gap,
            total: track.sample.frames(),
        });
        self.message = match edge {
            Edge::Whole => format!("corte {} agarrado: arrastra para moverlo entero", index + 1),
            edge => format!(
                "corte {} agarrado por {}: arrastra para moverlo",
                index + 1,
                edge.name()
            ),
        };
    }

    /// El mensaje de corte seleccionado (con su etiqueta, si la tiene).
    pub(super) fn select_slice_message(&mut self, index: usize) {
        let Some(track) = self.tracks().get(self.selected) else {
            return;
        };
        let label = track
            .labels
            .get(index)
            .cloned()
            .flatten()
            .map_or(String::new(), |l| format!(" ({l})"));
        self.message = format!("{}: corte {}{label} seleccionado", track.name, index + 1);
    }
}
