//! Poner el foco en un panel (Alt-1…9, el menú Ver o un clic). Para los
//! que tienen cursor (tracker, piano roll, ocupación, lanes) es abrir ese
//! cursor, como desde la paleta; para el editor y la onda, el modo editar
//! o tocar; para los demás (mezcla, efectos, scope, espectro), apuntarlo y
//! que sus teclas sean suyas. Esc vuelve a donde se estaba.

use ratatui_textarea::{Input, Key};

use super::lanes::LaneFocus;
use super::{App, Flow, Mode};
use crate::focus::Panel;
use crate::layout::Tab;

/// Las teclas de un panel sin cursor, para la línea de abajo.
pub fn panel_keys(panel: Panel) -> Option<&'static str> {
    Some(match panel {
        Panel::Mixer => "MEZCLA: ←→ pista · m mute · s solo · Esc vuelve",
        Panel::Fx => "EFECTOS: ←→ pista · Esc vuelve",
        Panel::Scope | Panel::Spectrum => "←→ pista · Esc vuelve · Alt-1…9 otro panel",
        _ => return None,
    })
}

impl App {
    /// Cierra los cursores de los paneles (el foco vuelve al modo).
    fn close_cursors(&mut self) {
        self.tracker_edit = None;
        self.pending_edit = None;
        self.occupancy.cursor = None;
        self.roll.cursor = None;
        self.lanes_focus = None;
        self.chosen_focus = None;
    }

    /// Pone el foco en `panel`.
    pub fn set_focus(&mut self, panel: Panel) {
        self.menu = None;
        self.completion = None;
        match panel {
            Panel::Code => {
                if self.is_main_window() {
                    self.message = "el editor está en su ventana".into();
                    return;
                }
                self.close_cursors();
                self.mode = Mode::Edit;
            }
            Panel::Wave => {
                self.close_cursors();
                self.mode = Mode::Play;
            }
            Panel::Tracker => {
                self.close_cursors();
                if self.local_roll_view_shown() {
                    self.open_roll_cursor();
                } else {
                    self.open_tracker_edit();
                }
            }
            Panel::Occupancy => {
                self.close_cursors();
                self.open_occupancy_cursor();
            }
            Panel::Lanes => {
                self.close_cursors();
                self.show_tab(Tab::Panel(Panel::Lanes));
                let first = self.lanes_view().and_then(|v| {
                    v.lanes
                        .first()
                        .map(|row| (v.track.clone(), row.param.clone()))
                });
                match first {
                    Some((track, param)) => {
                        self.lanes_focus = Some(LaneFocus {
                            track,
                            param,
                            point: 0,
                        });
                        self.message = "lanes: c curva · Supr borra · ←→ paso · ↑↓ valor · Tab siguiente · Esc suelta".into();
                    }
                    None => {
                        self.message =
                            "lanes: la pista no tiene; ponle una con pista:auto(...)".into();
                    }
                }
            }
            Panel::Files => {
                self.close_cursors();
                self.chosen_focus = Some(panel);
            }
            Panel::Mixer | Panel::Fx | Panel::Scope | Panel::Spectrum => {
                self.close_cursors();
                self.chosen_focus = Some(panel);
                self.dock_tab = Tab::Panel(panel);
                self.message = panel_keys(panel).unwrap_or_default().into();
            }
        }
    }

    /// Un clic en (`x`, `y`): en una pestaña, la elige (`true`: no hay más
    /// que hacer); en el editor sin el foco, se lo da (y nada más); en un
    /// panel sin cursor propio (mezcla, efectos, scope, espectro), le da el
    /// foco y el clic sigue (el del mezclador elige su tira). En los demás, el clic hace lo de siempre
    /// (abrir el cursor en una celda, mover una marca...).
    pub(super) fn focus_mouse(&mut self, x: u16, y: u16) -> bool {
        let at = ratatui::layout::Position::new(x, y);
        if let Some(&(_, tab)) = self.panels.tabs.iter().find(|(r, _)| r.contains(at)) {
            self.show_tab(tab);
            return true;
        }
        let Some(&(_, panel)) = self.panels.focus.iter().find(|(r, _)| r.contains(at)) else {
            return false;
        };
        match panel {
            // El primer clic en el editor solo le da el foco: no mueve el
            // cursor (desde el modo tocar no se edita sin querer).
            Panel::Code if self.focus() != Panel::Code => {
                self.set_focus(Panel::Code);
                return true;
            }
            Panel::Mixer | Panel::Fx | Panel::Scope | Panel::Spectrum
                if self.chosen_focus != Some(panel) =>
            {
                self.set_focus(panel)
            }
            _ => {}
        }
        false
    }

    /// Una tecla con el foco en un panel sin cursor: `Some` si era suya.
    /// Las letras y flechas que no usa se quedan aquí (no van al editor ni
    /// suenan); las combinaciones con Ctrl, Alt o F siguen su camino.
    pub(super) fn handle_chosen_focus(&mut self, input: &Input) -> Option<Flow> {
        let panel = self.chosen_focus?;
        let tracks = self.tracks().len();
        if input.ctrl || input.alt || matches!(input.key, Key::F(_)) {
            return None;
        }
        match input.key {
            Key::Esc => {
                self.chosen_focus = None;
                self.message = String::new();
            }
            Key::Left | Key::Right if tracks > 0 => {
                self.selected = if input.key == Key::Left {
                    (self.selected + tracks - 1) % tracks
                } else {
                    (self.selected + 1) % tracks
                };
            }
            Key::Char(c @ ('m' | 's')) if panel == Panel::Mixer && tracks > 0 => {
                self.toggle_live_at(self.selected.min(tracks - 1), c == 's');
            }
            _ => {}
        }
        Some(Flow::Continue)
    }
}
