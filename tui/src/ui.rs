//! Dibujado de la interfaz. Cada panel es una función que lee el
//! [`ViewModel`] (o, el editor, su propio texto) y nunca el audio.

use ratatui::Frame;
use ratatui::buffer::Buffer;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style, Stylize};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph, Widget, Wrap};

use crate::app::App;
use crate::model::ViewModel;

pub const ERROR: Color = Color::Red;
pub const ACCENT: Color = Color::Cyan;
pub const DIM: Color = Color::DarkGray;

/// Dibuja toda la interfaz.
pub fn draw(frame: &mut Frame, app: &mut App) {
    let view = app.view();
    let [main, help] =
        Layout::vertical([Constraint::Min(5), Constraint::Length(1)]).areas(frame.area());
    let [left, right] =
        Layout::horizontal([Constraint::Percentage(50), Constraint::Percentage(50)]).areas(main);

    draw_editor(frame, app, &view, left);
    frame.render_widget(EnginePanel(&view), right);
    frame.render_widget(HelpLine(&view), help);
}

fn draw_editor(frame: &mut Frame, app: &mut App, view: &ViewModel, area: Rect) {
    let dirty = if view.dirty { " ●" } else { "" };
    let mut block = Block::bordered().title(format!(" {}{dirty} ", view.file));
    if let Some(error) = &view.error {
        // El error se muestra en el propio editor: resumen en el borde y
        // mensaje completo al pie, en rojo.
        let summary = match error.line {
            Some(line) => format!(" ✗ error en la línea {line} · Ctrl-G "),
            None => " ✗ error ".to_string(),
        };
        block = block
            .border_style(Style::new().fg(ERROR))
            .title_bottom(Line::from(summary).fg(ERROR));
    }
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let editor_area = match &view.error {
        Some(error) => {
            let text = format!("✗ {}", error.message);
            // Líneas que ocupa al ajustarse al ancho, más el borde superior.
            let lines = text.chars().count().div_ceil(inner.width.max(1) as usize);
            let height = (lines as u16 + 1).clamp(2, 6);
            let message = Paragraph::new(text)
                .wrap(Wrap { trim: false })
                .fg(ERROR)
                .block(
                    Block::new()
                        .borders(Borders::TOP)
                        .border_style(Style::new().fg(ERROR)),
                );
            let [editor, error_area] =
                Layout::vertical([Constraint::Min(1), Constraint::Length(height)]).areas(inner);
            frame.render_widget(message, error_area);
            editor
        }
        None => inner,
    };
    app.editor.set_line_number_style(Style::new().fg(DIM));
    app.editor
        .set_cursor_line_style(Style::new().add_modifier(Modifier::UNDERLINED));
    frame.render_widget(&app.editor, editor_area);
}

/// Medidor de nivel en texto: `███░░░`.
pub fn meter(level: f32, width: usize) -> String {
    let filled = ((level.clamp(0.0, 1.0) * width as f32).round() as usize).min(width);
    format!("{}{}", "█".repeat(filled), "░".repeat(width - filled))
}

/// Estado del motor: transporte y pistas.
pub struct EnginePanel<'a>(pub &'a ViewModel);

impl Widget for EnginePanel<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let view = self.0;
        let t = view.transport;
        let state = if t.running {
            Span::from("▶ en marcha").fg(Color::Green)
        } else {
            Span::from("■ parado").fg(DIM)
        };
        let step = t.step.map_or("--".into(), |s| format!("{s:02}"));
        let mut lines = vec![
            Line::from(vec![
                state,
                Span::raw(format!(
                    "   {:.1} BPM   compás {}   paso {step}",
                    t.bpm, t.bar
                )),
            ]),
            Line::raw(""),
        ];
        if view.tracks.is_empty() {
            lines.push(Line::from("sin pistas").fg(DIM));
        }
        for (i, track) in view.tracks.iter().enumerate() {
            let selected = view.selected == Some(i);
            let marker = if selected { "›" } else { " " };
            let slice = track
                .playing_slice
                .map_or("--".into(), |s| format!("{:02}", s + 1));
            let name = Span::from(format!("{marker}{:<10}", track.name));
            let name = if selected {
                name.fg(ACCENT).bold()
            } else {
                name
            };
            lines.push(Line::from(vec![
                name,
                Span::from(meter(track.level, 8)).fg(Color::Green),
                Span::raw(format!(
                    " ▶{slice}  {} cortes  ciclo {}  pista {}",
                    track.slices.len(),
                    track.cycle,
                    track.slot.map_or("-".into(), |s| (s + 1).to_string()),
                )),
            ]));
        }
        Paragraph::new(lines)
            .block(Block::bordered().title(" motor "))
            .render(area, buf);
    }
}

/// Línea inferior: mensaje y atajos.
pub struct HelpLine<'a>(pub &'a ViewModel);

impl Widget for HelpLine<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let keys = "Ctrl-S guardar y cargar · Ctrl-L cargar · Ctrl-T marcha/paro · \
                    Ctrl-G ir al error · Ctrl-O pista · Ctrl-Q salir";
        let message = &self.0.message;
        let line = if message.is_empty() {
            Line::from(keys).fg(DIM)
        } else {
            Line::from(vec![
                Span::raw(format!("{message}  ")),
                Span::from(keys).fg(DIM),
            ])
        };
        line.render(area, buf);
    }
}
