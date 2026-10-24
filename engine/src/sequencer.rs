//! Secuenciador: patrones de cortes, BPM y transporte.
//!
//! El reloj va en semicorcheas y compases de 16 pasos (los cambios entran
//! al empezar un compás). Cada pista tiene su propia longitud de patrón
//! (`Pattern::length`, de 1 a [`MAX_STEPS`]): toca la semicorchea global
//! `g` como el paso `g % longitud` de la vuelta `g / longitud`, así que una
//! pista de 12 pasos contra otra de 16 hace polirritmia y queda alineada con
//! la canción aunque se recargue. Con 16 pasos, vuelta = compás.

use std::sync::Arc;

use crate::automation::{Automation, MAX_AUTOMATIONS, MIX_PARAMS, MixCurves};
use crate::engine::{MAX_CHOKE, Playback, SendError, SliceShape};
use crate::fx::{Fx, MAX_FX};
use crate::grain::Granular;
use crate::instrument::Instrument;
use crate::sample::Sample;
use crate::slice::Slices;
use crate::slice_fx::SliceFx;
use crate::synth::{Patch, SynthCurves};

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
/// Notas de un acorde (la del paso y las de `Step::chord`).
pub const MAX_CHORD: usize = 4;

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
    /// El pitch no cambia la duración (`3%7`): el corte suena transpuesto
    /// pero dura lo mismo (granos alineados, ver `crate::shift`).
    pub keep: bool,
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
    /// Pasos que se alarga el golpe (`_` en la notación): en el modo gate y
    /// en los instrumentos, la nota dura `1 + hold` pasos.
    pub hold: u8,
    /// Otras notas del acorde (varios `^`: `1^0^4^7`), en semitonos sobre
    /// `semitones`. Solo suenan en una pista con instrumento.
    pub chord: [Option<f32>; MAX_CHORD - 1],
    /// Efectos de este golpe (`3{lp=800}`), que se juntan con los de su
    /// corte (manda el paso; ver [`SliceFx::with_step`]).
    pub fx: Option<SliceFx>,
    /// Accent (`1!`): en un sintetizador, la nota suena más fuerte y más
    /// abierta (ver [`crate::synth::Accent`]).
    pub accent: bool,
    /// Slide (`1/`): en un sintetizador, la nota llega ligada desde la
    /// anterior (glide, sin volver a disparar las envolventes).
    pub slide: bool,
}

impl Step {
    pub fn new(slice: usize) -> Self {
        Step {
            slice,
            gain: 1.0,
            semitones: 0.0,
            keep: false,
            reverse: false,
            retrig: 1,
            delay: 0.0,
            lock: None,
            hold: 0,
            chord: [None; MAX_CHORD - 1],
            fx: None,
            accent: false,
            slide: false,
        }
    }

    /// Con accent (`1!`).
    pub fn accented(self) -> Self {
        Step {
            accent: true,
            ..self
        }
    }

    /// Con slide (`1/`).
    pub fn slid(self) -> Self {
        Step {
            slide: true,
            ..self
        }
    }

    /// Efectos de este golpe (ver [`crate::slice_fx`]).
    pub fn with_fx(self, fx: Option<SliceFx>) -> Self {
        Step { fx, ..self }
    }

    /// Alarga el golpe `hold` pasos más.
    pub fn with_hold(self, hold: u8) -> Self {
        Step { hold, ..self }
    }

    /// Acorde: las notas de `notes` (semitonos absolutos, como `semitones`)
    /// suenan con la del paso. Como mucho `MAX_CHORD - 1`; las demás se
    /// ignoran.
    pub fn with_chord(self, notes: &[f32]) -> Self {
        let mut chord = [None; MAX_CHORD - 1];
        for (slot, note) in chord.iter_mut().zip(notes) {
            *slot = Some(note - self.semitones);
        }
        Step { chord, ..self }
    }

    /// Semitonos de cada nota del paso: la suya y las del acorde.
    pub fn notes(&self) -> impl Iterator<Item = f32> + '_ {
        std::iter::once(self.semitones)
            .chain(self.chord.iter().flatten().map(|i| self.semitones + i))
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

