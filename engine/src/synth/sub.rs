//! «sub»: un subgrave limpio para dar profundidad. Un seno (o una
//! triangular, con algún armónico más) por un paso bajo suave, monofónico,
//! que empieza cada nota en el cruce por cero (sin golpe al arrancar) y
//! sigue su fase de una nota a la siguiente (sin clic entre notas). Una
//! saturación opcional antes del filtro (`drive`) le da los armónicos que
//! lo hacen oír en altavoces pequeños sin ensuciarlo por arriba. Es solo
//! una forma de llenar un [`Patch`] del núcleo ([`PRESET`]).

use super::preset::{Preset, PresetParam, Target, defaults};
use super::{Accent, CUTOFF, FilterMode, GLIDE, PRE_DRIVE, Patch, RESONANCE, Wave};
use crate::Adsr;

/// Nivel de salida: una nota sin accent pica en unos -6,5 dBFS (un seno
/// suena fuerte con poco pico) y con accent, en unos -3,5 (medido en
/// `engine/tests/synth.rs`).
pub const LEVEL: f32 = 0.5;

pub const PARAMS: [PresetParam; 5] = [
    PresetParam {
        name: "tone",
        target: Target::Param(CUTOFF),
        scale: 1.0,
        min: 40.0,
        max: 2000.0,
        default: 250.0,
    },
    PresetParam {
        name: "drive",
        target: Target::Param(PRE_DRIVE),
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
        default: 40.0,
    },
    PresetParam {
        name: "attack",
        target: Target::Attack,
        scale: 1.0,
        min: 0.0,
        max: 2000.0,
        default: 5.0,
    },
    PresetParam {
        name: "release",
        target: Target::Release,
        scale: 1.0,
        min: 0.0,
        max: 5000.0,
        default: 80.0,
    },
];

/// Formas de onda del sub.
pub const WAVES: [Wave; 2] = [Wave::Sine, Wave::Triangle];

/// El sub: raíz por defecto en C1 (32,7 Hz).
pub const PRESET: Preset = Preset::new("sub", &WAVES, 24, &PARAMS, patch);

/// El parámetro del sub que se llama `name`.
pub fn param(name: &str) -> Option<&'static PresetParam> {
    PRESET.param(name)
}

/// El sub con sus valores por defecto, con la raíz en `root` (nota MIDI).
pub fn patch(root: f32) -> Patch {
    let mut patch = Patch {
        wave: Wave::Sine,
        // Variable de estado sin resonancia (Q = 0,5): cae sin pico.
        filter: FilterMode::Lowpass,
        unison: 1,
        voices: 1,
        mono: true,
        root,
        // Sostenido mientras dura la nota, sin decay.
        amp: Adsr {
            attack_ms: 5.0,
            decay_ms: 0.0,
            sustain: 1.0,
            release_ms: 80.0,
        },
        // Sin envolvente de filtro (su cantidad es 0).
        filter_env: Adsr {
            attack_ms: 0.0,
            decay_ms: 0.0,
            sustain: 0.0,
            release_ms: 0.0,
        },
        // El accent: un 40 % más fuerte (unos 3 dB), sin abrir el filtro.
        accent: Accent {
            gain: 0.4,
            octaves: 0.0,
            decay_ms: None,
            sweep_ms: 0.0,
        },
        seed: 0,
        level: LEVEL,
        zero_phase: true,
        params: super::PARAMS.map(|p| p.default),
    };
    patch.params[RESONANCE] = 0.0;
    patch.params[super::ENV_AMOUNT] = 0.0;
    patch.params[super::ACCENT] = 1.0;
    defaults(&mut patch, &PARAMS);
    patch
}
