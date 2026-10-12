//! Dibujado de la interfaz. Cada panel es una función que lee el
//! [`ViewModel`] (o, el editor, su propio texto) y nunca el audio.

use ratatui::Frame;
use ratatui::buffer::Buffer;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style, Stylize};
use ratatui::symbols::Marker;
use ratatui::text::{Line, Span};
use ratatui::widgets::canvas::{Canvas, Line as CanvasLine};
use ratatui::widgets::{Block, Borders, Clear, Paragraph, Widget, Wrap};

use crate::app::App;
use crate::model::{CompletionView, PaletteView, ViewModel, fx_text, step_text};

pub const ERROR: Color = Color::Red;
/// Fondo de la línea con error (rojo oscuro de la paleta de 256 colores).
pub const ERROR_LINE: Color = Color::Indexed(52);
/// Texto subrayado de un paso con error en `steps("...")`.
pub const DIAGNOSTIC: Color = Color::LightRed;
pub const WARNING: Color = Color::Yellow;
pub const ACCENT: Color = Color::Cyan;
pub const DIM: Color = Color::DarkGray;
pub const WAVE: Color = Color::Gray;
pub const SLICE: Color = Color::Yellow;

/// Dibuja toda la interfaz.
pub fn draw(frame: &mut Frame, app: &mut App) {
    let view = app.view();
    let [main, fx, help] = Layout::vertical([
        Constraint::Min(8),
        Constraint::Length(FX_PANEL_HEIGHT),
        Constraint::Length(1),
    ])
    .areas(frame.area());
    let [left, right] =
        Layout::horizontal([Constraint::Percentage(50), Constraint::Percentage(50)]).areas(main);

    let popup = draw_editor(frame, app, &view, left);
    // Motor arriba (una línea por pista), sample y tracker debajo.
    let engine_height = (view.tracks.len() as u16 + 4).clamp(5, 12);
    // El tracker pide sus 16 filas (más cabecera y bordes); el sample se
    // queda con lo que sobre. Si no cabe, el tracker se desplaza.
    let [engine_area, sample_area, tracker_area] = Layout::vertical([
        Constraint::Length(engine_height),
        Constraint::Min(6),
        Constraint::Length(engine::STEPS as u16 + 3),
    ])
    .areas(right);
    frame.render_widget(EnginePanel(&view), engine_area);
    frame.render_widget(SamplePanel(&view), sample_area);
    frame.render_widget(TrackerPanel(&view), tracker_area);
    frame.render_widget(FxPanel(&view), fx);
    frame.render_widget(HelpLine(&view), help);
    // El autocompletado, encima de todo (puede salirse del editor).
    if let (Some(completion), Some(area)) = (&view.completion, popup) {
        frame.render_widget(CompletionPopup(completion), area);
    }
    if let Some(palette) = &view.palette {
        frame.render_widget(PalettePopup(palette), palette_area(frame.area()));
    }
}

/// Filas de opciones visibles en la paleta.
const PALETTE_ROWS: u16 = 12;

/// Sitio de la paleta: centrada arriba, como mucho 72 columnas.
pub fn palette_area(screen: Rect) -> Rect {
    let width = screen.width.saturating_sub(4).min(72);
    let height = (PALETTE_ROWS + 6).min(screen.height);
    Rect {
        x: screen.x + (screen.width - width) / 2,
        y: screen.y + (screen.height - height).min(2),
        width,
        height,
    }
}

/// Paleta de comandos: lo escrito, las acciones que coinciden (con su
/// atajo) y la descripción de la elegida.
pub struct PalettePopup<'a>(pub &'a PaletteView);

