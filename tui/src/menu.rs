//! La barra de menús (F10): las acciones de la paleta repartidas en
//! menús, cada una con su tecla. No hay una lista aparte: el menú de cada
//! acción sale de [`menu_of`] (un `match` sin comodín, así que una acción
//! nueva no compila hasta que se le da su menú) y sus nombres y teclas, de
//! la paleta y de `[teclas]`.

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Clear, Widget};

use crate::config::{Bound, Keymap};
use crate::palette::{ACTIONS, Action};

/// Un menú de la barra.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Menu {
    File,
    Edit,
    Track,
    Transport,
    View,
    Windows,
    Help,
}

impl Menu {
    pub const ALL: [Menu; 7] = [
        Menu::File,
        Menu::Edit,
        Menu::Track,
        Menu::Transport,
        Menu::View,
        Menu::Windows,
        Menu::Help,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Menu::File => "Archivo",
            Menu::Edit => "Editar",
            Menu::Track => "Pista",
            Menu::Transport => "Transporte",
            Menu::View => "Ver",
            Menu::Windows => "Ventanas",
            Menu::Help => "Ayuda",
        }
    }

    /// La letra que lo abre con el menú abierto (subrayada en la barra).
    pub fn letter(self) -> char {
        match self {
            Menu::File => 'a',
            Menu::Edit => 'e',
            Menu::Track => 'p',
            Menu::Transport => 't',
            Menu::View => 'v',
            Menu::Windows => 'n',
            Menu::Help => 'y',
        }
    }
}

/// El menú de cada acción (`None`: la que abre los menús).
pub fn menu_of(action: Action) -> Option<Menu> {
    use Action::*;
    Some(match action {
        SaveAndLoad | Load | ReloadExternal | Export | Commands | Versions | SaveBlock
        | InsertBlock | UpdateBlock | DeleteBlock | YankSlot | PasteSlot | Browser | Resample
        | Freeze | Unfreeze | ExportKnobs | Quit => Menu::File,
        Undo | Redo | ToggleComment | DuplicateLines | MoveLinesUp | MoveLinesDown | TextStart
        | TextEnd | GotoLine | Find | FindNext | FindPrevious | Complete | GotoError
        | ToggleVim | EditTracker | EditRoll | FillGap | SuggestFills => Menu::Edit,
        NextTrack | Mute | Solo | ClearMuteSolo | LabelSlice | SliceNotes | SampleKey | Preview
        | SliceEarlier | SliceLater | ClearSliceAdjust | NextSlice | PreviousSlice | SplitSlice
        | JoinSlice | ZoomIn | ZoomOut | ZoomReset | DetectBpm | MidiLearn | MidiKnobs | Macros
        | LaneTouch | Record => Menu::Track,
        Transport | Cue | ClearCue | Metronome | TapTempo | NextSection | PreviousSection
        | JumpSection | LoopSection | FollowTimeline | ToggleVariant | Panic | PanicPause
        | PlayMode => Menu::Transport,
        SidechainView | LanesView | Occupancy | PianoRoll | Highlight | ResetMeters
        | AnalyzeMasking => Menu::View,
        EditorWindow | EditorBack | RollWindow | RollBack | Devices | RefreshDevices
        | DevicesWindow | DevicesBack => Menu::Windows,
        TerminalInfo => Menu::Help,
        OpenMenu => return None,
    })
}

/// Una entrada de un menú: lo que hace, su nombre y su tecla.
#[derive(Debug, Clone, PartialEq)]
pub struct MenuItem {
    pub bound: Bound,
    pub name: String,
    pub keys: String,
}

/// Las entradas de `menu`, en el orden de la paleta (Ayuda empieza por la
/// paleta misma).
pub fn items(menu: Menu, keys: &Keymap) -> Vec<MenuItem> {
    let mut items = Vec::new();
    if menu == Menu::Help {
        items.push(MenuItem {
            bound: Bound::Palette,
            name: "Paleta de comandos".into(),
            keys: keys.describe(Bound::Palette),
        });
    }
    let entries = crate::palette::entries_with(None, keys);
    for def in ACTIONS.iter().filter(|a| menu_of(a.action) == Some(menu)) {
        let keys = entries
            .iter()
            .find(|e| e.item == crate::palette::Item::Action(def.action))
            .map_or(String::new(), |e| e.keys.clone());
        items.push(MenuItem {
            bound: Bound::Action(def.action),
            name: def.name.into(),
            keys,
        });
    }
    items
}

