//! Secuenciador de 16 pasos: patrón de cortes, BPM y transporte.

use std::sync::Arc;

use crate::engine::{Playback, SendError};
use crate::fx::{Fx, MAX_FX};
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
/// Repeticiones máximas de un paso con retrigger.
pub const MAX_RETRIG: u8 = 16;
/// Compases máximos del ciclo de variantes de un patrón.
pub const MAX_CYCLE_BARS: usize = 64;

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
    /// Retrigger/stutter: el paso se divide en `retrig` partes iguales y el
    /// corte vuelve a empezar en cada una. 1 = sin retrigger.
    pub retrig: u8,
}

impl Step {
    pub fn new(slice: usize) -> Self {
        Step {
            slice,
            gain: 1.0,
            semitones: 0.0,
            reverse: false,
            retrig: 1,
        }
    }

    pub fn with_retrig(self, retrig: u8) -> Self {
        Step { retrig, ..self }
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
    pub retrig: u8,
}

/// Patrón de un compás. Los pasos se resuelven a rangos de frames y
/// parámetros de reproducción en el hilo de control, así el motor solo copia
/// este valor (sin reservas).
pub struct Pattern {
    pub(crate) sample: Arc<Sample>,
    /// Ciclo de compases: en el compás `b` de la canción suena
    /// `bars[b % bars.len()]`. Un patrón fijo tiene un solo compás.
    pub(crate) bars: Vec<[Option<Trigger>; STEPS]>,
    /// Volumen de la pista (lineal), aplicado después de sus voces.
    pub(crate) gain: f32,
    /// Efectos de la pista, en orden.
    pub(crate) fx: [Option<Fx>; MAX_FX],
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
        Self::from_bars(sample, slices, &[steps])
    }

    /// Patrón de varios compases que se repiten en ciclo: en el compás `b`
    /// de la canción suena `bars[b % bars.len()]` (de 1 a `MAX_CYCLE_BARS`).
    pub fn from_bars(
        sample: Arc<Sample>,
        slices: &Slices,
        bars: &[[Option<Step>; STEPS]],
    ) -> Result<Self, SendError> {
        if bars.is_empty() || bars.len() > MAX_CYCLE_BARS {
            return Err(SendError::InvalidCycle(bars.len()));
        }
        let resolved = bars
            .iter()
            .map(|steps| resolve_bar(&sample, slices, steps))
            .collect::<Result<Vec<_>, _>>()?;
        Ok(Pattern {
            sample,
            bars: resolved,
            gain: 1.0,
            fx: [None; MAX_FX],
        })
    }

    /// Cadena de efectos de la pista (hasta `MAX_FX`, en orden). Entra con el
    /// patrón, al empezar el compás.
    pub fn with_fx(mut self, chain: &[Fx]) -> Result<Self, SendError> {
        if chain.len() > MAX_FX {
            return Err(SendError::InvalidFx(
                "demasiados efectos (máximo 4 por pista)",
            ));
        }
        let mut slots = [None; MAX_FX];
        for (slot, fx) in slots.iter_mut().zip(chain) {
            fx.validate().map_err(SendError::InvalidFx)?;
            *slot = Some(*fx);
        }
        self.fx = slots;
        Ok(self)
    }

    /// Volumen de la pista que toca este patrón (lineal, de 0 a `MAX_GAIN`).
    /// Entra con el patrón, al empezar el compás, con un suavizado de ~5 ms.
    pub fn with_gain(mut self, gain: f32) -> Result<Self, SendError> {
        if !(0.0..=MAX_GAIN).contains(&gain) {
            return Err(SendError::InvalidTrackGain(gain));
        }
        self.gain = gain;
        Ok(self)
    }
}

/// Resuelve los pasos de un compás a rangos de frames y parámetros.
fn resolve_bar(
    sample: &Sample,
    slices: &Slices,
    steps: &[Option<Step>; STEPS],
) -> Result<[Option<Trigger>; STEPS], SendError> {
    let mut resolved = [None; STEPS];
    for (i, (slot, step)) in resolved.iter_mut().zip(steps.iter().copied()).enumerate() {
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
        if !(1..=MAX_RETRIG).contains(&step.retrig) {
            return Err(invalid("retrigger fuera de rango (1 a 16)"));
        }
        *slot = Some(Trigger {
            start: range.start,
            end: range.end.min(sample.frames()),
            playback: Playback {
                gain: step.gain,
                rate: 2f64.powf(step.semitones as f64 / 12.0),
                reverse: step.reverse,
            },
            retrig: step.retrig,
        });
    }
    Ok(resolved)
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

/// Si `bpm` está en el rango admitido (`MIN_BPM`..=`MAX_BPM`).
pub fn valid_bpm(bpm: f64) -> bool {
    (MIN_BPM..=MAX_BPM).contains(&bpm)
}

/// Reloj del secuenciador en el hilo de audio: compás, paso y tempo. Los
/// patrones son de cada pista y viven en el motor.
pub(crate) struct Transport {
    pub running: bool,
    /// Frames desde que arrancó el transporte.
    pub frame: u64,
    /// Frame en que empezó el compás actual.
    bar_start: u64,
    /// Siguiente paso a disparar dentro del compás.
    step: usize,
    /// Compases completos desde que arrancó el transporte.
    bar: u64,
    bpm: f64,
    /// Tempo que espera al siguiente compás.
    pub pending_bpm: Option<f64>,
}

impl Transport {
    pub fn new() -> Self {
        Transport {
            running: false,
            frame: 0,
            bar_start: 0,
            step: 0,
            bar: 0,
            bpm: DEFAULT_BPM,
            pending_bpm: None,
        }
    }

    /// Arranca desde el paso 0 del compás.
    pub fn start(&mut self) {
        self.running = true;
        self.frame = 0;
        self.bar_start = 0;
        self.step = 0;
        self.bar = 0;
    }

    /// Frame (absoluto) del siguiente paso.
    pub fn next_step_frame(&self, sample_rate: u32) -> u64 {
        self.bar_start + step_offset(self.step, self.bpm, sample_rate)
    }

    /// Avanza al siguiente paso y devuelve el que toca disparar ahora y su
    /// compás. En el paso 0 (inicio de compás) aplica antes el tempo pendiente.
    pub fn advance(&mut self, sample_rate: u32) -> (usize, u64) {
        let (step, bar) = (self.step, self.bar);
        if step == 0
            && let Some(bpm) = self.pending_bpm.take()
        {
            self.bpm = bpm;
        }
        self.step += 1;
        if self.step == STEPS {
            self.bar_start += step_offset(STEPS, self.bpm, sample_rate);
            self.step = 0;
            self.bar += 1;
        }
        (step, bar)
    }
}

/// Desplazamiento en frames del paso `k` respecto al inicio del compás.
/// Se calcula desde el inicio del compás, sin acumular redondeos paso a paso.
fn step_offset(k: usize, bpm: f64, sample_rate: u32) -> u64 {
    (k as f64 * frames_per_step(bpm, sample_rate)).round() as u64
}