impl Widget for PalettePopup<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let view = self.0;
        Clear.render(area, buf);
        let block = Block::bordered()
            .border_style(Style::new().fg(ACCENT))
            .title(" paleta de comandos · ↑↓ Enter Esc ");
        let inner = block.inner(area);
        block.render(area, buf);
        if inner.height < 3 {
            return;
        }
        let width = inner.width as usize;
        let mut lines = vec![
            Line::from(vec![
                Span::from("› ").fg(ACCENT).bold(),
                Span::raw(format!("{}█", view.query)),
            ]),
            Line::from("─".repeat(width)).fg(DIM),
        ];
        let rows = (inner.height as usize)
            .saturating_sub(4)
            .min(PALETTE_ROWS as usize);
        let first = view.selected.saturating_sub(rows.saturating_sub(1));
        if view.items.is_empty() {
            lines.push(Line::from(" ninguna acción coincide").fg(DIM));
        }
        for (i, (name, keys)) in view.items.iter().enumerate().skip(first).take(rows) {
            let pad = width.saturating_sub(name.chars().count() + keys.chars().count() + 2);
            let line = Line::from(vec![
                Span::raw(format!(" {name}{}", " ".repeat(pad))),
                Span::from(format!("{keys} ")).fg(DIM),
            ]);
            lines.push(if i == view.selected {
                line.style(Style::new().fg(Color::Black).bg(ACCENT))
            } else {
                line
            });
        }
        while lines.len() < rows + 2 {
            lines.push(Line::raw(""));
        }
        lines.push(Line::from("─".repeat(width)).fg(DIM));
        lines.push(Line::from(view.description.clone()));
        Paragraph::new(lines).render(inner, buf);
    }
}

/// Dibuja el editor y devuelve dónde va el autocompletado, si está abierto.
fn draw_editor(frame: &mut Frame, app: &mut App, view: &ViewModel, area: Rect) -> Option<Rect> {
    let dirty = if view.dirty { " ●" } else { "" };
    let project = view
        .project
        .as_ref()
        .map_or(String::new(), |p| format!(" · {p}"));
    let mut block = Block::bordered().title(format!(" {}{dirty}{project} ", view.file));
    if view.external_change {
        block = block.title(
            Line::from(" ⚠ cambiado fuera · F3 recargar ")
                .fg(Color::Black)
                .bg(Color::Yellow),
        );
    }
    if view.playing_mode {
        block = block.title(
            Line::from(" TOCAR · 1-0 q-i · Esc ")
                .fg(Color::Black)
                .bg(Color::Yellow),
        );
    }
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
    // Ayuda de la función bajo el cursor, en la última línea del editor.
    let editor_area = match &view.help {
        Some(help) if editor_area.height > 3 => {
            let [editor, line] =
                Layout::vertical([Constraint::Min(1), Constraint::Length(1)]).areas(editor_area);
            frame.render_widget(
                Line::from(vec![
                    Span::from(format!("ⓘ {}", help.signature)).fg(ACCENT),
                    Span::from(format!("  {}", help.description)).fg(DIM),
                ]),
                line,
            );
            editor
        }
        _ => editor_area,
    };
    // Aviso de la notación de `steps("...")`, en vivo, al pie del editor.
    let editor_area = match view.diagnostics.first() {
        Some(first) => {
            let more = match view.diagnostics.len() {
                1 => String::new(),
                n => format!(" (+{} más)", n - 1),
            };
            let text = format!("⚠ línea {}: {}{more}", first.row + 1, first.message);
            let lines = text
                .chars()
                .count()
                .div_ceil(editor_area.width.max(1) as usize);
            let height = (lines as u16).clamp(1, 3);
            let [editor, warning] =
                Layout::vertical([Constraint::Min(1), Constraint::Length(height)])
                    .areas(editor_area);
            frame.render_widget(
                Paragraph::new(text).wrap(Wrap { trim: false }).fg(WARNING),
                warning,
            );
            editor
        }
        None => editor_area,
    };
    app.editor.set_line_number_style(Style::new().fg(DIM));
    app.editor
        .set_cursor_line_style(Style::new().add_modifier(Modifier::UNDERLINED));
    frame.render_widget(&app.editor, editor_area);

    // Subrayar los pasos con error de `steps("...")` que estén a la vista.
    let (top, left) = app.editor.scroll_offset();
    let text_x = editor_area.x + app.editor.line_number_width();
    for d in &view.diagnostics {
        let Some(y) = d
            .row
            .checked_sub(top as usize)
            .filter(|r| *r < editor_area.height as usize)
        else {
            continue;
        };
        let from = d.cols.start.saturating_sub(left as usize) as u16;
        let to = d.cols.end.saturating_sub(left as usize) as u16;
        let x = (text_x + from).min(editor_area.right());
        let width = (text_x + to).min(editor_area.right()).saturating_sub(x);
        frame.buffer_mut().set_style(
            Rect::new(x, editor_area.y + y as u16, width, 1),
            Style::new()
                .fg(DIAGNOSTIC)
                .underline_color(DIAGNOSTIC)
                .add_modifier(Modifier::UNDERLINED),
        );
    }

    let popup = view.completion.as_ref().map(|completion| {
        // Posición del cursor en pantalla (tras dibujar, con el desplazamiento
        // y los números de línea).
        let cursor = app.editor.screen_cursor();
        let (top, left) = app.editor.scroll_offset();
        let x = editor_area.x
            + app.editor.line_number_width()
            + (cursor.col.saturating_sub(left as usize) as u16);
        let y = editor_area.y + cursor.row.saturating_sub(top as usize) as u16;
        let x = x.saturating_sub(completion.prefix_len as u16);
        completion_area(completion, x, y, frame.area())
    });

    // Resaltar la línea del error si está a la vista. `scroll_offset` da la
    // primera línea visible tras dibujar (sin ajuste de línea).
    if let Some(line) = view.error.as_ref().and_then(|e| e.line) {
        let (top, _) = app.editor.scroll_offset();
        let row = line.saturating_sub(1);
        let top = top as usize;
        if row >= top && row < top + editor_area.height as usize {
            let y = editor_area.y + (row - top) as u16;
            frame.buffer_mut().set_style(
                Rect {
                    y,
                    height: 1,
                    ..editor_area
                },
                Style::new().bg(ERROR_LINE),
            );
        }
    }
    popup
}

