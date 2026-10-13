//! Secuenciador: patrones de cortes, BPM y transporte.
//!
//! El reloj va en semicorcheas y compases de 16 pasos (los cambios entran
//! al empezar un compás). Cada pista tiene su propia longitud de patrón
//! (`Pattern::length`, de 1 a [`MAX_STEPS`]): toca la semicorchea global
//! `g` como el paso `g % longitud` de la vuelta `g / longitud`, así que una
//! pista de 12 pasos contra otra de 16 hace polirritmia y queda alineada con
//! la canción aunque se recargue. Con 16 pasos, vuelta = compás.

use std::sync::Arc;

use crate::automation::{Automation, MAX_AUTOMATIONS};
use crate::engine::{Playback, SendError};
use crate::fx::{Fx, MAX_FX};
use crate::sample::Sample;
use crate::slice::Slices;

/// Pasos por compás (semicorcheas en 4/4) y longitud por defecto de un
/// patrón.
pub const STEPS: usize = 16;
/// Longitud máxima de un patrón, en pasos.
pub const MAX_STEPS: usize = 64;
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
/// Retraso máximo de un golpe, en fracción de paso: siempre suena antes
/// del paso siguiente.
pub const MAX_DELAY: f32 = 0.9;
/// Vueltas máximas del ciclo de variantes de un patrón (compases, con 16
/// pasos).
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
    /// Retraso del golpe (swing, humanize), en fracción de paso: de 0 a
    /// `MAX_DELAY`. El motor lo pasa a frames con la duración del paso.
    pub delay: f32,
    /// Corte (Hz) del primer filtro de la pista desde este golpe hasta el
    /// siguiente (parameter lock, `3~800`); `None`, el de la automatización
    /// o el fijo.
    pub lock: Option<f32>,
}

impl Step {
    pub fn new(slice: usize) -> Self {
        Step {
            slice,
            gain: 1.0,
            semitones: 0.0,
            reverse: false,
            retrig: 1,
            delay: 0.0,
            lock: None,
        }
    }

    pub fn with_lock(self, cutoff_hz: f32) -> Self {
        Step {
            lock: Some(cutoff_hz),
            ..self
        }
    }

    pub fn with_delay(self, delay: f32) -> Self {
        Step { delay, ..self }
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
    /// Retraso en fracción de paso.
    pub delay: f32,
    /// Corte del primer filtro desde este golpe (Hz).
    pub lock: Option<f32>,
}

/// Patrón de una pista. Los pasos se resuelven a rangos de frames y
/// parámetros de reproducción en el hilo de control, así el motor solo lee
/// este valor (sin reservas).
pub struct Pattern {
    pub(crate) sample: Arc<Sample>,
    /// Pasos de una vuelta del patrón.
    length: usize,
    /// Ciclo de vueltas, seguidas: en la vuelta `v` de la canción suenan los
    /// pasos `(v % ciclo) · length ..`. Un patrón fijo tiene una sola vuelta.
    steps: Vec<Option<Trigger>>,
    /// Volumen de la pista (lineal), aplicado después de sus voces.
    pub(crate) gain: f32,
    /// Efectos de la pista, en orden.
    pub(crate) fx: [Option<Fx>; MAX_FX],
    /// Silenciada desde el script (`:mute()`); se suma al mute en vivo.
    pub(crate) mute: bool,
    /// En solo desde el script (`:solo()`); se suma al solo en vivo.
    pub(crate) solo: bool,
    /// Envío a cada bus (delay, reverb), de 0 a `MAX_SEND`.
    pub(crate) sends: [f32; crate::bus::BUSES],
    /// Parámetros de efecto automatizados.
    pub(crate) automation: [Option<Automation>; MAX_AUTOMATIONS],
}

impl Pattern {
    /// `steps[i]` es el número de corte que suena en el paso `i`, o `None`
    /// para no disparar nada (lo que sonaba sigue sonando). La longitud del
    /// patrón es la de `steps` (de 1 a [`MAX_STEPS`]; 16 es un compás).
    pub fn new(
        sample: Arc<Sample>,
        slices: &Slices,
        steps: impl AsRef<[Option<usize>]>,
    ) -> Result<Self, SendError> {
        let steps: Vec<Option<Step>> = steps.as_ref().iter().map(|s| s.map(Step::new)).collect();
        Self::from_steps(sample, slices, steps)
    }

    /// Como [`Pattern::new`], con volumen, pitch y reverse por paso.
    pub fn from_steps(
        sample: Arc<Sample>,
        slices: &Slices,
        steps: impl AsRef<[Option<Step>]>,
    ) -> Result<Self, SendError> {
        Self::from_bars(sample, slices, &[steps])
    }

