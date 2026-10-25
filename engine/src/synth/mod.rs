//! Síntesis: osciladores, filtros y envolventes en un banco de voces por
//! pista, sin samples. Es el núcleo común de los instrumentos sintéticos
//! (el primero, [`acid`]; luego vendrán pads, leads y wavetable): nada de
//! aquí es de un instrumento concreto.
//!
//! Un [`Patch`] dice cómo suena (forma de onda, unison, filtro,
//! envolventes, glide, accent) y va en el patrón como cualquier otra
//! configuración. El estado ([`Synth`]) vive en la pista del motor, con
//! todas sus voces reservadas al crearla: tocar no reserva memoria.
//!
//! Cada voz: osciladores (hasta [`MAX_UNISON`] desafinados, con
//! suboscilador una octava abajo y ruido) → filtro (con su envolvente) →
//! amplitud (con la suya) → saturación. Los parámetros continuos
//! ([`PARAMS`]) admiten curvas: el motor las evalúa cada
//! [`crate::engine::MOD_BLOCK`] frames, las suaviza (~5 ms) y cada voz las
//! interpola muestra a muestra, así que nada salta ni hace clic.
//!
//! Todo es determinista: la fase inicial de cada oscilador y el ruido
//! salen de [`Patch::seed`] y del número de la nota.

pub mod acid;
mod env;
mod filter;
mod osc;
pub mod preset;
pub mod sub;
pub mod subharm;

use crate::Adsr;
use crate::automation::{Curve, mix};
use env::{Env, Rates, Stage};
use filter::{Filter, soft, warp};
use osc::{Halfband, Noise, Osc};

/// Osciladores desafinados por voz como máximo (unison).
pub const MAX_UNISON: usize = 7;
/// Voces de un sintetizador como máximo.
pub const MAX_SYNTH_VOICES: usize = 8;
/// Ranuras de voz por pista: las que suenan y las robadas que hacen su
/// fundido (nunca faltan: como mucho suenan [`MAX_SYNTH_VOICES`]).
const POOL: usize = 2 * MAX_SYNTH_VOICES;
/// Fundido de una voz robada o cortada, en segundos (el de las voces de
/// los samples).
const KILL_SECONDS: f32 = 0.0015;
/// Lo más corto que dura un tramo de la envolvente de amplitud (evita
/// los clics), en ms.
const MIN_AMP_MS: f32 = 1.5;
/// Lo más corto de un tramo de la envolvente de filtro, en ms.
const MIN_FILTER_MS: f32 = 0.5;
/// `gate` de una nota que suena hasta su note off.
pub const HELD: u64 = u64::MAX;

/// Forma de onda de los osciladores.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Wave {
    Sine,
    Saw,
    /// Pulso con el ancho de [`PW`].
    Square,
    Triangle,
}

impl Wave {
    pub const ALL: [Wave; 4] = [Wave::Sine, Wave::Saw, Wave::Square, Wave::Triangle];

    /// Nombre en el script.
    pub fn name(self) -> &'static str {
        match self {
            Wave::Sine => "sine",
            Wave::Saw => "saw",
            Wave::Square => "square",
            Wave::Triangle => "triangle",
        }
    }

    pub fn from_name(name: &str) -> Option<Wave> {
        Wave::ALL.into_iter().find(|w| w.name() == name)
    }

    /// Fase (0 a 1) en la que la onda cruza el cero subiendo: donde empieza
    /// una nota de un patch con [`Patch::zero_phase`]. La cuadrada no lo
    /// cruza (salta): empieza en 0.
    pub fn zero_phase(self) -> f64 {
        match self {
            Wave::Sine | Wave::Square => 0.0,
            Wave::Saw => 0.5,
            Wave::Triangle => 0.75,
        }
    }
}

/// Filtro de las voces.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FilterMode {
    /// Paso bajo de 12 dB/oct (variable de estado).
    Lowpass,
    Highpass,
    Bandpass,
    /// Paso bajo de 4 polos (24 dB/oct) con resonancia que llega a
    /// autooscilar y saturación interna.
    Ladder,
}

impl FilterMode {
    pub const ALL: [FilterMode; 4] = [
        FilterMode::Lowpass,
        FilterMode::Highpass,
        FilterMode::Bandpass,
        FilterMode::Ladder,
    ];

    pub fn name(self) -> &'static str {
        match self {
            FilterMode::Lowpass => "lowpass",
            FilterMode::Highpass => "highpass",
            FilterMode::Bandpass => "bandpass",
            FilterMode::Ladder => "ladder",
        }
    }

    pub fn from_name(name: &str) -> Option<FilterMode> {
        FilterMode::ALL.into_iter().find(|f| f.name() == name)
    }
}

/// Un parámetro continuo: nombre, rango, valor por defecto y si se
/// interpola en escala logarítmica (las frecuencias y los tiempos).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SynthParam {
    pub name: &'static str,
    pub min: f32,
    pub max: f32,
    pub default: f32,
    pub log: bool,
}

