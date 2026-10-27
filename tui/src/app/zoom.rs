//! Zoom de la vista del sample: se ve un trozo del sample (la mitad en
//! cada paso) para ajustar los cortes con más detalle. Con las teclas
//! (`+`/`-` en el modo tocar, Shift-←/→ para desplazarse) la vista sigue
//! al corte seleccionado; la rueda del ratón acerca y aleja alrededor de
//! donde se apunta y fija la vista ahí hasta que se elige otro corte con
//! las teclas. Solo la pista seleccionada: al cambiar de pista se ve
//! entera.
//!
//! También crear y borrar cortes en un sitio concreto: [`App::split_at`]
//! (clic derecho en la onda) y [`App::delete_slice`] (clic derecho en una
//! marca, o Supr con el seleccionado).

use std::ops::Range;
use std::sync::Arc;

use super::App;
use crate::model::Waveform;

/// Lo menos que se ve con el zoom, en frames (unos 6 ms a 44,1 kHz).
const MIN_WINDOW: usize = 256;

/// Resumen de la onda del trozo con zoom: (sample, trozo, resumen).
pub(crate) type ZoomedWave = (usize, Range<usize>, Arc<Waveform>);

/// El zoom de la pista seleccionada.
#[derive(Debug, Clone)]
pub(crate) struct WaveZoom {
    /// La pista (por nombre: en otra, se ve entera).
    track: String,
    /// Se ve 1/2^nivel del sample.
    level: u32,
    /// El frame del centro de la vista; `None`: el inicio del corte
    /// seleccionado.
    center: Option<usize>,
}

/// Los frames que se ven de un sample de `frames` con ese nivel y centro.
fn window_of(frames: usize, level: u32, center: usize) -> Range<usize> {
    let len = frames
        .checked_shr(level)
        .unwrap_or(0)
        .max(MIN_WINDOW)
        .min(frames);
    let start = center.saturating_sub(len / 2).min(frames - len);
    start..start + len
}

impl App {
    /// Los frames del sample de la pista seleccionada que se ven (con
    /// stretch, del estirado), o `None` si se ve entero.
    pub(crate) fn wave_window(&self) -> Option<Range<usize>> {
        let zoom = self.wave_zoom.as_ref()?;
        let track = self.tracks().get(self.selected)?;
        if track.name != zoom.track || zoom.level == 0 {
            return None;
        }
        let center = zoom.center.unwrap_or_else(|| {
            let index = self
                .selected_slice
                .min(track.slices.len().saturating_sub(1));
            track.slices.get(index).map_or(0, |r| r.start)
        });
        let window = window_of(track.sample.frames(), zoom.level, center);
        (window.len() < track.sample.frames()).then_some(window)
    }

    /// El resumen de la onda del trozo que se ve, si hay zoom (se guarda
    /// mientras no cambian el sample ni el trozo).
    pub(crate) fn zoomed_waveform(&self) -> Option<(Range<usize>, Arc<Waveform>)> {
        let window = self.wave_window()?;
        let track = self.tracks().get(self.selected)?;
        let key = Arc::as_ptr(&track.sample) as usize;
        let mut cache = self.zoom_cache.borrow_mut();
        if let Some((k, w, wave)) = cache.as_ref()
            && *k == key
            && *w == window
        {
            return Some((window, wave.clone()));
        }
        let wave = Arc::new(Waveform::window(&track.sample, window.clone()));
        *cache = Some((key, window.clone(), wave.clone()));
        Some((window, wave))
    }

    /// Frames del sample que se ven (con zoom o entero).
    pub(crate) fn shown_frames(&self) -> Range<usize> {
        self.wave_window().unwrap_or_else(|| {
            0..self
                .tracks()
                .get(self.selected)
                .map_or(0, |t| t.sample.frames())
        })
    }

