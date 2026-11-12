//! Desplazar el editor sin mover el cursor: la rueda (vertical y
//! horizontal, o Shift y la rueda) mueve una vista libre, y la siguiente
//! tecla que mueve el cursor o cambia el texto la suelta y vuelve a enseñar
//! el cursor con `[editor] margen` líneas por encima y por debajo. Las
//! barras finas (`[editor] barras`) dicen dónde está la vista y se
//! arrastran.
//!
//! El widget (`ratatui-textarea`) no tiene una vista aparte del cursor: su
//! `scroll()` arrastra el cursor dentro de la vista y, al dibujarse, se
//! desplaza hasta el cursor. Así que, con la vista libre, se dibuja con el
//! cursor puesto un momento dentro de ella (sin pintarlo: sin estilo de
//! cursor ni de su línea) y se devuelve a su sitio justo después. Lo
//! seleccionado se conserva (la selección empieza donde empezaba) y se
//! pinta encima, con su estilo, donde de verdad está.
//!
//! Los gestos del touchpad llegan como ráfagas de pasos de rueda muy
//! seguidos: dentro de una ráfaga cada paso mueve un tercio de lo que mueve
//! uno suelto (una línea, con el valor por defecto), y así se desliza en
//! vez de saltar.

use std::time::{Duration, Instant};

use ratatui::Frame;
use ratatui::layout::{Position, Rect};
use ratatui::style::Style;
use ratatui_textarea::{CursorMove, Scrolling};

use super::App;

/// Dos pasos de la rueda más juntos que esto son una ráfaga.
const BURST: Duration = Duration::from_millis(40);

/// La vista del editor: libre o siguiendo al cursor.
#[derive(Debug, Default)]
pub(crate) struct EditorView {
    /// La vista desplazada con la rueda o las barras: la primera fila y la
    /// primera columna (con los números de línea, como el widget). `None`:
    /// sigue al cursor.
    pub(crate) free: Option<(usize, usize)>,
    /// Tras una tecla que movió el cursor o cambió el texto: que se vea con
    /// su margen en el próximo dibujo.
    pub(crate) follow: bool,
    /// El último paso de la rueda (para las ráfagas).
    last_wheel: Option<Instant>,
    /// Arrastrando una barra.
    dragging: Option<Bar>,
    /// Dónde se dibujaron las barras la última vez (vertical y horizontal).
    pub(crate) bars: (Option<Rect>, Option<Rect>),
}

/// Una de las dos barras.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Bar {
    Vertical,
    Horizontal,
}

/// Lo que mira una tecla para saber si movió el cursor o cambió el texto:
/// el cursor, cuántas líneas hay y lo larga que es la del cursor (borrar
/// hacia delante no mueve el cursor pero acorta la línea).
pub(crate) type Fingerprint = ((usize, usize), usize, usize);

impl App {
    pub(crate) fn editor_fingerprint(&self) -> Fingerprint {
        let cursor = self.editor.cursor();
        let cursor = (cursor.0, cursor.1);
        let lines = self.editor.lines();
        let len = lines.get(cursor.0).map_or(0, String::len);
        (cursor, lines.len(), len)
    }

    /// Tras una tecla o un pegado: si movió el cursor o cambió el texto, la
    /// vista vuelve al cursor (con su margen).
    pub(crate) fn after_input(&mut self, before: Fingerprint) {
        if self.editor_fingerprint() != before {
            self.view.free = None;
            self.view.follow = true;
        }
    }

    /// Un paso de la rueda: cuánto mueve, de `base` (suelto, `base`; en una
    /// ráfaga, un tercio).
    pub(crate) fn wheel_step(&mut self, base: u16, now: Instant) -> usize {
        let burst = self
            .view
            .last_wheel
            .is_some_and(|t| now.saturating_duration_since(t) < BURST);
        self.view.last_wheel = Some(now);
        if burst {
            (base as usize / 3).max(1)
        } else {
            base as usize
        }
    }

    /// La rueda sobre el editor: un paso hacia `rows` o `cols` (-1, 0 o 1).
    pub(crate) fn wheel_editor(&mut self, rows: isize, cols: isize, now: Instant) {
        let base = if rows != 0 {
            self.editor_config.wheel_lines
        } else {
            self.editor_config.wheel_columns
        };
        let step = self.wheel_step(base, now) as isize;
        self.scroll_view(rows * step, cols * step);
    }