/// Opciones visibles a la vez en el autocompletado.
const COMPLETION_ROWS: usize = 8;
/// Líneas de detalle (firma y descripción) del autocompletado, como mucho.
const COMPLETION_DETAIL: usize = 5;

/// Sitio del popup: debajo del cursor (`x`, `y`) si cabe; si no, encima.
/// Siempre dentro de `screen`.
pub fn completion_area(view: &CompletionView, x: u16, y: u16, screen: Rect) -> Rect {
    let widest = view
        .items
        .iter()
        .map(|(label, kind)| label.chars().count() + kind.chars().count() + 3)
        .chain([view.signature.chars().count()])
        .max()
        .unwrap_or(0);
    let width = (widest as u16 + 2).clamp(44, 72).min(screen.width);
    // Detalle: la firma y la descripción ajustadas al ancho.
    let inner = width.saturating_sub(2).max(1) as usize;
    let detail: usize = std::iter::once(view.signature.as_str())
        .chain(view.description.lines())
        .map(|l| l.chars().count().div_ceil(inner).max(1))
        .sum();
    let rows = view.items.len().min(COMPLETION_ROWS) as u16;
    let detail = detail.min(COMPLETION_DETAIL) as u16;
    let height = (rows + 1 + detail + 2).min(screen.height);
    let x = x.min(screen.right().saturating_sub(width));
    let y = if y + 1 + height <= screen.bottom() {
        y + 1
    } else {
        y.saturating_sub(height).max(screen.y)
    };
    Rect {
        x,
        y,
        width,
        height,
    }
}

/// Popup de autocompletado: opciones (con la elegida resaltada) y, debajo,
/// la firma y la descripción de la elegida.
pub struct CompletionPopup<'a>(pub &'a CompletionView);

