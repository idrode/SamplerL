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

use engine::notation::parse_steps;
use engine::{
    Batch, Controller, FilterKind, Fx, MAX_CYCLE_BARS, MAX_FX, MAX_GAIN, MAX_RETRIG, MAX_SEMITONES,
    MAX_TRACKS, Pattern, STEPS, Sample, SendError, Slices, Step, valid_bpm,
};
mod watch;

pub use watch::{ScriptWatcher, WatchEvent};

use mlua::{
    Function, HookTriggers, Lua, LuaOptions, StdLib, Table, UserData, UserDataMethods, UserDataRef,
    Value, VmState,
};

/// Tiempo máximo de evaluación de un script por defecto.
pub const DEFAULT_TIME_LIMIT: Duration = Duration::from_secs(2);
/// Cada cuántas instrucciones de Lua se comprueba el tiempo.
const HOOK_INSTRUCTIONS: u32 = 10_000;
/// Compases que se tiran los dados de `chance` antes de repetirse.
pub const CHANCE_CYCLE_BARS: u64 = 16;

/// Pasos de un compás.
pub type Bar = [Option<Step>; STEPS];

/// Modelo de estado que produce un script. Las vistas leerán esto.
#[derive(Clone)]
pub struct Song {
    /// Tempo pedido con `bpm()`; `None` mantiene el que haya.
    pub bpm: Option<f64>,
    /// Pistas en el orden en que se definieron.
    pub tracks: Vec<TrackDef>,
}

