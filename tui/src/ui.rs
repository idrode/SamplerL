//! Dibujado de la interfaz. Cada panel es una función que lee el
//! [`ViewModel`] (o, el editor, su propio texto) y nunca el audio.

use ratatui::Frame;
use ratatui::buffer::Buffer;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style, Stylize};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph, Widget, Wrap};

use crate::app::App;
use crate::model::{
    BlockPickerView, BrowserView, ClashTableView, CompletionView, PaletteView, SlotsView,
    TimelineView, ViewModel, fx_text, fx_text_named,
};
use crate::theme;

// Colores del tema (ver `crate::theme`).
pub fn error_color() -> Color {
    theme::get().error
}
/// Fondo de la línea con error (rojo oscuro de la paleta de 256 colores).
pub fn error_line() -> Color {
    theme::get().error_line
}
/// Texto subrayado de un paso con error en `steps("...")`.
pub fn diagnostic() -> Color {
    theme::get().diagnostic
}
pub fn warning_color() -> Color {
    theme::get().warning
}
pub fn accent() -> Color {
    theme::get().accent
}
pub fn dim() -> Color {
    theme::get().dim
}
pub fn wave_color() -> Color {
    theme::get().wave
}
pub fn slice_color() -> Color {
    theme::get().slice
}
fn playing_color() -> Color {
    theme::get().playing
}
fn meter_color() -> Color {
    theme::get().meter
}

/// Dibuja toda la interfaz.
pub fn draw(frame: &mut Frame, app: &mut App) {
    let view = app.view();
    if view.editor_outside {
        // Sin editor aquí, el ratón no tiene dónde escribir.
        app.editor_area = None;
        draw_main_window(frame, &view);
        return;
    }
    let [main, fx, help] = Layout::vertical([
        Constraint::Min(8),
        Constraint::Length(fx_panel_height(&view)),
        Constraint::Length(1),
    ])
    .areas(frame.area());
    let [left, right] =
        Layout::horizontal([Constraint::Percentage(50), Constraint::Percentage(50)]).areas(main);

    let popup = draw_editor(frame, app, &view, left);
    // Motor arriba (una línea por pista), sample y tracker debajo.
    let engine_height = (view.tracks.len() as u16 + 4).clamp(5, 12);
    // El tracker pide 16 filas (más cabecera y bordes); el sample se queda
    // con lo que sobre. Si así no queda nada para el sample, el tracker se
    // queda sus filas mientras el sample tenga lo mínimo; si ni así, el
    // reparto de siempre y el tracker se desplaza.
    let rest = right.height.saturating_sub(engine_height);
    let [engine_area, sample_area, tracker_area] = Layout::vertical([
        Constraint::Length(engine_height),
        Constraint::Min(tracker_first(rest, SAMPLE_MIN, SAMPLE_SQUEEZED)),
        Constraint::Length(TRACKER_HEIGHT),
    ])
    .areas(right);
    // Con línea de tiempo, una fila con las secciones bajo el motor (se la
    // quita al sample).
    let sample_area = match &view.timeline {
        Some(timeline) if sample_area.height > 1 => {
            let [row, rest] =
                Layout::vertical([Constraint::Length(1), Constraint::Min(0)]).areas(sample_area);
            frame.render_widget(TimelineBar(timeline), row);
            rest
        }
        _ => sample_area,
    };
    frame.render_widget(EnginePanel(&view), engine_area);
    frame.render_widget(SamplePanel(&view), sample_area);
    frame.render_widget(TrackerPanel(&view), tracker_area);
    frame.render_widget(FxPanel(&view), fx);
    frame.render_widget(HelpLine(&view), help);
    draw_popups(frame, &view, popup);
}

/// Alto del tracker con sus 16 filas sin desplazarse (cabecera y bordes).
pub const TRACKER_HEIGHT: u16 = engine::STEPS as u16 + 3;
/// Alto mínimo del panel del sample con espacio de sobra y cuando hace
/// falta para que el tracker quepa entero (bordes y dos filas de onda).
const SAMPLE_MIN: u16 = 6;
const SAMPLE_SQUEEZED: u16 = 4;
/// Lo mismo para la forma de onda grande (con su fila de números y la de
/// los ms).
const BIG_SAMPLE_SQUEEZED: u16 = 6;

/// Alto mínimo del panel de encima del tracker con `height` filas para los
/// dos: `normal`, salvo que con él el tracker no quepa entero y con
/// `squeezed` sí (entonces, lo que deja el tracker).
fn tracker_first(height: u16, normal: u16, squeezed: u16) -> u16 {
    let left = height.saturating_sub(TRACKER_HEIGHT);
    if left < normal && left >= squeezed {
        left
    } else {
        normal
    }
}

/// La ventana principal con el editor en la suya (fase L): las vistas
/// ocupan toda la ventana. Arriba, el transporte y el enlace; la forma de
/// onda grande; debajo, el mezclador y el tracker; abajo, los efectos.
fn draw_main_window(frame: &mut Frame, view: &ViewModel) {
    let [top, body, fx, help] = Layout::vertical([
        Constraint::Length(1),
        Constraint::Min(8),
        Constraint::Length(fx_panel_height(view)),
        Constraint::Length(1),
    ])
    .areas(frame.area());
    frame.render_widget(StatusLine(view), top);
    // Con línea de tiempo, una fila con las secciones.
    let body = match &view.timeline {
        Some(timeline) if body.height > 1 => {
            let [row, rest] =
                Layout::vertical([Constraint::Length(1), Constraint::Min(0)]).areas(body);
            frame.render_widget(TimelineBar(timeline), row);
            rest
        }
        _ => body,
    };
    // El tracker pide sus 16 filas; el sample se queda con lo demás (y
    // al menos un tercio, o lo mínimo para que el tracker quepa entero).
    let [sample_area, lower] = Layout::vertical([
        Constraint::Min(tracker_first(
            body.height,
            body.height / 3,
            BIG_SAMPLE_SQUEEZED,
        )),
        Constraint::Length(TRACKER_HEIGHT),
    ])
    .areas(body);
    // Con un análisis de choques hecho, su tabla a la derecha del sample.
    let width = view
        .clash_table
        .as_ref()
        .map_or(0, |table| ClashPanel::width(table, sample_area.width));
    let [sample_area, clash_area] =
        Layout::horizontal([Constraint::Min(0), Constraint::Length(width)]).areas(sample_area);
    frame.render_widget(BigSamplePanel(view), sample_area);
    if width > 0 {
        frame.render_widget(ClashPanel(view), clash_area);
    }
    // El mezclador, lo que pidan sus tiras (hasta la mitad); el tracker,
    // lo demás.
    let mixer_width = MixerPanel::width(view.tracks.len()).min(lower.width / 2);
    let [mixer_area, tracker_area] =
        Layout::horizontal([Constraint::Length(mixer_width), Constraint::Min(0)]).areas(lower);
    frame.render_widget(MixerPanel(view), mixer_area);
    frame.render_widget(FullTrackerPanel(view), tracker_area);
    frame.render_widget(FxPanel(view), fx);
    frame.render_widget(HelpLine(view), help);
    draw_popups(frame, view, None);
}

/// Tabla de choques de frecuencia del último análisis (la de
/// `--analyze-masking`), en la ventana principal sin editor: una fila por
/// pareja y una columna por banda que choca, con la puntuación en rojo si
/// es fuerte, en amarillo si es media y normal si es leve (`·` suenan las
/// dos sin chocar, `-` banda vacía), y en el acento las parejas de la
/// pista seleccionada. Si la tabla no cabe, la lista de choques del más
/// fuerte al más débil. Si se ha cargado otra cosa después, lo avisa.
pub struct ClashPanel<'a>(pub &'a ViewModel);

/// Ancho de una columna de banda (` 1.25k`).
const CLASH_COLUMN: u16 = 6;
/// Ancho mínimo del panel de choques y lo que se deja como mínimo al
/// sample a su izquierda.
const CLASH_MIN: u16 = 28;
const SAMPLE_BESIDE_CLASHES: u16 = 40;

