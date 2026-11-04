// Hasta que lo pida el script (la API, más adelante), solo lo usan los tests.
#![allow(dead_code)]

//! Las visualizaciones pedidas y lo que se dibuja de ellas (ver
//! `crate::viz`): las de rejilla, del modelo del piano roll de su pista con
//! el cursor del resaltado en vivo; las de audio, de las muestras que copia
//! el motor, que se vacían en cada `tick`.

use crate::roll;
use crate::viz::{VizKind, VizSource, VizSpec, VizView, grid};

use super::App;

impl App {
    /// Pone las visualizaciones `specs` y enciende en el motor las colas de
    /// las fuentes de audio que piden (entran al compás, con un lote).
    pub fn set_viz(&mut self, specs: Vec<VizSpec>) {
        let flags = self.viz_flags(&specs);
        if flags != self.viz_sent {
            let mut batch = engine::Batch::new();
            batch.set_viz(flags);
            if self.controller.apply(batch).is_ok() {
                self.viz_sent = flags;
            }
        }
        self.viz = specs;
    }

    /// Qué fuentes del motor tienen que copiar muestras para `specs`.
    pub(super) fn viz_flags(&self, specs: &[VizSpec]) -> [bool; engine::VIZ_SOURCES] {
        let mut on = [false; engine::VIZ_SOURCES];
        for spec in specs.iter().filter(|s| s.kind.audio()) {
            match &spec.source {
                VizSource::Master => on[engine::VIZ_MASTER] = true,
                VizSource::Track(name) => {
                    if let Some(slot) = self.host.track_slot(name) {
                        on[slot] = true;
                    }
                }
            }
        }
        on
    }

    /// Lo que se dibuja de cada visualización.
    pub(super) fn viz_views(&self) -> Vec<VizView> {
        self.viz
            .iter()
            .filter_map(|spec| self.viz_view(spec))
            .collect()
    }

    fn viz_view(&self, spec: &VizSpec) -> Option<VizView> {
        match spec.kind {
            VizKind::Punchcard | VizKind::Pianoroll => {
                let VizSource::Track(name) = &spec.source else {
                    return None;
                };
                let song = self.host.song()?;
                let def = song.tracks.iter().find(|t| &t.name == name)?;
                let lines = self.editor.lines();
                let loaded = self.loaded_text.as_deref() == Some(self.text().as_str());
                let roll = roll::build(&roll::Source {
                    song,
                    def,
                    lines,
                    loaded,
                });
                let playhead = self.viz_playhead(def, roll.steps());
                Some(VizView::Grid(grid::build(
                    &roll,
                    spec.title(),
                    spec.size,
                    playhead,
                    spec.kind == VizKind::Punchcard,
                )))
            }
            _ => None,
        }
    }

    /// La columna del ciclo de `def` que se oye (como el cursor del piano
    /// roll: con la sincronía del resaltado).
    fn viz_playhead(&self, def: &script::TrackDef, cycle: usize) -> Option<usize> {
        let status = self.status.filter(|s| s.running)?;
        let cycle = cycle.max(1) as i64;
        let extra = self.highlight_delay_ms * status.bpm * 4.0 / 60_000.0;
        let heard = status.heard.map(|h| (h - extra).floor() as i64)?;
        let slot = self.host.track_slot(&def.name)?;
        if let Some(c) = self.cursors.at(slot, heard).filter(|c| c.tag == def.tag) {
            return Some((c.index as i64 + heard - c.global as i64).rem_euclid(cycle) as usize);
        }
        let (lap, step) = def.lap(super::section_global(status));
        let laps = def.bars.len().max(1) as u64;
        Some(((lap % laps) as usize * def.length.max(1) + step) % cycle as usize)
    }
}
