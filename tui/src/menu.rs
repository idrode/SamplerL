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
        | Freeze | Unfreeze | ExportKnobs | AddSample | ReferenceSample | GatherSamples | Quit => {
            Menu::File
        }
        Undo | Redo | ToggleComment | DuplicateLines | MoveLinesUp | MoveLinesDown | TextStart
        | TextEnd | GotoLine | Find | FindNext | FindPrevious | Complete | GotoError
        | ToggleVim | EditTracker | EditRoll | FillGap | SuggestFills => Menu::Edit,
        NextTrack | Mute | Solo | ClearMuteSolo | LabelSlice | SliceNotes | SampleKey | Preview
        | SliceEarlier | SliceLater | SliceEdge | ClearSliceAdjust | NextSlice | PreviousSlice
        | SplitSlice | JoinSlice | ZoomIn | ZoomOut | ZoomReset | DetectBpm | MidiLearn
        | MidiKnobs | Macros | LaneTouch | Record => Menu::Track,
        Transport | Cue | ClearCue | Metronome | TapTempo | NextSection | PreviousSection
        | JumpSection | LoopSection | FollowTimeline | ToggleVariant | Panic | PanicPause
        | PlayMode => Menu::Transport,
        SidechainView | LanesView | Occupancy | PianoRoll | Highlight | ResetMeters
        | AnalyzeMasking | Focus(_) | ToggleExplorer | RefreshFiles | EditorTaller
        | EditorShorter | Wider | Narrower | MaximizeEditor | FloatEditor | FloatUp | FloatDown
        | FloatLeft | FloatRight | WaveStyle | SpectrumStyle | MasterViz => Menu::View,
        EditorWindow | EditorBack | RollWindow | RollBack | Devices | RefreshDevices
        | DevicesWindow | DevicesBack => Menu::Windows,
        TerminalInfo => Menu::Help,
        OpenMenu => return None,
    })
}

/// Una entrada de un menú: lo que hace, su nombre y su tecla; o, con
/// `sub`, un submenú con esas entradas (y entonces `bound` no se usa).
#[derive(Debug, Clone, PartialEq)]
pub struct MenuItem {
    pub bound: Bound,
    pub name: String,
    pub keys: String,
    pub sub: Vec<MenuItem>,
}

/// El submenú de cada acción dentro de su menú (`None`: en el menú
/// mismo).
pub fn submenu_of(action: Action) -> Option<&'static str> {
    use Action::*;
    Some(match action {
        SaveBlock | InsertBlock | UpdateBlock | DeleteBlock => "Bloques",
        YankSlot | PasteSlot => "Ranuras",
        Freeze | Unfreeze => "Congelar",
        GotoLine | Find | FindNext | FindPrevious | TextStart | TextEnd => "Buscar e ir",
        LabelSlice | SliceNotes | SliceEarlier | SliceLater | SliceEdge | ClearSliceAdjust
        | NextSlice | PreviousSlice | SplitSlice | JoinSlice => "Cortes",
        ZoomIn | ZoomOut | ZoomReset => "Zoom de la onda",
        MidiLearn | MidiKnobs | ExportKnobs => "Mandos MIDI",
        NextSection | PreviousSection | JumpSection | LoopSection | FollowTimeline => "Secciones",
        Focus(_) => "Foco",
        EditorTaller | EditorShorter | Wider | Narrower | MaximizeEditor | FloatEditor
        | FloatUp | FloatDown | FloatLeft | FloatRight => "Editor",
        WaveStyle | SpectrumStyle => "Dibujo",
        EditorWindow | EditorBack => "Ventana del editor",
        RollWindow | RollBack => "Ventana del piano roll",
        Devices | RefreshDevices | DevicesWindow | DevicesBack => "Dispositivos",
        _ => return None,
    })
}

