//! Scripting en Lua para el sampler.
//!
//! El script no toca el audio: al evaluarse construye un modelo de estado
//! ([`Song`]: tempo y pistas con sus patrones) y [`ScriptHost::load`] lo
//! envía al motor como un lote atómico ([`engine::Batch`]) por la cola sin
//! bloqueos. Los cambios entran al empezar el siguiente compás. Si el script
//! falla (sintaxis, error en tiempo de ejecución, notación inválida, bucle
//! infinito...), no se envía nada y sigue sonando lo anterior.
//!
//! `every` y `chance` se expanden al evaluar a un ciclo de compases (ver
//! [`TrackDef::bars`]); el motor toca la variante del compás actual.
//!
//! API (ver CLAUDE.md):
//!
//! ```lua
//! bpm(170)
//! local amen = sample("amen.wav"):slice(8)
//! track("drums", amen)
//!   :steps("1 . . 1  . . 3 .  1 . 5 .  . 3 . 6")
//!   :fx("filter", { type = "lowpass", cutoff = 3000 })
//!   :every(4, function(t) t:reverse() end)
//!   :chance(0.2, "retrig", 4)
//! track("kick", amen:slice_at(1)):steps("x . . . x . . . x . . . x . . ."):gain(0.9)
//! ```

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::fmt;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime};

use engine::notation::{check_label, parse_pattern_with, split_label};
use engine::{
    Batch, Buses, Controller, Division, EQ_BANDS, EqBand, EqKind, FilterKind, Fx, MAX_CYCLE_BARS,
    MAX_FX, MAX_GAIN, MAX_RETRIG, MAX_SEMITONES, MAX_STEPS, MAX_TRACKS, Pattern, STEPS, Sample,
    SendError, SliceFx, SliceShape, Slices, Step, valid_bpm,
};
pub mod api;

/// `n` compases como texto: «1 compás», «4 compases».
pub fn bars_text(n: u64) -> String {
    if n == 1 {
        "1 compás".to_string()
    } else {
        format!("{n} compases")
    }
}

#[cfg(test)]
#[test]
fn one_bar_is_singular() {
    assert_eq!(bars_text(1), "1 compás");
    assert_eq!(bars_text(0), "0 compases");
    assert_eq!(bars_text(24), "24 compases");
}

pub mod blocks;
mod freeze;
pub mod learn;
pub use freeze::FreezeRender;
use freeze::apply_frozen;
pub mod masking;
pub mod patterns;
pub mod project;
pub mod step_fx;
mod watch;
mod written;

use patterns::Rng;
pub use project::{Cut, EditKind, Project, SliceEdit, SliceLabel, SlicePosition};
pub use watch::{ScriptWatcher, WatchEvent};
pub use written::TokenSpan;

use mlua::{
    AnyUserData, Function, HookTriggers, Lua, LuaOptions, StdLib, Table, UserData, UserDataMethods,
    Value, VmState,
};

/// Tiempo máximo de evaluación de un script por defecto.
pub const DEFAULT_TIME_LIMIT: Duration = Duration::from_secs(2);
/// Cada cuántas instrucciones de Lua se comprueba el tiempo.
const HOOK_INSTRUCTIONS: u32 = 10_000;
/// Se mezcla con la semilla de la carga para el generador de `mutate` y
/// `shuffle` sin semilla propia.
const PATTERN_SEED: u64 = 0x5eed_0f9a_77e2_a5c3;

/// Se mezcla con la semilla de la carga para humanize sin semilla propia.
const HUMANIZE_SEED: u64 = 0x4b0d_e1a7_c0de_7a11;
/// Humanize máximo, en ms.
const MAX_HUMANIZE_MS: f32 = 100.0;

/// Swing recto (sin retraso).
const STRAIGHT_SWING: f32 = 50.0;
/// Swing máximo: el contratiempo, a medio paso.
const MAX_SWING: f32 = 75.0;

/// Funciones que son de toda la canción: no van dentro de `section()`.
const GLOBAL_ONLY: &[&str] = &[
    "bpm",
    "bpm_ramp",
    "meter",
    "metronome",
    "bus",
    "group",
    "macro_init",
    "section",
    "arrange",
    "variant",
    "priority",
    "key",
    "transpose",
    "frozen",
];

/// Compases que se tiran los dados de `chance` antes de repetirse.
pub const CHANCE_CYCLE_BARS: u64 = 16;

/// Compases como mucho de la canción entera ([`Song::length_bars`]): con
/// longitudes que no encajan entre sí (7 contra 11 pasos), el ciclo común
/// puede ser enorme.
pub const MAX_SONG_BARS: u64 = 256;

/// Pasos de una vuelta del patrón de una pista (16 = un compás).
pub type Bar = Vec<Option<Step>>;

/// Modelo de estado que produce un script. Las vistas leerán esto.
#[derive(Clone)]
pub struct Song {
    /// Tempo pedido con `bpm()`; `None` mantiene el que haya.
    pub bpm: Option<f64>,
    /// Pistas en el orden en que se definieron.
    pub tracks: Vec<TrackDef>,
    /// Metrónomo pedido con `metronome()`; `None` deja el que haya. No entra
    /// en el render salvo que se pida ([`Song::render_with_metronome`]).
    pub metronome: Option<bool>,
    /// Parámetros de los buses de envío (`bus()`; si no, por defecto).
    pub buses: Buses,
    /// Rampa de tempo (`bpm_ramp()`): manda sobre `bpm`.
    pub tempo_ramp: Option<engine::TempoRamp>,
    /// Compás (`meter()`); `None`, 4/4.
    pub meter: Option<engine::Meter>,
    /// Avisos de la carga que no la impiden (por ejemplo, etiquetas del
    /// proyecto que no se aplican porque el script corta el sample en otro
    /// número de cortes).
    pub warnings: Vec<String>,
    /// Secciones (`section()`), en el orden en que se definieron; vacío
    /// sin secciones.
    pub sections: Vec<SectionDef>,
    /// Línea de tiempo (`arrange()`): (sección, compases); vacía sin
    /// `arrange`. Con ella, el motor lleva la canción entera.
    pub timeline: Vec<(usize, u32)>,
    /// Con `variant()`: qué variante es esta (`'A'` o `'B'`); `None` sin
    /// variantes.
    pub variant: Option<char>,
    /// La otra variante (la B, en la A que devuelve la evaluación; el
    /// anfitrión guarda las dos y alterna).
    pub alternate: Option<Box<Song>>,
    /// Prioridad entre pistas para los choques de frecuencia
    /// (`priority()`): la primera manda. Vacía, manda la más fuerte.
    pub priority: Vec<String>,
    /// Grupos (`group()`), en el orden en que se definieron (el índice es
    /// el del grupo en el motor).
    pub groups: Vec<GroupDef>,
    /// Macros (`macro()`), en el orden en que aparecen (el índice es el de
    /// la macro en el motor), con el valor con el que suenan.
    pub macros: Vec<MacroDef>,
    /// Tonalidad de la canción (`key()`); sin ella, todo como siempre.
    pub key: Option<SongKey>,
    /// Semitonos de `transpose()` (0 sin él).
    pub transpose: i32,
}

/// La tonalidad de la canción (`key("A", "minor")`): la usan `scale()`
/// sin tónica, los grados de `melody()` y los pads de los instrumentos.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SongKey {
    /// Tónica, en semitonos sobre do (de 0 a 11).
    pub tonic: i32,
    /// El modo: una escala de `scale()`.
    pub scale: &'static api::ScaleDef,
}

impl SongKey {
    /// Nombre de la tónica (`"A"`, `"F#"`).
    pub fn tonic_name(&self) -> &'static str {
        const NAMES: [&str; 12] = [
            "C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "A#", "B",
        ];
        NAMES[self.tonic.rem_euclid(12) as usize]
    }

    /// Como se escribe en `key()`: `key("A", "minor")`.
    pub fn call(&self) -> String {
        format!("key(\"{}\", \"{}\")", self.tonic_name(), self.scale.name)
    }

    /// Para los grados de `melody()` y los pads.
    pub fn degrees(&self) -> patterns::Key {
        patterns::Key {
            tonic: self.tonic,
            intervals: self.scale.intervals,
        }
    }

    /// La misma, subida `semitones` (con `transpose()`).
    pub fn transposed(&self, semitones: i32) -> SongKey {
        SongKey {
            tonic: (self.tonic + semitones).rem_euclid(12),
            ..*self
        }
    }
}

/// Un grupo (`group(nombre, pistas)`): sus pistas se suman y pasan juntas
/// por sus efectos, su volumen, su pan y sus envíos.
#[derive(Clone, Debug, PartialEq)]
pub struct GroupDef {
    pub name: String,
    /// Las pistas que van a él, por su nombre.
    pub tracks: Vec<String>,
    pub gain: f32,
    pub pan: f32,
    pub sends: [f32; engine::bus::BUSES],
    /// Volumen, pan y envíos con curva (como los de [`TrackDef`]).
    pub mix_curves: engine::MixCurves,
    pub fx: Vec<Fx>,
    pub automation: Vec<engine::Automation>,
    /// Pistas que escucha por sidechain cada EQ dinámico de la cadena.
    pub sidechains: Vec<(usize, String)>,
}

impl GroupDef {
    fn new(name: String, tracks: Vec<String>) -> Self {
        GroupDef {
            name,
            tracks,
            gain: 1.0,
            pan: 0.0,
            sends: [0.0; engine::bus::BUSES],
            mix_curves: [None; engine::MIX_PARAMS],
            fx: Vec::new(),
            automation: Vec::new(),
            sidechains: Vec::new(),
        }
    }
}

/// Una macro (`macro(nombre, desde, hasta)`): un valor de 0 a 1 que se
/// mueve en vivo y que siguen los parámetros que la usan.
#[derive(Clone, Debug, PartialEq)]
pub struct MacroDef {
    pub name: String,
    /// El valor con el que suena: el de `macro_init()` (0 sin él) o, tras
    /// recargar, el que se le dio en vivo (ver [`ScriptHost::set_macro`]).
    pub value: f32,
}

/// Una sección: su nombre y sus pistas (las de fuera de las secciones con
/// lo que la sección les cambia, y las suyas).
#[derive(Clone)]
pub struct SectionDef {
    pub name: String,
    pub tracks: Vec<TrackDef>,
}

#[derive(Clone)]
pub struct TrackDef {
    pub name: String,
    pub sample: Arc<Sample>,
    /// Ruta del WAV tal como se resolvió.
    pub path: PathBuf,
    pub slices: Slices,
    /// Pasos de una vuelta del patrón (`:length()`, 16 por defecto).
    pub length: usize,
    /// Patrón base (`:steps()`), de `length` pasos.
    pub steps: Bar,
    /// Ciclo de vueltas con `every` y `chance` ya aplicados: en la vuelta
    /// `v` de la canción suena `bars[v % bars.len()]` (con 16 pasos, una
    /// vuelta es un compás; ver [`TrackDef::lap`]).
    pub bars: Vec<Bar>,
    /// Volumen de la pista (`:gain()`), lineal.
    pub gain: f32,
    /// Efectos de la pista (`:fx()`), en orden.
    pub fx: Vec<Fx>,
    /// Etiqueta de cada corte de la pista (`labels[i]`, corte `i` desde 0).
    pub labels: Vec<Option<String>>,
    /// Si cada corte tiene el inicio ajustado a mano (proyecto o `:move()`).
    pub adjusted: Vec<bool>,
    /// Swing (`:swing()`), en porcentaje estilo MPC: 50 es recto.
    pub swing: f32,
    /// Humanize (`:humanize()`).
    pub humanize: Option<Humanize>,
    /// Parámetros de efecto con `ramp()` o `lfo()`.
    pub automation: Vec<engine::Automation>,
    /// Envíos a los buses (`:send()`): `[delay, reverb]`.
    pub sends: [f32; engine::bus::BUSES],
    /// Pan de la pista (`:pan()`), de -1 a 1.
    pub pan: f32,
    /// Pitch de la pista (`:pitch()`), en semitonos que se suman a los de
    /// cada golpe, y su curva (por golpe), si la tiene.
    pub pitch: f32,
    pub pitch_curve: Option<engine::Curve>,
    /// Curva del formante (`:formant(ramp(...))`), por golpe.
    pub formant_curve: Option<engine::Curve>,
    /// Volumen, pan y envíos con `ramp()` o `lfo()` (índices
    /// `engine::MIX_GAIN`, `MIX_PAN` y `MIX_SEND + bus`).
    pub mix_curves: engine::MixCurves,
    /// Forma de cada corte (`cortes:set()`), uno por corte.
    pub shapes: Vec<SliceShape>,
    /// Modo gate (`:mode("gate")`): cada golpe dura lo que su paso.
    pub gate: bool,
    /// Grupo de choke (`:choke(n)`).
    pub choke: Option<u8>,
    /// Notas MIDI de sus golpes (`:midi(canal, nota_base)`).
    pub midi: Option<(u8, u8)>,
    /// Silenciada (`:mute()`).
    pub mute: bool,
    /// En solo (`:solo()`).
    pub solo: bool,
    /// De qué sample y qué `slice(n)` salen los cortes (para etiquetarlos
    /// desde la interfaz y guardarlo en el proyecto).
    pub origin: SliceOrigin,
    /// Time-stretch al tempo de la canción (`sample:stretch()`), si lo hay.
    /// Entonces `sample` y `slices` ya están estirados.
    pub stretch: Option<Stretch>,
    /// Instrumento (`sample:instrument()`): sus golpes son notas, con
    /// polifonía, ADSR y bucle sostenido. Tiene un solo corte (el sample).
    pub instrument: Option<InstrumentDef>,
    /// Granos (`:grains()`, `:freeze()`): cada golpe abre una nube sobre su
    /// corte.
    pub granular: Option<engine::Granular>,
    /// Formantes de los pasos con `%` (`:formant(n)`): `Some(0)` donde
    /// estaban, `Some(n)` desplazados; `None`, se mueven con el pitch.
    pub formant: Option<f32>,
    /// Pistas que escucha por sidechain cada EQ dinámico de la cadena:
    /// (índice en `fx`, nombre de la pista).
    pub sidechains: Vec<(usize, String)>,
    /// Efectos de cada corte (`cortes:fx()`), uno por corte (o ninguno).
    pub slice_fx: Vec<SliceFxDef>,
    /// Etiqueta de esta definición de pista, única entre cargas (de 1 a
    /// [`engine::MAX_TAG`]; 0, sin etiqueta). Va al motor con su patrón y
    /// vuelve con el paso que suena ([`engine::TrackStatus::cursor`]): así
    /// se sabe de qué carga es lo que suena.
    pub tag: u32,
    /// Dónde está escrito el token de cada paso en el texto cargado, por
    /// vuelta del ciclo (como `bars`); `None` donde no se sabe (un patrón
    /// generado con `euclid`, `rotate`...). Ver [`TrackDef::sounding_span`].
    pub spans: Vec<Vec<Option<TokenSpan>>>,
    /// Congelada (`frozen()`): suena su WAV en vez de lo que dice el script.
    pub frozen: Option<FrozenDef>,
}

/// Una pista congelada (`frozen(pista, wav, huella)`): toca el WAV de su
/// salida tras sus efectos (y antes de su volumen), una vez por vuelta.
#[derive(Debug, Clone, PartialEq)]
pub struct FrozenDef {
    /// El WAV, como se escribió en `frozen()`.
    pub wav: String,
    /// Compases que dura (una vuelta).
    pub bars: u64,
    /// La pista ha cambiado desde que se congeló (la huella no coincide).
    pub stale: bool,
}

/// Efectos de un corte (`cortes:fx(i, ...)`): su cadena, en orden, y los
/// parámetros que llevan `ramp()` o `lfo()` (`Automation::fx`: el índice
/// en `fx`).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct SliceFxDef {
    pub fx: Vec<Fx>,
    pub automation: Vec<engine::Automation>,
}

/// `sample:instrument({ ... })`: el sample se toca como notas.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct InstrumentDef {
    /// Nota MIDI (C4 = 60) en la que suena el sample tal cual: la raíz de
    /// `melody`.
    pub root: i32,
    /// Bucle sostenido en ms del sample, si lo hay.
    pub loop_ms: Option<(f64, f64)>,
    /// Lo que recibe el motor (voces, ADSR, bucle en frames y fundido).
    pub engine: engine::Instrument,
    /// Un sintetizador (`synth(tipo, {...})`): no toca el sample (es un
    /// frame de silencio) sino su banco de voces.
    pub synth: Option<SynthDef>,
}

/// `synth(tipo, { ... })`: un instrumento sintetizado.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SynthDef {
    /// El tipo (`"acid"`, del registro [`api::SYNTHS`]).
    pub kind: &'static str,
    /// Lo que recibe el motor (con la raíz del instrumento).
    pub patch: engine::synth::Patch,
    /// Los parámetros continuos con `ramp()` o `lfo()`, en las unidades
    /// del motor.
    pub curves: engine::synth::SynthCurves,
    /// La semilla, si se dio (si no, sale del nombre de la pista).
    pub seed: Option<u64>,
}

/// Qué toca una pista: cortes, un instrumento de sample o un
/// sintetizador (mono o no; con capas, el subharm, sin bloque sub).
#[derive(Debug, Clone, Copy, PartialEq)]
enum Plays {
    Slices,
    Instrument,
    Synth { mono: bool, layers: bool },
}

impl Plays {
    fn of(def: &TrackDef) -> Plays {
        match def.instrument {
            None => Plays::Slices,
            Some(InstrumentDef { synth: None, .. }) => Plays::Instrument,
            Some(InstrumentDef { synth: Some(s), .. }) => Plays::Synth {
                mono: s.patch.mono,
                layers: s.patch.layers.is_some(),
            },
        }
    }
}

/// `sample:stretch(bpm_origen)`: el sample se estira del tempo de origen al
/// de la canción sin cambiar el tono.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Stretch {
    /// Tempo original del sample.
    pub source_bpm: f64,
    /// Duración nueva / original (`source_bpm / bpm de la canción`).
    pub ratio: f64,
}

impl TrackDef {
    /// Vuelta y paso (desde 0) que toca esta pista en la semicorchea
    /// `global` de la canción (contada desde que arrancó: `Status::global`).
    pub fn lap(&self, global: u64) -> (u64, usize) {
        let length = self.length.max(1) as u64;
        (global / length, (global % length) as usize)
    }

    /// El token que se resalta mientras suena el paso `index` del ciclo
    /// (`vuelta · length + paso`, el de [`engine::Cursor::index`]): el de
    /// ese paso si dispara algo; en un paso que alarga un golpe (`_`), el
    /// del golpe; en un silencio, nada.
    pub fn sounding_span(&self, index: usize) -> Option<TokenSpan> {
        let length = self.length.max(1);
        let (lap, step) = (index / length, index % length);
        let bar = self.bars.get(lap)?;
        let spans = self.spans.get(lap)?;
        let head = (0..=step)
            .rev()
            .find(|&k| bar.get(k).is_some_and(Option::is_some))?;
        let hold = bar[head].map_or(0, |s| s.hold as usize);
        (step - head <= hold).then(|| spans.get(head).copied().flatten())?
    }

    /// Pasos de la vuelta `lap` (la variante del ciclo que le toca).
    pub fn lap_steps(&self, lap: u64) -> &Bar {
        self.bars
            .get((lap % self.bars.len().max(1) as u64) as usize)
            .unwrap_or(&self.steps)
    }
}

/// `:humanize(ms, volumen [, semilla])`: cada golpe se retrasa al azar
/// entre 0 y `ms` y su volumen se multiplica por un factor entre
/// `1 - volume` y 1.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Humanize {
    pub ms: f32,
    pub volume: f32,
    /// Semilla propia; `None`: la de la carga con el nombre de la pista.
    pub seed: Option<u64>,
}

/// Origen de los cortes de una pista.
#[derive(Debug, Clone, PartialEq)]
pub struct SliceOrigin {
    /// El sample tal como se escribió en `sample("...")`.
    pub sample: String,
    /// La `n` de `slice(n)` (o de `slice_grid(n)`).
    pub count: usize,
    /// Cortado en rejilla (`slice_grid`).
    pub grid: bool,
    /// Corte del sample cortado así (desde 0, con las ediciones del
    /// proyecto) que es cada corte de la pista: todos en orden, o uno solo
    /// con `slice_at`. `None`: un corte añadido en el script (`:add()`), que
    /// no se puede etiquetar ni mover desde la interfaz.
    pub indices: Vec<Option<usize>>,
    /// `zero_cross` de `slice`/`slice_grid`: cada inicio va al cruce por
    /// cero más cercano, a no más de estos ms. `None`: tal cual.
    pub zero_cross: Option<f64>,
}

impl SliceOrigin {
    /// Cómo se cortó el sample (la clave de sus etiquetas y ajustes).
    pub fn cut(&self) -> Cut {
        Cut {
            count: self.count,
            grid: self.grid,
        }
    }
}

/// Error de un script, con archivo y línea cuando se conocen.
#[derive(Debug, Clone, PartialEq)]
pub struct ScriptError {
    pub message: String,
}

impl fmt::Display for ScriptError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for ScriptError {}

impl ScriptError {
    fn new(message: impl Into<String>) -> Self {
        ScriptError {
            message: message.into(),
        }
    }

    /// Mensaje legible de un error de mlua: la causa original, sin la pila.
    /// Si la causa no dice dónde ocurrió (p. ej. un argumento de tipo
    /// incorrecto), se toma la posición de la pila de llamadas.
    fn from_lua(error: &mlua::Error) -> Self {
        let mut error = error;
        let mut location = None;
        while let mlua::Error::CallbackError { cause, traceback } = error {
            location = location.or_else(|| script_location(traceback));
            error = cause;
        }
        let message = match error {
            mlua::Error::SyntaxError { message, .. } => return ScriptError::new(message.clone()),
            mlua::Error::RuntimeError(message) => return ScriptError::new(message.clone()),
            mlua::Error::BadArgument { pos, cause, .. } => {
                let detail = match cause.as_ref() {
                    mlua::Error::FromLuaConversionError { from, to, .. } => {
                        format!("se esperaba {to} y llegó {from}")
                    }
                    other => other.to_string(),
                };
                format!("argumento {pos} no válido: {detail}")
            }
            other => other.to_string(),
        };
        match location {
            Some(location) => ScriptError::new(format!("{location}: {message}")),
            None => ScriptError::new(message),
        }
    }
}

/// Primera posición del script (`archivo:línea`) en una traza de mlua,
/// saltando las funciones de Rust (`[C]`).
fn script_location(traceback: &str) -> Option<String> {
    traceback
        .lines()
        .map(str::trim)
        .filter(|line| !line.starts_with("[C]") && !line.starts_with("stack traceback"))
        .find_map(|line| {
            let (location, _) = line.split_once(": in ")?;
            Some(location.to_string())
        })
}

/// Evalúa scripts y aplica su resultado al motor. Vive en el hilo de control.
/// Un sample preparado al importar (`sample(nombre, { ... })`), que se
/// guarda entre cargas.
struct Imported {
    original: Arc<Sample>,
    options: engine::ImportOptions,
    result: Arc<Sample>,
}

/// Los preparados de la carga anterior y los que usa esta.
#[derive(Default)]
struct Imports {
    old: Vec<Imported>,
    used: Vec<Imported>,
}

impl Imports {
    /// El sample preparado con esas opciones (el de antes si ya estaba).
    fn get(&mut self, original: &Arc<Sample>, options: engine::ImportOptions) -> Arc<Sample> {
        let same = |e: &&Imported| Arc::ptr_eq(&e.original, original) && e.options == options;
        if let Some(e) = self.used.iter().find(same) {
            return e.result.clone();
        }
        let result = self.old.iter().find(same).map_or_else(
            || Arc::new(original.processed(&options)),
            |e| e.result.clone(),
        );
        self.used.push(Imported {
            original: original.clone(),
            options,
            result: result.clone(),
        });
        result
    }
}

/// Un sample estirado que se guarda entre cargas (estirar cuesta).
struct Stretched {
    original: Arc<Sample>,
    ratio: f64,
    bounds: Vec<usize>,
    result: Arc<Sample>,
}

pub struct ScriptHost {
    sample_rate: u32,
    time_limit: Duration,
    /// Samples ya cargados, por ruta: recargar el script no vuelve a
    /// decodificar el WAV (salvo que cambie) y conserva el mismo `Arc`.
    samples: HashMap<PathBuf, (Option<SystemTime>, Arc<Sample>)>,
    /// Pista del motor asignada a cada nombre. Se mantiene entre recargas.
    slots: [Option<String>; MAX_TRACKS],
    /// Último modelo aplicado con éxito (la variante que suena, si las
    /// hay) y la otra variante.
    song: Option<Song>,
    other: Option<Song>,
    /// Semilla de `chance`; `None` usa una distinta en cada evaluación.
    seed: Option<u64>,
    /// Carpeta de los samples (del proyecto); `None`: la del script.
    sample_dir: Option<PathBuf>,
    /// Tempo si el script no llama a `bpm()` (del proyecto).
    default_bpm: Option<f64>,
    /// Etiquetas de corte del proyecto (las de `:name()` mandan).
    labels: Vec<SliceLabel>,
    /// Ajustes manuales de cortes del proyecto.
    positions: Vec<SlicePosition>,
    /// Cortes añadidos y quitados en el proyecto.
    edits: Vec<SliceEdit>,
    /// Samples preparados al importar en la última carga.
    imported: Vec<Imported>,
    /// Samples estirados con `:stretch()` en la última carga (se reutilizan
    /// si no cambian el sample, la proporción ni los cortes).
    stretched: Vec<Stretched>,
    /// Última etiqueta dada a una pista ([`TrackDef::tag`]).
    last_tag: u32,
    /// Cada macro por su nombre: el valor que le daba el script en la
    /// última carga y el que tiene en vivo. Recargar conserva el de vivo
    /// salvo que el script cambie el suyo.
    macro_values: HashMap<String, (f32, f32)>,
    /// Mandos MIDI asignados (MIDI learn), por nombre de parámetro.
    learned: Vec<learn::Learned>,
}

impl ScriptHost {
    /// `sample_rate` es la del motor: los samples se cargan a esa frecuencia.
    pub fn new(sample_rate: u32) -> Self {
        ScriptHost {
            sample_rate,
            time_limit: DEFAULT_TIME_LIMIT,
            samples: HashMap::new(),
            slots: Default::default(),
            song: None,
            other: None,
            seed: None,
            sample_dir: None,
            default_bpm: None,
            labels: Vec::new(),
            positions: Vec::new(),
            edits: Vec::new(),
            imported: Vec::new(),
            stretched: Vec::new(),
            last_tag: 0,
            macro_values: HashMap::new(),
            learned: Vec::new(),
        }
    }

    /// Los mandos MIDI asignados (del proyecto y la configuración). Van al
    /// motor con la carga siguiente, o ya con [`ScriptHost::apply_learned`].
    pub fn set_learned(&mut self, learned: Vec<learn::Learned>) {
        self.learned = learned;
    }

    pub fn learned(&self) -> &[learn::Learned] {
        &self.learned
    }

    /// Manda al motor la tabla de mandos de lo cargado (entra al compás,
    /// como un lote). Devuelve los avisos de los que no encuentran su
    /// parámetro.
    pub fn apply_learned(&self, controller: &mut Controller) -> Result<Vec<String>, ScriptError> {
        let Some(song) = &self.song else {
            return Ok(Vec::new());
        };
        let (knobs, warnings) = learn::resolve(&self.learned, song, &self.slots);
        let mut batch = Batch::new();
        batch.set_knobs(knobs);
        controller
            .apply(batch)
            .map_err(|e| ScriptError::new(format!("no se pudo enviar al motor: {e}")))?;
        Ok(warnings)
    }

    /// El CC `cc` del canal `channel` (1-16) se ha movido a `value` (0 a
    /// 127): mueve los mandos que lo escuchan, ya. Devuelve cuántos.
    pub fn move_learned(
        &self,
        controller: &mut Controller,
        channel: u8,
        cc: u8,
        value: u8,
    ) -> Result<usize, ScriptError> {
        let mut moved = 0;
        for (i, l) in self.learned.iter().enumerate().take(engine::MAX_KNOBS) {
            if l.listens(channel, cc) {
                controller
                    .move_knob(i, value.min(127) as f32 / 127.0)
                    .map_err(|e| ScriptError::new(format!("no se pudo enviar al motor: {e}")))?;
                moved += 1;
            }
        }
        Ok(moved)
    }

    /// Ajustes manuales de cortes del proyecto: al cortar un sample con el
    /// mismo nombre y número de cortes, esos cortes empiezan donde dicen.
    pub fn set_positions(&mut self, positions: Vec<SlicePosition>) {
        self.positions = positions;
    }

    pub fn positions(&self) -> &[SlicePosition] {
        &self.positions
    }

    /// Cortes añadidos y quitados a mano del proyecto (se aplican en orden
    /// al cortar, antes que los ajustes y las etiquetas).
    pub fn set_edits(&mut self, edits: Vec<SliceEdit>) {
        self.edits = edits;
    }

