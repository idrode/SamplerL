//! Presets: cada instrumento sintetizado (el acid, el sub...) es una forma
//! de llenar un [`Patch`] del núcleo con unos pocos parámetros con nombre.
//! Un parámetro de un preset va a un parámetro continuo del núcleo (con
//! otra escala, si hace falta) o a la envolvente de amplitud.

use super::{Patch, Wave};
use crate::automation::Curve;

/// Adónde va un parámetro de un preset.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Target {
    /// Un parámetro continuo del núcleo (índice de [`super::PARAMS`]):
    /// admite curvas.
    Param(usize),
    /// El ataque de la envolvente de amplitud (ms).
    Attack,
    /// El release de la envolvente de amplitud (ms).
    Release,
}

/// Un parámetro de un preset: su nombre, adónde va, cuánto se multiplica
/// su valor al pasar al núcleo (`scale`), su rango y su valor por defecto
/// (en las unidades del preset).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PresetParam {
    pub name: &'static str,
    pub target: Target,
    pub scale: f32,
    pub min: f32,
    pub max: f32,
    pub default: f32,
}

impl PresetParam {
    /// Pone el parámetro (en las unidades del preset) en el patch.
    pub fn set(&self, patch: &mut Patch, value: f32) {
        let value = value * self.scale;
        match self.target {
            Target::Param(i) => patch.params[i] = value,
            Target::Attack => patch.amp.attack_ms = value,
            Target::Release => patch.amp.release_ms = value,
        }
    }

    /// Su valor en el patch, en las unidades del preset.
    pub fn get(&self, patch: &Patch) -> f32 {
        let value = match self.target {
            Target::Param(i) => patch.params[i],
            Target::Attack => patch.amp.attack_ms,
            Target::Release => patch.amp.release_ms,
        };
        value / self.scale
    }

    /// Admite `ramp()` y `lfo()`: los que van a un parámetro continuo (la
    /// envolvente se fija con el patch).
    pub fn moves(&self) -> bool {
        matches!(self.target, Target::Param(_))
    }

    /// La curva llevada a las unidades del núcleo (las curvas son rectas y
    /// senos entre dos extremos: basta escalarlos) y el parámetro que
    /// mueve; `None` si no admite curvas.
    pub fn curve(&self, curve: Curve) -> Option<(usize, Curve)> {
        let Target::Param(index) = self.target else {
            return None;
        };
        let s = self.scale;
        let scaled = match curve {
            Curve::Ramp { from, to, steps } => Curve::Ramp {
                from: from * s,
                to: to * s,
                steps,
            },
            Curve::Hit { from, to, steps } => Curve::Hit {
                from: from * s,
                to: to * s,
                steps,
            },
            Curve::Lfo { min, max, rate } => Curve::Lfo {
                min: min * s,
                max: max * s,
                rate,
            },
        };
        Some((index, scaled))
    }
}

/// Un instrumento sintetizado.
#[derive(Debug, Clone, Copy)]
pub struct Preset {
    pub name: &'static str,
    /// Formas de onda que admite (la primera, la de por defecto).
    pub waves: &'static [Wave],
    /// Raíz por defecto (nota MIDI).
    pub root: i32,
    pub params: &'static [PresetParam],
    /// El patch con sus valores por defecto y la raíz dada.
    build: fn(f32) -> Patch,
}

impl Preset {
    pub const fn new(
        name: &'static str,
        waves: &'static [Wave],
        root: i32,
        params: &'static [PresetParam],
        build: fn(f32) -> Patch,
    ) -> Preset {
        Preset {
            name,
            waves,
            root,
            params,
            build,
        }
    }

    /// El patch con sus valores por defecto, con la raíz en `root` (nota
    /// MIDI).
    pub fn patch(&self, root: f32) -> Patch {
        (self.build)(root)
    }

    pub fn param(&self, name: &str) -> Option<&'static PresetParam> {
        self.params.iter().find(|p| p.name == name)
    }
}

/// Los instrumentos sintetizados.
pub const PRESETS: [&Preset; 2] = [&super::acid::PRESET, &super::sub::PRESET];

/// El preset que se llama `name`.
pub fn preset(name: &str) -> Option<&'static Preset> {
    PRESETS.into_iter().find(|p| p.name == name)
}

/// Llena el patch con los valores por defecto de los parámetros.
pub(crate) fn defaults(patch: &mut Patch, params: &[PresetParam]) {
    for p in params {
        p.set(patch, p.default);
    }
}
