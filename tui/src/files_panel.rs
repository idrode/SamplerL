//! El panel del explorador de archivos: el árbol con su icono por fila,
//! la línea para escribir un nombre (crear o renombrar) o confirmar un
//! borrado, y la vista previa del sample elegido. [`FilesLayout`] es su
//! geometría, la misma para dibujar y para el ratón.

use std::path::PathBuf;
use std::sync::Arc;

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Color, Style, Stylize};
use ratatui::text::Line;
use ratatui::widgets::{Block, Clear, Widget};

use crate::files::{Kind, Row};
use crate::model::Waveform;

/// Lo que dibuja [`FilesPanel`].
#[derive(Debug, Clone, Default)]
pub struct FilesView {
    pub rows: Vec<Row>,
    pub selected: usize,
    pub top: usize,
    /// El script del editor (se marca en el árbol).
    pub current: Option<PathBuf>,
    /// Tiene el foco.
    pub focused: bool,
    /// La línea de abajo: lo que se pregunta y lo escrito.
    pub input: Option<(String, String)>,
    /// La vista previa del sample elegido.
    pub preview: Option<Preview>,
    /// Los marcados con `m` para añadirlos juntos.
    pub marked: Vec<PathBuf>,
}

/// La vista previa de un sample.
#[derive(Debug, Clone)]
pub struct Preview {
    pub name: String,
    pub info: String,
    pub waveform: Arc<Waveform>,
}

/// Filas de la vista previa (su nombre y la onda).
const PREVIEW_ROWS: u16 = 4;

/// Qué hay bajo el ratón en el explorador.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FilesHit {
    /// La fila `i` del árbol (su nombre).
    Row(usize),
    /// El icono de la fila `i` (en una carpeta, abre o cierra).
    Arrow(usize),
    /// El borde derecho: arrastrarlo cambia el ancho.
    Edge,
}

/// Dónde va cada parte del panel.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FilesLayout {
    pub area: Rect,
    /// Las filas del árbol.
    pub tree: Rect,
    pub input: Option<Rect>,
    pub preview: Option<Rect>,
    /// La primera fila que se ve y cuántas hay.
    pub top: usize,
    pub rows: usize,
}

impl FilesLayout {
    pub fn new(view: &FilesView, area: Rect) -> FilesLayout {
        let inner = Block::bordered().inner(area);
        let mut tree = inner;
        let mut take = |n: u16| {
            let n = n.min(tree.height);
            tree.height -= n;
            Rect {
                y: tree.y + tree.height,
                height: n,
                ..tree
            }
        };
        let input = view.input.as_ref().map(|_| take(1));
        let preview = view
            .preview
            .as_ref()
            .filter(|_| inner.height >= 10)
            .map(|_| take(PREVIEW_ROWS));
        FilesLayout {
            area,
            tree,
            input,
            preview,
            top: view.top,
            rows: view.rows.len(),
        }
    }

    /// Lo que hay en (`x`, `y`).
    pub fn hit(&self, x: u16, y: u16) -> Option<FilesHit> {
        let at = ratatui::layout::Position::new(x, y);
        if !self.area.contains(at) {
            return None;
        }
        if x + 1 == self.area.right() && y > self.area.y && y + 1 < self.area.bottom() {
            return Some(FilesHit::Edge);
        }
        if !self.tree.contains(at) {
            return None;
        }
        let i = self.top + (y - self.tree.y) as usize;
        if i >= self.rows {
            return None;
        }
        Some(FilesHit::Row(i))
    }

    /// Como [`hit`](Self::hit), distinguiendo el icono (la columna de su
    /// sangría) del nombre.
    pub fn hit_in(&self, view: &FilesView, x: u16, y: u16) -> Option<FilesHit> {
        match self.hit(x, y)? {
            FilesHit::Row(i) => {
                let row = view.rows.get(i)?;
                let icon = self.tree.x + 1 + 2 * row.depth as u16;
                Some(
                    if row.kind == Kind::Dir && !row.note && (x == icon || x == icon + 1) {
                        FilesHit::Arrow(i)
                    } else {
                        FilesHit::Row(i)
                    },
                )
            }
            other => Some(other),
        }
    }
}