    pub fn edits(&self) -> &[SliceEdit] {
        &self.edits
    }

    /// Parte el corte que contiene `ms` en dos (lo guarda en las ediciones
    /// del proyecto) y corre las etiquetas y los ajustes de los cortes
    /// siguientes. `new_slice` es el número (desde 1) que tendrá el corte
    /// nuevo. Que quepa se comprueba al cargar.
    pub fn split_slice(&mut self, sample: &str, cut: Cut, new_slice: usize, ms: f64) {
        self.edits.push(SliceEdit {
            sample: sample.to_string(),
            cut,
            kind: EditKind::Split(ms),
        });
        self.renumber(sample, cut, |slice| {
            Some(if slice >= new_slice { slice + 1 } else { slice })
        });
    }

    /// Quita el corte `slice` (desde 2: el anterior se alarga), lo guarda en
    /// las ediciones del proyecto y corre las etiquetas y los ajustes (los
    /// de ese corte se van).
    pub fn join_slice(&mut self, sample: &str, cut: Cut, slice: usize) {
        self.edits.push(SliceEdit {
            sample: sample.to_string(),
            cut,
            kind: EditKind::Join(slice),
        });
        self.renumber(sample, cut, |s| match s.cmp(&slice) {
            std::cmp::Ordering::Less => Some(s),
            std::cmp::Ordering::Equal => None,
            std::cmp::Ordering::Greater => Some(s - 1),
        });
    }

    /// Cambia el número de corte de las etiquetas y los ajustes de un
    /// sample cortado así (`None`: se quitan).
    fn renumber(&mut self, sample: &str, cut: Cut, map: impl Fn(usize) -> Option<usize>) {
        let ours = |s: &str, c: Cut| s == sample && c == cut;
        self.labels.retain_mut(|l| {
            if !ours(&l.sample, l.slices) {
                return true;
            }
            map(l.slice).map(|n| l.slice = n).is_some()
        });
        self.positions.retain_mut(|p| {
            if !ours(&p.sample, p.slices) {
                return true;
            }
            map(p.slice).map(|n| p.slice = n).is_some()
        });
    }

    /// Pone (o quita, con `None`) el ajuste de un corte en los del proyecto.
    /// Que quepa entre sus vecinos se comprueba al cargar (si no, se avisa
    /// y no se aplica).
    pub fn set_position(&mut self, sample: &str, count: Cut, slice: usize, ms: Option<f64>) {
        self.positions
            .retain(|p| !(p.sample == sample && p.slices == count && p.slice == slice));
        if let Some(ms) = ms {
            self.positions.push(SlicePosition {
                sample: sample.to_string(),
                slices: count,
                slice,
                ms,
            });
        }
    }

    /// Etiquetas de corte del proyecto: se aplican al cortar un sample con
    /// el mismo nombre y el mismo número de cortes. Las de `:name()` mandan.
    pub fn set_labels(&mut self, labels: Vec<SliceLabel>) {
        self.labels = labels;
    }

    pub fn labels(&self) -> &[SliceLabel] {
        &self.labels
    }

    /// Pone (o quita, con `None`) la etiqueta de un corte en las del
    /// proyecto. Error si el nombre no vale o ya es de otro corte del mismo
    /// sample cortado igual.
    pub fn set_label(
        &mut self,
        sample: &str,
        count: Cut,
        slice: usize,
        name: Option<&str>,
    ) -> Result<(), String> {
        if !(1..=64).contains(&slice) {
            return Err(format!("el corte {slice} no existe"));
        }
        let same = |l: &SliceLabel| l.sample == sample && l.slices == count;
        if let Some(name) = name {
            check_label(name)?;
            if let Some(other) = self
                .labels
                .iter()
                .find(|l| same(l) && l.name == name && l.slice != slice)
            {
                return Err(format!(
                    "la etiqueta \"{name}\" ya es del corte {}",
                    other.slice
                ));
            }
        }
        self.labels.retain(|l| !(same(l) && l.slice == slice));
        if let Some(name) = name {
            self.labels.push(SliceLabel {
                sample: sample.to_string(),
                slices: count,
                slice,
                name: name.to_string(),
            });
        }
        Ok(())
    }

    /// Carpeta donde `sample()` busca las rutas relativas. `None` (por
    /// defecto): la carpeta del script.
    pub fn set_sample_dir(&mut self, dir: Option<PathBuf>) {
        self.sample_dir = dir;
    }

    pub fn sample_dir(&self) -> Option<&Path> {
        self.sample_dir.as_deref()
    }

    /// Tempo que se usa si el script no llama a `bpm()`. Si lo llama, manda
    /// el script.
    pub fn set_default_bpm(&mut self, bpm: Option<f64>) {
        self.default_bpm = bpm;
    }

    pub fn default_bpm(&self) -> Option<f64> {
        self.default_bpm
    }

    /// Fija la semilla de `chance` (reproducible). `None`: una nueva cada vez.
    pub fn set_seed(&mut self, seed: Option<u64>) {
        self.seed = seed;
    }

    pub fn set_time_limit(&mut self, limit: Duration) {
        self.time_limit = limit;
    }

    /// Último modelo aplicado con éxito.
    pub fn song(&self) -> Option<&Song> {
        self.song.as_ref()
    }

    /// Pista del motor asignada a cada nombre.
    pub fn track_slot(&self, name: &str) -> Option<usize> {
        self.slots.iter().position(|s| s.as_deref() == Some(name))
    }

    /// Lee un archivo de script y lo carga (ver [`ScriptHost::load`]). Las
    /// rutas de `sample()` son relativas a la carpeta del script.
    pub fn load_file(
        &mut self,
        controller: &mut Controller,
        path: &Path,
    ) -> Result<&Song, ScriptError> {
        let source = std::fs::read_to_string(path)
            .map_err(|e| ScriptError::new(format!("{}: {e}", path.display())))?;
        let name = path.file_name().map_or_else(
            || path.display().to_string(),
            |n| n.to_string_lossy().into(),
        );
        let base = path.parent().unwrap_or(Path::new("."));
        self.load(controller, &source, &name, base)
    }

    /// Evalúa el script y, si todo va bien, envía el resultado al motor como
    /// un lote que entra en el siguiente compás. Si falla, no se envía nada y
    /// sigue sonando lo anterior.
    pub fn load(
        &mut self,
        controller: &mut Controller,
        source: &str,
        name: &str,
        base_dir: &Path,
    ) -> Result<&Song, ScriptError> {
        let mut song = self.eval(source, name, base_dir)?;
        // Con variantes, sigue sonando la que sonaba (la A al principio).
        let mut other = song.alternate.take().map(|b| *b);
        let wanted = self.song.as_ref().and_then(|s| s.variant).unwrap_or('A');
        if wanted == 'B'
            && let Some(b) = other.take()
        {
            other = Some(std::mem::replace(&mut song, b));
        }
        // Recargar a mitad de canción: el arreglo nuevo entra en el compás
        // siguiente; si la sección que tocaba entonces ya no está, el motor
        // vuelve al principio de la línea de tiempo. Se avisa.
        if let (Some(old), Some(place)) = (&self.song, controller.section())
            && let Some(current) = old.section_after(place)
            && !song.timeline.is_empty()
            && !song.sections.iter().any(|s| s.name == current)
        {
            song.warnings.push(format!(
                "la sección \"{current}\" ya no existe: la canción vuelve al principio de la línea de tiempo"
            ));
        }
        // Las pistas de las dos variantes tienen su ranura: alternar no
        // mueve ninguna.
        let mut names = song.track_names();
        for n in other.iter().flat_map(Song::track_names) {
            if !names.contains(&n) {
                names.push(n);
            }
        }
        let slots = self.assign_slots(&names);
        // Las macros siguen donde estaban en vivo (si el script no cambia su
        // valor inicial).
        let mut values = self.macro_values.clone();
        for def in &mut song.macros {
            let script = def.value;
            if let Some(&(before, live)) = values.get(&def.name)
                && before == script
            {
                def.value = live;
            }
            values.insert(def.name.clone(), (script, def.value));
        }
        if let Some(other) = &mut other {
            other.macros = song.macros.clone();
        }
        let mut batch = song_batch(&song, &slots, self.song.as_ref())?;
        // Los mandos MIDI, traducidos a esta canción, en el mismo lote.
        if !self.learned.is_empty() {
            let (knobs, warnings) = learn::resolve(&self.learned, &song, &slots);
            batch.set_knobs(knobs);
            for warning in warnings {
                if !song.warnings.contains(&warning) {
                    song.warnings.push(warning);
                }
            }
        }
        controller
            .apply(batch)
            .map_err(|e| ScriptError::new(format!("no se pudo enviar al motor: {e}")))?;

        self.slots = slots;
        self.song = Some(song);
        self.other = other;
        self.macro_values = values;
        Ok(self.song.as_ref().unwrap())
    }

    /// Las macros de lo que suena, con su valor.
    pub fn macros(&self) -> &[MacroDef] {
        self.song.as_ref().map_or(&[], |s| &s.macros)
    }

    /// Mueve la macro `index` a `value` (de 0 a 1) en vivo: el motor la
    /// sigue ya, y recargar el script la deja donde está (salvo que el
    /// script cambie su `macro_init`).
    pub fn set_macro(
        &mut self,
        controller: &mut Controller,
        index: usize,
        value: f32,
    ) -> Result<(), ScriptError> {
        let Some(name) = self.macros().get(index).map(|m| m.name.clone()) else {
            return Err(ScriptError::new(format!("no hay macro {}", index + 1)));
        };
        controller
            .set_macro(index, value)
            .map_err(|e| ScriptError::new(format!("no se pudo enviar al motor: {e}")))?;
        for song in self.song.iter_mut().chain(self.other.iter_mut()) {
            if let Some(def) = song.macros.get_mut(index) {
                def.value = value;
            }
        }
        if let Some((_, live)) = self.macro_values.get_mut(&name) {
            *live = value;
        }
        Ok(())
    }

    /// La variante que suena (`'A'` o `'B'`); `None` sin variantes.
    pub fn variant(&self) -> Option<char> {
        self.song.as_ref().and_then(|s| s.variant)
    }

    /// Pasa a la variante `letter`: manda esa versión de la canción como un
    /// lote (entra entera al empezar el compás siguiente; con secciones,
    /// sigue por la misma sección y el mismo compás). Sin variantes, error.
    pub fn set_variant(
        &mut self,
        controller: &mut Controller,
        letter: char,
    ) -> Result<&Song, ScriptError> {
        if self.variant().is_none() {
            return Err(ScriptError::new(
                "no hay variantes: defínelas con variant(\"sección o pista\", \"B\", function() ... end)",
            ));
        }
        if self.variant() != Some(letter) {
            let Some(other) = self.other.take().filter(|o| o.variant == Some(letter)) else {
                return Err(ScriptError::new(format!("no hay variante {letter}")));
            };
            let batch = match song_batch(&other, &self.slots, self.song.as_ref()) {
                Ok(batch) => batch,
                Err(e) => {
                    self.other = Some(other);
                    return Err(e);
                }
            };
            if let Err(e) = controller.apply(batch) {
                self.other = Some(other);
                return Err(ScriptError::new(format!("no se pudo enviar al motor: {e}")));
            }
            self.other = self.song.replace(other);
        }
        Ok(self.song.as_ref().unwrap())
    }

    /// Alterna entre A y B (ver [`ScriptHost::set_variant`]).
    pub fn toggle_variant(&mut self, controller: &mut Controller) -> Result<&Song, ScriptError> {
        let next = if self.variant() == Some('A') {
            'B'
        } else {
            'A'
        };
        self.set_variant(controller, next)
    }

    /// Evalúa el script y devuelve el modelo, sin tocar el motor.
    pub fn eval(&mut self, source: &str, name: &str, base_dir: &Path) -> Result<Song, ScriptError> {
        let lua = Lua::new_with(
            StdLib::MATH | StdLib::STRING | StdLib::TABLE | StdLib::UTF8,
            LuaOptions::default(),
        )
        .map_err(|e| ScriptError::from_lua(&e))?;

        // Cortar bucles infinitos: un script colgado tampoco rompe lo que suena.
        let deadline = Instant::now() + self.time_limit;
        let limit = self.time_limit;
        lua.set_hook(
            HookTriggers::new().every_nth_instruction(HOOK_INSTRUCTIONS),
            move |_, _| {
                if Instant::now() > deadline {
                    Err(mlua::Error::runtime(format!(
                        "el script tarda más de {:.1} s (¿bucle infinito?)",
                        limit.as_secs_f32()
                    )))
                } else {
                    Ok(VmState::Continue)
                }
            },
        )
        .map_err(|e| ScriptError::from_lua(&e))?;

        let warnings: Rc<RefCell<Vec<String>>> = Rc::default();
        // Semilla del azar de la carga: `chance` y, sin semilla propia,
        // `mutate` y `shuffle` (con un generador aparte, para que unos no
        // cambien los resultados de los otros).
        let seed = self.seed.unwrap_or_else(|| {
            SystemTime::now()
                .duration_since(SystemTime::UNIX_EPOCH)
                .map_or(0, |d| d.as_nanos() as u64)
        });
        let pattern_rng = Rc::new(RefCell::new(Rng::new(seed.rotate_left(29) ^ PATTERN_SEED)));
        let state = Rc::new(RefCell::new(Builder {
            bpm: None,
            metronome: None,
            buses: Buses::default(),
            tempo_ramp: None,
            meter: None,
            tracks: Vec::new(),
            expanding: Rc::new(Cell::new(false)),
            sections: Vec::new(),
            arrange: None,
            in_section: Rc::new(Cell::new(false)),
            names: Vec::new(),
            variants: Vec::new(),
            priority: Vec::new(),
            groups: Vec::new(),
            macros: Vec::new(),
            key: None,
            transpose: None,
            key_used: None,
            frozen: Vec::new(),
        }));
        // La caché pasa al estado de Lua durante la evaluación y vuelve después.
        let cache = Rc::new(RefCell::new(std::mem::take(&mut self.samples)));
        let imports = Rc::new(RefCell::new(Imports {
            old: std::mem::take(&mut self.imported),
            used: Vec::new(),
        }));
        let result = self
            .install_api(
                &lua,
                &state,
                (&cache, &imports),
                &warnings,
                &pattern_rng,
                base_dir,
            )
            .and_then(|()| {
                // Para saber dónde está escrito cada patrón.
                lua.set_app_data(written::ScriptText::new(source, format!("={name}")));
                lua.load(source).set_name(format!("={name}")).exec()
            })
            .and_then(|()| run_bodies(&state));
        self.samples = std::mem::take(&mut *cache.borrow_mut());
        self.imported = std::mem::take(&mut imports.borrow_mut().used);
        let versions = result.map_err(|e| ScriptError::from_lua(&e))?;

        let priority = std::mem::take(&mut state.borrow_mut().priority);
        let groups = std::mem::take(&mut state.borrow_mut().groups);
        let macros = std::mem::take(&mut state.borrow_mut().macros);
        let frozen = std::mem::take(&mut state.borrow_mut().frozen);
        let (key, transpose) = {
            let builder = state.borrow();
            (
                builder.key.as_ref().map(|(k, _)| *k),
                builder.transpose.as_ref().map_or(0, |(t, _)| *t),
            )
        };
        let (bpm, metronome, buses, tempo_ramp, meter, tracks, expanding, arrange) = {
            let mut builder = state.borrow_mut();
            (
                builder
                    .tempo_ramp
                    .map(|r| r.from)
                    .or(builder.bpm)
                    .or(self.default_bpm),
                builder.metronome,
                builder.buses,
                builder.tempo_ramp,
                builder.meter,
                std::mem::take(&mut builder.tracks),
                builder.expanding.clone(),
                builder.arrange.take(),
            )
        };
        if tracks.len() > MAX_TRACKS {
            return Err(ScriptError::new(format!(
                "{name}: hay {} pistas y el máximo es {MAX_TRACKS}",
                tracks.len()
            )));
        }
        if versions.iter().all(|v| v.sections.is_none())
            && let Some((_, location)) = &arrange
        {
            return Err(ScriptError::new(format!(
                "{location}arrange: no hay secciones; defínelas con section(\"nombre\", function() ... end)"
            )));
        }

        // Expandir `every` y `chance` a un ciclo de compases. Las funciones de
        // `every` se llaman ahora, con el estado de Lua todavía vivo. Cada
        // versión (A y B) con el azar desde la misma semilla: lo que no
        // cambia entre ellas sale igual.
        let tempo = bpm.unwrap_or(engine::DEFAULT_BPM);
        let timing = Timing {
            bpm: tempo,
            seed,
            key: key.map(|k| k.degrees()),
            transpose,
        };
        let mut used = Vec::new();
        let tags = Cell::new(self.last_tag);
        let base_warnings = warnings.borrow().clone();
        let mut songs = Vec::with_capacity(versions.len());
        for version in versions {
            let mut rng = Rng::new(seed);
            let mut finish = |mut tracks: Vec<TrackBuild>| -> Result<Vec<TrackDef>, ScriptError> {
                self.apply_stretch(&mut tracks, tempo, name, &mut used)?;
                expanding.set(true);
                let expanded = tracks
                    .into_iter()
                    .map(|track| expand(track, &mut rng, timing))
                    .collect::<Result<Vec<_>, _>>();
                expanding.set(false);
                expanded.map(|mut tracks| {
                    for track in &mut tracks {
                        let tag = tags.get() % engine::MAX_TAG + 1;
                        tags.set(tag);
                        track.tag = tag;
                    }
                    tracks
                })
            };
            let mut warnings = base_warnings.clone();
            let (mut tracks, sections, timeline) = match version.sections {
                None => (finish(version.base)?, Vec::new(), Vec::new()),
                Some(built) => {
                    let sections = built
                        .into_iter()
                        .map(|(name, tracks)| {
                            Ok(SectionDef {
                                name,
                                tracks: finish(tracks)?,
                            })
                        })
                        .collect::<Result<Vec<_>, ScriptError>>()?;
                    let timeline = match &arrange {
                        Some((entries, location)) => {
                            let timeline = entries
                                .iter()
                                .map(|(section, bars)| {
                                    let index = sections.iter().position(|s| &s.name == section);
                                    index.map(|i| (i, *bars)).ok_or_else(|| {
                                        let known: Vec<&str> =
                                            sections.iter().map(|s| s.name.as_str()).collect();
                                        ScriptError::new(format!(
                                            "{location}arrange: la sección \"{section}\" no existe (hay: {})",
                                            known.join(", ")
                                        ))
                                    })
                                })
                                .collect::<Result<Vec<_>, _>>()?;
                            let total: u64 = timeline.iter().map(|&(_, b)| b as u64).sum();
                            if total > MAX_SONG_BARS {
                                warnings.push(format!(
                                    "la línea de tiempo dura {total} compases: exportar se queda en los primeros {MAX_SONG_BARS}"
                                ));
                            }
                            timeline
                        }
                        None => {
                            if sections.len() > 1 {
                                warnings.push(format!(
                                    "hay {} secciones y ningún arrange(): suena solo \"{}\"",
                                    sections.len(),
                                    sections[0].name
                                ));
                            }
                            Vec::new()
                        }
                    };
                    let first = timeline.first().map_or(0, |&(s, _)| s);
                    (sections[first].tracks.clone(), sections, timeline)
                }
            };
            // Las pistas congeladas tocan su WAV (con secciones, no hay).
            if let Some(request) = frozen.first()
                && !sections.is_empty()
            {
                return Err(ScriptError::new(format!(
                    "{}frozen(\"{}\"): con secciones no se congela (cada sección suena distinta)",
                    request.location, request.track
                )));
            }
            for request in &frozen {
                apply_frozen(
                    &mut tracks,
                    request,
                    tempo,
                    meter,
                    self.sample_rate,
                    &mut warnings,
                )?;
            }
            let all = || {
                sections
                    .iter()
                    .flat_map(|s| s.tracks.iter())
                    .chain(sections.is_empty().then_some(&tracks).into_iter().flatten())
            };
            for warning in all()
                .filter_map(far_kept_pitch)
                .chain(all().filter_map(idle_formant))
                .chain(all().filter_map(grains_skip_slice_fx))
                .chain(all().filter_map(dry_loop_seam))
            {
                if !warnings.contains(&warning) {
                    warnings.push(warning);
                }
            }
            check_sidechains(all())?;
            for name in &priority {
                if !all().any(|t| &t.name == name) {
                    warnings.push(format!(
                        "priority: no hay ninguna pista \"{name}\" (no cuenta)"
                    ));
                }
            }
            songs.push(Song {
                bpm,
                tracks,
                metronome,
                buses,
                tempo_ramp,
                meter,
                warnings,
                sections,
                timeline,
                variant: version.letter,
                alternate: None,
                priority: priority.clone(),
                groups: groups.clone(),
                macros: macros.clone(),
                key,
                transpose,
            });
        }
        self.stretched = used;
        self.last_tag = tags.get();
        // Con variantes, la A lleva a la B.
        let mut songs = songs.into_iter();
        let mut song = songs.next().expect("al menos una versión");
        if let Some(b) = songs.next() {
            for warning in &b.warnings {
                if !song.warnings.contains(warning) {
                    song.warnings.push(warning.clone());
                }
            }
            let mut names = song.track_names();
            for n in b.track_names() {
                if !names.contains(&n) {
                    names.push(n);
                }
            }
            if names.len() > MAX_TRACKS {
                return Err(ScriptError::new(format!(
                    "{name}: entre las variantes A y B hay {} pistas y el máximo es {MAX_TRACKS}",
                    names.len()
                )));
            }
            song.alternate = Some(Box::new(b));
        }
        check_groups(&song)?;
        Ok(song)
    }

    /// Estira los samples de las pistas con `:stretch()` al tempo de la
    /// canción y escala sus cortes. Las pistas del mismo sample comparten
    /// el resultado (los inicios de todos sus cortes son límites de tramo),
    /// que se guarda para la carga siguiente.
    fn apply_stretch(
        &self,
        tracks: &mut [TrackBuild],
        bpm: f64,
        name: &str,
        used: &mut Vec<Stretched>,
    ) -> Result<(), ScriptError> {
        // Samples y cortes originales de las pistas con stretch (antes de
        // sustituirlos).
        let originals: Vec<Option<(Arc<Sample>, Vec<usize>)>> = tracks
            .iter()
            .map(|t| {
                t.def.stretch.map(|_| {
                    let starts = t.def.slices.iter().map(|r| r.start).collect();
                    (t.def.sample.clone(), starts)
                })
            })
            .collect();
        for track in tracks.iter_mut() {
            let Some(stretch) = track.def.stretch else {
                continue;
            };
            let ratio = stretch.source_bpm / bpm;
            if !(engine::MIN_STRETCH..=engine::MAX_STRETCH).contains(&ratio) {
                return Err(ScriptError::new(format!(
                    "{name}: pista \"{}\": estirar de {} a {bpm} BPM es ×{ratio:.2} \
                     (va de ×{} a ×{})",
                    track.def.name,
                    stretch.source_bpm,
                    engine::MIN_STRETCH,
                    engine::MAX_STRETCH
                )));
            }
            let original = track.def.sample.clone();
            let mut bounds: Vec<usize> = originals
                .iter()
                .flatten()
                .filter(|(sample, _)| Arc::ptr_eq(sample, &original))
                .flat_map(|(_, starts)| starts.iter().copied())
                .collect();
            bounds.sort_unstable();
            bounds.dedup();
            let same = |e: &Stretched| {
                Arc::ptr_eq(&e.original, &original) && e.ratio == ratio && e.bounds == bounds
            };
            let result = match used.iter().chain(&self.stretched).find(|e| same(e)) {
                Some(entry) => entry.result.clone(),
                None => Arc::new(original.stretched_with(ratio, &bounds)),
            };
            if !used.iter().any(same) {
                used.push(Stretched {
                    original: original.clone(),
                    ratio,
                    bounds,
                    result: result.clone(),
                });
            }
            let def = &mut track.def;
            def.slices = def.slices.scaled(ratio, result.frames());
            def.sample = result;
            def.stretch = Some(Stretch {
                source_bpm: stretch.source_bpm,
                ratio,
            });
        }
        Ok(())
    }

