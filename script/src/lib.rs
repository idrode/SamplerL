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

use engine::notation::{check_label, parse_pattern_labeled, split_label};
use engine::{
    Batch, Buses, Controller, Division, FilterKind, Fx, MAX_CYCLE_BARS, MAX_FX, MAX_GAIN,
    MAX_RETRIG, MAX_SEMITONES, MAX_STEPS, MAX_TRACKS, Pattern, STEPS, Sample, SendError,
    SliceShape, Slices, Step, valid_bpm,
};
pub mod api;
pub mod blocks;
pub mod patterns;
pub mod project;
mod watch;

use patterns::Rng;
pub use project::{Cut, EditKind, Project, SliceEdit, SliceLabel, SlicePosition};
pub use watch::{ScriptWatcher, WatchEvent};

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
    /// Último modelo aplicado con éxito.
    song: Option<Song>,
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
            seed: None,
            sample_dir: None,
            default_bpm: None,
            labels: Vec::new(),
            positions: Vec::new(),
            edits: Vec::new(),
            imported: Vec::new(),
            stretched: Vec::new(),
        }
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
        let song = self.eval(source, name, base_dir)?;
        let slots = self.assign_slots(&song);
        let batch = song_batch(&song, &slots)?;
        controller
            .apply(batch)
            .map_err(|e| ScriptError::new(format!("no se pudo enviar al motor: {e}")))?;

        self.slots = slots;
        self.song = Some(song);
        Ok(self.song.as_ref().unwrap())
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
            .and_then(|()| lua.load(source).set_name(format!("={name}")).exec());
        self.samples = std::mem::take(&mut *cache.borrow_mut());
        self.imported = std::mem::take(&mut imports.borrow_mut().used);
        result.map_err(|e| ScriptError::from_lua(&e))?;

        let (bpm, metronome, buses, tempo_ramp, meter, tracks, expanding) = {
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
            )
        };
        if tracks.len() > MAX_TRACKS {
            return Err(ScriptError::new(format!(
                "{name}: hay {} pistas y el máximo es {MAX_TRACKS}",
                tracks.len()
            )));
        }

        let mut tracks = tracks;
        self.apply_stretch(&mut tracks, bpm.unwrap_or(engine::DEFAULT_BPM), name)?;

        // Expandir `every` y `chance` a un ciclo de compases. Las funciones de
        // `every` se llaman ahora, con el estado de Lua todavía vivo.
        let mut rng = Rng::new(seed);
        let timing = Timing {
            bpm: bpm.unwrap_or(engine::DEFAULT_BPM),
            seed,
        };
        expanding.set(true);
        let expanded = tracks
            .into_iter()
            .map(|track| expand(track, &mut rng, timing))
            .collect::<Result<Vec<_>, _>>();
        expanding.set(false);
        let warnings = warnings.borrow().clone();
        Ok(Song {
            bpm,
            tracks: expanded?,
            metronome,
            buses,
            tempo_ramp,
            meter,
            warnings,
        })
    }

    /// Estira los samples de las pistas con `:stretch()` al tempo de la
    /// canción y escala sus cortes. Las pistas del mismo sample comparten
    /// el resultado (los inicios de todos sus cortes son límites de tramo),
    /// que se guarda para la carga siguiente.
    fn apply_stretch(
        &mut self,
        tracks: &mut [TrackBuild],
        bpm: f64,
        name: &str,
    ) -> Result<(), ScriptError> {
        let mut used: Vec<Stretched> = Vec::new();
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
        self.stretched = used;
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
                let number = |key: &str, default: f32, max: f32| -> mlua::Result<f32> {
                    let value = match get(key)? {
                        None => default,
                        Some(Value::Integer(i)) => i as f32,
                        Some(Value::Number(n)) => n as f32,
                        Some(_) => {
                            return Err(fail(lua, format!("{who}: {key} tiene que ser un número")));
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
                let mut builder = st.borrow_mut();
                let buses = &mut builder.buses;
                match name.as_str() {
                    "delay" => {
                        let d = engine::DelayParams::default();
                        let division = match get("time")? {
                            None => d.division,
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
                            feedback: number("feedback", d.feedback, engine::bus::MAX_FEEDBACK)?,
                            level: number("level", d.level, 1.0)?,
                        };
                    }
                    _ => {
                        let r = engine::ReverbParams::default();
                        buses.reverb = engine::ReverbParams {
                            size: number("size", r.size, 1.0)?,
                            damp: number("damp", r.damp, 1.0)?,
                            level: number("level", r.level, 1.0)?,
                        };
                    }
                }
                Ok(())
            })?,
        )?;

        let cache = cache.clone();
        let imports = imports.clone();
        let base_dir = self
            .sample_dir
            .clone()
            .unwrap_or_else(|| base_dir.to_path_buf());
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

        globals.set(
            "scale",
            lua.create_function(|lua, (name, tonic): (String, Option<String>)| {
                let scale = api::scale(&name).ok_or_else(|| {
                    let names: Vec<&str> = api::SCALES.iter().map(|s| s.name).collect();
                    fail(
                        lua,
                        format!("scale(\"{name}\"): no existe ({})", names.join(", ")),
                    )
                })?;
                let tonic = tonic.unwrap_or_else(|| "C".into());
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
    fn assign_slots(&self, song: &Song) -> [Option<String>; MAX_TRACKS] {
        let mut slots: [Option<String>; MAX_TRACKS] = Default::default();
        let mut new_names = Vec::new();
        for track in &song.tracks {
            match self.track_slot(&track.name) {
                Some(slot) => slots[slot] = Some(track.name.clone()),
                None => new_names.push(track.name.clone()),
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
fn song_batch(song: &Song, slots: &[Option<String>; MAX_TRACKS]) -> Result<Batch, ScriptError> {
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
    for (slot, assigned) in slots.iter().enumerate() {
        let pattern = match assigned {
            Some(name) => {
                let track = song.tracks.iter().find(|t| &t.name == name).unwrap();
                let pattern = Pattern::from_bars(track.sample.clone(), &track.slices, &track.bars)
                    .and_then(|p| p.with_gain(track.gain))
                    .and_then(|p| p.with_fx(&track.fx))
                    .and_then(|p| p.with_automation(&track.automation))
                    .and_then(|p| p.with_sends(track.sends))
                    .and_then(|p| p.with_shapes(&track.shapes))
                    .and_then(|p| p.with_choke(track.choke))
                    .and_then(|p| p.with_midi(track.midi))
                    .and_then(|p| p.with_instrument(track.instrument.map(|i| i.engine)))
                    .map(|p| p.with_gate(track.gate))
                    .map(|p| p.with_mute(track.mute).with_solo(track.solo))
                    .map_err(|e| ScriptError::new(format!("pista \"{name}\": {e}")))?;
                Some(pattern)
            }
            None => None,
        };
        batch.set_pattern(slot, pattern);
    }
    Ok(batch)
}

impl Song {
    /// Compases que dura la canción entera hasta repetirse: el mínimo
    /// común múltiplo de los ciclos de sus pistas (vueltas de `every`/
    /// `chance` por su longitud, y el compás de 16 pasos), y al menos
    /// `min_bars` (redondeado a ciclos completos). Como mucho
    /// [`MAX_SONG_BARS`].
    pub fn length_bars(&self, min_bars: u64) -> u64 {
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
        if !self.tracks.iter().any(|t| t.name == name) {
            return None;
        }
        let mut song = self.clone();
        for track in &mut song.tracks {
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
        for (slot, track) in slots.iter_mut().zip(&self.tracks) {
            *slot = Some(track.name.clone());
        }
        let (mut controller, mut engine) = engine::new_engine(sample_rate);
        let send = |e: SendError| ScriptError::new(format!("motor: {e}"));
        controller.apply(song_batch(self, &slots)?).map_err(send)?;
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
    mlua::Error::runtime(format!("{}{message}", location.unwrap_or_default()))
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
            transforms: Vec::new(),
            steps_set: false,
        });
    }
    if let Ok(source) = source.borrow::<InstrumentHandle>() {
        // Un solo corte: el sample entero, que no se etiqueta ni se mueve.
        let origin = SliceOrigin {
            sample: source.name.clone(),
            count: 1,
            grid: true,
            indices: vec![None],
        };
        let slices = Slices::grid(&source.sample, 1);
        let mut def = def(&source.sample, &source.path, slices, origin);
        def.instrument = Some(source.def);
        return Ok(TrackBuild {
            def,
            labels: Rc::new(RefCell::new(vec![None])),
            stale: Rc::new(Vec::new()),
            shapes: Rc::new(RefCell::new(vec![SliceShape::DEFAULT])),
            transforms: Vec::new(),
            steps_set: false,
        });
    }
    Err(fail(
        lua,
        format!(
            "track(\"{name}\", ...): el segundo argumento son cortes \
             (sample(...):slice(n)) o un instrumento (sample(...):instrument{{...}})"
        ),
    ))
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
}

/// Pista en construcción: su definición y las transformaciones pendientes.
struct TrackBuild {
    def: TrackDef,
    /// Etiquetas de los cortes, compartidas con la variable de Lua: un
    /// `:name()` posterior a `track()` vale para los `steps` siguientes.
    labels: Rc<RefCell<Vec<Option<String>>>>,
    /// Etiquetas del proyecto de otro `slice(n)` (para explicar errores).
    stale: Rc<Vec<(String, Cut)>>,
    /// Forma de los cortes, compartida con la variable como las etiquetas.
    shapes: Rc<RefCell<Vec<SliceShape>>>,
    transforms: Vec<Transform>,
    /// Ya se llamó a `:steps()` (luego `:length()` no puede cambiar la
    /// longitud).
    steps_set: bool,
}

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
}

/// Expande `every` y `chance` de una pista a su ciclo de compases.
/// Lo que necesita `expand` de la canción para humanize: el tempo (los ms
/// se pasan a pasos con él) y la semilla de la carga.
#[derive(Clone, Copy)]
struct Timing {
    bpm: f64,
    seed: u64,
}

fn expand(track: TrackBuild, rng: &mut Rng, timing: Timing) -> Result<TrackDef, ScriptError> {
    let TrackBuild {
        mut def,
        labels,
        stale,
        shapes,
        transforms,
        steps_set: _,
    } = track;
    def.labels = labels.borrow().clone();
    def.shapes = shapes.borrow().clone();
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
    for b in 0..cycle {
        let mut steps = def.steps.clone();
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
                        instrument: def.instrument.is_some(),
                    };
                    let shared = bar.steps.clone();
                    func.call::<()>(bar)
                        .map_err(|e| ScriptError::from_lua(&e))?;
                    steps = shared.borrow().clone();
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
    def.bars = bars;
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

        // `:slice(n)`: corta en los `n` transitorios más fuertes.
        methods.add_method("slice", |lua, this, n: i64| {
            if !(1..=64).contains(&n) {
                return Err(fail(
                    lua,
                    format!("slice({n}): el número de cortes va de 1 a 64"),
                ));
            }
            let slices = Slices::detect(&this.sample, n as usize);
            cut_sample(lua, this, slices, Cut::detect(n as usize))
        });

        // `:slice_grid(n)`: `n` cortes iguales (rejilla).
        methods.add_method("slice_grid", |lua, this, n: i64| {
            if !(1..=64).contains(&n) {
                return Err(fail(
                    lua,
                    format!("slice_grid({n}): el número de cortes va de 1 a 64"),
                ));
            }
            let slices = Slices::grid(&this.sample, n as usize);
            cut_sample(lua, this, slices, Cut::grid(n as usize))
        });
    }
}

/// `sample(...):instrument({ ... })` en Lua.
struct InstrumentHandle {
    name: String,
    path: PathBuf,
    sample: Arc<Sample>,
    def: InstrumentDef,
}

impl UserData for InstrumentHandle {}

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
    };
    let Some(table) = table else {
        return Ok(def);
    };
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
    Ok(def)
}

/// Termina de cortar un sample: aplica las ediciones del proyecto
/// (`partir`, `unir`), luego sus ajustes (`corte`) y sus etiquetas, todo
/// para este sample cortado así (`cut`). Lo que no cabe se avisa.
fn cut_sample(
    lua: &Lua,
    this: &SampleHandle,
    mut slices: Slices,
    cut: Cut,
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
        },
        slices,
        labels: Rc::new(RefCell::new(labels)),
        stale: Rc::new(stale),
        adjusted,
        stretch: this.stretch,
        shapes: Rc::new(RefCell::new(vec![SliceShape::DEFAULT; count])),
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
}

/// Lee la tabla de opciones de `sample(nombre, { ... })` (claves del
/// registro `api::SAMPLE_PARAMS`).
fn import_options(lua: &Lua, name: &str, table: &Table) -> mlua::Result<engine::ImportOptions> {
    let who = format!("sample(\"{name}\")");
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
    let known: Vec<&str> = api::SLICE_PARAMS.iter().map(|p| p.name).collect();
    for pair in table.pairs::<Value, Value>() {
        let (key, value) = pair?;
        let key = match key {
            Value::String(s) => s.to_str()?.to_string(),
            other => format!("{other:?}"),
        };
        let number = |v: &Value| -> mlua::Result<f32> {
            match v {
                Value::Integer(i) => Ok(*i as f32),
                Value::Number(n) => Ok(*n as f32),
                _ => Err(fail(lua, format!("{who}: {key} tiene que ser un número"))),
            }
        };
        match key.as_str() {
            "attack" => shape.attack_ms = number(&value)?,
            "release" => shape.release_ms = number(&value)?,
            "pan" => shape.pan = number(&value)?,
            "length" => shape.length = number(&value)?,
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

impl UserData for SlicedHandle {
    fn add_methods<M: UserDataMethods<Self>>(methods: &mut M) {
        methods.add_method("count", |_, this, ()| Ok(this.slices.len()));

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
            })
        });

        // `:add(ms)`: un corte nuevo que empieza en `ms` (parte en dos el que
        // lo contiene). Como `move`, las pistas definidas antes no lo ven.
        methods.add_method_mut("add", |lua, this, ms: f64| {
            if !(ms.is_finite() && ms >= 0.0) {
                return Err(fail(lua, format!("add({ms}): ms desde el principio (0 o más)")));
            }
            let frame = ms_to_frame(ms, &this.sample);
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
            shapes.insert(k, SliceShape::DEFAULT);
            this.shapes = Rc::new(RefCell::new(shapes));
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
            let frame = ms_to_frame(ms, &this.sample);
            this.slices = this
                .slices
                .with_start(i, frame, min_slice_gap(&this.sample))
                .map_err(|bounds| {
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
    /// `:steps(texto)` (y `:melody()`, que lo traduce a pasos).
    fn set_steps(&self, lua: &Lua, text: &str) -> mlua::Result<()> {
        let (labels, stale, count, length, instrument) = self.with_track(|t| {
            (
                t.labels.borrow().clone(),
                t.stale.clone(),
                t.def.origin.cut(),
                t.def.length,
                t.def.instrument.is_some(),
            )
        });
        let stale = StaleLabels {
            labels: &stale,
            count,
        };
        let steps = checked_steps(lua, &self.name, &labels, &stale, text, length, instrument)?;
        self.with_track(|t| {
            t.def.steps = steps;
            t.steps_set = true;
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
    instrument: bool,
) -> mlua::Result<Bar> {
    let slices = labels.len();
    let steps = parse_pattern_labeled(text, length, labels).map_err(|e| {
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
        if !instrument && step.chord.iter().any(Option::is_some) {
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
        let (key, _) = pair?;
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
    }
    Ok(())
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
    let curves: RefCell<Vec<(String, engine::Curve)>> = RefCell::default();
    let get = |key: &str| -> mlua::Result<Option<f32>> {
        let Some(p) = &params else { return Ok(None) };
        match p.get::<Value>(key)? {
            Value::Nil => Ok(None),
            Value::UserData(ud) if ud.is::<CurveHandle>() => {
                let curve = ud.borrow::<CurveHandle>()?.0;
                curves.borrow_mut().push((key.to_string(), curve));
                Ok(Some(curve.bounds().0))
            }
            other => lua.unpack::<f32>(other).map(Some).map_err(|_| {
                fail(
                    lua,
                    format!("fx(\"{name}\"): {key} tiene que ser un número, ramp() o lfo()"),
                )
            }),
        }
    };
    let filter = |kind: FilterKind| -> mlua::Result<Fx> {
        let cutoff_hz = get("cutoff")?
            .ok_or_else(|| fail(lua, format!("fx(\"{name}\"): falta cutoff (Hz)")))?;
        Ok(Fx::Filter {
            kind,
            cutoff_hz,
            q: get("q")?.unwrap_or(engine::DEFAULT_Q),
        })
    };
    let fx = match name {
        "filter" => {
            fx_params(lua, name, &params)?;
            let kind: Option<String> = match &params {
                Some(p) => p.get("type")?,
                None => None,
            };
            match kind.as_deref().unwrap_or("lowpass") {
                "lowpass" => filter(FilterKind::LowPass)?,
                "highpass" => filter(FilterKind::HighPass)?,
                other => {
                    return Err(fail(
                        lua,
                        format!("fx(\"filter\"): type \"{other}\" no existe (lowpass o highpass)"),
                    ));
                }
            }
        }
        "lowpass" | "highpass" => {
            fx_params(lua, name, &params)?;
            let kind = if name == "lowpass" {
                FilterKind::LowPass
            } else {
                FilterKind::HighPass
            };
            filter(kind)?
        }
        "bitcrush" | "bitcrusher" => {
            fx_params(lua, name, &params)?;
            Fx::Bitcrush {
                bits: get("bits")?.unwrap_or(8.0) as u32,
                downsample: get("downsample")?.unwrap_or(1.0) as u32,
            }
        }
        "compressor" | "comp" => {
            fx_params(lua, name, &params)?;
            Fx::Compressor {
                threshold_db: get("threshold")?.unwrap_or(-18.0),
                ratio: get("ratio")?.unwrap_or(4.0),
                attack_ms: get("attack")?.unwrap_or(5.0),
                release_ms: get("release")?.unwrap_or(100.0),
                makeup_db: get("makeup")?.unwrap_or(0.0),
            }
        }
        "distortion" | "drive" => {
            fx_params(lua, name, &params)?;
            Fx::Distortion {
                drive: get("drive")?.unwrap_or(4.0),
                mix: get("mix")?.unwrap_or(1.0),
            }
        }
        other => {
            return Err(fail(
                lua,
                format!(
                    "fx(\"{other}\"): no existe ({})",
                    names(api::EFFECTS.iter().map(|e| e.name))
                ),
            ));
        }
    };
    fx.validate()
        .map_err(|e| fail(lua, format!("fx(\"{name}\"): {e}")))?;
    let curves = curves.into_inner();
    Ok((fx, curves))
}

impl UserData for TrackHandle {
    fn add_methods<M: UserDataMethods<Self>>(methods: &mut M) {
        // Todos los métodos devuelven la pista para poder encadenar.

        // `:steps("1 . 3' ...")`: patrón base en mini-notación.
        methods.add_method("steps", |lua, this, text: String| {
            this.set_steps(lua, &text)?;
            Ok(this.clone())
        });

        // `:melody("c e g e")`: notas por su nombre, desde la raíz del
        // instrumento (en una pista de cortes, C4 es el corte 1 tal cual).
        methods.add_method("melody", |lua, this, text: String| {
            let root = this.with_track(|t| t.def.instrument.map_or(60, |i| i.root));
            let steps = patterns::notes(&text, root).map_err(|e| fail(lua, e))?;
            this.set_steps(lua, &steps)?;
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
        methods.add_method("gain", |lua, this, gain: f32| {
            if !(0.0..=MAX_GAIN).contains(&gain) {
                return Err(fail(lua, SendError::InvalidTrackGain(gain)));
            }
            this.with_track(|t| t.def.gain = gain);
            Ok(this.clone())
        });

        // `:swing(p)`: swing en porcentaje estilo MPC (50 recto, 75 máximo).
        methods.add_method("swing", |lua, this, percent: f32| {
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

        // `:send(bus, nivel)`: parte de la pista al delay o a la reverb.
        methods.add_method("send", |lua, this, (bus, level): (String, f32)| {
            let index = match bus.as_str() {
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
            if !(0.0..=engine::bus::MAX_SEND).contains(&level) {
                return Err(fail(
                    lua,
                    format!("send(\"{bus}\", {level}): el nivel va de 0 a 1"),
                ));
            }
            this.with_track(|t| t.def.sends[index] = level);
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
                let (fx, curves) = parse_fx(lua, &name, params)?;
                let count = this.with_track(|t| t.def.fx.len());
                if count >= MAX_FX {
                    return Err(fail(lua, format!("fx: máximo {MAX_FX} efectos por pista")));
                }
                // Los parámetros con ramp() o lfo(): automatizaciones.
                let mut chain = [None; MAX_FX];
                chain[count] = Some(fx);
                let mut automations = Vec::new();
                for (key, curve) in curves {
                    let Some(param) = fx.param_index(&key) else {
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
                        fx: count,
                        param,
                        curve,
                    };
                    automation
                        .validate(&chain)
                        .map_err(|e| fail(lua, format!("fx(\"{name}\"): {key}: {e}")))?;
                    automations.push(automation);
                }
                this.with_track(|t| {
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
    /// Raíz de `melody` (la del instrumento, o C4) y si la pista es un
    /// instrumento (acordes).
    root: i32,
    instrument: bool,
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
            instrument: self.instrument,
        }
    }

    fn set_steps(&self, lua: &Lua, text: &str) -> mlua::Result<()> {
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
            self.instrument,
        )?;
        *self.steps.borrow_mut() = steps;
        Ok(())
    }
}

impl UserData for BarHandle {
    fn add_methods<M: UserDataMethods<Self>>(methods: &mut M) {
        // Sustituye los pasos de este compás.
        methods.add_method("steps", |lua, this, text: String| {
            this.set_steps(lua, &text)?;
            Ok(this.handle())
        });
        // Sustituye los pasos por notas (como `pista:melody`).
        methods.add_method("melody", |lua, this, text: String| {
            let steps = patterns::notes(&text, this.root).map_err(|e| fail(lua, e))?;
            this.set_steps(lua, &steps)?;
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