/// Corte del filtro (Hz).
pub const CUTOFF: usize = 0;
/// Resonancia (0 a 1; en el ladder, 1 autooscila).
pub const RESONANCE: usize = 1;
/// Cuánto abre la envolvente de filtro el corte, en octavas (negativo,
/// lo cierra).
pub const ENV_AMOUNT: usize = 2;
/// Decay de la envolvente de filtro (ms).
pub const FILTER_DECAY: usize = 3;
/// Saturación a la salida de la voz (0 sin ella, 1 la máxima).
pub const DRIVE: usize = 4;
/// Cuánto pesa el accent de un paso (0 nada, 1 todo lo de
/// [`Accent`]).
pub const ACCENT: usize = 5;
/// Tiempo del glide de las notas ligadas (ms).
pub const GLIDE: usize = 6;
/// Ancho de pulso de la cuadrada (0,5 es cuadrada).
pub const PW: usize = 7;
/// Desafinación del unison: los osciladores de los extremos, ± estos
/// cents.
pub const DETUNE: usize = 8;
/// Nivel del sub: una onda a la frecuencia del oscilador central dividida
/// por [`Patch::sub_div`], sacada de su fase (ver [`SubNote`]).
pub const SUB: usize = 9;
/// Nivel del ruido.
pub const NOISE: usize = 10;
/// Apertura estéreo del unison (0 en el centro, 1 los extremos a cada
/// lado).
pub const SPREAD: usize = 11;
/// Saturación de los osciladores antes del filtro (0 sin ella): añade
/// armónicos que el filtro deja pasar o no.
pub const PRE_DRIVE: usize = 12;
/// Pan del sub (-1 a 1, potencia constante).
pub const SUB_PAN: usize = 13;
/// Saturación de la suma de los osciladores y el sub, antes del filtro (0
/// sin ella; solo con el sub sonando).
pub const SUB_DRIVE: usize = 14;
/// Parámetros continuos.
pub const PARAM_COUNT: usize = 15;
/// Divisor del sub como mucho.
pub const MAX_SUB_DIV: u8 = 16;

/// Los parámetros continuos, por su índice.
pub const PARAMS: [SynthParam; PARAM_COUNT] = [
    SynthParam {
        name: "cutoff",
        min: 20.0,
        max: 20_000.0,
        default: 2000.0,
        log: true,
    },
    SynthParam {
        name: "resonance",
        min: 0.0,
        max: 1.0,
        default: 0.2,
        log: false,
    },
    SynthParam {
        name: "env_amount",
        min: -8.0,
        max: 8.0,
        default: 0.0,
        log: false,
    },
    SynthParam {
        name: "filter_decay",
        min: 1.0,
        max: 10_000.0,
        default: 300.0,
        log: true,
    },
    SynthParam {
        name: "drive",
        min: 0.0,
        max: 1.0,
        default: 0.0,
        log: false,
    },
    SynthParam {
        name: "accent",
        min: 0.0,
        max: 1.0,
        default: 0.5,
        log: false,
    },
    SynthParam {
        name: "glide",
        min: 0.0,
        max: 2000.0,
        default: 60.0,
        log: false,
    },
    SynthParam {
        name: "pw",
        min: 0.05,
        max: 0.95,
        default: 0.5,
        log: false,
    },
    SynthParam {
        name: "detune",
        min: 0.0,
        max: 100.0,
        default: 0.0,
        log: false,
    },
    SynthParam {
        name: "sub",
        min: 0.0,
        max: 1.0,
        default: 0.0,
        log: false,
    },
    SynthParam {
        name: "noise",
        min: 0.0,
        max: 1.0,
        default: 0.0,
        log: false,
    },
    SynthParam {
        name: "spread",
        min: 0.0,
        max: 1.0,
        default: 0.0,
        log: false,
    },
    SynthParam {
        name: "pre_drive",
        min: 0.0,
        max: 1.0,
        default: 0.0,
        log: false,
    },
    SynthParam {
        name: "sub_pan",
        min: -1.0,
        max: 1.0,
        default: 0.0,
        log: false,
    },
    SynthParam {
        name: "sub_drive",
        min: 0.0,
        max: 1.0,
        default: 0.0,
        log: false,
    },
];

/// Lo que un paso cambia del sub de su nota (`{sub=0.5,sub_div=3}`): cada
/// cosa que se da manda sobre la del patch mientras suena esa nota.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct SubNote {
    pub level: Option<f32>,
    pub div: Option<u8>,
    pub wave: Option<Wave>,
    pub pan: Option<f32>,
    pub drive: Option<f32>,
}

/// Formas de onda del sub.
pub const SUB_WAVES: [Wave; 3] = [Wave::Square, Wave::Triangle, Wave::Sine];

/// Capas de un patch de capas ([`Patch::layers`], el del `subharm`): los
/// dos osciladores principales y dos subs de cada uno.
pub const LAYERS: usize = 6;
/// De qué oscilador principal sale cada capa (los dos primeros son ellos
/// mismos): `o1`, `o2`, `s1` y `s2` de `o1`, `s3` y `s4` de `o2`.
pub const LAYER_PARENT: [usize; LAYERS] = [0, 1, 0, 0, 1, 1];
/// Nombre de cada capa en el script.
pub const LAYER_NAMES: [&str; LAYERS] = ["o1", "o2", "s1", "s2", "s3", "s4"];

/// Una capa: su divisor de frecuencia (el de la nota en los osciladores
/// principales; el de su oscilador en los subs), su divisor rítmico (la
/// capa vuelve a sonar cada 1/`rdiv` del compás), su pan (potencia
/// constante) y su nivel.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Layer {
    pub div: u8,
    pub rdiv: u8,
    pub pan: f32,
    pub level: f32,
}

impl Layer {
    pub const fn new(div: u8, level: f32) -> Layer {
        Layer {
            div,
            rdiv: 1,
            pan: 0.0,
            level,
        }
    }

    /// Comprueba los rangos.
    pub fn validate(&self) -> Result<(), &'static str> {
        if !(1..=MAX_SUB_DIV).contains(&self.div) {
            return Err("div de una capa va de 1 a 16");
        }
        if !(1..=MAX_SUB_DIV).contains(&self.rdiv) {
            return Err("rdiv de una capa va de 1 a 16");
        }
        if !(-1.0..=1.0).contains(&self.pan) {
            return Err("pan de una capa va de -1 a 1");
        }
        if !(0.0..=1.0).contains(&self.level) {
            return Err("level de una capa va de 0 a 1");
        }
        Ok(())
    }
}

