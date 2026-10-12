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

use engine::notation::{check_label, parse_steps_labeled, split_label};
use engine::{
    Batch, Controller, FilterKind, Fx, MAX_CYCLE_BARS, MAX_FX, MAX_GAIN, MAX_RETRIG, MAX_SEMITONES,
    MAX_TRACKS, Pattern, STEPS, Sample, SendError, Slices, Step, valid_bpm,
};
pub mod api;
pub mod patterns;
pub mod project;
mod watch;

use patterns::Rng;
pub use project::{Project, SliceLabel};
pub use watch::{ScriptWatcher, WatchEvent};

use mlua::{
    Function, HookTriggers, Lua, LuaOptions, StdLib, Table, UserData, UserDataMethods, UserDataRef,
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

/// Pasos de un compás.
pub type Bar = [Option<Step>; STEPS];

/// Modelo de estado que produce un script. Las vistas leerán esto.
#[derive(Clone)]
pub struct Song {
    /// Tempo pedido con `bpm()`; `None` mantiene el que haya.
    pub bpm: Option<f64>,
    /// Pistas en el orden en que se definieron.
    pub tracks: Vec<TrackDef>,
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
    /// Patrón base (`:steps()`).
    pub steps: Bar,
    /// Ciclo de compases con `every` y `chance` ya aplicados: en el compás
    /// `b` de la canción suena `bars[b % bars.len()]`.
    pub bars: Vec<Bar>,
    /// Volumen de la pista (`:gain()`), lineal.
    pub gain: f32,
    /// Efectos de la pista (`:fx()`), en orden.
    pub fx: Vec<Fx>,
    /// Etiqueta de cada corte de la pista (`labels[i]`, corte `i` desde 0).
    pub labels: Vec<Option<String>>,
    /// Swing (`:swing()`), en porcentaje estilo MPC: 50 es recto.
    pub swing: f32,
    /// Humanize (`:humanize()`).
    pub humanize: Option<Humanize>,
    /// Silenciada (`:mute()`).
    pub mute: bool,
    /// En solo (`:solo()`).
    pub solo: bool,
    /// De qué sample y qué `slice(n)` salen los cortes (para etiquetarlos
    /// desde la interfaz y guardarlo en el proyecto).
    pub origin: SliceOrigin,
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
    /// La `n` de `slice(n)`.
    pub count: usize,
    /// Corte del `slice(n)` (desde 0) que es cada corte de la pista: todos
    /// en orden, o uno solo con `slice_at`.
    pub indices: Vec<usize>,
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
    /// Carpeta de los samples (del proyecto); `None`: la del script.
    sample_dir: Option<PathBuf>,
    /// Tempo si el script no llama a `bpm()` (del proyecto).
    default_bpm: Option<f64>,
    /// Etiquetas de corte del proyecto (las de `:name()` mandan).
    labels: Vec<SliceLabel>,
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
        count: usize,
        slice: usize,
        name: Option<&str>,
    ) -> Result<(), String> {
        if !(1..=count).contains(&slice) {
            return Err(format!("el corte {slice} no existe (hay {count})"));
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
            tracks: Vec::new(),
            expanding: Rc::new(Cell::new(false)),
        }));
        // La caché pasa al estado de Lua durante la evaluación y vuelve después.
        let cache = Rc::new(RefCell::new(std::mem::take(&mut self.samples)));
        let result = self
            .install_api(&lua, &state, &cache, &warnings, &pattern_rng, base_dir)
            .and_then(|()| lua.load(source).set_name(format!("={name}")).exec());
        self.samples = std::mem::take(&mut *cache.borrow_mut());
        result.map_err(|e| ScriptError::from_lua(&e))?;

        let (bpm, tracks, expanding) = {
            let mut builder = state.borrow_mut();
            (
                builder.bpm.or(self.default_bpm),
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
            warnings,
        })
    }

    fn install_api(
        &self,
        lua: &Lua,
        state: &Rc<RefCell<Builder>>,
        cache: &Rc<RefCell<SampleCache>>,
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

        let cache = cache.clone();
        let base_dir = self
            .sample_dir
            .clone()
            .unwrap_or_else(|| base_dir.to_path_buf());
        let rate = self.sample_rate;
        let g = guard.clone();
        let labels = Rc::new(self.labels.clone());
        let warnings = warnings.clone();
        globals.set(
            "sample",
            lua.create_function(move |lua, name: String| {
                g(lua, "sample")?;
                let path = base_dir.join(&name);
                let sample = load_cached(&mut cache.borrow_mut(), &path, rate)
                    .map_err(|e| fail(lua, format!("{}: {e}", path.display())))?;
                Ok(SampleHandle {
                    name,
                    path,
                    sample,
                    labels: labels.clone(),
                    warnings: warnings.clone(),
                })
            })?,
        )?;

        // Funciones de patrones: devuelven texto en mini-notación. Valen
        // también dentro de `every`.
        globals.set(
            "euclid",
            lua.create_function(|lua, (k, n, hit): (i64, i64, Option<String>)| {
                patterns::euclid(k, n, hit.as_deref().unwrap_or("x")).map_err(|e| fail(lua, e))
            })?,
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
                                    labels: Vec::new(),
                                    swing: STRAIGHT_SWING,
                                    humanize: None,
                                    mute: false,
                                    solo: false,
                                    origin: source.origin.clone(),
                                },
                                labels: source.labels.clone(),
                                stale: source.stale.clone(),
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

/// Lote que lleva la canción al motor: tempo y el patrón de cada pista en su
/// ranura (`slots[i]` es el nombre de la pista de la ranura `i`).
fn song_batch(song: &Song, slots: &[Option<String>; MAX_TRACKS]) -> Result<Batch, ScriptError> {
    let mut batch = Batch::new();
    if let Some(bpm) = song.bpm {
        batch.set_bpm(bpm);
    }
    for (slot, assigned) in slots.iter().enumerate() {
        let pattern = match assigned {
            Some(name) => {
                let track = song.tracks.iter().find(|t| &t.name == name).unwrap();
                let pattern = Pattern::from_bars(track.sample.clone(), &track.slices, &track.bars)
                    .and_then(|p| p.with_gain(track.gain))
                    .and_then(|p| p.with_fx(&track.fx))
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
    /// Compases que dura la canción entera: el mínimo común múltiplo de los
    /// ciclos de sus pistas (con `every`/`chance`), y al menos `min_bars`
    /// (redondeado a ciclos completos).
    pub fn length_bars(&self, min_bars: u64) -> u64 {
        let cycle = self
            .tracks
            .iter()
            .fold(1u64, |acc, t| lcm(acc, t.bars.len().max(1) as u64));
        cycle * min_bars.div_ceil(cycle).max(1)
    }

    /// Renderiza `bars` compases de la canción offline, en un motor aparte
    /// (no toca el que está sonando). Estéreo entrelazado a `sample_rate`.
    pub fn render(&self, sample_rate: u32, bars: u64) -> Result<Vec<f32>, ScriptError> {
        let mut slots: [Option<String>; MAX_TRACKS] = Default::default();
        for (slot, track) in slots.iter_mut().zip(&self.tracks) {
            *slot = Some(track.name.clone());
        }
        let (mut controller, mut engine) = engine::new_engine(sample_rate);
        let send = |e: SendError| ScriptError::new(format!("motor: {e}"));
        controller.apply(song_batch(self, &slots)?).map_err(send)?;
        controller.start().map_err(send)?;
        let bpm = self.bpm.unwrap_or(engine::DEFAULT_BPM);
        let frames = bars * engine::bar_frames(bpm, sample_rate);
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

struct Builder {
    bpm: Option<f64>,
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
    stale: Rc<Vec<(String, usize)>>,
    transforms: Vec<Transform>,
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

/// Aplica un relleno a los últimos `count` pasos de un compás.
fn apply_fill(bar: &mut Bar, count: usize, effect: Fill) {
    let start = STEPS - count;
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
        transforms,
    } = track;
    def.labels = labels.borrow().clone();
    // Humanize cambia cada compás: el ciclo es de 16 como con `chance`.
    let mut cycle = if def.humanize.is_some() {
        CHANCE_CYCLE_BARS
    } else {
        1u64
    };
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
        let mut steps = def.steps;
        for transform in &transforms {
            match transform {
                Transform::Every { n, func } if b % n == 0 => {
                    let bar = BarHandle {
                        steps: Rc::new(RefCell::new(steps)),
                        labels: Rc::new(def.labels.clone()),
                        stale: stale.clone(),
                        count: def.origin.count,
                        track: def.name.clone(),
                    };
                    let shared = bar.steps.clone();
                    func.call::<()>(bar)
                        .map_err(|e| ScriptError::from_lua(&e))?;
                    steps = *shared.borrow();
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
    // Swing: los contratiempos de semicorchea (pasos 2, 4, 6... desde 1)
    // se retrasan.
    let swing = swing_delay(def.swing);
    if swing > 0.0 {
        for bar in &mut bars {
            for step in bar.iter_mut().skip(1).step_by(2).flatten() {
                step.delay = (step.delay + swing).min(engine::MAX_DELAY);
            }
        }
    }
    if let Some(h) = def.humanize {
        humanize(&mut bars, h, &def.name, timing);
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
struct SampleHandle {
    /// El nombre tal como se escribió (clave de las etiquetas del proyecto).
    name: String,
    path: PathBuf,
    sample: Arc<Sample>,
    /// Etiquetas de corte del proyecto.
    labels: Rc<Vec<SliceLabel>>,
    /// Avisos de la carga.
    warnings: Rc<RefCell<Vec<String>>>,
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
            let count = n as usize;
            let slices = Slices::detect(&this.sample, count);
            // Etiquetas del proyecto para este sample, pero de otro `slice(n)`:
            // no se aplican (los cortes son otros). Se avisa, y el error de una
            // de ellas en `steps` lo explica.
            let ours = |l: &&SliceLabel| l.sample == this.name;
            let stale: Vec<(String, usize)> = if this.labels.iter().filter(ours).any(|l| l.slices == count) {
                Vec::new()
            } else {
                this.labels
                    .iter()
                    .filter(ours)
                    .map(|l| (l.name.clone(), l.slices))
                    .collect()
            };
            let mut old_counts: Vec<usize> = stale.iter().map(|(_, n)| *n).collect();
            old_counts.sort_unstable();
            old_counts.dedup();
            for old in old_counts {
                let names: Vec<String> = stale
                    .iter()
                    .filter(|(_, n)| *n == old)
                    .map(|(name, _)| format!("«{name}»"))
                    .collect();
                let warning = if names.len() == 1 {
                    format!(
                        "la etiqueta {} de {} es de slice({old}) y este script usa slice({count}): no se aplica",
                        names[0], this.name
                    )
                } else {
                    format!(
                        "las etiquetas {} de {} son de slice({old}) y este script usa slice({count}): no se aplican",
                        names.join(", "),
                        this.name
                    )
                };
                let mut warnings = this.warnings.borrow_mut();
                if !warnings.contains(&warning) {
                    warnings.push(warning);
                }
            }
            // Las etiquetas del proyecto para este sample cortado así.
            let mut labels = vec![None; slices.len()];
            for label in this.labels.iter() {
                if label.sample == this.name
                    && label.slices == count
                    && let Some(slot) = labels.get_mut(label.slice - 1)
                {
                    *slot = Some(label.name.clone());
                }
            }
            Ok(SlicedHandle {
                path: this.path.clone(),
                sample: this.sample.clone(),
                origin: SliceOrigin {
                    sample: this.name.clone(),
                    count,
                    indices: (0..slices.len()).collect(),
                },
                slices,
                labels: Rc::new(RefCell::new(labels)),
                stale: Rc::new(stale),
            })
        });
    }
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
    stale: Rc<Vec<(String, usize)>>,
}

impl UserData for SlicedHandle {
    fn add_methods<M: UserDataMethods<Self>>(methods: &mut M) {
        methods.add_method("count", |_, this, ()| Ok(this.slices.len()));

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
            })
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
    labels: &'a [(String, usize)],
    count: usize,
}

/// Lee y valida un patrón en mini-notación para una pista con esas
/// etiquetas (una por corte, así que también dan el número de cortes).
fn checked_steps(
    lua: &Lua,
    track: &str,
    labels: &[Option<String>],
    stale: &StaleLabels,
    text: &str,
) -> mlua::Result<Bar> {
    let slices = labels.len();
    let steps = parse_steps_labeled(text, labels).map_err(|e| {
        // Una etiqueta del proyecto de otro `slice(n)`: decirlo.
        let hint = split_label(&e.token)
            .and_then(|(name, _)| stale.labels.iter().find(|(n, _)| n == name))
            .map_or(String::new(), |(name, old)| {
                format!(
                    "; la etiqueta «{name}» del proyecto es de slice({old}) y este script usa slice({})",
                    stale.count
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
    Ok(fx)
}

impl UserData for TrackHandle {
    fn add_methods<M: UserDataMethods<Self>>(methods: &mut M) {
        // Todos los métodos devuelven la pista para poder encadenar.

        // `:steps("1 . 3' ...")`: patrón base en mini-notación.
        methods.add_method("steps", |lua, this, text: String| {
            let (labels, stale, count) = this.with_track(|t| {
                (
                    t.labels.borrow().clone(),
                    t.stale.clone(),
                    t.def.origin.count,
                )
            });
            let stale = StaleLabels {
                labels: &stale,
                count,
            };
            let steps = checked_steps(lua, &this.name, &labels, &stale, &text)?;
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
                if !(1..=STEPS as i64).contains(&count) {
                    return Err(fail(lua, format!("{who}: los pasos van de 1 a {STEPS}")));
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

/// El compás que recibe la función de `every`.
struct BarHandle {
    steps: Rc<RefCell<Bar>>,
    labels: Rc<Vec<Option<String>>>,
    stale: Rc<Vec<(String, usize)>>,
    count: usize,
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
            labels: self.labels.clone(),
            stale: self.stale.clone(),
            count: self.count,
            track: self.track.clone(),
        }
    }
}

impl UserData for BarHandle {
    fn add_methods<M: UserDataMethods<Self>>(methods: &mut M) {
        // Sustituye los pasos de este compás.
        methods.add_method("steps", |lua, this, text: String| {
            let stale = StaleLabels {
                labels: &this.stale,
                count: this.count,
            };
            let steps = checked_steps(lua, &this.track, &this.labels, &stale, &text)?;
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