impl ClashPanel<'_> {
    /// Ancho que pide la tabla (con el borde) en una fila de `room` de
    /// ancho: la tabla entera si cabe en dos quintos, si no la lista; 0 si
    /// no queda sitio para ella y el sample.
    pub fn width(table: &ClashTableView, room: u16) -> u16 {
        let most = room * 2 / 5;
        if most < CLASH_MIN || room - most < SAMPLE_BESIDE_CLASHES {
            return 0;
        }
        let grid = Self::names_width(table) + CLASH_COLUMN * table.bands.len() as u16 + 2;
        let wanted = if grid <= most {
            grid
        } else {
            Self::list_width(table) + 2
        };
        wanted.clamp(CLASH_MIN, most)
    }

    fn pair(a: &str, b: &str) -> String {
        format!("{a} × {b}")
    }

    fn names_width(table: &ClashTableView) -> u16 {
        table
            .rows
            .iter()
            .map(|r| Self::pair(&r.a, &r.b).chars().count() as u16)
            .max()
            .unwrap_or(0)
    }

    /// Ancho de una línea de la lista: pareja, banda y puntuación.
    fn list_width(table: &ClashTableView) -> u16 {
        table
            .clashes
            .iter()
            .map(|(a, b, band, _)| {
                (Self::pair(a, b).chars().count() + 2 + band.chars().count() + 3 + 2 + 4) as u16
            })
            .max()
            .unwrap_or(0)
    }
}

/// Color de la puntuación de un choque según lo fuerte que es.
fn clash_style(score: f32) -> Style {
    match script::masking::strength(score) {
        "fuerte" => Style::new().fg(error_color()).bold(),
        "medio" => Style::new().fg(Color::Yellow),
        _ => Style::new(),
    }
}

impl Widget for ClashPanel<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let view = self.0;
        let Some(table) = &view.clash_table else {
            return;
        };
        let border = if table.stale {
            Style::new().fg(warning_color())
        } else {
            Style::new()
        };
        let block = Block::bordered()
            .border_style(border)
            .title(format!(" choques · {} ", script::bars_text(table.bars)));
        let mut inner = block.inner(area);
        block.render(area, buf);
        if inner.height == 0 || inner.width == 0 {
            return;
        }
        // Abajo, el aviso de que es de antes de la última carga.
        if table.stale {
            let y = inner.y + inner.height - 1;
            buf.set_stringn(
                inner.x,
                y,
                "⚠ de una carga anterior",
                inner.width as usize,
                Style::new().fg(warning_color()),
            );
            inner.height -= 1;
        }
        if table.rows.is_empty() {
            buf.set_stringn(
                inner.x,
                inner.y,
                format!("sin choques entre {} pistas", table.tracks),
                inner.width as usize,
                Style::new().fg(dim()),
            );
            return;
        }
        let selected = view
            .selected
            .and_then(|i| view.tracks.get(i))
            .map(|t| t.name.as_str());
        let name_style = |a: &str, b: &str| {
            if selected.is_some_and(|s| s == a || s == b) {
                Style::new().fg(accent()).bold()
            } else {
                Style::new()
            }
        };
        let names = ClashPanel::names_width(table);
        let grid = names + CLASH_COLUMN * table.bands.len() as u16;
        let width = inner.width as usize;
        if grid <= inner.width && table.rows.len() < inner.height as usize {
            // La tabla: cabecera con las bandas y una fila por pareja.
            for (i, band) in table.bands.iter().enumerate() {
                let x = inner.x + names + i as u16 * CLASH_COLUMN;
                buf.set_string(x, inner.y, format!(" {band:>5}"), Style::new().fg(dim()));
            }
            for (row, y) in table.rows.iter().zip(inner.y + 1..) {
                buf.set_string(
                    inner.x,
                    y,
                    ClashPanel::pair(&row.a, &row.b),
                    name_style(&row.a, &row.b),
                );
                for (i, cell) in row.cells.iter().enumerate() {
                    let x = inner.x + names + i as u16 * CLASH_COLUMN;
                    let style = match cell {
                        script::masking::Cell::Clash(score) => clash_style(*score),
                        _ => Style::new().fg(dim()),
                    };
                    buf.set_string(x, y, format!(" {:>5}", cell.text()), style);
                }
            }
            return;
        }
        // La lista, del más fuerte al más débil; si no caben, cuántos
        // faltan en la última fila.
        let rows = inner.height as usize;
        let shown = if table.clashes.len() > rows {
            rows.saturating_sub(1)
        } else {
            table.clashes.len()
        };
        let pair_width = table
            .clashes
            .iter()
            .map(|(a, b, _, _)| ClashPanel::pair(a, b).chars().count())
            .max()
            .unwrap_or(0);
        for ((a, b, band, score), y) in table.clashes.iter().take(shown).zip(inner.y..) {
            let pair = ClashPanel::pair(a, b);
            let line = Line::from(vec![
                Span::styled(format!("{pair:<pair_width$}"), name_style(a, b)),
                Span::styled(format!("  {band:>5} Hz  "), Style::new().fg(dim())),
                Span::styled(script::masking::score_text(*score), clash_style(*score)),
            ]);
            buf.set_line(inner.x, y, &line, inner.width);
        }
        if shown < table.clashes.len() && rows > 0 {
            buf.set_stringn(
                inner.x,
                inner.y + shown as u16,
                format!("… y {} más", table.clashes.len() - shown),
                width,
                Style::new().fg(dim()),
            );
        }
    }
}

/// Línea de arriba de la ventana principal sin editor: el transporte (como
/// en el panel del motor) y, a la derecha, el archivo y el enlace.
pub struct StatusLine<'a>(pub &'a ViewModel);

impl Widget for StatusLine<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let view = self.0;
        transport_line(view).render(area, buf);
        let dirty = if view.dirty { " ●" } else { "" };
        let right = format!(
            "{}{dirty} · {} ",
            view.file,
            view.link.as_deref().unwrap_or_default()
        );
        let width = right.chars().count() as u16;
        if width < area.width {
            let x = area.x + area.width - width;
            buf.set_string(x, area.y, right, Style::new().fg(dim()));
        }
    }
}

/// Dibuja el editor en su propia ventana (`tui --window-editor`): el
/// editor entero, la línea del enlace con la principal y la de ayuda.
pub fn draw_editor_window(frame: &mut Frame, app: &mut App) {
    let view = app.view();
    let [main, link, help] = Layout::vertical([
        Constraint::Min(3),
        Constraint::Length(1),
        Constraint::Length(1),
    ])
    .areas(frame.area());
    let popup = draw_editor(frame, app, &view, main);
    let text = view.link.clone().unwrap_or_default();
    let color = if text.starts_with('✗') {
        warning_color()
    } else {
        accent()
    };
    frame.render_widget(Line::from(text).fg(color), link);
    frame.render_widget(HelpLine(&view), help);
    draw_popups(frame, &view, popup);
}

/// Lo que se abre encima de los paneles.
fn draw_popups(frame: &mut Frame, view: &ViewModel, popup: Option<Rect>) {
    // El autocompletado, encima de todo (puede salirse del editor).
    if let (Some(completion), Some(area)) = (&view.completion, popup) {
        frame.render_widget(CompletionPopup(completion), area);
    }
    if let Some(palette) = &view.palette {
        frame.render_widget(PalettePopup(palette), palette_area(frame.area()));
    }
    if let Some(slots) = &view.slots {
        frame.render_widget(SlotsPopup(slots), slots_area(frame.area()));
    }
    if let Some(browser) = &view.browser {
        frame.render_widget(BrowserPopup(browser), palette_area(frame.area()));
    }
    if let Some(picker) = &view.block_picker {
        frame.render_widget(BlockPickerPopup(picker), picker_area(frame.area(), picker));
    }
    if let Some(picker) = &view.versions {
        frame.render_widget(BlockPickerPopup(picker), picker_area(frame.area(), picker));
    }
    if let Some(picker) = &view.masking {
        frame.render_widget(BlockPickerPopup(picker), picker_area(frame.area(), picker));
    }
}

/// Sitio de las ranuras: centradas arriba, una fila por ranura.
pub fn slots_area(screen: Rect) -> Rect {
    let width = screen.width.saturating_sub(4).min(72);
    let height = (crate::app::SLOTS as u16 + 2).min(screen.height);
    Rect {
        x: screen.x + (screen.width - width) / 2,
        y: screen.y + (screen.height - height).min(2),
        width,
        height,
    }
}

/// Ranuras rápidas: número, cuántas líneas tiene y la primera con texto.
pub struct SlotsPopup<'a>(pub &'a SlotsView);