impl SubNote {
    /// Comprueba los rangos (los de [`PARAMS`] y [`MAX_SUB_DIV`]).
    pub fn validate(&self) -> Result<(), &'static str> {
        let within = |v: Option<f32>, i: usize| {
            v.is_none_or(|v| (PARAMS[i].min..=PARAMS[i].max).contains(&v))
        };
        if !within(self.level, SUB) || !within(self.drive, SUB_DRIVE) {
            return Err("sub y sub_drive van de 0 a 1");
        }
        if !within(self.pan, SUB_PAN) {
            return Err("sub_pan va de -1 a 1");
        }
        if self.div.is_some_and(|d| !(1..=MAX_SUB_DIV).contains(&d)) {
            return Err("sub_div va de 1 a 16");
        }
        if self.wave.is_some_and(|w| !SUB_WAVES.contains(&w)) {
            return Err("sub_wave es square, tri o sine");
        }
        Ok(())
    }
}

/// Ganancias izquierda y derecha de un pan de potencia constante (-1 a
/// 1): en el centro, 1 en cada canal; a un lado, √2 en ese y 0 en el otro.
#[inline]
pub(crate) fn constant_power(pan: f32) -> (f32, f32) {
    let angle = (pan.clamp(-1.0, 1.0) + 1.0) * std::f32::consts::FRAC_PI_4;
    (
        std::f32::consts::SQRT_2 * angle.cos(),
        std::f32::consts::SQRT_2 * angle.sin(),
    )
}

/// Una curva (o ninguna) por parámetro continuo.
pub type SynthCurves = [Option<Curve>; PARAM_COUNT];

/// Lo que hace el accent de un paso con [`ACCENT`] = 1 (con menos, la
/// parte proporcional).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Accent {
    /// Amplitud de más: 1 la dobla.
    pub gain: f32,
    /// Octavas de más que abre la envolvente de filtro.
    pub octaves: f32,
    /// Decay de la envolvente de filtro de las notas con accent (ms);
    /// `None`, el de siempre.
    pub decay_ms: Option<f32>,
    /// Memoria del accent: lo que abre se suaviza con esta constante de
    /// tiempo (ms), así que varios accents seguidos se acumulan; 0, sin
    /// memoria.
    pub sweep_ms: f32,
}

impl Accent {
    pub const NONE: Accent = Accent {
        gain: 0.0,
        octaves: 0.0,
        decay_ms: None,
        sweep_ms: 0.0,
    };
}

/// Cómo suena un sintetizador.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Patch {
    pub wave: Wave,
    pub filter: FilterMode,
    /// Osciladores por voz (1 a [`MAX_UNISON`]).
    pub unison: u8,
    /// Voces a la vez (1 a [`MAX_SYNTH_VOICES`]); con todas ocupadas, la
    /// nota nueva roba la más antigua.
    pub voices: u8,
    /// Monofónico: una sola voz, que se reutiliza (los osciladores siguen
    /// su fase y las envolventes salen del nivel en que estén); las notas
    /// con slide se ligan (glide, sin volver a disparar las envolventes).
    pub mono: bool,
    /// Nota MIDI (con decimales) que suena con 0 semitonos.
    pub root: f32,
    /// Envolvente de amplitud.
    pub amp: Adsr,
    /// Envolvente de filtro (su decay es el parámetro [`FILTER_DECAY`]).
    pub filter_env: Adsr,
    pub accent: Accent,
    /// Semilla de las fases iniciales y del ruido.
    pub seed: u64,
    /// Ganancia de salida de cada voz (lineal, de 0 a 4): deja sitio al
    /// accent y a la resonancia (el volumen que se mueve es el de la
    /// pista).
    pub level: f32,
    /// Cada voz que empieza desde el silencio lo hace donde su onda cruza
    /// el cero subiendo ([`Wave::zero_phase`]), sin fase al azar: una nota
    /// grave no empieza con un salto (un golpe sordo).
    pub zero_phase: bool,
    /// Divisor de frecuencia del sub (1 a [`MAX_SUB_DIV`]) y su forma de
    /// onda (una de [`SUB_WAVES`]).
    pub sub_div: u8,
    pub sub_wave: Wave,
    /// Capas (el `subharm`): con ellas, cada voz toca dos osciladores
    /// principales (a la nota dividida por su `div`) y dos subs de cada uno
    /// (sacados de su fase como el sub, sin unison ni bloque sub), cada
    /// capa con su nivel y su pan. Ver [`Layer`].
    pub layers: Option<[Layer; LAYERS]>,
    /// Valores de los parámetros continuos ([`PARAMS`]).
    pub params: [f32; PARAM_COUNT],
}

impl Patch {
    /// Un sintetizador sencillo: sierra, paso bajo, una voz por nota, los
    /// parámetros por defecto.
    pub fn new() -> Patch {
        Patch {
            wave: Wave::Saw,
            filter: FilterMode::Lowpass,
            unison: 1,
            voices: 4,
            mono: false,
            root: 60.0,
            amp: Adsr {
                attack_ms: 2.0,
                decay_ms: 100.0,
                sustain: 1.0,
                release_ms: 100.0,
            },
            filter_env: Adsr {
                attack_ms: 1.0,
                decay_ms: 300.0,
                sustain: 0.0,
                release_ms: 300.0,
            },
            accent: Accent::NONE,
            seed: 0,
            level: 1.0,
            zero_phase: false,
            sub_div: 2,
            sub_wave: Wave::Square,
            layers: None,
            params: PARAMS.map(|p| p.default),
        }
    }

    pub fn with(mut self, param: usize, value: f32) -> Patch {
        self.params[param] = value;
        self
    }