    /// Patrón de varias vueltas que se repiten en ciclo: en la vuelta `v`
    /// de la canción suena `bars[v % bars.len()]` (de 1 a
    /// `MAX_CYCLE_BARS`). Todas con la misma longitud, de 1 a
    /// [`MAX_STEPS`]; con 16 pasos, cada vuelta es un compás.
    pub fn from_bars<B: AsRef<[Option<Step>]>>(
        sample: Arc<Sample>,
        slices: &Slices,
        bars: &[B],
    ) -> Result<Self, SendError> {
        if bars.is_empty() || bars.len() > MAX_CYCLE_BARS {
            return Err(SendError::InvalidCycle(bars.len()));
        }
        let length = bars[0].as_ref().len();
        if !(1..=MAX_STEPS).contains(&length) {
            return Err(SendError::InvalidLength(length));
        }
        let mut steps = Vec::with_capacity(length * bars.len());
        for bar in bars {
            let bar = bar.as_ref();
            if bar.len() != length {
                return Err(SendError::InvalidLength(bar.len()));
            }
            resolve_bar(&sample, slices, bar, &mut steps)?;
        }
        Ok(Pattern {
            sample,
            length,
            steps,
            gain: 1.0,
            fx: [None; MAX_FX],
            mute: false,
            solo: false,
            sends: [0.0; crate::bus::BUSES],
            automation: [None; MAX_AUTOMATIONS],
        })
    }

    /// Parámetros de efecto que cambian con el tiempo (ver
    /// [`crate::automation`]). Va después de [`Pattern::with_fx`]: cada
    /// automatización tiene que ser de un efecto de la cadena.
    pub fn with_automation(mut self, list: &[Automation]) -> Result<Self, SendError> {
        if list.len() > MAX_AUTOMATIONS {
            return Err(SendError::InvalidFx(
                "demasiadas automatizaciones (máximo 8)",
            ));
        }
        let mut slots = [None; MAX_AUTOMATIONS];
        for (slot, a) in slots.iter_mut().zip(list) {
            a.validate(&self.fx).map_err(SendError::InvalidFx)?;
            *slot = Some(*a);
        }
        self.automation = slots;
        Ok(self)
    }

    /// Envíos de la pista a los buses: `[delay, reverb]`, de 0 a
    /// [`crate::bus::MAX_SEND`], después de su volumen (entran con el
    /// patrón).
    pub fn with_sends(mut self, sends: [f32; crate::bus::BUSES]) -> Result<Self, SendError> {
        if sends
            .iter()
            .any(|s| !(0.0..=crate::bus::MAX_SEND).contains(s))
        {
            return Err(SendError::InvalidBus(
                "nivel de envío fuera de rango (0 a 1)",
            ));
        }
        self.sends = sends;
        Ok(self)
    }

    /// Pista silenciada (entra con el patrón, al empezar el compás).
    pub fn with_mute(self, mute: bool) -> Self {
        Pattern { mute, ..self }
    }

    /// Pista en solo: si alguna pista está en solo, solo suenan las que lo
    /// están (entra con el patrón).
    pub fn with_solo(self, solo: bool) -> Self {
        Pattern { solo, ..self }
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

    /// Pasos de una vuelta.
    pub fn length(&self) -> usize {
        self.length
    }

    /// Vueltas del ciclo de variantes.
    pub fn cycle(&self) -> usize {
        self.steps.len() / self.length
    }

    /// Lo que suena en la semicorchea `global` de la canción (contada desde
    /// que arrancó el transporte): el paso `global % length` de la vuelta
    /// `global / length`.
    pub(crate) fn hit(&self, global: u64) -> Option<Trigger> {
        let length = self.length as u64;
        let lap = (global / length) % self.cycle() as u64;
        self.steps[(lap * length + global % length) as usize]
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

/// Resuelve los pasos de una vuelta a rangos de frames y parámetros y los
/// añade a `out`.
fn resolve_bar(
    sample: &Sample,
    slices: &Slices,
    steps: &[Option<Step>],
    out: &mut Vec<Option<Trigger>>,
) -> Result<(), SendError> {
    for (i, step) in steps.iter().copied().enumerate() {
        let Some(step) = step else {
            out.push(None);
            continue;
        };
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
        if !(0.0..=MAX_DELAY).contains(&step.delay) {
            return Err(invalid("retraso fuera de rango (0 a 0,9 pasos)"));
        }
        if step.lock.is_some_and(|hz| !(20.0..=20_000.0).contains(&hz)) {
            return Err(invalid("corte por paso (~) fuera de rango (20 a 20000 Hz)"));
        }
        out.push(Some(Trigger {
            start: range.start,
            end: range.end.min(sample.frames()),
            playback: Playback {
                gain: step.gain,
                rate: 2f64.powf(step.semitones as f64 / 12.0),
                reverse: step.reverse,
            },
            retrig: step.retrig,
            delay: step.delay,
            lock: step.lock,
        }));
    }
    Ok(())
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

    /// Tempo que está sonando.
    pub fn bpm(&self) -> f64 {
        self.bpm
    }

    /// Posición de la canción en pasos (con decimales) en el frame actual:
    /// lo que siguen las automatizaciones.
    pub fn position_steps(&self, sample_rate: u32) -> f64 {
        let step = frames_per_step(self.bpm, sample_rate);
        // Durante el último paso del compás, `bar` y `bar_start` ya son los
        // del siguiente: la resta sale negativa y la suma, bien.
        self.bar as f64 * STEPS as f64 + (self.frame as f64 - self.bar_start as f64) / step
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
