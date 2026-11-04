//! El ratón en el tracker, la vista de ocupación y el mezclador. Cada
//! panel guarda al dibujarse su geometría (`ui::TrackerLayout`,
//! `ui::OccupancyLayout`, `ui::MixerLayout`, la misma con la que se
//! dibuja), y aquí se traduce un clic a lo que ya hacen las teclas:
//!
//! - **Tracker**: un clic en el nombre de una pista la elige (como Ctrl-O)
//!   y el clic derecho le pone o le quita el mute en vivo (como `m`). Un
//!   clic en una celda abre ahí el cursor de edición (como «Editar el
//!   tracker» y las flechas); otro clic en la celda del cursor abre sus
//!   efectos en la línea de órdenes (como Enter). Con el cursor abierto,
//!   la rueda lo sube y lo baja.
//! - **Vista de ocupación**: igual, con su cursor para rellenar: un clic
//!   en una celda lo pone ahí, otro clic en la misma abre `rellenar` (como
//!   Enter); el nombre elige la pista y el clic derecho en él, el mute. La
//!   rueda mueve el cursor de pista en pista, y Shift y la rueda (o la
//!   horizontal), de paso en paso.
//! - **Mezclador** (la ventana principal sin editor): un clic en una tira
//!   elige la pista; en su `M` o su `S`, mute o solo en vivo.
//!
//! Nada de esto suena ni escribe en el código por sí solo: lo que escribe
//! es la línea de órdenes que abre el segundo clic, como con las teclas.
//! Solo sin nada abierto encima (paleta, línea de órdenes...), en los dos
//! modos y en la ventana principal con el editor fuera.

use ratatui::layout::{Position, Rect};

use super::occupancy::OccupancyCursor;
use super::tracker_edit::TrackerCursor;
use super::{App, MouseAction};
use crate::ui::{MixerHit, MixerLayout, OccupancyHit, OccupancyLayout, TrackerHit, TrackerLayout};

/// Dónde quedaron los paneles en el último dibujo.
#[derive(Debug, Default, Clone)]
pub struct PanelAreas {
    pub tracker: Option<TrackerLayout>,
    pub occupancy: Option<OccupancyLayout>,
    pub mixer: Option<MixerLayout>,
    /// Dónde quedó cada panel (para darle el foco con un clic).
    pub focus: Vec<(Rect, crate::focus::Panel)>,
    /// Las pestañas del dock.
    pub tabs: Vec<(Rect, crate::layout::Tab)>,
}

impl App {
    /// El ratón sobre uno de estos paneles: `true` si era suyo.
    pub(super) fn panels_mouse(&mut self, action: MouseAction, x: u16, y: u16) -> bool {
        // Arrastrar y soltar no son de estos paneles: así una selección del
        // editor que pase por encima sigue.
        if !self.nothing_on_top() || matches!(action, MouseAction::Drag | MouseAction::Up) {
            return false;
        }
        let at = Position::new(x, y);
        if let Some(hit) = self.panels.occupancy.and_then(|l| l.hit(x, y)) {
            self.occupancy_mouse(action, hit);
            return true;
        }
        if let Some(hit) = self.panels.tracker.and_then(|l| l.hit(x, y)) {
            self.tracker_mouse(action, hit);
            return true;
        }
        if let Some(hit) = self.panels.mixer.and_then(|l| l.hit(x, y)) {
            self.mixer_mouse(action, hit);
            return true;
        }
        // La rueda sobre el tracker o la vista, fuera de sus celdas (el
        // borde, los números de paso), también mueve su cursor.
        let inside = |r: Option<Rect>| r.is_some_and(|r| r.contains(at));
        if matches!(action, MouseAction::ScrollUp | MouseAction::ScrollDown)
            && inside(self.panels.tracker.map(|l| l.inner))
            && self.tracker_edit.is_some()
        {
            self.tracker_wheel(action);
            return true;
        }
        false
    }

    /// Elige la pista `index` (como Ctrl-O hasta ella).
    fn select_track_at(&mut self, index: usize) {
        if index < self.tracks().len() && index != self.selected {
            self.release_pads();
            self.selected = index;
            self.selected_slice = 0;
        }
        if let Some(name) = self.tracks().get(index).map(|t| t.name.clone()) {
            self.message = format!("pista {name}");
        }
    }

