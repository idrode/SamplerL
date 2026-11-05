//! La barra de menús (F10) en la aplicación: abrirla, moverse, ejecutar
//! la entrada elegida (lo mismo que la paleta o su tecla) y el ratón. Lo
//! que hay en cada menú está en [`crate::menu`].

use ratatui::layout::Rect;
use ratatui_textarea::{Input, Key};

use super::{App, Flow, MouseAction};
use crate::config::Bound;
use crate::menu::{self, Menu, MenuHit, MenuState, MenuView};

impl App {
    /// F10: abre la barra de menús en el primero (o la cierra).
    pub fn toggle_menu(&mut self) {
        self.completion = None;
        self.menu = match self.menu {
            Some(_) => None,
            None => Some(MenuState::default()),
        };
    }

    /// Lo que se dibuja del menú abierto.
    pub(super) fn menu_view(&self) -> Option<MenuView> {
        let state = self.menu?;
        Some(MenuView {
            open: state.menu,
            items: menu::rows(Menu::ALL[state.menu], &self.keys),
            selected: state.item,
            sub: state.sub,
        })
    }

    /// Una tecla con el menú abierto: ←→ de menú en menú (→ abre el
    /// submenú de la fila elegida y ← lo cierra), ↑↓ de entrada en entrada,
    /// Enter ejecuta (o abre el submenú), Esc cierra el submenú o el menú y
    /// F10 lo cierra todo; la letra de un menú (con Alt) lo abre y otra
    /// letra va a la siguiente entrada que empieza por ella.
    pub(super) fn handle_menu(&mut self, input: Input) -> Flow {
        let Some(mut state) = self.menu else {
            return Flow::Continue;
        };
        let rows = menu::rows(Menu::ALL[state.menu], &self.keys);
        let row = rows.get(state.item);
        let has_sub = row.is_some_and(|r| !r.sub.is_empty());
        // La lista activa: el submenú abierto o las filas.
        let level: &[menu::MenuItem] = match (state.sub, row) {
            (Some(_), Some(r)) if has_sub => &r.sub,
            _ => &rows,
        };
        let count = level.len().max(1);
        let at = state.sub.unwrap_or(state.item);
        let set = |state: &mut MenuState, i: usize| match state.sub {
            Some(_) => state.sub = Some(i),
            None => state.item = i,
        };
        let menus = Menu::ALL.len();
        let other_menu = |m: usize| MenuState {
            menu: m,
            item: 0,
            sub: None,
        };
        match input.key {
            Key::F(10) => {
                self.menu = None;
                return Flow::Continue;
            }
            Key::Esc if state.sub.is_some() => state.sub = None,
            Key::Esc => {
                self.menu = None;
                return Flow::Continue;
            }
            Key::Left if state.sub.is_some() => state.sub = None,
            Key::Left => state = other_menu((state.menu + menus - 1) % menus),
            Key::Right if state.sub.is_none() && has_sub => state.sub = Some(0),
            Key::Right => state = other_menu((state.menu + 1) % menus),
            Key::Up => set(&mut state, (at + count - 1) % count),
            Key::Down => set(&mut state, (at + 1) % count),
            Key::Home => set(&mut state, 0),
            Key::End => set(&mut state, count - 1),
            Key::Enter if state.sub.is_none() && has_sub => state.sub = Some(0),
            Key::Enter => {
                self.menu = None;
                return match level.get(at) {
                    Some(item) => self.run_menu_item(item.bound),
                    None => Flow::Continue,
                };
            }
            Key::Char(c) if !input.ctrl => {
                let c = c.to_ascii_lowercase();
                let letter_menu = Menu::ALL.iter().position(|m| m.letter() == c);
                let next_item = (1..=level.len())
                    .map(|d| (at + d) % level.len())
                    .find(|&i| initial(&level[i].name) == Some(c));
                match (input.alt, next_item, letter_menu) {
                    (false, Some(i), _) => set(&mut state, i),
                    (_, _, Some(m)) => state = other_menu(m),
                    _ => {}
                }
            }
            _ => {}
        }
        self.menu = Some(state);
        Flow::Continue
    }

    /// Ejecuta una entrada de un menú.
    fn run_menu_item(&mut self, bound: Bound) -> Flow {
        match bound {
            Bound::Palette => {
                self.open_palette();
                Flow::Continue
            }
            Bound::Action(action) => self.run_action(action),
        }
    }