    fn install_api(
        &self,
        lua: &Lua,
        state: &Rc<RefCell<Builder>>,
        // Los WAV cargados y los samples preparados al importar.
        (cache, imports): (&Rc<RefCell<SampleCache>>, &Rc<RefCell<Imports>>),
        warnings: &Rc<RefCell<Vec<String>>>,
        pattern_rng: &Rc<RefCell<Rng>>,
        base_dir: &Path,
    ) -> mlua::Result<()> {
        let globals = lua.globals();

        let expanding = state.borrow().expanding.clone();
        let guard = move |lua: &Lua, what: &str| {
            if expanding.get() {
                Err(fail(
                    lua,
                    format!("no se puede llamar a {what}() dentro de every()"),
                ))
            } else {
                Ok(())
            }
        };

        // Lo que es de toda la canción no va dentro de una sección.
        let in_section = state.borrow().in_section.clone();
        let guard = move |lua: &Lua, what: &str| {
            guard(lua, what)?;
            if in_section.get() && GLOBAL_ONLY.contains(&what) {
                return Err(fail(
                    lua,
                    format!("{what}() es de toda la canción: va fuera de section()"),
                ));
            }
            Ok(())
        };

        let st = state.clone();
        let g = guard.clone();
        globals.set(
            "section",
            lua.create_function(move |lua, (name, func): (String, Function)| {
                g(lua, "section")?;
                let mut builder = st.borrow_mut();
                if name.trim().is_empty() {
                    return Err(fail(lua, "section: el nombre está vacío"));
                }
                if builder.sections.iter().any(|s| s.name == name) {
                    return Err(fail(
                        lua,
                        format!("section(\"{name}\"): ya hay una sección con ese nombre"),
                    ));
                }
                builder.sections.push(SectionBuild { name, func });
                Ok(())
            })?,
        )?;

        let st = state.clone();
        let g = guard.clone();
        globals.set(
            "arrange",
            lua.create_function(move |lua, list: Table| {
                g(lua, "arrange")?;
                let mut entries = Vec::new();
                for (i, entry) in list.sequence_values::<Value>().enumerate() {
                    let n = i + 1;
                    let bad = || {
                        fail(
                            lua,
                            format!("arrange: la entrada {n} es {{\"sección\", compases}}"),
                        )
                    };
                    let Value::Table(entry) = entry? else {
                        return Err(bad());
                    };
                    let (Ok(Value::String(section)), Ok(bars)) =
                        (entry.raw_get::<Value>(1), entry.raw_get::<Value>(2))
                    else {
                        return Err(bad());
                    };
                    let section = section.to_str()?.to_string();
                    let bars = match bars {
                        Value::Integer(b) if (1..=MAX_SONG_BARS as i64).contains(&b) => b as u32,
                        Value::Number(b) if b.fract() == 0.0 && (1.0..=MAX_SONG_BARS as f64).contains(&b) => b as u32,
                        other => {
                            let shown = match other {
                                Value::Integer(b) => b.to_string(),
                                Value::Number(b) => b.to_string(),
                                other => other.type_name().to_string(),
                            };
                            return Err(fail(
                                lua,
                                format!(
                                    "arrange: \"{section}\" con {shown} compases: van de 1 a {MAX_SONG_BARS}"
                                ),
                            ));
                        }
                    };
                    entries.push((section, bars));
                }
                if entries.is_empty() {
                    return Err(fail(lua, "arrange: la línea de tiempo está vacía"));
                }
                if entries.len() > engine::MAX_ENTRIES {
                    return Err(fail(
                        lua,
                        format!("arrange: como mucho {} entradas", engine::MAX_ENTRIES),
                    ));
                }
                st.borrow_mut().arrange = Some((entries, location(lua)));
                Ok(())
            })?,
        )?;

        let st = state.clone();
        let g = guard.clone();
        globals.set(
            "variant",
            lua.create_function(
                move |lua, (target, letter, func): (String, String, Function)| {
                    g(lua, "variant")?;
                    let letter = match letter.as_str() {
                        "A" => 'A',
                        "B" => 'B',
                        other => {
                            return Err(fail(
                                lua,
                                format!("variant(\"{target}\", \"{other}\"): la variante es \"A\" o \"B\""),
                            ));
                        }
                    };
                    let mut builder = st.borrow_mut();
                    if builder
                        .variants
                        .iter()
                        .any(|v| v.target == target && v.letter == letter)
                    {
                        return Err(fail(
                            lua,
                            format!("variant(\"{target}\", \"{letter}\"): ya está definida"),
                        ));
                    }
                    builder.variants.push(VariantBuild {
                        target,
                        letter,
                        func,
                        location: location(lua),
                    });
                    Ok(())
                },
            )?,
        )?;

        let st = state.clone();
        let g = guard.clone();
        globals.set(
            "key",
            lua.create_function(move |lua, (tonic, mode): (String, Option<String>)| {
                g(lua, "key")?;
                let mode = mode.unwrap_or_else(|| "major".into());
                let who = format!("key(\"{tonic}\", \"{mode}\")");
                let root = api::note(&tonic).ok_or_else(|| {
                    fail(
                        lua,
                        format!("{who}: tónica no válida (de C a B, con # o b: F#, Bb)"),
                    )
                })?;
                let scale = api::scale(&mode).ok_or_else(|| {
                    let names: Vec<&str> = api::SCALES.iter().map(|s| s.name).collect();
                    fail(
                        lua,
                        format!(
                            "{who}: el modo es una escala de scale() ({})",
                            names.join(", ")
                        ),
                    )
                })?;
                let mut builder = st.borrow_mut();
                if let Some((_, at)) = &builder.key {
                    return Err(fail(
                        lua,
                        format!("{who}: la tonalidad ya está en {at}(una por canción)"),
                    ));
                }
                if let Some(at) = &builder.key_used {
                    return Err(fail(
                        lua,
                        format!(
                            "{who}: va antes de scale() y de melody(), y en {at}ya se usó la \
                             tonalidad por defecto; súbelo al principio"
                        ),
                    ));
                }
                builder.key = Some((
                    SongKey {
                        tonic: root.rem_euclid(12),
                        scale: api::scale(scale.name).expect("existe"),
                    },
                    location(lua),
                ));
                Ok(())
            })?,
        )?;

        let st = state.clone();
        let g = guard.clone();
        globals.set(
            "transpose",
            lua.create_function(move |lua, semitones: Value| {
                g(lua, "transpose")?;
                let n = match semitones {
                    Value::Integer(n) if (-24..=24).contains(&n) => n as i32,
                    Value::Number(n) if n.fract() == 0.0 && (-24.0..=24.0).contains(&n) => n as i32,
                    other => {
                        return Err(fail(
                            lua,
                            format!(
                                "transpose({}): semitonos enteros de -24 a 24",
                                value_text(&other)
                            ),
                        ));
                    }
                };
                let mut builder = st.borrow_mut();
                if let Some((_, at)) = &builder.transpose {
                    return Err(fail(
                        lua,
                        format!("transpose({n}): ya está en {at}(uno por canción)"),
                    ));
                }
                builder.transpose = Some((n, location(lua)));
                Ok(())
            })?,
        )?;

        let st = state.clone();
        let g = guard.clone();
        globals.set(
            "bpm",
            lua.create_function(move |lua, bpm: f64| {
                g(lua, "bpm")?;
                if !valid_bpm(bpm) {
                    return Err(fail(lua, SendError::InvalidBpm(bpm)));
                }
                st.borrow_mut().bpm = Some(bpm);
                Ok(())
            })?,
        )?;

        let st = state.clone();
        let g = guard.clone();
        globals.set(
            "bpm_ramp",
            lua.create_function(move |lua, (from, to, duration): (f64, f64, String)| {
                g(lua, "bpm_ramp")?;
                let who = format!("bpm_ramp({from}, {to}, \"{duration}\")");
                for bpm in [from, to] {
                    if !valid_bpm(bpm) {
                        return Err(fail(lua, format!("{who}: {}", SendError::InvalidBpm(bpm))));
                    }
                }
                let bars = parse_duration(&duration)
                    .filter(|steps| steps.fract() == 0.0 && (*steps as usize).is_multiple_of(STEPS))
                    .map(|steps| steps as u64 / STEPS as u64)
                    .ok_or_else(|| {
                        fail(
                            lua,
                            format!("{who}: la duración es \"N compases\" (hasta 64)"),
                        )
                    })?;
                st.borrow_mut().tempo_ramp = Some(engine::TempoRamp { from, to, bars });
                Ok(())
            })?,
        )?;

        let st = state.clone();
        let g = guard.clone();
        globals.set(
            "meter",
            lua.create_function(move |lua, (n, d): (i64, i64)| {
                g(lua, "meter")?;
                let meter = usize::try_from(n)
                    .ok()
                    .zip(usize::try_from(d).ok())
                    .and_then(|(n, d)| engine::Meter::new(n, d))
                    .ok_or_else(|| {
                        fail(
                            lua,
                            format!(
                                "meter({n}, {d}): el denominador es 4, 8 o 16 y el compás, de 1 a 64 semicorcheas"
                            ),
                        )
                    })?;
                st.borrow_mut().meter = Some(meter);
                Ok(())
            })?,
        )?;

        let st = state.clone();
        let g = guard.clone();
        globals.set(
            "priority",
            lua.create_function(move |lua, names: Table| {
                g(lua, "priority")?;
                let mut list: Vec<String> = Vec::new();
                for value in names.sequence_values::<Value>() {
                    let name = match value? {
                        Value::String(s) => s.to_str()?.to_string(),
                        other => {
                            return Err(fail(
                                lua,
                                format!(
                                    "priority: cada pista va por su nombre, entre comillas (no {})",
                                    other.type_name()
                                ),
                            ));
                        }
                    };
                    if list.contains(&name) {
                        return Err(fail(lua, format!("priority: \"{name}\" está dos veces")));
                    }
                    list.push(name);
                }
                if list.is_empty() {
                    return Err(fail(
                        lua,
                        "priority: falta la lista de pistas, de la que más manda a la que menos ({ \"kick\", \"bass\" })",
                    ));
                }
                st.borrow_mut().priority = list;
                Ok(())
            })?,
        )?;

        let st = state.clone();
        let g = guard.clone();
        globals.set(
            "metronome",
            lua.create_function(move |lua, on: Option<bool>| {
                g(lua, "metronome")?;
                st.borrow_mut().metronome = Some(on.unwrap_or(true));
                Ok(())
            })?,
        )?;

        let st = state.clone();
        let g = guard.clone();
        globals.set(
            "bus",
            lua.create_function(move |lua, (name, params): (String, Option<Table>)| {
                g(lua, "bus")?;
                let who = format!("bus(\"{name}\")");
                let Some(def) = api::bus(&name) else {
                    return Err(fail(
                        lua,
                        format!(
                            "{who}: no existe ({})",
                            names(api::BUSES.iter().map(|b| b.name))
                        ),
                    ));
                };
                let known: Vec<&str> = def.params.iter().map(|p| p.name).collect();
                let get = |key: &str| -> mlua::Result<Option<Value>> {
                    match &params {
                        Some(p) => p.get::<Option<Value>>(key),
                        None => Ok(None),
                    }
                };
                if let Some(params) = &params {
                    for pair in params.pairs::<Value, Value>() {
                        let (key, _) = pair?;
                        let key = match key {
                            Value::String(s) => s.to_str()?.to_string(),
                            other => format!("{other:?}"),
                        };
                        if !known.contains(&key.as_str()) {
                            return Err(fail(
                                lua,
                                format!(
                                    "{who}: parámetro desconocido \"{key}\" (admite: {})",
                                    known.join(", ")
                                ),
                            ));
                        }
                    }
                }
                // Un número o una curva (con su índice en `Buses::curves`).
                let curves = RefCell::new([None; engine::bus::BUS_PARAMS]);
                let number = |key: &str, default: f32, max: f32, slot: usize| -> mlua::Result<f32> {
                    let value = match get(key)? {
                        None => default,
                        Some(Value::Integer(i)) => i as f32,
                        Some(Value::Number(n)) => n as f32,
                        Some(Value::UserData(ud)) if ud.is::<CurveHandle>() => {
                            if let Some(param) = def.param(key) {
                                no_curve(lua, &who, param)?;
                            }
                            let curve = ud.borrow::<CurveHandle>()?.0;
                            let (a, b) = curve.bounds();
                            if ![a, b].iter().all(|v| (0.0..=max).contains(v)) {
                                return Err(fail(
                                    lua,
                                    format!("{who}: {key}: la curva se sale del rango (0 a {max})"),
                                ));
                            }
                            curves.borrow_mut()[slot] = Some(curve);
                            a
                        }
                        Some(_) => {
                            return Err(fail(
                                lua,
                                format!("{who}: {key} tiene que ser un número, ramp() o lfo()"),
                            ));
                        }
                    };
                    if !(0.0..=max).contains(&value) {
                        return Err(fail(
                            lua,
                            format!("{who}: {key} = {value} fuera de rango (0 a {max})"),
                        ));
                    }
                    Ok(value)
                };
                use engine::bus::{
                    DELAY_FEEDBACK, DELAY_LEVEL, REVERB_DAMP, REVERB_LEVEL, REVERB_SIZE,
                };
                let mut builder = st.borrow_mut();
                let buses = &mut builder.buses;
                match name.as_str() {
                    "delay" => {
                        let d = engine::DelayParams::default();
                        let division = match get("time")? {
                            None => d.division,
                            Some(value) if is_curve(&value) => {
                                let param = def.param("time").expect("time es del registro");
                                no_curve(lua, &who, param)?;
                                d.division
                            }
                            Some(Value::String(text)) => {
                                let text = text.to_str()?.to_string();
                                Division::parse(&text).ok_or_else(|| {
                                    fail(
                                        lua,
                                        format!(
                                            "{who}: time = \"{text}\" no existe (\"1/4\", \"3/16\", \"1/8\" o \"1/16\")"
                                        ),
                                    )
                                })?
                            }
                            Some(_) => {
                                return Err(fail(
                                    lua,
                                    format!("{who}: time va entre comillas: \"1/8\""),
                                ));
                            }
                        };
                        buses.delay = engine::DelayParams {
                            division,
                            feedback: number(
                                "feedback",
                                d.feedback,
                                engine::bus::MAX_FEEDBACK,
                                DELAY_FEEDBACK,
                            )?,
                            level: number("level", d.level, 1.0, DELAY_LEVEL)?,
                        };
                        let curves = curves.borrow();
                        for slot in [DELAY_FEEDBACK, DELAY_LEVEL] {
                            buses.curves[slot] = curves[slot];
                        }
                    }
                    _ => {
                        let r = engine::ReverbParams::default();
                        buses.reverb = engine::ReverbParams {
                            size: number("size", r.size, 1.0, REVERB_SIZE)?,
                            damp: number("damp", r.damp, 1.0, REVERB_DAMP)?,
                            level: number("level", r.level, 1.0, REVERB_LEVEL)?,
                        };
                        let curves = curves.borrow();
                        for slot in [REVERB_SIZE, REVERB_DAMP, REVERB_LEVEL] {
                            buses.curves[slot] = curves[slot];
                        }
                    }
                }
                Ok(())
            })?,
        )?;

        let cache = cache.clone();
        let cache_for_frozen = cache.clone();
        let imports = imports.clone();
        let base_dir = self
            .sample_dir
            .clone()
            .unwrap_or_else(|| base_dir.to_path_buf());
        let frozen_dir = base_dir.clone();
        let rate = self.sample_rate;
        let g = guard.clone();
        let labels = Rc::new(self.labels.clone());
        let positions = Rc::new(self.positions.clone());
        let edits = Rc::new(self.edits.clone());
        let warnings = warnings.clone();
        globals.set(
            "sample",
            lua.create_function(move |lua, (name, options): (String, Option<Table>)| {
                g(lua, "sample")?;
                let path = base_dir.join(&name);
                let mut sample = load_cached(&mut cache.borrow_mut(), &path, rate)
                    .map_err(|e| fail(lua, format!("{}: {e}", path.display())))?;
                if let Some(options) = options {
                    let options = import_options(lua, &name, &options)?;
                    if !options.is_empty() {
                        sample = imports.borrow_mut().get(&sample, options);
                    }
                }
                Ok(SampleHandle {
                    name,
                    path,
                    sample,
                    labels: labels.clone(),
                    positions: positions.clone(),
                    warnings: warnings.clone(),
                    stretch: None,
                    edits: edits.clone(),
                })
            })?,
        )?;

        let cache = cache_for_frozen;
        let base = frozen_dir;
        let st = state.clone();
        let g = guard.clone();
        globals.set(
            "frozen",
            lua.create_function(
                move |lua, (track, wav, fingerprint): (String, String, Option<String>)| {
                    g(lua, "frozen")?;
                    let who = format!("frozen(\"{track}\", \"{wav}\")");
                    let path = base.join(&wav);
                    let sample =
                        load_cached(&mut cache.borrow_mut(), &path, rate).map_err(|e| {
                            fail(
                                lua,
                                format!(
                                    "{who}: {}: {e} (descongélala o vuelve a congelarla)",
                                    path.display()
                                ),
                            )
                        })?;
                    let mut builder = st.borrow_mut();
                    if let Some(old) = builder.frozen.iter().find(|r| r.track == track) {
                        return Err(fail(
                            lua,
                            format!(
                                "{who}: ya está congelada en {}(una vez por pista)",
                                old.location
                            ),
                        ));
                    }
                    builder.frozen.push(FreezeRequest {
                        track,
                        wav,
                        path,
                        sample,
                        fingerprint,
                        location: location(lua),
                    });
                    Ok(())
                },
            )?,
        )?;

        let g = guard.clone();
        globals.set(
            "sh101",
            lua.create_function(move |lua, options: Option<Table>| {
                g(lua, "sh101")?;
                let def = synth_options(lua, "sh101", options.as_ref())?;
                Ok(SynthHandle {
                    def,
                    sample_rate: rate,
                })
            })?,
        )?;
        let g = guard.clone();
        globals.set(
            "subharm",
            lua.create_function(move |lua, options: Option<Table>| {
                g(lua, "subharm")?;
                let def = synth_options(lua, "subharm", options.as_ref())?;
                Ok(SynthHandle {
                    def,
                    sample_rate: rate,
                })
            })?,
        )?;
        let g = guard.clone();
        globals.set(
            "synth",
            lua.create_function(move |lua, (kind, options): (String, Option<Table>)| {
                g(lua, "synth")?;
                let def = synth_options(lua, &kind, options.as_ref())?;
                Ok(SynthHandle {
                    def,
                    sample_rate: rate,
                })
            })?,
        )?;

        // Funciones de patrones: devuelven texto en mini-notación. Valen
        // también dentro de `every`.
        globals.set(
            "euclid",
            lua.create_function(
                |lua, (k, n, hit, length): (i64, i64, Option<String>, Option<i64>)| {
                    patterns::euclid(k, n, hit.as_deref().unwrap_or("x"), length)
                        .map_err(|e| fail(lua, e))
                },
            )?,
        )?;

        globals.set(
            "rotate",
            lua.create_function(|lua, (text, n): (String, i64)| {
                patterns::rotate(&text, n).map_err(|e| fail(lua, e))
            })?,
        )?;

        globals.set(
            "reverse_pattern",
            lua.create_function(|lua, text: String| {
                patterns::reverse_pattern(&text).map_err(|e| fail(lua, e))
            })?,
        )?;

        // Con semilla, su propio generador (mismo resultado siempre); sin
        // ella, el de la carga, como `chance`.
        let rng = pattern_rng.clone();
        globals.set(
            "mutate",
            lua.create_function(move |lua, (text, p, seed): (String, f64, Option<i64>)| {
                let result = match seed {
                    Some(seed) => patterns::mutate(&text, p, &mut Rng::new(seed as u64)),
                    None => patterns::mutate(&text, p, &mut rng.borrow_mut()),
                };
                result.map_err(|e| fail(lua, e))
            })?,
        )?;

        let rng = pattern_rng.clone();
        globals.set(
            "shuffle",
            lua.create_function(move |lua, (text, seed): (String, Option<i64>)| {
                let result = match seed {
                    Some(seed) => patterns::shuffle(&text, &mut Rng::new(seed as u64)),
                    None => patterns::shuffle(&text, &mut rng.borrow_mut()),
                };
                result.map_err(|e| fail(lua, e))
            })?,
        )?;

        let st = state.clone();
        let g = guard.clone();
        globals.set(
            "group",
            lua.create_function(move |lua, (name, tracks): (String, Option<Table>)| {
                g(lua, "group")?;
                let who = format!("group(\"{name}\")");
                let mut builder = st.borrow_mut();
                let exists = builder.groups.iter().any(|g| g.name == name);
                match (tracks, exists) {
                    (None, true) => {}
                    (None, false) => {
                        return Err(fail(
                            lua,
                            format!(
                                "{who}: no existe; la primera vez va con sus pistas: group(\"{name}\", {{ \"kick\", \"snare\" }})"
                            ),
                        ));
                    }
                    (Some(_), true) => {
                        return Err(fail(
                            lua,
                            format!(
                                "{who}: ya existe (para cambiar sus efectos, group(\"{name}\") sin las pistas)"
                            ),
                        ));
                    }
                    (Some(list), false) => {
                        if name.trim().is_empty() {
                            return Err(fail(lua, "group: el grupo necesita un nombre"));
                        }
                        if builder.groups.len() >= engine::MAX_GROUPS {
                            return Err(fail(
                                lua,
                                format!("{who}: hay {} grupos como mucho", engine::MAX_GROUPS),
                            ));
                        }
                        let mut members: Vec<String> = Vec::new();
                        for value in list.sequence_values::<Value>() {
                            let track = match value? {
                                Value::String(s) => s.to_str()?.to_string(),
                                other => {
                                    return Err(fail(
                                        lua,
                                        format!(
                                            "{who}: cada pista va por su nombre, entre comillas (no {})",
                                            other.type_name()
                                        ),
                                    ));
                                }
                            };
                            if members.contains(&track) {
                                return Err(fail(
                                    lua,
                                    format!("{who}: \"{track}\" está dos veces"),
                                ));
                            }
                            if let Some(other) =
                                builder.groups.iter().find(|g| g.tracks.contains(&track))
                            {
                                return Err(fail(
                                    lua,
                                    format!(
                                        "{who}: \"{track}\" ya va al grupo \"{}\" (cada pista, a uno)",
                                        other.name
                                    ),
                                ));
                            }
                            members.push(track);
                        }
                        if members.is_empty() {
                            return Err(fail(
                                lua,
                                format!(
                                    "{who}: falta la lista de pistas ({{ \"kick\", \"snare\" }})"
                                ),
                            ));
                        }
                        builder.groups.push(GroupDef::new(name.clone(), members));
                    }
                }
                Ok(GroupHandle {
                    name,
                    state: st.clone(),
                })
            })?,
        )?;

        let st = state.clone();
        let g = guard.clone();
        globals.set(
            "macro",
            lua.create_function(move |lua, (name, from, to): (String, f32, f32)| {
                g(lua, "macro")?;
                let index = st
                    .borrow_mut()
                    .macro_index(&name)
                    .map_err(|e| fail(lua, format!("macro(\"{name}\", ...): {e}")))?;
                Ok(CurveHandle(engine::Curve::Macro {
                    index: index as u8,
                    from,
                    to,
                }))
            })?,
        )?;

        let st = state.clone();
        let g = guard.clone();
        globals.set(
            "macro_init",
            lua.create_function(move |lua, (name, value): (String, f32)| {
                g(lua, "macro_init")?;
                let who = format!("macro_init(\"{name}\", {value})");
                if !(0.0..=1.0).contains(&value) {
                    return Err(fail(lua, format!("{who}: el valor va de 0 a 1")));
                }
                let mut builder = st.borrow_mut();
                let index = builder
                    .macro_index(&name)
                    .map_err(|e| fail(lua, format!("{who}: {e}")))?;
                builder.macros[index].value = value;
                Ok(())
            })?,
        )?;

        globals.set(
            "ramp",
            lua.create_function(|lua, (from, to, duration): (f32, f32, String)| {
                let steps = parse_duration(&duration).ok_or_else(|| {
                    fail(
                        lua,
                        format!(
                            "ramp({from}, {to}, \"{duration}\"): la duración es \"N compases\" o \"N pasos\" (hasta 64 compases)"
                        ),
                    )
                })?;
                Ok(CurveHandle(engine::Curve::Ramp { from, to, steps }))
            })?,
        )?;

        globals.set(
            "lfo",
            lua.create_function(|lua, (rate, min, max): (Value, f32, f32)| {
                let rate = match rate {
                    Value::Integer(hz) => engine::LfoRate::Hz(hz as f32),
                    Value::Number(hz) => engine::LfoRate::Hz(hz as f32),
                    Value::String(text) => {
                        let text = text.to_str()?.to_string();
                        engine::LfoRate::Steps(parse_duration(&text).ok_or_else(|| {
                            fail(
                                lua,
                                format!(
                                    "lfo(\"{text}\", ...): la frecuencia es un número (Hz) o lo que dura un ciclo: \"1 compás\", \"8 pasos\""
                                ),
                            )
                        })?)
                    }
                    _ => {
                        return Err(fail(
                            lua,
                            "lfo(frecuencia, mín, máx): la frecuencia es un número (Hz) o \"N compases\"",
                        ));
                    }
                };
                if let engine::LfoRate::Hz(hz) = rate
                    && !(0.01..=50.0).contains(&hz)
                {
                    return Err(fail(lua, format!("lfo({hz}, ...): de 0.01 a 50 Hz")));
                }
                Ok(CurveHandle(engine::Curve::Lfo { min, max, rate }))
            })?,
        )?;

        let st = state.clone();
        globals.set(
            "scale",
            lua.create_function(move |lua, (name, tonic): (Option<String>, Option<String>)| {
                // Sin nombre o sin tónica, los de `key()`.
                let key = st.borrow().key.as_ref().map(|(k, _)| *k);
                if name.is_none() || tonic.is_none() {
                    let mut builder = st.borrow_mut();
                    if builder.key_used.is_none() {
                        builder.key_used = Some(location(lua));
                    }
                }
                let name = match (name, key) {
                    (Some(name), _) => name,
                    (None, Some(key)) => key.scale.name.to_string(),
                    (None, None) => {
                        return Err(fail(
                            lua,
                            "scale(): sin nombre usa la tonalidad de la canción; pon antes \
                             key(\"A\", \"minor\") o di la escala: scale(\"minor\")",
                        ));
                    }
                };
                let scale = api::scale(&name).ok_or_else(|| {
                    let names: Vec<&str> = api::SCALES.iter().map(|s| s.name).collect();
                    fail(
                        lua,
                        format!("scale(\"{name}\"): no existe ({})", names.join(", ")),
                    )
                })?;
                let tonic = tonic.unwrap_or_else(|| {
                    key.map_or("C", |k| k.tonic_name()).to_string()
                });
                let root = api::note(&tonic).ok_or_else(|| {
                    fail(
                        lua,
                        format!(
                            "scale(\"{name}\", \"{tonic}\"): tónica no válida (de C a B, con # o b: F#, Bb)"
                        ),
                    )
                })?;
                Ok(ScaleHandle {
                    intervals: scale.intervals,
                    root,
                })
            })?,
        )?;

        let st = state.clone();
        globals.set(
            "track",
            lua.create_function(move |lua, (name, source): (String, Option<AnyUserData>)| {
                guard(lua, "track")?;
                let def = source
                    .map(|source| track_source(lua, &name, &source))
                    .transpose()?;
                let mut builder = st.borrow_mut();
                match def {
                    Some(def) => {
                        // Una pista nueva en una sección: entre todas las
                        // secciones, como mucho MAX_TRACKS nombres.
                        let in_section = builder.in_section.get();
                        if in_section && !builder.names.contains(&name) {
                            if builder.names.len() >= MAX_TRACKS {
                                return Err(fail(
                                    lua,
                                    format!(
                                        "track(\"{name}\"): entre todas las secciones hay más de {MAX_TRACKS} pistas (el máximo)"
                                    ),
                                ));
                            }
                            builder.names.push(name.clone());
                        }
                        // Redefinir una pista en el mismo script la reemplaza.
                        match builder.tracks.iter_mut().find(|t| t.def.name == name) {
                            Some(existing) => *existing = def,
                            None => builder.tracks.push(def),
                        }
                    }
                    None if builder.tracks.iter().any(|t| t.def.name == name) => {}
                    None => {
                        return Err(fail(
                            lua,
                            format!(
                                "la pista \"{name}\" no existe; defínela con \
                                     track(\"{name}\", sample(...):slice(n))"
                            ),
                        ));
                    }
                }
                Ok(TrackHandle {
                    name,
                    state: st.clone(),
                })
            })?,
        )?;
        Ok(())
    }

    /// Asigna una pista del motor a cada nombre: los que siguen conservan la
    /// suya; los nuevos ocupan las libres.
    fn assign_slots(&self, names: &[&str]) -> [Option<String>; MAX_TRACKS] {
        let mut slots: [Option<String>; MAX_TRACKS] = Default::default();
        let mut new_names = Vec::new();
        for &name in names {
            match self.track_slot(name) {
                Some(slot) => slots[slot] = Some(name.to_string()),
                None => new_names.push(name.to_string()),
            }
        }
        for name in new_names {
            // `eval` ya comprobó que hay como mucho MAX_TRACKS pistas.
            let free = slots.iter().position(Option::is_none).unwrap();
            slots[free] = Some(name);
        }
        slots
    }
}

/// Lote que lleva la canción al motor: tempo y el patrón de cada pista en su
/// ranura (`slots[i]` es el nombre de la pista de la ranura `i`).
/// El lote que hace sonar `song`. `before`, lo que sonaba (para quitar sus
/// grupos si `song` ya no tiene): los grupos y las macros solo van si hacen
/// falta, para no gastar sitio en la cola de comandos.
fn song_batch(
    song: &Song,
    slots: &[Option<String>; MAX_TRACKS],
    before: Option<&Song>,
) -> Result<Batch, ScriptError> {
    let mut batch = Batch::new();
    match (song.tempo_ramp, song.bpm) {
        (Some(ramp), _) => {
            batch.set_tempo_ramp(ramp);
        }
        (None, Some(bpm)) => {
            batch.set_bpm(bpm);
        }
        (None, None) => {}
    }
    // Sin `meter()`, 4/4 (así quitarlo vuelve al compás de siempre).
    batch.set_meter(song.meter.unwrap_or(engine::Meter::COMMON));
    batch.set_buses(song.buses);
    if !song.groups.is_empty() || before.is_some_and(|b| !b.groups.is_empty()) {
        batch.set_groups(song_groups(song, slots)?);
    }
    if !song.macros.is_empty() {
        let mut macros = [0.0; engine::MAX_MACROS];
        for (value, def) in macros.iter_mut().zip(&song.macros) {
            *value = def.value;
        }
        batch.set_macros(macros);
    }
    if !song.timeline.is_empty() {
        // Con línea de tiempo, el motor lleva todas las secciones.
        let mut arrangement = engine::Arrangement::new();
        for section in &song.sections {
            let mut patterns = Vec::new();
            for track in &section.tracks {
                let slot = slots
                    .iter()
                    .position(|s| s.as_deref() == Some(&track.name))
                    .expect("cada pista tiene su ranura");
                patterns.push((
                    slot,
                    track_pattern(track, slots).map_err(|e| {
                        ScriptError::new(format!("sección \"{}\": {}", section.name, e.message))
                    })?,
                ));
            }
            arrangement
                .add_section(section_id(&section.name), patterns)
                .map_err(|e| ScriptError::new(format!("motor: {e}")))?;
        }
        for &(section, bars) in &song.timeline {
            arrangement
                .push(section, bars)
                .map_err(|e| ScriptError::new(format!("motor: {e}")))?;
        }
        batch.set_arrangement(Some(arrangement));
        return Ok(batch);
    }
    // Sin línea de tiempo, los patrones sueltos (y fuera el arreglo que
    // hubiera).
    batch.set_arrangement(None);
    for (slot, assigned) in slots.iter().enumerate() {
        let pattern = match assigned {
            Some(name) => {
                let track = song.tracks.iter().find(|t| &t.name == name).unwrap();
                Some(track_pattern(track, slots)?)
            }
            None => None,
        };
        batch.set_pattern(slot, pattern);
    }
    Ok(batch)
}

/// Los grupos de la canción para el motor: cada uno en su índice y cada
/// pista con ranura en el suyo.
fn song_groups(
    song: &Song,
    slots: &[Option<String>; MAX_TRACKS],
) -> Result<engine::Groups, ScriptError> {
    let mut groups = engine::Groups::new();
    for (g, def) in song.groups.iter().enumerate() {
        let mut fx = def.fx.clone();
        resolve_sidechains(&mut fx, &def.sidechains, slots);
        let group = engine::Group::new()
            .with_fx(&fx)
            .and_then(|g| g.with_automation(&def.automation))
            .and_then(|g| g.with_gain(def.gain))
            .and_then(|g| g.with_pan(def.pan))
            .and_then(|g| g.with_sends(def.sends))
            .and_then(|g| g.with_mix_automation(def.mix_curves))
            .map_err(|e| ScriptError::new(format!("grupo \"{}\": {e}", def.name)))?;
        groups.groups[g] = Some(group);
        for name in &def.tracks {
            if let Some(slot) = slots.iter().position(|s| s.as_deref() == Some(name)) {
                groups.member[slot] = Some(g);
            }
        }
    }
    Ok(groups)
}

/// Pone en cada EQ dinámico de `fx` la ranura de la pista que escucha.
fn resolve_sidechains(
    fx: &mut [Fx],
    sidechains: &[(usize, String)],
    slots: &[Option<String>; MAX_TRACKS],
) {
    for (index, name) in sidechains {
        let slot = slots
            .iter()
            .position(|s| s.as_deref() == Some(name.as_str()));
        if let (Some(Fx::DynEq(params)), Some(slot)) = (fx.get_mut(*index), slot) {
            params.sidechain = Some(slot);
        }
    }
}