    /// Desplaza la vista libre (sin tocar el cursor), dentro del texto.
    pub(crate) fn scroll_view(&mut self, rows: isize, cols: isize) {
        let (top, left) = self.view.free.unwrap_or_else(|| {
            let (t, l) = self.editor.scroll_offset();
            (t as usize, l as usize)
        });
        let top = top.saturating_add_signed(rows);
        let left = left.saturating_add_signed(cols);
        self.view.free = Some(self.clamp_view(top, left));
    }

    /// La vista más abajo y más a la derecha que tiene sentido: la última
    /// línea abajo del todo y el final de la línea más larga a la vista en
    /// el borde derecho.
    fn clamp_view(&self, top: usize, left: usize) -> (usize, usize) {
        let Some(area) = self.editor_area else {
            return (0, 0);
        };
        let lines = self.editor.lines();
        let height = area.height.max(1) as usize;
        let top = top.min(lines.len().saturating_sub(height));
        let numbers = self.editor.line_number_width() as usize;
        let longest = lines
            .iter()
            .skip(top)
            .take(height)
            .map(|l| display_width(l))
            .max()
            .unwrap_or(0);
        let max_left = (numbers + longest + 1).saturating_sub(area.width as usize);
        (top, left.min(max_left))
    }

    /// Dibuja el texto del editor en `area` (con la vista libre si la hay, o
    /// con el margen alrededor del cursor) y sus barras si están
    /// encendidas. Devuelve dónde quedó el texto.
    pub(crate) fn render_editor(&mut self, frame: &mut Frame, area: Rect) -> Rect {
        let (area, vertical, horizontal) = self.bar_areas(area);
        self.editor_area = Some(area);
        if let Some((top, left)) = self.view.free {
            let (top, left) = self.clamp_view(top, left);
            self.render_free(frame, area, top, left);
            let (t, l) = self.editor.scroll_offset();
            self.view.free = Some((t as usize, l as usize));
        } else {
            if std::mem::take(&mut self.view.follow) {
                self.keep_margin(area);
            }
            frame.render_widget(&self.editor, area);
        }
        self.view.bars = (vertical, horizontal);
        self.draw_bars(frame, area);
        area
    }

    /// Antes de dibujar tras una tecla: la vista se mueve lo justo para que
    /// el cursor tenga `margen` líneas por encima y por debajo.
    fn keep_margin(&mut self, area: Rect) {
        let height = area.height as usize;
        if height == 0 {
            return;
        }
        let margin = (self.editor_config.margin as usize).min((height - 1) / 2);
        let cursor = self.editor.cursor();
        let cursor = (cursor.0, cursor.1);
        let row = cursor.0;
        let top = self.editor.scroll_offset().0 as usize;
        let want = if row < top + margin {
            row.saturating_sub(margin)
        } else if row + margin >= top + height {
            row + margin + 1 - height
        } else {
            top
        };
        if want != top {
            self.move_viewport(want as isize - top as isize, 0, cursor);
        }
    }

    /// Mueve el viewport del widget sin mover el cursor (su `scroll` lo
    /// arrastra: se devuelve a `cursor`).
    fn move_viewport(&mut self, rows: isize, cols: isize, cursor: (usize, usize)) {
        let clamp = |n: isize| n.clamp(i16::MIN as isize, i16::MAX as isize) as i16;
        self.editor.scroll(Scrolling::Delta {
            rows: clamp(rows),
            cols: clamp(cols),
        });
        jump(&mut self.editor, cursor);
    }