impl Widget for SlotsPopup<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let view = self.0;
        Clear.render(area, buf);
        let block = Block::bordered()
            .border_style(Style::new().fg(accent()))
            .title(format!(" {} · 1-9 · Esc ", view.title));
        let lines: Vec<Line> = view
            .items
            .iter()
            .enumerate()
            .map(|(i, slot)| {
                let number = Span::from(format!(" {} ", i + 1)).fg(accent()).bold();
                match slot {
                    None => Line::from(vec![number, Span::from("vacía").fg(dim())]),
                    Some(lines) => {
                        let first = lines.iter().find(|l| !l.trim().is_empty());
                        // Cuántas líneas, delante (la primera puede no caber).
                        let count = match lines.len() {
                            0 | 1 => String::new(),
                            n => format!("{n} líneas · "),
                        };
                        Line::from(vec![
                            number,
                            Span::from(count).fg(dim()),
                            Span::raw(first.map_or("", |l| l.trim()).to_string()),
                        ])
                    }
                }
            })
            .collect();
        Paragraph::new(lines).block(block).render(area, buf);
    }
}

/// Explorador de samples: lo escrito, los WAV que coinciden y lo que se sabe
/// del elegido.
pub struct BrowserPopup<'a>(pub &'a BrowserView);

impl Widget for BrowserPopup<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let view = self.0;
        Clear.render(area, buf);
        let block = Block::bordered()
            .border_style(Style::new().fg(accent()))
            .title(format!(" samples · {} · ↑↓ Tab Enter Esc ", view.folder));
        let inner = block.inner(area);
        block.render(area, buf);
        if inner.height < 4 {
            return;
        }
        let width = inner.width as usize;
        let mut lines = vec![
            Line::from(vec![
                Span::from("› ").fg(accent()).bold(),
                Span::raw(format!("{}█", view.query)),
            ]),
            Line::from("─".repeat(width)).fg(dim()),
        ];
        let rows = (inner.height as usize).saturating_sub(4);
        let first = view.selected.saturating_sub(rows.saturating_sub(1));
        if view.items.is_empty() {
            lines.push(Line::from(" ningún WAV coincide").fg(dim()));
        }
        for (i, name) in view.items.iter().enumerate().skip(first).take(rows) {
            let line = Line::raw(format!(" {name}"));
            lines.push(if i == view.selected {
                line.style(Style::new().fg(Color::Black).bg(accent()))
            } else {
                line
            });
        }
        while lines.len() < rows + 2 {
            lines.push(Line::raw(""));
        }
        lines.push(Line::from("─".repeat(width)).fg(dim()));
        lines.push(Line::from(view.info.clone()).fg(dim()));
        Paragraph::new(lines).render(inner, buf);
    }
}

/// Filas de bloques visibles en el selector.
const PICKER_ROWS: u16 = 8;

/// Líneas de vista previa que se intentan mostrar en el selector.
const PICKER_PREVIEW: usize = 16;

/// Sitio del selector de bloques: centrado arriba, como mucho 90 columnas;
/// de alto, la lista y la vista previa del elegido (hasta
/// [`PICKER_PREVIEW`] líneas), sin salirse de la pantalla.
pub fn picker_area(screen: Rect, view: &BlockPickerView) -> Rect {
    let width = screen.width.saturating_sub(4).min(90);
    let preview = view.preview.len().clamp(3, PICKER_PREVIEW) as u16;
    let confirm = u16::from(!view.confirm.is_empty());
    let height = (PICKER_ROWS + preview + confirm + 6).min(screen.height.saturating_sub(2));
    Rect {
        x: screen.x + (screen.width - width) / 2,
        y: screen.y + (screen.height - height).min(1),
        width,
        height,
    }
}

/// Selector de bloques: lo escrito, los que coinciden (con su carpeta y
/// sus etiquetas), la descripción y el texto del elegido.
pub struct BlockPickerPopup<'a>(pub &'a BlockPickerView);

impl Widget for BlockPickerPopup<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let view = self.0;
        Clear.render(area, buf);
        let block = Block::bordered()
            .border_style(Style::new().fg(accent()))
            .title(format!(" {} · ↑↓ Enter Esc ", view.title));
        let inner = block.inner(area);
        block.render(area, buf);
        if inner.height < 4 {
            return;
        }
        let width = inner.width as usize;
        let mut lines = vec![
            Line::from(vec![
                Span::from("› ").fg(accent()).bold(),
                Span::raw(format!("{}█", view.query)),
            ]),
            Line::from("─".repeat(width)).fg(dim()),
        ];
        let rows = (PICKER_ROWS as usize).min(inner.height as usize - 3);
        let first = view.selected.saturating_sub(rows.saturating_sub(1));
        if view.items.is_empty() {
            lines.push(Line::from(" ningún bloque coincide").fg(dim()));
        }
        for (i, (name, scope, tags)) in view.items.iter().enumerate().skip(first).take(rows) {
            let detail = format!("{tags} {scope} ");
            let pad = width.saturating_sub(name.chars().count() + detail.chars().count() + 1);
            let line = Line::from(vec![
                Span::raw(format!(" {name}{}", " ".repeat(pad))),
                Span::from(detail).fg(dim()),
            ]);
            lines.push(if i == view.selected {
                line.style(Style::new().fg(Color::Black).bg(accent()))
            } else {
                line
            });
        }
        while lines.len() < rows + 2 {
            lines.push(Line::raw(""));
        }
        lines.push(Line::from("─".repeat(width)).fg(dim()));
        if !view.confirm.is_empty() {
            lines.push(Line::from(view.confirm.clone()).fg(Color::Yellow).bold());
        }
        if !view.description.is_empty() {
            lines.push(Line::from(view.description.clone()).italic());
        }
        let room = (inner.height as usize).saturating_sub(lines.len());
        let shown = view.preview.len().min(room);
        for (i, text) in view.preview.iter().take(shown).enumerate() {
            if i + 1 == shown && shown < view.preview.len() {
                lines.push(
                    Line::from(format!("… ({} líneas más)", view.preview.len() - i)).fg(dim()),
                );
            } else {
                lines.push(Line::from(text.clone()).fg(Color::Cyan));
            }
        }
        Paragraph::new(lines).render(inner, buf);
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
            .border_style(Style::new().fg(accent()))
            .title(" paleta de comandos · ↑↓ Enter Esc ");
        let inner = block.inner(area);
        block.render(area, buf);
        if inner.height < 3 {
            return;
        }
        let width = inner.width as usize;
        let mut lines = vec![
            Line::from(vec![
                Span::from("› ").fg(accent()).bold(),
                Span::raw(format!("{}█", view.query)),
            ]),
            Line::from("─".repeat(width)).fg(dim()),
        ];
        let rows = (inner.height as usize)
            .saturating_sub(4)
            .min(PALETTE_ROWS as usize);
        let first = view.selected.saturating_sub(rows.saturating_sub(1));
        if view.items.is_empty() {
            lines.push(Line::from(" ninguna acción coincide").fg(dim()));
        }
        for (i, (name, keys)) in view.items.iter().enumerate().skip(first).take(rows) {
            let pad = width.saturating_sub(name.chars().count() + keys.chars().count() + 2);
            let line = Line::from(vec![
                Span::raw(format!(" {name}{}", " ".repeat(pad))),
                Span::from(format!("{keys} ")).fg(dim()),
            ]);
            lines.push(if i == view.selected {
                line.style(Style::new().fg(Color::Black).bg(accent()))
            } else {
                line
            });
        }
        while lines.len() < rows + 2 {
            lines.push(Line::raw(""));
        }
        lines.push(Line::from("─".repeat(width)).fg(dim()));
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
    let link = view
        .link
        .as_ref()
        .filter(|_| view.playing_mode)
        .map_or(String::new(), |l| format!(" · {l}"));
    let mut block = Block::bordered().title(format!(" {}{dirty}{project}{link} ", view.file));
    if view.external_change {
        block = block.title(
            Line::from(" ⚠ cambiado fuera · F3 recargar ")
                .fg(Color::Black)
                .bg(Color::Yellow),
        );
    }
    if let Some(mode) = view.vim.filter(|_| !view.playing_mode) {
        block = block.title(
            Line::from(format!(" {mode} "))
                .fg(Color::Black)
                .bg(accent()),
        );
    }
    if view.playing_mode {
        // (En la principal con el editor fuera no se vuelve al editor.)
        let esc = if view.link.is_some() { "" } else { " · Esc" };
        block = block.title(
            Line::from(format!(" TOCAR · 1-0 q-i{esc} "))
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
            .border_style(Style::new().fg(error_color()))
            .title_bottom(Line::from(summary).fg(error_color()));
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
                .fg(error_color())
                .block(
                    Block::new()
                        .borders(Borders::TOP)
                        .border_style(Style::new().fg(error_color())),
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
                    Span::from(format!("ⓘ {}", help.signature)).fg(accent()),
                    Span::from(format!("  {}", help.description)).fg(dim()),
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
                Paragraph::new(text)
                    .wrap(Wrap { trim: false })
                    .fg(warning_color()),
                warning,
            );
            editor
        }
        None => editor_area,
    };
    app.editor.set_line_number_style(Style::new().fg(dim()));
    app.editor
        .set_cursor_line_style(Style::new().add_modifier(Modifier::UNDERLINED));
    app.editor
        .set_search_style(Style::new().fg(Color::Black).bg(warning_color()));
    frame.render_widget(&app.editor, editor_area);
    // Dónde quedó el texto: lo que necesita el ratón para saber qué se pulsa.
    app.editor_area = Some(editor_area);

    // Celdas de unas columnas de una fila del texto, si están a la vista.
    let (top, left) = app.editor.scroll_offset();
    let text_x = editor_area.x + app.editor.line_number_width();
    let cells = |row: usize, cols: &std::ops::Range<usize>| {
        let y = row
            .checked_sub(top as usize)
            .filter(|r| *r < editor_area.height as usize)?;
        let from = cols
            .start
            .saturating_sub(left as usize)
            .min(u16::MAX as usize) as u16;
        let to = cols
            .end
            .saturating_sub(left as usize)
            .min(u16::MAX as usize) as u16;
        let x = text_x.saturating_add(from).min(editor_area.right());
        let width = text_x
            .saturating_add(to)
            .min(editor_area.right())
            .saturating_sub(x);
        (width > 0).then(|| Rect::new(x, editor_area.y + y as u16, width, 1))
    };
    // Lo que suena, sobre el propio texto (en orden: el último manda).
    for h in &view.highlights {
        if let Some(area) = cells(h.row, &h.cols) {
            frame.buffer_mut().set_style(area, highlight_style(h));
        }
    }
    // Subrayar los pasos con error de `steps("...")` que estén a la vista.
    for d in &view.diagnostics {
        if let Some(area) = cells(d.row, &d.cols) {
            frame.buffer_mut().set_style(
                area,
                Style::new()
                    .fg(diagnostic())
                    .underline_color(diagnostic())
                    .add_modifier(Modifier::UNDERLINED),
            );
        }
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
                Style::new().bg(error_line()),
            );
        }
    }
    popup
}

