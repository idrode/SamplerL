//! El ratón en el editor (fase R): un clic lleva el cursor a donde se
//! pulsa (con el desplazamiento, los números de línea y las líneas largas,
//! con `TextArea::screen_to_data`), arrastrar selecciona y la rueda
//! desplaza. Solo en el modo editor y sin nada abierto encima (la paleta,
//! la línea de órdenes...); también en la ventana del editor de `--split`.
//! Con la captura del ratón, la terminal ya no selecciona sola: Shift y
//! arrastrar lo hace (casi todas las terminales).

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

impl App {
    /// El ratón en la columna `column` y la fila `row` de la terminal.
    pub fn mouse(&mut self, action: MouseAction, column: u16, row: u16) {
        if !self.mouse_enabled || self.mode != Mode::Edit || !self.pastes_in_editor() {
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