    /// Acerca (o aleja) la vista del sample un paso. Con `anchor`, la
    /// fracción (0 a 1) del ancho donde está el ratón: ese punto del
    /// sample se queda debajo.
    pub fn zoom_wave(&mut self, closer: bool, anchor: Option<f64>) {
        let Some(track) = self.tracks().get(self.selected).cloned() else {
            self.message = "no hay pistas".into();
            return;
        };
        if track.slices.is_empty() || track.instrument.and_then(|i| i.synth).is_some() {
            self.message = "la pista no tiene sample".into();
            return;
        }
        let frames = track.sample.frames();
        let shown = self.shown_frames();
        let (level, center) = match &self.wave_zoom {
            Some(z) if z.track == track.name => (z.level, z.center),
            _ => (0, None),
        };
        let level = if closer {
            if shown.len() <= MIN_WINDOW.min(frames) {
                self.message = "zoom al máximo".into();
                return;
            }
            level + 1
        } else {
            if level == 0 {
                self.message = "ya se ve el sample entero".into();
                return;
            }
            level - 1
        };
        // Con el ratón, el punto apuntado se queda donde estaba.
        let center = match anchor {
            Some(f) => {
                let at = shown.start as f64 + f.clamp(0.0, 1.0) * shown.len() as f64;
                let len = window_of(frames, level, 0).len() as f64;
                Some((at - f * len + len / 2.0).max(0.0) as usize)
            }
            None => center,
        };
        self.wave_zoom = (level > 0).then(|| WaveZoom {
            track: track.name.clone(),
            level,
            center,
        });
        self.zoom_message();
    }

    /// Ve el sample entero.
    pub fn reset_wave_zoom(&mut self) {
        self.wave_zoom = None;
        self.message = "se ve el sample entero".into();
    }

    /// Desplaza la vista con zoom un cuarto de lo que se ve.
    pub fn pan_wave(&mut self, forward: bool) {
        let Some(window) = self.wave_window() else {
            self.message = "sin zoom: + acerca la vista del sample".into();
            return;
        };
        let quarter = window.len() / 4;
        let middle = window.start + window.len() / 2;
        let center = if forward {
            middle + quarter
        } else {
            middle.saturating_sub(quarter)
        };
        let frames = self.shown_frames_total();
        if let Some(zoom) = &mut self.wave_zoom {
            // Sin pasarse de los bordes (si no, no volvería enseguida).
            let half = window.len() / 2;
            zoom.center = Some(center.clamp(half, frames.saturating_sub(half).max(half)));
        }
        self.zoom_message();
    }

    /// Frames del sample de la pista seleccionada.
    fn shown_frames_total(&self) -> usize {
        self.tracks()
            .get(self.selected)
            .map_or(0, |t| t.sample.frames())
    }

    /// La vista vuelve a seguir al corte seleccionado (al elegirlo o
    /// moverlo con las teclas).
    pub(crate) fn follow_selected_slice(&mut self) {
        if let Some(zoom) = &mut self.wave_zoom {
            zoom.center = None;
        }
    }

    /// Fija la vista donde está (antes de tocarla con el ratón: así no
    /// salta al cambiar el corte seleccionado).
    pub(crate) fn pin_wave_view(&mut self) {
        let Some(window) = self.wave_window() else {
            return;
        };
        if let Some(zoom) = &mut self.wave_zoom {
            zoom.center = Some(window.start + window.len() / 2);
        }
    }

    fn zoom_message(&mut self) {
        let shown = self.shown_frames();
        let (from, to) = (self.frame_ms(shown.start), self.frame_ms(shown.end));
        self.message = match &self.wave_zoom {
            Some(z) => format!(
                "zoom ×{}: de {from:.1} a {to:.1} ms (+ - zoom · Shift-←→ desplazar)",
                1u64 << z.level.min(63)
            ),
            None => "se ve el sample entero".into(),
        };
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_window_halves_and_stays_inside_the_sample() {
        assert_eq!(window_of(10_000, 1, 5_000), 2_500..7_500);
        // Pegado a los bordes.
        assert_eq!(window_of(10_000, 1, 0), 0..5_000);
        assert_eq!(window_of(10_000, 2, 9_999), 7_500..10_000);
        // Nunca menos de MIN_WINDOW ni más que el sample.
        assert_eq!(window_of(10_000, 20, 5_000).len(), MIN_WINDOW);
        assert_eq!(window_of(100, 3, 50), 0..100);
    }
}