/// Comprueba que las pistas de cada grupo y las que escuchan sus EQ
/// dinámicos existen (en alguna sección o variante).
fn check_groups(song: &Song) -> Result<(), ScriptError> {
    let mut known = song.track_names();
    for n in song.alternate.iter().flat_map(|b| b.track_names()) {
        if !known.contains(&n) {
            known.push(n);
        }
    }
    for group in &song.groups {
        let names = group.tracks.iter().map(|t| (t, "group")).chain(
            group
                .sidechains
                .iter()
                .map(|(_, t)| (t, "fx(\"dyneq\"): sidechain")),
        );
        for (name, what) in names {
            if !known.contains(&name.as_str()) {
                return Err(ScriptError::new(format!(
                    "grupo \"{}\": {what} \"{name}\" no es ninguna pista (hay: {})",
                    group.name,
                    known.join(", ")
                )));
            }
        }
    }
    Ok(())
}

/// Comprueba el sidechain de los EQ dinámicos de `tracks`: que cada pista
/// que se escucha exista y que nadie se escuche en círculo (a escucha a b y
/// b a a, directa o indirectamente).
fn check_sidechains<'a>(
    tracks: impl Iterator<Item = &'a TrackDef> + Clone,
) -> Result<(), ScriptError> {
    let mut names: Vec<&str> = Vec::new();
    for t in tracks.clone() {
        if !names.contains(&t.name.as_str()) {
            names.push(&t.name);
        }
    }
    let mut edges: Vec<(&str, &str)> = Vec::new();
    for t in tracks {
        for (_, key) in &t.sidechains {
            if !names.contains(&key.as_str()) {
                return Err(ScriptError::new(format!(
                    "pista \"{}\": fx(\"dyneq\"): sidechain \"{key}\" no es ninguna pista (hay: {})",
                    t.name,
                    names.join(", ")
                )));
            }
            if !edges.contains(&(t.name.as_str(), key.as_str())) {
                edges.push((&t.name, key));
            }
        }
    }
    // Un círculo: desde alguna pista se vuelve a ella siguiendo a quién
    // escucha.
    for &start in &names {
        let mut seen = vec![start];
        let mut frontier = vec![start];
        while let Some(at) = frontier.pop() {
            for &(from, to) in &edges {
                if from != at {
                    continue;
                }
                if to == start {
                    return Err(ScriptError::new(format!(
                        "pista \"{start}\": fx(\"dyneq\"): sidechain en círculo (\"{start}\" acaba escuchándose a sí misma a través de \"{}\")",
                        if at == start { to } else { at }
                    )));
                }
                if !seen.contains(&to) {
                    seen.push(to);
                    frontier.push(to);
                }
            }
        }
    }
    Ok(())
}

/// El patrón que manda al motor una pista (`slots[i]`: el nombre de la
/// pista de la ranura `i`, para el sidechain).
fn track_pattern(
    track: &TrackDef,
    slots: &[Option<String>; MAX_TRACKS],
) -> Result<Pattern, ScriptError> {
    let mut fx = track.fx.clone();
    resolve_sidechains(&mut fx, &track.sidechains, slots);
    let chains = track
        .slice_fx
        .iter()
        .enumerate()
        .map(|(i, c)| {
            if c.fx.is_empty() {
                return Ok(None);
            }
            SliceFx::new(&c.fx, &c.automation).map(Some).map_err(|e| {
                ScriptError::new(format!("pista \"{}\": corte {}: {e}", track.name, i + 1))
            })
        })
        .collect::<Result<Vec<_>, _>>()?;
    Pattern::from_bars(track.sample.clone(), &track.slices, &track.bars)
        .and_then(|p| p.with_gain(track.gain))
        .and_then(|p| p.with_fx(&fx))
        .and_then(|p| p.with_automation(&track.automation))
        .and_then(|p| p.with_sends(track.sends))
        .and_then(|p| p.with_pan(track.pan))
        .and_then(|p| p.with_mix_automation(track.mix_curves))
        .and_then(|p| p.with_shapes(&track.shapes))
        .and_then(|p| p.with_slice_fx(&chains))
        .and_then(|p| p.with_choke(track.choke))
        .and_then(|p| p.with_midi(track.midi))
        .and_then(|p| {
            p.with_instrument(
                track
                    .instrument
                    .filter(|i| i.synth.is_none())
                    .map(|i| i.engine),
            )
        })
        .and_then(|p| match track.instrument.and_then(|i| i.synth) {
            Some(s) => p.with_synth(Some(s.patch), s.curves),
            None => Ok(p),
        })
        .and_then(|p| p.with_granular(track.granular))
        .and_then(|p| p.with_formant(track.formant))
        .and_then(|p| p.with_formant_curve(track.formant_curve))
        // Un pitch fijo es una curva que no se mueve.
        .and_then(|p| {
            p.with_pitch_curve(track.pitch_curve.or((track.pitch != 0.0).then_some(
                engine::Curve::Ramp {
                    from: track.pitch,
                    to: track.pitch,
                    steps: 1.0,
                },
            )))
        })
        .map(|p| p.with_gate(track.gate))
        .map(|p| {
            if track.frozen.is_some() {
                p.with_raw()
            } else {
                p
            }
        })
        .map(|p| p.with_mute(track.mute).with_solo(track.solo))
        .and_then(|p| p.with_tag(track.tag))
        .map_err(|e| ScriptError::new(format!("pista \"{}\": {e}", track.name)))
}

/// Identificador de una sección entre recargas (FNV-1a de su nombre).
fn section_id(name: &str) -> u64 {
    name.bytes().fold(0xcbf2_9ce4_8422_2325u64, |h, b| {
        (h ^ b as u64).wrapping_mul(0x100_0000_01b3)
    })
}

impl Song {
    /// Compases que dura la canción entera hasta repetirse: el mínimo
    /// común múltiplo de los ciclos de sus pistas (vueltas de `every`/
    /// `chance` por su longitud, y el compás de 16 pasos), y al menos
    /// `min_bars` (redondeado a ciclos completos). Como mucho
    /// [`MAX_SONG_BARS`].
    pub fn length_bars(&self, min_bars: u64) -> u64 {
        // Con línea de tiempo, la canción entera (hasta el tope).
        if !self.timeline.is_empty() {
            return self.timeline_bars().min(MAX_SONG_BARS);
        }
        let bar = self.bar_steps() as u64;
        let steps = self.tracks.iter().fold(bar, |acc, t| {
            lcm(acc, (t.bars.len().max(1) * t.length.max(1)) as u64).min(MAX_SONG_BARS * bar + 1)
        });
        let cycle = steps.div_ceil(bar);
        if cycle > MAX_SONG_BARS {
            return MAX_SONG_BARS;
        }
        (cycle * min_bars.div_ceil(cycle).max(1)).min(MAX_SONG_BARS)
    }

    /// Compases de la línea de tiempo entera (0 sin `arrange`).
    pub fn timeline_bars(&self) -> u64 {
        self.timeline.iter().map(|&(_, b)| b as u64).sum()
    }

    /// Nombre de la sección que suena en el compás siguiente al de `place`
    /// (siguiendo la línea de tiempo, o repitiendo la entrada).
    pub fn section_after(&self, place: engine::SectionStatus) -> Option<&str> {
        let &(_, bars) = self.timeline.get(place.entry)?;
        let entry = if place.bar + 1 < bars || place.looping {
            place.entry
        } else {
            (place.entry + 1) % self.timeline.len()
        };
        self.section_at(entry)
    }

    /// Nombre de la sección de la entrada `entry` de la línea de tiempo.
    pub fn section_at(&self, entry: usize) -> Option<&str> {
        let &(section, _) = self.timeline.get(entry)?;
        Some(&self.sections[section].name)
    }

    /// Pistas de la sección `name` (sin secciones, las de la canción).
    pub fn section_tracks(&self, name: Option<&str>) -> &[TrackDef] {
        name.and_then(|n| self.sections.iter().find(|s| s.name == n))
            .map_or(&self.tracks, |s| &s.tracks)
    }

    /// Nombres de todas las pistas, de todas las secciones, sin repetir
    /// (cada uno ocupa una pista del motor).
    /// La definición de pista con esa etiqueta ([`TrackDef::tag`]): de las
    /// pistas, de las secciones o de la otra variante; `None` si es de
    /// otra carga (o 0).
    pub fn track_by_tag(&self, tag: u32) -> Option<&TrackDef> {
        if tag == 0 {
            return None;
        }
        self.tracks
            .iter()
            .chain(self.sections.iter().flat_map(|s| s.tracks.iter()))
            .find(|t| t.tag == tag)
            .or_else(|| self.alternate.as_ref()?.track_by_tag(tag))
    }

    pub fn track_names(&self) -> Vec<&str> {
        let mut names: Vec<&str> = Vec::new();
        let all = self
            .sections
            .iter()
            .flat_map(|s| s.tracks.iter())
            .chain(&self.tracks);
        for track in all {
            if !names.contains(&track.name.as_str()) {
                names.push(&track.name);
            }
        }
        names
    }

    /// Pasos de un compás (16 en 4/4; `meter()` lo cambia).
    pub fn bar_steps(&self) -> usize {
        self.meter.map_or(STEPS, |m| m.steps)
    }

    /// Tempo del compás `bar` (desde que arranca la canción): el de la
    /// rampa si la hay.
    pub fn bpm_at(&self, bar: u64) -> f64 {
        match self.tempo_ramp {
            Some(ramp) => ramp.bpm_at(bar),
            None => self.bpm.unwrap_or(engine::DEFAULT_BPM),
        }
    }

    /// Frames que duran los primeros `bars` compases (con la rampa y el
    /// compás de la canción), con el mismo redondeo que el motor.
    pub fn frames(&self, sample_rate: u32, bars: u64) -> u64 {
        let steps = self.bar_steps();
        (0..bars)
            .map(|b| engine::bar_frames_steps(steps, self.bpm_at(b), sample_rate))
            .sum()
    }

    /// Renderiza `bars` compases de la canción offline, en un motor aparte
    /// (no toca el que está sonando). Estéreo entrelazado a `sample_rate`.
    pub fn render(&self, sample_rate: u32, bars: u64) -> Result<Vec<f32>, ScriptError> {
        self.render_with_metronome(sample_rate, bars, false)
    }

    /// Como [`Song::render`], con el clic del metrónomo si `metronome`
    /// (el `metronome()` del script no cuenta: el render solo lo lleva si
    /// se pide).
    pub fn render_with_metronome(
        &self,
        sample_rate: u32,
        bars: u64,
        metronome: bool,
    ) -> Result<Vec<f32>, ScriptError> {
        self.render_all(sample_rate, bars, metronome)
    }

    /// Renderiza solo la pista `name` (remuestreo): como si estuviera en
    /// solo (y sin mute), con sus efectos, su volumen, sus envíos y lo que
    /// devuelven los buses, y el limitador del master. `None` si no existe.
    pub fn render_track(
        &self,
        sample_rate: u32,
        bars: u64,
        name: &str,
    ) -> Option<Result<Vec<f32>, ScriptError>> {
        if !self.track_names().contains(&name) {
            return None;
        }
        let mut song = self.clone();
        let all = song
            .tracks
            .iter_mut()
            .chain(song.sections.iter_mut().flat_map(|s| s.tracks.iter_mut()));
        for track in all {
            track.solo = track.name == name;
            if track.solo {
                track.mute = false;
            }
        }
        Some(song.render_all(sample_rate, bars, false))
    }

    /// Compases que tarda la pista `name` en repetirse (su ciclo de
    /// vueltas con su longitud, en compases enteros), como mucho
    /// [`MAX_SONG_BARS`].
    pub fn track_length_bars(&self, name: &str) -> Option<u64> {
        let track = self.tracks.iter().find(|t| t.name == name)?;
        let bar = self.bar_steps() as u64;
        let steps = lcm(bar, (track.bars.len().max(1) * track.length.max(1)) as u64);
        Some(steps.div_ceil(bar).min(MAX_SONG_BARS))
    }

    fn render_all(
        &self,
        sample_rate: u32,
        bars: u64,
        metronome: bool,
    ) -> Result<Vec<f32>, ScriptError> {
        let mut slots: [Option<String>; MAX_TRACKS] = Default::default();
        for (slot, name) in slots.iter_mut().zip(self.track_names()) {
            *slot = Some(name.to_string());
        }
        let (mut controller, mut engine) = engine::new_engine(sample_rate);
        let send = |e: SendError| ScriptError::new(format!("motor: {e}"));
        controller
            .apply(song_batch(self, &slots, None)?)
            .map_err(send)?;
        if metronome {
            controller.set_metronome(true).map_err(send)?;
        }
        controller.start().map_err(send)?;
        let frames = self.frames(sample_rate, bars);
        Ok(engine::offline::render(&mut engine, frames as usize, 2))
    }
}

type SampleCache = HashMap<PathBuf, (Option<SystemTime>, Arc<Sample>)>;

/// Carga un WAV o lo reutiliza de la caché si no ha cambiado en disco.
fn load_cached(
    cache: &mut SampleCache,
    path: &Path,
    rate: u32,
) -> Result<Arc<Sample>, engine::LoadError> {
    let modified = std::fs::metadata(path).and_then(|m| m.modified()).ok();
    if let Some((when, sample)) = cache.get(path)
        && *when == modified
        && modified.is_some()
    {
        return Ok(sample.clone());
    }
    let sample = Arc::new(Sample::load_wav(path, rate)?);
    cache.insert(path.to_path_buf(), (modified, sample.clone()));
    Ok(sample)
}

/// Error de una función de la API con la posición del script que la llamó.
fn fail(lua: &Lua, message: impl fmt::Display) -> mlua::Error {
    mlua::Error::runtime(format!("{}{message}", location(lua)))
}

/// Dónde está el script que llamó a la función de la API (`archivo:línea:
/// `), o nada.
fn location(lua: &Lua) -> String {
    let location = (1..8).find_map(|level| {
        lua.inspect_stack(level, |debug| {
            let source = debug.source();
            if source.what == "C" {
                return None;
            }
            let file = source.short_src.map(|s| s.into_owned())?;
            let line = debug.current_line()?;
            Some(format!("{file}:{line}: "))
        })
        .flatten()
    });
    location.unwrap_or_default()
}

/// Una sección evaluada: su nombre y sus pistas sin expandir.
type BuiltSection = (String, Vec<TrackBuild>);

/// Evalúa las secciones y las variantes (al acabar el script). Sin
/// variantes, una versión; con ellas, la A y la B. En cada una: las
/// variantes de pista cambian las pistas de fuera (su función recibe la
/// pista), y cada sección es una copia de ellas que cambian su función y
/// la de su variante.
fn run_bodies(state: &Rc<RefCell<Builder>>) -> mlua::Result<Vec<BuiltVersion>> {
    let (sections, variants, main, in_section) = {
        let mut builder = state.borrow_mut();
        builder.names = builder.tracks.iter().map(|t| t.def.name.clone()).collect();
        (
            std::mem::take(&mut builder.sections),
            std::mem::take(&mut builder.variants),
            builder.tracks.clone(),
            builder.in_section.clone(),
        )
    };
    // De qué es cada variante: una sección (si se llama así) o una pista.
    for v in &variants {
        let section = sections.iter().any(|s| s.name == v.target);
        let track = main.iter().any(|t| t.def.name == v.target);
        if !(section || track) {
            return Err(mlua::Error::runtime(format!(
                "{}variant: no hay ninguna sección ni pista \"{}\" (fuera de las secciones)",
                v.location, v.target
            )));
        }
    }
    let is_section = |v: &VariantBuild| sections.iter().any(|s| s.name == v.target);
    let letters: &[Option<char>] = if variants.is_empty() {
        &[None]
    } else {
        &[Some('A'), Some('B')]
    };
    let call = |tracks: Vec<TrackBuild>, run: &dyn Fn() -> mlua::Result<()>| {
        state.borrow_mut().tracks = tracks;
        in_section.set(true);
        let result = run();
        in_section.set(false);
        result.map(|()| std::mem::take(&mut state.borrow_mut().tracks))
    };
    let mut versions = Vec::with_capacity(letters.len());
    for &letter in letters {
        let mine = |v: &&VariantBuild| Some(v.letter) == letter;
        let mut base = main.clone();
        for v in variants.iter().filter(mine).filter(|v| !is_section(v)) {
            let handle = TrackHandle {
                name: v.target.clone(),
                state: state.clone(),
            };
            base = call(base, &|| v.func.call::<()>(handle.clone()))?;
        }
        let built = if sections.is_empty() {
            None
        } else {
            let mut built = Vec::with_capacity(sections.len());
            for section in &sections {
                let mut tracks = call(base.clone(), &|| section.func.call::<()>(()))?;
                for v in variants
                    .iter()
                    .filter(mine)
                    .filter(|v| v.target == section.name)
                {
                    tracks = call(tracks, &|| v.func.call::<()>(()))?;
                }
                built.push((section.name.clone(), tracks));
            }
            Some(built)
        };
        versions.push(BuiltVersion {
            letter,
            base,
            sections: built,
        });
    }
    state.borrow_mut().tracks = main;
    Ok(versions)
}

/// La pista que define `track(nombre, origen)`: unos cortes
/// (`sample(...):slice(n)`) o un instrumento (`sample(...):instrument()`).
fn track_source(lua: &Lua, name: &str, source: &AnyUserData) -> mlua::Result<TrackBuild> {
    let def = |sample: &Arc<Sample>, path: &Path, slices: Slices, origin: SliceOrigin| TrackDef {
        name: name.to_string(),
        sample: sample.clone(),
        path: path.to_path_buf(),
        adjusted: vec![false; slices.len()],
        slices,
        length: STEPS,
        steps: vec![None; STEPS],
        bars: Vec::new(),
        gain: 1.0,
        fx: Vec::new(),
        labels: Vec::new(),
        swing: STRAIGHT_SWING,
        humanize: None,
        sends: [0.0; engine::bus::BUSES],
        pan: 0.0,
        mix_curves: [None; engine::MIX_PARAMS],
        pitch: 0.0,
        pitch_curve: None,
        formant_curve: None,
        automation: Vec::new(),
        shapes: Vec::new(),
        gate: false,
        choke: None,
        midi: None,
        mute: false,
        solo: false,
        origin,
        stretch: None,
        instrument: None,
        granular: None,
        formant: None,
        sidechains: Vec::new(),
        slice_fx: Vec::new(),
        tag: 0,
        spans: Vec::new(),
        frozen: None,
    };
    if let Ok(source) = source.borrow::<SlicedHandle>() {
        let mut def = def(
            &source.sample,
            &source.path,
            source.slices.clone(),
            source.origin.clone(),
        );
        def.adjusted = source.adjusted.clone();
        def.stretch = source.stretch.map(|source_bpm| Stretch {
            source_bpm,
            ratio: 1.0,
        });
        return Ok(TrackBuild {
            def,
            labels: source.labels.clone(),
            stale: source.stale.clone(),
            shapes: source.shapes.clone(),
            slice_fx: source.slice_fx.clone(),
            transforms: Vec::new(),
            steps_set: false,
            written: Vec::new(),
            steps_at: String::new(),
            melodic: Rc::default(),
        });
    }
    if let Ok(source) = source.borrow::<InstrumentHandle>() {
        // Un solo corte: el sample entero, que no se etiqueta ni se mueve.
        let origin = SliceOrigin {
            sample: source.name.clone(),
            count: 1,
            grid: true,
            indices: vec![None],
            zero_cross: None,
        };
        let slices = Slices::grid(&source.sample, 1);
        let mut def = def(&source.sample, &source.path, slices, origin);
        def.instrument = Some(source.def);
        return Ok(TrackBuild {
            def,
            labels: Rc::new(RefCell::new(vec![None])),
            stale: Rc::new(Vec::new()),
            shapes: Rc::new(RefCell::new(vec![DEFAULT_SHAPE])),
            slice_fx: Rc::default(),
            transforms: Vec::new(),
            steps_set: false,
            written: Vec::new(),
            steps_at: String::new(),
            melodic: Rc::default(),
        });
    }
    if let Ok(source) = source.borrow::<SynthHandle>() {
        // Un frame de silencio: el sample no suena, pero la pista es como
        // la de un instrumento (un solo corte).
        let silence = Arc::new(Sample::silence(source.sample_rate));
        let origin = SliceOrigin {
            sample: format!("synth(\"{}\")", source.def.synth.map_or("", |s| s.kind)),
            count: 1,
            grid: true,
            indices: vec![None],
            zero_cross: None,
        };
        let slices = Slices::grid(&silence, 1);
        let mut def = def(&silence, Path::new(""), slices, origin);
        let mut instrument = source.def;
        if let Some(synth) = &mut instrument.synth {
            synth.patch.seed = synth.seed.unwrap_or_else(|| name_seed(name));
        }
        def.instrument = Some(instrument);
        return Ok(TrackBuild {
            def,
            labels: Rc::new(RefCell::new(vec![None])),
            stale: Rc::new(Vec::new()),
            shapes: Rc::new(RefCell::new(vec![DEFAULT_SHAPE])),
            slice_fx: Rc::default(),
            transforms: Vec::new(),
            steps_set: false,
            written: Vec::new(),
            steps_at: String::new(),
            melodic: Rc::default(),
        });
    }
    Err(fail(
        lua,
        format!(
            "track(\"{name}\", ...): el segundo argumento son cortes \
             (sample(...):slice(n)), un instrumento (sample(...):instrument{{...}}) \
             o un sintetizador (synth(\"acid\", {{...}}))"
        ),
    ))
}

/// Semilla de una pista sacada de su nombre (FNV-1a).
fn name_seed(name: &str) -> u64 {
    name.bytes().fold(0xcbf2_9ce4_8422_2325u64, |h, b| {
        (h ^ b as u64).wrapping_mul(0x100_0000_01b3)
    })
}

struct Builder {
    bpm: Option<f64>,
    metronome: Option<bool>,
    buses: Buses,
    tempo_ramp: Option<engine::TempoRamp>,
    meter: Option<engine::Meter>,
    tracks: Vec<TrackBuild>,
    /// Activo mientras se llaman las funciones de `every`.
    expanding: Rc<Cell<bool>>,
    /// Secciones por evaluar (`section()`), con dónde se definieron.
    sections: Vec<SectionBuild>,
    /// `arrange()`: (sección, compases) y dónde se llamó.
    arrange: Option<(Vec<(String, u32)>, String)>,
    /// Activo mientras se evalúa una sección.
    in_section: Rc<Cell<bool>>,
    /// Nombres de pista de todas las secciones (no pueden pasar de
    /// `MAX_TRACKS`: cada uno ocupa una pista del motor).
    names: Vec<String>,
    /// Variantes por evaluar (`variant()`).
    variants: Vec<VariantBuild>,
    /// `priority()`: las pistas de la que más manda a la que menos.
    priority: Vec<String>,
    /// Grupos (`group()`) y macros (`macro()`, `macro_init()`).
    groups: Vec<GroupDef>,
    macros: Vec<MacroDef>,
    /// `key()` y dónde se llamó.
    key: Option<(SongKey, String)>,
    /// `transpose()` y dónde se llamó.
    transpose: Option<(i32, String)>,
    /// Dónde se usó por primera vez la tonalidad por defecto (`scale()`
    /// sin tónica o un grado en `melody()`): `key()` tiene que ir antes.
    key_used: Option<String>,
    /// Pistas congeladas (`frozen()`).
    frozen: Vec<FreezeRequest>,
}

/// `frozen(pista, wav [, huella])`, por aplicar al acabar el script.
struct FreezeRequest {
    track: String,
    wav: String,
    path: PathBuf,
    sample: Arc<Sample>,
    fingerprint: Option<String>,
    location: String,
}

