//! «acid»: un bajo de un oscilador (sierra o cuadrada) por un paso bajo
//! de 4 polos con mucha resonancia, monofónico, con accent y slide por
//! paso, como el del 303. Es solo una forma de llenar un [`Patch`] del
//! núcleo: sus parámetros son los del núcleo con otro nombre y otra
//! escala (ver [`PARAMS`]).

use super::{
    ACCENT, Accent, CUTOFF, DRIVE, ENV_AMOUNT, FILTER_DECAY, FilterMode, GLIDE, Patch, RESONANCE,
    Wave,
};
use crate::Adsr;
use crate::automation::Curve;

/// Nivel de salida: una nota con accent, la resonancia al máximo y el
/// filtro abierto se queda en unos -2 dBFS (medido en
/// `engine/tests/synth.rs`); una sin accent, en unos -10.
pub const LEVEL: f32 = 0.4;

/// Octavas que abre la envolvente de filtro con `envmod` = 1.
pub const ENVMOD_OCTAVES: f32 = 4.0;

/// Un parámetro del acid: su nombre, el del núcleo al que va
/// (`param`), cuánto se multiplica su valor al pasar al núcleo
/// (`scale`), su rango y su valor por defecto (en las unidades del acid).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct AcidParam {
    pub name: &'static str,
    pub param: usize,
    pub scale: f32,
    pub min: f32,
    pub max: f32,
    pub default: f32,
}

pub const PARAMS: [AcidParam; 7] = [
    AcidParam {
        name: "cutoff",
        param: CUTOFF,
        scale: 1.0,
        min: 20.0,
        max: 20_000.0,
        default: 400.0,
    },
    AcidParam {
        name: "res",
        param: RESONANCE,
        scale: 1.0,
        min: 0.0,
        max: 1.0,
        default: 0.7,
    },
    AcidParam {
        name: "envmod",
        param: ENV_AMOUNT,
        scale: ENVMOD_OCTAVES,
        min: 0.0,
        max: 1.0,
        default: 0.6,
    },
    AcidParam {
        name: "decay",
        param: FILTER_DECAY,
        scale: 1.0,
        min: 10.0,
        max: 5000.0,
        default: 300.0,
    },
    AcidParam {
        name: "accent",
        param: ACCENT,
        scale: 1.0,
        min: 0.0,
        max: 1.0,
        default: 0.6,
    },
    AcidParam {
        name: "dist",
        param: DRIVE,
        scale: 1.0,
        min: 0.0,
        max: 1.0,
        default: 0.0,
    },
    AcidParam {
        name: "glide",
        param: GLIDE,
        scale: 1.0,
        min: 0.0,
        max: 1000.0,
        default: 60.0,
    },
];

/// El parámetro del acid que se llama `name`.
pub fn param(name: &str) -> Option<&'static AcidParam> {
    PARAMS.iter().find(|p| p.name == name)
}

/// Formas de onda del acid.
pub const WAVES: [Wave; 2] = [Wave::Saw, Wave::Square];

/// El acid con sus valores por defecto, con la raíz en `root` (nota MIDI).
pub fn patch(root: f32) -> Patch {
    let mut patch = Patch {
        wave: Wave::Saw,
        filter: FilterMode::Ladder,
        unison: 1,
        voices: 1,
        mono: true,
        root,
        // El VCA: sube en 1,5 ms, baja despacio mientras dura la nota y
        // cae rápido al acabar.
        amp: Adsr {
            attack_ms: 1.5,
            decay_ms: 3000.0,
            sustain: 0.0,
            release_ms: 12.0,
        },
        // La del filtro no tiene sustain: decae hasta 0 aunque la nota
        // siga (el decay es `decay`).
        filter_env: Adsr {
            attack_ms: 0.5,
            decay_ms: 300.0,
            sustain: 0.0,
            release_ms: 300.0,
        },
        // El accent dobla la amplitud, abre el filtro dos octavas más con
        // un decay corto y se acumula si hay varios seguidos.
        accent: Accent {
            gain: 1.0,
            octaves: 2.0,
            decay_ms: Some(200.0),
            sweep_ms: 120.0,
        },
        seed: 0,
        level: LEVEL,
        params: super::PARAMS.map(|p| p.default),
    };
    for p in &PARAMS {
        set(&mut patch, p, p.default);
    }
    patch
}

/// Pone el parámetro `p` del acid (en sus unidades) en el patch.
pub fn set(patch: &mut Patch, p: &AcidParam, value: f32) {
    patch.params[p.param] = value * p.scale;
}

/// La curva de un parámetro del acid llevada a las unidades del núcleo
/// (las curvas son rectas y senos entre dos extremos: basta escalarlos).
pub fn curve(p: &AcidParam, curve: Curve) -> Curve {
    let s = p.scale;
    match curve {
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
    }
}
