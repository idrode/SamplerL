//! Mandos MIDI asignados a parámetros (MIDI learn). Un mando mueve un
//! parámetro de efecto de una pista, su volumen, pan o envíos, un
//! parámetro de los buses o de un sintetizador, entre un mínimo y un máximo
//! con su curva.
//!
//! - **La tabla** ([`Knobs`]) entra con el lote, al empezar el compás (va con
//!   la canción: sus índices son los de sus pistas y sus efectos).
//! - **El valor** ([`crate::Controller::move_knob`]) entra ya: cada trozo
//!   lo sigue con un polo de [`KNOB_SMOOTHING_SECONDS`] (los 128 pasos de un
//!   CC no se oyen como escalones) y después el parámetro tiene su
//!   suavizado de siempre (~5 ms).
//! - **Quién manda**: el mando entra en [`TAKEOVER_SECONDS`] desde que se
//!   mueve. Sobre un parámetro con curva (`ramp`, `lfo`, `macro`), cuando
//!   lleva [`HOLD_SECONDS`] quieto devuelve el control en
//!   [`RETURN_SECONDS`]; sobre un valor fijo se queda donde se dejó
//!   ([`KnobMap::latch`]).
//!
//! Todo es de tamaño fijo y `Copy`: el hilo de audio no reserva nada.

use crate::automation::mix;

/// Mandos asignados como mucho.
pub const MAX_KNOBS: usize = 16;
/// Lo que tarda el valor del mando en seguir al CC.
pub const KNOB_SMOOTHING_SECONDS: f32 = 0.03;
/// Lo que tarda el mando en mandar desde que se mueve.
pub const TAKEOVER_SECONDS: f32 = 0.03;
/// Quieto este tiempo, un mando sobre una curva la suelta...
pub const HOLD_SECONDS: f32 = 1.5;
/// ... y la curva vuelve a mandar en esto.
pub const RETURN_SECONDS: f32 = 0.5;

/// Qué mueve un mando.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KnobTarget {
    /// El parámetro `param` del efecto de la ranura `slot` de la pista.
    Fx { track: u8, slot: u8, param: u8 },
    /// Volumen, pan o envío de la pista (índices de
    /// [`crate::automation::MixCurves`]).
    Mix { track: u8, index: u8 },
    /// Un parámetro de los buses (índices de [`crate::bus::BusCurves`]).
    Bus { param: u8 },
    /// Un parámetro del sintetizador de la pista (índices de
    /// [`crate::synth::SynthCurves`]).
    Synth { track: u8, param: u8 },
}

/// Cómo va el mando de su mínimo a su máximo.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum KnobCurve {
    /// En línea recta.
    #[default]
    Linear,
    /// En escala logarítmica (frecuencias, tiempos): la mitad del mando es
    /// la media geométrica. Con un extremo de 0 o menos, en línea recta.
    Log,
    /// Al cuadrado: más fino cerca del mínimo.
    Square,
}

/// Un mando asignado.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct KnobMap {
    pub target: KnobTarget,
    pub min: f32,
    pub max: f32,
    pub curve: KnobCurve,
    /// El parámetro no tiene curva: el mando se queda donde se deja (no
    /// devuelve el control).
    pub latch: bool,
}

impl KnobMap {
    /// El valor del parámetro con el mando en `t` (de 0 a 1).
    pub fn value(&self, t: f32) -> f32 {
        let t = t.clamp(0.0, 1.0);
        match self.curve {
            KnobCurve::Linear => mix(self.min, self.max, t, false),
            KnobCurve::Log => mix(self.min, self.max, t, self.min > 0.0 && self.max > 0.0),
            KnobCurve::Square => mix(self.min, self.max, t * t, false),
        }
    }
}

/// La tabla de mandos: el índice es el del mando.
pub type Knobs = [Option<KnobMap>; MAX_KNOBS];

/// Lo que mandan los mandos en un trozo: por cada uno, su destino, el
/// valor del parámetro y cuánto manda (0: nada, 1: todo).
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct KnobLive {
    entries: [Option<(KnobTarget, f32, f32)>; MAX_KNOBS],
}