impl Builder {
    /// El índice de la macro `name` (la añade si es nueva).
    fn macro_index(&mut self, name: &str) -> Result<usize, String> {
        if let Some(i) = self.macros.iter().position(|m| m.name == name) {
            return Ok(i);
        }
        if name.trim().is_empty() {
            return Err("la macro necesita un nombre".into());
        }
        if self.macros.len() >= engine::MAX_MACROS {
            return Err(format!(
                "hay {} macros como mucho ({})",
                engine::MAX_MACROS,
                self.macros
                    .iter()
                    .map(|m| m.name.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            ));
        }
        self.macros.push(MacroDef {
            name: name.to_string(),
            value: 0.0,
        });
        Ok(self.macros.len() - 1)
    }
}

/// `variant(nombre, letra, función)`: lo que cambia una sección o una
/// pista en la variante A o B.
struct VariantBuild {
    target: String,
    letter: char,
    func: Function,
    /// Dónde se llamó (para los errores que salen después).
    location: String,
}

/// Una versión de la canción ya evaluada, sin expandir: sin variantes hay
/// una (`letter` = `None`); con ellas, la A y la B.
struct BuiltVersion {
    letter: Option<char>,
    /// Las pistas de fuera de las secciones.
    base: Vec<TrackBuild>,
    sections: Option<Vec<BuiltSection>>,
}

/// `section(nombre, función)`: se evalúa al acabar el script, sobre una
/// copia de las pistas de fuera de las secciones.
struct SectionBuild {
    name: String,
    func: Function,
}

/// Pista en construcción: su definición y las transformaciones pendientes.
#[derive(Clone)]
struct TrackBuild {
    def: TrackDef,
    /// Etiquetas de los cortes, compartidas con la variable de Lua: un
    /// `:name()` posterior a `track()` vale para los `steps` siguientes.
    labels: Rc<RefCell<Vec<Option<String>>>>,
    /// Etiquetas del proyecto de otro `slice(n)` (para explicar errores).
    stale: Rc<Vec<(String, Cut)>>,
    /// Forma de los cortes, compartida con la variable como las etiquetas.
    shapes: Rc<RefCell<Vec<SliceShape>>>,
    /// Efectos de los cortes, compartidos con la variable como las formas.
    slice_fx: Rc<RefCell<Vec<SliceFxDef>>>,
    transforms: Vec<Transform>,
    /// Ya se llamó a `:steps()` (luego `:length()` no puede cambiar la
    /// longitud).
    steps_set: bool,
    /// Dónde está escrito cada paso de `:steps()` (ver [`written`]).
    written: Vec<Option<TokenSpan>>,
    /// Archivo y línea del último `:steps()` (`song.lua:3: `): para los
    /// errores que solo se ven al acabar el script (los efectos de un paso
    /// que no caben con los de su corte, si el corte los recibe después).
    steps_at: String,
    /// Tiene notas de `melody()` (en una pista de cortes): `transpose()`
    /// las mueve, como las de un instrumento.
    melodic: Rc<Cell<bool>>,
}

#[derive(Clone)]
enum Transform {
    /// `:every(n, fn)`: en los compases 0, n, 2n... se llama `fn(compás)`.
    Every { n: u64, func: Function },
    /// `:chance(p, efecto)`: cada golpe sufre el efecto con probabilidad `p`.
    Chance { p: f64, effect: Chance },
    /// `:fill(efecto, n, pasos)`: los últimos `steps` pasos del último compás
    /// de cada `n`.
    Fill { n: u64, steps: usize, effect: Fill },
}

#[derive(Clone, Copy)]
enum Fill {
    /// El último golpe antes del relleno, repetido en cada paso con ese
    /// retrigger.
    Stutter(u8),
    /// Como stutter, con retrigger creciente: 2, 4, 8, 16.
    Roll,
    Reverse,
    /// Suma semitonos a los golpes.
    Pitch(f32),
    Mute,
}

/// Aplica un relleno a los últimos `count` pasos de una vuelta (todos si
/// la vuelta es más corta).
fn apply_fill(bar: &mut Bar, count: usize, effect: Fill) {
    let start = bar.len() - count.min(bar.len());
    // Golpe que se repite en stutter y roll: el primero del relleno o, si es
    // silencio, el último antes; si no hay, el primero del compás.
    let source = bar[..=start]
        .iter()
        .rev()
        .flatten()
        .next()
        .or_else(|| bar.iter().flatten().next())
        .copied();
    for (j, slot) in bar[start..].iter_mut().enumerate() {
        match effect {
            Fill::Stutter(n) => *slot = source.map(|s| s.with_retrig(n)),
            Fill::Roll => *slot = source.map(|s| s.with_retrig((2u8 << j.min(3)).min(MAX_RETRIG))),
            Fill::Reverse => {
                if let Some(step) = slot {
                    step.reverse = !step.reverse;
                }
            }
            Fill::Pitch(st) => {
                if let Some(step) = slot {
                    step.semitones = (step.semitones + st).clamp(-MAX_SEMITONES, MAX_SEMITONES);
                }
            }
            Fill::Mute => *slot = None,
        }
    }
}

#[derive(Clone, Copy)]
enum Chance {
    Retrig(u8),
    Reverse,
    Pitch(f32),
    Gain(f32),
    Mute,
    Accent,
    Slide,
}

/// Expande `every` y `chance` de una pista a su ciclo de compases.
/// Lo que necesita `expand` de la canción para humanize: el tempo (los ms
/// se pasan a pasos con él) y la semilla de la carga.
#[derive(Clone, Copy)]
struct Timing {
    bpm: f64,
    seed: u64,
    /// La tonalidad (para los grados de `c:melody()` en `every`) y los
    /// semitonos de `transpose()`.
    key: Option<patterns::Key>,
    transpose: i32,
}

fn expand(track: TrackBuild, rng: &mut Rng, timing: Timing) -> Result<TrackDef, ScriptError> {
    let TrackBuild {
        mut def,
        labels,
        stale,
        shapes,
        slice_fx,
        transforms,
        steps_set: _,
        written,
        steps_at,
        melodic,
    } = track;
    def.labels = labels.borrow().clone();
    def.shapes = shapes.borrow().clone();
    def.slice_fx = slice_fx.borrow().clone();
    // Humanize cambia cada vuelta: el ciclo es de 16 como con `chance`.
    let mut cycle = if def.humanize.is_some() {
        CHANCE_CYCLE_BARS
    } else {
        1u64
    };
    // El swing va en los contratiempos de la rejilla de la canción: con una
    // longitud impar, una vuelta empieza en tiempo y la siguiente en
    // contratiempo, así que hacen falta dos variantes.
    let swing = swing_delay(def.swing);
    if swing > 0.0 && def.length % 2 == 1 {
        cycle = lcm(cycle, 2);
    }
    for transform in &transforms {
        let n = match transform {
            Transform::Every { n, .. } | Transform::Fill { n, .. } => *n,
            Transform::Chance { .. } => CHANCE_CYCLE_BARS,
        };
        cycle = lcm(cycle, n);
        if cycle > MAX_CYCLE_BARS as u64 {
            return Err(ScriptError::new(format!(
                "pista \"{}\": every/chance/fill dan un ciclo de {cycle} compases (máximo {MAX_CYCLE_BARS})",
                def.name
            )));
        }
    }

    let mut bars = Vec::with_capacity(cycle as usize);
    let mut spans = Vec::with_capacity(cycle as usize);
    for b in 0..cycle {
        let mut steps = def.steps.clone();
        let mut lap_spans = written.clone();
        for transform in &transforms {
            match transform {
                Transform::Every { n, func } if b % n == 0 => {
                    let bar = BarHandle {
                        steps: Rc::new(RefCell::new(steps.clone())),
                        length: def.length,
                        labels: Rc::new(def.labels.clone()),
                        stale: stale.clone(),
                        count: def.origin.cut(),
                        track: def.name.clone(),
                        root: def.instrument.map_or(60, |i| i.root),
                        key: timing.key,
                        melodic: melodic.clone(),
                        plays: Plays::of(&def),
                        written: Rc::new(RefCell::new(lap_spans.clone())),
                    };
                    let (shared, shared_spans) = (bar.steps.clone(), bar.written.clone());
                    func.call::<()>(bar)
                        .map_err(|e| ScriptError::from_lua(&e))?;
                    steps = shared.borrow().clone();
                    lap_spans = shared_spans.borrow().clone();
                }
                Transform::Every { .. } => {}
                Transform::Fill {
                    n,
                    steps: count,
                    effect,
                } if (b + 1) % n == 0 => {
                    apply_fill(&mut steps, *count, *effect);
                }
                Transform::Fill { .. } => {}
                Transform::Chance { p, effect } => {
                    for slot in &mut steps {
                        if slot.is_some() && rng.next_f64() < *p {
                            apply_chance(slot, *effect);
                        }
                    }
                }
            }
        }
        bars.push(steps);
        spans.push(lap_spans);
    }
    // Swing: los contratiempos de semicorchea de la canción (pasos 2, 4,
    // 6... desde 1 en cada compás) se retrasan.
    if swing > 0.0 {
        let length = def.length;
        for (lap, bar) in bars.iter_mut().enumerate() {
            for (i, slot) in bar.iter_mut().enumerate() {
                if let Some(step) = slot
                    && (lap * length + i) % 2 == 1
                {
                    step.delay = (step.delay + swing).min(engine::MAX_DELAY);
                }
            }
        }
    }
    if let Some(h) = def.humanize {
        humanize(&mut bars, h, &def.name, timing);
    }
    // Un corte por paso (`~`) necesita un filtro en la pista.
    let locked = bars.iter().flatten().flatten().any(|s| s.lock.is_some());
    if locked && !def.fx.iter().any(|f| matches!(f, Fx::Filter { .. })) {
        return Err(ScriptError::new(format!(
            "pista \"{}\": ~ (corte por paso) necesita un filtro en la pista, \
             por ejemplo :fx(\"lowpass\", {{ cutoff = 3000 }})",
            def.name
        )));
    }
    // `transpose()`: las notas de un instrumento o un sintetizador, y las
    // de `melody()` en una pista de cortes.
    if timing.transpose != 0 && (def.instrument.is_some() || melodic.get()) {
        let t = timing.transpose as f32;
        for step in bars.iter_mut().chain([&mut def.steps]).flatten().flatten() {
            step.semitones += t;
            let highest = step
                .chord
                .iter()
                .flatten()
                .fold(0.0f32, |m, c| m.max(c.abs()));
            if step.semitones.abs() + highest > engine::MAX_SEMITONES {
                return Err(ScriptError::new(format!(
                    "pista \"{}\": transpose({}): una nota queda a {} semitonos de la raíz (máximo ±{})",
                    def.name,
                    timing.transpose,
                    step.semitones + highest.copysign(step.semitones),
                    engine::MAX_SEMITONES
                )));
            }
        }
    }
    def.bars = bars;
    def.spans = spans;
    step_fx_fit(&def).map_err(|e| ScriptError::new(format!("{steps_at}steps: {e}")))?;
    Ok(def)
}

/// Aplica humanize a los compases del ciclo: a cada golpe, un retraso al
/// azar de 0 a `ms` (en pasos al tempo de la canción, sumado al que tenga,
/// con tope `MAX_DELAY`) y un volumen de `1 - volumen` a 1 veces el suyo.
fn humanize(bars: &mut [Bar], h: Humanize, track: &str, timing: Timing) {
    let seed = h.seed.unwrap_or_else(|| {
        // FNV-1a del nombre: cada pista con su azar, fijo con la semilla.
        let hash = track.bytes().fold(0xcbf2_9ce4_8422_2325u64, |h, b| {
            (h ^ b as u64).wrapping_mul(0x0100_0000_01b3)
        });
        timing.seed ^ hash ^ HUMANIZE_SEED
    });
    let mut rng = Rng::new(seed);
    let step_seconds = 60.0 / (timing.bpm * 4.0);
    let max_steps = (h.ms as f64 / 1000.0 / step_seconds) as f32;
    for bar in bars {
        for step in bar.iter_mut().flatten() {
            let delay = rng.next_f64() as f32 * max_steps;
            let gain = 1.0 - h.volume * rng.next_f64() as f32;
            step.delay = (step.delay + delay).min(engine::MAX_DELAY);
            step.gain *= gain;
        }
    }
}

/// Retraso (en pasos) de los contratiempos con un swing en porcentaje
/// estilo MPC: el contratiempo cae en ese porcentaje del par de
/// semicorcheas (50 = recto, 66 ≈ tresillo, 75 = máximo).
fn swing_delay(percent: f32) -> f32 {
    (2.0 * percent / 100.0 - 1.0).max(0.0)
}

fn apply_chance(slot: &mut Option<Step>, effect: Chance) {
    let Some(step) = slot else { return };
    match effect {
        Chance::Retrig(n) => step.retrig = n,
        Chance::Reverse => step.reverse = !step.reverse,
        Chance::Pitch(st) => step.semitones = st,
        Chance::Gain(g) => step.gain = g,
        Chance::Mute => *slot = None,
        Chance::Accent => step.accent = true,
        Chance::Slide => step.slide = true,
    }
}

fn lcm(a: u64, b: u64) -> u64 {
    fn gcd(a: u64, b: u64) -> u64 {
        if b == 0 { a } else { gcd(b, a % b) }
    }
    a / gcd(a, b) * b
}

/// Separación mínima de un corte ajustado a sus vecinos: 1 ms.
fn min_slice_gap(sample: &Sample) -> usize {
    (sample.sample_rate() as usize / 1000).max(1)
}

/// Frames de `zero_cross` (ms) en `sample`.
fn zero_frames(ms: f64, sample: &Sample) -> usize {
    ms_to_frame(ms, sample)
}

/// Lee las opciones de `slice(n, { ... })` y `slice_grid(n, { ... })`
/// (claves del registro `api::CUT_PARAMS`): el `zero_cross` en ms.
fn cut_options(lua: &Lua, who: &str, table: Option<&Table>) -> mlua::Result<Option<f64>> {
    let Some(table) = table else {
        return Ok(None);
    };
    table_curves(lua, who, table, api::CUT_PARAMS)?;
    let mut zero = None;
    for pair in table.pairs::<Value, Value>() {
        let (key, value) = pair?;
        let key = match key {
            Value::String(s) => s.to_str()?.to_string(),
            other => format!("{other:?}"),
        };
        if key != "zero_cross" {
            let known: Vec<&str> = api::CUT_PARAMS.iter().map(|p| p.name).collect();
            return Err(fail(
                lua,
                format!(
                    "{who}: opción desconocida «{key}» (valen: {})",
                    known.join(", ")
                ),
            ));
        }
        zero = match value {
            Value::Boolean(true) => Some(DEFAULT_ZERO_CROSS_MS),
            Value::Boolean(false) => None,
            Value::Integer(i) if (0..=50).contains(&i) => (i > 0).then_some(i as f64),
            Value::Number(n) if (0.0..=50.0).contains(&n) => (n > 0.0).then_some(n),
            other => {
                return Err(fail(
                    lua,
                    format!(
                        "{who}: zero_cross es true, false o los ms de 0 a 50 (no {})",
                        value_text(&other)
                    ),
                ));
            }
        };
    }
    Ok(zero)
}

/// `zero_cross = true`: hasta 2 ms (medio ciclo de 250 Hz).
const DEFAULT_ZERO_CROSS_MS: f64 = 2.0;

fn ms_to_frame(ms: f64, sample: &Sample) -> usize {
    (ms / 1000.0 * sample.sample_rate() as f64).round() as usize
}

fn frame_to_ms(frame: usize, sample: &Sample) -> f64 {
    frame as f64 * 1000.0 / sample.sample_rate() as f64
}

/// `scale("minor", "C")` en Lua.
struct ScaleHandle {
    intervals: &'static [i32],
    /// Tónica, en semitonos sobre do.
    root: i32,
}

impl UserData for ScaleHandle {
    fn add_methods<M: UserDataMethods<Self>>(methods: &mut M) {
        methods.add_method("pitch", |lua, this, degree: i64| {
            patterns::scale_pitch(this.intervals, this.root, degree)
                .map_err(|e| fail(lua, format!("pitch({degree}): {e}")))
        });
        methods.add_method(
            "melody",
            |lua, this, (degrees, hit): (String, Option<String>)| {
                patterns::melody(
                    this.intervals,
                    this.root,
                    &degrees,
                    hit.as_deref().unwrap_or("1"),
                )
                .map_err(|e| fail(lua, e))
            },
        );
    }
}

/// `sample("archivo.wav")` en Lua.
#[derive(Clone)]
struct SampleHandle {
    /// El nombre tal como se escribió (clave de las etiquetas del proyecto).
    name: String,
    path: PathBuf,
    sample: Arc<Sample>,
    /// Etiquetas de corte del proyecto.
    labels: Rc<Vec<SliceLabel>>,
    /// Ajustes manuales de cortes del proyecto.
    positions: Rc<Vec<SlicePosition>>,
    /// Avisos de la carga.
    warnings: Rc<RefCell<Vec<String>>>,
    /// Tempo de origen de `:stretch()`: se estira al de la canción.
    stretch: Option<f64>,
    /// Cortes añadidos y quitados en el proyecto.
    edits: Rc<Vec<SliceEdit>>,
}

impl UserData for SampleHandle {
    fn add_methods<M: UserDataMethods<Self>>(methods: &mut M) {
        // `:stretch([bpm_origen])`: al cargar, el sample se estira de ese
        // tempo (o el que se detecte) al de la canción, sin cambiar el tono.
        methods.add_method("stretch", |lua, this, bpm: Option<f64>| {
            let source = match bpm {
                Some(bpm) if valid_bpm(bpm) => bpm,
                Some(bpm) => {
                    return Err(fail(
                        lua,
                        format!(
                            "stretch({bpm}): el tempo de origen va de {} a {}",
                            engine::MIN_BPM,
                            engine::MAX_BPM
                        ),
                    ));
                }
                None => engine::detect_bpm(&this.sample)
                    .map(|g| g.rounded())
                    .ok_or_else(|| {
                        fail(
                            lua,
                            format!(
                                "stretch(): no se detecta el tempo de {}; pásalo: stretch(bpm)",
                                this.name
                            ),
                        )
                    })?,
            };
            Ok(SampleHandle {
                stretch: Some(source),
                ..this.clone()
            })
        });

        // `:instrument({ root, loop, voices, adsr, crossfade })`: el sample se
        // toca como notas (ver `engine::instrument`).
        methods.add_method("instrument", |lua, this, table: Option<Table>| {
            if this.stretch.is_some() {
                return Err(fail(
                    lua,
                    format!(
                        "instrument: {} está estirado (stretch); un instrumento toca el sample tal cual",
                        this.name
                    ),
                ));
            }
            let def = instrument_options(lua, this, table.as_ref())?;
            Ok(InstrumentHandle {
                name: this.name.clone(),
                path: this.path.clone(),
                sample: this.sample.clone(),
                def,
            })
        });

        // `:detect_bpm()`: tempo y compases del loop (o nil si no se ve).
        methods.add_method("detect_bpm", |_, this, ()| {
            Ok(engine::detect_bpm(&this.sample)
                .map(|g| {
                    // Entero de Lua si sale entero (`140`, no `140.0`).
                    let bpm = g.rounded();
                    let bpm = if bpm.fract() == 0.0 {
                        Value::Integer(bpm as i64)
                    } else {
                        Value::Number(bpm)
                    };
                    (bpm, g.bars)
                })
                .unzip())
        });

        // `:slice(n [, { zero_cross }])`: corta en los `n` transitorios más
        // fuertes.
        methods.add_method("slice", |lua, this, (n, options): (i64, Option<Table>)| {
            if !(1..=64).contains(&n) {
                return Err(fail(
                    lua,
                    format!("slice({n}): el número de cortes va de 1 a 64"),
                ));
            }
            let zero = cut_options(lua, &format!("slice({n})"), options.as_ref())?;
            let slices = Slices::detect(&this.sample, n as usize);
            cut_sample(lua, this, slices, Cut::detect(n as usize), zero)
        });

        // `:slice_grid(n [, { zero_cross }])`: `n` cortes iguales (rejilla).
        methods.add_method(
            "slice_grid",
            |lua, this, (n, options): (i64, Option<Table>)| {
                if !(1..=64).contains(&n) {
                    return Err(fail(
                        lua,
                        format!("slice_grid({n}): el número de cortes va de 1 a 64"),
                    ));
                }
                let zero = cut_options(lua, &format!("slice_grid({n})"), options.as_ref())?;
                let slices = Slices::grid(&this.sample, n as usize);
                cut_sample(lua, this, slices, Cut::grid(n as usize), zero)
            },
        );
    }
}

/// `sample(...):instrument({ ... })` en Lua.
struct InstrumentHandle {
    name: String,
    path: PathBuf,
    sample: Arc<Sample>,
    def: InstrumentDef,
}

impl UserData for InstrumentHandle {
    fn add_methods<M: UserDataMethods<Self>>(methods: &mut M) {
        // Un instrumento es un solo corte: sus efectos van en la pista.
        methods.add_method("fx", |lua, _, _: mlua::MultiValue| -> mlua::Result<()> {
            Err(fail(
                lua,
                "fx: un instrumento no tiene efectos por corte (es un solo corte); \
                 ponlos en la pista: track(...):fx(nombre, { ... })",
            ))
        });
    }
}

/// `synth(tipo, { ... })`: un sintetizador, que se pasa a `track()`.
struct SynthHandle {
    def: InstrumentDef,
    sample_rate: u32,
}

impl UserData for SynthHandle {
    fn add_methods<M: UserDataMethods<Self>>(methods: &mut M) {
        methods.add_method("fx", |lua, _, _: mlua::MultiValue| -> mlua::Result<()> {
            Err(fail(
                lua,
                "fx: los efectos de un sintetizador van en la pista: \
                 track(...):fx(nombre, { ... })",
            ))
        });
    }
}

/// Lee `synth(tipo, { ... })` (tipos y claves del registro
/// [`api::SYNTHS`]).
fn synth_options(lua: &Lua, kind: &str, table: Option<&Table>) -> mlua::Result<InstrumentDef> {
    let who = format!("synth(\"{kind}\")");
    let Some(registered) = api::synth(kind) else {
        return Err(fail(
            lua,
            format!(
                "{who}: no hay ese sintetizador (hay: {})",
                names(api::SYNTHS.iter().map(|s| s.name))
            ),
        ));
    };
    let preset = engine::synth::preset::preset(registered.name)
        .expect("cada sintetizador del registro está en el motor");
    let mut root = preset.root;
    let mut patch = preset.patch(root as f32);
    let mut curves = [None; engine::synth::PARAM_COUNT];
    let mut seed = None;
    if let Some(table) = table {
        let known: Vec<&str> = registered.params.iter().map(|p| p.name).collect();
        for pair in table.pairs::<Value, Value>() {
            let (key, value) = pair?;
            let key = match &key {
                Value::String(k) => k.to_str()?.to_string(),
                other => {
                    return Err(fail(
                        lua,
                        format!("{who}: clave no válida ({})", other.type_name()),
                    ));
                }
            };
            let Some(param) = registered.params.iter().find(|p| p.name == key) else {
                return Err(fail(
                    lua,
                    format!(
                        "{who}: clave desconocida \"{key}\" (hay: {})",
                        known.join(", ")
                    ),
                ));
            };
            if is_curve(&value) {
                no_curve(lua, &who, param)?;
            }
            match key.as_str() {
                "wave" => {
                    let wave = match &value {
                        Value::String(w) => engine::synth::Wave::from_name(w.to_str()?.as_ref())
                            .filter(|w| preset.waves.contains(w)),
                        _ => None,
                    };
                    patch.wave = wave.ok_or_else(|| {
                        let waves: Vec<String> = preset
                            .waves
                            .iter()
                            .map(|w| format!("\"{}\"", w.name()))
                            .collect();
                        fail(lua, format!("{who}: wave es {}", waves.join(" o ")))
                    })?;
                }
                "root" => {
                    root = match &value {
                        Value::String(name) => {
                            let name = name.to_str()?.to_string();
                            patterns::note_number(&name).ok_or_else(|| {
                                fail(
                                    lua,
                                    format!(
                                        "{who}: root = \"{name}\" no es una nota (C2, F#1, Bb2...)"
                                    ),
                                )
                            })?
                        }
                        Value::Integer(n) if (0..=127).contains(n) => *n as i32,
                        _ => {
                            return Err(fail(
                                lua,
                                format!(
                                    "{who}: root es una nota (\"C2\") o un número MIDI (0 a 127)"
                                ),
                            ));
                        }
                    }
                }
                layer if engine::synth::LAYER_NAMES.contains(&layer) => {
                    let i = engine::synth::LAYER_NAMES
                        .iter()
                        .position(|n| *n == layer)
                        .expect("es una capa");
                    let layers = patch.layers.as_mut().expect("el subharm tiene capas");
                    layers[i] = layer_options(lua, &who, layer, &value, layers[i])?;
                }
                choice @ ("glide_mode" | "vca" | "pwm_src" | "lfo_wave") => {
                    use engine::synth::{GlideMode, LfoWave, PwmSource, VcaMode};
                    let analog = patch
                        .analog
                        .as_mut()
                        .expect("solo la voz analógica tiene estas claves");
                    let name = match &value {
                        Value::String(w) => Some(w.to_str()?.to_string()),
                        _ => None,
                    };
                    let name = name.as_deref().unwrap_or("");
                    let set = match choice {
                        "glide_mode" => GlideMode::from_name(name).map(|m| analog.glide = m),
                        "vca" => VcaMode::from_name(name).map(|m| analog.vca = m),
                        "pwm_src" => PwmSource::from_name(name).map(|m| analog.pwm = m),
                        _ => LfoWave::from_name(name).map(|m| analog.lfo_wave = m),
                    };
                    if set.is_none() {
                        let values: Vec<String> =
                            param.values.iter().map(|v| format!("\"{v}\"")).collect();
                        return Err(fail(
                            lua,
                            format!("{who}: {choice} es {}", or_list(&values)),
                        ));
                    }
                }
                "sub_wave" => {
                    let wave = match &value {
                        Value::String(w) => step_fx::sub_wave(w.to_str()?.as_ref()),
                        _ => None,
                    };
                    patch.sub_wave = wave.ok_or_else(|| {
                        fail(
                            lua,
                            format!(
                                "{who}: sub_wave es \"square\", \"tri\", \"sine\" o \"pulse25\""
                            ),
                        )
                    })?;
                }
                "seed" => match value {
                    Value::Integer(n) if n >= 0 => seed = Some(n as u64),
                    _ => {
                        return Err(fail(
                            lua,
                            format!("{who}: seed es un entero de 0 en adelante"),
                        ));
                    }
                },
                name => {
                    let p = preset
                        .param(name)
                        .expect("cada parámetro numérico del registro está en el motor");
                    let (fixed, curve) = number_or_curve(
                        lua,
                        &format!("{who}: {name}"),
                        name,
                        value,
                        (p.min, p.max),
                    )?;
                    if !(p.min..=p.max).contains(&fixed) {
                        return Err(fail(
                            lua,
                            format!(
                                "{who}: {name} = {fixed} fuera de rango ({} a {})",
                                p.min, p.max
                            ),
                        ));
                    }
                    use engine::synth::preset::Target;
                    let integer = matches!(
                        p.target,
                        Target::SubDiv
                            | Target::Voices
                            | Target::Hpf
                            | Target::Quality
                            | Target::LfoSync
                    );
                    if integer && fixed.fract() != 0.0 {
                        return Err(fail(
                            lua,
                            format!(
                                "{who}: {name} es un entero ({} a {}), no {fixed}",
                                p.min, p.max
                            ),
                        ));
                    }
                    p.set(&mut patch, fixed);
                    if let Some((index, curve)) = curve.and_then(|c| p.curve(c)) {
                        curves[index] = Some(curve);
                    }
                }
            }
        }
    }
    patch.root = root as f32;
    let def = SynthDef {
        kind: registered.name,
        patch,
        curves,
        seed,
    };
    Ok(InstrumentDef {
        root,
        loop_ms: None,
        engine: engine::Instrument::new(patch.voices),
        synth: Some(def),
    })
}

/// Lee la tabla de `:mpe{ ... }` del sintetizador `kind`: cada eje
/// (`bend`, `pressure`, `slide`) a `"pitch"`, `"gain"` o un parámetro del
/// sintetizador que se mueva por nota, como texto o como tabla con `range`
/// y `curve` (`{ "cutoff", range = 3, curve = 2 }`). Sin tabla, ninguno.
fn mpe_options(
    lua: &Lua,
    kind: &str,
    table: Option<&Table>,
) -> mlua::Result<[Option<engine::synth::ExprMap>; engine::synth::AXES]> {
    use engine::synth::{AXIS_NAMES, BEND, ExprMap, ExprTarget, PARAMS};
    let mut out = [None; engine::synth::AXES];
    let Some(table) = table else {
        return Ok(out);
    };
    let preset = engine::synth::preset::preset(kind)
        .expect("cada sintetizador del registro está en el motor");
    let movable = || {
        preset
            .params
            .iter()
            .filter(|p| matches!(p.target, engine::synth::preset::Target::Param(k) if engine::synth::expressive(k)))
            .map(|p| p.name)
            .collect::<Vec<_>>()
            .join(", ")
    };
    for pair in table.pairs::<Value, Value>() {
        let (key, value) = pair?;
        let axis_name = match &key {
            Value::String(k) => k.to_str()?.to_string(),
            other => {
                return Err(fail(
                    lua,
                    format!("mpe: clave no válida ({})", other.type_name()),
                ));
            }
        };
        let Some(axis) = AXIS_NAMES.iter().position(|a| *a == axis_name) else {
            return Err(fail(
                lua,
                format!(
                    "mpe: el eje \"{axis_name}\" no existe (hay: {})",
                    AXIS_NAMES.join(", ")
                ),
            ));
        };
        let who = format!("mpe: {axis_name}");
        let (target, range, curve) = match &value {
            Value::Boolean(false) | Value::Nil => continue,
            Value::String(t) => (t.to_str()?.to_string(), None, None),
            Value::Table(t) => {
                let target: Option<String> = t.get(1)?;
                let Some(target) = target else {
                    return Err(fail(
                        lua,
                        format!(
                            "{who}: la tabla empieza por adónde va: {{ \"cutoff\", range = 2 }}"
                        ),
                    ));
                };
                for pair in t.pairs::<Value, Value>() {
                    let (k, _) = pair?;
                    let known = match &k {
                        Value::Integer(1) => true,
                        Value::String(k) => matches!(k.to_str()?.as_ref(), "range" | "curve"),
                        _ => false,
                    };
                    if !known {
                        return Err(fail(
                            lua,
                            format!(
                                "{who}: clave desconocida {} (hay: range, curve)",
                                value_text(&k)
                            ),
                        ));
                    }
                }
                let range: Option<f64> = t.get("range")?;
                let curve: Option<f64> = t.get("curve")?;
                (target, range.map(|r| r as f32), curve.map(|c| c as f32))
            }
            other => {
                return Err(fail(
                    lua,
                    format!(
                        "{who}: va a un texto (\"pitch\", \"gain\" o un parámetro) o a una tabla, no a {}",
                        other.type_name()
                    ),
                ));
            }
        };
        // El rango, en las unidades del motor; por defecto, lo que se espera
        // de cada uno.
        let (target, range) = match target.as_str() {
            "pitch" => (
                ExprTarget::Pitch,
                range.unwrap_or(if axis == BEND { 12.0 } else { 2.0 }),
            ),
            "gain" => (ExprTarget::Gain, range.unwrap_or(1.0)),
            name => {
                let param = preset.param(name).filter(|p| {
                    matches!(p.target, engine::synth::preset::Target::Param(k) if engine::synth::expressive(k))
                });
                let Some(p) = param else {
                    return Err(fail(
                        lua,
                        format!(
                            "{who}: \"{name}\" no se mueve por nota en {kind} (hay: pitch, gain, {})",
                            movable()
                        ),
                    ));
                };
                let engine::synth::preset::Target::Param(k) = p.target else {
                    unreachable!("filtrado arriba")
                };
                let range = if PARAMS[k].log {
                    range.unwrap_or(2.0)
                } else {
                    range.unwrap_or((p.max - p.min) / 2.0) * p.scale
                };
                (ExprTarget::Param(k), range)
            }
        };
        let map = ExprMap {
            target,
            range,
            curve: curve.unwrap_or(1.0),
        };
        if let Err(reason) = map.validate() {
            let (lo, hi) = (
                engine::synth::EXPR_CURVE.start(),
                engine::synth::EXPR_CURVE.end(),
            );
            return Err(fail(
                lua,
                format!(
                    "{who}: {reason} (range de -{max} a {max}; con gain, de 0 a 1; curve de {lo} a {hi})",
                    max = engine::synth::MAX_EXPR_RANGE
                ),
            ));
        }
        out[axis] = Some(map);
    }
    Ok(out)
}

/// La tabla de una capa del subharm (`o1 = { div = 1, pan = -0.6 }`),
/// sobre la que tenía (claves del registro `api::LAYER_PARAMS`).
fn layer_options(
    lua: &Lua,
    who: &str,
    name: &str,
    value: &Value,
    mut layer: engine::synth::Layer,
) -> mlua::Result<engine::synth::Layer> {
    let Value::Table(table) = value else {
        return Err(fail(
            lua,
            format!("{who}: {name} es una tabla {{ div = 2, pan = 0, level = 1 }}"),
        ));
    };
    let known: Vec<&str> = api::LAYER_PARAMS.iter().map(|p| p.name).collect();
    for pair in table.pairs::<Value, Value>() {
        let (key, value) = pair?;
        let key = match &key {
            Value::String(k) => k.to_str()?.to_string(),
            other => {
                return Err(fail(
                    lua,
                    format!("{who}: {name}: clave no válida ({})", other.type_name()),
                ));
            }
        };
        if !known.contains(&key.as_str()) {
            return Err(fail(
                lua,
                format!(
                    "{who}: {name}: clave desconocida \"{key}\" (hay: {})",
                    known.join(", ")
                ),
            ));
        }
        if is_curve(&value) {
            return Err(fail(
                lua,
                format!(
                    "{who}: {name}.{key} no admite ramp() ni lfo(): {}",
                    "es una capa del subharm: sus divisores son enteros y su nivel y su pan entran con el patrón al compás"
                ),
            ));
        }
        let number = match value {
            Value::Integer(n) => n as f64,
            Value::Number(n) => n,
            _ => {
                return Err(fail(lua, format!("{who}: {name}.{key} es un número")));
            }
        };
        let integer = |what: &str| -> mlua::Result<u8> {
            if number.fract() != 0.0 || !(1.0..=engine::synth::MAX_SUB_DIV as f64).contains(&number)
            {
                return Err(fail(
                    lua,
                    format!("{who}: {name}.{what} es un entero de 1 a 16, no {number}"),
                ));
            }
            Ok(number as u8)
        };
        match key.as_str() {
            "div" => layer.div = integer("div")?,
            "rdiv" => layer.rdiv = integer("rdiv")?,
            "pan" if (-1.0..=1.0).contains(&number) => layer.pan = number as f32,
            "pan" => {
                return Err(fail(
                    lua,
                    format!("{who}: {name}.pan va de -1 a 1, no {number}"),
                ));
            }
            "level" if (0.0..=1.0).contains(&number) => layer.level = number as f32,
            "level" => {
                return Err(fail(
                    lua,
                    format!("{who}: {name}.level va de 0 a 1, no {number}"),
                ));
            }
            _ => unreachable!("claves del registro"),
        }
    }
    Ok(layer)
}

/// Fundido cruzado del bucle si no se dice otro, en ms.
const DEFAULT_CROSSFADE_MS: f64 = 10.0;

/// Lee la tabla de `sample:instrument()` (claves del registro
/// `api::INSTRUMENT_PARAMS`). El fundido del bucle se acorta si no cabe
/// (antes del inicio o dentro del bucle), con un aviso.
fn instrument_options(
    lua: &Lua,
    this: &SampleHandle,
    table: Option<&Table>,
) -> mlua::Result<InstrumentDef> {
    let who = format!("instrument({})", this.name);
    let mut def = InstrumentDef {
        root: 60,
        loop_ms: None,
        engine: engine::Instrument::new(4),
        synth: None,
    };
    let Some(table) = table else {
        return Ok(def);
    };
    table_curves(lua, &who, table, api::INSTRUMENT_PARAMS)?;
    let known: Vec<&str> = api::INSTRUMENT_PARAMS.iter().map(|p| p.name).collect();
    let mut crossfade = None;
    for pair in table.pairs::<Value, Value>() {
        let (key, value) = pair?;
        let key = match &key {
            Value::String(k) => k.to_str()?.to_string(),
            other => {
                return Err(fail(
                    lua,
                    format!("{who}: clave no válida ({})", other.type_name()),
                ));
            }
        };
        let number = |value: &Value| match value {
            Value::Integer(n) => Some(*n as f64),
            Value::Number(n) => Some(*n),
            _ => None,
        };
        let numbers = |value: &Value, n: usize| -> Option<Vec<f64>> {
            let Value::Table(t) = value else { return None };
            let list: Vec<f64> = t
                .sequence_values::<Value>()
                .filter_map(|v| v.ok())
                .filter_map(|v| number(&v))
                .collect();
            (list.len() == n && t.raw_len() == n).then_some(list)
        };
        match key.as_str() {
            "root" => {
                def.root = match &value {
                    Value::String(name) => {
                        let name = name.to_str()?.to_string();
                        patterns::note_number(&name).ok_or_else(|| {
                            fail(
                                lua,
                                format!(
                                    "{who}: root = \"{name}\" no es una nota (C3, F#4, Bb2...)"
                                ),
                            )
                        })?
                    }
                    Value::Integer(n) if (0..=127).contains(n) => *n as i32,
                    _ => {
                        return Err(fail(
                            lua,
                            format!("{who}: root es una nota (\"C3\") o un número MIDI (0 a 127)"),
                        ));
                    }
                }
            }
            "voices" => {
                def.engine.voices = match value {
                    Value::Integer(n) if (1..=engine::MAX_VOICES as i64).contains(&n) => n as u8,
                    _ => {
                        return Err(fail(
                            lua,
                            format!("{who}: voices va de 1 a {}", engine::MAX_VOICES),
                        ));
                    }
                }
            }
            "loop" => {
                let Some(range) = numbers(&value, 2) else {
                    return Err(fail(lua, format!("{who}: loop es {{inicio, fin}} en ms")));
                };
                let length = frame_to_ms(this.sample.frames(), &this.sample);
                let (from, to) = (range[0], range[1]);
                if !(0.0 <= from && from < to && to <= length) {
                    return Err(fail(
                        lua,
                        format!(
                            "{who}: loop = {{{from}, {to}}}: tiene que ir dentro del sample (0 a {length:.1} ms) y con el fin después del inicio"
                        ),
                    ));
                }
                def.loop_ms = Some((from, to));
            }
            "adsr" => {
                let Some(adsr) = numbers(&value, 4) else {
                    return Err(fail(
                        lua,
                        format!(
                            "{who}: adsr es {{ataque, decay, sustain, release}} (ms, ms, 0 a 1, ms)"
                        ),
                    ));
                };
                let max = engine::instrument::MAX_ENVELOPE_MS as f64;
                let ms = |v: f64| (0.0..=max).contains(&v);
                if !(ms(adsr[0]) && ms(adsr[1]) && ms(adsr[3])) {
                    return Err(fail(
                        lua,
                        format!("{who}: ataque, decay y release de adsr van de 0 a {max} ms"),
                    ));
                }
                if !(0.0..=1.0).contains(&adsr[2]) {
                    return Err(fail(lua, format!("{who}: el sustain de adsr va de 0 a 1")));
                }
                def.engine.adsr = engine::Adsr {
                    attack_ms: adsr[0] as f32,
                    decay_ms: adsr[1] as f32,
                    sustain: adsr[2] as f32,
                    release_ms: adsr[3] as f32,
                };
            }
            "formant" => match number(&value) {
                Some(semitones) => {
                    def.engine.formant = Some(check_formant(lua, &who, semitones as f32)?);
                }
                None => {
                    return Err(fail(
                        lua,
                        format!(
                            "{who}: formant son semitonos (0 deja los formantes donde estaban)"
                        ),
                    ));
                }
            },
            "keep" => match value {
                Value::Boolean(on) => def.engine.keep = on,
                _ => {
                    return Err(fail(
                        lua,
                        format!("{who}: keep es true o false (transponer sin cambiar la duración)"),
                    ));
                }
            },
            "crossfade" => match number(&value) {
                Some(ms) if (0.0..=1000.0).contains(&ms) => crossfade = Some(ms),
                _ => {
                    return Err(fail(lua, format!("{who}: crossfade va de 0 a 1000 ms")));
                }
            },
            other => {
                return Err(fail(
                    lua,
                    format!(
                        "{who}: clave desconocida \"{other}\" (hay: {})",
                        known.join(", ")
                    ),
                ));
            }
        }
    }
    if let Some((from, to)) = def.loop_ms {
        let (start, end) = (
            ms_to_frame(from, &this.sample),
            ms_to_frame(to, &this.sample),
        );
        let end = end.min(this.sample.frames()).max(start + 1);
        let wanted = crossfade.unwrap_or(DEFAULT_CROSSFADE_MS);
        let frames = ms_to_frame(wanted, &this.sample)
            .min(start)
            .min(end - start);
        if frames < ms_to_frame(wanted, &this.sample) {
            let warning = format!(
                "{who}: el fundido del bucle se queda en {:.1} ms (pedía {wanted} ms): \
                 necesita audio antes del inicio del bucle y caber dentro{}",
                frame_to_ms(frames, &this.sample),
                if frames == 0 {
                    "; el salto puede hacer clic"
                } else {
                    ""
                }
            );
            let mut warnings = this.warnings.borrow_mut();
            if !warnings.contains(&warning) {
                warnings.push(warning);
            }
        }
        def.engine.sustain_loop = Some((start, end));
        def.engine.crossfade = frames;
    } else if crossfade.is_some() {
        return Err(fail(
            lua,
            format!("{who}: crossfade es del bucle: falta loop = {{inicio, fin}}"),
        ));
    }
    if def.engine.formant.is_some() && !def.engine.keep {
        return Err(fail(
            lua,
            format!(
                "{who}: formant necesita keep = true (los formantes se corrigen al transponer sin cambiar la duración)"
            ),
        ));
    }
    Ok(def)
}

/// Comprueba el desplazamiento de los formantes (semitonos).
fn check_formant(lua: &Lua, who: &str, semitones: f32) -> mlua::Result<f32> {
    let max = engine::MAX_FORMANT;
    if (-max..=max).contains(&semitones) {
        Ok(semitones)
    } else {
        Err(fail(
            lua,
            format!("{who}: formant = {semitones} fuera de rango (-{max} a {max} semitonos)"),
        ))
    }
}

/// Un parámetro de los granos: un número o una curva (`ramp`, `lfo`).
fn knob(lua: &Lua, who: &str, key: &str, value: Value) -> mlua::Result<engine::Knob> {
    match value {
        Value::UserData(ud) if ud.is::<CurveHandle>() => {
            Ok(engine::Knob::Moving(ud.borrow::<CurveHandle>()?.0))
        }
        other => lua
            .unpack::<f32>(other)
            .map(engine::Knob::Fixed)
            .map_err(|_| {
                fail(
                    lua,
                    format!("{who}: {key} tiene que ser un número, ramp() o lfo()"),
                )
            }),
    }
}

/// Pone los granos de la pista: los parámetros de la tabla (ver
/// `api::GRAIN_PARAMS`) sobre los de por defecto; `freeze` congela la
/// posición en `pos`. La semilla por defecto sale del nombre de la pista
/// (dos pistas iguales no suenan igual).
fn set_granular(
    lua: &Lua,
    this: &TrackHandle,
    who: &str,
    freeze: Option<engine::Knob>,
    params: Option<&Table>,
) -> mlua::Result<()> {
    let (name, instrument) = this.with_track(|t| (t.def.name.clone(), t.def.instrument.is_some()));
    if instrument {
        return Err(fail(
            lua,
            format!(
                "{who}: la pista \"{name}\" es un instrumento; los granos van en pistas de cortes"
            ),
        ));
    }
    // FNV-1a del nombre.
    let seed = name.bytes().fold(0xcbf2_9ce4_8422_2325u64, |h, b| {
        (h ^ b as u64).wrapping_mul(0x100_0000_01b3)
    });
    if let Some(params) = params {
        table_curves(lua, who, params, api::GRAIN_PARAMS)?;
    }
    if this.with_track(|t| t.def.steps.iter().flatten().any(|s| s.fx.is_some())) {
        return Err(fail(
            lua,
            format!("{who}: {}", grains_without_step_fx(&name)),
        ));
    }
    let mut granular = engine::Granular::new(seed);
    if let Some(pos) = freeze {
        granular.position = pos;
        granular.speed = engine::Knob::Fixed(0.0);
    }
    if let Some(table) = params {
        let known: Vec<&str> = api::GRAIN_PARAMS.iter().map(|p| p.name).collect();
        for pair in table.pairs::<Value, Value>() {
            let (key, value) = pair?;
            let key = match &key {
                Value::String(k) => k.to_str()?.to_string(),
                other => {
                    return Err(fail(
                        lua,
                        format!("{who}: clave no válida ({})", other.type_name()),
                    ));
                }
            };
            if key == "seed" {
                granular.seed = match value {
                    Value::Integer(n) if n >= 0 => n as u64,
                    _ => {
                        return Err(fail(
                            lua,
                            format!("{who}: seed es un número entero (0 o más)"),
                        ));
                    }
                };
                continue;
            }
            let Some(index) = engine::GRAIN_PARAMS.iter().position(|p| p.0 == key) else {
                return Err(fail(
                    lua,
                    format!(
                        "{who}: clave desconocida \"{key}\" (hay: {})",
                        known.join(", ")
                    ),
                ));
            };
            if freeze.is_some() && key == "speed" {
                return Err(fail(
                    lua,
                    format!(
                        "{who}: congelado no avanza (speed = 0); para moverlo, grains({{ speed = ... }})"
                    ),
                ));
            }
            *granular.knobs_mut()[index] = knob(lua, who, &key, value)?;
        }
    }
    granular
        .validate()
        .map_err(|e| fail(lua, format!("{who}: {e}")))?;
    this.with_track(|t| t.def.granular = Some(granular));
    Ok(())
}

/// Aviso si una pista tiene `:formant()` pero ningún paso con `%`: los
/// formantes solo se corrigen ahí.
fn idle_formant(track: &TrackDef) -> Option<String> {
    let used = track.bars.iter().flatten().flatten().any(|step| step.keep);
    (track.formant.is_some() && !used).then(|| {
        format!(
            "pista \"{}\": formant solo actúa en los pasos con % (pitch sin cambiar la duración) y no hay ninguno",
            track.name
        )
    })
}

/// Forma de un corte desde el script: la del motor, con el fundido del
/// salto de los bucles ([`DEFAULT_SLICE_CROSSFADE_MS`]; solo actúa con
/// `play = "loop"`).
pub const DEFAULT_SHAPE: SliceShape = SliceShape {
    crossfade_ms: DEFAULT_SLICE_CROSSFADE_MS,
    ..SliceShape::DEFAULT
};

/// Fundido por defecto en el salto de un bucle de corte, en ms (el del
/// bucle de un instrumento).
pub const DEFAULT_SLICE_CROSSFADE_MS: f32 = 10.0;

/// Aviso si una pista toca en bucle con fundido un corte sin nada antes ni
/// después en el sample (un corte con el sample entero): el salto queda
/// seco.
fn dry_loop_seam(track: &TrackDef) -> Option<String> {
    let frames = track.sample.frames();
    let played: Vec<usize> = track
        .bars
        .iter()
        .flatten()
        .flatten()
        .map(|step| step.slice)
        .collect();
    let dry: Vec<String> = track
        .shapes
        .iter()
        .enumerate()
        .filter(|(i, shape)| {
            shape.play == engine::PlayMode::Loop
                && shape.crossfade_ms > 0.0
                && played.contains(i)
                && track.slices.get(*i).is_some_and(|r| {
                    let r = shape.range(r);
                    engine::loop_seam_room(r.start, r.end, frames) == 0
                })
        })
        .map(|(i, _)| (i + 1).to_string())
        .collect();
    (!dry.is_empty()).then(|| {
        format!(
            "pista \"{}\": el bucle del corte {} no tiene sonido antes ni después en el sample \
             para el fundido (crossfade): el salto queda seco",
            track.name,
            dry.join(", ")
        )
    })
}

/// Aviso si una pista con granos tiene cortes con efectos: los granos no
/// son voces y no pasan por ellos.
fn grains_skip_slice_fx(track: &TrackDef) -> Option<String> {
    let any = track.slice_fx.iter().any(|c| !c.fx.is_empty());
    (track.granular.is_some() && any).then(|| {
        format!(
            "pista \"{}\": los efectos de corte (cortes:fx) no suenan en los granos; \
             para toda la nube, usa :fx de la pista",
            track.name
        )
    })
}

/// Transposición sin cambiar la duración a partir de la cual los golpes
/// se emborronan (ver `engine::shift`): se avisa.
pub const KEEP_PITCH_LIMIT: f32 = 12.0;

/// Aviso si una pista transpone sin cambiar la duración (`%` o un
/// instrumento con `keep`) más de [`KEEP_PITCH_LIMIT`] semitonos.
fn far_kept_pitch(track: &TrackDef) -> Option<String> {
    let all = track.instrument.as_ref().is_some_and(|i| i.engine.keep);
    let far = track
        .bars
        .iter()
        .flatten()
        .flatten()
        .filter(|step| all || step.keep)
        .flat_map(|step| step.notes())
        .map(f32::abs)
        .fold(0.0f32, f32::max);
    (far > KEEP_PITCH_LIMIT).then(|| {
        format!(
            "pista \"{}\": transpone {far} semitonos sin cambiar la duración; \
             más allá de ±{KEEP_PITCH_LIMIT} los golpes se emborronan y suena metálico",
            track.name
        )
    })
}

/// Termina de cortar un sample: aplica las ediciones del proyecto
/// (`partir`, `unir`), luego sus ajustes (`corte`) y sus etiquetas, todo
/// para este sample cortado así (`cut`). Lo que no cabe se avisa.
fn cut_sample(
    lua: &Lua,
    this: &SampleHandle,
    mut slices: Slices,
    cut: Cut,
    zero: Option<f64>,
) -> mlua::Result<SlicedHandle> {
    let _ = lua;
    let gap = min_slice_gap(&this.sample);
    let warn = |warning: String| {
        let mut warnings = this.warnings.borrow_mut();
        if !warnings.contains(&warning) {
            warnings.push(warning);
        }
    };
    // Cortes añadidos y quitados en el proyecto, en orden.
    for edit in this.edits.iter() {
        if edit.sample != this.name || edit.cut != cut {
            continue;
        }
        match edit.kind {
            EditKind::Split(ms) => match slices.split(ms_to_frame(ms, &this.sample), gap) {
                Ok((split, _)) => slices = split,
                Err(_) => warn(format!(
                    "partir {} en {ms} ms: ahí no cabe un corte nuevo (muy cerca de otro, o fuera): no se aplica",
                    this.name
                )),
            },
            EditKind::Join(slice) => match slices.without(slice - 1) {
                Some(joined) => slices = joined,
                None => warn(format!(
                    "unir el corte {slice} de {}: no existe (hay {}): no se aplica",
                    this.name,
                    slices.len()
                )),
            },
        }
    }
    // Ajustes manuales del proyecto: mandan sobre el detector.
    let mut adjusted = vec![false; slices.len()];
    for p in this.positions.iter() {
        if p.sample != this.name || p.slices != cut {
            continue;
        }
        let frame = ms_to_frame(p.ms, &this.sample);
        match slices.with_start(p.slice - 1, frame, gap) {
            Ok(moved) => {
                slices = moved;
                adjusted[p.slice - 1] = true;
            }
            Err(_) if p.slice > slices.len() => {}
            Err(bounds) => warn(format!(
                "el ajuste del corte {} de {} ({} ms) se sale de sus vecinos \
                 ({:.1} a {:.1} ms): no se aplica",
                p.slice,
                this.name,
                p.ms,
                frame_to_ms(bounds.start, &this.sample),
                frame_to_ms(bounds.end, &this.sample),
            )),
        }
    }
    // Con `zero_cross`, todos los inicios (detectados, en rejilla, partidos
    // o ajustados en el proyecto) van al cruce por cero más cercano.
    if let Some(ms) = zero {
        slices = slices.snapped(&this.sample, zero_frames(ms, &this.sample), gap);
    }
    // Etiquetas del proyecto para este sample, pero cortado de otra forma:
    // no se aplican (los cortes son otros). Se avisa, y el error de una de
    // ellas en `steps` lo explica.
    let ours = |l: &&SliceLabel| l.sample == this.name;
    let stale: Vec<(String, Cut)> = if this.labels.iter().filter(ours).any(|l| l.slices == cut) {
        Vec::new()
    } else {
        this.labels
            .iter()
            .filter(ours)
            .map(|l| (l.name.clone(), l.slices))
            .collect()
    };
    let mut old_cuts: Vec<Cut> = Vec::new();
    for (_, old) in &stale {
        if !old_cuts.contains(old) {
            old_cuts.push(*old);
        }
    }
    old_cuts.sort_by_key(|c| (c.grid, c.count));
    for old in old_cuts {
        let names: Vec<String> = stale
            .iter()
            .filter(|(_, n)| *n == old)
            .map(|(name, _)| format!("«{name}»"))
            .collect();
        warn(if names.len() == 1 {
            format!(
                "la etiqueta {} de {} es de {} y este script usa {}: no se aplica",
                names[0],
                this.name,
                old.call(),
                cut.call()
            )
        } else {
            format!(
                "las etiquetas {} de {} son de {} y este script usa {}: no se aplican",
                names.join(", "),
                this.name,
                old.call(),
                cut.call()
            )
        });
    }
    // Las etiquetas del proyecto para este sample cortado así.
    let mut labels = vec![None; slices.len()];
    for label in this.labels.iter() {
        if label.sample == this.name
            && label.slices == cut
            && let Some(slot) = labels.get_mut(label.slice - 1)
        {
            *slot = Some(label.name.clone());
        }
    }
    let count = slices.len();
    Ok(SlicedHandle {
        path: this.path.clone(),
        sample: this.sample.clone(),
        origin: SliceOrigin {
            sample: this.name.clone(),
            count: cut.count,
            grid: cut.grid,
            indices: (0..count).map(Some).collect(),
            zero_cross: zero,
        },
        slices,
        labels: Rc::new(RefCell::new(labels)),
        stale: Rc::new(stale),
        adjusted,
        stretch: this.stretch,
        shapes: Rc::new(RefCell::new(vec![DEFAULT_SHAPE; count])),
        slice_fx: Rc::new(RefCell::new(vec![SliceFxDef::default(); count])),
    })
}

/// `sample(...):slice(n)` en Lua: sample con sus cortes y sus etiquetas.
#[derive(Clone)]
struct SlicedHandle {
    path: PathBuf,
    sample: Arc<Sample>,
    slices: Slices,
    /// Etiqueta de cada corte; compartidas por la variable y sus pistas.
    labels: Rc<RefCell<Vec<Option<String>>>>,
    origin: SliceOrigin,
    /// Etiquetas del proyecto de otro `slice(n)` (nombre y su `n`), para
    /// explicar el error si se usan.
    stale: Rc<Vec<(String, Cut)>>,
    /// Cortes con el inicio ajustado a mano.
    adjusted: Vec<bool>,
    /// Tempo de origen de `sample:stretch()`.
    stretch: Option<f64>,
    /// Forma de cada corte (`:set()`); compartida por la variable y sus
    /// pistas, como las etiquetas.
    shapes: Rc<RefCell<Vec<SliceShape>>>,
    /// Efectos de cada corte (`:fx()`); compartidos como las formas.
    slice_fx: Rc<RefCell<Vec<SliceFxDef>>>,
}

/// Lee la tabla de opciones de `sample(nombre, { ... })` (claves del
/// registro `api::SAMPLE_PARAMS`).
fn import_options(lua: &Lua, name: &str, table: &Table) -> mlua::Result<engine::ImportOptions> {
    let who = format!("sample(\"{name}\")");
    table_curves(lua, &who, table, api::SAMPLE_PARAMS)?;
    let known: Vec<&str> = api::SAMPLE_PARAMS.iter().map(|p| p.name).collect();
    let mut options = engine::ImportOptions::default();
    for pair in table.pairs::<Value, Value>() {
        let (key, value) = pair?;
        let key = match key {
            Value::String(s) => s.to_str()?.to_string(),
            other => format!("{other:?}"),
        };
        let number = |v: &Value| -> Option<f32> {
            match v {
                Value::Integer(i) => Some(*i as f32),
                Value::Number(n) => Some(*n as f32),
                _ => None,
            }
        };
        // `true` (el nivel por defecto), `false` (nada) o un número en dB.
        let level = |default: f32| -> mlua::Result<Option<f32>> {
            match &value {
                Value::Boolean(true) => Ok(Some(default)),
                Value::Boolean(false) | Value::Nil => Ok(None),
                v => number(v).map(Some).ok_or_else(|| {
                    fail(lua, format!("{who}: {key} es true, false o un nivel en dB"))
                }),
            }
        };
        let ms = || -> mlua::Result<f32> {
            number(&value).ok_or_else(|| fail(lua, format!("{who}: {key} son ms (un número)")))
        };
        match key.as_str() {
            "trim" => options.trim_db = level(engine::ImportOptions::TRIM_DB)?,
            "normalize" => options.normalize_db = level(engine::ImportOptions::NORMALIZE_DB)?,
            "fade_in" => options.fade_in_ms = ms()?,
            "fade_out" => options.fade_out_ms = ms()?,
            _ => {
                return Err(fail(
                    lua,
                    format!(
                        "{who}: opción desconocida \"{key}\" (admite: {})",
                        known.join(", ")
                    ),
                ));
            }
        }
    }
    options
        .validate()
        .map_err(|e| fail(lua, format!("{who}: {e}")))?;
    Ok(options)
}

/// Aplica una tabla de `cortes:set()` a la forma de un corte. Las claves
/// salen del registro (`api::SLICE_PARAMS`).
fn apply_slice_table(
    lua: &Lua,
    who: &str,
    shape: &mut SliceShape,
    table: &Table,
) -> mlua::Result<()> {
    table_curves(lua, who, table, api::SLICE_PARAMS)?;
    let known: Vec<&str> = api::SLICE_PARAMS.iter().map(|p| p.name).collect();
    for pair in table.pairs::<Value, Value>() {
        let (key, value) = pair?;
        let key = match key {
            Value::String(s) => s.to_str()?.to_string(),
            other => format!("{other:?}"),
        };
        // Un número o una curva (por golpe): el valor fijo es su primer
        // extremo; un número quita la curva que hubiera.
        let mut number = |v: &Value, slot: usize| -> mlua::Result<f32> {
            match v {
                Value::Integer(i) => {
                    shape.curves[slot] = None;
                    Ok(*i as f32)
                }
                Value::Number(n) => {
                    shape.curves[slot] = None;
                    Ok(*n as f32)
                }
                Value::UserData(ud) if ud.is::<CurveHandle>() => {
                    let curve = ud.borrow::<CurveHandle>()?.0;
                    shape.curves[slot] = Some(curve);
                    Ok(curve.bounds().0)
                }
                _ => Err(fail(
                    lua,
                    format!("{who}: {key} tiene que ser un número, ramp() o lfo()"),
                )),
            }
        };
        match key.as_str() {
            "attack" => shape.attack_ms = number(&value, engine::SHAPE_ATTACK)?,
            "release" => shape.release_ms = number(&value, engine::SHAPE_RELEASE)?,
            "pan" => shape.pan = number(&value, engine::SHAPE_PAN)?,
            "length" => shape.length = number(&value, engine::SHAPE_LENGTH)?,
            "crossfade" => shape.crossfade_ms = number(&value, engine::SHAPE_CROSSFADE)?,
            "play" => {
                let text = match &value {
                    Value::String(s) => s.to_str()?.to_string(),
                    _ => String::new(),
                };
                shape.play = match text.as_str() {
                    "normal" => engine::PlayMode::Once,
                    "loop" => engine::PlayMode::Loop,
                    "pingpong" => engine::PlayMode::PingPong,
                    _ => {
                        return Err(fail(
                            lua,
                            format!(
                                "{who}: play = \"{text}\" no existe (\"normal\", \"loop\" o \"pingpong\")"
                            ),
                        ));
                    }
                };
            }
            _ => {
                return Err(fail(
                    lua,
                    format!(
                        "{who}: parámetro desconocido \"{key}\" (admite: {})",
                        known.join(", ")
                    ),
                ));
            }
        }
    }
    shape
        .validate()
        .map_err(|e| fail(lua, format!("{who}: {e}")))
}

/// El primer argumento de `cortes:fx()` como se escribió (para los
/// errores): `3`, `"snare"` o `{1, "snare"}`.
fn which_text(which: &Value) -> String {
    match which {
        Value::Integer(i) => i.to_string(),
        Value::Number(n) => n.to_string(),
        Value::String(s) => format!("\"{}\"", s.to_string_lossy()),
        Value::Table(t) => {
            let items: Vec<String> = t
                .clone()
                .sequence_values::<Value>()
                .filter_map(Result::ok)
                .map(|v| which_text(&v))
                .collect();
            format!("{{{}}}", items.join(", "))
        }
        _ => "?".to_string(),
    }
}

/// Los cortes (desde 0) a los que se refiere el primer argumento de
/// `cortes:fx()`: un número (desde 1), una etiqueta o una lista de ellos.
fn slice_targets(
    lua: &Lua,
    who: &str,
    this: &SlicedHandle,
    which: &Value,
) -> mlua::Result<Vec<usize>> {
    let count = this.slices.len();
    let one = |v: &Value| -> mlua::Result<usize> {
        let number = match v {
            Value::Integer(i) => Some(*i),
            Value::Number(n) if n.fract() == 0.0 => Some(*n as i64),
            _ => None,
        };
        if let Some(n) = number {
            return usize::try_from(n - 1)
                .ok()
                .filter(|&i| i < count)
                .ok_or_else(|| fail(lua, format!("{who}: el corte {n} no existe (hay {count})")));
        }
        let Value::String(label) = v else {
            return Err(fail(
                lua,
                format!("{who}: el corte es un número, una etiqueta o una lista de ellos"),
            ));
        };
        let label = label.to_str()?.to_string();
        let labels = this.labels.borrow();
        if let Some(i) = labels
            .iter()
            .position(|l| l.as_deref() == Some(label.as_str()))
        {
            return Ok(i);
        }
        let known: Vec<&str> = labels.iter().flatten().map(String::as_str).collect();
        Err(fail(
            lua,
            if known.is_empty() {
                format!(
                    "{who}: ningún corte se llama \"{label}\" (no hay etiquetas: ponlas antes con cortes:name(i, \"{label}\"))"
                )
            } else {
                format!(
                    "{who}: ningún corte se llama \"{label}\" (etiquetas: {})",
                    known.join(", ")
                )
            },
        ))
    };
    let mut targets = Vec::new();
    match which {
        Value::Table(t) => {
            for v in t.clone().sequence_values::<Value>() {
                let i = one(&v?)?;
                if !targets.contains(&i) {
                    targets.push(i);
                }
            }
            if targets.is_empty() {
                return Err(fail(lua, format!("{who}: la lista de cortes está vacía")));
            }
        }
        v => targets.push(one(v)?),
    }
    Ok(targets)
}

/// Las automatizaciones de los parámetros de `fx` (en la ranura `index` de
/// su cadena) que llevan `ramp()` o `lfo()`.
fn fx_automations(
    lua: &Lua,
    name: &str,
    fx: Fx,
    index: usize,
    curves: &[(String, engine::Curve)],
) -> mlua::Result<Vec<engine::Automation>> {
    let mut chain = [None; MAX_FX];
    chain[index] = Some(fx);
    let mut automations = Vec::new();
    for (key, curve) in curves {
        let Some(param) = fx.param_index(key) else {
            let can = if fx.params().is_empty() {
                "este efecto no tiene parámetros automatizables".to_string()
            } else {
                format!("sí: {}", fx.params().join(", "))
            };
            return Err(fail(
                lua,
                format!("fx(\"{name}\"): {key} no se puede automatizar ({can})"),
            ));
        };
        let automation = engine::Automation {
            fx: index,
            param,
            curve: *curve,
        };
        automation
            .validate(&chain)
            .map_err(|e| fail(lua, format!("fx(\"{name}\"): {key}: {e}")))?;
        automations.push(automation);
    }
    Ok(automations)
}

impl UserData for SlicedHandle {
    fn add_methods<M: UserDataMethods<Self>>(methods: &mut M) {
        methods.add_method("count", |_, this, ()| Ok(this.slices.len()));

        // `:note(i)`: la nota del corte `i` en cifrado americano (`"C#4"`)
        // y su desafinación en cents, o nil si no tiene un tono claro.
        methods.add_method("note", |lua, this, index: i64| {
            let range = usize::try_from(index - 1)
                .ok()
                .and_then(|i| this.slices.get(i))
                .ok_or_else(|| {
                    fail(
                        lua,
                        format!(
                            "note({index}): el corte no existe (hay {})",
                            this.slices.len()
                        ),
                    )
                })?;
            Ok(engine::detect_note(&this.sample, range)
                .map(|g| (g.name(), g.cents()))
                .unzip())
        });

        // `:set(i, { attack = ms, release = ms })` (o `:set({ ... })` para
        // todos): la forma de un corte.
        methods.add_method(
            "set",
            |lua, this, (first, second): (Value, Option<Table>)| {
                let count = this.slices.len();
                let (index, table) = match (first, second) {
                    (Value::Table(table), None) => (None, table),
                    (Value::Integer(i), Some(table)) => (Some(i), table),
                    (Value::Number(n), Some(table)) if n.fract() == 0.0 => (Some(n as i64), table),
                    _ => {
                        return Err(fail(
                            lua,
                            "set(i, { ... }) para el corte i, o set({ ... }) para todos",
                        ));
                    }
                };
                let who = match index {
                    Some(i) => format!("set({i})"),
                    None => "set".to_string(),
                };
                let range = match index {
                    Some(i) => {
                        let i = usize::try_from(i - 1)
                            .ok()
                            .filter(|&i| i < count)
                            .ok_or_else(|| {
                                fail(lua, format!("{who}: el corte no existe (hay {count})"))
                            })?;
                        i..i + 1
                    }
                    None => 0..count,
                };
                let mut shapes = this.shapes.borrow_mut();
                for shape in &mut shapes[range] {
                    apply_slice_table(lua, &who, shape, &table)?;
                }
                drop(shapes);
                Ok(this.clone())
            },
        );

        // `:slice_at(i)`: solo el corte `i` (desde 1), como fuente de un
        // único corte (en el patrón, `1`, `x` o su etiqueta).
        methods.add_method("slice_at", |lua, this, index: i64| {
            let i = usize::try_from(index - 1).ok();
            let only = i.and_then(|i| this.slices.only(i)).ok_or_else(|| {
                fail(
                    lua,
                    format!(
                        "slice_at({index}): el corte no existe (hay {})",
                        this.slices.len()
                    ),
                )
            })?;
            let i = i.unwrap_or(0);
            Ok(SlicedHandle {
                path: this.path.clone(),
                sample: this.sample.clone(),
                slices: only,
                labels: Rc::new(RefCell::new(vec![this.labels.borrow()[i].clone()])),
                origin: SliceOrigin {
                    indices: vec![this.origin.indices[i]],
                    ..this.origin.clone()
                },
                stale: this.stale.clone(),
                adjusted: vec![this.adjusted[i]],
                stretch: this.stretch,
                shapes: Rc::new(RefCell::new(vec![this.shapes.borrow()[i]])),
                slice_fx: Rc::new(RefCell::new(vec![this.slice_fx.borrow()[i].clone()])),
            })
        });

        // `:add(ms)`: un corte nuevo que empieza en `ms` (parte en dos el que
        // lo contiene). Como `move`, las pistas definidas antes no lo ven.
        methods.add_method_mut("add", |lua, this, ms: f64| {
            if !(ms.is_finite() && ms >= 0.0) {
                return Err(fail(lua, format!("add({ms}): ms desde el principio (0 o más)")));
            }
            let mut frame = ms_to_frame(ms, &this.sample);
            if let Some(zero) = this.origin.zero_cross {
                frame = engine::snap_frame(
                    &this.sample,
                    frame,
                    zero_frames(zero, &this.sample),
                    0..this.sample.frames(),
                );
            }
            let (slices, k) = this
                .slices
                .split(frame, min_slice_gap(&this.sample))
                .map_err(|bounds| {
                    fail(
                        lua,
                        format!(
                            "add({ms}): ahí no cabe un corte nuevo (en ese corte, de {:.1} a {:.1} ms)",
                            frame_to_ms(bounds.start, &this.sample),
                            frame_to_ms(bounds.end, &this.sample)
                        ),
                    )
                })?;
            this.slices = slices;
            let mut labels = this.labels.borrow().clone();
            labels.insert(k, None);
            this.labels = Rc::new(RefCell::new(labels));
            let mut shapes = this.shapes.borrow().clone();
            shapes.insert(k, DEFAULT_SHAPE);
            this.shapes = Rc::new(RefCell::new(shapes));
            let mut chains = this.slice_fx.borrow().clone();
            chains.insert(k, SliceFxDef::default());
            this.slice_fx = Rc::new(RefCell::new(chains));
            this.adjusted.insert(k, false);
            this.origin.indices.insert(k, None);
            Ok(this.clone())
        });

        // `:remove(i)`: quita el corte `i` (desde 2): el anterior se alarga.
        methods.add_method_mut("remove", |lua, this, index: i64| {
            let count = this.slices.len();
            let i = usize::try_from(index - 1)
                .ok()
                .filter(|&i| i < count)
                .ok_or_else(|| {
                    fail(
                        lua,
                        format!("remove({index}): el corte no existe (hay {count})"),
                    )
                })?;
            let slices = this.slices.without(i).ok_or_else(|| {
                fail(
                    lua,
                    format!(
                        "remove({index}): el corte 1 no tiene anterior; quita el 2 para unirlos"
                    ),
                )
            })?;
            this.slices = slices;
            let mut labels = this.labels.borrow().clone();
            labels.remove(i);
            this.labels = Rc::new(RefCell::new(labels));
            let mut shapes = this.shapes.borrow().clone();
            shapes.remove(i);
            this.shapes = Rc::new(RefCell::new(shapes));
            let mut chains = this.slice_fx.borrow().clone();
            chains.remove(i);
            this.slice_fx = Rc::new(RefCell::new(chains));
            this.adjusted.remove(i);
            this.origin.indices.remove(i);
            Ok(this.clone())
        });

        // `:move(i, ms)`: el corte `i` empieza en `ms` (ajuste manual; las
        // pistas definidas antes con esta variable no lo ven).
        methods.add_method_mut("move", |lua, this, (index, ms): (i64, f64)| {
            let count = this.slices.len();
            let i = usize::try_from(index - 1)
                .ok()
                .filter(|&i| i < count)
                .ok_or_else(|| {
                    fail(
                        lua,
                        format!("move({index}): el corte no existe (hay {count})"),
                    )
                })?;
            if !(ms.is_finite() && ms >= 0.0) {
                return Err(fail(
                    lua,
                    format!("move({index}, {ms}): ms desde el principio (0 o más)"),
                ));
            }
            let gap = min_slice_gap(&this.sample);
            let mut frame = ms_to_frame(ms, &this.sample);
            if let Some(zero) = this.origin.zero_cross {
                // Sin salirse de sus vecinos: el cruce más cercano dentro.
                let low = i
                    .checked_sub(1)
                    .map_or(0, |p| this.slices.get(p).map_or(0, |r| r.start + gap));
                let high = this.slices.get(i).map_or(0, |r| r.end.saturating_sub(gap));
                frame = engine::snap_frame(
                    &this.sample,
                    frame,
                    zero_frames(zero, &this.sample),
                    low..high + 1,
                );
            }
            this.slices = this.slices.with_start(i, frame, gap).map_err(|bounds| {
                fail(
                    lua,
                    format!(
                        "move({index}, {ms}): se sale de sus vecinos ({:.1} a {:.1} ms)",
                        frame_to_ms(bounds.start, &this.sample),
                        frame_to_ms(bounds.end, &this.sample)
                    ),
                )
            })?;
            this.adjusted[i] = true;
            Ok(this.clone())
        });

        // `:fx(cuál, nombre, { parámetros })`: un efecto que solo suena en
        // ese corte (número, etiqueta o una lista de ellos).
        methods.add_method(
            "fx",
            |lua, this, (which, name, params): (Value, String, Option<Table>)| {
                let who = format!("fx({}, \"{name}\")", which_text(&which));
                let targets = slice_targets(lua, &who, this, &which)?;
                if let Some(p) = &params
                    && !matches!(p.get::<Value>("sidechain")?, Value::Nil)
                {
                    return Err(fail(
                        lua,
                        format!(
                            "{who}: el EQ dinámico de un corte no puede escuchar otra pista \
                             (sidechain solo en el :fx de la pista)"
                        ),
                    ));
                }
                let (fx, curves) = parse_fx(lua, &name, params)?;
                let mut chains = this.slice_fx.borrow_mut();
                for &i in &targets {
                    let chain = &mut chains[i];
                    if chain.fx.len() >= MAX_FX {
                        return Err(fail(
                            lua,
                            format!(
                                "{who}: el corte {} ya tiene {MAX_FX} efectos (máximo)",
                                i + 1
                            ),
                        ));
                    }
                    let index = chain.fx.len();
                    let automations = fx_automations(lua, &name, fx, index, &curves)?;
                    let mut all = chain.automation.clone();
                    all.extend(automations);
                    let mut list = chain.fx.clone();
                    list.push(fx);
                    SliceFx::new(&list, &all).map_err(|e| fail(lua, format!("{who}: {e}")))?;
                    chain.fx = list;
                    chain.automation = all;
                }
                drop(chains);
                Ok(this.clone())
            },
        );

        // `:name(i, "kick")`: etiqueta el corte `i` para usarla en `steps`
        // (`nil` la quita).
        methods.add_method("name", |lua, this, (index, name): (i64, Option<String>)| {
            let count = this.slices.len();
            let i = usize::try_from(index - 1)
                .ok()
                .filter(|&i| i < count)
                .ok_or_else(|| {
                    fail(
                        lua,
                        format!("name({index}): el corte no existe (hay {count})"),
                    )
                })?;
            let mut labels = this.labels.borrow_mut();
            if let Some(name) = &name {
                check_label(name)
                    .map_err(|e| fail(lua, format!("name({index}, \"{name}\"): {e}")))?;
                if let Some(other) = labels
                    .iter()
                    .position(|l| l.as_deref() == Some(name.as_str()))
                    .filter(|&o| o != i)
                {
                    return Err(fail(
                        lua,
                        format!(
                            "name({index}, \"{name}\"): la etiqueta ya es del corte {}",
                            other + 1
                        ),
                    ));
                }
            }
            labels[i] = name;
            drop(labels);
            Ok(this.clone())
        });
    }
}

/// `track(nombre, ...)` en Lua.
#[derive(Clone)]
struct TrackHandle {
    name: String,
    state: Rc<RefCell<Builder>>,
}

impl TrackHandle {
    /// `:steps(texto)` (y `:melody()`, que lo traduce a pasos: `written` es
    /// el texto tal como se escribió, para saber dónde está cada paso).
    fn set_steps(&self, lua: &Lua, text: &str, written: &str) -> mlua::Result<()> {
        let (labels, stale, count, length, plays) = self.with_track(|t| {
            (
                t.labels.borrow().clone(),
                t.stale.clone(),
                t.def.origin.cut(),
                t.def.length,
                Plays::of(&t.def),
            )
        });
        let stale = StaleLabels {
            labels: &stale,
            count,
        };
        let steps = checked_steps(lua, &self.name, &labels, &stale, text, length, plays)?;
        if self.with_track(|t| t.def.granular.is_some())
            && steps.iter().flatten().any(|s| s.fx.is_some())
        {
            return Err(fail(lua, grains_without_step_fx(&self.name)));
        }
        let spans = written::spans(lua, written, length);
        let at = location(lua);
        self.with_track(|t| {
            t.def.steps = steps;
            t.steps_set = true;
            t.written = spans;
            t.steps_at = at;
        });
        Ok(())
    }

