//! Las visualizaciones pedidas y lo que se dibuja de ellas (ver
//! `crate::viz`): las de rejilla, del modelo del piano roll de su pista con
//! el cursor del resaltado en vivo; las de audio, de las muestras que copia
//! el motor, que se vacían en cada `tick`.

use crate::roll;
use crate::viz::{Placed, VizKind, VizSource, VizSpec, VizView, grid};

use super::App;

/// Bandas del espectro del master de su panel.
const MASTER_BANDS: usize = 32;

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
        self.set_specs(specs);
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
                        inline: v.inline,
                    })
                })
                .collect()
        });
        self.viz_sent = self.viz_flags(&specs);
        self.set_specs(specs);
    }

    /// Pone las visualizaciones; si aparecen las primeras de panel (no
    /// junto al código), elige su pestaña.
    fn set_specs(&mut self, specs: Vec<VizSpec>) {
        let panel = |specs: &[VizSpec]| specs.iter().any(|s| s.inline.is_none());
        if panel(&specs) && !panel(&self.viz) {
            self.show_tab(crate::layout::Tab::Viz);
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
    pub(super) fn viz_views(&self) -> Vec<Placed> {
        // En la ventana del editor, las que dibuja la principal.
        if let Some(remote) = self.remote_viz() {
            return remote;
        }
        self.viz
            .iter()
            .enumerate()
            .filter_map(|(i, spec)| {
                Some(Placed {
                    line: spec.inline,
                    view: self.viz_view(i, spec)?,
                })
            })
            .collect()
    }

    /// Las de aquí dibujadas en texto, para la ventana del editor: las de
    /// junto al código en [`crate::viz::INLINE_WIDTH`] columnas y las demás
    /// en [`crate::viz::REMOTE_WIDTH`].
    pub(super) fn viz_frames(&self) -> Vec<(Option<usize>, Vec<String>)> {
        self.viz_views()
            .iter()
            .map(|p| {
                let width = if p.line.is_some() {
                    crate::viz::INLINE_WIDTH
                } else {
                    crate::viz::REMOTE_WIDTH
                };
                (p.line, crate::viz::to_text(&p.view, width))
            })
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
            VizKind::Pitchwheel | VizKind::Spiral => {
                let history = self.viz_history(&spec.source)?;
                let (a, b) = history.as_slices();
                let samples: Vec<f32> = a.iter().chain(b).copied().collect();
                Some(if spec.kind == VizKind::Pitchwheel {
                    VizView::Wheel(crate::viz::wheel::wheel(
                        &samples,
                        self.sample_rate,
                        spec.title(),
                        spec.size,
                    ))
                } else {
                    VizView::Spiral(crate::viz::wheel::spiral(
                        &samples,
                        self.sample_rate,
                        spec.title(),
                        spec.size,
                    ))
                })
            }
        }
    }

    /// Las fuentes que pide la interfaz por su cuenta: el master, para el
    /// scope y el espectro de sus paneles (no en la ventana del editor,
    /// que no tiene motor).
    fn ui_viz(&self) -> [bool; engine::VIZ_SOURCES] {
        let mut on = [false; engine::VIZ_SOURCES];
        on[engine::VIZ_MASTER] = !self.editor_window();
        on
    }

    /// Enciende en el motor lo que piden el script y la interfaz juntos, si
    /// no es lo que ya tiene (tras cada carga, el lote del script deja solo
    /// lo suyo). Entra al compás, como cualquier lote; no cambia el audio:
    /// el motor solo copia a sus colas.
    pub(super) fn sync_ui_viz(&mut self) {
        let script = self.viz_flags(&self.viz);
        let ui = self.ui_viz();
        let wanted: [bool; engine::VIZ_SOURCES] = std::array::from_fn(|i| script[i] || ui[i]);
        if wanted == self.viz_sent {
            return;
        }
        let mut batch = engine::Batch::new();
        batch.set_viz(wanted);
        if self.controller.apply(batch).is_ok() {
            self.viz_sent = wanted;
        }
    }

    /// El scope y el espectro del master para sus paneles (vacío hasta que
    /// llega audio).
    pub(super) fn master_views(&self) -> Vec<VizView> {
        let Some(history) = self
            .viz_history
            .get(engine::VIZ_MASTER)
            .filter(|h| !h.is_empty())
        else {
            return Vec::new();
        };
        let (a, b) = history.as_slices();
        let samples: Vec<f32> = a.iter().chain(b).copied().collect();
        // Nivel eficaz de lo último (mono, tras el limitador): sin
        // ponderación, así que no es sonoridad (LUFS), solo una idea.
        let recent = &samples[samples.len().saturating_sub(self.sample_rate as usize / 3)..];
        let rms = (recent.iter().map(|s| s * s).sum::<f32>() / recent.len().max(1) as f32).sqrt();
        let rms = if rms > 0.0 {
            format!("RMS {:+.1} dB", 20.0 * rms.log10())
        } else {
            "RMS -∞".into()
        };
        vec![
            VizView::Scope(crate::viz::scope::build(
                &samples,
                format!("7 scope · master · {rms}"),
                4,
            )),
            VizView::Spectrum(crate::viz::spectrum::SpectrumView {
                title: "8 espectro · master".into(),
                size: 4,
                levels: self.master_spectrum.clone(),
            }),
        ]
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
        // El del master de su panel.
        if self.viz_sent[engine::VIZ_MASTER] {
            let history = &self.viz_history[engine::VIZ_MASTER];
            if !history.is_empty() {
                let (a, b) = history.as_slices();
                let samples: Vec<f32> = a.iter().chain(b).copied().collect();
                let fresh = crate::viz::spectrum::analyze(&samples, self.sample_rate, MASTER_BANDS);
                crate::viz::spectrum::fall(&mut self.master_spectrum, &fresh);
            }
        }
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
