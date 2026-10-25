//! «sh101»: un monosynth de bajo y lead como el SH-101. Un oscilador con
//! la sierra y el pulso sumados, el bloque sub (con sus tres modos del
//! original: cuadrada una y dos octavas abajo y pulso al 25 % dos octavas
//! abajo) y ruido, por un paso bajo de 4 polos con saturación en cada
//! etapa que autooscila (y pierde graves al subir la resonancia, como el
//! original, salvo con `res_comp`) y un paso alto fijo (`hpf`), con una
//! sola envolvente para el filtro y el VCA. Monofónico con prioridad a la última nota y
//! legato (ver [`super::Analog`]). Es solo una forma de llenar un
//! [`Patch`] del núcleo ([`PRESET`]).

use super::preset::{Preset, PresetParam, Target, defaults, with_sub_block};
use super::{
    Accent, Analog, CUTOFF, ENV_AMOUNT, FilterMode, GLIDE, KBD, NOISE, PULSE, PW, Patch, RES_COMP,
    RESONANCE, SAW, Wave,
};
use crate::Adsr;

/// Nivel de salida: el preset por defecto pica en unos -9 dBFS y con todo
/// el oscilador, el sub y la resonancia alta y compensada, en unos -5,6
/// (medido en `engine/tests/sh101.rs`).
pub const LEVEL: f32 = 0.8;

/// Octavas que abre (o cierra, en negativo) la envolvente con `env_amt` =
/// 1.
pub const ENV_OCTAVES: f32 = 6.0;

/// Los suyos y los del bloque sub ([`super::preset::SUB_BLOCK`]).
pub const PARAMS: [PresetParam; 19] = with_sub_block(OWN);

const OWN: [PresetParam; 15] = [
    PresetParam {
        name: "saw",
        target: Target::Param(SAW),
        scale: 1.0,
        min: 0.0,
        max: 1.0,
        default: 1.0,
    },
    PresetParam {
        name: "pulse",
        target: Target::Param(PULSE),
        scale: 1.0,
        min: 0.0,
        max: 1.0,
        default: 0.0,
    },
    PresetParam {
        name: "pw",
        target: Target::Param(PW),
        scale: 1.0,
        min: 0.05,
        max: 0.95,
        default: 0.5,
    },
    PresetParam {
        name: "noise",
        target: Target::Param(NOISE),
        scale: 1.0,
        min: 0.0,
        max: 1.0,
        default: 0.0,
    },
    PresetParam {
        name: "cutoff",
        target: Target::Param(CUTOFF),
        scale: 1.0,
        min: 20.0,
        max: 20_000.0,
        default: 1500.0,
    },
    PresetParam {
        name: "res",
        target: Target::Param(RESONANCE),
        scale: 1.0,
        min: 0.0,
        max: 1.0,
        default: 0.3,
    },
    PresetParam {
        name: "kbd",
        target: Target::Param(KBD),
        scale: 1.0,
        min: 0.0,
        max: 1.0,
        default: 0.0,
    },
    PresetParam {
        name: "res_comp",
        target: Target::Param(RES_COMP),
        scale: 1.0,
        min: 0.0,
        max: 1.0,
        default: 0.0,
    },
    PresetParam {
        name: "hpf",
        target: Target::Hpf,
        scale: 1.0,
        min: 0.0,
        max: 3.0,
        default: 0.0,
    },
    PresetParam {
        name: "env_amt",
        target: Target::Param(ENV_AMOUNT),
        scale: ENV_OCTAVES,
        min: -1.0,
        max: 1.0,
        default: 0.4,
    },
    PresetParam {
        name: "a",
        target: Target::Attack,
        scale: 1.0,
        min: 0.0,
        max: 5000.0,
        default: 2.0,
    },
    PresetParam {
        name: "d",
        target: Target::Decay,
        scale: 1.0,
        min: 1.0,
        max: 10_000.0,
        default: 300.0,
    },
    PresetParam {
        name: "s",
        target: Target::Sustain,
        scale: 1.0,
        min: 0.0,
        max: 1.0,
        default: 0.6,
    },
    PresetParam {
        name: "r",
        target: Target::Release,
        scale: 1.0,
        min: 0.0,
        max: 10_000.0,
        default: 100.0,
    },
    PresetParam {
        name: "glide",
        target: Target::Param(GLIDE),
        scale: 1.0,
        min: 0.0,
        max: 2000.0,
        default: 0.0,
    },
];

/// El parámetro del sh101 que se llama `name`.
pub fn param(name: &str) -> Option<&'static PresetParam> {
    PRESET.param(name)
}

/// Su forma de onda es la mezcla de `saw` y `pulse`: no se elige.
pub const WAVES: [Wave; 1] = [Wave::Saw];

/// El sh101: raíz por defecto en C2.
pub const PRESET: Preset = Preset::new("sh101", &WAVES, 36, &PARAMS, patch);

/// El sh101 con sus valores por defecto, con la raíz en `root`.
pub fn patch(root: f32) -> Patch {
    let mut patch = Patch {
        wave: Wave::Saw,
        filter: FilterMode::Ladder,
        unison: 1,
        voices: 1,
        mono: true,
        root,
        // La envolvente de todo (filtro y VCA): la de amplitud.
        amp: Adsr {
            attack_ms: 2.0,
            decay_ms: 300.0,
            sustain: 0.6,
            release_ms: 100.0,
        },
        // No se usa (con sustain, el release no la deja libre).
        filter_env: Adsr {
            attack_ms: 1.0,
            decay_ms: 300.0,
            sustain: 1.0,
            release_ms: 300.0,
        },
        accent: Accent::NONE,
        seed: 0,
        level: LEVEL,
        zero_phase: false,
        sub_div: 2,
        sub_wave: Wave::Square,
        layers: None,
        analog: Some(Analog::DEFAULT),
        params: super::PARAMS.map(|p| p.default),
    };
    defaults(&mut patch, &PARAMS);
    patch
}
