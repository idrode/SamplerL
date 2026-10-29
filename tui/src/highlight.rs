//! Resaltado en vivo del código al sonar (fase M): los tokens de la
//! mini-notación que se oyen ahora, sobre el propio texto del editor.
//!
//! El motor publica por pista el último paso que pasó por ella con la
//! etiqueta de su patrón ([`engine::Cursor`]) y lo que se oye por los
//! altavoces ([`engine::Status::heard`]). Aquí se guardan los últimos
//! cursores de cada pista ([`Cursors`]), se elige el del paso que se oye y,
//! con la etiqueta, la definición de pista de la carga ([`Song::track_by_tag`])
//! que dice dónde está escrito ([`script::TrackDef::sounding_span`]).

use std::collections::VecDeque;
use std::ops::Range;

use engine::{Cursor, MAX_TRACKS, Status};
use script::Song;

/// Cursores que se recuerdan por pista: los de los últimos pasos (la
/// latencia de la salida es menor que eso).
const KEEP: usize = 8;
/// Pasos que puede tener de retraso el cursor elegido respecto al que se
/// oye (si la interfaz se perdió alguno); más, ya no es lo que suena.
const STALE: i64 = 2;

/// Un token que suena.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Highlight {
    /// Fila (desde 0) y columnas en caracteres.
    pub row: usize,
    pub cols: Range<usize>,
    /// Color: el orden de la pista en el script.
    pub color: usize,
    /// Es la pista seleccionada (más intensa).
    pub selected: bool,
}

/// Los últimos cursores vistos de cada pista del motor, del más viejo al
/// más nuevo.
#[derive(Debug, Default)]
pub struct Cursors {
    seen: [VecDeque<Cursor>; MAX_TRACKS],
}

impl Cursors {
    /// Apunta los cursores de una lectura del estado.
    pub fn observe(&mut self, status: &Status) {
        for (seen, track) in self.seen.iter_mut().zip(&status.tracks) {
            let Some(cursor) = track.cursor else {
                continue;
            };
            match seen.back() {
                Some(last) if *last == cursor => continue,
                // Volvió a arrancar: lo de antes ya no vale.
                Some(last) if cursor.global < last.global => seen.clear(),
                _ => {}
            }
            if seen.len() == KEEP {
                seen.pop_front();
            }
            seen.push_back(cursor);
        }
    }

    /// El cursor de la pista `slot` en la semicorchea `step`: el último que
    /// empezó en ella o antes (si no es demasiado viejo).
    pub fn at(&self, slot: usize, step: i64) -> Option<Cursor> {
        self.seen
            .get(slot)?
            .iter()
            .rev()
            .find(|c| c.global as i64 <= step)
            .filter(|c| step - (c.global as i64) < STALE)
            .copied()
    }

    /// El último cursor visto de la pista `slot`, sea de cuando sea.
    pub fn last(&self, slot: usize) -> Option<Cursor> {
        self.seen.get(slot)?.back().copied()
    }
}

/// Los tokens que suenan en la semicorchea que se oye (`heard`), en el orden
/// en que se dibujan: el último manda si dos marcan las mismas celdas. Así
/// manda la pista seleccionada y, entre las demás, la definida antes.
/// `slot` da la pista del motor de cada nombre.
pub fn highlights(
    song: &Song,
    cursors: &Cursors,
    heard: Option<f64>,
    slot: impl Fn(&str) -> Option<usize>,
    selected: Option<&str>,
) -> Vec<Highlight> {
    let Some(step) = heard.map(|h| h.floor() as i64).filter(|s| *s >= 0) else {
        return Vec::new();
    };
    let mut out: Vec<Highlight> = song
        .track_names()
        .iter()
        .enumerate()
        .filter_map(|(color, &name)| {
            let cursor = cursors.at(slot(name)?, step)?;
            let track = song.track_by_tag(cursor.tag).filter(|t| t.name == name)?;
            let span = track.sounding_span(cursor.index)?;
            Some(Highlight {
                row: span.row,
                cols: span.start..span.end,
                color,
                selected: Some(name) == selected,
            })
        })
        .collect();
    // Las definidas antes, al final (encima); la seleccionada, la última.
    out.sort_by_key(|h| (h.selected, std::cmp::Reverse(h.color)));
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn status(cursors: &[(usize, Cursor)]) -> Status {
        let mut status = Status {
            running: true,
            bpm: 120.0,
            bar: 0,
            step: Some(0),
            global: 0,
            bar_steps: 16,
            tracks: Default::default(),
            groups: Default::default(),
            master: Default::default(),
            section: None,
            section_lost: false,
            heard: None,
        };
        for &(slot, cursor) in cursors {
            status.tracks[slot].cursor = Some(cursor);
        }
        status
    }

    fn cursor(global: u64) -> Cursor {
        Cursor {
            tag: 1,
            index: global as usize % 16,
            global,
        }
    }

    #[test]
    fn the_cursor_of_the_heard_step_and_not_a_stale_one() {
        let mut cursors = Cursors::default();
        for g in 0..12 {
            cursors.observe(&status(&[(0, cursor(g))]));
            // Leído dos veces: no se repite.
            cursors.observe(&status(&[(0, cursor(g))]));
        }
        assert_eq!(cursors.seen[0].len(), KEEP);
        // Se oye el paso 9 aunque el motor vaya por el 11.
        assert_eq!(cursors.at(0, 9), Some(cursor(9)));
        // Uno que se perdió: el anterior, si es de hace poco.
        cursors.seen[0].retain(|c| c.global != 10);
        assert_eq!(cursors.at(0, 10), Some(cursor(9)));
        assert_eq!(cursors.at(0, 20), None);
        // Antes de los que se recuerdan, nada; en otra pista, nada.
        assert_eq!(cursors.at(0, 2), None);
        assert_eq!(cursors.at(1, 9), None);
        // Al volver a arrancar se olvidan los de antes.
        cursors.observe(&status(&[(0, cursor(0))]));
        assert_eq!(cursors.seen[0].len(), 1);
    }
}