/// Las filas del desplegable de `menu`: las entradas sueltas y, donde
/// aparece la primera de un submenú, el submenú con todas las suyas.
pub fn rows(menu: Menu, keys: &Keymap) -> Vec<MenuItem> {
    let mut rows: Vec<MenuItem> = Vec::new();
    for item in items(menu, keys) {
        let group = match item.bound {
            Bound::Action(a) => submenu_of(a),
            Bound::Palette => None,
        };
        match group {
            Some(name) => match rows
                .iter_mut()
                .find(|r| !r.sub.is_empty() && r.name == name)
            {
                Some(row) => row.sub.push(item),
                None => rows.push(MenuItem {
                    bound: item.bound,
                    name: name.into(),
                    keys: "▸".into(),
                    sub: vec![item],
                }),
            },
            None => rows.push(item),
        }
    }
    rows
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
            sub: Vec::new(),
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
            sub: Vec::new(),
        });
    }
    items
}

/// El menú abierto.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct MenuState {
    /// Cuál (índice en [`Menu::ALL`]), la fila elegida y, con su submenú
    /// abierto, la entrada elegida en él.
    pub menu: usize,
    pub item: usize,
    pub sub: Option<usize>,
}

/// Lo que se dibuja del menú abierto.
#[derive(Debug, Clone, PartialEq)]
pub struct MenuView {
    pub open: usize,
    /// Las filas (ver [`rows`]).
    pub items: Vec<MenuItem>,
    pub selected: usize,
    /// El submenú abierto de la fila elegida: su entrada elegida.
    pub sub: Option<usize>,
}

/// El submenú abierto de `view` (sus entradas y la elegida).
fn open_sub(view: &MenuView) -> Option<(&[MenuItem], usize)> {
    let sub = view.sub?;
    let row = view.items.get(view.selected)?;
    (!row.sub.is_empty()).then_some((&row.sub[..], sub))
}

/// Dónde va el submenú abierto de `view`, al lado de su fila del
/// desplegable de `area` (a la derecha si cabe; si no, a la izquierda).
pub fn sub_area(screen: Rect, area: Rect, view: &MenuView) -> Option<Rect> {
    let (items, _) = open_sub(view)?;
    let names = items
        .iter()
        .map(|i| i.name.chars().count())
        .max()
        .unwrap_or(0);
    let keys = items
        .iter()
        .map(|i| i.keys.chars().count())
        .max()
        .unwrap_or(0);
    let width = ((names + keys + 5) as u16).min(screen.width);
    let row = area.y + 1 + (view.selected - first_shown(view, area)) as u16;
    let y = row.saturating_sub(1).max(screen.y);
    let height = (items.len() as u16 + 2).min(screen.bottom().saturating_sub(y));
    let x = if area.right() + width <= screen.right() {
        area.right()
    } else {
        area.x.saturating_sub(width)
    };
    Some(Rect {
        x,
        y,
        width,
        height,
    })
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

/// El desplegable del menú abierto (y su submenú, si hay uno abierto, en
/// `sub_area`).
pub struct Dropdown<'a>(pub &'a MenuView);

impl Dropdown<'_> {
    /// Dibuja el desplegable en `area` y el submenú abierto a su lado.
    pub fn draw(&self, screen: Rect, area: Rect, buf: &mut Buffer) {
        let view = self.0;
        entries(
            &view.items,
            view.selected,
            first_shown(view, area),
            view.sub.is_none(),
            area,
            buf,
        );
        if let (Some((items, selected)), Some(rect)) =
            (open_sub(view), sub_area(screen, area, view))
        {
            entries(items, selected, 0, true, rect, buf);
        }
    }
}

impl Widget for Dropdown<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let screen = buf.area;
        self.draw(screen, area, buf);
    }
}

/// Una lista de entradas con borde en `area`, desde la `first`, con la
/// `selected` resaltada (en el acento si es la lista activa; si no, con el
/// fondo de la cabecera).
fn entries(
    items: &[MenuItem],
    selected: usize,
    first: usize,
    active: bool,
    area: Rect,
    buf: &mut Buffer,
) {
    let theme = crate::theme::get();
    Clear.render(area, buf);
    let block = Block::bordered().border_style(Style::new().fg(theme.border_focus));
    let inner = block.inner(area);
    block.render(area, buf);
    for (row, (i, item)) in items
        .iter()
        .enumerate()
        .skip(first)
        .take(inner.height as usize)
        .enumerate()
    {
        let y = inner.y + row as u16;
        let lit = i == selected && active;
        let style = if lit {
            Style::new().fg(Color::Black).bg(theme.accent)
        } else if i == selected {
            Style::new().bg(theme.header).bold()
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
            let color = if lit { Color::Black } else { theme.dim };
            buf.set_string(inner.right() - keys - 1, y, &item.keys, style.fg(color));
        }
    }
}