/// Estilo de un token que suena, según `[resaltado] estilo`, con el color
/// de su pista. Con fondo: la pista seleccionada, con el color entero, el
/// texto en negro y en negrita; las demás, con el color apagado. Subrayado:
/// la seleccionada, también con el texto de su color y en negrita.
/// Invertido: el texto de la seleccionada en su color (de fondo al
/// invertirse) y en negrita; el de las demás, apagado.
pub fn highlight_style(h: &crate::highlight::Highlight) -> Style {
    use theme::HighlightMode;
    let theme = theme::get();
    let full = theme.highlight[h.color % theme::HIGHLIGHT_COLORS];
    let dim = theme.highlight_dim[h.color % theme::HIGHLIGHT_COLORS];
    let bold = if h.selected {
        Modifier::BOLD
    } else {
        Modifier::empty()
    };
    match theme.highlight_mode {
        HighlightMode::Background if h.selected => {
            Style::new().bg(full).fg(Color::Black).add_modifier(bold)
        }
        HighlightMode::Background => Style::new().bg(dim),
        HighlightMode::Underline => {
            let style = Style::new()
                .underline_color(full)
                .add_modifier(Modifier::UNDERLINED | bold);
            if h.selected { style.fg(full) } else { style }
        }
        HighlightMode::Reversed => Style::new()
            .fg(if h.selected { full } else { dim })
            .add_modifier(Modifier::REVERSED | bold),
    }
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
            .border_style(Style::new().fg(accent()))
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
                Span::from(format!("{kind} ")).fg(dim()),
            ]);
            lines.push(if i == view.selected {
                line.style(Style::new().fg(Color::Black).bg(accent()))
            } else {
                line
            });
        }
        lines.push(Line::from("─".repeat(width)).fg(dim()));
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

/// Línea del master: medidor (antes del limitador), nivel en dB y avisos.
fn master_line(master: &crate::model::MasterView) -> Line<'static> {
    let db = master
        .db()
        .map_or("  -∞ dB".to_string(), |db| format!("{db:+5.1} dB"));
    let color = if master.clipping {
        error_color()
    } else if master.limiting {
        Color::Yellow
    } else {
        meter_color()
    };
    let mut spans = vec![
        Span::from(format!(" {:<10}", "master")).bold(),
        Span::from(meter(master.level, 8)).fg(color),
        Span::raw(format!(" {db}")),
    ];
    if master.limiting {
        spans.push(Span::raw(" "));
        spans.push(Span::from(" LIMITADOR ").fg(Color::Black).bg(Color::Yellow));
    }
    if master.clipping {
        spans.push(Span::raw(" "));
        spans.push(Span::from(" SATURA ").fg(Color::Black).bg(error_color()));
    }
    Line::from(spans)
}

/// Transporte: marcha, tempo, compás y paso, metrónomo, grabación y
/// variante.
fn transport_line(view: &ViewModel) -> Line<'static> {
    let t = view.transport;
    let state = if t.running {
        Span::from("▶ en marcha").fg(meter_color())
    } else {
        Span::from("■ parado").fg(dim())
    };
    let step = t.step.map_or("--".into(), |s| format!("{s:02}"));
    Line::from(vec![
        state,
        Span::raw(format!(
            "   {:.1} BPM   compás {}   paso {step}",
            t.bpm, t.bar
        )),
        match &view.cue {
            Some(cue) => Span::from(format!("   ⏮ cue {cue}")).fg(accent()),
            None => Span::raw(""),
        },
        if view.metronome {
            Span::from("   ♩ clic").fg(Color::Yellow)
        } else {
            Span::raw("")
        },
        match &view.recording {
            Some(rec) => Span::from(format!("   ● {rec}")).fg(error_color()).bold(),
            None => Span::raw(""),
        },
        match view.variant {
            Some(letter) => Span::from(format!("   variante {letter}"))
                .fg(Color::Black)
                .bg(accent()),
            None => Span::raw(""),
        },
    ])
}

/// Estado del motor: transporte y pistas.
/// La línea de tiempo en una fila: cada entrada con su nombre y sus
/// compases (la que suena, resaltada y con el compás por el que va: `drop
/// 3/8`; `⟳` si se repite). Si no cabe, empieza más adelante para que se
/// vea la que suena.
pub struct TimelineBar<'a>(pub &'a TimelineView);

impl TimelineBar<'_> {
    /// El texto de cada entrada.
    pub fn labels(view: &TimelineView) -> Vec<String> {
        view.entries
            .iter()
            .enumerate()
            .map(|(i, (name, bars))| {
                // El cue, delante de su entrada.
                let cue = if view.cue == Some(i) { "⏮ " } else { "" };
                if i == view.current {
                    let mark = if view.looping { " ⟳" } else { "" };
                    format!("{cue}▶ {name} {}/{bars}{mark}", view.bar + 1)
                } else {
                    format!("{cue}{name} {bars}")
                }
            })
            .collect()
    }
}

impl Widget for TimelineBar<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let view = self.0;
        let labels = Self::labels(view);
        // Desde dónde: la primera entrada que deja ver entera la que suena
        // (cada separador ocupa 3; el «… » del principio, 2).
        let upto = |from: usize| -> usize {
            let prefix = if from > 0 { 2 } else { 0 };
            prefix
                + labels[from..=view.current.min(labels.len() - 1)]
                    .iter()
                    .map(|l| l.chars().count() + 3)
                    .sum::<usize>()
        };
        let room = area.width as usize;
        let mut first = 0;
        while first < view.current && upto(first) > room {
            first += 1;
        }
        let mut spans = Vec::new();
        if first > 0 {
            spans.push(Span::from("… ").fg(dim()));
        }
        for (i, label) in labels.iter().enumerate().skip(first) {
            if i > first {
                spans.push(Span::from(" │ ").fg(dim()));
            }
            let span = Span::from(label.clone());
            spans.push(if i == view.current {
                span.fg(Color::Black).bg(accent())
            } else {
                span
            });
        }
        Line::from(spans).render(area, buf);
    }
}

pub struct EnginePanel<'a>(pub &'a ViewModel);

