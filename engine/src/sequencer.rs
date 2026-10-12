//! Secuenciador de 16 pasos: patrón de cortes, BPM y transporte.

use std::sync::Arc;

use crate::engine::{Playback, SendError};
use crate::sample::Sample;
use crate::slice::Slices;

/// Pasos por compás (semicorcheas en 4/4).
pub const STEPS: usize = 16;
/// Pasos por negra.
const STEPS_PER_BEAT: f64 = 4.0;
pub const MIN_BPM: f64 = 20.0;
pub const MAX_BPM: f64 = 400.0;
pub const DEFAULT_BPM: f64 = 120.0;
/// Ganancia máxima de un paso (+12 dB).
pub const MAX_GAIN: f32 = 4.0;
/// Transposición máxima de un paso, en semitonos (arriba o abajo).
pub const MAX_SEMITONES: f32 = 36.0;

/// Un paso del patrón: qué corte suena y cómo.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Step {
    /// Número de corte (desde 0).
    pub slice: usize,
    /// Ganancia lineal, de 0 a `MAX_GAIN`.
    pub gain: f32,
    /// Transposición en semitonos. Cambia la velocidad de lectura, así que
    /// también la duración (sin time-stretch).
    pub semitones: f32,
    /// Reproducir el corte del final al principio.
    pub reverse: bool,
}

impl Step {
    pub fn new(slice: usize) -> Self {
        Step {
            slice,
            gain: 1.0,
            semitones: 0.0,
            reverse: false,
        }
    }

    pub fn with_gain(self, gain: f32) -> Self {
        Step { gain, ..self }
    }

    pub fn with_pitch(self, semitones: f32) -> Self {
        Step { semitones, ..self }
    }

    pub fn reversed(self) -> Self {
        Step {
            reverse: true,
            ..self
        }
    }
}

/// Paso ya resuelto: rango de frames y forma de reproducirlo.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Trigger {
    pub start: usize,
    pub end: usize,
    pub playback: Playback,
}

/// Patrón de un compás. Los pasos se resuelven a rangos de frames y
/// parámetros de reproducción en el hilo de control, así el motor solo copia
/// este valor (sin reservas).
pub struct Pattern {
    pub(crate) sample: Arc<Sample>,
    pub(crate) steps: [Option<Trigger>; STEPS],
}

impl Pattern {
    /// `steps[i]` es el número de corte que suena en el paso `i`, o `None`
    /// para no disparar nada (lo que sonaba sigue sonando).
    pub fn new(
        sample: Arc<Sample>,
        slices: &Slices,
        steps: [Option<usize>; STEPS],
    ) -> Result<Self, SendError> {
        Self::from_steps(sample, slices, steps.map(|s| s.map(Step::new)))
    }

    /// Como [`Pattern::new`], con volumen, pitch y reverse por paso.
    pub fn from_steps(
        sample: Arc<Sample>,
        slices: &Slices,
        steps: [Option<Step>; STEPS],
    ) -> Result<Self, SendError> {
        let mut resolved = [None; STEPS];
        for (i, (slot, step)) in resolved.iter_mut().zip(steps).enumerate() {
            let Some(step) = step else { continue };
            let range = slices.get(step.slice).ok_or(SendError::NoSuchSlice {
                index: step.slice,
                count: slices.len(),
            })?;
            let invalid = |reason| SendError::InvalidStep { step: i, reason };
            if !(0.0..=MAX_GAIN).contains(&step.gain) {
                return Err(invalid("ganancia fuera de rango (0 a 4)"));
            }
            if !(-MAX_SEMITONES..=MAX_SEMITONES).contains(&step.semitones) {
                return Err(invalid("pitch fuera de rango (-36 a 36 semitonos)"));
            }
            *slot = Some(Trigger {
                start: range.start,
                end: range.end.min(sample.frames()),
                playback: Playback {
                    gain: step.gain,
                    rate: 2f64.powf(step.semitones as f64 / 12.0),
                    reverse: step.reverse,
                },
            });
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
    pub fn advance(&mut self, sample_rate: u32) -> (Option<Pattern>, Option<Trigger>) {
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
