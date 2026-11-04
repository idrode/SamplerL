//! La ventana principal sobre la rejilla de [`crate::layout`]: cabecera
//! con el transporte y el master, la regla de pasos con la línea de
//! tiempo, los paneles en su sitio, las pestañas con lo que no cabe, el
//! mensaje y las teclas del panel con el foco. Cada panel sigue siendo su
//! widget de `ui`; aquí solo se decide dónde va y se apunta, para el
//! ratón, dónde quedó.

use ratatui::Frame;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Color, Style, Stylize};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Paragraph, Widget};

use crate::app::{App, PanelAreas};
use crate::focus::Panel;
use crate::layout::{self, Dock, Needs, Size, Tab};
use crate::model::ViewModel;
use crate::ui::{self, accent, dim, error_color, warning_color};

/// Lo que pide la vista a la rejilla.
pub fn needs(view: &ViewModel) -> Needs {
    Needs {
        editor: !view.editor_outside,
        roll: view.roll.is_some(),
        mixer_width: ui::MixerPanel::width(view.tracks.len()),
        fx_height: ui::fx_panel_height(view),
        dock_want: 0,
        menu: view.menu_bar,
        master_viz: view.master_viz,
        occupancy: view.occupancy.is_some(),
        viz: view.viz.iter().any(|p| p.line.is_none()),
        sidechain: view.sidechain.is_some(),
        clash: view.clash_table.is_some(),
    }
}

/// La pestaña que se ve en `dock`: la del panel con el foco si está ahí;
/// si no, la elegida; si tampoco está, la primera.
pub fn shown_tab(view: &ViewModel, dock: &Dock) -> Tab {
    let focus = Tab::Panel(view.focus);
    if dock.tabs.contains(&focus) {
        focus
    } else if dock.tabs.contains(&view.dock_tab) {
        view.dock_tab
    } else {
        dock.tabs.first().copied().unwrap_or_default()
    }
}

/// Filas que pide `tab` para verse entera (0: se adapta a las que haya).
fn tab_height(view: &ViewModel, tab: Tab) -> u16 {
    match tab {
        // Los efectos: el título, el volumen, el nivel, los de la pista que
        // más tenga (y la línea de los de los cortes) y los bordes.
        Tab::Panel(Panel::Fx) => {
            let most = view.tracks.iter().map(|t| t.fx.len()).max().unwrap_or(0);
            let slices = view.tracks.iter().any(|t| !t.slice_fx.is_empty());
            (most.max(1) + 3 + usize::from(slices) + 2) as u16
        }
        Tab::Panel(Panel::Lanes) => view.lanes.as_ref().map_or(0, |l| l.height()),
        Tab::Panel(Panel::Occupancy) => ui::OccupancyPanel::height(view),
        Tab::Sidechain => view.sidechain.as_ref().map_or(0, |s| s.height()),
        Tab::Viz => crate::viz::total_height(&ui::stacked_viz(view)),
        Tab::Clash => view
            .clash_table
            .as_ref()
            .map_or(0, |t| t.rows.len().max(t.clashes.len()) as u16 + 5),
        _ => 0,
    }
}

/// La rejilla de `screen` para `view`: con el alto que pida la pestaña
/// que se va a ver.
pub fn grid_of(view: &ViewModel, screen: Rect) -> layout::Grid {
    let mut needs = needs(view);
    let first = layout::grid(screen, &needs);
    let Some(dock) = &first.dock else {
        return first;
    };
    needs.dock_want = tab_height(view, shown_tab(view, dock));
    if needs.dock_want == 0 {
        return first;
    }
    layout::grid(screen, &needs)
}