impl Widget for EnginePanel<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let view = self.0;
        let mut lines = vec![transport_line(view), master_line(&view.master)];
        if view.tracks.is_empty() {
            lines.push(Line::from("sin pistas").fg(dim()));
        }
        for (i, track) in view.tracks.iter().enumerate() {
            let selected = view.selected == Some(i);
            let marker = if selected { "›" } else { " " };
            let slice = track
                .playing_slice
                .map_or("--".into(), |s| format!("{:02}", s + 1));
            let name = Span::from(format!("{marker}{:<10}", track.name));
            let name = if selected {
                name.fg(accent()).bold()
            } else {
                name
            };
            lines.push(Line::from(vec![
                name,
                Span::from(meter(track.level, 8)).fg(meter_color()),
                Span::raw(format!(
                    " ▶{slice}  {}  ciclo {}{}  pista {}",
                    // Un sintetizador no tiene cortes: su tipo.
                    track.synth.as_ref().map_or_else(
                        || format!("{} cortes", track.slices.len()),
                        |s| format!("synth {}", s.kind)
                    ),
                    track.cycle,
                    // Longitud distinta del compás: polirritmia.
                    if track.length == engine::STEPS || track.length == 0 {
                        String::new()
                    } else {
                        format!("  {} pasos", track.length)
                    },
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
        } else if self.0.slots.is_some() {
            "RANURA: pulsa 1-9 · cualquier otra tecla cancela"
        } else if self.0.browser.is_some() {
            "SAMPLES: escribe para filtrar · ↑↓ elegir y preescuchar · Tab preescuchar · Enter insertar · Esc cerrar"
        } else if self.0.block_picker.is_some() {
            "BLOQUES: escribe para filtrar (nombre, #etiqueta, descripción) · ↑↓ elegir · \
             Enter · Esc cerrar"
        } else if self.0.completion.is_some() {
            "COMPLETAR: Tab aceptar · ↑↓ elegir y Enter aceptar · Esc cerrar · Enter sin elegir: salto de línea"
        } else if !self.0.key_help.is_empty() {
            self.0.key_help.as_str()
        } else if self.0.playing_mode && self.0.editor_outside {
            "TOCAR: 1-0 q-i cortes · ←→ preescucha · m mute · s solo · Ctrl-O pista · Ctrl-Espacio marcha/paro · F1 paleta"
        } else if self.0.playing_mode {
            "TOCAR: 1-0 q-i cortes · ←→ preescucha · m mute · s solo · Ctrl-O pista · Ctrl-Espacio marcha/paro · F1 paleta · Esc editor"
        } else {
            "Ctrl-S guardar y cargar · Ctrl-L cargar · Ctrl-Espacio marcha/paro · \
             Ctrl-G ir al error · Ctrl-O pista · Esc tocar · F1 paleta · F2 órdenes · \
             F4 completar · Ctrl-Q salir"
        };
        if let Some(prompt) = &self.0.prompt {
            Line::from(vec![
                Span::from("orden> ").fg(accent()).bold(),
                Span::raw(format!("{prompt}█  ")),
                Span::from(
                    "exportar [archivo.wav] [compases] · guardar [proyecto] · \
                     abrir proyecto · muestras carpeta · etiquetar corte nombre · bpm N · \
                     bloque nombre [etiquetas] [-- descripción] · remuestrear [pista] · Esc cancelar",
                )
                .fg(dim()),
            ])
            .render(area, buf);
            return;
        }
        let message = &self.0.message;
        let line = if message.is_empty() {
            Line::from(keys).fg(dim())
        } else {
            Line::from(vec![
                Span::raw(format!("{message}  ")),
                Span::from(keys).fg(dim()),
            ])
        };
        line.render(area, buf);
    }
}

/// Forma de onda del sample de la pista seleccionada, en barras (una por
/// columna), con los cortes marcados y el que está sonando resaltado.
pub struct SamplePanel<'a>(pub &'a ViewModel);

impl Widget for SamplePanel<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let view = self.0;
        let Some(track) = view.selected.and_then(|i| view.tracks.get(i)) else {
            Paragraph::new(Line::from("sin pistas").fg(dim()))
                .block(Block::bordered().title(" sample "))
                .render(area, buf);
            return;
        };
        if let Some(synth) = &track.synth {
            synth_panel(track, synth, area, buf);
            return;
        }
        let block = Block::bordered().title(sample_title(view, track));
        let inner = block.inner(area);
        block.render(area, buf);
        if inner.width < 2 || inner.height < 2 || track.sample_frames == 0 {
            return;
        }
        draw_wave(track, inner, buf);
        // Los números, en lo alto de las marcas (encima de la onda).
        slice_numbers(track, view.selected_slice, false, inner, buf);
    }
}

/// Un sintetizador no tiene sample: en su lugar, su tipo y sus
/// parámetros (`∿` en los que se mueven).
fn synth_panel(
    track: &crate::model::TrackView,
    synth: &crate::model::SynthView,
    area: Rect,
    buf: &mut Buffer,
) {
    let title = format!(" sintetizador · {} · {} ", track.name, synth.kind);
    Paragraph::new(Line::from(synth.params.clone()).fg(dim()))
        .wrap(Wrap { trim: true })
        .block(Block::bordered().title(title))
        .render(area, buf);
}

/// Título del panel del sample: pista, archivo, duración, cortes y el
/// corte elegido.
fn sample_title(view: &ViewModel, track: &crate::model::TrackView) -> String {
    let seconds = track.sample_frames as f64 / track.sample_rate.max(1) as f64;
    // Con stretch, la posición va en ms del sample original (la de los
    // ajustes y del proyecto) y se dice de qué tempo a cuál.
    let chosen = view.selected_slice.map_or(String::new(), |i| {
        format!(
            " · corte {} {:.1} ms",
            slice_mark(track, i),
            slice_ms(track, i)
        )
    });
    let stretch = track.stretch.map_or(String::new(), |(source, ratio)| {
        let round = |b: f64| ((b * 10.0).round() / 10.0).to_string();
        format!(" · {}→{} BPM", round(source), round(source / ratio))
    });
    format!(
        " sample · {} · {}{stretch} · {seconds:.2} s · {} cortes{chosen} ",
        track.name,
        track.sample_name,
        track.slices.len()
    )
}

/// Dónde empieza el corte `i`, en ms del sample original.
fn slice_ms(track: &crate::model::TrackView, i: usize) -> f64 {
    let ratio = track.stretch.map_or(1.0, |(_, r)| r);
    track.slices.get(i).map_or(0.0, |r| {
        r.start as f64 * 1000.0 / track.sample_rate.max(1) as f64 / ratio
    })
}

/// Columna (desde 0, dentro de `width`) donde empieza el corte `i`.
fn slice_column(track: &crate::model::TrackView, i: usize, width: u16) -> u16 {
    let frames = track.sample_frames.max(1) as f64;
    let start = track.slices.get(i).map_or(0, |r| r.start);
    (start as f64 / frames * width as f64).floor() as u16
}

/// La forma de onda en barras (una por columna, de lo más bajo a lo más
/// alto de su trozo, con octavos de celda en los bordes), las marcas de
/// los cortes y el corte que suena resaltado.
fn draw_wave(track: &crate::model::TrackView, inner: Rect, buf: &mut Buffer) {
    let columns = inner.width as usize;
    let frames = track.sample_frames as f64;
    let playing = track
        .playing_slice
        .and_then(|i| track.slices.get(i))
        .map(|r| (r.start as f64 / frames, r.end as f64 / frames));
    let marks: Vec<usize> = track
        .slices
        .iter()
        .map(|slice| (slice.start as f64 / frames * columns as f64).floor() as usize)
        .collect();
    let wave = &track.waveform;
    for x in 0..columns {
        let from = x as f64 / columns as f64;
        let to = (x + 1) as f64 / columns as f64;
        let (lo, hi) = wave.range(from, to);
        let lit = playing.is_some_and(|(a, b)| from < b && to > a);
        let mark = marks.contains(&x);
        let color = if mark {
            slice_color()
        } else if lit {
            accent()
        } else {
            wave_color()
        };
        // Desde el centro hasta el pico, aunque el trozo no cruce el 0.
        let cells = wave_bar(lo.min(0.0), hi.max(0.0), inner.height);
        for (y, cell) in cells.into_iter().enumerate() {
            let symbol = match cell {
                Some(c) => c,
                // La marca de un corte cruza el panel entero.
                None if mark => '│',
                None => continue,
            };
            buf[(inner.x + x as u16, inner.y + y as u16)]
                .set_char(symbol)
                .set_fg(color);
        }
    }
}

