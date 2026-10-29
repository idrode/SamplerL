//! El panel de dispositivos: tres listas (audio de salida, MIDI de entrada
//! y MIDI de salida) con el que está en uso marcado, un cursor que se mueve
//! con las flechas y Enter para elegir. El mismo panel sale encima de la
//! ventana principal (paleta, «Dispositivos») o llena su propia ventana
//! (`tui --window-devices`); aquí están el dibujo y las teclas, que son las
//! mismas en los dos sitios.

use std::time::{Duration, Instant};

use ratatui::buffer::Buffer;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Style, Stylize};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Clear, Paragraph, Widget};
use ratatui_textarea::{Input, Key};

use crate::devices::{DevicesView, Entry, Kind};
use crate::ui::{accent, dim, warning_color};

/// Lo que dura encendido el indicador de actividad MIDI tras una nota.
pub const BLINK: Duration = Duration::from_millis(150);
/// Desde este ancho, las tres listas una al lado de otra.
const COLUMNS_FROM: u16 = 110;

/// Dónde está el cursor: la lista y la fila.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Cursor {
    pub kind: Kind,
    pub row: usize,
}

impl Default for Cursor {
    fn default() -> Self {
        Cursor {
            kind: Kind::Audio,
            row: 0,
        }
    }
}

impl Cursor {
    /// El cursor sobre el que está en uso en la lista `kind`.
    pub fn on_active(view: &DevicesView, kind: Kind) -> Cursor {
        let row = view.list(kind).iter().position(|e| e.active).unwrap_or(0);
        Cursor { kind, row }
    }

    /// Lo deja dentro de la lista (si cambió de tamaño al refrescar).
    pub fn clamp(&mut self, view: &DevicesView) {
        let len = view.list(self.kind).len();
        self.row = self.row.min(len.saturating_sub(1));
    }
}

/// Qué pide una tecla del panel.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PanelKey {
    Choose(Kind, String),
    Refresh,
    Close,
    /// Movió el cursor (o nada).
    Moved,
}

/// Una tecla en el panel: ↑↓ fila, ←→ o Tab lista, Enter elige, `r`
/// refresca, Esc (o `q`) cierra.
pub fn key(view: &DevicesView, cursor: &mut Cursor, input: &Input) -> PanelKey {
    let len = view.list(cursor.kind).len();
    let index = Kind::ALL
        .iter()
        .position(|k| *k == cursor.kind)
        .unwrap_or(0);
    let other = |step: usize| Kind::ALL[(index + step) % Kind::ALL.len()];
    match input.key {
        Key::Up => cursor.row = cursor.row.saturating_sub(1),
        Key::Down => cursor.row = (cursor.row + 1).min(len.saturating_sub(1)),
        Key::Home => cursor.row = 0,
        Key::End => cursor.row = len.saturating_sub(1),
        Key::Right | Key::Tab => *cursor = Cursor::on_active(view, other(1)),
        Key::Left => *cursor = Cursor::on_active(view, other(Kind::ALL.len() - 1)),
        Key::Enter => {
            if let Some(entry) = view.list(cursor.kind).get(cursor.row) {
                return PanelKey::Choose(cursor.kind, entry.name.clone());
            }
        }
        Key::Char('r') | Key::Char('R') if !input.ctrl && !input.alt => return PanelKey::Refresh,
        Key::Esc => return PanelKey::Close,
        Key::Char('q') if !input.ctrl && !input.alt => return PanelKey::Close,
        _ => {}
    }
    PanelKey::Moved
}

/// El indicador de actividad MIDI: se enciende un momento cada vez que
/// cambia la cuenta de notas recibidas.
#[derive(Debug, Clone, Default)]
pub struct Blink {
    count: u64,
    at: Option<Instant>,
}

impl Blink {
    pub fn observe(&mut self, count: u64, now: Instant) {
        if count != self.count {
            self.count = count;
            self.at = Some(now);
        }
    }

    pub fn lit(&self, now: Instant) -> bool {
        self.at.is_some_and(|at| now.duration_since(at) < BLINK)
    }
}

/// El panel: la salida que suena arriba, las tres listas y las teclas.
pub struct DevicesPanel<'a> {
    pub view: &'a DevicesView,
    pub cursor: Cursor,
    /// El indicador de actividad MIDI, encendido.
    pub lit: bool,
    /// Lo que se ve en el título (con el enlace en la ventana propia).
    pub title: &'a str,
}

