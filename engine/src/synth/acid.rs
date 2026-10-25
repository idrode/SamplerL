//! «acid»: un bajo de un oscilador (sierra o cuadrada) por un paso bajo
//! de 4 polos con mucha resonancia, monofónico, con accent y slide por
//! paso, como el del 303. Es solo una forma de llenar un [`Patch`] del
//! núcleo ([`PRESET`]): sus parámetros son los del núcleo con otro nombre
//! y otra escala (ver [`PARAMS`]).

use super::preset::{Preset, PresetParam, Target, defaults, with_sub_block};
use super::{
    ACCENT, Accent, CUTOFF, DRIVE, ENV_AMOUNT, FILTER_DECAY, FilterMode, GLIDE, Patch, RESONANCE,
    Wave,
};
use crate::Adsr;

/// Nivel de salida: una nota con accent, la resonancia al máximo y el
/// filtro abierto se queda en unos -2 dBFS (medido en
/// `engine/tests/synth.rs`); una sin accent, en unos -10.
pub const LEVEL: f32 = 0.4;

/// Octavas que abre la envolvente de filtro con `envmod` = 1.
pub const ENVMOD_OCTAVES: f32 = 4.0;

/// Los suyos y los del bloque sub ([`super::preset::SUB_BLOCK`]).
pub const PARAMS: [PresetParam; 11] = with_sub_block(OWN);

const OWN: [PresetParam; 7] = [
    PresetParam {
        name: "cutoff",
        target: Target::Param(CUTOFF),
        scale: 1.0,
        min: 20.0,
        max: 20_000.0,
        default: 400.0,
    },
    PresetParam {
        name: "res",
        target: Target::Param(RESONANCE),
        scale: 1.0,
        min: 0.0,
        max: 1.0,
        default: 0.7,
    },
    PresetParam {
        name: "envmod",
        target: Target::Param(ENV_AMOUNT),
        scale: ENVMOD_OCTAVES,
        min: 0.0,
        max: 1.0,
        default: 0.6,
    },
    PresetParam {
        name: "decay",
        target: Target::Param(FILTER_DECAY),
        scale: 1.0,
        min: 10.0,
        max: 5000.0,
        default: 300.0,
    },
    PresetParam {
        name: "accent",
        target: Target::Param(ACCENT),
        scale: 1.0,
        min: 0.0,
        max: 1.0,
        default: 0.6,
    },
    PresetParam {
        name: "dist",
        target: Target::Param(DRIVE),
        scale: 1.0,
        min: 0.0,
        max: 1.0,
        default: 0.0,
    },
    PresetParam {
        name: "glide",
        target: Target::Param(GLIDE),
        scale: 1.0,
        min: 0.0,
        max: 1000.0,
        default: 60.0,
    },
];

/// El parámetro del acid que se llama `name`.
pub fn param(name: &str) -> Option<&'static PresetParam> {
    PRESET.param(name)
}

/// Formas de onda del acid.
pub const WAVES: [Wave; 2] = [Wave::Saw, Wave::Square];

/// El acid: raíz por defecto en C2.
pub const PRESET: Preset = Preset::new("acid", &WAVES, 36, &PARAMS, patch);

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
        zero_phase: false,
        sub_div: 2,
        sub_wave: Wave::Square,
        params: super::PARAMS.map(|p| p.default),
    };
    defaults(&mut patch, &PARAMS);
    patch
}