impl KnobLive {
    /// El valor y el peso del mando que mueve `target`, si alguno manda.
    pub fn find(&self, target: KnobTarget) -> Option<(f32, f32)> {
        self.entries
            .iter()
            .flatten()
            .find(|(t, _, _)| *t == target)
            .map(|&(_, v, w)| (v, w))
    }

    /// `base` (el valor fijo, o el de su curva ahora) con el mando que
    /// mueve `target` encima, según cuánto mande (`log`: el parámetro se
    /// mueve en escala logarítmica).
    pub fn apply(&self, target: KnobTarget, base: f32, log: bool) -> f32 {
        match self.find(target) {
            Some((value, weight)) => mix(base, value, weight, log),
            None => base,
        }
    }

    /// Algún mando manda ahora en lo que cumple `what`.
    pub fn any(&self, what: impl Fn(KnobTarget) -> bool) -> bool {
        self.entries.iter().flatten().any(|&(t, _, _)| what(t))
    }

    /// Algún mando manda en los efectos de la pista `track`.
    pub fn on_fx(&self, track: usize) -> bool {
        self.any(|t| matches!(t, KnobTarget::Fx { track: k, .. } if k as usize == track))
    }

    /// Algún mando manda en el volumen, el pan o los envíos de `track`.
    pub fn on_mix(&self, track: usize) -> bool {
        self.any(|t| matches!(t, KnobTarget::Mix { track: k, .. } if k as usize == track))
    }

    /// Algún mando manda en los buses.
    pub fn on_buses(&self) -> bool {
        self.any(|t| matches!(t, KnobTarget::Bus { .. }))
    }
}

/// Los mandos en el motor: la tabla que suena, la que espera al compás, y
/// el estado de cada uno.
#[derive(Debug, Clone, Copy)]
pub(crate) struct KnobState {
    pub map: Knobs,
    pub pending: Option<Knobs>,
    pub staged: Option<Knobs>,
    /// Lo último que llegó del CC (0 a 1) y lo que suena (suavizado).
    value: [f32; MAX_KNOBS],
    smoothed: [f32; MAX_KNOBS],
    /// Cuánto manda cada uno, y frames desde que se movió por última vez
    /// (`None`: no se ha movido nunca).
    weight: [f32; MAX_KNOBS],
    idle: [Option<u64>; MAX_KNOBS],
}

impl Default for KnobState {
    fn default() -> Self {
        KnobState {
            map: [None; MAX_KNOBS],
            pending: None,
            staged: None,
            value: [0.0; MAX_KNOBS],
            smoothed: [0.0; MAX_KNOBS],
            weight: [0.0; MAX_KNOBS],
            idle: [None; MAX_KNOBS],
        }
    }
}

impl KnobState {
    /// El mando `index` se ha movido a `value` (de 0 a 1).
    pub fn moved(&mut self, index: usize, value: f32) {
        if index >= MAX_KNOBS {
            return;
        }
        // La primera vez, sin glide desde 0: desde donde está.
        if self.idle[index].is_none() {
            self.smoothed[index] = value;
        }
        self.value[index] = value;
        self.idle[index] = Some(0);
    }

    /// Entra la tabla nueva (al empezar el compás). Un mando que cambia de
    /// destino empieza sin mandar.
    pub fn set_map(&mut self, map: Knobs) {
        for (i, (old, new)) in self.map.iter().zip(map.iter()).enumerate() {
            if old.map(|m| m.target) != new.map(|m| m.target) {
                self.weight[i] = 0.0;
            }
        }
        self.map = map;
    }

    /// Avanza `frames` frames y da lo que mandan en ese trozo. Sin mandos
    /// asignados, nada (y nada que calcular).
    pub fn advance(&mut self, frames: usize, sample_rate: u32) -> KnobLive {
        let mut live = KnobLive::default();
        if self.map.iter().all(Option::is_none) {
            return live;
        }
        let rate = sample_rate as f32;
        let n = frames as f32;
        let follow = 1.0 - (-n / (KNOB_SMOOTHING_SECONDS * rate)).exp();
        let hold = (HOLD_SECONDS * rate) as u64;
        for i in 0..MAX_KNOBS {
            let Some(map) = self.map[i] else { continue };
            let Some(idle) = self.idle[i] else { continue };
            self.smoothed[i] += (self.value[i] - self.smoothed[i]) * follow;
            if idle < hold || map.latch {
                self.weight[i] = (self.weight[i] + n / (TAKEOVER_SECONDS * rate)).min(1.0);
            } else {
                self.weight[i] = (self.weight[i] - n / (RETURN_SECONDS * rate)).max(0.0);
            }
            self.idle[i] = Some(idle.saturating_add(frames as u64));
            if self.weight[i] > 0.0 {
                live.entries[i] = Some((map.target, map.value(self.smoothed[i]), self.weight[i]));
            }
        }
        live
    }

