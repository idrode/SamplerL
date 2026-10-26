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
//! ([`App::place_slice`]: al proyecto, recarga y preescucha). Pulsar en
//! otro sitio de la onda selecciona el corte que hay ahí.

use ratatui_textarea::{CursorMove, Scrolling};

use super::{App, Mode};

/// Lo que hace el ratón (sin depender de crossterm: así se prueba).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MouseAction {
    /// Botón izquierdo pulsado.
    Down,
    /// Movido con el botón izquierdo pulsado.
    Drag,
    /// Botón izquierdo soltado.
    Up,
    ScrollUp,
    ScrollDown,
}

/// Líneas que mueve cada paso de la rueda.
const WHEEL_LINES: i16 = 3;

/// Columnas a cada lado de una marca en las que un clic la agarra.
const GRAB_COLUMNS: f64 = 1.0;

/// Lo que se deja como mínimo entre un corte y sus vecinos al arrastrar,
/// en ms: algo más que el margen de 1 ms del script, para que el
/// redondeo a 0,1 ms no lo pise.
const DRAG_GAP_MS: f64 = 1.2;

/// La marca de un corte que se arrastra (en frames del sample que suena,
/// estirado si la pista tiene stretch).
#[derive(Debug, Clone)]
pub(crate) struct SliceDrag {
    /// La pista (por nombre: si cambia la seleccionada, no se aplica).
    pub(crate) track: String,
    pub(crate) index: usize,
    /// Columna de la terminal donde se pulsó.
    column: u16,
    /// Dónde empezaba el corte y dónde lo deja el arrastre.
    start: usize,
    pub(crate) at: usize,
    /// Hasta dónde puede llegar (sus vecinos con el margen).
    low: usize,
    high: usize,
}

impl App {
    /// El ratón en la columna `column` y la fila `row` de la terminal.
    pub fn mouse(&mut self, action: MouseAction, column: u16, row: u16) {
        if !self.mouse_enabled {
            return;
        }
        // Un clic nuevo suelta el arrastre que se quedó a medias (soltado
        // fuera de la terminal, sin `Up`).
        if action == MouseAction::Down {
            self.slice_drag = None;
        }
        if self.slice_drag.is_some() || self.on_sample(action, column, row) {
            self.mouse_sample(action, column);
            return;
        }
        if self.mode != Mode::Edit || !self.pastes_in_editor() {
            self.drag_from = None;
            return;
        }
        let Some(area) = self.editor_area else {
            return;
        };
        match action {
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
    /// Pulsa en la forma de onda de una pista con sample, sin nada encima.
    fn on_sample(&self, action: MouseAction, column: u16, row: u16) -> bool {
        action == MouseAction::Down
            && self.nothing_on_top()
            && self
                .sample_area
                .is_some_and(|a| a.contains(ratatui::layout::Position::new(column, row)))
            && self.tracks().get(self.selected).is_some_and(|t| {
                t.instrument.and_then(|i| i.synth).is_none() && !t.slices.is_empty()
            })
    }

    /// Agarra, arrastra o suelta la marca de un corte en la vista del sample.
    fn mouse_sample(&mut self, action: MouseAction, column: u16) {
        match action {
            MouseAction::Down => self.grab_slice(column),
            MouseAction::Drag => {
                let Some(area) = self.sample_area else { return };
                let frames = self
                    .tracks()
                    .get(self.selected)
                    .map_or(0, |t| t.sample.frames());
                let Some(drag) = self.slice_drag.as_mut() else {
                    return;
                };
                // Lo que se mueve el ratón, en frames (sin saltar al pulsar
                // un poco al lado de la marca).
                let per_column = frames as f64 / area.width.max(1) as f64;
                let moved = (column as f64 - drag.column as f64) * per_column;
                let at = (drag.start as f64 + moved).round().max(0.0) as usize;
                drag.at = at.clamp(drag.low, drag.high);
                let (index, at) = (drag.index, drag.at);
                let ms = self.frame_ms(at);
                self.message = format!(
                    "corte {}: arrastrando a {ms:.1} ms (suelta para fijarlo)",
                    index + 1
                );
            }
            MouseAction::Up => {
                let Some(drag) = self.slice_drag.take() else {
                    return;
                };
                let same = self
                    .tracks()
                    .get(self.selected)
                    .is_some_and(|t| t.name == drag.track);
                if !same || drag.at == drag.start {
                    if same {
                        self.select_slice_message(drag.index);
                    }
                    return;
                }
                let ms = self.frame_ms(drag.at);
                self.place_slice(drag.index, Some(ms));
            }
            MouseAction::ScrollUp | MouseAction::ScrollDown => {}
        }
    }

    /// Clic en la onda: cerca de una marca, la agarra; si no, selecciona
    /// el corte que hay donde se pulsa.
    fn grab_slice(&mut self, column: u16) {
        let Some(area) = self.sample_area else { return };
        let Some(track) = self.tracks().get(self.selected).cloned() else {
            return;
        };
        let frames = track.sample.frames().max(1) as f64;
        let width = area.width.max(1) as f64;
        // Centro de la columna pulsada, en columnas desde el borde.
        let x = column.saturating_sub(area.x) as f64 + 0.5;
        let nearest = track
            .slices
            .iter()
            .enumerate()
            .map(|(i, r)| (i, (r.start as f64 / frames * width - x).abs()))
            .min_by(|a, b| a.1.total_cmp(&b.1));
        let Some((index, distance)) = nearest else {
            return;
        };
        self.selected_slice = index;
        if distance > GRAB_COLUMNS + 0.5 {
            // Lejos de las marcas: el corte de ahí.
            let at = (x / width * frames) as usize;
            let index = track
                .slices
                .iter()
                .take_while(|r| r.start <= at)
                .count()
                .saturating_sub(1);
            self.selected_slice = index;
            self.select_slice_message(index);
            return;
        }
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
        let Some(range) = track.slices.get(index) else {
            return;
        };
        let low = match index {
            0 => 0,
            i => track.slices.get(i - 1).map_or(0, |r| r.start + gap),
        };
        // Sin sitio para moverse, se queda donde está.
        let (low, high) = (
            low.min(range.start),
            range.end.saturating_sub(gap).max(range.start),
        );
        self.slice_drag = Some(SliceDrag {
            track: track.name.clone(),
            index,
            column,
            start: range.start,
            at: range.start,
            low,
            high,
        });
        self.message = format!("corte {} agarrado: arrastra para moverlo", index + 1);
    }

    /// El mensaje de corte seleccionado (con su etiqueta, si la tiene).
    fn select_slice_message(&mut self, index: usize) {
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