    /// Comprueba los rangos.
    pub fn validate(&self) -> Result<(), &'static str> {
        if !(1..=MAX_UNISON).contains(&(self.unison as usize)) {
            return Err("unison fuera de rango (1 a 7)");
        }
        if !(1..=MAX_SYNTH_VOICES).contains(&(self.voices as usize)) {
            return Err("voces del sintetizador fuera de rango (1 a 8)");
        }
        if !(0.0..=127.0).contains(&self.root) {
            return Err("raíz fuera de rango (0 a 127)");
        }
        if !(0.0..=4.0).contains(&self.level) {
            return Err("nivel de salida fuera de rango (0 a 4)");
        }
        SubNote {
            div: Some(self.sub_div),
            wave: Some(self.sub_wave),
            ..SubNote::default()
        }
        .validate()?;
        if let Some(layers) = &self.layers {
            for layer in layers {
                layer.validate()?;
            }
        }
        for (value, p) in self.params.iter().zip(&PARAMS) {
            if !(p.min..=p.max).contains(value) {
                return Err("parámetro del sintetizador fuera de rango");
            }
        }
        for env in [&self.amp, &self.filter_env] {
            let ms = 0.0..=crate::instrument::MAX_ENVELOPE_MS;
            if !(ms.contains(&env.attack_ms)
                && ms.contains(&env.decay_ms)
                && ms.contains(&env.release_ms)
                && (0.0..=1.0).contains(&env.sustain))
            {
                return Err("envolvente del sintetizador fuera de rango");
            }
        }
        let a = &self.accent;
        if !((0.0..=4.0).contains(&a.gain)
            && (0.0..=8.0).contains(&a.octaves)
            && (0.0..=10_000.0).contains(&a.sweep_ms)
            && a.decay_ms.is_none_or(|d| (1.0..=10_000.0).contains(&d)))
        {
            return Err("accent del sintetizador fuera de rango");
        }
        Ok(())
    }
}

impl Default for Patch {
    fn default() -> Self {
        Patch::new()
    }
}

/// Comprueba que cada curva cabe en el rango de su parámetro (y que es
/// una curva de la canción, no de un golpe).
pub fn check_curves(curves: &SynthCurves) -> Result<(), &'static str> {
    for (curve, p) in curves.iter().zip(&PARAMS) {
        let Some(curve) = curve else { continue };
        let (a, b) = curve.bounds();
        if ![a, b].iter().all(|v| (p.min..=p.max).contains(v)) {
            return Err("la curva se sale del rango del parámetro del sintetizador");
        }
        crate::automation::check_curve(curve)?;
    }
    Ok(())
}

/// Una nota para el sintetizador.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct NoteEvent {
    /// Nota MIDI con decimales (60 es el do central).
    pub pitch: f32,
    /// Volumen (lineal).
    pub gain: f32,
    pub accent: bool,
    /// Llega ligada desde la nota anterior: en un sintetizador mono que
    /// sigue sonando, glide sin volver a disparar las envolventes; si no,
    /// empieza en la altura de la anterior y se desliza.
    pub slide: bool,
    /// Frames hasta el release ([`HELD`]: hasta su note off).
    pub gate: u64,
    /// Tecla que la toca en vivo (su note off la suelta).
    pub key: Option<u8>,
    /// Lo que el paso cambia del sub.
    pub sub: SubNote,
}

impl NoteEvent {
    pub fn new(pitch: f32, gate: u64) -> NoteEvent {
        NoteEvent {
            pitch,
            gain: 1.0,
            accent: false,
            slide: false,
            gate,
            key: None,
            sub: SubNote::default(),
        }
    }
}

/// Una voz.
#[derive(Debug, Clone, Copy)]
struct Voice {
    active: bool,
    born: u64,
    key: Option<u8>,
    /// Altura que suena y a la que va (nota MIDI), y su frecuencia en
    /// ciclos por muestra.
    pitch: f32,
    target: f32,
    dt: f64,
    osc: [Osc; MAX_UNISON],
    /// Sub: los reinicios del oscilador central contados módulo su
    /// divisor (su fase es `(cuenta + fase del central) / divisor`), lo
    /// que cambia el paso y el nivel y el pan suavizados.
    sub_count: u32,
    sub_note: SubNote,
    sub_level: f32,
    sub_pan: f32,
    /// La voz sonaba en estéreo en la muestra anterior.
    was_stereo: bool,
    /// Capas: los reinicios contados de los subs (`s1` a `s4`) y el nivel
    /// y el pan de cada capa, suavizados.
    layer_count: [u32; LAYERS],
    layer_level: [f32; LAYERS],
    layer_pan: [f32; LAYERS],
    noise: Noise,
    /// Diezmado de los osciladores (van al doble de frecuencia de
    /// muestreo), por canal.
    down: [Halfband; 2],
    filter: [Filter; 2],
    amp: Env,
    fenv: Env,
    /// Volumen y accent (0 a 1) de la nota.
    gain: f32,
    accent: f32,
    /// Memoria del accent ([`Accent::sweep_ms`]).
    sweep: f32,
    /// Frames hasta el release ([`HELD`]: ninguno).
    gate: u64,
    /// Robada o cortada: frames que le quedan al fundido.
    killed: Option<u32>,
}

impl Default for Voice {
    fn default() -> Self {
        Voice {
            active: false,
            born: 0,
            key: None,
            pitch: 60.0,
            target: 60.0,
            dt: 0.0,
            osc: [Osc::default(); MAX_UNISON],
            sub_count: 0,
            sub_note: SubNote::default(),
            sub_level: 0.0,
            sub_pan: 0.0,
            was_stereo: false,
            layer_count: [0; LAYERS],
            layer_level: [0.0; LAYERS],
            layer_pan: [0.0; LAYERS],
            noise: Noise::new(0),
            down: [Halfband::default(); 2],
            filter: [Filter::default(); 2],
            amp: Env::default(),
            fenv: Env::default(),
            gain: 0.0,
            accent: 0.0,
            sweep: 0.0,
            gate: HELD,
            killed: None,
        }
    }
}

/// Ciclos por muestra de la nota MIDI `pitch`.
fn cycles(pitch: f32, sample_rate: f32) -> f64 {
    440.0 * ((pitch as f64 - 69.0) / 12.0).exp2() / sample_rate as f64
}