/// El menú abierto.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct MenuState {
    /// Cuál (índice en [`Menu::ALL`]) y la entrada elegida.
    pub menu: usize,
    pub item: usize,
}

/// Lo que se dibuja del menú abierto.
#[derive(Debug, Clone, PartialEq)]
pub struct MenuView {
    pub open: usize,
    pub items: Vec<MenuItem>,
    pub selected: usize,
}

/// Dónde va el nombre de cada menú en la barra de la fila `area`.
pub fn bar_layout(area: Rect) -> Vec<Rect> {
    let mut x = area.x;
    Menu::ALL
        .iter()
        .map(|m| {
            let width = m.name().chars().count() as u16 + 2;
            let rect = Rect {
                x,
                y: area.y,
                width: width.min(area.right().saturating_sub(x)),
                height: 1,
            };
            x = x.saturating_add(width + 1);
            rect
        })
        .collect()
}

/// Dónde va el desplegable del menú `open` bajo la barra de `bar` en
/// `screen`: debajo de su nombre, tan ancho como su entrada más larga y,
/// si no cabe, con las que quepan.
pub fn dropdown_area(screen: Rect, bar: Rect, view: &MenuView) -> Rect {
    let name = bar_layout(bar)[view.open];
    let names = view
        .items
        .iter()
        .map(|i| i.name.chars().count())
        .max()
        .unwrap_or(0);
    let keys = view
        .items
        .iter()
        .map(|i| i.keys.chars().count())
        .max()
        .unwrap_or(0);
    let width = ((names + keys + 5) as u16).min(screen.width);
    let height = (view.items.len() as u16 + 2).min(screen.bottom().saturating_sub(bar.bottom()));
    let x = name.x.min(screen.right().saturating_sub(width));
    Rect {
        x,
        y: bar.bottom(),
        width,
        height,
    }
}

/// La primera entrada que se ve del desplegable (para que se vea la
/// elegida si no caben todas).
pub fn first_shown(view: &MenuView, area: Rect) -> usize {
    let rows = area.height.saturating_sub(2) as usize;
    if rows == 0 || view.selected < rows {
        0
    } else {
        view.selected + 1 - rows
    }
}

/// La barra de menús, con el abierto resaltado y la letra de cada uno
/// subrayada.
pub struct MenuBar<'a> {
    pub open: Option<usize>,
    pub hint: &'a str,
}

impl Widget for MenuBar<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let theme = crate::theme::get();
        // Tapa lo que hubiera debajo (sin fila propia, la cabecera).
        Clear.render(area, buf);
        buf.set_style(area, Style::new().bg(theme.header));
        for (i, (menu, rect)) in Menu::ALL.iter().zip(bar_layout(area)).enumerate() {
            if rect.width == 0 {
                break;
            }
            let base = if self.open == Some(i) {
                Style::new().fg(Color::Black).bg(theme.accent).bold()
            } else {
                Style::new().bg(theme.header)
            };
            let name = menu.name();
            let mut spans = vec![Span::styled(" ", base)];
            let mut marked = false;
            for c in name.chars() {
                let style = if !marked && c.to_ascii_lowercase() == menu.letter() {
                    marked = true;
                    base.add_modifier(Modifier::UNDERLINED)
                } else {
                    base
                };
                spans.push(Span::styled(c.to_string(), style));
            }
            spans.push(Span::styled(" ", base));
            Line::from(spans).render(rect, buf);
        }
        let hint = self.hint.chars().count() as u16;
        let end = bar_layout(area).last().map_or(area.x, |r| r.right());
        if end + 2 + hint <= area.right() {
            buf.set_string(
                area.right() - hint,
                area.y,
                self.hint,
                Style::new().fg(theme.dim).bg(theme.header),
            );
        }
    }
}

/// El desplegable del menú abierto.
pub struct Dropdown<'a>(pub &'a MenuView);