/// Qué hay bajo el ratón con el menú abierto o en la barra.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MenuHit {
    /// El nombre del menú `i` en la barra.
    Bar(usize),
    /// La fila `i` del desplegable.
    Item(usize),
    /// La entrada `i` del submenú abierto.
    Sub(usize),
}

/// Lo que pulsa el ratón en (`x`, `y`), con la barra en `bar` y el
/// desplegable (si hay uno abierto) en `dropdown`.
pub fn hit(
    screen: Rect,
    bar: Rect,
    dropdown: Option<(Rect, &MenuView)>,
    x: u16,
    y: u16,
) -> Option<MenuHit> {
    let at = ratatui::layout::Position::new(x, y);
    if let Some((area, view)) = dropdown
        && let Some(rect) = sub_area(screen, area, view)
    {
        let inner = Block::bordered().inner(rect);
        if inner.contains(at) {
            let i = (y - inner.y) as usize;
            let count = open_sub(view).map_or(0, |(items, _)| items.len());
            return (i < count).then_some(MenuHit::Sub(i));
        }
    }
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
    fn rows_group_the_submenus_where_their_first_entry_was() {
        let keys = Keymap::default();
        let view = rows(Menu::View, &keys);
        let foco = view.iter().find(|r| r.name == "Foco").unwrap();
        assert_eq!(foco.keys, "▸");
        assert_eq!(foco.sub.len(), 10);
        assert!(
            foco.sub
                .iter()
                .any(|i| i.name == "Foco: archivos" && i.keys == "Alt-0")
        );
        let editor = view.iter().find(|r| r.name == "Editor").unwrap();
        assert!(
            editor
                .sub
                .iter()
                .any(|i| i.name == "Editor flotante" && i.keys == "Alt-W")
        );
        // Lo que no va en un submenú, suelto; y nada se pierde.
        assert!(
            view.iter()
                .any(|r| r.name == "Explorador de archivos" && r.sub.is_empty())
        );
        for menu in Menu::ALL {
            let flat: usize = rows(menu, &keys).iter().map(|r| r.sub.len().max(1)).sum();
            assert_eq!(flat, items(menu, &keys).len(), "{}", menu.name());
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
        let screen = Rect::new(0, 0, 120, 40);
        assert_eq!(hit(screen, bar, None, 3, 0), Some(MenuHit::Bar(0)));
        assert_eq!(hit(screen, bar, None, names[2].x, 0), Some(MenuHit::Bar(2)));
        let mut view = MenuView {
            open: 2,
            items: rows(Menu::Track, &Keymap::default()),
            selected: 0,
            sub: None,
        };
        let area = dropdown_area(screen, bar, &view);
        assert_eq!((area.x, area.y), (names[2].x, 1));
        assert_eq!(
            hit(screen, bar, Some((area, &view)), area.x + 2, area.y + 1),
            Some(MenuHit::Item(0))
        );
        assert_eq!(
            hit(screen, bar, Some((area, &view)), area.x + 2, area.y + 3),
            Some(MenuHit::Item(2))
        );
        // Con un submenú abierto, a su derecha y alineado con su fila.
        let cortes = view.items.iter().position(|r| r.name == "Cortes").unwrap();
        view.selected = cortes;
        view.sub = Some(0);
        let wide = Rect::new(0, 0, 250, 40);
        let sub = sub_area(wide, area, &view).unwrap();
        assert_eq!((sub.x, sub.y), (area.right(), area.y + cortes as u16));
        let screen = wide;
        assert_eq!(
            hit(screen, bar, Some((area, &view)), sub.x + 2, sub.y + 2),
            Some(MenuHit::Sub(1))
        );
        // Sin sitio a la derecha, a la izquierda (dentro de la pantalla).
        let narrow = Rect::new(0, 0, area.right() + 5, 40);
        let left = sub_area(narrow, area, &view).unwrap();
        assert!(
            left.x < area.x && left.right() <= narrow.right(),
            "{left:?}"
        );
    }
}