impl Voice {
    /// Suena (en su release o en el fundido de robo también).
    fn sounding(&self) -> bool {
        self.active
    }

    /// Cuenta como voz: suena y no está robada ni cortada.
    fn is_voice(&self) -> bool {
        self.active && self.killed.is_none()
    }

    /// Empieza desde el silencio: fases al azar (de la semilla), filtro
    /// vacío, envolventes desde 0.
    fn start(&mut self, patch: &Patch, born: u64, sample_rate: f32) {
        let mut rng = patch.seed ^ born.wrapping_mul(0x2545_f491_4f6c_dd1d);
        *self = Voice::default();
        for osc in &mut self.osc {
            osc.phase = osc::unit(&mut rng);
        }
        // Lo que antes era la fase del suboscilador: se sigue sacando del
        // azar para que las demás fases no cambien.
        let _ = osc::unit(&mut rng);
        if patch.zero_phase {
            for osc in &mut self.osc {
                osc.phase = patch.wave.zero_phase();
            }
        }
        // Las capas empiezan donde las pide el patch (sin subir desde 0:
        // la envolvente de amplitud ya sube).
        if let Some(layers) = &patch.layers {
            self.layer_level = layers.map(|l| l.level);
            self.layer_pan = layers.map(|l| l.pan);
        }
        self.noise = Noise::new(osc::splitmix(&mut rng));
        self.active = true;
        self.dt = cycles(self.pitch, sample_rate);
    }

    /// Dispara la nota en esta voz (nueva o reutilizada).
    fn play(&mut self, ev: &NoteEvent, born: u64, from: Option<f32>, sr: f32) {
        self.born = born;
        self.key = ev.key;
        self.target = ev.pitch;
        self.pitch = from.unwrap_or(ev.pitch);
        self.dt = cycles(self.pitch, sr);
        self.gain = ev.gain;
        self.accent = if ev.accent { 1.0 } else { 0.0 };
        self.gate = ev.gate;
        self.killed = None;
        self.sub_note = ev.sub;
        self.amp.trigger();
        self.fenv.trigger();
    }

    /// Liga la nota a la que suena: va a su altura con glide, con su gate
    /// y su volumen, sin volver a disparar las envolventes (la de amplitud
    /// vuelve a subir si estaba en su release).
    fn tie(&mut self, ev: &NoteEvent, born: u64) {
        self.born = born;
        self.key = ev.key;
        self.sub_note = ev.sub;
        self.target = ev.pitch;
        self.gain = ev.gain;
        self.accent = if ev.accent { 1.0 } else { 0.0 };
        self.gate = ev.gate;
        if self.amp.stage == Stage::Release {
            self.amp.trigger();
        }
        if self.fenv.stage == Stage::Release {
            self.fenv.stage = Stage::Decay;
        }
    }

    /// Empieza el release. La envolvente de filtro sin sustain sigue su
    /// decay hasta 0 sin mirar el gate (`free_filter`).
    fn release(&mut self, free_filter: bool) {
        self.amp.release();
        if !(free_filter && self.fenv.stage == Stage::Decay) {
            self.fenv.release();
        }
        self.gate = HELD;
    }

    fn kill(&mut self, fade: u32) {
        if self.killed.is_none() {
            self.killed = Some(fade);
        }
    }
}

/// Lo que una voz necesita de un bloque: los parámetros al principio y al
/// final (los interpola) y lo que se calcula una vez por bloque.
struct Block<'a> {
    patch: &'a Patch,
    from: [f32; PARAM_COUNT],
    to: [f32; PARAM_COUNT],
    amp: Rates,
    fenv: Rates,
    /// Las notas con accent, con el decay de [`Accent::decay_ms`].
    fenv_accent: Rates,
    /// Coeficiente del glide (1: sin glide) y de la memoria del accent.
    glide: f32,
    sweep: f32,
    /// Multiplicador de frecuencia y pan (de -1 a 1) de cada oscilador.
    ratio: [f64; MAX_UNISON],
    pan: [f32; MAX_UNISON],
    stereo: bool,
    /// Coeficiente del suavizado por muestra del nivel y el pan del sub.
    smooth: f32,
    sample_rate: f32,
    fade: u32,
}

/// Coeficiente de un polo de `ms` (99 % del camino en ese tiempo); con 0,
/// salta.
fn follow(ms: f32, sample_rate: f32) -> f32 {
    if ms <= 0.0 {
        return 1.0;
    }
    1.0 - (-4.605_17 / (ms / 1000.0 * sample_rate)).exp()
}

impl Voice {
    /// Una muestra de salida de las capas (estéreo, ya diezmada): los dos
    /// osciladores principales a la nota dividida por su `div` y los subs
    /// de cada uno sacados de su fase, cada capa con su nivel y su pan
    /// (suavizados), al doble de la frecuencia de muestreo como siempre.
    fn layers_sample(&mut self, layers: &[Layer; LAYERS], wave: Wave, b: &Block) -> (f32, f32) {
        let mut gains = [(0.0f32, 0.0f32); LAYERS];
        for (i, layer) in layers.iter().enumerate() {
            self.layer_level[i] += (layer.level - self.layer_level[i]) * b.smooth;
            self.layer_pan[i] += (layer.pan - self.layer_pan[i]) * b.smooth;
            if (layer.level - self.layer_level[i]).abs() < 1e-6 {
                self.layer_level[i] = layer.level;
            }
            if (layer.pan - self.layer_pan[i]).abs() < 1e-6 {
                self.layer_pan[i] = layer.pan;
            }
            let (gl, gr) = constant_power(self.layer_pan[i]);
            gains[i] = (gl * self.layer_level[i], gr * self.layer_level[i]);
        }
        let dt = 0.5 * self.dt;
        for _ in 0..2 {
            let (mut l, mut r) = (0.0f32, 0.0f32);
            let mut add = |y: f32, (gl, gr): (f32, f32)| {
                l += y * gl;
                r += y * gr;
            };
            for m in 0..2 {
                let main_dt = dt / layers[m].div.max(1) as f64;
                let at = self.osc[m].phase;
                // Los subs de este oscilador, de su fase de ahora.
                for i in (0..LAYERS).filter(|&i| i >= 2 && LAYER_PARENT[i] == m) {
                    let div = layers[i].div.max(1) as u32;
                    if self.layer_count[i] >= div {
                        self.layer_count[i] %= div;
                    }
                    if gains[i] != (0.0, 0.0) {
                        let phase = (self.layer_count[i] as f64 + at) / div as f64;
                        add(
                            osc::shape(wave, phase, main_dt / div as f64, 0.5) as f32,
                            gains[i],
                        );
                    }
                }
                let y = self.osc[m].next(wave, main_dt, 0.5);
                if gains[m] != (0.0, 0.0) {
                    add(y, gains[m]);
                }
                if self.osc[m].phase < at {
                    for i in (0..LAYERS).filter(|&i| i >= 2 && LAYER_PARENT[i] == m) {
                        let div = layers[i].div.max(1) as u32;
                        self.layer_count[i] = (self.layer_count[i] + 1) % div;
                    }
                }
            }
            self.down[0].push(l);
            self.down[1].push(r);
        }
        (self.down[0].output(), self.down[1].output())
    }
}

