//! Secuenciador de 16 pasos: patrón de cortes, BPM y transporte.

use std::sync::Arc;

use crate::engine::SendError;
use crate::sample::Sample;
use crate::slice::Slices;

/// Pasos por compás (semicorcheas en 4/4).
pub const STEPS: usize = 16;
/// Pasos por negra.
const STEPS_PER_BEAT: f64 = 4.0;
pub const MIN_BPM: f64 = 20.0;
pub const MAX_BPM: f64 = 400.0;
pub const DEFAULT_BPM: f64 = 120.0;

/// Patrón de un compás. Los números de corte se resuelven a rangos de frames
/// en el hilo de control, así el motor solo copia este valor (sin reservas).
pub struct Pattern {
    pub(crate) sample: Arc<Sample>,
    pub(crate) steps: [Option<(usize, usize)>; STEPS],
}

impl Pattern {
    /// `steps[i]` es el número de corte que suena en el paso `i`, o `None`
    /// para no disparar nada (lo que sonaba sigue sonando).
    pub fn new(
        sample: Arc<Sample>,
        slices: &Slices,
        steps: [Option<usize>; STEPS],
    ) -> Result<Self, SendError> {
        let mut resolved = [None; STEPS];
        for (slot, step) in resolved.iter_mut().zip(steps) {
            if let Some(index) = step {
                let range = slices.get(index).ok_or(SendError::NoSuchSlice {
                    index,
                    count: slices.len(),
                })?;
                *slot = Some((range.start, range.end.min(sample.frames())));
            }
        }
        Ok(Pattern {
            sample,
            steps: resolved,
        })
    }
}

/// Frames de un paso a `bpm` y `sample_rate`.
pub(crate) fn frames_per_step(bpm: f64, sample_rate: u32) -> f64 {
    sample_rate as f64 * 60.0 / (bpm * STEPS_PER_BEAT)
}

/// Frames de un compás a `bpm` y `sample_rate`, con el mismo redondeo que
/// usa el transporte.
pub fn bar_frames(bpm: f64, sample_rate: u32) -> u64 {
    step_offset(STEPS, bpm, sample_rate)
}

pub(crate) fn valid_bpm(bpm: f64) -> bool {
    (MIN_BPM..=MAX_BPM).contains(&bpm)
}

/// Estado del transporte en el hilo de audio.
pub(crate) struct Transport {
    pub running: bool,
    /// Frames desde que arrancó el transporte.
    pub frame: u64,
    /// Frame en que empezó el compás actual.
    bar_start: u64,
    /// Siguiente paso a disparar dentro del compás.
    step: usize,
    bpm: f64,
    pub pattern: Option<Pattern>,
    /// Cambios que esperan al siguiente compás.
    pub pending_pattern: Option<Pattern>,
    pub pending_bpm: Option<f64>,
}

impl Transport {
    pub fn new() -> Self {
        Transport {
            running: false,
            frame: 0,
            bar_start: 0,
            step: 0,
            bpm: DEFAULT_BPM,
            pattern: None,
            pending_pattern: None,
            pending_bpm: None,
        }
    }

    /// Arranca desde el paso 0 del compás.
    pub fn start(&mut self) {
        self.running = true;
        self.frame = 0;
        self.bar_start = 0;
        self.step = 0;
    }

    /// Frame (absoluto) del siguiente paso.
    pub fn next_step_frame(&self, sample_rate: u32) -> u64 {
        self.bar_start + step_offset(self.step, self.bpm, sample_rate)
    }

    /// Avanza al siguiente paso. Al empezar un compás aplica los cambios
    /// pendientes (cuantización) y devuelve el patrón anterior si se sustituyó,
    /// para que el motor lo descarte fuera del callback. Devuelve también el
    /// rango a disparar en este paso, si lo hay.
    pub fn advance(&mut self, sample_rate: u32) -> (Option<Pattern>, Option<(usize, usize)>) {
        let mut replaced = None;
        if self.step == 0 {
            if let Some(bpm) = self.pending_bpm.take() {
                self.bpm = bpm;
            }
            if let Some(pattern) = self.pending_pattern.take() {
                replaced = self.pattern.replace(pattern);
            }
        }
        let hit = self.pattern.as_ref().and_then(|p| p.steps[self.step]);
        self.step += 1;
        if self.step == STEPS {
            self.bar_start += step_offset(STEPS, self.bpm, sample_rate);
            self.step = 0;
        }
        (replaced, hit)
    }
}

/// Desplazamiento en frames del paso `k` respecto al inicio del compás.
/// Se calcula desde el inicio del compás, sin acumular redondeos paso a paso.
fn step_offset(k: usize, bpm: f64, sample_rate: u32) -> u64 {
    (k as f64 * frames_per_step(bpm, sample_rate)).round() as u64
}