/// Dibuja la ventana principal.
pub fn draw(frame: &mut Frame, app: &mut App) {
    let view = app.view();
    let screen = frame.area();
    let grid = grid_of(&view, screen);
    let mut panels = PanelAreas::default();
    app.editor_area = None;
    app.sample_area = None;
    let mut roll_area = None;
    let mut lanes_area = None;

    if grid.size == Size::Tiny {
        if let Some(area) = grid.header {
            Header(&view).render(area, frame.buffer_mut());
        }
        if let Some(area) = grid.message {
            let text = format!(
                "terminal demasiado pequeña ({}×{}) · el audio sigue · {}",
                screen.width, screen.height, view.message
            );
            Paragraph::new(text)
                .wrap(ratatui::widgets::Wrap { trim: true })
                .fg(dim())
                .render(area, frame.buffer_mut());
        }
        app.set_roll_area(None);
        app.set_lanes_area(None);
        app.panels = panels;
        return;
    }

    if let Some(area) = grid.menu {
        draw_menu_bar(&view, area, frame.buffer_mut());
    }
    if let Some(area) = grid.header {
        Header(&view).render(area, frame.buffer_mut());
    }
    if let Some(area) = grid.ruler {
        Ruler(&view).render(area, frame.buffer_mut());
    }
    let popup = match grid.editor {
        Some(area) => {
            panels.focus.push((area, Panel::Code));
            ui::draw_editor(frame, app, &view, area)
        }
        None => None,
    };
    if let Some(area) = grid.wave {
        panels.focus.push((area, Panel::Wave));
        draw_wave(&view, area, frame.buffer_mut());
        app.sample_area = Some(Block::bordered().inner(area));
    }
    let mut place = |panel: Panel, area: Rect, frame: &mut Frame, panels: &mut PanelAreas| {
        panels.focus.push((area, panel));
        draw_panel(
            &view,
            panel,
            area,
            frame,
            panels,
            &mut roll_area,
            &mut lanes_area,
        );
    };
    for panel in [
        Panel::Tracker,
        Panel::Mixer,
        Panel::Fx,
        Panel::Lanes,
        Panel::Scope,
        Panel::Spectrum,
    ] {
        if let Some(area) = grid.area_of(panel) {
            place(panel, area, frame, &mut panels);
        }
    }
    if let Some(dock) = &grid.dock {
        let tab = shown_tab(&view, dock);
        let (strip, body) = take_strip(dock.area);
        panels.tabs = tab_strip(dock, tab, strip, frame.buffer_mut());
        match tab {
            Tab::Panel(panel) => place(panel, body, frame, &mut panels),
            Tab::Viz => {
                let stacked = ui::stacked_viz(&view);
                frame.render_widget(crate::viz::VizPanels(&stacked), body);
            }
            Tab::Sidechain => {
                if let Some(sidechain) = &view.sidechain {
                    frame.render_widget(crate::sidechain::SidechainPanel(sidechain), body);
                }
            }
            Tab::Clash => frame.render_widget(ui::ClashPanel(&view), body),
            Tab::None => {}
        }
    }
    if let Some(area) = grid.reference {
        Reference(&view).render(area, frame.buffer_mut());
    }
    if let Some(area) = grid.message {
        MessageLine(&view).render(area, frame.buffer_mut());
    }
    if let Some(area) = grid.keys {
        KeysLine(&view).render(area, frame.buffer_mut());
    }
    app.set_roll_area(roll_area);
    app.set_lanes_area(lanes_area);
    app.panels = panels;
    ui::draw_popups(frame, &view, popup);
    // El menú abierto, encima de todo; sin fila propia (terminal pequeña),
    // su barra tapa la de arriba mientras está abierto.
    if let Some(menu) = &view.menu {
        let bar = grid.menu.unwrap_or(Rect {
            height: 1,
            ..screen
        });
        draw_menu_bar(&view, bar, frame.buffer_mut());
        let area = crate::menu::dropdown_area(screen, bar, menu);
        frame.render_widget(crate::menu::Dropdown(menu), area);
    }
    app.set_grid(grid);
    if crate::theme::ascii() {
        to_ascii(frame.buffer_mut());
    }
}

/// El dibujo de cajas, los bloques y los símbolos, en ASCII (para una
/// terminal sin Unicode: `[interfaz] ascii` o un `LANG` sin UTF-8). Lo que
/// no está en la tabla (las letras con tilde del texto) se queda.
pub fn to_ascii(buf: &mut Buffer) {
    for cell in buf.content.iter_mut() {
        let mut chars = cell.symbol().chars();
        let (Some(c), None) = (chars.next(), chars.next()) else {
            continue;
        };
        if c.is_ascii() {
            continue;
        }
        if let Some(plain) = ascii_of(c) {
            cell.set_char(plain);
        }
    }
}