impl Widget for CompletionPopup<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let view = self.0;
        Clear.render(area, buf);
        let block = Block::bordered()
            .border_style(Style::new().fg(ACCENT))
            .title(" completar · Tab · ↑↓ Enter · Esc ");
        let inner = block.inner(area);
        block.render(area, buf);
        if inner.height == 0 {
            return;
        }
        let rows = view.items.len().min(COMPLETION_ROWS);
        // Ventana de opciones que contiene la elegida.
        let first = view.selected.saturating_sub(rows.saturating_sub(1));
        let width = inner.width as usize;
        let mut lines: Vec<Line> = Vec::new();
        for (i, (label, kind)) in view.items.iter().enumerate().skip(first).take(rows) {
            let pad = width.saturating_sub(label.chars().count() + kind.chars().count() + 2);
            let line = Line::from(vec![
                Span::raw(format!(" {label}{}", " ".repeat(pad))),
                Span::from(format!("{kind} ")).fg(DIM),
            ]);
            lines.push(if i == view.selected {
                line.style(Style::new().fg(Color::Black).bg(ACCENT))
            } else {
                line
            });
        }
        lines.push(Line::from("─".repeat(width)).fg(DIM));
        let mut detail = vec![Line::from(view.signature.clone()).bold()];
        detail.extend(view.description.lines().map(|l| Line::from(l.to_string())));
        let [list, info] =
            Layout::vertical([Constraint::Length(lines.len() as u16), Constraint::Min(0)])
                .areas(inner);
        Paragraph::new(lines).render(list, buf);
        Paragraph::new(detail)
            .wrap(Wrap { trim: false })
            .render(info, buf);
    }
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
        let keys = if self.0.palette.is_some() {
            "PALETA: escribe para filtrar · ↑↓ elegir · Enter ejecutar · Esc cerrar"
        } else if self.0.completion.is_some() {
            "COMPLETAR: Tab aceptar · ↑↓ elegir y Enter aceptar · Esc cerrar · Enter sin elegir: salto de línea"
        } else if self.0.playing_mode {
            "TOCAR: 1-0 q-i cortes de la pista · Ctrl-O pista · Ctrl-T marcha/paro · F1 paleta · Esc editor"
        } else {
            "Ctrl-S guardar y cargar · Ctrl-L cargar · Ctrl-T marcha/paro · \
             Ctrl-G ir al error · Ctrl-O pista · Esc tocar · F1 paleta · F2 órdenes · \
             F4 completar · Ctrl-Q salir"
        };
        if let Some(prompt) = &self.0.prompt {
            Line::from(vec![
                Span::from("orden> ").fg(ACCENT).bold(),
                Span::raw(format!("{prompt}█  ")),
                Span::from(
                    "exportar [archivo.wav] [compases] · guardar [proyecto] · \
                     abrir proyecto · muestras carpeta · etiquetar corte nombre · Esc cancelar",
                )
                .fg(DIM),
            ])
            .render(area, buf);
            return;
        }
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

/// Forma de onda del sample de la pista seleccionada, en braille, con los
/// cortes marcados y el que está sonando resaltado.
pub struct SamplePanel<'a>(pub &'a ViewModel);