impl Voice {
    /// Suma `frames` frames de la voz en `out` (estéreo entrelazado).
    fn render(&mut self, out: &mut [f32], b: &Block) {
        let patch = b.patch;
        let n = patch.unison as usize;
        let norm = 1.0 / (n as f32).sqrt();
        let frames = out.len() / 2;
        let step = 1.0 / frames as f32;
        let fenv_rates = if self.accent > 0.0 && patch.accent.decay_ms.is_some() {
            &b.fenv_accent
        } else {
            &b.fenv
        };
        let (log_from, log_to) = (b.from[CUTOFF].log2(), b.to[CUTOFF].log2());
        for (i, frame) in out.as_chunks_mut::<2>().0.iter_mut().enumerate() {
            if !self.active {
                break;
            }
            let t = i as f32 * step;
            let p = |k: usize| b.from[k] + (b.to[k] - b.from[k]) * t;
            // Gate: al llegar a 0 empieza el release.
            if self.gate != HELD {
                if self.gate == 0 {
                    self.release(patch.filter_env.sustain == 0.0);
                } else {
                    self.gate -= 1;
                }
            }
            if self.pitch != self.target {
                self.pitch += (self.target - self.pitch) * b.glide;
                if (self.target - self.pitch).abs() < 1e-4 {
                    self.pitch = self.target;
                }
                self.dt = cycles(self.pitch, b.sample_rate);
            }
            let (mut l, mut r, stereo, sub_on, sub) = match &patch.layers {
                Some(layers) => {
                    let (l, r) = self.layers_sample(layers, patch.wave, b);
                    (l, r, true, false, SubNote::default())
                }
                None => {
                    // Sub: el nivel y el pan del paso (o los del patch), suavizados.
                    let sub = &self.sub_note;
                    let level_to = sub.level.unwrap_or_else(|| p(SUB));
                    let pan_to = sub.pan.unwrap_or_else(|| p(SUB_PAN));
                    self.sub_level += (level_to - self.sub_level) * b.smooth;
                    self.sub_pan += (pan_to - self.sub_pan) * b.smooth;
                    if (level_to - self.sub_level).abs() < 1e-6 {
                        self.sub_level = level_to;
                    }
                    if (pan_to - self.sub_pan).abs() < 1e-6 {
                        self.sub_pan = pan_to;
                    }
                    let sub_on = self.sub_level > 0.0 || level_to > 0.0;
                    let sub_wide = sub_on && (self.sub_pan != 0.0 || pan_to != 0.0);
                    let stereo = b.stereo || sub_wide;
                    if stereo && !self.was_stereo && !b.stereo {
                        // El pan del sub abre la voz: hasta aquí la derecha era la
                        // izquierda, así que su estado es el mismo.
                        self.down[1] = self.down[0];
                        self.filter[1] = self.filter[0];
                    }
                    self.was_stereo = stereo;
                    let (sub_l, sub_r) = if sub_wide {
                        let (gl, gr) = constant_power(self.sub_pan);
                        (self.sub_level * gl, self.sub_level * gr)
                    } else {
                        (self.sub_level, self.sub_level)
                    };
                    let div = sub.div.unwrap_or(patch.sub_div).max(1) as u32;
                    let sub_wave = sub.wave.unwrap_or(patch.sub_wave);
                    // Osciladores, al doble de la frecuencia de muestreo y
                    // diezmados: lo que queda del aliasing de PolyBLEP se pliega
                    // por encima de lo audible y el diezmado lo quita.
                    let pw = p(PW) as f64;
                    let noise = p(NOISE);
                    let dt = 0.5 * self.dt;
                    let center = n / 2;
                    for _ in 0..2 {
                        let (mut l, mut r) = (0.0f32, 0.0f32);
                        // El sub sale de la fase del central y de cuántas veces ha
                        // vuelto a empezar (módulo el divisor): siempre en fase con
                        // él, también con glide.
                        let at = self.osc[center].phase;
                        if self.sub_count >= div {
                            self.sub_count %= div;
                        }
                        let sub_dt = dt * b.ratio[center] / div as f64;
                        for u in 0..n {
                            let y = self.osc[u].next(patch.wave, dt * b.ratio[u], pw);
                            if b.stereo {
                                l += y * (1.0 - b.pan[u]).min(1.0);
                                r += y * (1.0 + b.pan[u]).min(1.0);
                            } else {
                                l += y;
                            }
                        }
                        let sub_y = if sub_on {
                            let phase = (self.sub_count as f64 + at) / div as f64;
                            osc::shape(sub_wave, phase, sub_dt, 0.5) as f32
                        } else {
                            0.0
                        };
                        if self.osc[center].phase < at {
                            self.sub_count = (self.sub_count + 1) % div;
                        }
                        let mut extra = 0.0;
                        if noise > 0.0 {
                            extra += noise * self.noise.next();
                        }
                        // Sin sub, exactamente la suma de siempre.
                        let r = if b.stereo { r } else { l };
                        let (mut l, mut r) = (l * norm + extra, r * norm + extra);
                        if sub_on {
                            l += sub_y * sub_l;
                            r += sub_y * sub_r;
                        }
                        self.down[0].push(l);
                        if stereo {
                            self.down[1].push(r);
                        }
                    }
                    let l = self.down[0].output();
                    let r = if stereo { self.down[1].output() } else { l };
                    (l, r, stereo, sub_on, *sub)
                }
            };
            let sub_drive = if sub_on {
                sub.drive.unwrap_or_else(|| p(SUB_DRIVE))
            } else {
                0.0
            };
            if sub_drive > 0.0 {
                let gain = 1.0 + 15.0 * sub_drive * sub_drive;
                let post = 1.0 / soft(gain);
                l = soft(gain * l) * post;
                r = soft(gain * r) * post;
            }
            let pre = p(PRE_DRIVE);
            if pre > 0.0 {
                let gain = 1.0 + 15.0 * pre * pre;
                let post = 1.0 / soft(gain);
                l = soft(gain * l) * post;
                r = soft(gain * r) * post;
            }
            // Filtro, con su envolvente y el accent (y su memoria).
            let fenv = self.fenv.next(fenv_rates);
            let accent = self.accent * p(ACCENT);
            self.sweep += (fenv * accent - self.sweep) * b.sweep;
            let octaves = log_from
                + (log_to - log_from) * t
                + p(ENV_AMOUNT) * fenv
                + patch.accent.octaves * self.sweep;
            let g = warp(octaves.exp2(), b.sample_rate);
            let res = p(RESONANCE);
            let level = self.amp.next(&b.amp) * self.gain * (1.0 + patch.accent.gain * accent);
            let mut yl = self.filter[0].next(l, g, res, patch.filter);
            let mut yr = if stereo {
                self.filter[1].next(r, g, res, patch.filter)
            } else {
                yl
            };
            yl *= level;
            yr *= level;
            let drive = p(DRIVE);
            if drive > 0.0 {
                // Saturación con la ganancia de entrada en función de
                // `drive` y la salida normalizada (el pico sigue en 1).
                let pre = 1.0 + 15.0 * drive * drive;
                let post = 1.0 / soft(pre);
                yl = soft(pre * yl) * post;
                yr = soft(pre * yr) * post;
            }
            yl *= patch.level;
            yr *= patch.level;
            if let Some(left) = &mut self.killed {
                let k = *left as f32 / b.fade as f32;
                yl *= k;
                yr *= k;
                *left = left.saturating_sub(1);
                if *left == 0 {
                    self.active = false;
                }
            }
            frame[0] += yl;
            frame[1] += yr;
            if self.amp.is_idle() {
                self.active = false;
            }
        }
    }
}