/// Número de cada corte (desde 1) en la primera fila de `area`, en la
/// columna de su marca, con su etiqueta si la tiene (`1 kick`; la marca
/// siguiente la puede tapar). El seleccionado, encima y resaltado; con
/// `playing`, el que suena también.
fn slice_numbers(
    track: &crate::model::TrackView,
    selected: Option<usize>,
    playing: bool,
    area: Rect,
    buf: &mut Buffer,
) {
    let put = |i: usize, style: Style, buf: &mut Buffer| {
        let column = slice_column(track, i, area.width);
        if column < area.width {
            let text = slice_mark(track, i);
            let room = (area.width - column) as usize;
            buf.set_string(
                area.x + column,
                area.y,
                text.chars().take(room).collect::<String>(),
                style,
            );
        }
    };
    for i in 0..track.slices.len() {
        put(i, Style::new().fg(slice_color()).bold(), buf);
    }
    if playing && let Some(i) = track.playing_slice.filter(|&i| i < track.slices.len()) {
        put(i, Style::new().fg(Color::Black).bg(accent()).bold(), buf);
    }
    if let Some(i) = selected.filter(|&i| i < track.slices.len()) {
        put(
            i,
            Style::new().fg(Color::Black).bg(slice_color()).bold(),
            buf,
        );
    }
}

/// La forma de onda grande de la ventana principal sin editor: una fila
/// con los números de los cortes (el que suena y el elegido resaltados),
/// la onda debajo, sin números encima, y al pie dónde empieza cada corte
/// (en ms, los que quepan sin pisarse).
pub struct BigSamplePanel<'a>(pub &'a ViewModel);

impl Widget for BigSamplePanel<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let view = self.0;
        let Some(track) = view.selected.and_then(|i| view.tracks.get(i)) else {
            Paragraph::new(Line::from("sin pistas").fg(dim()))
                .block(Block::bordered().title(" sample "))
                .render(area, buf);
            return;
        };
        if let Some(synth) = &track.synth {
            synth_panel(track, synth, area, buf);
            return;
        }
        let block = Block::bordered().title(sample_title(view, track));
        let inner = block.inner(area);
        block.render(area, buf);
        if inner.width < 2 || track.sample_frames == 0 {
            return;
        }
        // Poco alto: como el panel pequeño.
        if inner.height < 5 {
            if inner.height >= 2 {
                draw_wave(track, inner, buf);
                slice_numbers(track, view.selected_slice, true, inner, buf);
            }
            return;
        }
        let [numbers, wave, times] = Layout::vertical([
            Constraint::Length(1),
            Constraint::Min(1),
            Constraint::Length(1),
        ])
        .areas(inner);
        slice_numbers(track, view.selected_slice, true, numbers, buf);
        draw_wave(track, wave, buf);
        // Al pie, los ms de cada corte que quepan (sin pisar al anterior).
        let mut free = 0u16;
        for i in 0..track.slices.len() {
            let column = slice_column(track, i, times.width);
            let text = format!("{:.0}", slice_ms(track, i));
            let width = text.chars().count() as u16;
            if column < free || column + width > times.width {
                continue;
            }
            let style = if view.selected_slice == Some(i) {
                Style::new().fg(slice_color()).bold()
            } else {
                Style::new().fg(dim())
            };
            buf.set_string(times.x + column, times.y, text, style);
            free = column + width + 1;
        }
    }
}

/// Celdas de una barra de la forma de onda que va de `lo` a `hi` (en
/// [-1, 1], arriba el 1) en una columna de `rows` filas: `None` donde no
/// llega. Los bordes van con octavos de celda (`▁`…`▇` por abajo de la
/// celda; por arriba solo hay `▔` y `▀`); una barra que no llena ni un
/// octavo (silencio) es una raya en el centro.
pub fn wave_bar(lo: f32, hi: f32, rows: u16) -> Vec<Option<char>> {
    const LOWER: [char; 8] = ['▁', '▂', '▃', '▄', '▅', '▆', '▇', '█'];
    let eighths = rows as f32 * 8.0;
    // Octavos desde arriba: el 1 es 0 y el −1, `eighths`.
    let to_eighths = |v: f32| ((1.0 - v.clamp(-1.0, 1.0)) / 2.0 * eighths).round() as i32;
    let (top, bottom) = (to_eighths(hi.max(lo)), to_eighths(lo.min(hi)));
    let mut cells = vec![None; rows as usize];
    if bottom - top < 1 {
        let centre = (rows as usize).saturating_sub(1) / 2;
        if let Some(cell) = cells.get_mut(centre) {
            *cell = Some('─');
        }
        return cells;
    }
    for (row, cell) in cells.iter_mut().enumerate() {
        let (cell_top, cell_bottom) = (row as i32 * 8, row as i32 * 8 + 8);
        let a = top.max(cell_top);
        let b = bottom.min(cell_bottom);
        if b <= a {
            continue;
        }
        let filled = (b - a) as usize;
        *cell = Some(if filled == 8 {
            '█'
        } else if b == cell_bottom {
            // Llega al fondo de la celda: borde de arriba de la barra.
            LOWER[filled - 1]
        } else if a == cell_top {
            // Sale del techo de la celda: borde de abajo.
            if filled >= 6 {
                '█'
            } else if filled >= 3 {
                '▀'
            } else {
                '▔'
            }
        } else if (a + b) / 2 >= cell_top + 4 {
            '▄'
        } else {
            '▀'
        });
    }
    cells
}

/// Texto de la marca de un corte en la vista del sample: `3` o `3 snare`.
fn slice_mark(track: &crate::model::TrackView, i: usize) -> String {
    // Un `*` marca los cortes ajustados a mano.
    let star = if track.adjusted.get(i).copied().unwrap_or(false) {
        "*"
    } else {
        ""
    };
    match track.label(i) {
        Some(label) => format!("{}{star} {label}", i + 1),
        None => format!("{}{star}", i + 1),
    }
}

/// Ancho de cada columna del tracker.
const TRACKER_COLUMN: usize = 11;

/// Tracker vertical: un paso por fila y una columna por pista, con el paso
/// que suena en cada pista resaltado. Tiene tantas filas como la pista más
/// larga; una pista más corta deja en blanco las que no tiene. Si todas
/// están en la misma fila (las de 16 pasos), se resalta la fila entera.
pub struct TrackerPanel<'a>(pub &'a ViewModel);

impl Widget for TrackerPanel<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let view = self.0;
        let block = Block::bordered().title(format!(" tracker · compás {} ", view.transport.bar));
        let inner = block.inner(area);
        block.render(area, buf);
        render_tracker(view, inner, buf, TRACKER_COLUMN, view.tracks.len());
    }
}

/// Ancho mínimo y máximo de cada columna del tracker con todas las pistas.
const FULL_TRACKER_MIN: usize = 6;
const FULL_TRACKER_MAX: usize = 16;

/// Tracker de la ventana principal sin editor: todas las pistas a la vez,
/// con las columnas tan anchas como quepan (de 6 a 16). Si aun así no
/// caben, el título dice cuántas se ven.
pub struct FullTrackerPanel<'a>(pub &'a ViewModel);

impl FullTrackerPanel<'_> {
    /// Ancho de cada columna y cuántas pistas caben en `width` celdas
    /// (sin el borde).
    pub fn columns(tracks: usize, width: u16) -> (usize, usize) {
        let room = (width as usize).saturating_sub(3);
        if tracks == 0 {
            return (FULL_TRACKER_MAX, 0);
        }
        let column = (room / tracks).clamp(FULL_TRACKER_MIN, FULL_TRACKER_MAX);
        (column, tracks.min(room / column))
    }
}

impl Widget for FullTrackerPanel<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let view = self.0;
        let count = view.tracks.len();
        let (column, shown) = Self::columns(count, area.width.saturating_sub(2));
        // Si no caben todas, cuántas se ven.
        let tracks = if shown < count {
            format!("{shown} de {count} pistas")
        } else {
            format!("{count} pistas")
        };
        let block = Block::bordered().title(format!(
            " tracker · compás {} · {tracks} ",
            view.transport.bar
        ));
        let inner = block.inner(area);
        block.render(area, buf);
        render_tracker(view, inner, buf, column, shown);
    }
}