/// El equivalente en ASCII de un símbolo de la interfaz.
fn ascii_of(c: char) -> Option<char> {
    Some(match c {
        '┌' | '┐' | '└' | '┘' | '├' | '┤' | '┬' | '┴' | '┼' | '╭' | '╮' | '╯' | '╰' => {
            '+'
        }
        '─' | '━' | '╌' | '┄' | '−' | '–' => '-',
        '│' | '┃' | '┊' | '┆' | '╎' => '|',
        '█' | '▇' | '▆' | '▓' | '■' | '▰' | '▙' | '▛' | '▜' | '▟' => '#',
        '▅' | '▄' | '▃' | '▒' | '▌' | '▐' | '▚' | '▞' => '=',
        '▂' | '▁' | '░' | '▱' | '▖' | '▗' | '▘' | '▝' => '.',
        '▀' | '▔' => '\'',
        '▶' | '▸' | '›' | '→' | '»' => '>',
        '◀' | '‹' | '←' | '«' | '⏮' => '<',
        '▲' | '↑' => '^',
        '▼' | '↓' => 'v',
        '·' | '•' | '●' | '◉' | '◇' | '◆' | '▣' => '*',
        '✗' | '×' => 'x',
        '⚠' => '!',
        'ⓘ' => 'i',
        '…' => '~',
        '∿' | '⟳' => '~',
        '±' => '+',
        '÷' => '/',
        '❄' => '*',
        '♪' | '♩' => 'o',
        _ => return None,
    })
}
fn draw_menu_bar(view: &ViewModel, area: Rect, buf: &mut Buffer) {
    crate::menu::MenuBar {
        open: view.menu.as_ref().map(|m| m.open),
        hint: "F10 menú · F1 paleta ",
    }
    .render(area, buf);
}

/// La primera fila de `area` (las pestañas) y lo demás.
fn take_strip(area: Rect) -> (Rect, Rect) {
    let strip = Rect { height: 1, ..area };
    let body = Rect {
        y: area.y + 1,
        height: area.height.saturating_sub(1),
        ..area
    };
    (strip, body)
}

/// La onda del sample: la grande si hay alto, la compacta si no.
fn draw_wave(view: &ViewModel, area: Rect, buf: &mut Buffer) {
    if area.height >= 9 {
        ui::BigSamplePanel(view).render(area, buf);
    } else {
        ui::SamplePanel(view).render(area, buf);
    }
}

/// Un panel en `area`, y lo que el ratón necesita saber de él.
fn draw_panel(
    view: &ViewModel,
    panel: Panel,
    area: Rect,
    frame: &mut Frame,
    panels: &mut PanelAreas,
    roll_area: &mut Option<Rect>,
    lanes_area: &mut Option<Rect>,
) {
    let buf = frame.buffer_mut();
    match panel {
        Panel::Tracker => match &view.roll {
            Some(roll) => {
                crate::roll_panel::RollPanel(roll).render(area, buf);
                *roll_area = Some(area);
            }
            None => {
                ui::FullTrackerPanel(view).render(area, buf);
                panels.tracker = Some(ui::TrackerLayout::new(view, area));
            }
        },
        Panel::Mixer => {
            ui::MixerPanel(view).render(area, buf);
            panels.mixer = Some(ui::MixerLayout::new(view, area));
        }
        Panel::Fx => ui::FxPanel(view).render(area, buf),
        Panel::Lanes => match &view.lanes {
            Some(lanes) => {
                crate::lanes_panel::LanesPanel(lanes).render(area, buf);
                *lanes_area = Some(area);
            }
            None => empty(view, panel, "sin pistas", area, buf),
        },
        Panel::Scope | Panel::Spectrum => match master_viz(view, panel) {
            // Tan alto como su sitio.
            Some(viz) => {
                let mut viz = viz.clone();
                let rows = area.height.saturating_sub(2);
                match &mut viz {
                    crate::viz::VizView::Scope(s) => s.size = rows,
                    crate::viz::VizView::Spectrum(s) => s.size = rows,
                    _ => {}
                }
                crate::viz::VizPanels(std::slice::from_ref(&viz)).render(area, buf);
            }
            None => empty(
                view,
                panel,
                "esperando audio del master (suena al dar marcha)",
                area,
                buf,
            ),
        },
        Panel::Occupancy => {
            ui::OccupancyPanel(view).render(area, buf);
            panels.occupancy = ui::OccupancyLayout::new(view, area);
        }
        Panel::Code | Panel::Wave => {}
    }
}

/// La visualización del master de `panel`.
fn master_viz(view: &ViewModel, panel: Panel) -> Option<&crate::viz::VizView> {
    let index = usize::from(panel == Panel::Spectrum);
    view.master_views.get(index)
}

/// Un panel sin nada que enseñar: su borde y por qué.
fn empty(view: &ViewModel, panel: Panel, why: &str, area: Rect, buf: &mut Buffer) {
    Paragraph::new(Line::from(why.to_string()).fg(dim()))
        .block(ui::panel_block(view, panel).title(format!(" {} ", panel.title())))
        .render(area, buf);
}

