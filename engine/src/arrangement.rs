//! Secciones y línea de tiempo: el motor lleva la canción entera.
//!
//! Un [`Arrangement`] son secciones (los patrones de cada pista, ya
//! resueltos en el hilo de control) y una línea de tiempo que las encadena
//! con sus compases. Entra con un lote ([`Batch::set_arrangement`]) al
//! empezar un compás, como todo. Desde ahí, al empezar cada compás el
//! motor mira en qué sección está: si cambia, mueve los patrones de la
//! sección que acaba a su sitio en el arreglo y pone los de la nueva (sin
//! reservar ni liberar memoria: son movimientos). Al acabar la línea de
//! tiempo vuelve a empezar.
//!
//! Dentro de una sección todo cuenta desde su primer compás: las vueltas
//! de los patrones (`every`, `chance`, `fill`, `humanize`, `:length()`) y
//! las curvas de los efectos y de los granos. El tempo, su rampa y el
//! compás son de la canción.
//!
//! Al cambiar de arreglo (recargar el script), la posición se conserva por
//! sección: la misma entrada de la línea de tiempo si sigue siendo la
//! misma sección, si no la primera entrada de esa sección, con el mismo
//! compás; si la sección ya no está, el principio (y el estado lo dice).
//!
//! [`Batch::set_arrangement`]: crate::Batch::set_arrangement

use crate::engine::{MAX_TRACKS, SendError};
use crate::sequencer::Pattern;

/// Secciones como mucho en un arreglo, y entradas en su línea de tiempo.
pub const MAX_SECTIONS: usize = 64;
pub const MAX_ENTRIES: usize = 256;

/// Una sección: lo que toca cada pista mientras suena.
pub(crate) struct Section {
    /// Identifica la sección entre recargas (el anfitrión lo saca de su
    /// nombre).
    pub id: u64,
    /// Patrón de cada pista; el de la sección que suena está en la pista
    /// (aquí, `None` mientras tanto).
    pub patterns: [Option<Pattern>; MAX_TRACKS],
}

/// Secciones y línea de tiempo de una canción.
pub struct Arrangement {
    pub(crate) sections: Vec<Section>,
    /// (sección, compases) en orden.
    pub(crate) timeline: Vec<(usize, u32)>,
}

impl Default for Arrangement {
    fn default() -> Self {
        Self::new()
    }
}

impl Arrangement {
    pub fn new() -> Self {
        Arrangement {
            sections: Vec::new(),
            timeline: Vec::new(),
        }
    }

    /// Añade una sección con los patrones de sus pistas (`(pista,
    /// patrón)`; las que no salen, calladas) y devuelve su índice.
    pub fn add_section(
        &mut self,
        id: u64,
        patterns: Vec<(usize, Pattern)>,
    ) -> Result<usize, SendError> {
        if self.sections.len() >= MAX_SECTIONS {
            return Err(SendError::InvalidArrangement(
                "demasiadas secciones (máximo 64)",
            ));
        }
        let mut slots: [Option<Pattern>; MAX_TRACKS] = Default::default();
        for (track, pattern) in patterns {
            let slot = slots.get_mut(track).ok_or(SendError::NoSuchTrack(track))?;
            *slot = Some(pattern);
        }
        self.sections.push(Section {
            id,
            patterns: slots,
        });
        Ok(self.sections.len() - 1)
    }

    /// Añade a la línea de tiempo `bars` compases de la sección `section`.
    pub fn push(&mut self, section: usize, bars: u32) -> Result<(), SendError> {
        if section >= self.sections.len() {
            return Err(SendError::InvalidArrangement("la sección no existe"));
        }
        if bars == 0 {
            return Err(SendError::InvalidArrangement(
                "una sección dura al menos un compás",
            ));
        }
        if self.timeline.len() >= MAX_ENTRIES {
            return Err(SendError::InvalidArrangement(
                "la línea de tiempo tiene como mucho 256 entradas",
            ));
        }
        self.timeline.push((section, bars));
        Ok(())
    }

    pub(crate) fn validate(&self) -> Result<(), SendError> {
        if self.timeline.is_empty() {
            return Err(SendError::InvalidArrangement(
                "la línea de tiempo está vacía",
            ));
        }
        Ok(())
    }

    /// Entradas de la línea de tiempo.
    pub fn len(&self) -> usize {
        self.timeline.len()
    }

    pub fn is_empty(&self) -> bool {
        self.timeline.is_empty()
    }

    /// Compases de la línea de tiempo entera.
    pub fn bars(&self) -> u64 {
        self.timeline.iter().map(|&(_, b)| b as u64).sum()
    }

    /// Dónde cae el compás `bar` de la canción (contando desde el
    /// principio de la línea de tiempo, que da la vuelta al acabar).
    pub(crate) fn place_at(&self, bar: u64) -> Place {
        let total = self.bars();
        let mut left = if total == 0 { 0 } else { bar % total };
        for (entry, &(_, bars)) in self.timeline.iter().enumerate() {
            if left < bars as u64 {
                return Place {
                    entry,
                    bar: left as u32,
                };
            }
            left -= bars as u64;
        }
        Place { entry: 0, bar: 0 }
    }

    fn id_of(&self, entry: usize) -> u64 {
        self.sections[self.timeline[entry].0].id
    }
}

/// Dónde está la canción en la línea de tiempo.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Place {
    pub entry: usize,
    /// Compás dentro de la entrada (desde 0).
    pub bar: u32,
}

/// El arreglo que suena y por dónde va.
pub(crate) struct Playlist {
    pub arrangement: Arrangement,
    /// La entrada y el compás del compás que suena.
    pub place: Place,
    /// Sección cuyos patrones están en las pistas.
    pub current: usize,
    /// Semicorchea y frame de la canción en que empezó la entrada.
    pub origin_step: u64,
    pub origin_frame: u64,
    /// Repetir la entrada actual en vez de seguir.
    pub looping: bool,
    /// Ir a esta entrada al empezar el compás siguiente.
    pub jump: Option<usize>,
    /// Volver al principio al arrancar el transporte.
    pub restart: bool,
}

impl Playlist {
    /// Dónde cae el compás siguiente: el siguiente de la entrada, o la
    /// entrada siguiente (la misma si se repite, la primera al acabar), o
    /// la del salto.
    pub fn next(&self) -> Place {
        if let Some(entry) = self.jump {
            return Place { entry, bar: 0 };
        }
        let (_, bars) = self.arrangement.timeline[self.place.entry];
        if self.place.bar + 1 < bars {
            return Place {
                entry: self.place.entry,
                bar: self.place.bar + 1,
            };
        }
        let entry = if self.looping {
            self.place.entry
        } else {
            (self.place.entry + 1) % self.arrangement.timeline.len()
        };
        Place { entry, bar: 0 }
    }
}

/// Dónde seguir en `new` lo que en `old` iba a sonar en `place`: la misma
/// entrada si es la misma sección, si no la primera de esa sección, con el
/// mismo compás (si no cabe, la entrada siguiente). `None`: la sección ya
/// no está.
pub(crate) fn resume(old: &Arrangement, place: Place, new: &Arrangement) -> Option<Place> {
    let id = old.id_of(place.entry);
    let entry = if place.entry < new.timeline.len() && new.id_of(place.entry) == id {
        place.entry
    } else {
        (0..new.timeline.len()).find(|&e| new.id_of(e) == id)?
    };
    let (_, bars) = new.timeline[entry];
    Some(if place.bar < bars {
        Place {
            entry,
            bar: place.bar,
        }
    } else {
        Place {
            entry: (entry + 1) % new.timeline.len(),
            bar: 0,
        }
    })
}