    /// Dibuja con la vista en (`top`, `left`) aunque el cursor no se vea.
    fn render_free(&mut self, frame: &mut Frame, area: Rect, top: usize, left: usize) {
        let (vt, vl) = self.editor.scroll_offset();
        let real = self.editor.cursor();
        let real = (real.0, real.1);
        self.move_viewport(
            top as isize - vt as isize,
            left as isize - vl as isize,
            real,
        );
        let height = area.height as usize;
        let width = area.width as usize;
        let numbers = self.editor.line_number_width() as usize;
        let shown = |col: usize| (left..left + width).contains(&widget_column(col, numbers));
        let screen = self.editor.screen_cursor();
        if (top..top + height).contains(&real.0) && shown(screen.col) {
            // El cursor está a la vista: se dibuja como siempre.
            frame.render_widget(&self.editor, area);
            return;
        }
        // Un sitio a la vista donde poner el cursor un momento: la primera
        // línea que llega a la columna `left`.
        let lines = self.editor.lines();
        let spot = (top..(top + height).min(lines.len())).find_map(|row| {
            let len = lines[row].chars().count();
            (0..=len).find(|&c| shown(c)).map(|c| (row, c))
        });
        let selecting = self.editor.selection_range();
        let styles = (
            self.editor.cursor_style(),
            self.editor.cursor_line_style(),
            self.editor.selection_style(),
        );
        self.editor.set_cursor_style(self.editor.style());
        self.editor.set_cursor_line_style(Style::default());
        if selecting.is_some() {
            self.editor.set_selection_style(Style::default());
        }
        if let Some(spot) = spot {
            jump(&mut self.editor, spot);
        }
        frame.render_widget(&self.editor, area);
        jump(&mut self.editor, real);
        self.editor.set_cursor_style(styles.0);
        self.editor.set_cursor_line_style(styles.1);
        self.editor.set_selection_style(styles.2);
        // Lo seleccionado, donde está de verdad.
        if let Some(((r0, c0), (r1, c1))) = selecting {
            let text_x = area.x as usize + numbers;
            for row in r0.max(top)..=r1.min(top + height - 1) {
                let len = self.editor.lines()[row].chars().count();
                let from = if row == r0 { c0 } else { 0 };
                let to = if row == r1 { c1 } else { len + 1 };
                let skip = left.saturating_sub(numbers);
                let from = (text_x + from).saturating_sub(skip).max(area.x as usize);
                let to = (text_x + to)
                    .saturating_sub(skip)
                    .min(area.right() as usize);
                if to > from {
                    let y = area.y + (row - top) as u16;
                    let cells = Rect::new(from as u16, y, (to - from) as u16, 1);
                    frame.buffer_mut().set_style(cells, styles.2);
                }
            }
        }
    }

    /// Las barras se comen la última columna y, si hay líneas más anchas
    /// que el editor, la última fila.
    fn bar_areas(&self, area: Rect) -> (Rect, Option<Rect>, Option<Rect>) {
        if !self.editor_config.bars || area.width < 8 || area.height < 3 {
            return (area, None, None);
        }
        let vertical = Rect::new(area.right() - 1, area.y, 1, area.height);
        let text = Rect {
            width: area.width - 1,
            ..area
        };
        let numbers = self.editor.line_number_width() as usize;
        let longest = self
            .editor
            .lines()
            .iter()
            .map(|l| display_width(l))
            .max()
            .unwrap_or(0);
        if numbers + longest < text.width as usize {
            return (text, Some(vertical), None);
        }
        let horizontal = Rect::new(area.x, area.bottom() - 1, text.width, 1);
        let text = Rect {
            height: area.height - 1,
            ..text
        };
        let vertical = Rect {
            height: text.height,
            ..vertical
        };
        (text, Some(vertical), Some(horizontal))
    }

    /// Las barras: la guía tenue y, encima, lo que se ve.
    fn draw_bars(&self, frame: &mut Frame, area: Rect) {
        let (vertical, horizontal) = self.view.bars;
        if vertical.is_none() && horizontal.is_none() {
            return;
        }
        let ascii = crate::theme::ascii();
        let (top, left) = self.editor.scroll_offset();
        let lines = self.editor.lines();
        let track = Style::new().fg(crate::ui::dim());
        let thumb = Style::new().fg(crate::ui::accent());
        let buf = frame.buffer_mut();
        if let Some(bar) = vertical {
            let (from, len) = thumb_span(
                top as usize,
                area.height as usize,
                lines.len().max(1),
                bar.height,
            );
            for i in 0..bar.height {
                let on = (from..from + len).contains(&i);
                let (symbol, style) = match (on, ascii) {
                    (true, false) => ("┃", thumb),
                    (true, true) => ("#", thumb),
                    (false, false) => ("│", track),
                    (false, true) => ("|", track),
                };
                buf.set_string(bar.x, bar.y + i, symbol, style);
            }
        }
        if let Some(bar) = horizontal {
            let numbers = self.editor.line_number_width() as usize;
            let widest = numbers + lines.iter().map(|l| display_width(l)).max().unwrap_or(0) + 1;
            let (from, len) = thumb_span(left as usize, area.width as usize, widest, bar.width);
            for i in 0..bar.width {
                let on = (from..from + len).contains(&i);
                let (symbol, style) = match (on, ascii) {
                    (true, false) => ("━", thumb),
                    (true, true) => ("=", thumb),
                    (false, false) => ("─", track),
                    (false, true) => ("-", track),
                };
                buf.set_string(bar.x + i, bar.y, symbol, style);
            }
        }
    }