/// Las pestañas del dock en `strip`, con la que se ve resaltada. Si no
/// caben todas, empiezan más adelante para que se vea la elegida, con `‹`
/// y `›` donde hay más. Devuelve dónde quedó cada una (para el ratón).
fn tab_strip(dock: &Dock, shown: Tab, strip: Rect, buf: &mut Buffer) -> Vec<(Rect, Tab)> {
    let theme = crate::theme::get();
    buf.set_style(strip, Style::new().bg(theme.header));
    let labels: Vec<String> = dock
        .labels()
        .into_iter()
        .map(|l| format!(" {l} "))
        .collect();
    let widths: Vec<u16> = labels
        .iter()
        .map(|l| l.chars().count() as u16 + 1)
        .collect();
    let room = strip.width.saturating_sub(2);
    let chosen = dock.tabs.iter().position(|t| *t == shown).unwrap_or(0);
    // La primera que se ve: la menor que deja ver la elegida.
    let mut first = 0;
    while first < chosen && widths[first..=chosen].iter().sum::<u16>() > room {
        first += 1;
    }
    let mut x = strip.x;
    if first > 0 {
        buf.set_string(x, strip.y, "‹", Style::new().fg(theme.dim).bg(theme.header));
        x += 1;
    }
    let mut hits = Vec::new();
    for (i, (tab, label)) in dock.tabs.iter().zip(&labels).enumerate().skip(first) {
        let width = label.chars().count() as u16;
        if x + width
            > strip
                .right()
                .saturating_sub(u16::from(i + 1 < dock.tabs.len()))
        {
            buf.set_string(
                strip.right() - 1,
                strip.y,
                "›",
                Style::new().fg(theme.dim).bg(theme.header),
            );
            break;
        }
        let style = if *tab == shown {
            Style::new().fg(Color::Black).bg(accent()).bold()
        } else {
            Style::new().fg(dim()).bg(theme.header)
        };
        buf.set_string(x, strip.y, label, style);
        hits.push((
            Rect {
                x,
                y: strip.y,
                width,
                height: 1,
            },
            *tab,
        ));
        x += width + 1;
    }
    hits
}

/// La cabecera: el transporte (con los avisos) a la izquierda y el
/// master a la derecha.
pub struct Header<'a>(pub &'a ViewModel);

impl Widget for Header<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let view = self.0;
        let theme = crate::theme::get();
        buf.set_style(area, Style::new().bg(theme.header));
        let mut left = ui::transport_line(view);
        if let Some(error) = &view.error {
            let at = error.line.map_or(String::new(), |l| format!(" l.{l}"));
            left.push_span(
                Span::from(format!("   ✗ error{at}"))
                    .fg(error_color())
                    .bold(),
            );
        }
        if !view.diagnostics.is_empty() {
            left.push_span(
                Span::from(format!("   ⚠ {}", view.diagnostics.len())).fg(warning_color()),
            );
        }
        // Con el editor en su ventana, el archivo y el enlace (aquí no hay
        // borde del editor que los diga).
        if view.editor_outside {
            let dirty = if view.dirty { " ●" } else { "" };
            let link = view.link.as_deref().unwrap_or_default();
            left.push_span(Span::from(format!("   {}{dirty} · {link}", view.file)).fg(dim()));
        }
        let right = ui::master_line(&view.master);
        let right_width = right.width() as u16;
        let left_width = left.width() as u16;
        left.render(area, buf);
        // El master, a la derecha, si cabe detrás del transporte.
        if left_width + 2 + right_width <= area.width {
            let x = area.right() - right_width;
            right.render(
                Rect {
                    x,
                    width: right_width,
                    ..area
                },
                buf,
            );
        }
    }
}

/// La regla: los 16 pasos con el que suena encendido y, detrás, la línea
/// de tiempo de las secciones.
pub struct Ruler<'a>(pub &'a ViewModel);

impl Widget for Ruler<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let view = self.0;
        let t = view.transport;
        let ascii = crate::theme::ascii();
        let (on, off) = if ascii { ('#', '-') } else { ('▰', '▱') };
        let lit = t.step.filter(|_| t.running);
        let mut spans = Vec::new();
        for step in 0..engine::STEPS {
            let span = if lit == Some(step) {
                Span::from(on.to_string()).fg(accent()).bold()
            } else if lit.is_some_and(|l| step < l) {
                Span::from(on.to_string()).fg(dim())
            } else {
                Span::from(off.to_string()).fg(dim())
            };
            spans.push(span);
        }
        let ruler = Line::from(spans);
        let width = ruler.width() as u16;
        ruler.render(area, buf);
        if let Some(timeline) = &view.timeline {
            let x = area.x + width + 3;
            if x < area.right() {
                ui::TimelineBar(timeline).render(
                    Rect {
                        x,
                        width: area.right() - x,
                        ..area
                    },
                    buf,
                );
            }
        }
    }
}