impl Widget for Dropdown<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let view = self.0;
        let theme = crate::theme::get();
        Clear.render(area, buf);
        let block = Block::bordered().border_style(Style::new().fg(theme.border_focus));
        let inner = block.inner(area);
        block.render(area, buf);
        let first = first_shown(view, area);
        for (row, (i, item)) in view
            .items
            .iter()
            .enumerate()
            .skip(first)
            .take(inner.height as usize)
            .enumerate()
        {
            let y = inner.y + row as u16;
            let style = if i == view.selected {
                Style::new().fg(Color::Black).bg(theme.accent)
            } else {
                Style::new()
            };
            let keys = item.keys.chars().count() as u16;
            let room = inner.width.saturating_sub(keys + 2) as usize;
            let name: String = item.name.chars().take(room).collect();
            buf.set_style(
                Rect {
                    y,
                    height: 1,
                    ..inner
                },
                style,
            );
            buf.set_string(inner.x + 1, y, name, style);
            if keys + 2 <= inner.width {
                buf.set_string(
                    inner.right() - keys - 1,
                    y,
                    &item.keys,
                    style.fg(if i == view.selected {
                        Color::Black
                    } else {
                        theme.dim
                    }),
                );
            }
        }
    }
}

/// Qué hay bajo el ratón con el menú abierto o en la barra.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MenuHit {
    /// El nombre del menú `i` en la barra.
    Bar(usize),
    /// La entrada `i` del desplegable.
    Item(usize),
}

/// Lo que pulsa el ratón en (`x`, `y`), con la barra en `bar` y el
/// desplegable (si hay uno abierto) en `dropdown`.
pub fn hit(bar: Rect, dropdown: Option<(Rect, &MenuView)>, x: u16, y: u16) -> Option<MenuHit> {
    let at = ratatui::layout::Position::new(x, y);
    if let Some((area, view)) = dropdown {
        let inner = Block::bordered().inner(area);
        if inner.contains(at) {
            let i = first_shown(view, area) + (y - inner.y) as usize;
            return (i < view.items.len()).then_some(MenuHit::Item(i));
        }
    }
    bar_layout(bar)
        .iter()
        .position(|r| r.contains(at))
        .map(MenuHit::Bar)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_action_but_the_menu_key_is_in_a_menu() {
        let keys = Keymap::default();
        let listed: usize = Menu::ALL.iter().map(|m| items(*m, &keys).len()).sum();
        // Todas las de la paleta salvo la de abrir los menús, y la paleta.
        assert_eq!(listed, ACTIONS.len() - 1 + 1);
        for def in ACTIONS {
            assert_eq!(
                menu_of(def.action).is_none(),
                def.action == Action::OpenMenu,
                "{}",
                def.name
            );
        }
    }

    #[test]
    fn items_carry_their_keys() {
        let keys = Keymap::default();
        let file = items(Menu::File, &keys);
        let save = file
            .iter()
            .find(|i| i.bound == Bound::Action(Action::SaveAndLoad))
            .unwrap();
        assert_eq!(save.keys, "Ctrl-S");
        let help = items(Menu::Help, &keys);
        assert_eq!(help[0].bound, Bound::Palette);
        assert_eq!(help[0].keys, "F1");
    }

    #[test]
    fn the_bar_and_the_dropdown_are_where_the_mouse_finds_them() {
        let bar = Rect::new(0, 0, 120, 1);
        let names = bar_layout(bar);
        assert_eq!(names[0], Rect::new(0, 0, 9, 1));
        assert_eq!(hit(bar, None, 3, 0), Some(MenuHit::Bar(0)));
        assert_eq!(hit(bar, None, names[2].x, 0), Some(MenuHit::Bar(2)));
        let view = MenuView {
            open: 2,
            items: items(Menu::Track, &Keymap::default()),
            selected: 0,
        };
        let screen = Rect::new(0, 0, 120, 40);
        let area = dropdown_area(screen, bar, &view);
        assert_eq!((area.x, area.y), (names[2].x, 1));
        assert_eq!(
            hit(bar, Some((area, &view)), area.x + 2, area.y + 1),
            Some(MenuHit::Item(0))
        );
        assert_eq!(
            hit(bar, Some((area, &view)), area.x + 2, area.y + 3),
            Some(MenuHit::Item(2))
        );
    }
}