/// Un sintetizador en una pista: sus voces y el valor de sus parámetros.
pub struct Synth {
    voices: [Voice; POOL],
    /// Notas tocadas (el orden de llegada; también siembra las fases).
    born: u64,
    sample_rate: f32,
    /// Valor (suavizado) de cada parámetro al final del último bloque.
    values: [f32; PARAM_COUNT],
    /// `values` vale (hay voces sonando desde entonces); si no, el
    /// siguiente bloque empieza ya en el destino.
    primed: bool,
    /// Altura de la última nota (para el slide de un polifónico).
    last_pitch: Option<f32>,
}

/// Dónde está la canción al empezar un trozo (lo que siguen las curvas).
#[derive(Debug, Clone, Copy)]
pub struct SongClock {
    pub steps: f64,
    pub seconds: f64,
    pub frames_per_step: f64,
}

impl Synth {
    pub fn new(sample_rate: u32) -> Synth {
        osc::prepare();
        Synth {
            voices: [Voice::default(); POOL],
            born: 0,
            sample_rate: sample_rate as f32,
            values: PARAMS.map(|p| p.default),
            primed: false,
            last_pitch: None,
        }
    }

    /// Alguna voz suena (también en su release o su fundido).
    pub fn is_active(&self) -> bool {
        self.voices.iter().any(Voice::sounding)
    }

    /// Voces que cuentan (suenan y no están robadas).
    pub fn voices(&self) -> usize {
        self.voices.iter().filter(|v| v.is_voice()).count()
    }

    fn fade(&self) -> u32 {
        ((KILL_SECONDS * self.sample_rate) as u32).max(1)
    }

    /// Toca una nota con `patch` (ver [`Patch::mono`] y
    /// [`NoteEvent::slide`]). Devuelve el número de la nota (orden de
    /// llegada).
    pub fn note_on(&mut self, patch: &Patch, ev: NoteEvent) -> u64 {
        self.born += 1;
        let born = self.born;
        let sr = self.sample_rate;
        let fade = self.fade();
        // El slide de un polifónico sale de la altura de la nota anterior;
        // en un mono sin nada sonando, la nota empieza en la suya.
        let from = self.last_pitch.filter(|_| ev.slide && !patch.mono);
        self.last_pitch = Some(ev.pitch);
        if patch.mono {
            let current = (0..POOL).find(|&v| self.voices[v].is_voice());
            if let Some(v) = current {
                let voice = &mut self.voices[v];
                if ev.slide && !voice.amp.is_idle() {
                    voice.tie(&ev, born);
                } else {
                    // La misma voz vuelve a sonar: sin glide, las
                    // envolventes desde su nivel.
                    voice.play(&ev, born, None, sr);
                }
                return born;
            }
        } else {
            // Sin voz libre, se roba la más antigua (fundido aparte).
            while self.voices() >= patch.voices as usize {
                let oldest = (0..POOL)
                    .filter(|&v| self.voices[v].is_voice())
                    .min_by_key(|&v| self.voices[v].born)
                    .expect("hay voces sonando");
                self.voices[oldest].kill(fade);
            }
        }
        // Una ranura libre o, si no, la robada a la que menos fundido le
        // queda.
        let slot = (0..POOL)
            .find(|&v| !self.voices[v].active)
            .unwrap_or_else(|| {
                (0..POOL)
                    .filter(|&v| !self.voices[v].is_voice())
                    .min_by_key(|&v| self.voices[v].killed.unwrap_or(u32::MAX))
                    .unwrap_or(0)
            });
        let voice = &mut self.voices[slot];
        voice.start(patch, born, sr);
        voice.play(&ev, born, from, sr);
        born
    }

