//! Grupos: varias pistas que se suman antes de llegar al master y pasan
//! juntas por una cadena de efectos (un compresor que las pega, una
//! saturación, un filtro), con su volumen, su pan y sus envíos.
//!
//! Cada pista de un grupo suma su salida (después de su volumen, pan, mute
//! y solo) al buffer del grupo en vez de al master; sus envíos siguen
//! saliendo de ella, sin pasar por los efectos del grupo. El grupo pasa la
//! suma por su cadena y la manda al master y a los buses con su volumen,
//! pan y envíos. Un nivel: un grupo no va a otro grupo.
//!
//! Los grupos ([`Groups`]) entran en un lote, al empezar el compás, como
//! los buses. Los buffers son fijos (se reservan al crear el motor). Un
//! grupo que no recibe nada durante un segundo y ya no suena se duerme:
//! no se procesa, y sin grupos la salida es la de siempre bit a bit.

use crate::automation::{Automation, MAX_AUTOMATIONS, MIX_PARAMS, MixCurves, check_mix};
use crate::bus::{BUSES, MAX_SEND};
use crate::engine::{MAX_TRACKS, SendError};
use crate::fx::{Fx, FxState, MAX_FX, MAX_PARAMS};
use crate::sequencer::MAX_GAIN;

/// Grupos como máximo.
pub const MAX_GROUPS: usize = 4;

/// Un grupo: su cadena de efectos (con sus automatizaciones), volumen, pan
/// y envíos, como los de una pista.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Group {
    pub fx: [Option<Fx>; MAX_FX],
    pub automation: [Option<Automation>; MAX_AUTOMATIONS],
    pub gain: f32,
    pub pan: f32,
    pub sends: [f32; BUSES],
    pub mix_curves: MixCurves,
}

impl Default for Group {
    fn default() -> Self {
        Group {
            fx: [None; MAX_FX],
            automation: [None; MAX_AUTOMATIONS],
            gain: 1.0,
            pan: 0.0,
            sends: [0.0; BUSES],
            mix_curves: [None; MIX_PARAMS],
        }
    }
}

impl Group {
    pub fn new() -> Self {
        Self::default()
    }