/// Las filas del tracker: un paso por fila, `shown` pistas en columnas de
/// `column` celdas.
fn render_tracker(view: &ViewModel, inner: Rect, buf: &mut Buffer, column: usize, shown: usize) {
    let t = view.transport;
    if inner.height < 2 {
        return;
    }
    // Paso de cada pista; la fila que se sigue al desplazarse es la de la
    // pista seleccionada (o la primera que suene).
    let positions: Vec<Option<usize>> = view.tracks.iter().map(|t| t.position).collect();
    // Con el cursor de edición, se sigue el cursor.
    let current = view
        .tracker_cursor
        .map(|(_, row)| row)
        .or_else(|| {
            view.selected
                .and_then(|i| positions.get(i).copied().flatten())
        })
        .or_else(|| positions.iter().flatten().next().copied())
        .or(t.step.filter(|_| t.running && view.tracks.is_empty()));
    let rows = view
        .tracks
        .iter()
        .map(|t| t.steps.len())
        .max()
        .unwrap_or(engine::STEPS)
        .max(1);

    // Cabecera con los nombres de las pistas.
    let mut header = vec![Span::raw("   ")];
    for (i, track) in view.tracks.iter().enumerate().take(shown) {
        // Con mute o solo, la marca al final del nombre.
        let mark = match (track.mute, track.solo) {
            (true, _) => " M",
            (false, true) => " S",
            _ => "",
        };
        let name: String = track.name.chars().take(column - 1 - mark.len()).collect();
        let span = Span::from(format!(
            "{:<column$}",
            format!("{name}{mark}"),
            column = column
        ));
        header.push(if !view.audible(i) {
            span.fg(dim())
        } else if view.selected == Some(i) {
            span.fg(accent()).bold()
        } else {
            span.bold()
        });
    }
    Line::from(header).render(Rect { height: 1, ..inner }, buf);

    // Si no caben todas las filas, se desplaza para ver la que suena.
    let visible = (inner.height - 1) as usize;
    let first = if visible >= rows {
        0
    } else {
        current
            .map_or(0, |s| s.saturating_sub(visible / 2))
            .min(rows - visible)
    };
    let playing = Style::new().bg(playing_color()).fg(Color::White).bold();
    for (row, step) in (first..rows).take(visible).enumerate() {
        let here: Vec<bool> = positions.iter().map(|p| *p == Some(step)).collect();
        // Todas las pistas que suenan, en esta fila: la fila entera.
        let whole =
            here.iter().any(|h| *h) && positions.iter().zip(&here).all(|(p, h)| p.is_none() || *h);
        let number = Span::from(format!("{step:02} "));
        let mut spans = vec![if here.iter().any(|h| *h) && !whole {
            number.style(playing)
        } else {
            number.fg(dim())
        }];
        for (i, track) in view.tracks.iter().enumerate().take(shown) {
            let cell = track.steps.get(step);
            // Una fila que alarga (`_`) la nota de antes: `│`.
            let held = cell.is_some_and(Option::is_none) && track.held(step);
            let cell_text: String = cell
                .map_or(String::new(), |c| {
                    if held {
                        " │".into()
                    } else {
                        track.cell_text(c)
                    }
                })
                .chars()
                .take(column - 1)
                .collect();
            let text = format!("{cell_text:<column$}", column = column);
            // Las pistas que no se oyen (mute, o solo de otra), atenuadas.
            let span = if (held || cell.is_some_and(Option::is_some)) && view.audible(i) {
                Span::raw(text)
            } else {
                Span::from(text).fg(dim())
            };
            let cursor = view.tracker_cursor == Some((i, step));
            spans.push(if cursor {
                // El cursor de edición: invertido, encima de lo demás.
                span.add_modifier(Modifier::REVERSED).bold()
            } else if here[i] && !whole {
                span.style(playing)
            } else {
                span
            });
        }
        let mut line = Line::from(spans);
        if whole {
            line = line.style(playing);
        }
        let y = inner.y + 1 + row as u16;
        let row_area = Rect {
            y,
            height: 1,
            ..inner
        };
        if whole {
            buf.set_style(row_area, Style::new().bg(playing_color()));
        }
        line.render(row_area, buf);
    }
}

/// Ancho de cada tira del mezclador.
const STRIP: u16 = 9;

/// Medidor vertical de `rows` filas, de abajo arriba (la primera es la de
/// arriba), con octavos de celda: `level` de 0 a 1.
pub fn vertical_meter(level: f32, rows: u16) -> Vec<char> {
    const LOWER: [char; 8] = ['▁', '▂', '▃', '▄', '▅', '▆', '▇', '█'];
    let eighths = (level.clamp(0.0, 1.0) * rows as f32 * 8.0).round() as u32;
    (0..rows as u32)
        .map(|row| {
            // Octavos que llenan esta fila, contando desde abajo.
            let from_bottom = rows as u32 - 1 - row;
            match eighths.saturating_sub(from_bottom * 8).min(8) {
                0 => ' ',
                n => LOWER[n as usize - 1],
            }
        })
        .collect()
}

/// Texto de un nivel en dB (`-6.0`, `-∞`).
fn db_text(level: f32) -> String {
    if level > 0.0 {
        format!("{:+.1}", 20.0 * level.log10())
    } else {
        "-∞".into()
    }
}

/// Mezclador de la ventana principal sin editor: una tira por pista con
/// su medidor vertical (en rojo si satura), el nivel en dB, el volumen,
/// mute y solo y los envíos, y una más para el master (antes del
/// limitador, con LIM y SAT). Si no caben, el título dice cuántas se ven
/// (`mezclador 6/8`).
pub struct MixerPanel<'a>(pub &'a ViewModel);

impl MixerPanel<'_> {
    /// Ancho que piden `tracks` tiras más la del master (con el borde).
    pub fn width(tracks: usize) -> u16 {
        (tracks as u16 + 1) * STRIP + 2
    }
}

impl Widget for MixerPanel<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let view = self.0;
        let count = view.tracks.len();
        // La del master siempre; las pistas, las que quepan.
        let room = (area.width.saturating_sub(2) / STRIP).saturating_sub(1) as usize;
        let shown = count.min(room);
        let title = if shown < count {
            format!(" mezclador {shown}/{count} ")
        } else {
            " mezclador ".to_string()
        };
        let block = Block::bordered().title(title);
        let inner = block.inner(area);
        block.render(area, buf);
        // Nombre, al menos una fila de medidor, dB, volumen, mute/solo y
        // envíos.
        if inner.height < 6 || inner.width < STRIP {
            return;
        }
        let meter_rows = inner.height - 5;
        let strip = |i: u16| Rect {
            x: inner.x + i * STRIP,
            width: STRIP - 1,
            ..inner
        };
        for (i, track) in view.tracks.iter().enumerate().take(shown) {
            let area = strip(i as u16);
            let audible = view.audible(i);
            let name: String = track.name.chars().take(STRIP as usize - 1).collect();
            let name_style = if view.selected == Some(i) {
                Style::new().fg(accent()).bold()
            } else if audible {
                Style::new().bold()
            } else {
                Style::new().fg(dim())
            };
            buf.set_string(area.x, area.y, name, name_style);
            let color = if track.clipping {
                error_color()
            } else if audible {
                meter_color()
            } else {
                dim()
            };
            draw_vertical_meter(track.level, area, meter_rows, color, buf);
            let below = area.y + 1 + meter_rows;
            buf.set_string(area.x, below, db_text(track.level), Style::new().fg(dim()));
            buf.set_string(
                area.x,
                below + 1,
                format!("vol {:.2}", track.gain),
                Style::new(),
            );
            let flag = |on: bool, bg: Color| {
                if on {
                    Style::new().fg(Color::Black).bg(bg).bold()
                } else {
                    Style::new().fg(dim())
                }
            };
            buf.set_string(area.x, below + 2, "M", flag(track.mute, error_color()));
            buf.set_string(area.x + 2, below + 2, "S", flag(track.solo, Color::Yellow));
            if track.clipping {
                buf.set_string(
                    area.x + 4,
                    below + 2,
                    "SAT",
                    Style::new().fg(error_color()).bold(),
                );
            }
            let sends = sends_text(track.sends);
            buf.set_string(
                area.x,
                below + 3,
                sends
                    .trim_start_matches(" → ")
                    .chars()
                    .take(STRIP as usize - 1)
                    .collect::<String>(),
                Style::new().fg(dim()),
            );
        }
        // El master, a continuación.
        let area = strip(shown as u16);
        let master = &view.master;
        buf.set_string(area.x, area.y, "master", Style::new().bold());
        let color = if master.clipping {
            error_color()
        } else if master.limiting {
            Color::Yellow
        } else {
            meter_color()
        };
        draw_vertical_meter(master.level, area, meter_rows, color, buf);
        let below = area.y + 1 + meter_rows;
        buf.set_string(area.x, below, db_text(master.level), Style::new().fg(dim()));
        if master.limiting {
            buf.set_string(
                area.x,
                below + 2,
                "LIM",
                Style::new().fg(Color::Black).bg(Color::Yellow),
            );
        }
        if master.clipping {
            buf.set_string(
                area.x + 4,
                below + 2,
                "SAT",
                Style::new().fg(Color::Black).bg(error_color()),
            );
        }
    }
}

