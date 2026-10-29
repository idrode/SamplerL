//! La vista de ocupación por paso en la aplicación (ver
//! [`crate::occupancy`]): si se ve y la rejilla de lo que suena.

use super::App;
use crate::model::ViewModel;
use crate::occupancy::{self, Grid, Source};

/// Estado de la vista de ocupación.
#[derive(Debug, Default)]
pub struct Occupancy {
    /// Se ve (la paleta la enseña y la esconde).
    pub visible: bool,
}

impl App {
    /// La rejilla de la página que se oye, si la vista se ve.
    pub(super) fn occupancy_grid(&self, view: &ViewModel) -> Option<Grid> {
        if !self.occupancy.visible {
            return None;
        }
        let song = self.host.song()?;
        let audible: Vec<bool> = (0..view.tracks.len()).map(|i| view.audible(i)).collect();
        // Lo que se oye, con el mismo retardo a mano que el resaltado.
        let heard = self.status.and_then(|s| {
            let extra = self.highlight_delay_ms * s.bpm * 4.0 / 60_000.0;
            s.heard.map(|h| h - extra)
        });
        let slot = |name: &str| self.host.track_slot(name);
        Some(occupancy::grid(&Source {
            song,
            tracks: self.tracks(),
            cursors: &self.cursors,
            status: self.status,
            heard,
            slot: &slot,
            audible: &audible,
        }))
    }
}

impl App {
    /// Enseña o esconde la vista de ocupación.
    pub fn set_occupancy(&mut self, visible: bool) {
        self.occupancy.visible = visible;
    }
}