    /// Pitch que conserva la duración (`3%7`).
    pub fn with_kept_pitch(self, semitones: f32) -> Self {
        Step {
            semitones,
            keep: true,
            ..self
        }
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
    /// Corte (desde 0) del que sale.
    pub slice: usize,
    pub start: usize,
    pub end: usize,
    /// El final del corte entero (antes de `length`): para una curva en
    /// `length`.
    pub full_end: usize,
    pub playback: Playback,
    pub retrig: u8,
    /// Retraso en fracción de paso.
    pub delay: f32,
    /// Corte del primer filtro desde este golpe (Hz).
    pub lock: Option<f32>,
    /// Pasos que se alarga el golpe (`_`).
    pub hold: u8,
    /// Velocidad de lectura de las otras notas del acorde.
    pub chord: [Option<f64>; MAX_CHORD - 1],
    /// Efectos del paso: su índice en `Pattern::step_fx`.
    pub fx: Option<u16>,
    /// Paso de la canción en que cae el golpe (lo pone el motor al
    /// dispararlo): ahí empiezan las rampas de golpe.
    pub origin: f64,
    /// Accent y slide (solo en un sintetizador).
    pub accent: bool,
    pub slide: bool,
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
    /// Pan de la pista, de -1 a 1 (ley de balance, como el de los cortes),
    /// después de su volumen.
    pub(crate) pan: f32,
    /// Volumen, pan y envíos que siguen una curva (ver
    /// [`crate::automation::MIX_PARAMS`]).
    pub(crate) mix_curves: MixCurves,
    /// Modo gate: cada golpe suena solo lo que dura su paso (luego, su
    /// cola). Si no, one-shot: el corte entero.
    pub(crate) gate: bool,
    /// Grupo de choke: un golpe corta lo que suena en las demás pistas del
    /// mismo grupo.
    pub(crate) choke: Option<u8>,
    /// Notas MIDI de sus golpes: (canal 1-16, nota del corte 1).
    pub(crate) midi: Option<(u8, u8)>,
    /// Toca el sample como instrumento (polifonía, ADSR, bucle sostenido).
    pub(crate) instrument: Option<Instrument>,
    /// Toca el sample en granos (nube por golpe).
    pub(crate) granular: Option<Granular>,
    /// Sintetizador: los golpes son notas de su banco de voces (el sample
    /// no suena), con sus parámetros siguiendo `synth_curves`.
    pub(crate) synth: Option<Patch>,
    pub(crate) synth_curves: SynthCurves,
    /// Formantes de los pasos con `%`: `Some(0)` los mantiene, `Some(n)`
    /// los desplaza `n` semitonos; `None`, se mueven con el pitch.
    pub(crate) formant: Option<f32>,
    /// Pitch de la pista (semitonos que se suman a los de cada golpe) y
    /// formante que siguen una curva: cada golpe toma su valor al sonar.
    pub(crate) pitch_curve: Option<crate::automation::Curve>,
    pub(crate) formant_curve: Option<crate::automation::Curve>,
    /// Efectos de cada corte (`slice_fx[i]`, los del corte `i`); vacío si
    /// ninguno tiene.
    slice_fx: Vec<Option<SliceFx>>,
    /// Las cadenas distintas de los efectos de paso (`Trigger::fx` es el
    /// índice); vacío si ningún paso tiene.
    step_fx: Vec<SliceFx>,
    /// Etiqueta de quien lo hizo (de 0 a [`crate::MAX_TAG`]): vuelve con
    /// el paso que suena ([`crate::Cursor`]).
    pub(crate) tag: u32,
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
        let mut step_fx = Vec::new();
        for bar in bars {
            let bar = bar.as_ref();
            if bar.len() != length {
                return Err(SendError::InvalidLength(bar.len()));
            }
            resolve_bar(&sample, slices, bar, &mut steps, &mut step_fx)?;
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
            pan: 0.0,
            mix_curves: [None; MIX_PARAMS],
            gate: false,
            choke: None,
            midi: None,
            instrument: None,
            granular: None,
            synth: None,
            synth_curves: [None; crate::synth::PARAM_COUNT],
            formant: None,
            pitch_curve: None,
            formant_curve: None,
            slice_fx: Vec::new(),
            step_fx,
            tag: 0,
        })
    }

    /// Toca los golpes como notas de un instrumento (ver
    /// [`crate::instrument`]): cada uno abre una voz sin cortar las demás y
    /// dura su paso (más sus `_`) y su release.
    pub fn with_instrument(self, instrument: Option<Instrument>) -> Result<Self, SendError> {
        if instrument.is_some() && self.synth.is_some() {
            return Err(SendError::InvalidSynth(
                "una pista con sintetizador no puede ser instrumento",
            ));
        }
        if let Some(i) = &instrument {
            i.validate(self.sample.frames())
                .map_err(SendError::InvalidInstrument)?;
            if self.granular.is_some() {
                return Err(SendError::InvalidGranular(
                    "una pista con granos no puede ser instrumento",
                ));
            }
            if !self.slice_fx.is_empty() {
                return Err(SendError::InvalidFx(
                    "un instrumento no tiene efectos por corte (usa los de la pista)",
                ));
            }
            if !self.step_fx.is_empty() {
                return Err(SendError::InvalidFx(
                    "un instrumento no tiene efectos por paso (usa los de la pista)",
                ));
            }
        }
        Ok(Pattern { instrument, ..self })
    }

    /// Toca los golpes con un sintetizador (ver [`crate::synth`]): cada uno
    /// es una nota (las de su acorde, si no es mono) con el accent y el
    /// slide del paso; el sample no suena. Un golpe seguido de uno con
    /// slide dura un paso más (sigue sonando hasta que llega el ligado).
    /// `curves`: los parámetros continuos que siguen una curva.
    pub fn with_synth(
        mut self,
        patch: Option<Patch>,
        curves: SynthCurves,
    ) -> Result<Self, SendError> {
        let Some(p) = patch else {
            return Ok(Pattern {
                synth: None,
                synth_curves: [None; crate::synth::PARAM_COUNT],
                ..self
            });
        };
        p.validate().map_err(SendError::InvalidSynth)?;
        crate::synth::check_curves(&curves).map_err(SendError::InvalidSynth)?;
        if self.instrument.is_some() || self.granular.is_some() {
            return Err(SendError::InvalidSynth(
                "una pista con sintetizador no toca su sample (ni instrumento ni granos)",
            ));
        }
        if !self.slice_fx.is_empty() || !self.step_fx.is_empty() {
            return Err(SendError::InvalidSynth(
                "un sintetizador no tiene efectos por corte ni por paso (usa los de la pista)",
            ));
        }
        if p.mono
            && let Some(i) = self
                .steps
                .iter()
                .position(|t| t.is_some_and(|t| t.chord.iter().any(Option::is_some)))
        {
            return Err(SendError::InvalidStep {
                step: i % self.length,
                reason: "un sintetizador monofónico no toca acordes",
            });
        }
        // Lo que va ligado al siguiente: suena hasta que llega.
        let total = self.steps.len();
        for i in 0..total {
            let Some(hit) = self.steps[i] else { continue };
            let next = (i + 1 + hit.hold as usize) % total;
            if self.steps[next].is_some_and(|t| t.slide)
                && (hit.hold as usize) + 1 < MAX_STEPS
                && let Some(t) = &mut self.steps[i]
            {
                t.hold += 1;
            }
        }
        self.synth = Some(p);
        self.synth_curves = curves;
        Ok(self)
    }

    /// Formantes de los pasos con `%` (pitch sin cambiar la duración):
    /// `Some(0.0)` los deja donde estaban, `Some(n)` los desplaza `n`
    /// semitonos (de ±[`crate::shift::MAX_FORMANT`]); `None`, se mueven con
    /// el pitch (como siempre). Ver [`crate::shift`].
    pub fn with_formant(self, formant: Option<f32>) -> Result<Self, SendError> {
        crate::shift::check_formant(formant)
            .map_err(|reason| SendError::InvalidStep { step: 0, reason })?;
        Ok(Pattern { formant, ..self })
    }

    /// Pitch de la pista que sigue una curva (`ramp`, `lfo`; de ±36
    /// semitonos): cada golpe suma a los suyos el valor de la curva en el
    /// momento en que suena (y lo mantiene; el total, como mucho ±36).
    pub fn with_pitch_curve(
        self,
        curve: Option<crate::automation::Curve>,
    ) -> Result<Self, SendError> {
        let invalid = |reason| SendError::InvalidStep { step: 0, reason };
        if let Some(c) = &curve {
            let (a, b) = c.bounds();
            if ![a, b]
                .iter()
                .all(|v| (-MAX_SEMITONES..=MAX_SEMITONES).contains(v))
            {
                return Err(invalid("la curva del pitch se sale de -36 a 36 semitonos"));
            }
            crate::automation::check_curve(c).map_err(invalid)?;
        }
        Ok(Pattern {
            pitch_curve: curve,
            ..self
        })
    }

    /// Formantes de los pasos con `%` que siguen una curva (como
    /// [`Pattern::with_formant`], por golpe).
    pub fn with_formant_curve(
        self,
        curve: Option<crate::automation::Curve>,
    ) -> Result<Self, SendError> {
        let invalid = |reason| SendError::InvalidStep { step: 0, reason };
        if let Some(c) = &curve {
            let (a, b) = c.bounds();
            let max = crate::shift::MAX_FORMANT;
            if ![a, b].iter().all(|v| (-max..=max).contains(v)) {
                return Err(invalid("la curva del formante se sale de su rango"));
            }
            crate::automation::check_curve(c).map_err(invalid)?;
        }
        Ok(Pattern {
            formant_curve: curve,
            ..self
        })
    }

    /// Toca el sample en granos (ver [`crate::grain`]): cada golpe abre
    /// una nube sobre su corte mientras dura su paso y sus `_`.
    pub fn with_granular(self, granular: Option<Granular>) -> Result<Self, SendError> {
        if let Some(g) = &granular {
            g.validate().map_err(SendError::InvalidGranular)?;
            if self.instrument.is_some() {
                return Err(SendError::InvalidGranular(
                    "una pista con granos no puede ser instrumento",
                ));
            }
            if self.synth.is_some() {
                return Err(SendError::InvalidSynth(
                    "una pista con sintetizador no toca su sample en granos",
                ));
            }
            if !self.step_fx.is_empty() {
                return Err(SendError::InvalidGranular(
                    "una pista con granos no tiene efectos por paso (los granos no son voces)",
                ));
            }
        }
        Ok(Pattern { granular, ..self })
    }

    /// Manda cada golpe como una nota MIDI por `canal` (1 a 16): el corte 1
    /// es `base`, el 2 `base + 1`... (ver `Controller::midi_events`). En
    /// un instrumento (`with_instrument`), `base` es la raíz y cada nota
    /// (también las de un acorde) sale como `base` más sus semitonos, y
    /// dura su paso y sus `_` (o hasta que se roba o se corta).
    pub fn with_midi(self, midi: Option<(u8, u8)>) -> Result<Self, SendError> {
        if midi.is_some_and(|(c, b)| !(1..=16).contains(&c) || b > 127) {
            return Err(SendError::InvalidStep {
                step: 0,
                reason: "MIDI: canal de 1 a 16 y nota de 0 a 127",
            });
        }
        Ok(Pattern { midi, ..self })
    }

    /// Modo gate (`true`) u one-shot (`false`, por defecto).
    pub fn with_gate(self, gate: bool) -> Self {
        Pattern { gate, ..self }
    }

    /// Grupo de choke (de 1 a [`MAX_CHOKE`]), o ninguno.
    pub fn with_choke(self, group: Option<u8>) -> Result<Self, SendError> {
        if group.is_some_and(|g| !(1..=MAX_CHOKE).contains(&g)) {
            return Err(SendError::InvalidStep {
                step: 0,
                reason: "grupo de choke fuera de rango (1 a 16)",
            });
        }
        Ok(Pattern {
            choke: group,
            ..self
        })
    }

    /// Forma de cada corte (`shapes[i]` es la del corte `i`; los que falten,
    /// la de siempre). Se aplica a todos los golpes del patrón (una vez:
    /// `length` acorta su rango).
    pub fn with_shapes(mut self, shapes: &[SliceShape]) -> Result<Self, SendError> {
        for (i, shape) in shapes.iter().enumerate() {
            shape
                .validate()
                .map_err(|reason| SendError::InvalidStep { step: i, reason })?;
        }
        for hit in self.steps.iter_mut().flatten() {
            if let Some(shape) = shapes.get(hit.slice) {
                hit.playback.shape = *shape;
                let range = shape.range(hit.start..hit.end);
                hit.end = range.end;
            }
        }
        Ok(self)
    }

    /// Efectos de cada corte (`chains[i]`, los del corte `i`; ver
    /// [`crate::slice_fx`]): cada golpe de ese corte suena con ellos,
    /// antes de los de la pista. Los cortes que falten, sin efectos. No
    /// vale en un instrumento; con granos no se aplican (los granos no son
    /// voces).
    pub fn with_slice_fx(mut self, chains: &[Option<SliceFx>]) -> Result<Self, SendError> {
        // Una cadena vacía es como no tenerla (la voz no pasa por ella).
        let chains: Vec<Option<SliceFx>> = chains
            .iter()
            .map(|c| c.filter(|c| c.effects().next().is_some()))
            .collect();
        if self.instrument.is_some() && chains.iter().any(Option::is_some) {
            return Err(SendError::InvalidFx(
                "un instrumento no tiene efectos por corte (usa los de la pista)",
            ));
        }
        self.slice_fx = if chains.iter().any(Option::is_some) {
            chains
        } else {
            Vec::new()
        };
        // Cada golpe con efectos de paso cabe con los de su corte.
        for hit in self.steps.iter().flatten() {
            if hit.fx.is_some() {
                self.hit_fx_checked(hit).map_err(SendError::InvalidFx)?;
            }
        }
        Ok(self)
    }

    /// Efectos del corte `slice`, si tiene.
    pub(crate) fn slice_fx(&self, slice: usize) -> Option<SliceFx> {
        self.slice_fx.get(slice).copied().flatten()
    }

    /// Los efectos con los que suena `hit`: los de su corte y los de su
    /// paso (manda el paso), o ninguno.
    pub(crate) fn hit_fx(&self, hit: &Trigger) -> Option<SliceFx> {
        // Ya comprobado al crear el patrón: siempre cabe.
        self.hit_fx_checked(hit).ok().flatten()
    }

    fn hit_fx_checked(&self, hit: &Trigger) -> Result<Option<SliceFx>, &'static str> {
        let step = hit.fx.and_then(|i| self.step_fx.get(i as usize));
        Ok(match (self.slice_fx(hit.slice), step) {
            (slice, None) => slice,
            (None, Some(step)) => Some(*step),
            (Some(slice), Some(step)) => Some(slice.with_step(step)?),
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

    /// Pan de la pista (de -1 a 1; 0, al centro sin tocar nada), después de
    /// su volumen y antes de los envíos.
    pub fn with_pan(self, pan: f32) -> Result<Self, SendError> {
        if !(-1.0..=1.0).contains(&pan) {
            return Err(SendError::InvalidFx("pan de pista fuera de rango (-1 a 1)"));
        }
        Ok(Pattern { pan, ..self })
    }

    /// Volumen, pan y envíos que siguen una curva (`ramp`, `lfo`; índices
    /// [`crate::MIX_GAIN`], [`crate::MIX_PAN`] y [`crate::MIX_SEND`]` + bus`).
    /// La curva manda sobre el valor fijo; se evalúa cada 32 frames y se
    /// suaviza como el volumen.
    pub fn with_mix_automation(self, curves: MixCurves) -> Result<Self, SendError> {
        crate::automation::check_mix(&curves).map_err(SendError::InvalidFx)?;
        Ok(Pattern {
            mix_curves: curves,
            ..self
        })
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
        self.steps[self.index(global)]
    }

    /// Paso del ciclo aplanado (`vuelta · length + paso`) que suena en la
    /// semicorchea `global`.
    pub(crate) fn index(&self, global: u64) -> usize {
        let length = self.length as u64;
        let lap = (global / length) % self.cycle() as u64;
        (lap * length + global % length) as usize
    }

    /// Etiqueta de quien hizo el patrón (de 0 a [`crate::MAX_TAG`]; el
    /// script da una por definición de pista): el motor la publica con el
    /// paso que suena ([`crate::TrackStatus::cursor`]). No cambia el audio.
    pub fn with_tag(self, tag: u32) -> Result<Self, SendError> {
        if tag > crate::MAX_TAG {
            return Err(SendError::InvalidTag(tag));
        }
        Ok(Pattern { tag, ..self })
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
    chains: &mut Vec<SliceFx>,
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
        if step
            .notes()
            .any(|st| !(-MAX_SEMITONES..=MAX_SEMITONES).contains(&st))
        {
            return Err(invalid(
                "nota del acorde fuera de rango (-36 a 36 semitonos)",
            ));
        }
        if step.hold as usize >= MAX_STEPS {
            return Err(invalid("golpe alargado (_) más que el patrón más largo"));
        }
        // Los efectos del paso: la cadena (sin vacías) en la tabla del
        // patrón, sin repetirla.
        let fx = match step.fx.filter(|c| c.effects().next().is_some()) {
            None => None,
            Some(chain) => {
                let index = match chains.iter().position(|c| *c == chain) {
                    Some(i) => i,
                    None => {
                        chains.push(chain);
                        chains.len() - 1
                    }
                };
                Some(u16::try_from(index).map_err(|_| invalid("demasiados efectos de paso"))?)
            }
        };
        let rate = |st: f32| 2f64.powf(st as f64 / 12.0);
        out.push(Some(Trigger {
            slice: step.slice,
            start: range.start,
            end: range.end.min(sample.frames()),
            full_end: range.end.min(sample.frames()),
            playback: Playback {
                gain: step.gain,
                rate: rate(step.semitones),
                keep: step.keep,
                formant: None,
                reverse: step.reverse,
                shape: SliceShape::DEFAULT,
            },
            retrig: step.retrig,
            delay: step.delay,
            lock: step.lock,
            hold: step.hold,
            chord: step.chord.map(|i| i.map(|i| rate(step.semitones + i))),
            fx,
            origin: 0.0,
            accent: step.accent,
            slide: step.slide,
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

/// Frames de un compás de `steps` pasos (otro compás que 4/4), con el
/// mismo redondeo que el transporte.
pub fn bar_frames_steps(steps: usize, bpm: f64, sample_rate: u32) -> u64 {
    step_offset(steps, bpm, sample_rate)
}

/// Si `bpm` está en el rango admitido (`MIN_BPM`..=`MAX_BPM`).
pub fn valid_bpm(bpm: f64) -> bool {
    (MIN_BPM..=MAX_BPM).contains(&bpm)
}

/// Rampa de tempo: de `from` a `to` BPM en `bars` compases. Cada compás
/// suena a su tempo (cuantizado al compás): el `k` desde que entra, a
/// `from + (to − from)·k/bars`; después, a `to`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TempoRamp {
    pub from: f64,
    pub to: f64,
    pub bars: u64,
}

impl TempoRamp {
    /// Tempo del compás `k` desde que entró la rampa.
    pub fn bpm_at(&self, k: u64) -> f64 {
        if self.bars == 0 || k >= self.bars {
            return self.to;
        }
        self.from + (self.to - self.from) * k as f64 / self.bars as f64
    }
}

/// Compás: cuántos pasos tiene y cada cuántos pasos cae un pulso (del
/// metrónomo). 4/4 son 16 y 4; 3/4, 12 y 4; 7/8, 14 y 2.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Meter {
    pub steps: usize,
    pub beat: usize,
}

impl Meter {
    pub const COMMON: Meter = Meter { steps: 16, beat: 4 };

    /// `n/d` (d = 4, 8 o 16): `n · 16/d` pasos y un pulso cada `16/d`.
    pub fn new(n: usize, d: usize) -> Option<Meter> {
        if !matches!(d, 4 | 8 | 16) {
            return None;
        }
        let beat = STEPS / d;
        let steps = n * beat;
        (1..=MAX_STEPS)
            .contains(&steps)
            .then_some(Meter { steps, beat })
    }
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
    /// Pasos de los compases anteriores (desde que arrancó).
    steps_before: u64,
    bpm: f64,
    meter: Meter,
    /// Rampa de tempo en curso y el compás en que entró.
    ramp: Option<(TempoRamp, u64)>,
    /// Tempo, rampa y compás que esperan al siguiente compás.
    pub pending_bpm: Option<f64>,
    pub pending_ramp: Option<TempoRamp>,
    pub pending_meter: Option<Meter>,
}

impl Transport {
    pub fn new() -> Self {
        Transport {
            running: false,
            frame: 0,
            bar_start: 0,
            step: 0,
            bar: 0,
            steps_before: 0,
            bpm: DEFAULT_BPM,
            meter: Meter::COMMON,
            ramp: None,
            pending_bpm: None,
            pending_ramp: None,
            pending_meter: None,
        }
    }

    /// Arranca desde el paso 0 del compás.
    pub fn start(&mut self) {
        self.running = true;
        self.frame = 0;
        self.bar_start = 0;
        self.step = 0;
        self.bar = 0;
        self.steps_before = 0;
        // Una rampa en curso vuelve a empezar con la canción.
        if let Some((_, start)) = &mut self.ramp {
            *start = 0;
        }
    }

    /// Compás que está sonando.
    pub fn meter(&self) -> Meter {
        self.meter
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
        self.steps_before as f64 + (self.frame as f64 - self.bar_start as f64) / step
    }

    /// Frame (absoluto) del siguiente paso.
    pub fn next_step_frame(&self, sample_rate: u32) -> u64 {
        self.bar_start + step_offset(self.step, self.bpm, sample_rate)
    }

    /// Avanza al siguiente paso y devuelve el que toca disparar ahora, su
    /// compás y su semicorchea desde que arrancó (la que leen las pistas).
    /// En el paso 0 (inicio de compás) aplica antes el tempo, la rampa y el
    /// compás pendientes.
    pub fn advance(&mut self, sample_rate: u32) -> (usize, u64, u64) {
        let (step, bar) = (self.step, self.bar);
        if step == 0 {
            if let Some(bpm) = self.pending_bpm.take() {
                self.bpm = bpm;
                self.ramp = None;
            }
            // La misma rampa otra vez (al recargar) sigue por donde iba.
            if let Some(ramp) = self.pending_ramp.take()
                && self.ramp.map(|(r, _)| r) != Some(ramp)
            {
                self.ramp = Some((ramp, bar));
            }
            if let Some(meter) = self.pending_meter.take() {
                self.meter = meter;
            }
            if let Some((ramp, start)) = self.ramp {
                self.bpm = ramp.bpm_at(bar - start);
            }
        }
        let global = self.steps_before + step as u64;
        self.step += 1;
        if self.step == self.meter.steps {
            self.bar_start += step_offset(self.meter.steps, self.bpm, sample_rate);
            self.steps_before += self.meter.steps as u64;
            self.step = 0;
            self.bar += 1;
        }
        (step, bar, global)
    }
}

/// Desplazamiento en frames del paso `k` respecto al inicio del compás.
/// Se calcula desde el inicio del compás, sin acumular redondeos paso a paso.
fn step_offset(k: usize, bpm: f64, sample_rate: u32) -> u64 {
    (k as f64 * frames_per_step(bpm, sample_rate)).round() as u64
}
