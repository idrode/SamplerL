//! El cue (fase S): dónde empieza el siguiente arranque. Es un compás de
//! la canción (contando desde 0, como el de la barra de estado); con
//! secciones, el primero de una entrada de la línea de tiempo. Se fija con
//! Shift-Ctrl-Espacio (o F9): el compás que suena o, con la canción parada,
//! el del cursor del tracker (si no está abierto, donde se paró). Marcha
//! (Ctrl-Espacio) arranca en él con `Controller::start_at`, que lo hace
//! sonar como si la canción hubiera llegado hasta ahí. Se queda hasta que
//! se borra desde la paleta: cada arranque vuelve a él.

use super::App;

impl App {
    /// Fija el cue donde va la canción (ver el módulo).
    pub fn set_cue(&mut self) {
        let Some(bar) = self.cue_here() else {
            self.message = "cue: no hay canción cargada".into();
            return;
        };
        self.cue = Some(bar);
        self.message = format!(
            "cue en {}: el siguiente arranque empieza ahí (Ctrl-Espacio)",
            self.cue_text(bar)
        );
    }

    /// Quita el cue: se vuelve a arrancar desde el principio.
    pub fn clear_cue(&mut self) {
        self.message = if self.cue.take().is_some() {
            "cue borrado: se arranca desde el principio".into()
        } else {
            "no hay cue".into()
        };
    }

    /// El compás del cue que se fijaría ahora.
    fn cue_here(&self) -> Option<u64> {
        let song = self.host.song()?;
        let status = self.status;
        let running = status.is_some_and(|s| s.running);
        if !song.timeline.is_empty() {
            // Con secciones: el principio de la entrada que suena (o en la
            // que se paró).
            let entry = status
                .and_then(|s| s.section)
                .map_or(0, |p| p.entry)
                .min(song.timeline.len() - 1);
            return Some(entry_start(song, entry));
        }
        let bar_steps = song.bar_steps().max(1) as u64;
        if running {
            return Some(status.map_or(0, |s| s.bar));
        }
        // Parada: el paso del cursor del tracker en la vuelta que se ve; sin
        // cursor, el compás en que se paró.
        let global = status.map_or(0, |s| s.global);
        match self.tracker_edit {
            Some(cursor) => {
                let track = self.tracks().get(cursor.track)?;
                let (lap, _) = track.lap(global);
                let step = lap * track.length.max(1) as u64 + cursor.row as u64;
                Some(step / bar_steps)
            }
            None => Some(status.map_or(0, |s| s.bar)),
        }
    }

    /// El cue como se ve: `compás 8`, o con secciones `drop (entrada 3)`.
    pub(super) fn cue_text(&self, bar: u64) -> String {
        if let Some(song) = self.host.song()
            && let Some((entry, within)) = place_of(song, bar)
        {
            let name = &song.sections[song.timeline[entry].0].name;
            return if within == 0 {
                format!("{name} (entrada {})", entry + 1)
            } else {
                format!("{name} (entrada {}, compás {})", entry + 1, within + 1)
            };
        }
        format!("compás {bar}")
    }

    /// La entrada de la línea de tiempo en que cae el cue (para marcarla).
    pub(super) fn cue_entry(&self) -> Option<usize> {
        let song = self.host.song()?;
        place_of(song, self.cue?).map(|(entry, _)| entry)
    }
}

/// Primer compás (desde el principio de la canción) de la entrada `entry`.
fn entry_start(song: &script::Song, entry: usize) -> u64 {
    song.timeline[..entry]
        .iter()
        .map(|&(_, bars)| bars as u64)
        .sum()
}

/// La entrada y el compás dentro de ella donde cae el compás `bar` (la
/// línea de tiempo da la vuelta al acabar, como en el motor). `None` sin
/// secciones.
fn place_of(song: &script::Song, bar: u64) -> Option<(usize, u64)> {
    let total: u64 = song.timeline.iter().map(|&(_, b)| b as u64).sum();
    if total == 0 {
        return None;
    }
    let mut left = bar % total;
    for (entry, &(_, bars)) in song.timeline.iter().enumerate() {
        if left < bars as u64 {
            return Some((entry, left));
        }
        left -= bars as u64;
    }
    None
}
