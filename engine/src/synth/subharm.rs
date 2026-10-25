//! «subharm»: inspirado en el Moog Subharmonicon. Dos osciladores de
//! sierra, cada uno a la nota dividida por su `div`, y dos subs de cada
//! uno (su frecuencia dividida otra vez), seis capas con su nivel y su
//! pan, por un paso bajo de 4 polos. Cada capa vuelve a sonar cada
//! 1/`rdiv` del compás (ver [`super::Layer`]): dos ejes de división,
//! frecuencia y tiempo. Es solo una forma de llenar un [`Patch`] del
//! núcleo ([`PRESET`]).

use super::preset::{Preset, PresetParam, Target, defaults};
use super::{
    ACCENT, Accent, CUTOFF, DRIVE, ENV_AMOUNT, FILTER_DECAY, FilterMode, GLIDE, LAYER_ATTACK,
    LAYER_DECAY, LAYERS, Layer, Patch, RESONANCE, Wave,
};
use crate::Adsr;

/// Nivel de salida: las seis capas al máximo y en el centro, con el
/// filtro abierto, resonancia y accent, pican en unos -5 dBFS (medido en
/// `engine/tests/synth.rs`).
pub const LEVEL: f32 = 0.3;

/// Las capas por defecto: `o1` a la nota y `s1` una octava abajo; las
/// demás, calladas (con sus divisores de partida: `o2` a la nota, `s2` a
/// un tercio, `s3` a la mitad y `s4` a un cuarto de `o2`).
pub const DEFAULT_LAYERS: [Layer; LAYERS] = [
    Layer::new(1, 1.0),
    Layer::new(1, 0.0),
    Layer::new(2, 0.6),
    Layer::new(3, 0.0),
    Layer::new(2, 0.0),
    Layer::new(4, 0.0),
];

pub const PARAMS: [PresetParam; 9] = [
    PresetParam {
        name: "cutoff",
        target: Target::Param(CUTOFF),
        scale: 1.0,
        min: 20.0,
        max: 20_000.0,
        default: 1200.0,
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
        name: "envmod",
        target: Target::Param(ENV_AMOUNT),
        scale: super::acid::ENVMOD_OCTAVES,
        min: 0.0,
        max: 1.0,
        default: 0.3,
    },
    PresetParam {
        name: "fdecay",
        target: Target::Param(FILTER_DECAY),
        scale: 1.0,
        min: 10.0,
        max: 5000.0,
        default: 400.0,
    },
    PresetParam {
        name: "glide",
        target: Target::Param(GLIDE),
        scale: 1.0,
        min: 0.0,
        max: 1000.0,
        default: 0.0,
    },
    PresetParam {
        name: "drive",
        target: Target::Param(DRIVE),
        scale: 1.0,
        min: 0.0,
        max: 1.0,
        default: 0.0,
    },
    PresetParam {
        name: "attack",
        target: Target::Param(LAYER_ATTACK),
        scale: 1.0,
        min: 0.5,
        max: 2000.0,
        default: 2.0,
    },
    PresetParam {
        name: "decay",
        target: Target::Param(LAYER_DECAY),
        scale: 1.0,
        min: 10.0,
        max: 10_000.0,
        default: 800.0,
    },
    PresetParam {
        name: "voices",
        target: Target::Voices,
        scale: 1.0,
        min: 1.0,
        max: super::MAX_SYNTH_VOICES as f32,
        default: 1.0,
    },
];

/// Formas de onda de sus capas.
pub const WAVES: [Wave; 2] = [Wave::Saw, Wave::Square];

/// El subharm: raíz por defecto en C2.
pub const PRESET: Preset = Preset::new("subharm", &WAVES, 36, &PARAMS, patch);

/// El parámetro del subharm que se llama `name`.
pub fn param(name: &str) -> Option<&'static PresetParam> {
    PRESET.param(name)
}

/// El subharm con sus valores por defecto, con la raíz en `root`.
pub fn patch(root: f32) -> Patch {
    let mut patch = Patch {
        wave: Wave::Saw,
        filter: FilterMode::Ladder,
        unison: 1,
        voices: 1,
        mono: true,
        root,
        // La nota abre la voz; el ritmo lo hacen las capas.
        amp: Adsr {
            attack_ms: 2.0,
            decay_ms: 0.0,
            sustain: 1.0,
            release_ms: 150.0,
        },
        filter_env: Adsr {
            attack_ms: 1.0,
            decay_ms: 400.0,
            sustain: 0.0,
            release_ms: 400.0,
        },
        accent: Accent {
            gain: 0.5,
            octaves: 1.0,
            decay_ms: None,
            sweep_ms: 0.0,
        },
        seed: 0,
        level: LEVEL,
        zero_phase: false,
        sub_div: 2,
        sub_wave: Wave::Square,
        layers: Some(DEFAULT_LAYERS),
        params: super::PARAMS.map(|p| p.default),
    };
    patch.params[ACCENT] = 1.0;
    defaults(&mut patch, &PARAMS);
    patch
}