/// La línea del mensaje.
pub struct MessageLine<'a>(pub &'a ViewModel);

impl Widget for MessageLine<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        Line::from(self.0.message.clone()).render(area, buf);
    }
}

/// Lo que tiene las teclas ahora (un popup o el panel con el foco), en
/// mayúsculas, para el principio de la línea de las teclas.
pub fn keys_owner(view: &ViewModel) -> String {
    if ui::popup_keys(view).is_some() {
        return String::new();
    }
    match view.focus {
        Panel::Code => "EDITOR".into(),
        Panel::Wave => "TOCAR".into(),
        panel => panel.name().to_uppercase(),
    }
}

/// La línea de las teclas del foco (o la de órdenes, abierta).
pub struct KeysLine<'a>(pub &'a ViewModel);

impl Widget for KeysLine<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let view = self.0;
        if view.prompt.is_some() {
            ui::prompt_line(view).render(area, buf);
            return;
        }
        let keys = ui::keys_text(view);
        // Si las teclas ya dicen de quién son («TOCAR: ...»), no se repite.
        let labelled = keys.split_once(": ").is_some_and(|(head, _)| {
            !head.is_empty() && head.chars().all(|c| c.is_uppercase() || c == ' ')
        });
        let owner = if labelled {
            String::new()
        } else {
            keys_owner(view)
        };
        let mut spans = Vec::new();
        if !owner.is_empty() {
            spans.push(Span::from(format!("{owner}  ")).fg(accent()).bold());
        }
        spans.push(Span::from(keys).fg(dim()));
        Line::from(spans).render(area, buf);
    }
}

/// La referencia de teclas: las del foco, una por fila.
pub struct Reference<'a>(pub &'a ViewModel);

impl Widget for Reference<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let view = self.0;
        let owner = keys_owner(view);
        let title = if owner.is_empty() {
            " referencia ".to_string()
        } else {
            format!(" referencia · {owner} ")
        };
        let block = Block::bordered()
            .border_style(Style::new().fg(crate::theme::get().border))
            .title(title);
        let inner = block.inner(area);
        block.render(area, buf);
        let lines: Vec<Line> = reference_items(&ui::keys_text(view))
            .into_iter()
            .map(|(keys, what)| {
                Line::from(vec![
                    Span::from(format!("{keys:<14}")).fg(accent()),
                    Span::raw(what),
                ])
            })
            .collect();
        Paragraph::new(lines).render(inner, buf);
    }
}

/// Parte una línea de teclas («Ctrl-S guardar · Esc tocar») en (teclas,
/// qué hacen). La tecla es la primera palabra, o las que llevan `-` o son
/// flechas y signos (`↑↓ elegir`, `Ctrl-S o F5 cargar`).
pub fn reference_items(keys: &str) -> Vec<(String, String)> {
    keys.split(" · ")
        .filter(|item| !item.trim().is_empty())
        .map(|item| {
            let item = item.trim();
            // Lo que va delante de los dos puntos de un título
            // («TRACKER: ...»), fuera.
            let item = match item.split_once(": ") {
                Some((head, rest)) if head.chars().all(|c| c.is_uppercase() || c == ' ') => rest,
                _ => item,
            };
            let words: Vec<&str> = item.split(' ').collect();
            let mut n = 1;
            while n < words.len() && (words[n] == "o" || words[n] == "y") && n + 1 < words.len() {
                n += 2;
            }
            let (keys, what) = words.split_at(n.min(words.len()));
            (keys.join(" "), what.join(" "))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reference_splits_keys_and_what_they_do() {
        let items = reference_items("Ctrl-S guardar y cargar · Ctrl-L o F5 cargar · Esc tocar");
        assert_eq!(
            items,
            vec![
                ("Ctrl-S".into(), "guardar y cargar".into()),
                ("Ctrl-L o F5".into(), "cargar".into()),
                ("Esc".into(), "tocar".into()),
            ]
        );
        let items = reference_items("TRACKER: ↑↓ elegir · Enter escribir");
        assert_eq!(items[0], ("↑↓".into(), "elegir".into()));
    }
}