impl Widget for SamplePanel<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let view = self.0;
        let Some(track) = view.selected.and_then(|i| view.tracks.get(i)) else {
            Paragraph::new(Line::from("sin pistas").fg(DIM))
                .block(Block::bordered().title(" sample "))
                .render(area, buf);
            return;
        };
        let seconds = track.sample_frames as f64 / track.sample_rate.max(1) as f64;
        let selected = view.selected_slice;
        let chosen = selected.map_or(String::new(), |i| match track.label(i) {
            Some(label) => format!(" · corte {} {label}", i + 1),
            None => format!(" · corte {}", i + 1),
        });
        let block = Block::bordered().title(format!(
            " sample · {} · {} · {seconds:.2} s · {} cortes{chosen} ",
            track.name,
            track.sample_name,
            track.slices.len()
        ));
        let inner = block.inner(area);
        block.render(area, buf);
        if inner.width < 2 || inner.height < 2 || track.sample_frames == 0 {
            return;
        }

        // Resolución en braille: 2 puntos por celda en horizontal.
        let columns = inner.width as usize * 2;
        let frames = track.sample_frames as f64;
        let playing = track
            .playing_slice
            .and_then(|i| track.slices.get(i))
            .map(|r| (r.start as f64 / frames, r.end as f64 / frames));
        let wave = &track.waveform;
        Canvas::default()
            .marker(Marker::Braille)
            .x_bounds([0.0, columns as f64])
            .y_bounds([-1.0, 1.0])
            .paint(|ctx| {
                for x in 0..columns {
                    let from = x as f64 / columns as f64;
                    let to = (x + 1) as f64 / columns as f64;
                    let (lo, hi) = wave.range(from, to);
                    let lit = playing.is_some_and(|(a, b)| from < b && to > a);
                    ctx.draw(&CanvasLine {
                        x1: x as f64,
                        y1: lo as f64,
                        x2: x as f64,
                        y2: hi as f64,
                        color: if lit { ACCENT } else { WAVE },
                    });
                }
                // Marcas de los cortes encima.
                ctx.layer();
                for slice in &track.slices {
                    let x = (slice.start as f64 / frames * columns as f64).floor();
                    ctx.draw(&CanvasLine {
                        x1: x,
                        y1: -1.0,
                        x2: x,
                        y2: 1.0,
                        color: SLICE,
                    });
                }
            })
            .render(inner, buf);

        // Número de cada corte (desde 1) en lo alto de su marca, con su
        // etiqueta si la tiene (`1 kick`; la marca siguiente la puede tapar).
        // Se escribe en el buffer para que caiga en la misma celda que la
        // línea braille (`ctx.print` redondea la posición de otra forma).
        for (i, slice) in track.slices.iter().enumerate() {
            let column = (slice.start as f64 / frames * inner.width as f64).floor() as u16;
            if column < inner.width {
                let text = slice_mark(track, i);
                let room = (inner.width - column) as usize;
                buf.set_string(
                    inner.x + column,
                    inner.y,
                    text.chars().take(room).collect::<String>(),
                    Style::new().fg(SLICE).bold(),
                );
            }
        }
        // El corte seleccionado, encima y resaltado.
        if let Some(i) = selected
            && let Some(slice) = track.slices.get(i)
        {
            let column = (slice.start as f64 / frames * inner.width as f64).floor() as u16;
            if column < inner.width {
                let text = slice_mark(track, i);
                let room = (inner.width - column) as usize;
                buf.set_string(
                    inner.x + column,
                    inner.y,
                    text.chars().take(room).collect::<String>(),
                    Style::new().fg(Color::Black).bg(SLICE).bold(),
                );
            }
        }
    }
}

/// Texto de la marca de un corte en la vista del sample: `3` o `3 snare`.
fn slice_mark(track: &crate::model::TrackView, i: usize) -> String {
    match track.label(i) {
        Some(label) => format!("{} {label}", i + 1),
        None => (i + 1).to_string(),
    }
}

/// Ancho de cada columna del tracker.
const TRACKER_COLUMN: usize = 11;

/// Tracker vertical: un paso por fila y una columna por pista, con la fila
/// que está sonando resaltada.
pub struct TrackerPanel<'a>(pub &'a ViewModel);