    /// Cadena de efectos del grupo (hasta `MAX_FX`, en orden).
    pub fn with_fx(mut self, chain: &[Fx]) -> Result<Self, SendError> {
        if chain.len() > MAX_FX {
            return Err(SendError::InvalidGroup(
                "demasiados efectos (máximo 4 por grupo)",
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

    /// Parámetros de efecto que siguen una curva. Va después de
    /// [`Group::with_fx`].
    pub fn with_automation(mut self, list: &[Automation]) -> Result<Self, SendError> {
        if list.len() > MAX_AUTOMATIONS {
            return Err(SendError::InvalidGroup(
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

    /// Volumen del grupo (lineal, de 0 a `MAX_GAIN`).
    pub fn with_gain(self, gain: f32) -> Result<Self, SendError> {
        if !(0.0..=MAX_GAIN).contains(&gain) {
            return Err(SendError::InvalidGroup(
                "volumen de grupo fuera de rango (0 a 4)",
            ));
        }
        Ok(Group { gain, ..self })
    }

    /// Pan del grupo (de -1 a 1).
    pub fn with_pan(self, pan: f32) -> Result<Self, SendError> {
        if !(-1.0..=1.0).contains(&pan) {
            return Err(SendError::InvalidGroup(
                "pan de grupo fuera de rango (-1 a 1)",
            ));
        }
        Ok(Group { pan, ..self })
    }

    /// Envíos del grupo a los buses: `[delay, reverb]`, de 0 a 1.
    pub fn with_sends(self, sends: [f32; BUSES]) -> Result<Self, SendError> {
        if sends.iter().any(|s| !(0.0..=MAX_SEND).contains(s)) {
            return Err(SendError::InvalidBus(
                "nivel de envío fuera de rango (0 a 1)",
            ));
        }
        Ok(Group { sends, ..self })
    }

    /// Volumen, pan y envíos que siguen una curva (como los de una pista).
    pub fn with_mix_automation(self, curves: MixCurves) -> Result<Self, SendError> {
        check_mix(&curves).map_err(SendError::InvalidGroup)?;
        Ok(Group {
            mix_curves: curves,
            ..self
        })
    }

    fn validate(&self) -> Result<(), &'static str> {
        for fx in self.fx.iter().flatten() {
            fx.validate()?;
        }
        for a in self.automation.iter().flatten() {
            a.validate(&self.fx)?;
        }
        check_mix(&self.mix_curves)?;
        if !(0.0..=MAX_GAIN).contains(&self.gain) || !(-1.0..=1.0).contains(&self.pan) {
            return Err("volumen o pan de grupo fuera de rango");
        }
        if self.sends.iter().any(|s| !(0.0..=MAX_SEND).contains(s)) {
            return Err("nivel de envío fuera de rango (0 a 1)");
        }
        Ok(())
    }
}

/// Los grupos de la canción y a cuál va cada pista.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Groups {
    pub groups: [Option<Group>; MAX_GROUPS],
    /// El grupo de cada pista (`member[pista]`); `None`, directa al master.
    pub member: [Option<usize>; MAX_TRACKS],
}

impl Groups {
    pub fn new() -> Self {
        Self::default()
    }

    /// Comprueba cada grupo y que cada pista va a un grupo que existe.
    pub fn validate(&self) -> Result<(), &'static str> {
        for group in self.groups.iter().flatten() {
            group.validate()?;
        }
        if self
            .member
            .iter()
            .flatten()
            .any(|&g| self.groups.get(g).is_none_or(Option::is_none))
        {
            return Err("una pista va a un grupo que no existe");
        }
        Ok(())
    }

    /// El grupo al que va la pista `track`, si existe.
    pub(crate) fn of(&self, track: usize) -> Option<usize> {
        self.member[track].filter(|&g| self.groups.get(g).is_some_and(Option::is_some))
    }
}

/// Lo que dura despierto un grupo sin recibir nada (en segundos): lo que
/// tarde en apagarse la cola de sus efectos.
pub(crate) const GROUP_TAIL_SECONDS: f32 = 1.0;

/// Un grupo en el hilo de audio: el estado de su cadena y lo que se
/// suaviza.
#[derive(Clone, Copy)]
pub(crate) struct GroupState {
    pub fx: [FxState; MAX_FX],
    /// Valor (suavizado) de cada parámetro automatizable mientras se mueve.
    pub mods: [[Option<f32>; MAX_PARAMS]; MAX_FX],
    pub gain: f32,
    pub pan: f32,
    pub sends: [f32; BUSES],
    /// Frames seguidos sin entrada, y si está despierto.
    pub idle: usize,
    pub awake: bool,
}

impl Default for GroupState {
    fn default() -> Self {
        GroupState {
            fx: [FxState::Off; MAX_FX],
            mods: [[None; MAX_PARAMS]; MAX_FX],
            gain: 1.0,
            pan: 0.0,
            sends: [0.0; BUSES],
            idle: 0,
            awake: false,
        }
    }
}

impl GroupState {
    /// Entra la definición `new` del grupo (al empezar el compás) en lugar
    /// de `old`: el estado de los efectos se conserva donde el efecto es del
    /// mismo tipo, y dormido, el volumen, el pan y los envíos entran ya.
    pub fn update(&mut self, old: Option<&Group>, new: Option<&Group>, sample_rate: u32) {
        for (i, mods) in self.mods.iter_mut().enumerate() {
            let kind =
                |g: Option<&Group>| g.and_then(|g| g.fx[i]).map(|f| std::mem::discriminant(&f));
            if kind(old) != kind(new) {
                *mods = [None; MAX_PARAMS];
            }
        }
        for (i, state) in self.fx.iter_mut().enumerate() {
            *state = state.update(new.and_then(|g| g.fx[i]), sample_rate);
        }
        if !self.awake {
            self.snap(new);
        }
    }

    /// Se duerme: los efectos sin memoria y lo suavizado en su sitio.
    pub fn sleep(&mut self, group: &Group, sample_rate: u32) {
        self.awake = false;
        for state in &mut self.fx {
            state.reset();
        }
        for (i, state) in self.fx.iter_mut().enumerate() {
            for (p, m) in self.mods[i].iter_mut().enumerate() {
                if m.take().is_some()
                    && let Some(base) = group.fx[i].and_then(|fx| fx.param_value(p))
                {
                    state.set_param(p, base, sample_rate);
                }
            }
        }
        self.snap(Some(group));
    }

    fn snap(&mut self, group: Option<&Group>) {
        let g = group.copied().unwrap_or_default();
        self.gain = g.gain;
        self.pan = g.pan;
        self.sends = g.sends;
    }
}