    fn with_track<R>(&self, f: impl FnOnce(&mut TrackBuild) -> R) -> R {
        let mut builder = self.state.borrow_mut();
        let track = builder
            .tracks
            .iter_mut()
            .find(|t| t.def.name == self.name)
            .expect("la pista existe desde track()");
        f(track)
    }
}

/// `group(nombre, ...)` en Lua: sus métodos cambian el grupo y lo
/// devuelven (para encadenar).
#[derive(Clone)]
struct GroupHandle {
    name: String,
    state: Rc<RefCell<Builder>>,
}

impl GroupHandle {
    fn with_group<R>(&self, f: impl FnOnce(&mut GroupDef) -> R) -> R {
        let mut builder = self.state.borrow_mut();
        let group = builder
            .groups
            .iter_mut()
            .find(|g| g.name == self.name)
            .expect("el grupo existe desde group()");
        f(group)
    }
}

impl UserData for GroupHandle {
    fn add_methods<M: UserDataMethods<Self>>(methods: &mut M) {
        // Como los de una pista, sobre la suma de las suyas.
        methods.add_method(
            "fx",
            |lua, this, (name, params): (String, Option<Table>)| {
                let count = this.with_group(|g| g.fx.len());
                let (fx, automations, sidechain) = fx_arg(lua, &name, params, count, None)?;
                this.with_group(|g| {
                    if let Some(key) = sidechain {
                        g.sidechains.push((g.fx.len(), key));
                    }
                    g.fx.push(fx);
                    g.automation.extend(automations);
                });
                Ok(this.clone())
            },
        );
        methods.add_method("gain", |lua, this, gain: Value| {
            let (gain, curve) = gain_arg(lua, gain)?;
            this.with_group(|g| {
                g.gain = gain;
                g.mix_curves[engine::MIX_GAIN] = curve;
            });
            Ok(this.clone())
        });
        methods.add_method("pan", |lua, this, pan: Value| {
            let (pan, curve) = pan_arg(lua, pan)?;
            this.with_group(|g| {
                g.pan = pan;
                g.mix_curves[engine::MIX_PAN] = curve;
            });
            Ok(this.clone())
        });
        methods.add_method("send", |lua, this, (bus, level): (String, Value)| {
            let (index, level, curve) = send_arg(lua, &bus, level)?;
            this.with_group(|g| {
                g.sends[index] = level;
                g.mix_curves[engine::MIX_SEND + index] = curve;
            });
            Ok(this.clone())
        });
    }
}

/// Etiquetas del proyecto que no se aplican a una pista (de otro
/// `slice(n)`), con el número de cortes que usa el script.
struct StaleLabels<'a> {
    labels: &'a [(String, Cut)],
    count: Cut,
}