    /// Ayuda › «Qué sabe hacer esta terminal»: lo que se ha detectado y
    /// dónde está la configuración.
    pub(super) fn terminal_info(&mut self) {
        let yes = |on: bool| if on { "sí" } else { "no" };
        let config = crate::config::dir()
            .map(|d| d.join("config.toml").display().to_string())
            .unwrap_or_else(|| "sin carpeta de configuración".into());
        self.message = format!(
            "terminal: truecolor {} · Unicode {} · teclas al soltar (Shift-Ctrl-Espacio) {} · \
             ratón {} · configuración: {config}",
            yes(crate::theme::truecolor()),
            yes(!crate::theme::ascii()),
            yes(self.key_releases),
            yes(self.mouse_enabled),
        );
    }

    /// El ratón sobre la barra o el menú abierto: `true` si era suyo (con
    /// un menú abierto, todo clic lo es: fuera de él, lo cierra).
    pub(super) fn menu_mouse(&mut self, action: MouseAction, x: u16, y: u16) -> bool {
        let Some(bar) = self.menu_bar_area() else {
            return false;
        };
        let view = self.menu_view();
        let screen = self.grid.as_ref().map_or(bar, |g| g.screen);
        let dropdown = view
            .as_ref()
            .map(|v| (menu::dropdown_area(screen, bar, v), v));
        let hit = menu::hit(screen, bar, dropdown.as_ref().map(|(a, v)| (*a, *v)), x, y);
        match (action, hit) {
            (MouseAction::Down, Some(MenuHit::Bar(i))) => {
                self.menu = match self.menu {
                    Some(s) if s.menu == i => None,
                    _ => Some(MenuState {
                        menu: i,
                        item: 0,
                        sub: None,
                    }),
                };
                true
            }
            (MouseAction::Down, Some(MenuHit::Item(i))) => {
                let row = view.and_then(|v| v.items.get(i).cloned());
                match row {
                    // Un submenú: se abre (o se cierra) a su lado.
                    Some(row) if !row.sub.is_empty() => {
                        if let Some(state) = &mut self.menu {
                            let open = state.item == i && state.sub.is_some();
                            state.item = i;
                            state.sub = (!open).then_some(0);
                        }
                    }
                    Some(row) => {
                        self.menu = None;
                        self.run_menu_item(row.bound);
                    }
                    None => {}
                }
                true
            }
            (MouseAction::Down, Some(MenuHit::Sub(i))) => {
                let bound = view.and_then(|v| {
                    v.items
                        .get(v.selected)
                        .and_then(|r| r.sub.get(i))
                        .map(|item| item.bound)
                });
                self.menu = None;
                if let Some(bound) = bound {
                    self.run_menu_item(bound);
                }
                true
            }
            (
                MouseAction::ScrollUp | MouseAction::ScrollDown,
                Some(MenuHit::Item(_) | MenuHit::Sub(_)),
            ) => {
                let count = view.map_or(1, |v| v.items.len().max(1));
                if let Some(state) = &mut self.menu {
                    state.item = if action == MouseAction::ScrollUp {
                        (state.item + count - 1) % count
                    } else {
                        (state.item + 1) % count
                    };
                }
                true
            }
            (MouseAction::Down | MouseAction::RightDown, None) if self.menu.is_some() => {
                self.menu = None;
                true
            }
            _ => self.menu.is_some(),
        }
    }

    /// La fila de la barra de menús: la suya, o con el menú abierto en una
    /// terminal pequeña (sin fila propia), la de arriba.
    pub(super) fn menu_bar_area(&self) -> Option<Rect> {
        let grid = self.grid.as_ref()?;
        grid.menu.or_else(|| {
            self.menu.map(|_| Rect {
                height: 1,
                ..grid.screen
            })
        })
    }
}

/// La primera letra de un nombre, en minúscula y sin tilde.
fn initial(name: &str) -> Option<char> {
    name.chars()
        .next()
        .map(|c| match c.to_lowercase().next().unwrap_or(c) {
            'á' => 'a',
            'é' => 'e',
            'í' => 'i',
            'ó' => 'o',
            'ú' => 'u',
            c => c,
        })
}