    /// Lo que mandan ahora, sin avanzar.
    #[cfg(test)]
    pub fn live(&self) -> KnobLive {
        let mut live = KnobLive::default();
        for i in 0..MAX_KNOBS {
            if let Some(map) = self.map[i]
                && self.weight[i] > 0.0
            {
                live.entries[i] = Some((map.target, map.value(self.smoothed[i]), self.weight[i]));
            }
        }
        live
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const RATE: u32 = 48_000;

    fn cutoff(latch: bool) -> Knobs {
        let mut map = [None; MAX_KNOBS];
        map[0] = Some(KnobMap {
            target: KnobTarget::Fx {
                track: 0,
                slot: 0,
                param: 0,
            },
            min: 200.0,
            max: 8000.0,
            curve: KnobCurve::Log,
            latch,
        });
        map
    }

    #[test]
    fn curves_go_from_min_to_max() {
        let map = cutoff(false)[0].unwrap();
        assert_eq!(map.value(0.0), 200.0);
        assert_eq!(map.value(1.0), 8000.0);
        assert!((map.value(0.5) - (200.0f32 * 8000.0).sqrt()).abs() < 1.0);
        let lin = KnobMap {
            curve: KnobCurve::Linear,
            ..map
        };
        assert_eq!(lin.value(0.5), 4100.0);
        let sq = KnobMap {
            curve: KnobCurve::Square,
            ..map
        };
        assert_eq!(sq.value(0.5), 200.0 + 0.25 * 7800.0);
    }

    #[test]
    fn a_knob_takes_over_while_it_moves_and_gives_back_a_curve() {
        let mut state = KnobState::default();
        state.set_map(cutoff(false));
        let target = cutoff(false)[0].unwrap().target;
        // Sin moverlo, no manda.
        assert_eq!(state.advance(256, RATE).find(target), None);
        state.moved(0, 1.0);
        // Entra en 30 ms (1440 frames).
        let mut live = state.advance(256, RATE);
        assert!(live.find(target).unwrap().1 < 1.0);
        for _ in 0..6 {
            live = state.advance(256, RATE);
        }
        let (value, weight) = live.find(target).unwrap();
        assert_eq!((value, weight), (8000.0, 1.0));
        // Quieto hasta casi 1,5 s, sigue mandando; después suelta en 0,5 s.
        for _ in 0..(48_000 * 3 / 2 / 256 - 8) {
            live = state.advance(256, RATE);
        }
        assert_eq!(live.find(target).map(|f| f.1), Some(1.0));
        for _ in 0..(48_000 / 256 + 10) {
            live = state.advance(256, RATE);
        }
        assert_eq!(live.find(target), None);
        assert_eq!(live.apply(target, 1234.0, true), 1234.0);
    }

    #[test]
    fn on_a_fixed_value_it_stays_where_it_was_left() {
        let mut state = KnobState::default();
        state.set_map(cutoff(true));
        let target = cutoff(true)[0].unwrap().target;
        state.moved(0, 0.0);
        for _ in 0..(48_000 * 5 / 256) {
            state.advance(256, RATE);
        }
        assert_eq!(state.live().find(target), Some((200.0, 1.0)));
    }

    #[test]
    fn the_value_is_smoothed() {
        let mut state = KnobState::default();
        state.set_map(cutoff(false));
        let target = cutoff(false)[0].unwrap().target;
        state.moved(0, 0.0);
        state.advance(256, RATE);
        state.moved(0, 1.0);
        let first = state.advance(32, RATE).find(target).unwrap().0;
        // A 32 frames de un salto de todo el recorrido, apenas se ha movido.
        assert!(first > 200.0 && first < 300.0, "{first}");
        for _ in 0..100 {
            state.advance(256, RATE);
        }
        assert!((state.live().find(target).unwrap().0 - 8000.0).abs() < 1.0);
    }
}