/// Lee y valida un patrón en mini-notación para una pista con esas
/// etiquetas (una por corte, así que también dan el número de cortes).
fn checked_steps(
    lua: &Lua,
    track: &str,
    labels: &[Option<String>],
    stale: &StaleLabels,
    text: &str,
    length: usize,
    plays: Plays,
) -> mlua::Result<Bar> {
    let slices = labels.len();
    let steps = parse_pattern_with(text, length, labels, Some(&step_fx::resolve)).map_err(|e| {
        // Una etiqueta del proyecto de otro `slice(n)`: decirlo.
        let hint = split_label(&e.token)
            .and_then(|(name, _)| stale.labels.iter().find(|(n, _)| n == name))
            .map_or(String::new(), |(name, old)| {
                format!(
                    "; la etiqueta «{name}» del proyecto es de {} y este script usa {}",
                    old.call(),
                    stale.count.call()
                )
            });
        fail(lua, format!("steps: {e}{hint}"))
    })?;
    for step in steps.iter().flatten() {
        if step.slice >= slices {
            return Err(fail(
                lua,
                format!(
                    "steps: el corte {} no existe (\"{track}\" tiene {slices})",
                    step.slice + 1
                ),
            ));
        }
        check_step(lua, step)?;
        if plays != Plays::Slices && step.fx.is_some() {
            let what = match plays {
                Plays::Synth { .. } => "un sintetizador",
                _ => "un instrumento",
            };
            return Err(fail(
                lua,
                format!(
                    "steps: \"{track}\" es {what}: sus notas no tienen efectos por paso \
                     ({{...}}); usa los de la pista (:fx)"
                ),
            ));
        }
        if !matches!(plays, Plays::Synth { .. }) && step.sub.is_some() {
            return Err(fail(
                lua,
                format!(
                    "steps: {{sub=...}} es de los sintetizadores (\"{track}\" no lo es); \
                     usa synth(\"acid\", {{...}}) o synth(\"sub\", {{...}})"
                ),
            ));
        }
        if !matches!(plays, Plays::Synth { .. }) && (step.accent || step.slide) {
            return Err(fail(
                lua,
                format!(
                    "steps: {} es de los sintetizadores (\"{track}\" no lo es); \
                     usa synth(\"acid\", {{...}})",
                    if step.accent {
                        "! (accent)"
                    } else {
                        "/ (slide)"
                    }
                ),
            ));
        }
        if matches!(plays, Plays::Synth { layers: true, .. }) && step.sub.is_some() {
            return Err(fail(
                lua,
                format!(
                    "steps: \"{track}\" es un subharm: no tiene bloque sub ({{sub=...}}); \
                     sus subs son las capas s1 a s4"
                ),
            ));
        }
        if matches!(plays, Plays::Synth { mono: true, .. })
            && step.chord.iter().any(Option::is_some)
        {
            return Err(fail(
                lua,
                format!(
                    "steps: \"{track}\" es un sintetizador monofónico: toca una nota cada vez \
                     (sin acordes)"
                ),
            ));
        }
        if plays == Plays::Slices && step.chord.iter().any(Option::is_some) {
            return Err(fail(
                lua,
                format!(
                    "steps: un acorde (varios ^) necesita una pista con instrumento \
                     (\"{track}\" es de cortes: suena una nota cada vez); \
                     usa sample(...):instrument{{...}}"
                ),
            ));
        }
    }
    Ok(steps)
}

/// Mismos rangos que valida el motor, pero con la línea del script.
fn check_step(lua: &Lua, step: &Step) -> mlua::Result<()> {
    if !(0.0..=MAX_GAIN).contains(&step.gain) {
        return Err(fail(lua, "steps: ganancia fuera de rango (0 a 4)"));
    }
    if step
        .notes()
        .any(|st| !(-MAX_SEMITONES..=MAX_SEMITONES).contains(&st))
    {
        return Err(fail(
            lua,
            "steps: pitch fuera de rango (-36 a 36 semitonos)",
        ));
    }
    if !(1..=MAX_RETRIG).contains(&step.retrig) {
        return Err(fail(lua, "steps: retrigger fuera de rango (1 a 16)"));
    }
    if step.lock.is_some_and(|hz| !(20.0..=20_000.0).contains(&hz)) {
        return Err(fail(lua, "steps: corte (~) fuera de rango (20 a 20000 Hz)"));
    }
    Ok(())
}

fn check_retrig(lua: &Lua, n: i64) -> mlua::Result<u8> {
    u8::try_from(n)
        .ok()
        .filter(|n| (1..=MAX_RETRIG).contains(n))
        .ok_or_else(|| fail(lua, format!("retrig({n}): fuera de rango (1 a 16)")))
}

fn check_pitch(lua: &Lua, semitones: f32) -> mlua::Result<f32> {
    if (-MAX_SEMITONES..=MAX_SEMITONES).contains(&semitones) {
        Ok(semitones)
    } else {
        Err(fail(
            lua,
            format!("pitch({semitones}): fuera de rango (-36 a 36)"),
        ))
    }
}

fn check_gain(lua: &Lua, gain: f32) -> mlua::Result<f32> {
    if (0.0..=MAX_GAIN).contains(&gain) {
        Ok(gain)
    } else {
        Err(fail(lua, format!("gain({gain}): fuera de rango (0 a 4)")))
    }
}

/// Lee los parámetros de `:fx()`; error si hay claves desconocidas (las
/// conocidas salen del registro de la API).
fn fx_params(lua: &Lua, name: &str, params: &Option<Table>) -> mlua::Result<()> {
    let Some(params) = params else { return Ok(()) };
    let known: Vec<&str> = api::effect(name)
        .map(|e| e.params.iter().map(|p| p.name).collect())
        .unwrap_or_default();
    for pair in params.pairs::<Value, Value>() {
        let (key, value) = pair?;
        let key = match key {
            Value::String(s) => s.to_str()?.to_string(),
            other => format!("{other:?}"),
        };
        if !known.contains(&key.as_str()) {
            return Err(fail(
                lua,
                format!(
                    "fx(\"{name}\"): parámetro desconocido \"{key}\" (admite: {})",
                    known.join(", ")
                ),
            ));
        }
        if is_curve(&value)
            && let Some(param) = api::effect(name).and_then(|e| e.param(&key))
        {
            no_curve(lua, &format!("fx(\"{name}\")"), param)?;
        }
    }
    Ok(())
}

/// Error con la línea si alguna clave de `table` lleva una curva y su
/// parámetro (en `params`, del registro) no la admite.
fn table_curves(lua: &Lua, who: &str, table: &Table, params: &[api::FxParam]) -> mlua::Result<()> {
    for pair in table.pairs::<Value, Value>() {
        let (key, value) = pair?;
        let Value::String(key) = key else { continue };
        let key = key.to_str()?.to_string();
        if is_curve(&value)
            && let Some(param) = params.iter().find(|p| p.name == key)
        {
            no_curve(lua, who, param)?;
        }
    }
    Ok(())
}

/// Que los efectos de cada paso quepan con los de su corte (como mucho 4
/// efectos y 8 curvas entre los dos; manda el paso).
fn step_fx_fit(def: &TrackDef) -> Result<(), String> {
    for (lap, bar) in def.bars.iter().enumerate() {
        for (k, step) in bar.iter().enumerate() {
            let Some((step, fx)) = step.and_then(|s| Some((s, s.fx?))) else {
                continue;
            };
            let Some(chain) = def.slice_fx.get(step.slice).filter(|c| !c.fx.is_empty()) else {
                continue;
            };
            let slice = SliceFx::new(&chain.fx, &chain.automation).map_err(str::to_string)?;
            slice.with_step(&fx).map_err(|e| {
                let at = if def.bars.len() > 1 {
                    format!("vuelta {}, paso {}", lap + 1, k + 1)
                } else {
                    format!("paso {}", k + 1)
                };
                format!(
                    "{at}: {e} (con los de cortes:fx del corte {})",
                    step.slice + 1
                )
            })?;
        }
    }
    Ok(())
}

/// Por qué una pista con granos no admite efectos por paso.
fn grains_without_step_fx(track: &str) -> String {
    format!(
        "\"{track}\" toca en granos: sus golpes no tienen efectos por paso ({{...}}; los granos no \
         son voces), usa los de la pista (:fx)"
    )
}

/// Si un valor de Lua es una curva (`ramp()`, `lfo()`).
fn is_curve(value: &Value) -> bool {
    matches!(value, Value::UserData(ud) if ud.is::<CurveHandle>())
}

/// Error con la línea si `param` no admite curvas (el porqué, del registro).
fn no_curve(lua: &Lua, who: &str, param: &api::FxParam) -> mlua::Result<()> {
    match param.automate {
        api::Auto::No(why) => Err(fail(
            lua,
            format!("{who}: {} no admite ramp() ni lfo(): {why}", param.name),
        )),
        _ => Ok(()),
    }
}

/// Un número o una curva para el parámetro de pista `param` (del registro,
/// [`api::TRACK_PARAMS`]) de `who`, dentro de `lo..=hi`: el valor fijo (el
/// primer extremo de la curva) y la curva, si la hay.
fn number_or_curve(
    lua: &Lua,
    who: &str,
    param: &str,
    value: Value,
    (lo, hi): (f32, f32),
) -> mlua::Result<(f32, Option<engine::Curve>)> {
    let (fixed, curve) = match &value {
        Value::UserData(ud) if ud.is::<CurveHandle>() => {
            if let Some(p) = api::track_param(param) {
                no_curve(lua, who, p)?;
            }
            let curve = ud.borrow::<CurveHandle>()?.0;
            (curve.bounds().0, Some(curve))
        }
        other => (
            lua.unpack::<f32>(other.clone()).map_err(|_| {
                fail(
                    lua,
                    format!("{who}: tiene que ser un número, ramp() o lfo()"),
                )
            })?,
            None,
        ),
    };
    if let Some(c) = curve {
        let (a, b) = c.bounds();
        if ![a, b].iter().all(|v| (lo..=hi).contains(v)) {
            return Err(fail(
                lua,
                format!("{who}: la curva se sale del rango ({lo} a {hi})"),
            ));
        }
    }
    Ok((fixed, curve))
}

/// Un argumento tal como se escribió, para los mensajes: el número, o
/// `ramp(...)`/`lfo(...)`.
fn value_text(value: &Value) -> String {
    match value {
        Value::Integer(i) => i.to_string(),
        Value::Number(n) => n.to_string(),
        Value::UserData(ud) => match ud.borrow::<CurveHandle>().map(|c| c.0) {
            Ok(engine::Curve::Ramp { .. }) => "ramp(...)".into(),
            Ok(engine::Curve::Lfo { .. }) => "lfo(...)".into(),
            Ok(engine::Curve::Macro { .. }) => "macro(...)".into(),
            _ => "...".into(),
        },
        _ => "...".into(),
    }
}

/// `:gain(g)` de una pista o un grupo (lineal, de 0 a 4): el valor fijo y
/// su curva.
fn gain_arg(lua: &Lua, gain: Value) -> mlua::Result<(f32, Option<engine::Curve>)> {
    let who = format!("gain({})", value_text(&gain));
    let (gain, curve) = number_or_curve(lua, &who, "gain", gain, (0.0, MAX_GAIN))?;
    if !(0.0..=MAX_GAIN).contains(&gain) {
        return Err(fail(lua, SendError::InvalidTrackGain(gain)));
    }
    Ok((gain, curve))
}

/// `:pan(p)` de una pista o un grupo (de -1 a 1).
fn pan_arg(lua: &Lua, pan: Value) -> mlua::Result<(f32, Option<engine::Curve>)> {
    let who = format!("pan({})", value_text(&pan));
    let (pan, curve) = number_or_curve(lua, &who, "pan", pan, (-1.0, 1.0))?;
    if !(-1.0..=1.0).contains(&pan) {
        return Err(fail(lua, format!("{who}: el pan va de -1 a 1")));
    }
    Ok((pan, curve))
}

/// `:send(bus, nivel)` de una pista o un grupo: el bus, el nivel fijo y su
/// curva.
fn send_arg(
    lua: &Lua,
    bus: &str,
    level: Value,
) -> mlua::Result<(usize, f32, Option<engine::Curve>)> {
    let index = match bus {
        "delay" => engine::bus::DELAY,
        "reverb" => engine::bus::REVERB,
        other => {
            return Err(fail(
                lua,
                format!(
                    "send(\"{other}\"): el bus no existe ({})",
                    names(api::BUSES.iter().map(|b| b.name))
                ),
            ));
        }
    };
    let who = format!("send(\"{bus}\", {})", value_text(&level));
    let (level, curve) = number_or_curve(lua, &who, "send", level, (0.0, engine::bus::MAX_SEND))?;
    if !(0.0..=engine::bus::MAX_SEND).contains(&level) {
        return Err(fail(lua, format!("{who}: el nivel va de 0 a 1")));
    }
    Ok((index, level, curve))
}