#[derive(Clone)]
pub struct TrackDef {
    pub name: String,
    pub sample: Arc<Sample>,
    /// Ruta del WAV tal como se resolvió.
    pub path: PathBuf,
    pub slices: Slices,
    /// Patrón base (`:steps()`).
    pub steps: Bar,
    /// Ciclo de compases con `every` y `chance` ya aplicados: en el compás
    /// `b` de la canción suena `bars[b % bars.len()]`.
    pub bars: Vec<Bar>,
    /// Volumen de la pista (`:gain()`), lineal.
    pub gain: f32,
    /// Efectos de la pista (`:fx()`), en orden.
    pub fx: Vec<Fx>,
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
        }
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

        let mut batch = Batch::new();
        if let Some(bpm) = song.bpm {
            batch.set_bpm(bpm);
        }
        for (slot, assigned) in slots.iter().enumerate() {
            let pattern = match assigned {
                Some(name) => {
                    let track = song.tracks.iter().find(|t| &t.name == name).unwrap();
                    let pattern =
                        Pattern::from_bars(track.sample.clone(), &track.slices, &track.bars)
                            .and_then(|p| p.with_gain(track.gain))
                            .and_then(|p| p.with_fx(&track.fx))
                            .map_err(|e| ScriptError::new(format!("pista \"{name}\": {e}")))?;
                    Some(pattern)
                }
                None => None,
            };
            batch.set_pattern(slot, pattern);
        }
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

        let state = Rc::new(RefCell::new(Builder {
            bpm: None,
            tracks: Vec::new(),
            expanding: Rc::new(Cell::new(false)),
        }));
        // La caché pasa al estado de Lua durante la evaluación y vuelve después.
        let cache = Rc::new(RefCell::new(std::mem::take(&mut self.samples)));
        let result = self
            .install_api(&lua, &state, &cache, base_dir)
            .and_then(|()| lua.load(source).set_name(format!("={name}")).exec());
        self.samples = std::mem::take(&mut *cache.borrow_mut());
        result.map_err(|e| ScriptError::from_lua(&e))?;

        let (bpm, tracks, expanding) = {
            let mut builder = state.borrow_mut();
            (
                builder.bpm,
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

        // Expandir `every` y `chance` a un ciclo de compases. Las funciones de
        // `every` se llaman ahora, con el estado de Lua todavía vivo.
        let seed = self.seed.unwrap_or_else(|| {
            SystemTime::now()
                .duration_since(SystemTime::UNIX_EPOCH)
                .map_or(0, |d| d.as_nanos() as u64)
        });
        let mut rng = Rng::new(seed);
        expanding.set(true);
        let expanded = tracks
            .into_iter()
            .map(|track| expand(track, &mut rng))
            .collect::<Result<Vec<_>, _>>();
        expanding.set(false);
        Ok(Song {
            bpm,
            tracks: expanded?,
        })
    }

    fn install_api(
        &self,
        lua: &Lua,
        state: &Rc<RefCell<Builder>>,
        cache: &Rc<RefCell<SampleCache>>,
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

        let cache = cache.clone();
        let base_dir = base_dir.to_path_buf();
        let rate = self.sample_rate;
        let g = guard.clone();
        globals.set(
            "sample",
            lua.create_function(move |lua, path: String| {
                g(lua, "sample")?;
                let path = base_dir.join(path);
                let sample = load_cached(&mut cache.borrow_mut(), &path, rate)
                    .map_err(|e| fail(lua, format!("{}: {e}", path.display())))?;
                Ok(SampleHandle { path, sample })
            })?,
        )?;

        let st = state.clone();
        globals.set(
            "track",
            lua.create_function(
                move |lua, (name, source): (String, Option<UserDataRef<SlicedHandle>>)| {
                    guard(lua, "track")?;
                    let mut builder = st.borrow_mut();
                    match source {
                        Some(source) => {
                            let def = TrackBuild {
                                def: TrackDef {
                                    name: name.clone(),
                                    sample: source.sample.clone(),
                                    path: source.path.clone(),
                                    slices: source.slices.clone(),
                                    steps: [None; STEPS],
                                    bars: Vec::new(),
                                    gain: 1.0,
                                    fx: Vec::new(),
                                },
                                transforms: Vec::new(),
                            };
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
                },
            )?,
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

struct Builder {
    bpm: Option<f64>,
    tracks: Vec<TrackBuild>,
    /// Activo mientras se llaman las funciones de `every`.
    expanding: Rc<Cell<bool>>,
}

/// Pista en construcción: su definición y las transformaciones pendientes.
struct TrackBuild {
    def: TrackDef,
    transforms: Vec<Transform>,
}

enum Transform {
    /// `:every(n, fn)`: en los compases 0, n, 2n... se llama `fn(compás)`.
    Every { n: u64, func: Function },
    /// `:chance(p, efecto)`: cada golpe sufre el efecto con probabilidad `p`.
    Chance { p: f64, effect: Chance },
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
fn expand(track: TrackBuild, rng: &mut Rng) -> Result<TrackDef, ScriptError> {
    let TrackBuild {
        mut def,
        transforms,
    } = track;
    let mut cycle = 1u64;
    for transform in &transforms {
        let n = match transform {
            Transform::Every { n, .. } => *n,
            Transform::Chance { .. } => CHANCE_CYCLE_BARS,
        };
        cycle = lcm(cycle, n);
        if cycle > MAX_CYCLE_BARS as u64 {
            return Err(ScriptError::new(format!(
                "pista \"{}\": every/chance dan un ciclo de {cycle} compases (máximo {MAX_CYCLE_BARS})",
                def.name
            )));
        }
    }

    let mut bars = Vec::with_capacity(cycle as usize);
    for b in 0..cycle {
        let mut steps = def.steps;
        for transform in &transforms {
            match transform {
                Transform::Every { n, func } if b % n == 0 => {
                    let bar = BarHandle {
                        steps: Rc::new(RefCell::new(steps)),
                        slices: def.slices.len(),
                        track: def.name.clone(),
                    };
                    let shared = bar.steps.clone();
                    func.call::<()>(bar)
                        .map_err(|e| ScriptError::from_lua(&e))?;
                    steps = *shared.borrow();
                }
                Transform::Every { .. } => {}
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
    def.bars = bars;
    Ok(def)
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

/// Generador pseudoaleatorio de `chance` (xorshift64*): rápido y
/// reproducible con la misma semilla.
struct Rng(u64);

impl Rng {
    fn new(seed: u64) -> Self {
        // El estado no puede ser 0.
        Rng(seed ^ 0x9e37_79b9_7f4a_7c15 | 1)
    }

    /// Número uniforme en [0, 1).
    fn next_f64(&mut self) -> f64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        let x = self.0.wrapping_mul(0x2545_f491_4f6c_dd1d);
        (x >> 11) as f64 / (1u64 << 53) as f64
    }
}

/// `sample("archivo.wav")` en Lua.
struct SampleHandle {
    path: PathBuf,
    sample: Arc<Sample>,
}

impl UserData for SampleHandle {
    fn add_methods<M: UserDataMethods<Self>>(methods: &mut M) {
        // `:slice(n)`: corta en los `n` transitorios más fuertes.
        methods.add_method("slice", |lua, this, n: i64| {
            if !(1..=64).contains(&n) {
                return Err(fail(
                    lua,
                    format!("slice({n}): el número de cortes va de 1 a 64"),
                ));
            }
            Ok(SlicedHandle {
                path: this.path.clone(),
                sample: this.sample.clone(),
                slices: Slices::detect(&this.sample, n as usize),
            })
        });
    }
}

/// `sample(...):slice(n)` en Lua: sample con sus cortes.
struct SlicedHandle {
    path: PathBuf,
    sample: Arc<Sample>,
    slices: Slices,
}

impl UserData for SlicedHandle {
    fn add_methods<M: UserDataMethods<Self>>(methods: &mut M) {
        methods.add_method("count", |_, this, ()| Ok(this.slices.len()));

        // `:slice_at(i)`: solo el corte `i` (desde 1), como fuente de un
        // único corte (en el patrón, `1` o `x`).
        methods.add_method("slice_at", |lua, this, index: i64| {
            let only = usize::try_from(index - 1)
                .ok()
                .and_then(|i| this.slices.only(i))
                .ok_or_else(|| {
                    fail(
                        lua,
                        format!(
                            "slice_at({index}): el corte no existe (hay {})",
                            this.slices.len()
                        ),
                    )
                })?;
            Ok(SlicedHandle {
                path: this.path.clone(),
                sample: this.sample.clone(),
                slices: only,
            })
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

/// Lee y valida un patrón en mini-notación para una pista con `slices` cortes.
fn checked_steps(lua: &Lua, track: &str, slices: usize, text: &str) -> mlua::Result<Bar> {
    let steps = parse_steps(text).map_err(|e| fail(lua, format!("steps: {e}")))?;
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
    }
    Ok(steps)
}

/// Mismos rangos que valida el motor, pero con la línea del script.
fn check_step(lua: &Lua, step: &Step) -> mlua::Result<()> {
    if !(0.0..=MAX_GAIN).contains(&step.gain) {
        return Err(fail(lua, "steps: ganancia fuera de rango (0 a 4)"));
    }
    if !(-MAX_SEMITONES..=MAX_SEMITONES).contains(&step.semitones) {
        return Err(fail(
            lua,
            "steps: pitch fuera de rango (-36 a 36 semitonos)",
        ));
    }
    if !(1..=MAX_RETRIG).contains(&step.retrig) {
        return Err(fail(lua, "steps: retrigger fuera de rango (1 a 16)"));
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

/// Lee los parámetros de `:fx()`; error si hay claves desconocidas.
fn fx_params(lua: &Lua, name: &str, params: &Option<Table>, known: &[&str]) -> mlua::Result<()> {
    let Some(params) = params else { return Ok(()) };
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

fn parse_fx(lua: &Lua, name: &str, params: Option<Table>) -> mlua::Result<Fx> {
    let get = |key: &str| -> mlua::Result<Option<f32>> {
        match &params {
            Some(p) => p.get::<Option<f32>>(key),
            None => Ok(None),
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
            fx_params(lua, name, &params, &["type", "cutoff", "q"])?;
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
            fx_params(lua, name, &params, &["cutoff", "q"])?;
            let kind = if name == "lowpass" {
                FilterKind::LowPass
            } else {
                FilterKind::HighPass
            };
            filter(kind)?
        }
        "bitcrush" | "bitcrusher" => {
            fx_params(lua, name, &params, &["bits", "downsample"])?;
            Fx::Bitcrush {
                bits: get("bits")?.unwrap_or(8.0) as u32,
                downsample: get("downsample")?.unwrap_or(1.0) as u32,
            }
        }
        other => {
            return Err(fail(
                lua,
                format!("fx(\"{other}\"): no existe (filter, lowpass, highpass, bitcrush)"),
            ));
        }
    };
    fx.validate()
        .map_err(|e| fail(lua, format!("fx(\"{name}\"): {e}")))?;
    Ok(fx)
}

impl UserData for TrackHandle {
    fn add_methods<M: UserDataMethods<Self>>(methods: &mut M) {
        // Todos los métodos devuelven la pista para poder encadenar.

        // `:steps("1 . 3' ...")`: patrón base en mini-notación.
        methods.add_method("steps", |lua, this, text: String| {
            let slices = this.with_track(|t| t.def.slices.len());
            let steps = checked_steps(lua, &this.name, slices, &text)?;
            this.with_track(|t| t.def.steps = steps);
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

        // `:fx(nombre, { parámetros })`: añade un efecto a la cadena.
        methods.add_method(
            "fx",
            |lua, this, (name, params): (String, Option<Table>)| {
                let fx = parse_fx(lua, &name, params)?;
                let count = this.with_track(|t| t.def.fx.len());
                if count >= MAX_FX {
                    return Err(fail(lua, format!("fx: máximo {MAX_FX} efectos por pista")));
                }
                this.with_track(|t| t.def.fx.push(fx));
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
                                "chance: efecto \"{other}\" no existe \
                                 (retrig, reverse, pitch, gain, mute)"
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

/// El compás que recibe la función de `every`.
struct BarHandle {
    steps: Rc<RefCell<Bar>>,
    slices: usize,
    track: String,
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
            slices: self.slices,
            track: self.track.clone(),
        }
    }
}

impl UserData for BarHandle {
    fn add_methods<M: UserDataMethods<Self>>(methods: &mut M) {
        // Sustituye los pasos de este compás.
        methods.add_method("steps", |lua, this, text: String| {
            let steps = checked_steps(lua, &this.track, this.slices, &text)?;
            *this.steps.borrow_mut() = steps;
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