impl Widget for TrackerPanel<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let view = self.0;
        let t = view.transport;
        let block = Block::bordered().title(format!(" tracker · compás {} ", t.bar));
        let inner = block.inner(area);
        block.render(area, buf);
        if inner.height < 2 {
            return;
        }
        let current = t.step.filter(|_| t.running);

        // Cabecera con los nombres de las pistas.
        let mut header = vec![Span::raw("   ")];
        for (i, track) in view.tracks.iter().enumerate() {
            let name: String = track.name.chars().take(TRACKER_COLUMN - 1).collect();
            let span = Span::from(format!("{name:<TRACKER_COLUMN$}"));
            header.push(if view.selected == Some(i) {
                span.fg(ACCENT).bold()
            } else {
                span.bold()
            });
        }
        Line::from(header).render(Rect { height: 1, ..inner }, buf);

        // Si no caben las 16 filas, se desplaza para ver la que suena.
        let visible = (inner.height - 1) as usize;
        let first = if visible >= engine::STEPS {
            0
        } else {
            current
                .map_or(0, |s| s.saturating_sub(visible / 2))
                .min(engine::STEPS - visible)
        };
        for (row, step) in (first..engine::STEPS).take(visible).enumerate() {
            let mut spans = vec![Span::from(format!("{step:02} ")).fg(DIM)];
            for track in &view.tracks {
                let cell = &track.steps[step];
                let cell_text: String = step_text(cell, &track.labels)
                    .chars()
                    .take(TRACKER_COLUMN - 1)
                    .collect();
                let text = format!("{cell_text:<TRACKER_COLUMN$}");
                spans.push(if cell.is_some() {
                    Span::raw(text)
                } else {
                    Span::from(text).fg(DIM)
                });
            }
            let mut line = Line::from(spans);
            if current == Some(step) {
                line = line.style(Style::new().bg(Color::Blue).fg(Color::White).bold());
            }
            let y = inner.y + 1 + row as u16;
            let row_area = Rect {
                y,
                height: 1,
                ..inner
            };
            if current == Some(step) {
                buf.set_style(row_area, Style::new().bg(Color::Blue));
            }
            line.render(row_area, buf);
        }
    }
}

/// Alto del panel de efectos: nombre, volumen, nivel y hasta 4 efectos.
pub const FX_PANEL_HEIGHT: u16 = 3 + engine::MAX_FX as u16 + 2;
/// Ancho mínimo de cada columna del panel de efectos.
const FX_COLUMN_MIN: u16 = 20;

/// Panel inferior: una columna por pista con su volumen, su nivel y su
/// cadena de efectos.
pub struct FxPanel<'a>(pub &'a ViewModel);

impl Widget for FxPanel<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let view = self.0;
        let block = Block::bordered().title(" efectos por pista ");
        let inner = block.inner(area);
        block.render(area, buf);
        if view.tracks.is_empty() {
            Line::from("sin pistas").fg(DIM).render(inner, buf);
            return;
        }
        // Tantas columnas como quepan; las pistas que no caben no se dibujan.
        let fit = (inner.width / FX_COLUMN_MIN).max(1) as usize;
        let shown = view.tracks.len().min(fit);
        let columns =
            Layout::horizontal(vec![Constraint::Ratio(1, shown as u32); shown]).split(inner);
        for (i, (track, column)) in view.tracks.iter().zip(columns.iter()).enumerate() {
            let width = column.width.saturating_sub(7).clamp(4, 12) as usize;
            let name = Span::from(track.name.clone()).bold();
            let name = if view.selected == Some(i) {
                name.fg(ACCENT)
            } else {
                name
            };
            let mut title = vec![name];
            if track.swing > 50.0 {
                title.push(Span::from(format!(" · swing {}", track.swing)).fg(DIM));
            }
            let mut lines = vec![
                Line::from(title),
                Line::from(vec![
                    Span::raw(format!("vol {:.2} ", track.gain)),
                    Span::from(meter(
                        track.gain / engine::MAX_GAIN,
                        width.saturating_sub(4),
                    ))
                    .fg(ACCENT),
                ]),
                Line::from(vec![
                    Span::raw("nivel "),
                    Span::from(meter(track.level, width)).fg(Color::Green),
                ]),
            ];
            if track.fx.is_empty() {
                lines.push(Line::from("sin efectos").fg(DIM));
            }
            for fx in &track.fx {
                lines.push(Line::from(format!("› {}", fx_text(fx))));
            }
            Paragraph::new(lines).render(*column, buf);
        }
    }
}