    /// Suelta las notas de la tecla `key` (empiezan su release).
    pub fn note_off(&mut self, patch: &Patch, key: u8) {
        let free_filter = patch.filter_env.sustain == 0.0;
        for voice in self.voices.iter_mut().filter(|v| v.active) {
            if voice.key == Some(key) {
                voice.release(free_filter);
                voice.key = None;
            }
        }
    }

    /// Corta todas las voces con el fundido mínimo.
    pub fn kill(&mut self) {
        let fade = self.fade();
        for voice in self.voices.iter_mut().filter(|v| v.active) {
            voice.kill(fade);
        }
    }

    /// Suma en `out` (estéreo entrelazado) lo que suena, con los
    /// parámetros de `patch` y sus curvas (`curves`, evaluadas en `clock`
    /// y suavizadas). Los trozos de [`crate::engine::MOD_BLOCK`] frames
    /// recalculan los parámetros.
    pub fn mix_into(
        &mut self,
        out: &mut [f32],
        patch: &Patch,
        curves: &SynthCurves,
        clock: &SongClock,
    ) {
        let block = crate::engine::MOD_BLOCK;
        let sr = self.sample_rate;
        if !self.is_active() {
            self.primed = false;
            return;
        }
        let keep = (-(block as f32) / (crate::engine::GAIN_SMOOTHING_SECONDS * sr)).exp();
        for (k, sub) in out.chunks_mut(2 * block).enumerate() {
            let frames = (k * block + sub.len() / 2) as f64;
            let steps = clock.steps + frames / clock.frames_per_step;
            let seconds = clock.seconds + frames / sr as f64;
            let from = self.values;
            let mut to = [0.0; PARAM_COUNT];
            for (i, p) in PARAMS.iter().enumerate() {
                let goal = curves[i].map_or(patch.params[i], |c| c.value(steps, seconds, p.log));
                to[i] = if self.primed {
                    mix(goal, from[i], keep, p.log)
                } else {
                    goal
                };
            }
            let from = if self.primed { from } else { to };
            self.values = to;
            self.primed = true;
            let n = patch.unison as usize;
            let mut ratio = [1.0; MAX_UNISON];
            let mut pan = [0.0; MAX_UNISON];
            for u in 0..n {
                let x = if n > 1 {
                    2.0 * u as f32 / (n - 1) as f32 - 1.0
                } else {
                    0.0
                };
                ratio[u] = ((to[DETUNE] * x) as f64 / 1200.0).exp2();
                pan[u] = to[SPREAD] * x;
            }
            let decay = to[FILTER_DECAY];
            let b = Block {
                patch,
                from,
                to,
                amp: Rates::new(&patch.amp, patch.amp.decay_ms, sr, MIN_AMP_MS),
                fenv: Rates::new(&patch.filter_env, decay, sr, MIN_FILTER_MS),
                fenv_accent: Rates::new(
                    &patch.filter_env,
                    patch.accent.decay_ms.unwrap_or(decay),
                    sr,
                    MIN_FILTER_MS,
                ),
                glide: follow(to[GLIDE], sr),
                sweep: follow(patch.accent.sweep_ms, sr),
                ratio,
                pan,
                stereo: n > 1 && (to[SPREAD] > 0.0 || from[SPREAD] > 0.0),
                smooth: 1.0 - (-1.0 / (crate::engine::GAIN_SMOOTHING_SECONDS * sr)).exp(),
                sample_rate: sr,
                fade: self.fade(),
            };
            for voice in self.voices.iter_mut().filter(|v| v.active) {
                voice.render(sub, &b);
            }
        }
    }

    /// Algún estado de algún filtro no es finito (para los tests).
    pub fn is_finite(&self) -> bool {
        self.voices
            .iter()
            .all(|v| v.filter.iter().all(Filter::is_finite))
    }
}

/// Renderiza notas sin el motor: `notes` son (frame de inicio, nota),
/// con el `gate` de cada nota en frames. Para tests y medidas.
pub fn render(
    patch: &Patch,
    curves: &SynthCurves,
    notes: &[(usize, NoteEvent)],
    frames: usize,
    sample_rate: u32,
) -> Vec<f32> {
    let mut synth = Synth::new(sample_rate);
    let mut out = vec![0.0f32; 2 * frames];
    let chunk = crate::engine::CHUNK;
    let mut at = 0;
    let mut next = 0;
    while at < frames {
        while next < notes.len() && notes[next].0 <= at {
            synth.note_on(patch, notes[next].1);
            next += 1;
        }
        let until = notes
            .get(next)
            .map_or(frames, |n| n.0)
            .min(at + chunk)
            .min(frames);
        // 120 BPM: 8 pasos por segundo.
        let frames_per_step = sample_rate as f64 / 8.0;
        let clock = SongClock {
            steps: at as f64 / frames_per_step,
            seconds: at as f64 / sample_rate as f64,
            frames_per_step,
        };
        synth.mix_into(&mut out[2 * at..2 * until], patch, curves, &clock);
        at = until;
    }
    out
}