pub struct FilesPanel<'a>(pub &'a FilesView);

impl Widget for FilesPanel<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let view = self.0;
        let theme = crate::theme::get();
        let ascii = crate::theme::ascii();
        Clear.render(area, buf);
        let block = if view.focused {
            Block::bordered()
                .border_style(Style::new().fg(theme.border_focus))
                .title_style(Style::new().fg(theme.border_focus).bold())
        } else {
            Block::bordered().border_style(Style::new().fg(theme.border))
        };
        block.title(" 0 archivos ").render(area, buf);
        let layout = FilesLayout::new(view, area);
        let tree = layout.tree;
        for (row_y, (i, row)) in view
            .rows
            .iter()
            .enumerate()
            .skip(view.top)
            .take(tree.height as usize)
            .enumerate()
        {
            let y = tree.y + row_y as u16;
            let selected = i == view.selected;
            let current = view.current.as_ref() == Some(&row.path) && !row.note;
            let mut style = if row.note || row.kind == Kind::Unsupported {
                Style::new().fg(theme.dim)
            } else if row.kind == Kind::Dir {
                Style::new().fg(theme.accent)
            } else if current {
                Style::new().bold()
            } else {
                Style::new()
            };
            if selected {
                style = if view.focused {
                    style.fg(Color::Black).bg(theme.accent)
                } else {
                    style.bg(theme.header)
                };
                buf.set_style(
                    Rect {
                        y,
                        height: 1,
                        ..tree
                    },
                    style,
                );
            }
            let indent = "  ".repeat(row.depth);
            let icon = if row.note {
                " "
            } else {
                row.kind.icon(row.open, ascii)
            };
            let mark = match (current, ascii) {
                (true, false) => " ◂",
                (true, true) => " <",
                _ => "",
            };
            let hint = row
                .hint
                .as_ref()
                .map_or_else(String::new, |h| format!("  {h}"));
            let label = row
                .label
                .as_ref()
                .map_or_else(String::new, |l| format!("[{l}] "));
            let marked = if !row.note && view.marked.contains(&row.path) {
                if ascii { "* " } else { "● " }
            } else {
                ""
            };
            let text = format!(" {indent}{marked}{icon} {label}{}{mark}{hint}", row.name);
            let text: String = text.chars().take(tree.width as usize).collect();
            buf.set_string(tree.x, y, text, style);
        }
        // Lo que hay más abajo o más arriba, en el borde.
        if view.top > 0 {
            buf.set_string(area.right() - 3, area.y, "▲", Style::new().fg(theme.dim));
        }
        if view.top + (tree.height as usize) < view.rows.len() {
            buf.set_string(
                area.right() - 3,
                area.bottom() - 1,
                "▼",
                Style::new().fg(theme.dim),
            );
        }
        if let (Some(rect), Some((label, text))) = (layout.input, &view.input) {
            let line = Line::from(format!("{label}{text}▏"))
                .fg(Color::Black)
                .bg(theme.accent);
            buf.set_style(rect, Style::new().bg(theme.accent));
            line.render(rect, buf);
        }
        if let (Some(rect), Some(preview)) = (layout.preview, &view.preview) {
            let head = Rect { height: 1, ..rect };
            let name = format!("{} · {}", preview.name, preview.info);
            buf.set_string(
                head.x,
                head.y,
                name.chars().take(head.width as usize).collect::<String>(),
                Style::new().fg(theme.dim),
            );
            let wave = Rect {
                y: rect.y + 1,
                height: rect.height.saturating_sub(1),
                ..rect
            };
            crate::ui::draw_waveform(&preview.waveform, wave, buf, crate::ui::wave_color());
        }
    }
}
