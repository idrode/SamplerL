//! Scripting en Lua para el sampler.
//!
//! El script no toca el audio: al evaluarse construye un modelo de estado
//! ([`Song`]: tempo y pistas con sus patrones) y [`ScriptHost::load`] lo
//! envía al motor como un lote atómico ([`engine::Batch`]) por la cola sin
//! bloqueos. Los cambios entran al empezar el siguiente compás. Si el script
//! falla (sintaxis, error en tiempo de ejecución, notación inválida, bucle
//! infinito...), no se envía nada y sigue sonando lo anterior.
//!
//! API (ver CLAUDE.md):
//!
//! ```lua
//! bpm(170)
//! local amen = sample("amen.wav"):slice(8)
//! track("drums", amen):steps("1 . . 1  . . 3 .  1 . 5 .  . 3 . 6")
//! track("drums"):steps("1 . 3' .  1^7 . 5@0.5 .  . 3 . 6  . . . .")
//! ```

use std::cell::RefCell;
use std::collections::HashMap;
use std::fmt;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime};

use engine::notation::parse_steps;
use engine::{
    Batch, Controller, MAX_TRACKS, Pattern, STEPS, Sample, SendError, Slices, Step, valid_bpm,
};
use mlua::{
    HookTriggers, Lua, LuaOptions, StdLib, UserData, UserDataMethods, UserDataRef, VmState,
};

/// Tiempo máximo de evaluación de un script por defecto.
pub const DEFAULT_TIME_LIMIT: Duration = Duration::from_secs(2);
/// Cada cuántas instrucciones de Lua se comprueba el tiempo.
const HOOK_INSTRUCTIONS: u32 = 10_000;

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
    pub steps: [Option<Step>; STEPS],
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
        }
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
                        Pattern::from_steps(track.sample.clone(), &track.slices, track.steps)
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
        }));
        // La caché pasa al estado de Lua durante la evaluación y vuelve después.
        let cache = Rc::new(RefCell::new(std::mem::take(&mut self.samples)));
        let result = self
            .install_api(&lua, &state, &cache, base_dir)
            .and_then(|()| lua.load(source).set_name(format!("={name}")).exec());
        self.samples = std::mem::take(&mut *cache.borrow_mut());
        result.map_err(|e| ScriptError::from_lua(&e))?;

        let builder = state.borrow();
        if builder.tracks.len() > MAX_TRACKS {
            return Err(ScriptError::new(format!(
                "{name}: hay {} pistas y el máximo es {MAX_TRACKS}",
                builder.tracks.len()
            )));
        }
        Ok(Song {
            bpm: builder.bpm,
            tracks: builder.tracks.clone(),
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

        let st = state.clone();
        globals.set(
            "bpm",
            lua.create_function(move |lua, bpm: f64| {
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
        globals.set(
            "sample",
            lua.create_function(move |lua, path: String| {
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
                    let mut builder = st.borrow_mut();
                    match source {
                        Some(source) => {
                            let def = TrackDef {
                                name: name.clone(),
                                sample: source.sample.clone(),
                                path: source.path.clone(),
                                slices: source.slices.clone(),
                                steps: [None; STEPS],
                            };
                            // Redefinir una pista en el mismo script la reemplaza.
                            match builder.tracks.iter_mut().find(|t| t.name == name) {
                                Some(existing) => *existing = def,
                                None => builder.tracks.push(def),
                            }
                        }
                        None if builder.tracks.iter().any(|t| t.name == name) => {}
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
    tracks: Vec<TrackDef>,
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
    }
}

/// `track(nombre, ...)` en Lua.
struct TrackHandle {
    name: String,
    state: Rc<RefCell<Builder>>,
}

impl UserData for TrackHandle {
    fn add_methods<M: UserDataMethods<Self>>(methods: &mut M) {
        // `:steps("1 . 3' ...")`: patrón en mini-notación. Devuelve la pista
        // para poder encadenar.
        methods.add_method("steps", |lua, this, text: String| {
            let steps = parse_steps(&text).map_err(|e| fail(lua, format!("steps: {e}")))?;
            let mut builder = this.state.borrow_mut();
            let track = builder
                .tracks
                .iter_mut()
                .find(|t| t.name == this.name)
                .expect("la pista existe desde track()");
            // Validar ya (cortes que existen, rangos) para dar la línea exacta.
            Pattern::from_steps(track.sample.clone(), &track.slices, steps).map_err(|e| {
                let e = match e {
                    SendError::NoSuchSlice { index, count } => format!(
                        "el corte {} no existe (\"{}\" tiene {count})",
                        index + 1,
                        this.name
                    ),
                    other => other.to_string(),
                };
                fail(lua, format!("steps: {e}"))
            })?;
            track.steps = steps;
            Ok(TrackHandle {
                name: this.name.clone(),
                state: this.state.clone(),
            })
        });
    }
}