/// El medidor vertical de una tira: dos columnas bajo el nombre.
fn draw_vertical_meter(level: f32, strip: Rect, rows: u16, color: Color, buf: &mut Buffer) {
    for (row, cell) in vertical_meter(level, rows).into_iter().enumerate() {
        let y = strip.y + 1 + row as u16;
        // Lo que no llega, una guía tenue.
        let (symbol, style) = match cell {
            ' ' => ('·', Style::new().fg(dim())),
            c => (c, Style::new().fg(color)),
        };
        for dx in 0..2 {
            buf[(strip.x + dx, y)].set_char(symbol).set_style(style);
        }
    }
}

/// Envíos de una pista a los buses, para el panel de efectos: `
/// → d0.30 r0.15` (vacío sin envíos).
pub fn sends_text(sends: [f32; 2]) -> String {
    sends_text_moving(sends, [false; 2])
}

/// Como [`sends_text`], con `∿` en los envíos que se mueven con una curva.
pub fn sends_text_moving(sends: [f32; 2], moving: [bool; 2]) -> String {
    let parts: Vec<String> = [("d", sends[0], moving[0]), ("r", sends[1], moving[1])]
        .iter()
        .filter(|(_, s, m)| *s > 0.0 || *m)
        .map(|(bus, s, m)| {
            if *m {
                format!("{bus}∿")
            } else {
                format!("{bus}{s:.2}")
            }
        })
        .collect();
    if parts.is_empty() {
        String::new()
    } else {
        format!(" → {}", parts.join(" "))
    }
}

/// El pan de una pista para el panel de efectos: ` pan -0.30`, ` pan ∿`
/// si se mueve, o nada al centro.
fn pan_text(track: &crate::model::TrackView) -> String {
    if track.mix_moving[engine::MIX_PAN] {
        " pan ∿".into()
    } else if track.pan != 0.0 {
        format!(" pan {:+.2}", track.pan)
    } else {
        String::new()
    }
}

/// Alto del panel de efectos: nombre, volumen, nivel y hasta 4 efectos.
pub const FX_PANEL_HEIGHT: u16 = 3 + engine::MAX_FX as u16 + 2;
/// Alto del panel de efectos: una fila más si alguna pista tiene efectos
/// por corte (así siempre cabe al menos su línea resumida).
pub fn fx_panel_height(view: &ViewModel) -> u16 {
    FX_PANEL_HEIGHT + u16::from(view.tracks.iter().any(|t| !t.slice_fx.is_empty()))
}

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
            Line::from("sin pistas").fg(dim()).render(inner, buf);
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
                name.fg(accent())
            } else {
                name
            };
            let mut title = vec![name];
            if track.mute {
                title.push(Span::from(" MUTE").fg(Color::Black).bg(error_color()));
            } else if track.solo {
                title.push(Span::from(" SOLO").fg(Color::Black).bg(Color::Yellow));
            } else if !view.audible(i) {
                title.push(Span::from(" (solo de otra)").fg(dim()));
            }
            if track.swing > 50.0 {
                title.push(Span::from(format!(" · swing {}", track.swing)).fg(dim()));
            }
            if track.gate {
                title.push(Span::from(" · gate").fg(dim()));
            }
            if let Some((channel, base)) = track.midi {
                // En un instrumento, cada nota sale a su altura: sin base.
                let text = match track.instrument {
                    Some(_) => format!(" · MIDI {channel}"),
                    None => format!(" · MIDI {channel}/{base}"),
                };
                title.push(Span::from(text).fg(dim()));
            }
            if let Some(i) = track.instrument {
                let kind = track
                    .synth
                    .as_ref()
                    .map_or(String::new(), |s| format!("{} ", s.kind));
                title.push(
                    Span::from(format!(
                        " · ♪ {kind}{} {}/{}",
                        script::patterns::note_name(i.root),
                        i.sounding,
                        i.voices
                    ))
                    .fg(dim()),
                );
            }
            if let Some(formant) = track.formant {
                let text = if formant == 0.0 {
                    " · formant 0".to_string()
                } else {
                    format!(" · formant {formant:+}")
                };
                title.push(Span::from(text).fg(dim()));
            }
            if let Some(grains) = &track.grains {
                title.push(Span::from(format!(" · {grains}")).fg(dim()));
            }
            if let Some(group) = track.choke {
                title.push(Span::from(format!(" · choke {group}")).fg(dim()));
            }
            let mut lines = vec![
                Line::from(title),
                Line::from(vec![
                    Span::raw(format!(
                        "vol {:.2}{} ",
                        track.gain,
                        if track.mix_moving[engine::MIX_GAIN] {
                            "∿"
                        } else {
                            ""
                        }
                    )),
                    Span::from(meter(
                        track.gain / engine::MAX_GAIN,
                        width.saturating_sub(4),
                    ))
                    .fg(accent()),
                    // Pan de la pista, si no está al centro (o se mueve).
                    Span::from(pan_text(track)).fg(dim()),
                    // Envíos a los buses: `→ d0.30 r0.15`.
                    Span::from(sends_text_moving(
                        track.sends,
                        [
                            track.mix_moving[engine::MIX_SEND],
                            track.mix_moving[engine::MIX_SEND + 1],
                        ],
                    ))
                    .fg(dim()),
                ]),
                Line::from(vec![
                    Span::raw("nivel "),
                    Span::from(meter(track.level, width)).fg(if track.clipping {
                        error_color()
                    } else {
                        meter_color()
                    }),
                    if track.clipping {
                        Span::from(" SAT").fg(error_color()).bold()
                    } else {
                        Span::raw("")
                    },
                ]),
            ];
            if track.fx.is_empty() && track.slice_fx.is_empty() {
                lines.push(Line::from("sin efectos").fg(dim()));
            }
            // Los de cada corte, que suenan antes que los de la pista:
            // `◇ 3 snare › crush 6b ÷1 · LP 800 Hz q2.00 ∿cutoff`. Si no
            // caben con los de la pista, una sola línea con los cortes.
            let room = (column.height as usize).saturating_sub(lines.len() + track.fx.len());
            if track.slice_fx.len() > room && !track.slice_fx.is_empty() {
                let names: Vec<String> = track
                    .slice_fx
                    .iter()
                    .map(|c| c.name(&track.labels))
                    .collect();
                lines.push(Line::from(format!("◇ efectos en {}", names.join(", "))).fg(dim()));
            } else {
                for chain in &track.slice_fx {
                    let mut spans =
                        vec![Span::from(format!("◇ {} › ", chain.name(&track.labels))).fg(dim())];
                    for (i, fx) in chain.fx.iter().enumerate() {
                        if i > 0 {
                            spans.push(Span::from(" · ").fg(dim()));
                        }
                        spans.push(Span::raw(fx_text(fx)));
                        for (_, p) in chain.automated.iter().filter(|(f, _)| *f == i) {
                            spans.push(Span::from(format!(" ∿{p}")).fg(accent()));
                        }
                    }
                    lines.push(Line::from(spans));
                }
            }
            for (i, fx) in track.fx.iter().enumerate() {
                // Los parámetros que se mueven (ramp, lfo): `∿cutoff`.
                let moving: Vec<String> = track
                    .automated
                    .iter()
                    .filter(|(f, _)| *f == i)
                    .map(|(_, p)| format!("∿{p}"))
                    .collect();
                lines.push(Line::from(vec![
                    Span::raw(format!(
                        "› {}",
                        fx_text_named(
                            fx,
                            fx.sidechain().and_then(|k| view
                                .tracks
                                .iter()
                                .find(|t| t.slot == Some(k))
                                .map(|t| t.name.as_str()))
                        )
                    )),
                    Span::from(if moving.is_empty() {
                        String::new()
                    } else {
                        format!(" {}", moving.join(" "))
                    })
                    .fg(accent()),
                ]));
            }
            Paragraph::new(lines).render(*column, buf);
        }
    }
}