    /// El ratón en las barras: pulsar salta ahí y arrastrar la lleva.
    /// `true` si era suyo.
    pub(crate) fn bars_mouse(&mut self, action: super::MouseAction, column: u16, row: u16) -> bool {
        use super::MouseAction;
        let at = Position::new(column, row);
        let hit = match self.view.bars {
            (Some(r), _) if r.contains(at) => Some(Bar::Vertical),
            (_, Some(r)) if r.contains(at) => Some(Bar::Horizontal),
            _ => None,
        };
        match action {
            MouseAction::Down => {
                let Some(bar) = hit else { return false };
                self.view.dragging = Some(bar);
                self.drag_bar(bar, column, row);
                true
            }
            MouseAction::Drag => match self.view.dragging {
                Some(bar) => {
                    self.drag_bar(bar, column, row);
                    true
                }
                None => false,
            },
            MouseAction::Up => self.view.dragging.take().is_some(),
            _ => false,
        }
    }

    /// Lleva la vista a lo que corresponde a ese punto de la barra.
    fn drag_bar(&mut self, bar: Bar, column: u16, row: u16) {
        let (Some(area), (vertical, horizontal)) = (self.editor_area, self.view.bars) else {
            return;
        };
        let (top, left) = self.view.free.unwrap_or_else(|| {
            let (t, l) = self.editor.scroll_offset();
            (t as usize, l as usize)
        });
        let fraction = |at: u16, start: u16, len: u16| {
            (at.saturating_sub(start) as f64 + 0.5) / len.max(1) as f64
        };
        let view = match (bar, vertical, horizontal) {
            (Bar::Vertical, Some(r), _) => {
                let lines = self.editor.lines().len();
                let page = area.height as usize;
                let at = fraction(row, r.y, r.height) * lines as f64;
                ((at - page as f64 / 2.0).max(0.0) as usize, left)
            }
            (Bar::Horizontal, _, Some(r)) => {
                let numbers = self.editor.line_number_width() as usize;
                let widest = numbers
                    + self
                        .editor
                        .lines()
                        .iter()
                        .map(|l| display_width(l))
                        .max()
                        .unwrap_or(0)
                    + 1;
                let page = area.width as usize;
                let at = fraction(column, r.x, r.width) * widest as f64;
                (top, (at - page as f64 / 2.0).max(0.0) as usize)
            }
            _ => return,
        };
        self.view.free = Some(self.clamp_view(view.0, view.1));
    }
}

/// Columnas que ocupa una línea (los tabuladores, como los dibuja el
/// widget: 4).
fn display_width(line: &str) -> usize {
    line.chars().map(|c| if c == '\t' { 4 } else { 1 }).sum()
}

/// La columna del widget (la que compara con su vista) de la columna `col`
/// del cursor en pantalla: con números de línea, el widget la desplaza
/// (ver `scroll_top_col` de `ratatui-textarea`).
fn widget_column(col: usize, numbers: usize) -> usize {
    if numbers == 0 {
        col
    } else if col <= numbers {
        col * 2
    } else {
        col + numbers
    }
}

/// Dónde empieza y cuánto mide el trozo que se ve en una barra de `cells`
/// celdas: `first` de `total` (con `page` a la vista).
fn thumb_span(first: usize, page: usize, total: usize, cells: u16) -> (u16, u16) {
    let cells_f = cells as f64;
    let total = total.max(page).max(1) as f64;
    let len = ((page as f64 / total) * cells_f).ceil().clamp(1.0, cells_f);
    let from = ((first as f64 / total) * cells_f).round();
    let from = from.min(cells_f - len);
    (from as u16, len as u16)
}

fn jump(editor: &mut ratatui_textarea::TextArea, (row, col): (usize, usize)) {
    let clamp = |n: usize| n.min(u16::MAX as usize) as u16;
    editor.move_cursor(CursorMove::Jump(clamp(row), clamp(col)));
}