/// Sitio del panel encima de la ventana principal: centrado, tan alto como
/// pida (y quepa).
pub fn popup_area(screen: Rect, view: &DevicesView) -> Rect {
    let width = screen.width.saturating_sub(4).min(96);
    let rows: usize = Kind::ALL
        .iter()
        .map(|k| 1 + view.list(*k).len().max(1))
        .sum();
    // Bordes, la línea de estado, el aviso y un hueco entre listas.
    let height = (rows as u16 + 6).min(screen.height);
    Rect {
        x: screen.x + (screen.width - width) / 2,
        y: screen.y + (screen.height - height).min(2),
        width,
        height,
    }
}

impl Widget for DevicesPanel<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        Clear.render(area, buf);
        let block = Block::bordered()
            .border_style(Style::new().fg(accent()))
            .title(format!(" {} ", self.title))
            .title_bottom(" ↑↓ elegir · ←→/Tab lista · Enter usar · r refrescar · Esc ");
        let inner = block.inner(area);
        block.render(area, buf);
        if inner.height == 0 || inner.width == 0 {
            return;
        }
        let [status, lists, notice] = Layout::vertical([
            Constraint::Length(1),
            Constraint::Min(0),
            Constraint::Length(1),
        ])
        .areas(inner);
        Line::from(self.view.status.clone())
            .fg(accent())
            .render(status, buf);
        let notice_color = if self.view.notice.contains("no ") {
            warning_color()
        } else {
            dim()
        };
        Line::from(self.view.notice.clone())
            .fg(notice_color)
            .render(notice, buf);
        let areas: Vec<Rect> = if inner.width >= COLUMNS_FROM {
            Layout::horizontal([Constraint::Ratio(1, 3); 3])
                .spacing(1)
                .split(lists)
                .to_vec()
        } else {
            // Una debajo de otra, con el sitio según lo que tiene cada una
            // (y al menos su título y una fila).
            let wants: Vec<u16> = Kind::ALL
                .iter()
                .map(|k| 1 + self.view.list(*k).len().max(1) as u16)
                .collect();
            Layout::vertical(wants.iter().map(|w| Constraint::Max(*w + 1)))
                .split(lists)
                .to_vec()
        };
        for (kind, area) in Kind::ALL.into_iter().zip(areas) {
            self.render_list(kind, area, buf);
        }
    }
}

impl DevicesPanel<'_> {
    fn render_list(&self, kind: Kind, area: Rect, buf: &mut Buffer) {
        if area.height == 0 {
            return;
        }
        let focused = self.cursor.kind == kind;
        let mut title = vec![Span::from(kind.title()).bold()];
        if focused {
            title[0] = title[0].clone().fg(accent());
        }
        if kind == Kind::MidiIn && !self.view.midi_in.is_empty() {
            let (mark, color) = if self.lit {
                ("  ● nota", accent())
            } else {
                ("  ○ nota", dim())
            };
            title.push(Span::from(mark).fg(color));
        }
        Line::from(title).render(Rect { height: 1, ..area }, buf);
        let rows = Rect {
            y: area.y + 1,
            height: area.height - 1,
            ..area
        };
        let entries = self.view.list(kind);
        if entries.is_empty() {
            let note = self.view.note(kind);
            Paragraph::new(Line::from(format!("  {note}")).fg(dim())).render(rows, buf);
            return;
        }
        // Desplazada para que se vea el cursor.
        let visible = rows.height as usize;
        let first = if focused {
            (self.cursor.row + 1).saturating_sub(visible)
        } else {
            entries
                .iter()
                .position(|e| e.active)
                .map_or(0, |a| (a + 1).saturating_sub(visible))
        };
        let lines: Vec<Line> = entries
            .iter()
            .enumerate()
            .skip(first)
            .take(visible)
            .map(|(i, e)| entry_line(e, focused && i == self.cursor.row, rows.width))
            .collect();
        Paragraph::new(lines).render(rows, buf);
    }
}

/// Una fila: el cursor, la marca del que está en uso, el nombre y lo demás.
fn entry_line(entry: &Entry, cursor: bool, width: u16) -> Line<'static> {
    let pointer = if cursor { "▸" } else { " " };
    let mark = if entry.active { "●" } else { " " };
    let mut name = Span::from(format!("{pointer}{mark} {}", entry.name));
    if entry.active {
        name = name.fg(accent()).bold();
    }
    if cursor {
        name = name.reversed();
    }
    let mut spans = vec![name];
    let mut extra = Vec::new();
    if entry.default {
        extra.push("predeterminado".to_string());
    }
    if !entry.detail.is_empty() {
        extra.push(entry.detail.clone());
    }
    let used = entry.name.chars().count() + 3;
    if !extra.is_empty() && (width as usize) > used + 4 {
        spans.push(Span::from(format!("  {}", extra.join(" · "))).fg(dim()));
    }
    Line::from(spans)
}
