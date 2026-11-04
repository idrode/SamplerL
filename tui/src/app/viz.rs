//! Las visualizaciones pedidas y lo que se dibuja de ellas (ver
//! `crate::viz`): las de rejilla, del modelo del piano roll de su pista con
//! el cursor del resaltado en vivo; las de audio, de las muestras que copia
//! el motor, que se vacían en cada `tick`.

use crate::roll;
use crate::viz::{VizKind, VizSource, VizSpec, VizView, grid};

use super::App;

impl App {
    /// Pone las visualizaciones `specs` y enciende en el motor las colas de
    /// las fuentes de audio que piden (entran al compás, con un lote). Para
    /// los tests: en uso, las pone el script ([`App::sync_viz`]).
    #[cfg(test)]
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

    /// Tras una carga buena, las visualizaciones de su modelo: el lote de
    /// la carga ya enciende sus fuentes en el motor.
    pub(super) fn sync_viz(&mut self) {
        let specs: Vec<VizSpec> = self.host.song().map_or_else(Vec::new, |song| {
            song.viz
                .iter()
                .filter_map(|v| {
                    Some(VizSpec {
                        kind: crate::viz::VizKind::from_name(v.kind)?,
                        source: match &v.track {
                            Some(name) => VizSource::Track(name.clone()),
                            None => VizSource::Master,
                        },
                        size: v.size,
                        bands: v.bands,
                    })
                })
                .collect()
        });
        self.viz_sent = self.viz_flags(&specs);
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
            .enumerate()
            .filter_map(|(i, spec)| self.viz_view(i, spec))
            .collect()
    }

    fn viz_view(&self, index: usize, spec: &VizSpec) -> Option<VizView> {
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
            VizKind::Scope => {
                let history = self.viz_history(&spec.source)?;
                let samples: Vec<f32> = history.iter().copied().collect();
                Some(VizView::Scope(crate::viz::scope::build(
                    &samples,
                    spec.title(),
                    spec.size,
                )))
            }
            VizKind::Spectrum => {
                self.viz_history(&spec.source)?;
                Some(VizView::Spectrum(crate::viz::spectrum::SpectrumView {
                    title: spec.title(),
                    size: spec.size,
                    levels: self.viz_spectra.get(index).cloned().unwrap_or_default(),
                }))
            }
            _ => None,
        }
    }

    /// Lo último que ha llegado de la fuente de `source`.
    fn viz_history(&self, source: &VizSource) -> Option<&std::collections::VecDeque<f32>> {
        let slot = match source {
            VizSource::Master => engine::VIZ_MASTER,
            VizSource::Track(name) => self.host.track_slot(name)?,
        };
        self.viz_history.get(slot)
    }

    /// Vacía las colas del motor de las fuentes encendidas en su historial
    /// (en cada `tick`).
    pub(super) fn tick_viz(&mut self) {
        let mut fresh = std::mem::take(&mut self.viz_scratch);
        for source in 0..engine::VIZ_SOURCES {
            if !self.viz_sent[source] {
                continue;
            }
            fresh.clear();
            self.controller.viz_samples(source, &mut fresh);
            let history = &mut self.viz_history[source];
            history.extend(fresh.iter().copied());
            let extra = history.len().saturating_sub(crate::viz::HISTORY);
            history.drain(..extra);
        }
        self.viz_scratch = fresh;
        // Los espectros, con su caída (un cuadro por `tick`).
        self.viz_spectra.resize(self.viz.len(), Vec::new());
        for i in 0..self.viz.len() {
            let spec = &self.viz[i];
            if spec.kind != VizKind::Spectrum {
                continue;
            }
            let Some(history) = self.viz_history(&spec.source) else {
                continue;
            };
            let (a, b) = history.as_slices();
            let samples: Vec<f32> = a.iter().chain(b).copied().collect();
            let fresh = crate::viz::spectrum::analyze(&samples, self.sample_rate, spec.bands);
            crate::viz::spectrum::fall(&mut self.viz_spectra[i], &fresh);
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