    fn tracker_mouse(&mut self, action: MouseAction, hit: TrackerHit) {
        match (action, hit) {
            (MouseAction::Down, TrackerHit::Header(i)) => {
                self.select_track_at(i);
                if let Some(cursor) = self.tracker_edit {
                    let row = cursor.row.min(self.track_rows(i).saturating_sub(1));
                    let cursor = TrackerCursor { track: i, row };
                    self.tracker_edit = Some(cursor);
                    self.message = self.step_summary(cursor);
                }
            }
            (MouseAction::RightDown, TrackerHit::Header(i)) => self.toggle_live_at(i, false),
            (MouseAction::Down, TrackerHit::Cell(i, step)) => {
                if step >= self.track_rows(i) {
                    // Más allá de lo que dura esa pista: solo la elige.
                    self.select_track_at(i);
                    return;
                }
                let cursor = TrackerCursor {
                    track: i,
                    row: step,
                };
                if self.tracker_edit == Some(cursor) {
                    self.start_step_edit(cursor);
                    return;
                }
                self.select_track_at(i);
                self.tracker_edit = Some(cursor);
                self.occupancy.cursor = None;
                self.roll.cursor = None;
                self.message = self.step_summary(cursor);
            }
            (MouseAction::ScrollUp | MouseAction::ScrollDown, _) => self.tracker_wheel(action),
            _ => {}
        }
    }

    /// La rueda con el cursor del tracker abierto: un paso arriba o abajo.
    fn tracker_wheel(&mut self, action: MouseAction) {
        let Some(mut cursor) = self.tracker_edit else {
            return;
        };
        let last = self.track_rows(cursor.track).saturating_sub(1);
        cursor.row = match action {
            MouseAction::ScrollUp => cursor.row.saturating_sub(1),
            _ => (cursor.row + 1).min(last),
        };
        self.tracker_edit = Some(cursor);
        self.message = self.step_summary(cursor);
    }

    fn occupancy_mouse(&mut self, action: MouseAction, hit: OccupancyHit) {
        let Some(grid) = self.current_grid() else {
            return;
        };
        let (rows, cols) = (grid.rows.len(), grid.steps());
        if rows == 0 || cols == 0 {
            return;
        }
        match (action, hit) {
            (MouseAction::Down, OccupancyHit::Name(r)) => {
                self.select_track_at(r);
                if let Some(mut cursor) = self.occupancy.cursor {
                    cursor.row = r.min(rows - 1);
                    self.occupancy.cursor = Some(cursor);
                    self.message = self.fill_summary(&grid, cursor);
                }
            }
            (MouseAction::RightDown, OccupancyHit::Name(r)) => self.toggle_live_at(r, false),
            (MouseAction::Down, OccupancyHit::Cell(r, c)) => {
                let cursor = OccupancyCursor {
                    row: r.min(rows - 1),
                    col: c.min(cols - 1),
                };
                if self.occupancy.cursor == Some(cursor) {
                    self.start_fill(&grid, cursor);
                    return;
                }
                self.select_track_at(cursor.row);
                self.tracker_edit = None;
                self.pending_edit = None;
                self.roll.cursor = None;
                self.occupancy.cursor = Some(cursor);
                self.message = self.fill_summary(&grid, cursor);
            }
            (
                MouseAction::ScrollUp
                | MouseAction::ScrollDown
                | MouseAction::ScrollLeft
                | MouseAction::ScrollRight,
                _,
            ) => {
                let Some(mut cursor) = self.occupancy.cursor else {
                    return;
                };
                match action {
                    MouseAction::ScrollUp => cursor.row = cursor.row.saturating_sub(1),
                    MouseAction::ScrollDown => cursor.row = (cursor.row + 1).min(rows - 1),
                    MouseAction::ScrollLeft => cursor.col = cursor.col.saturating_sub(1),
                    _ => cursor.col = (cursor.col + 1).min(cols - 1),
                }
                self.selected = cursor.row;
                self.occupancy.cursor = Some(cursor);
                self.message = self.fill_summary(&grid, cursor);
            }
            _ => {}
        }
    }

    fn mixer_mouse(&mut self, action: MouseAction, hit: MixerHit) {
        if action != MouseAction::Down {
            return;
        }
        match hit {
            MixerHit::Strip(i) => self.select_track_at(i),
            MixerHit::Mute(i) => self.toggle_live_at(i, false),
            MixerHit::Solo(i) => self.toggle_live_at(i, true),
        }
    }
}