/// `:fx(nombre, { parámetros })` de una pista (`own`, su nombre) o un
/// grupo, que ya tiene `count` efectos: el efecto, sus automatizaciones
/// (los parámetros con `ramp()`, `lfo()` o `macro()`) y la pista que
/// escucha por sidechain el EQ dinámico (se busca al acabar el script:
/// puede definirse después).
fn fx_arg(
    lua: &Lua,
    name: &str,
    params: Option<Table>,
    count: usize,
    own: Option<&str>,
) -> mlua::Result<(Fx, Vec<engine::Automation>, Option<String>)> {
    if let (Some(effect), Some(p)) = (api::effect(name), &params) {
        table_curves(lua, &format!("fx(\"{name}\")"), p, effect.params)?;
    }
    let sidechain = match (&params, name) {
        (Some(p), "dyneq") => match p.get::<Value>("sidechain")? {
            Value::Nil => None,
            Value::String(s) => Some(s.to_str()?.to_string()),
            _ => {
                return Err(fail(
                    lua,
                    "fx(\"dyneq\"): sidechain es el nombre de una pista, entre comillas",
                ));
            }
        },
        _ => None,
    };
    if let Some(own) = own
        && sidechain.as_deref() == Some(own)
    {
        return Err(fail(
            lua,
            format!(
                "fx(\"dyneq\"): \"{own}\" no puede escucharse a sí misma por sidechain (sin sidechain ya escucha la propia pista)"
            ),
        ));
    }
    let (fx, curves) = parse_fx(lua, name, params)?;
    if count >= MAX_FX {
        let whose = if own.is_some() { "pista" } else { "grupo" };
        return Err(fail(
            lua,
            format!("fx: máximo {MAX_FX} efectos por {whose}"),
        ));
    }
    let automations = fx_automations(lua, name, fx, count, &curves)?;
    Ok((fx, automations, sidechain))
}

/// Lista separada por comas.
fn names(list: impl Iterator<Item = &'static str>) -> String {
    list.collect::<Vec<_>>().join(", ")
}

/// `ramp(...)` o `lfo(...)` en Lua: un valor que cambia con el tiempo,
/// para un parámetro de `:fx`.
#[derive(Clone, Copy)]
struct CurveHandle(engine::Curve);

impl UserData for CurveHandle {}

/// Duración escrita como `"4 compases"`, `"1 compás"`, `"8 pasos"` o `"1
/// paso"`, en pasos.
fn parse_duration(text: &str) -> Option<f64> {
    let mut words = text.split_whitespace();
    let n: f64 = words.next()?.replace(',', ".").parse().ok()?;
    let unit = words.next()?;
    if words.next().is_some() || !(n > 0.0 && n.is_finite()) {
        return None;
    }
    let steps = match unit {
        "compás" | "compas" | "compases" | "bar" | "bars" => n * STEPS as f64,
        "paso" | "pasos" | "step" | "steps" => n,
        _ => return None,
    };
    (steps <= (MAX_CYCLE_BARS * STEPS) as f64).then_some(steps)
}

/// Lee los parámetros de `:fx()` y devuelve el efecto con sus valores fijos
/// y los parámetros que llevan una curva (`ramp`, `lfo`): el valor fijo de
/// esos es el primer extremo de la curva.
fn parse_fx(
    lua: &Lua,
    name: &str,
    params: Option<Table>,
) -> mlua::Result<(Fx, Vec<(String, engine::Curve)>)> {
    if api::effect(name).is_some() {
        fx_params(lua, name, &params)?;
    }
    let arg = |key: &str| -> Result<Option<FxArg>, String> {
        let Some(p) = &params else { return Ok(None) };
        Ok(match p.get::<Value>(key).map_err(|e| e.to_string())? {
            Value::Nil => None,
            Value::UserData(ud) if ud.is::<CurveHandle>() => Some(FxArg::Curve(
                ud.borrow::<CurveHandle>().map_err(|e| e.to_string())?.0,
            )),
            Value::Integer(i) => Some(FxArg::Number(i as f32)),
            Value::Number(n) => Some(FxArg::Number(n as f32)),
            Value::String(s) => Some(FxArg::Text(s.to_string_lossy().to_string())),
            _ => Some(FxArg::Other),
        })
    };
    build_fx(name, &arg).map_err(|e| fail(lua, e))
}

/// Un valor de un parámetro de efecto, venga de una tabla de Lua o de los
/// efectos de un paso.
pub(crate) enum FxArg {
    Number(f32),
    Curve(engine::Curve),
    Text(String),
    Other,
}

/// El efecto `name` (nombre o alias del registro) con los parámetros que da
/// `arg`: sus valores fijos y los parámetros que llevan una curva (el valor
/// fijo de esos es su primer extremo). Las claves desconocidas las
/// comprueba quien llama.
pub(crate) fn build_fx(
    name: &str,
    arg: &dyn Fn(&str) -> Result<Option<FxArg>, String>,
) -> Result<(Fx, Vec<(String, engine::Curve)>), String> {
    let curves: RefCell<Vec<(String, engine::Curve)>> = RefCell::default();
    let get = |key: &str| -> Result<Option<f32>, String> {
        let wrong = || format!("fx(\"{name}\"): {key} tiene que ser un número, ramp() o lfo()");
        match arg(key)? {
            None => Ok(None),
            Some(FxArg::Curve(curve)) => {
                curves.borrow_mut().push((key.to_string(), curve));
                Ok(Some(curve.bounds().0))
            }
            Some(FxArg::Number(v)) => Ok(Some(v)),
            Some(FxArg::Text(t)) => t.trim().parse().map(Some).map_err(|_| wrong()),
            Some(FxArg::Other) => Err(wrong()),
        }
    };
    // Un parámetro de texto (`type`): `None` si no está.
    let text = |key: &str, wrong: String| -> Result<Option<String>, String> {
        match arg(key)? {
            None => Ok(None),
            Some(FxArg::Text(t)) => Ok(Some(t)),
            Some(FxArg::Number(n)) => Ok(Some(n.to_string())),
            Some(_) => Err(wrong),
        }
    };
    let filter = |kind: FilterKind| -> Result<Fx, String> {
        let cutoff_hz =
            get("cutoff")?.ok_or_else(|| format!("fx(\"{name}\"): falta cutoff (Hz)"))?;
        Ok(Fx::Filter {
            kind,
            cutoff_hz,
            q: get("q")?.unwrap_or(engine::DEFAULT_Q),
        })
    };
    let canonical = api::effect(name).map_or(name, |e| e.name);
    let fx = match canonical {
        "filter" => {
            let kind = text("type", "fx(\"filter\"): type tiene que ser un texto".into())?;
            match kind.as_deref().unwrap_or("lowpass") {
                "lowpass" => filter(FilterKind::LowPass)?,
                "highpass" => filter(FilterKind::HighPass)?,
                other => {
                    return Err(format!(
                        "fx(\"filter\"): type \"{other}\" no existe (lowpass o highpass)"
                    ));
                }
            }
        }
        "lowpass" | "highpass" => {
            let kind = if canonical == "lowpass" {
                FilterKind::LowPass
            } else {
                FilterKind::HighPass
            };
            filter(kind)?
        }
        "bitcrush" => Fx::Bitcrush {
            bits: get("bits")?.unwrap_or(8.0) as u32,
            downsample: get("downsample")?.unwrap_or(1.0) as u32,
        },
        "compressor" => Fx::Compressor {
            threshold_db: get("threshold")?.unwrap_or(-18.0),
            ratio: get("ratio")?.unwrap_or(4.0),
            attack_ms: get("attack")?.unwrap_or(5.0),
            release_ms: get("release")?.unwrap_or(100.0),
            makeup_db: get("makeup")?.unwrap_or(0.0),
        },
        "distortion" => Fx::Distortion {
            drive: get("drive")?.unwrap_or(4.0),
            mix: get("mix")?.unwrap_or(1.0),
        },
        "gain" => Fx::Gain {
            level: get("level")?.unwrap_or(1.0),
        },
        "dyneq" => {
            let kind = text("type", "fx(\"dyneq\"): type tiene que ser un texto".into())?;
            let kind = match kind.as_deref() {
                None => EqKind::Bell,
                Some(k) => EqKind::from_name(k).ok_or_else(|| {
                    format!("fx(\"dyneq\"): type \"{k}\" no existe (bell, lowshelf o highshelf)")
                })?,
            };
            let freq_hz = get("freq")?.ok_or("fx(\"dyneq\"): falta freq (Hz)")?;
            let d = engine::DynEqParams::new(freq_hz);
            Fx::DynEq(engine::DynEqParams {
                kind,
                freq_hz,
                q: get("q")?.unwrap_or(d.q),
                threshold_db: get("threshold")?.unwrap_or(d.threshold_db),
                ratio: get("ratio")?.unwrap_or(d.ratio),
                range_db: get("range")?.unwrap_or(d.range_db),
                attack_ms: get("attack")?.unwrap_or(d.attack_ms),
                release_ms: get("release")?.unwrap_or(d.release_ms),
                sidechain: None,
            })
        }
        "eq" => {
            let mut bands = [EqBand::flat(0); EQ_BANDS];
            for (i, band) in bands.iter_mut().enumerate() {
                let n = i + 1;
                let kind = text(
                    &format!("type{n}"),
                    format!("fx(\"eq\"): type{n} tiene que ser un texto"),
                )?;
                let kind = match kind.as_deref() {
                    None => EqKind::Bell,
                    Some(k) => EqKind::from_name(k).ok_or_else(|| {
                        format!(
                            "fx(\"eq\"): type{n} \"{k}\" no existe (bell, lowshelf o highshelf)"
                        )
                    })?,
                };
                *band = EqBand {
                    kind,
                    freq_hz: get(&format!("freq{n}"))?.unwrap_or(engine::EQ_DEFAULT_FREQS[i]),
                    gain_db: get(&format!("gain{n}"))?.unwrap_or(0.0),
                    q: get(&format!("q{n}"))?.unwrap_or(engine::EQ_DEFAULT_Q),
                };
            }
            Fx::Eq { bands }
        }
        _ => {
            return Err(format!(
                "fx(\"{name}\"): no existe ({})",
                names(api::EFFECTS.iter().map(|e| e.name))
            ));
        }
    };
    fx.validate().map_err(|e| format!("fx(\"{name}\"): {e}"))?;
    let curves = curves.into_inner();
    Ok((fx, curves))
}

impl UserData for TrackHandle {
    fn add_methods<M: UserDataMethods<Self>>(methods: &mut M) {
        // Todos los métodos devuelven la pista para poder encadenar.

        // `:steps("1 . 3' ...")`: patrón base en mini-notación.
        methods.add_method("steps", |lua, this, text: String| {
            this.set_steps(lua, &text, &text)?;
            Ok(this.clone())
        });

        // `:melody("c e g e")`: notas por su nombre, desde la raíz del
        // instrumento (en una pista de cortes, C4 es el corte 1 tal cual).
        methods.add_method("melody", |lua, this, text: String| {
            let root = this.with_track(|t| t.def.instrument.map_or(60, |i| i.root));
            let key = this.state.borrow().key.as_ref().map(|(k, _)| k.degrees());
            let steps = patterns::notes(&text, root, key).map_err(|e| fail(lua, e))?;
            // Un grado ha usado la tonalidad: `key()` ya no puede cambiar.
            if key.is_some() && patterns::notes(&text, root, None).is_err() {
                let mut builder = this.state.borrow_mut();
                if builder.key_used.is_none() {
                    builder.key_used = Some(location(lua));
                }
            }
            this.set_steps(lua, &steps, &text)?;
            this.with_track(|t| t.melodic.set(true));
            Ok(this.clone())
        });

        // `:length(n)`: pasos de una vuelta del patrón (16 por defecto). Va
        // antes de `:steps()`, que entonces pide n pasos.
        methods.add_method("length", |lua, this, n: i64| {
            if !(1..=MAX_STEPS as i64).contains(&n) {
                return Err(fail(
                    lua,
                    format!("length({n}): la longitud va de 1 a {MAX_STEPS} pasos"),
                ));
            }
            let n = n as usize;
            let (set, current) = this.with_track(|t| (t.steps_set, t.def.length));
            if set && current != n {
                return Err(fail(
                    lua,
                    format!(
                        "length({n}): steps ya tiene {current} pasos; pon :length({n}) antes de :steps()"
                    ),
                ));
            }
            this.with_track(|t| {
                t.def.length = n;
                if !t.steps_set {
                    t.def.steps = vec![None; n];
                }
            });
            Ok(this.clone())
        });

        // `:gain(g)`: volumen de la pista (lineal, de 0 a 4).
        methods.add_method("gain", |lua, this, gain: Value| {
            let (gain, curve) = gain_arg(lua, gain)?;
            this.with_track(|t| {
                t.def.gain = gain;
                t.def.mix_curves[engine::MIX_GAIN] = curve;
            });
            Ok(this.clone())
        });

        // `:pan(p)`: pan de la pista (de -1 a 1), después de su volumen.
        methods.add_method("pan", |lua, this, pan: Value| {
            let (pan, curve) = pan_arg(lua, pan)?;
            this.with_track(|t| {
                t.def.pan = pan;
                t.def.mix_curves[engine::MIX_PAN] = curve;
            });
            Ok(this.clone())
        });

        // `:swing(p)`: swing en porcentaje estilo MPC (50 recto, 75 máximo).
        methods.add_method("swing", |lua, this, percent: Value| {
            let who = format!("swing({})", value_text(&percent));
            let (percent, _) =
                number_or_curve(lua, &who, "swing", percent, (STRAIGHT_SWING, MAX_SWING))?;
            if !(STRAIGHT_SWING..=MAX_SWING).contains(&percent) {
                return Err(fail(
                    lua,
                    format!("swing({percent}): va de 50 (recto) a 75"),
                ));
            }
            this.with_track(|t| t.def.swing = percent);
            Ok(this.clone())
        });

        // `:humanize(ms, volumen [, semilla])`: retraso y volumen al azar.
        methods.add_method(
            "humanize",
            |lua, this, (ms, volume, seed): (f32, Option<f32>, Option<i64>)| {
                let volume = volume.unwrap_or(0.0);
                if !(0.0..=MAX_HUMANIZE_MS).contains(&ms) {
                    return Err(fail(lua, format!("humanize({ms}): los ms van de 0 a 100")));
                }
                if !(0.0..=1.0).contains(&volume) {
                    return Err(fail(
                        lua,
                        format!("humanize({ms}, {volume}): el volumen va de 0 a 1"),
                    ));
                }
                this.with_track(|t| {
                    t.def.humanize = Some(Humanize {
                        ms,
                        volume,
                        seed: seed.map(|s| s as u64),
                    })
                });
                Ok(this.clone())
            },
        );

        // `:mode("gate" | "oneshot")`: cada golpe dura su paso, o el corte
        // entero (por defecto).
        methods.add_method("mode", |lua, this, mode: String| {
            let gate = match mode.as_str() {
                "gate" => true,
                "oneshot" | "one-shot" => false,
                other => {
                    return Err(fail(
                        lua,
                        format!("mode(\"{other}\"): es \"gate\" u \"oneshot\""),
                    ));
                }
            };
            this.with_track(|t| t.def.gate = gate);
            Ok(this.clone())
        });

        // `:midi(canal, nota_base)`: cada golpe sale también como una nota
        // MIDI (con `--allow-midi`, por «sampler salida»).
        methods.add_method("midi", |lua, this, (channel, base): (i64, Option<i64>)| {
            // En un instrumento la nota es la que suena: raíz + semitonos.
            if let Some(root) = this.with_track(|t| t.def.instrument.map(|i| i.root)) {
                if !(1..=16).contains(&channel) {
                    return Err(fail(
                        lua,
                        format!("midi({channel}): el canal va de 1 a 16"),
                    ));
                }
                if let Some(base) = base {
                    return Err(fail(
                        lua,
                        format!(
                            "midi({channel}, {base}): \"{}\" es un instrumento: cada nota sale a su altura (la raíz es root); quita nota_base",
                            this.name
                        ),
                    ));
                }
                this.with_track(|t| t.def.midi = Some((channel as u8, root as u8)));
                return Ok(this.clone());
            }
            let base = base.unwrap_or(36);
            if !(1..=16).contains(&channel) || !(0..=127).contains(&base) {
                return Err(fail(
                    lua,
                    format!("midi({channel}, {base}): el canal va de 1 a 16 y la nota de 0 a 127"),
                ));
            }
            this.with_track(|t| t.def.midi = Some((channel as u8, base as u8)));
            Ok(this.clone())
        });

        // `:choke(n)`: grupo de choke (0 o nil, ninguno).
        methods.add_method("choke", |lua, this, group: Option<i64>| {
            let group = match group.unwrap_or(0) {
                0 => None,
                g if (1..=engine::MAX_CHOKE as i64).contains(&g) => Some(g as u8),
                g => {
                    return Err(fail(
                        lua,
                        format!(
                            "choke({g}): el grupo va de 1 a {} (0 lo quita)",
                            engine::MAX_CHOKE
                        ),
                    ));
                }
            };
            this.with_track(|t| t.def.choke = group);
            Ok(this.clone())
        });

        // `:grains({ ... })` y `:freeze([pos [, { ... }]])`: la pista toca
        // en granos.
        methods.add_method("grains", |lua, this, params: Option<Table>| {
            set_granular(lua, this, "grains", None, params.as_ref())?;
            Ok(this.clone())
        });
        methods.add_method(
            "freeze",
            |lua, this, (pos, params): (Option<Value>, Option<Table>)| {
                let pos = match pos {
                    None | Some(Value::Nil) => engine::Knob::Fixed(0.0),
                    Some(value) => knob(lua, "freeze", "pos", value)?,
                };
                set_granular(lua, this, "freeze", Some(pos), params.as_ref())?;
                Ok(this.clone())
            },
        );

        // `:formant(n)`: los formantes de los pasos con `%`.
        methods.add_method("formant", |lua, this, semitones: Value| {
            let who = format!("formant({})", value_text(&semitones));
            let max = engine::MAX_FORMANT;
            let (semitones, curve) =
                number_or_curve(lua, &who, "formant", semitones, (-max, max))?;
            let (name, instrument) =
                this.with_track(|t| (t.def.name.clone(), t.def.instrument.is_some()));
            if instrument {
                return Err(fail(
                    lua,
                    format!(
                        "formant: la pista \"{name}\" es un instrumento; ponlo en su tabla: sample:instrument({{ keep = true, formant = {semitones} }})"
                    ),
                ));
            }
            check_formant(lua, "formant", semitones)?;
            this.with_track(|t| {
                t.def.formant = Some(semitones);
                t.def.formant_curve = curve;
            });
            Ok(this.clone())
        });

        // `:mpe{ bend = "pitch", pressure = "cutoff", slide = "pw" }`: la
        // expresión por nota de un sintetizador (MIDI en vivo, MPE).
        methods.add_method("mpe", |lua, this, table: Option<Table>| {
            let (name, synth) = this.with_track(|t| {
                (
                    t.def.name.clone(),
                    t.def.instrument.and_then(|i| i.synth),
                )
            });
            let Some(synth) = synth else {
                return Err(fail(
                    lua,
                    format!(
                        "mpe: la pista \"{name}\" no es un sintetizador (la expresión por nota es de synth(...), subharm{{...}} y sh101{{...}})"
                    ),
                ));
            };
            let expr = mpe_options(lua, synth.kind, table.as_ref())?;
            this.with_track(|t| {
                if let Some(s) = t.def.instrument.as_mut().and_then(|i| i.synth.as_mut()) {
                    s.patch.expr = expr;
                }
            });
            Ok(this.clone())
        });

        // `:pitch(st)`: transpone todos los golpes de la pista (por golpe).
        methods.add_method("pitch", |lua, this, semitones: Value| {
            let who = format!("pitch({})", value_text(&semitones));
            let (semitones, curve) = number_or_curve(
                lua,
                &who,
                "pitch",
                semitones,
                (-MAX_SEMITONES, MAX_SEMITONES),
            )?;
            if !(-MAX_SEMITONES..=MAX_SEMITONES).contains(&semitones) {
                return Err(fail(lua, format!("{who}: fuera de rango (-36 a 36)")));
            }
            this.with_track(|t| {
                t.def.pitch = semitones;
                t.def.pitch_curve = curve;
            });
            Ok(this.clone())
        });

        // `:send(bus, nivel)`: parte de la pista al delay o a la reverb.
        methods.add_method("send", |lua, this, (bus, level): (String, Value)| {
            let (index, level, curve) = send_arg(lua, &bus, level)?;
            this.with_track(|t| {
                t.def.sends[index] = level;
                t.def.mix_curves[engine::MIX_SEND + index] = curve;
            });
            Ok(this.clone())
        });

        // `:mute([sí])` y `:solo([sí])`: entran con el patrón, al compás.
        methods.add_method("mute", |_, this, on: Option<bool>| {
            this.with_track(|t| t.def.mute = on.unwrap_or(true));
            Ok(this.clone())
        });
        methods.add_method("solo", |_, this, on: Option<bool>| {
            this.with_track(|t| t.def.solo = on.unwrap_or(true));
            Ok(this.clone())
        });

        // `:fx(nombre, { parámetros })`: añade un efecto a la cadena.
        methods.add_method(
            "fx",
            |lua, this, (name, params): (String, Option<Table>)| {
                let count = this.with_track(|t| t.def.fx.len());
                let (fx, automations, sidechain) =
                    fx_arg(lua, &name, params, count, Some(&this.name))?;
                this.with_track(|t| {
                    if let Some(key) = sidechain {
                        t.def.sidechains.push((t.def.fx.len(), key));
                    }
                    t.def.fx.push(fx);
                    t.def.automation.extend(automations);
                });
                Ok(this.clone())
            },
        );

        // `:every(n, function(t) ... end)`: en los compases 0, n, 2n...
        methods.add_method("every", |lua, this, (n, func): (i64, Function)| {
            if !(1..=MAX_CYCLE_BARS as i64).contains(&n) {
                return Err(fail(
                    lua,
                    format!("every({n}): n va de 1 a {MAX_CYCLE_BARS}"),
                ));
            }
            this.with_track(|t| t.transforms.push(Transform::Every { n: n as u64, func }));
            Ok(this.clone())
        });

        // `:fill(efecto, n [, pasos [, valor]])`: relleno al final de cada n
        // compases.
        methods.add_method(
            "fill",
            |lua, this, (effect, n, count, value): (String, i64, Option<i64>, Option<f64>)| {
                let who = format!("fill(\"{effect}\", {n})");
                if !(1..=MAX_CYCLE_BARS as i64).contains(&n) {
                    return Err(fail(lua, format!("{who}: n va de 1 a {MAX_CYCLE_BARS}")));
                }
                let count = count.unwrap_or(4);
                if !(1..=MAX_STEPS as i64).contains(&count) {
                    return Err(fail(
                        lua,
                        format!("{who}: los pasos van de 1 a {MAX_STEPS}"),
                    ));
                }
                let effect = match effect.as_str() {
                    "stutter" => Fill::Stutter(check_retrig(lua, value.unwrap_or(4.0) as i64)?),
                    "roll" => Fill::Roll,
                    "reverse" => Fill::Reverse,
                    "pitch" => Fill::Pitch(check_pitch(lua, value.unwrap_or(12.0) as f32)?),
                    "mute" => Fill::Mute,
                    other => {
                        return Err(fail(
                            lua,
                            format!(
                                "fill: efecto \"{other}\" no existe ({})",
                                names(api::FILL_EFFECTS.iter().map(|e| e.name))
                            ),
                        ));
                    }
                };
                this.with_track(|t| {
                    t.transforms.push(Transform::Fill {
                        n: n as u64,
                        steps: count as usize,
                        effect,
                    })
                });
                Ok(this.clone())
            },
        );

        // `:chance(p, efecto, valor)`: cada golpe, con probabilidad p.
        methods.add_method(
            "chance",
            |lua, this, (p, effect, value): (f64, String, Option<f64>)| {
                if !(0.0..=1.0).contains(&p) {
                    return Err(fail(
                        lua,
                        format!("chance({p}): la probabilidad va de 0 a 1"),
                    ));
                }
                let need = |what: &str| {
                    value.ok_or_else(|| {
                        fail(
                            lua,
                            format!("chance(p, \"{effect}\", {what}): falta {what}"),
                        )
                    })
                };
                let effect = match effect.as_str() {
                    "retrig" | "stutter" => Chance::Retrig(check_retrig(lua, need("n")? as i64)?),
                    "reverse" => Chance::Reverse,
                    "pitch" => Chance::Pitch(check_pitch(lua, need("semitonos")? as f32)?),
                    "gain" => Chance::Gain(check_gain(lua, need("volumen")? as f32)?),
                    "mute" => Chance::Mute,
                    "accent" | "slide" if !matches!(this.with_track(|t| Plays::of(&t.def)), Plays::Synth { .. }) => {
                        return Err(fail(
                            lua,
                            format!(
                                "chance: \"{effect}\" es de los sintetizadores (la pista \"{}\" no lo es)",
                                this.name
                            ),
                        ));
                    }
                    "accent" => Chance::Accent,
                    "slide" => Chance::Slide,
                    other => {
                        return Err(fail(
                            lua,
                            format!(
                                "chance: efecto \"{other}\" no existe ({})",
                                names(api::CHANCE_EFFECTS.iter().map(|e| e.name))
                            ),
                        ));
                    }
                };
                this.with_track(|t| t.transforms.push(Transform::Chance { p, effect }));
                Ok(this.clone())
            },
        );
    }
}

/// El compás (la vuelta del patrón) que recibe la función de `every`.
struct BarHandle {
    steps: Rc<RefCell<Bar>>,
    /// Pasos de la vuelta (la longitud de la pista).
    length: usize,
    labels: Rc<Vec<Option<String>>>,
    stale: Rc<Vec<(String, Cut)>>,
    count: Cut,
    track: String,
    /// Raíz de `melody` (la del instrumento, o C4) y qué toca la pista
    /// (acordes, accent y slide).
    root: i32,
    /// La tonalidad para los grados de `melody`, y si se usó (`transpose`
    /// mueve entonces la pista).
    key: Option<patterns::Key>,
    melodic: Rc<Cell<bool>>,
    plays: Plays,
    /// Dónde está escrito cada paso de esta vuelta: el de la pista hasta
    /// que un `:steps()` lo sustituye.
    written: Rc<RefCell<Vec<Option<TokenSpan>>>>,
}

impl BarHandle {
    fn each(&self, f: impl Fn(&mut Step)) {
        for step in self.steps.borrow_mut().iter_mut().flatten() {
            f(step);
        }
    }

    fn handle(&self) -> BarHandle {
        BarHandle {
            steps: self.steps.clone(),
            length: self.length,
            labels: self.labels.clone(),
            stale: self.stale.clone(),
            count: self.count,
            track: self.track.clone(),
            root: self.root,
            key: self.key,
            melodic: self.melodic.clone(),
            plays: self.plays,
            written: self.written.clone(),
        }
    }

    fn set_steps(&self, lua: &Lua, text: &str, written: &str) -> mlua::Result<()> {
        let stale = StaleLabels {
            labels: &self.stale,
            count: self.count,
        };
        let steps = checked_steps(
            lua,
            &self.track,
            &self.labels,
            &stale,
            text,
            self.length,
            self.plays,
        )?;
        *self.steps.borrow_mut() = steps;
        *self.written.borrow_mut() = written::spans(lua, written, self.length);
        Ok(())
    }
}

impl UserData for BarHandle {
    fn add_methods<M: UserDataMethods<Self>>(methods: &mut M) {
        // Sustituye los pasos de este compás.
        methods.add_method("steps", |lua, this, text: String| {
            this.set_steps(lua, &text, &text)?;
            Ok(this.handle())
        });
        // Sustituye los pasos por notas (como `pista:melody`).
        methods.add_method("melody", |lua, this, text: String| {
            let steps = patterns::notes(&text, this.root, this.key).map_err(|e| fail(lua, e))?;
            this.set_steps(lua, &steps, &text)?;
            this.melodic.set(true);
            Ok(this.handle())
        });
        // Invierte el sentido de cada golpe del compás.
        methods.add_method("reverse", |_, this, ()| {
            this.each(|s| s.reverse = !s.reverse);
            Ok(this.handle())
        });
        methods.add_method("pitch", |lua, this, semitones: f32| {
            let semitones = check_pitch(lua, semitones)?;
            this.each(|s| s.semitones = semitones);
            Ok(this.handle())
        });
        methods.add_method("retrig", |lua, this, n: i64| {
            let n = check_retrig(lua, n)?;
            this.each(|s| s.retrig = n);
            Ok(this.handle())
        });
        methods.add_method("gain", |lua, this, gain: f32| {
            let gain = check_gain(lua, gain)?;
            this.each(|s| s.gain = gain);
            Ok(this.handle())
        });
    }
}

/// `"a"`, `"a" o "b"`, `"a", "b" o "c"`.
fn or_list(values: &[String]) -> String {
    match values {
        [] => String::new(),
        [one] => one.clone(),
        [rest @ .., last] => format!("{} o {last}", rest.join(", ")),
    }
}
