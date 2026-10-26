//! Estado y lógica de la interfaz, sin terminal (se prueba sin pantalla).

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime};

use engine::{Controller, MAX_TRACKS, Sample, Status};
use ratatui_textarea::{CursorMove, Input, Key, TextArea};
use script::blocks::{self, Library, SaveError, Scope};
use script::{Project, ScriptError, ScriptHost, api};

use crate::complete::{self, Candidate};
use crate::config::{Bound, Config, Keymap};
use crate::diagnose;
use crate::editing;
use crate::model::{
    BlockPickerView, BrowserView, ClashRow, ClashTableView, CompletionView, ErrorView, GroupView,
    InstrumentView, MasterView, PaletteView, SlotsView, SynthView, TimelineView, TrackView,
    TransportView, ViewModel, Waveform, error_line,
};
use crate::palette::{self, Action, Item};

mod cue;
mod macros;
mod mouse;
mod vim;
pub use mouse::MouseAction;
pub use vim::DEFAULT_EXIT as VIM_EXIT;
mod split;
#[cfg(test)]
pub use split::editor_command;
mod tracker_edit;

/// Plantilla para un script nuevo.
const TEMPLATE: &str = r#"bpm(170)

local amen = sample("amen.wav"):slice(8)

track("drums", amen)
  :steps("1 . . 1  . . 3 .  1 . 5 .  . 3 . 6")
"#;

/// Cada cuánto se mira si el archivo cambió fuera del editor.
const EXTERNAL_CHECK: Duration = Duration::from_millis(500);

/// Cuánto se mantiene a la vista un aviso de limitador o saturación.
const WARNING_HOLD: Duration = Duration::from_secs(1);

/// Caída del medidor de nivel por refresco.
const METER_DECAY: f32 = 0.85;

/// Teclas del modo tocar: la fila de números dispara los cortes 1 a 10 y
/// q-i los cortes 11 a 18.
pub const PADS: [char; 18] = [
    '1', '2', '3', '4', '5', '6', '7', '8', '9', '0', 'q', 'w', 'e', 'r', 't', 'y', 'u', 'i',
];

/// Teclas de las notas en vivo que vienen de los pads (las de 0 a 127 son
/// las notas MIDI): la del pad `i` es `PAD_KEY + i`.
const PAD_KEY: u8 = 128;
/// Sin eventos de soltar tecla (la mayoría de las terminales, tmux), una
/// nota de un pad suena mientras llegan sus repeticiones de teclado: se
/// suelta si no llega ninguna en este tiempo tras pulsarla (la primera
/// repetición tarda lo que el retardo del teclado)...
const PAD_FIRST_HOLD: Duration = Duration::from_millis(700);
/// ...o en este tras la última repetición.
const PAD_REPEAT_HOLD: Duration = Duration::from_millis(150);
/// Desafinación (en cents) a partir de la cual «Notas de los cortes» la
/// escribe junto a la nota.
const CENTS_SHOWN: i32 = 10;

/// Una nota en vivo de un instrumento que todavía suena (de un pad o del
/// MIDI de entrada).
#[derive(Debug, Clone)]
struct HeldNote {
    /// Tecla en el motor (`PAD_KEY + i` o la nota MIDI).
    key: u8,
    /// Pista del motor y nombre de la pista.
    slot: usize,
    track: String,
    /// Último evento de la tecla (la pulsación o su última repetición).
    last: Instant,
    repeated: bool,
    /// Paso en que se grabó, si se está grabando esa pista.
    recorded: Option<usize>,
    /// Es una nube de granos (se suelta con `grain_off`), no una nota.
    grain: bool,
}

/// Modo de la interfaz: escribir en el editor o tocar cortes con el teclado.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Mode {
    #[default]
    Edit,
    Play,
}

/// Autocompletado abierto.
#[derive(Debug, Clone)]
struct Completion {
    /// Lo ya escrito que se sustituye al aceptar.
    prefix: String,
    /// Opciones que empiezan por el prefijo (nunca vacío).
    matches: Vec<Candidate>,
    selected: usize,
    /// Abierto con F4: sigue abierto aunque no haya prefijo.
    manual: bool,
    /// Se ha movido la selección con las flechas: solo entonces Enter
    /// acepta (si no, es un salto de línea).
    navigated: bool,
}

/// Acción compuesta de varios cambios del editor (mover líneas, insertar
/// una plantilla...): se deshace y se rehace de una vez.
#[derive(Debug, Clone)]
struct EditGroup {
    before: Vec<String>,
    after: Vec<String>,
}

/// Paleta de comandos abierta (F1).
#[derive(Debug, Clone, Default)]
struct Palette {
    query: String,
    selected: usize,
}

/// Qué hace el selector de bloques con el elegido.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BlockMode {
    Insert,
    /// Sustituir su texto por la selección (conserva la cabecera).
    Update,
    Delete,
}

/// Selector de bloques abierto: lista filtrable con vista previa.
#[derive(Debug, Clone)]
struct BlockPicker {
    mode: BlockMode,
    query: String,
    selected: usize,
    /// Texto nuevo al actualizar (la selección al abrirlo, sin sangría
    /// común).
    body: String,
    /// Bloque elegido una vez al actualizar o borrar: Enter otra vez lo
    /// confirma.
    armed: Option<String>,
}

/// Selector de versiones del historial abierto.
struct VersionPicker {
    query: String,
    selected: usize,
    /// Las versiones (de la más nueva a la más antigua) con su texto.
    versions: Vec<(crate::history::Version, String)>,
}

/// Informe de choques de frecuencia abierto: el análisis y sus choques
/// (del más fuerte al más débil).
struct MaskingPicker {
    query: String,
    selected: usize,
    report: script::masking::MaskingReport,
    clashes: Vec<script::masking::Clash>,
    /// Pista cuya sugerencia espera el segundo Enter para insertarse.
    confirm: Option<usize>,
}

/// Explorador de samples abierto.
#[derive(Clone, Default)]
struct Browser {
    query: String,
    selected: usize,
    /// WAV de la carpeta de muestras (rutas relativas).
    files: Vec<String>,
    /// Samples ya leídos para la preescucha (por ruta).
    loaded: HashMap<String, Arc<Sample>>,
    /// Lo que se sabe del elegido (duración, error al leerlo...).
    info: String,
}

/// Número de ranuras rápidas.
pub const SLOTS: usize = 9;

/// Qué se hace con la ranura cuyo número se pulse.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SlotOp {
    /// Guardar la selección (o la línea del cursor).
    Yank,
    Paste,
}

/// Qué hacer tras una tecla.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Flow {
    Continue,
    Quit,
}

pub struct App {
    pub editor: TextArea<'static>,
    /// Dónde se dibujó el texto del editor la última vez (el ratón lo usa
    /// para saber qué se pulsa); `None` si no se dibujó.
    pub editor_area: Option<ratatui::layout::Rect>,
    /// Arrastrando con el ratón: dónde se pulsó (fila y columna del texto).
    drag_from: Option<(usize, usize)>,
    /// Dónde se dibujó la forma de onda la última vez (dentro del borde):
    /// el ratón arrastra ahí las marcas de los cortes.
    pub sample_area: Option<ratatui::layout::Rect>,
    /// Arrastrando la marca de un corte con el ratón.
    slice_drag: Option<mouse::SliceDrag>,
    /// El ratón en el editor (`[editor] raton`).
    mouse_enabled: bool,
    /// El modo vim (`[editor] modo = "vim"`): su modo y lo que lleva entre
    /// teclas; `None`, el editor de siempre.
    vim: Option<vim::Vim>,
    /// Las dos teclas que salen de Insert (`[editor] salir_insert`).
    vim_exit: [char; 2],
    /// Portapapeles del sistema (OSC 52 y `wl-copy`/`wl-paste`).
    pub(crate) clipboard: crate::clipboard::Clipboard,
    path: PathBuf,
    /// Proyecto abierto o guardado (`.sampler`), si lo hay.
    project: Option<PathBuf>,
    sample_rate: u32,
    /// Línea de órdenes abierta (F2) y su texto.
    prompt: Option<String>,
    /// Contenido guardado en disco (para saber si hay cambios).
    saved: String,
    host: ScriptHost,
    controller: Controller,
    error: Option<ScriptError>,
    message: String,
    started: bool,
    /// Compás en que empieza el siguiente arranque (ver `app::cue`).
    cue: Option<u64>,
    selected: usize,
    /// Corte seleccionado de la pista seleccionada (desde 0): el último
    /// tocado en el modo tocar o el elegido desde la paleta.
    selected_slice: usize,
    /// Metrónomo encendido.
    metronome: bool,
    /// Mute y solo en vivo, por nombre de pista (sobreviven a las recargas).
    muted: Vec<String>,
    soloed: Vec<String>,
    /// Lo último enviado al motor por pista: (mute, solo).
    sent_mix: [(bool, bool); MAX_TRACKS],
    status: Option<Status>,
    meters: [f32; MAX_TRACKS],
    /// Medidor de cada grupo (pico con caída).
    group_meters: [f32; engine::MAX_GROUPS],
    /// Panel de las macros abierto, con la elegida.
    macro_panel: Option<usize>,
    /// Medidor del master (pico antes del limitador, con caída; puede
    /// pasar de 1).
    master_meter: f32,
    /// Hasta cuándo se muestran los avisos de limitador, saturación del
    /// master y saturación de cada pista.
    limiting_until: Option<Instant>,
    clipping_until: Option<Instant>,
    track_clip_until: [Option<Instant>; MAX_TRACKS],
    /// Se pidió salir con cambios sin guardar: otra vez para confirmar.
    quit_armed: bool,
    mode: Mode,
    /// Contenido del disco distinto del último guardado o cargado por el
    /// editor: el archivo cambió fuera. Se ofrece recargar (F3).
    external: Option<String>,
    last_external_check: Instant,
    /// Ctrl-S con un cambio externo pendiente: otra vez para sobrescribir.
    overwrite_armed: bool,
    /// Resúmenes de forma de onda por sample (se guarda el `Arc` para que la
    /// dirección no se reutilice mientras está en la caché).
    waveforms: HashMap<usize, (Arc<Sample>, Arc<Waveform>)>,
    /// Nota de cada corte, por sample y cortes (ver `slice_notes_key`); el
    /// `Arc<Sample>` mantiene vivo el puntero de la clave.
    slice_notes: HashMap<SliceNotesKey, (Arc<Sample>, Vec<Option<engine::NoteGuess>>)>,
    /// Autocompletado abierto (popup junto al cursor).
    completion: Option<Completion>,
    /// Campos de la última plantilla que faltan por rellenar; Tab salta
    /// al siguiente.
    marks: editing::Fields,
    /// La selección del editor es un parámetro marcado por una plantilla.
    mark_selected: bool,
    /// Paleta de comandos abierta (F1).
    palette: Option<Palette>,
    /// Última entrada usada de la paleta (sale la primera).
    last_palette: Option<Item>,
    /// Acciones compuestas que se pueden deshacer y rehacer enteras.
    undo_groups: Vec<EditGroup>,
    redo_groups: Vec<EditGroup>,
    /// Carpeta global de bloques (la pone `main`; sin ella, solo los del
    /// proyecto).
    global_blocks: Option<PathBuf>,
    /// Biblioteca de bloques (global y del proyecto).
    blocks: Library,
    /// Selector de bloques abierto.
    block_picker: Option<BlockPicker>,
    /// Carpeta de los historiales (la pone `main`; sin ella, no se guarda
    /// nada) y el selector de versiones abierto.
    history_root: Option<PathBuf>,
    versions: Option<VersionPicker>,
    /// Informe de choques de frecuencia abierto.
    masking: Option<MaskingPicker>,
    /// El último análisis de choques (también sin choques) y cuántas
    /// cargas se llevaban al hacerlo.
    last_masking: Option<(script::masking::MaskingReport, u64)>,
    /// Cargas que han entrado bien (y cambios de variante): lo que sabe si
    /// el último análisis es de lo que suena.
    loads: u64,
    /// Explorador de samples abierto.
    browser: Option<Browser>,
    /// Cursor del tracker para editar los efectos de los pasos, y el paso
    /// que se edita en la línea de órdenes (ver `tracker_edit`).
    tracker_edit: Option<tracker_edit::TrackerCursor>,
    pending_edit: Option<tracker_edit::PendingEdit>,
    /// Grabación de lo tocado en curso.
    recording: Option<crate::record::Recording>,
    /// Notas en vivo de instrumentos que suenan (pads y MIDI).
    held: Vec<HeldNote>,
    /// La terminal avisa al soltar una tecla: los pads sueltan su nota con
    /// ese evento (si no, por tiempo: ver `PAD_FIRST_HOLD`).
    key_releases: bool,
    /// Tap tempo en curso (espacio golpea, Enter pone el tempo).
    tapping: Option<crate::tap::TapTempo>,
    /// Ranuras rápidas 1-9 (como los registros de vim).
    slots: [Option<String>; SLOTS],
    /// Carpeta donde se guardan las ranuras entre sesiones (`N.txt`).
    slots_dir: Option<PathBuf>,
    /// Esperando el número de ranura.
    slot_prompt: Option<SlotOp>,
    /// Atajos de teclado (los de la configuración o los de siempre).
    keys: Keymap,
    /// Lo último buscado (F6 y F7 lo repiten).
    search: Option<String>,
    /// Entrada MIDI: qué se hace con ella, el tempo del reloj y la cola
    /// por la que llegan los mensajes del puerto (sin bloqueos).
    midi: crate::midi::MidiConfig,
    /// La expresión por nota de lo que entra por MIDI (fase Q) y los
    /// cambios que esperan a ir al motor.
    expression: crate::expression::Expression,
    expr_changes: Vec<crate::expression::Change>,
    clock: crate::midi::ClockTempo,
    midi_in: Option<rtrb::Consumer<crate::midi::TimedEvent>>,
    /// Destino de las notas que salen de las pistas con `:midi()`.
    midi_out: Option<Box<dyn crate::midi::MidiSink>>,
    /// Con el editor en su propia ventana: de qué lado está esta (ver
    /// `split`). Sin él, una sola ventana como siempre.
    link: Option<split::Link>,
    /// Comando que abre una terminal (`[editor] terminal`), para sacar el
    /// editor a su ventana, y la carpeta de los sockets (`None`: la de
    /// siempre, `link::runtime_dir`).
    editor_terminal: Option<Vec<String>>,
    socket_dir: Option<PathBuf>,
    /// Resaltado en vivo del código al sonar: el texto de la última carga
    /// buena (solo se resalta si el editor tiene ese mismo texto) y los
    /// últimos pasos de cada pista que ha publicado el motor.
    loaded_text: Option<String>,
    cursors: crate::highlight::Cursors,
    /// El resaltado está encendido (`[resaltado] activo` y la paleta) y lo
    /// que se le suma al retardo de la salida (`retardo_ms`).
    highlighting: bool,
    highlight_delay_ms: f64,
}

impl App {
    /// Abre el script (o una plantilla si no existe) y lo carga.
    pub fn open(path: &Path, controller: Controller, sample_rate: u32) -> std::io::Result<App> {
        let mut app = App::build(path, controller, sample_rate)?;
        app.load();
        Ok(app)
    }

    /// Abre el script sin cargarlo.
    fn build(path: &Path, controller: Controller, sample_rate: u32) -> std::io::Result<App> {
        let (text, saved) = read_script(path)?;
        let mut app = App {
            editor: TextArea::new(text.lines().map(String::from).collect()),
            editor_area: None,
            drag_from: None,
            sample_area: None,
            slice_drag: None,
            mouse_enabled: true,
            vim: None,
            vim_exit: vim::DEFAULT_EXIT,
            clipboard: crate::clipboard::Clipboard::system(),
            path: path.to_path_buf(),
            project: None,
            sample_rate,
            prompt: None,
            saved,
            host: ScriptHost::new(sample_rate),
            controller,
            error: None,
            message: String::new(),
            started: false,
            cue: None,
            selected: 0,
            selected_slice: 0,
            metronome: false,
            muted: Vec::new(),
            soloed: Vec::new(),
            sent_mix: [(false, false); MAX_TRACKS],
            status: None,
            meters: [0.0; MAX_TRACKS],
            group_meters: [0.0; engine::MAX_GROUPS],
            macro_panel: None,
            master_meter: 0.0,
            limiting_until: None,
            clipping_until: None,
            track_clip_until: [None; MAX_TRACKS],
            quit_armed: false,
            mode: Mode::Edit,
            external: None,
            last_external_check: Instant::now(),
            overwrite_armed: false,
            waveforms: HashMap::new(),
            slice_notes: HashMap::new(),
            completion: None,
            marks: editing::Fields::default(),
            mark_selected: false,
            palette: None,
            last_palette: None,
            undo_groups: Vec::new(),
            redo_groups: Vec::new(),
            global_blocks: None,
            blocks: Library::default(),
            block_picker: None,
            history_root: None,
            versions: None,
            masking: None,
            last_masking: None,
            loads: 0,
            editor_terminal: None,
            socket_dir: None,
            browser: None,
            tracker_edit: None,
            pending_edit: None,
            tapping: None,
            recording: None,
            held: Vec::new(),
            key_releases: false,
            slots: Default::default(),
            slots_dir: None,
            slot_prompt: None,
            keys: Keymap::default(),
            search: None,
            midi: crate::midi::MidiConfig::default(),
            expression: crate::expression::Expression::new(&crate::midi::MidiConfig::default()),
            expr_changes: Vec::new(),
            clock: crate::midi::ClockTempo::default(),
            midi_in: None,
            midi_out: None,
            link: None,
            loaded_text: None,
            cursors: Default::default(),
            highlighting: true,
            highlight_delay_ms: 0.0,
        };
        app.reload_blocks();
        Ok(app)
    }

    /// Abre un proyecto (`.sampler`): su script, su carpeta de samples y su
    /// tempo por defecto.
    pub fn open_project(
        path: &Path,
        controller: Controller,
        sample_rate: u32,
    ) -> Result<App, Box<dyn std::error::Error>> {
        let project = Project::load(path)?;
        let mut app = App::open(&project.script, controller, sample_rate)?;
        app.apply_project(path, &project)?;
        Ok(app)
    }

    /// Pone el proyecto en el editor y el motor (entra en el compás
    /// siguiente, como cualquier carga).
    fn apply_project(&mut self, path: &Path, project: &Project) -> std::io::Result<()> {
        self.set_project(path, project)?;
        self.load();
        Ok(())
    }

    /// Pone el proyecto en el editor, sin cargarlo.
    fn set_project(&mut self, path: &Path, project: &Project) -> std::io::Result<()> {
        let (text, saved) = read_script(&project.script)?;
        self.editor = TextArea::new(text.lines().map(String::from).collect());
        self.path = project.script.clone();
        self.saved = saved;
        self.external = None;
        self.overwrite_armed = false;
        self.project = Some(path.to_path_buf());
        self.host.set_sample_dir(project.samples.clone());
        self.host.set_default_bpm(project.bpm);
        self.host.set_labels(project.labels.clone());
        self.host.set_positions(project.positions.clone());
        self.host.set_edits(project.edits.clone());
        self.reload_blocks();
        Ok(())
    }

    /// Carpeta respecto a la que se resuelven las rutas de las órdenes: la
    /// del proyecto o, si no hay, la del script.
    fn base_dir(&self) -> PathBuf {
        self.project
            .as_deref()
            .unwrap_or(&self.path)
            .parent()
            .unwrap_or(Path::new("."))
            .to_path_buf()
    }

    fn stem(&self) -> String {
        self.path
            .file_stem()
            .map_or_else(|| "canción".into(), |s| s.to_string_lossy().into())
    }

    /// Aplica la configuración del usuario (atajos; el tema lo pone `main`
    /// en el hilo que dibuja). Sus avisos van al mensaje.
    pub fn set_config(&mut self, config: &Config) {
        self.editor_terminal = config.editor_terminal.clone();
        self.mouse_enabled = config.editor.mouse;
        self.vim_exit = config.editor.insert_exit;
        // El modo vim se queda como estaba si ya lo era (con su modo).
        self.vim = match (config.editor.vim, self.vim.take()) {
            (false, _) => None,
            (true, Some(mut vim)) => {
                vim.set_exit(config.editor.insert_exit);
                Some(vim)
            }
            (true, None) => Some(vim::Vim::new(config.editor.insert_exit)),
        };
        self.keys = config.keys.clone();
        self.midi = config.midi;
        self.expression = crate::expression::Expression::new(&config.midi);
        self.highlighting = config.highlight.enabled;
        self.highlight_delay_ms = config.highlight.delay_ms;
        if !config.warnings.is_empty() {
            self.message = config
                .warnings
                .iter()
                .map(|w| format!("⚠ {w}"))
                .collect::<Vec<_>>()
                .join(" · ");
        }
    }

    /// Pone la carpeta global de bloques y relee la biblioteca.
    pub fn set_global_blocks(&mut self, dir: Option<PathBuf>) {
        self.global_blocks = dir;
        self.reload_blocks();
    }

    /// Relee los bloques de la carpeta global y la del proyecto (o del
    /// script).
    fn reload_blocks(&mut self) {
        self.blocks = Library::open(
            self.global_blocks.clone(),
            Some(blocks::project_dir(&self.base_dir())),
        );
    }

    /// Ejecuta una orden de la línea de órdenes (F2).
    pub fn run_command(&mut self, line: &str) {
        let line = line.trim();
        // Las órdenes de vim (`:w`), aquí también en la ventana del editor.
        if let Some(rest) = line.strip_prefix(':')
            && self.vim.is_some()
        {
            self.vim_command(rest);
            return;
        }
        if self.link_command(line) {
            return;
        }
        let mut words = line.split_whitespace();
        let Some(command) = words.next() else { return };
        let args: Vec<&str> = words.collect();
        let rest = line[command.len()..].trim();
        match command {
            "exportar" | "export" => self.export(&args),
            "guardar" | "save" => self.save_project(args.first().copied()),
            "abrir" | "open" => self.open_project_command(args.first().copied(), false),
            "abrir!" | "open!" => self.open_project_command(args.first().copied(), true),
            "muestras" | "samples" => self.set_samples(args.first().copied()),
            "etiquetar" | "label" => self.label_slice(&args),
            tracker_edit::COMMAND => self.run_step_fx_command(rest),
            "bpm" => self.set_bpm_line(args.first().copied()),
            "grabar" | "record" => self.start_recording(args.first().copied()),
            "seccion" | "sección" | "section" => self.jump_to_section(rest),
            "linea" | "línea" | "line" => self.goto_line(rest),
            "buscar" | "find" => self.find(rest),
            "remuestrear" | "remuestrear!" | "resample" => {
                self.resample(&args, command.ends_with('!'))
            }
            "bloque" | "bloque!" | "bloque-proyecto" | "bloque-proyecto!" => {
                self.save_block(command, rest)
            }
            other => {
                self.message = format!(
                    "orden desconocida «{other}» (exportar, guardar, abrir, muestras, etiquetar, \
                     bpm, bloque, remuestrear, seccion, linea, buscar)"
                );
            }
        }
    }

    /// `exportar [archivo.wav] [compases]`: renderiza offline lo último
    /// cargado (en un motor aparte, sin tocar lo que suena) y lo guarda.
    fn export(&mut self, args: &[&str]) {
        // `metronomo` en cualquier sitio añade el clic; el resto, por orden.
        let click = ["metronomo", "metrónomo", "clic"];
        let metronome = args.iter().any(|a| click.contains(a));
        let args: Vec<&str> = args
            .iter()
            .copied()
            .filter(|a| !click.contains(a))
            .collect();
        let args = &args[..];
        let Some(song) = self.host.song() else {
            self.message = "no hay nada cargado que exportar".into();
            return;
        };
        let mut name = args
            .first()
            .map_or_else(|| format!("{}-mezcla.wav", self.stem()), |a| a.to_string());
        if Path::new(&name).extension().is_none() {
            name.push_str(".wav");
        }
        let bars = match args.get(1) {
            Some(text) => match text.parse::<u64>() {
                Ok(bars) if (1..=256).contains(&bars) => bars,
                _ => {
                    self.message = format!("compases no válidos: {text} (1 a 256)");
                    return;
                }
            },
            None => song.length_bars(4),
        };
        let path = self.base_dir().join(&name);
        let bpm = song.bpm.unwrap_or(engine::DEFAULT_BPM);
        let result = song
            .render_with_metronome(self.sample_rate, bars, metronome)
            .map_err(|e| e.to_string())
            .and_then(|audio| {
                engine::offline::write_wav(&path, &audio, 2, self.sample_rate)
                    .map(|()| audio.len() / 2)
                    .map_err(|e| e.to_string())
            });
        let mut note = if self.dirty() || self.error.is_some() {
            " (lo último cargado bien, no el texto actual)"
        } else {
            ""
        }
        .to_string();
        if metronome {
            note.push_str(" con metrónomo");
        }
        self.message = match result {
            Ok(frames) => format!(
                "exportado {}: {} a {bpm} BPM ({:.1} s){note}",
                path.display(),
                script::bars_text(bars as u64),
                frames as f64 / self.sample_rate as f64
            ),
            Err(e) => format!("no se pudo exportar: {e}"),
        };
    }

    /// `guardar [proyecto.sampler]`: guarda el script y el proyecto.
    fn save_project(&mut self, target: Option<&str>) {
        let path = match target {
            Some(name) => {
                let mut path = self.base_dir().join(name);
                if path.extension().is_none() {
                    path.set_extension(script::project::EXTENSION);
                }
                path
            }
            None => self.project.clone().unwrap_or_else(|| {
                self.base_dir()
                    .join(format!("{}.{}", self.stem(), script::project::EXTENSION))
            }),
        };
        if self.dirty() || self.external.is_some() {
            self.save();
            if self.external.is_some() || self.dirty() {
                // `save` pidió confirmación (cambio externo) o falló.
                return;
            }
        }
        let bpm = self
            .host
            .song()
            .and_then(|s| s.bpm)
            .or(self.host.default_bpm());
        self.message = match self.project_with(bpm).save(&path) {
            Ok(()) => {
                self.project = Some(path.clone());
                format!("proyecto guardado en {}", path.display())
            }
            Err(e) => format!("no se pudo guardar el proyecto: {e}"),
        };
    }

    /// El proyecto con el estado actual (script, samples, etiquetas) y ese
    /// tempo.
    fn project_with(&self, bpm: Option<f64>) -> Project {
        Project {
            script: self.path.clone(),
            samples: self.host.sample_dir().map(Path::to_path_buf),
            bpm,
            labels: self.host.labels().to_vec(),
            positions: self.host.positions().to_vec(),
            edits: self.host.edits().to_vec(),
        }
    }

    /// `etiquetar corte [nombre]`: pone (o, sin nombre, quita) la etiqueta
    /// de un corte de la pista seleccionada. Se guarda en el proyecto abierto
    /// (si no hay, con `guardar`) y se recarga para que se vea y se pueda usar
    /// en `steps`.
    fn label_slice(&mut self, args: &[&str]) {
        let usage = "uso: etiquetar corte nombre (sin nombre la quita)";
        let Some(slice) = args.first().and_then(|a| a.parse::<usize>().ok()) else {
            self.message = usage.into();
            return;
        };
        let name = args.get(1).copied();
        if args.len() > 2 {
            self.message = usage.into();
            return;
        }
        let Some(track) = self.tracks().get(self.selected) else {
            self.message = "no hay pistas".into();
            return;
        };
        let (track_name, origin) = (track.name.clone(), track.origin.clone());
        let Some(&index) = slice.checked_sub(1).and_then(|i| origin.indices.get(i)) else {
            self.message = format!(
                "{track_name} no tiene corte {slice} (tiene {})",
                origin.indices.len()
            );
            return;
        };
        let Some(index) = index else {
            self.message = format!(
                "el corte {slice} de {track_name} lo añade el script con :add(): etiquétalo con :name()"
            );
            return;
        };
        if let Err(e) = self
            .host
            .set_label(&origin.sample, origin.cut(), index + 1, name)
        {
            self.message = format!("no se pudo etiquetar: {e}");
            return;
        }
        let saved = self.save_project_file();
        self.load();
        let what = match name {
            Some(name) => format!("«{name}»"),
            None => "sin etiqueta".into(),
        };
        let mut message = format!("corte {slice} de {track_name}: {what}{saved}");
        // Si el script nombra ese corte con :name(), manda el script.
        let now = self
            .tracks()
            .iter()
            .find(|t| t.name == track_name)
            .and_then(|t| t.labels.get(slice - 1).cloned().flatten());
        if self.error.is_none() && now.as_deref() != name {
            message.push_str(&format!(
                "; el script lo nombra «{}» con :name() y manda",
                now.unwrap_or_default()
            ));
        }
        if self.error.is_some() {
            message.push_str("; el script tiene errores: sigue sonando lo anterior");
        }
        self.message = message;
    }

    /// Guarda el proyecto abierto (etiquetas y ajustes de cortes) con su
    /// tempo, sin tocar el script. Devuelve la nota para el mensaje.
    fn save_project_file(&self) -> String {
        match self.project.clone() {
            Some(path) => match self.project_with(self.host.default_bpm()).save(&path) {
                Ok(()) => format!(" (guardado en {})", path.display()),
                Err(e) => format!(" (no se pudo guardar el proyecto: {e})"),
            },
            None => " (sin proyecto: se guardará con «guardar»)".into(),
        }
    }

    /// Mueve el inicio del corte seleccionado `delta_ms` (o, con `None`,
    /// le quita el ajuste): se guarda en el proyecto como posición
    /// absoluta, manda sobre el detector, se recarga y se preescucha. No
    /// puede pasar de sus vecinos.
    pub fn adjust_slice(&mut self, delta_ms: Option<f64>) {
        let Some(track) = self.tracks().get(self.selected) else {
            self.message = "no hay pistas".into();
            return;
        };
        let index = self
            .selected_slice
            .min(track.slices.len().saturating_sub(1));
        let Some(range) = track.slices.get(index) else {
            return;
        };
        let current = self.frame_ms(range.start);
        self.place_slice(index, delta_ms.map(|d| current + d));
    }

    /// Ms del sample original (sin estirar) del frame `frame` del sample
    /// de la pista seleccionada: las posiciones del proyecto van así.
    fn frame_ms(&self, frame: usize) -> f64 {
        let Some(track) = self.tracks().get(self.selected) else {
            return 0.0;
        };
        let rate = track.sample.sample_rate() as f64;
        let ratio = track.stretch.map_or(1.0, |s| s.ratio);
        frame as f64 * 1000.0 / rate / ratio
    }

    /// Pone el inicio del corte `index` de la pista seleccionada en `ms`
    /// (del sample original), o le quita el ajuste con `None`: lo de
    /// [`App::adjust_slice`], también para arrastrar la marca con el ratón.
    fn place_slice(&mut self, index: usize, ms: Option<f64>) {
        let Some(track) = self.tracks().get(self.selected) else {
            self.message = "no hay pistas".into();
            return;
        };
        let (origin, name) = (track.origin.clone(), track.name.clone());
        if index >= track.slices.len() {
            return;
        }
        let Some(slice) = origin.indices[index].map(|i| i + 1) else {
            self.message = format!(
                "el corte {} de {name} lo añade el script con :add(): muévelo en el script",
                index + 1
            );
            return;
        };
        let previous = self
            .host
            .positions()
            .iter()
            .find(|p| p.sample == origin.sample && p.slices == origin.cut() && p.slice == slice)
            .map(|p| p.ms);
        // Con la resolución de un frame, redondeado a 0,1 ms.
        let target = ms.map(|ms| (ms.max(0.0) * 10.0).round() / 10.0);
        self.host
            .set_position(&origin.sample, origin.cut(), slice, target);
        self.load();
        let blocked = format!("el ajuste del corte {slice} de {}", origin.sample);
        let warning = self
            .host
            .song()
            .and_then(|s| s.warnings.iter().find(|w| w.starts_with(&blocked)).cloned());
        if let Some(warning) = warning {
            // Se sale de sus vecinos: se deja como estaba.
            self.host
                .set_position(&origin.sample, origin.cut(), slice, previous);
            self.load();
            self.message = format!("no se mueve: {warning}");
            return;
        }
        if self.error.is_some() {
            self.message = "el script tiene errores: arréglalos antes de ajustar cortes".into();
            return;
        }
        let saved = self.save_project_file();
        self.preview_selected();
        self.message = match target {
            Some(ms) => format!("corte {} de {name}: empieza en {ms} ms{saved}", index + 1),
            None => format!(
                "corte {} de {name}: sin ajuste, donde lo pone el detector{saved}",
                index + 1
            ),
        };
    }

    /// `remuestrear [pista|master] [compases] [archivo.wav]`: graba por
    /// render offline la salida de una pista (en solo, con sus efectos y sus
    /// envíos) o del master como un WAV en la carpeta de samples, para
    /// usarlo con `sample("...")`. Sin pista, la seleccionada; sin compases,
    /// lo que tarda en repetirse. No sobrescribe salvo con `!`.
    fn resample(&mut self, args: &[&str], overwrite: bool) {
        let Some(song) = self.host.song() else {
            self.message = "no hay nada cargado que remuestrear".into();
            return;
        };
        let mut args = args.iter().copied().peekable();
        let selected = self.tracks().get(self.selected).map(|t| t.name.clone());
        let target = match args.peek() {
            Some(&"master") => {
                args.next();
                None
            }
            Some(name) if song.track_names().contains(name) => {
                let name = name.to_string();
                args.next();
                Some(name)
            }
            _ => match selected {
                Some(name) => Some(name),
                None => {
                    self.message = "no hay pistas".into();
                    return;
                }
            },
        };
        let bars = match args.peek().map(|a| a.parse::<u64>()) {
            Some(Ok(bars)) if (1..=256).contains(&bars) => {
                args.next();
                bars
            }
            Some(Ok(bars)) => {
                self.message = format!("compases no válidos: {bars} (1 a 256)");
                return;
            }
            _ => match &target {
                Some(name) => song.track_length_bars(name).unwrap_or(1),
                None => song.length_bars(1),
            },
        };
        let label = target.clone().unwrap_or_else(|| "master".into());
        let mut file = args
            .next()
            .map_or_else(|| format!("{label}-remuestreo.wav"), String::from);
        if Path::new(&file).extension().is_none() {
            file.push_str(".wav");
        }
        let path = self.sample_folder().join(&file);
        if path.exists() && !overwrite {
            self.message = format!(
                "{} ya existe: «remuestrear! ...» lo sustituye",
                path.display()
            );
            return;
        }
        let audio = match &target {
            Some(name) => song.render_track(self.sample_rate, bars, name).unwrap(),
            None => song.render(self.sample_rate, bars),
        };
        let result = audio.map_err(|e| e.to_string()).and_then(|audio| {
            engine::offline::write_wav(&path, &audio, 2, self.sample_rate)
                .map_err(|e| e.to_string())
        });
        self.message = match result {
            Ok(()) => format!(
                "remuestreo de {label}: {} en {} · úsalo con sample(\"{file}\")",
                script::bars_text(bars as u64),
                path.display()
            ),
            Err(e) => format!("no se pudo remuestrear: {e}"),
        };
    }

    /// «Explorador de samples»: los WAV de la carpeta de muestras.
    pub fn open_browser(&mut self) {
        self.completion = None;
        let files = crate::browser::list(&self.sample_folder());
        self.message = if files.is_empty() {
            format!("no hay WAV en {}", self.sample_folder().display())
        } else {
            "↑↓ elegir y preescuchar · Tab otra vez · Enter inserta sample(\"...\") · Esc cierra"
                .into()
        };
        self.browser = Some(Browser {
            files,
            ..Browser::default()
        });
    }

    /// Los archivos del explorador que pasan el filtro.
    fn browser_matches(&self) -> Vec<String> {
        self.browser.as_ref().map_or_else(Vec::new, |b| {
            crate::browser::filter(&b.files, &b.query)
                .into_iter()
                .cloned()
                .collect()
        })
    }

    /// Preescucha el archivo elegido en el explorador (en la voz aparte:
    /// no toca las pistas).
    fn browser_preview(&mut self) {
        let Some(file) = self
            .browser_matches()
            .get(self.browser.as_ref().map_or(0, |b| b.selected))
            .cloned()
        else {
            return;
        };
        let path = self.sample_folder().join(&file);
        let rate = self.sample_rate;
        let Some(browser) = &mut self.browser else {
            return;
        };
        let sample = match browser.loaded.get(&file) {
            Some(sample) => sample.clone(),
            None => match Sample::load_wav(&path, rate) {
                Ok(sample) => {
                    let sample = Arc::new(sample);
                    browser.loaded.insert(file.clone(), sample.clone());
                    sample
                }
                Err(e) => {
                    browser.info = format!("{file}: {e}");
                    return;
                }
            },
        };
        browser.info = format!(
            "{file} · {:.2} s",
            sample.frames() as f64 / sample.sample_rate() as f64
        );
        let whole = engine::Slices::grid(&sample, 1);
        if let Err(e) = self.controller.preview_slice(&sample, &whole, 0) {
            self.message = format!("motor: {e}");
        }
    }

    /// Tecla con el explorador abierto.
    fn handle_browser(&mut self, input: Input) -> Flow {
        let matches = self.browser_matches();
        let count = matches.len();
        let Some(browser) = &mut self.browser else {
            return Flow::Continue;
        };
        match input.key {
            Key::Esc => {
                self.browser = None;
                let _ = self.controller.stop_preview();
                self.message = String::new();
            }
            Key::Enter => {
                let file = matches.get(browser.selected).cloned();
                self.browser = None;
                let _ = self.controller.stop_preview();
                if let Some(file) = file {
                    let text = format!("sample(\"{file}\")");
                    self.marks.clear();
                    self.grouped(|editor| editor.insert_str(&text));
                    self.message = format!("{text} en el script");
                }
            }
            Key::Up | Key::Down if count > 0 => {
                browser.selected = if input.key == Key::Up {
                    (browser.selected + count - 1) % count
                } else {
                    (browser.selected + 1) % count
                };
                self.browser_preview();
            }
            Key::Tab => self.browser_preview(),
            Key::Backspace => {
                browser.query.pop();
                browser.selected = 0;
            }
            Key::Char(c) if !input.ctrl && !input.alt => {
                browser.query.push(c);
                browser.selected = 0;
            }
            _ => {}
        }
        Flow::Continue
    }

    /// `grabar [N]`: graba N compases (1 por defecto) de lo que se toca en
    /// la pista seleccionada, desde el compás siguiente.
    fn start_recording(&mut self, bars: Option<&str>) {
        let Some(track) = self.tracks().get(self.selected) else {
            self.message = "no hay pistas".into();
            return;
        };
        let name = track.name.clone();
        let (Some(status), Some(_)) = (self.status, self.controller.position()) else {
            self.message = "arranca el secuenciador (marcha) para grabar".into();
            return;
        };
        let bar_steps = status.bar_steps.max(1);
        let most = engine::MAX_STEPS / bar_steps;
        let bars = match bars.map(str::parse::<usize>) {
            None => 1,
            Some(Ok(n)) if (1..=most).contains(&n) => n,
            _ => {
                self.message = format!("uso: grabar N (de 1 a {most} compases)");
                return;
            }
        };
        // Desde el principio del compás siguiente.
        let bar_start = status.global - status.step.unwrap_or(0) as u64;
        let start = if status.step.is_some() {
            bar_start + bar_steps as u64
        } else {
            0
        };
        self.recording = Some(crate::record::Recording::new(&name, start, bars, bar_steps));
        self.message = format!(
            "● grabando {name} en el compás siguiente, {bars} {}: toca (pads o MIDI); «Grabar lo tocado» otra vez para antes",
            if bars == 1 { "compás" } else { "compases" }
        );
    }

    /// Mira cómo va la grabación: si ha acabado, la escribe; si se paró el
    /// secuenciador, la deja.
    fn watch_recording(&mut self) {
        let Some(rec) = &self.recording else { return };
        match self.controller.position() {
            None => {
                self.recording = None;
                self.message = "grabación cancelada: se paró el secuenciador".into();
            }
            Some(position) if rec.done(position) => self.finish_recording(),
            Some(_) => {}
        }
    }

    /// Escribe lo grabado en el cursor como `steps("...")` (con
    /// `length(N):` delante si no son 16 pasos), en un paso de deshacer.
    fn finish_recording(&mut self) {
        let Some(rec) = self.recording.take() else {
            return;
        };
        let labels = self
            .tracks()
            .iter()
            .find(|t| t.name == rec.track)
            .map(|t| t.labels.clone())
            .unwrap_or_default();
        let steps = format!("steps(\"{}\")", rec.notation(&labels));
        let text = if rec.steps == engine::STEPS {
            steps
        } else {
            format!("length({}):{steps}", rec.steps)
        };
        self.completion = None;
        self.marks.clear();
        self.grouped(|editor| editor.insert_str(&text));
        self.message = format!(
            "grabado {} en el cursor ({} pasos); Ctrl-S para cargarlo",
            rec.track, rec.steps
        );
    }

    /// «Tap tempo»: empieza a contar golpes (este es el primero). Espacio
    /// golpea, Enter pone `bpm(N)` en el script y Esc cancela.
    pub fn start_tap(&mut self, now: Instant) {
        self.completion = None;
        let mut tap = crate::tap::TapTempo::default();
        tap.tap(now);
        self.tapping = Some(tap);
        self.message = "tap tempo: espacio al pulso · Enter pone bpm() · Esc cancela".into();
    }

    /// Un golpe de tap tempo en `now`.
    pub fn tap_at(&mut self, now: Instant) {
        let Some(tap) = &mut self.tapping else { return };
        self.message = match tap.tap(now) {
            Some(bpm) => format!(
                "tap tempo: {bpm} BPM ({} golpes) · Enter pone bpm({bpm}) · Esc cancela",
                tap.count()
            ),
            None => "tap tempo: sigue golpeando con espacio · Esc cancela".into(),
        };
    }

    /// Tecla durante el tap tempo.
    fn handle_tap(&mut self, input: Input) -> Flow {
        match input.key {
            Key::Char(' ') => self.tap_at(Instant::now()),
            Key::Enter => {
                let bpm = self.tapping.take().and_then(|t| t.bpm());
                match bpm {
                    Some(bpm) => self.set_bpm_line(Some(&bpm.to_string())),
                    None => self.message = "tap tempo: hacen falta al menos dos golpes".into(),
                }
            }
            _ => {
                self.tapping = None;
                self.message = "tap tempo cancelado".into();
            }
        }
        Flow::Continue
    }

    /// Abre la línea de órdenes para remuestrear la pista seleccionada.
    pub fn start_resample(&mut self) {
        let name = self
            .tracks()
            .get(self.selected)
            .map_or_else(|| "master".to_string(), |t| t.name.clone());
        self.prompt = Some(format!("remuestrear {name} "));
        self.message =
            "remuestrear [pista|master] [compases] [archivo.wav]: Enter graba en la carpeta de samples"
                .into();
    }

    /// «Partir el corte seleccionado»: un corte nuevo en su mitad (se guarda
    /// en el proyecto como `partir`; las etiquetas y los ajustes de los
    /// siguientes se corren uno).
    pub fn split_selected(&mut self) {
        self.edit_slices(true);
    }

    /// «Unir el corte seleccionado con el anterior»: quita su marca (se
    /// guarda en el proyecto como `unir`).
    pub fn join_selected(&mut self) {
        self.edit_slices(false);
    }

    fn edit_slices(&mut self, split: bool) {
        let Some(track) = self.tracks().get(self.selected) else {
            self.message = "no hay pistas".into();
            return;
        };
        let index = self
            .selected_slice
            .min(track.slices.len().saturating_sub(1));
        let (origin, name) = (track.origin.clone(), track.name.clone());
        let Some(range) = track.slices.get(index) else {
            return;
        };
        let Some(original) = origin.indices.get(index).copied().flatten() else {
            self.message = format!(
                "el corte {} de {name} lo añade el script con :add(): cámbialo en el script",
                index + 1
            );
            return;
        };
        if self.error.is_some() {
            self.message =
                "el script tiene errores: arréglalos antes de partir o unir cortes".into();
            return;
        }
        // Para deshacerlo si al cargar no cabe.
        let before = (
            self.host.labels().to_vec(),
            self.host.positions().to_vec(),
            self.host.edits().to_vec(),
        );
        let ratio = track.stretch.map_or(1.0, |s| s.ratio);
        let rate = track.sample.sample_rate() as f64;
        let slices_before = track.slices.len();
        if split {
            let middle = (range.start + range.end) as f64 / 2.0;
            // Al microsegundo: el corte cae en el frame de la mitad.
            let ms = (middle * 1000.0 / rate / ratio * 1000.0).round() / 1000.0;
            self.host
                .split_slice(&origin.sample, origin.cut(), original + 2, ms);
        } else {
            if original == 0 {
                self.message = format!("el corte 1 de {name} no tiene anterior");
                return;
            }
            self.host
                .join_slice(&origin.sample, origin.cut(), original + 1);
        }
        self.load();
        let after = self
            .tracks()
            .iter()
            .find(|t| t.name == name)
            .map_or(0, |t| t.slices.len());
        let warning = self.host.song().and_then(|s| {
            s.warnings
                .iter()
                .find(|w| w.starts_with("partir ") || w.starts_with("unir "))
                .cloned()
        });
        if self.error.is_some() || after == slices_before || warning.is_some() {
            let (labels, positions, edits) = before;
            self.host.set_labels(labels);
            self.host.set_positions(positions);
            self.host.set_edits(edits);
            self.load();
            self.message = format!(
                "no se puede: {}",
                warning.unwrap_or_else(|| "el corte es demasiado corto".into())
            );
            return;
        }
        let saved = self.save_project_file();
        if split {
            self.selected_slice = index + 1;
            self.message = format!(
                "corte {} de {name} partido: ahora hay {after} (el nuevo es el {}){saved}",
                index + 1,
                index + 2
            );
        } else {
            self.selected_slice = index - 1;
            self.message = format!(
                "corte {} de {name} unido al {}: ahora hay {after}{saved}",
                index + 1,
                index
            );
        }
    }

    /// `abrir proyecto.sampler` (o `abrir!` para descartar cambios).
    fn open_project_command(&mut self, target: Option<&str>, force: bool) {
        let Some(name) = target else {
            self.message = "uso: abrir proyecto.sampler".into();
            return;
        };
        if self.dirty() && !force {
            self.message = "hay cambios sin guardar: guárdalos o usa «abrir! ...»".into();
            return;
        }
        let path = self.base_dir().join(name);
        let result = Project::load(&path)
            .map_err(|e| e.to_string())
            .and_then(|project| {
                self.apply_project(&path, &project)
                    .map_err(|e| format!("{}: {e}", project.script.display()))
            });
        if let Err(e) = result {
            self.message = format!("no se pudo abrir: {e}");
        } else if self.error.is_none() {
            self.message = format!(
                "proyecto {} abierto: entra en el siguiente compás",
                path.display()
            );
        }
    }

    /// «Guardar la selección como bloque»: abre la línea de órdenes con
    /// `bloque ` para escribir el nombre, las etiquetas y la descripción.
    pub fn start_save_block(&mut self) {
        if editing::selected_text(&self.editor).is_none() {
            self.message = "selecciona antes el texto del bloque".into();
            return;
        }
        self.completion = None;
        self.prompt = Some("bloque ".into());
        self.message = "nombre [etiquetas] [-- descripción] · «bloque-proyecto ...» lo guarda \
                        en el proyecto"
            .into();
    }

    /// `bloque nombre [etiquetas...] [-- descripción]`: guarda la selección
    /// como bloque en la carpeta global (`bloque-proyecto`: en la del
    /// proyecto). Con `!` sustituye el que se llame igual.
    fn save_block(&mut self, command: &str, rest: &str) {
        let usage = "uso: bloque nombre [etiquetas] [-- descripción] (con texto seleccionado)";
        let overwrite = command.ends_with('!');
        let scope = if command.starts_with("bloque-proyecto") {
            Scope::Project
        } else {
            Scope::Global
        };
        let (head, description) = rest.split_once("--").unwrap_or((rest, ""));
        let mut words = head.split_whitespace();
        let Some(name) = words.next() else {
            self.message = usage.into();
            return;
        };
        let tags = blocks::parse_tags(&words.collect::<Vec<_>>().join(" "));
        let Some(body) = editing::selected_text(&self.editor) else {
            self.message = format!("selecciona antes el texto del bloque · {usage}");
            return;
        };
        let body = editing::dedent(&body);
        self.message =
            match self
                .blocks
                .save(scope, name, description.trim(), &tags, &body, overwrite)
            {
                Ok(block) => format!(
                    "bloque «{name}» guardado en {} ({})",
                    block.path.display(),
                    scope.label()
                ),
                Err(SaveError::Exists(path)) => format!(
                    "ya hay un bloque «{name}» en {}: «{command}! ...» lo sustituye",
                    path.display()
                ),
                Err(e) => format!("no se pudo guardar el bloque: {e}"),
            };
    }

    /// `muestras carpeta`: carpeta de los samples (y recarga). Sin argumento
    /// dice cuál es.
    fn set_samples(&mut self, target: Option<&str>) {
        match target {
            None => {
                self.message = match self.host.sample_dir() {
                    Some(dir) => format!("samples en {}", dir.display()),
                    None => "samples junto al script".into(),
                };
            }
            Some(dir) => {
                let dir = self.base_dir().join(dir);
                self.host.set_sample_dir(Some(dir));
                self.load();
            }
        }
    }

    pub fn file_name(&self) -> String {
        self.path.file_name().map_or_else(
            || self.path.display().to_string(),
            |n| n.to_string_lossy().into(),
        )
    }

    fn text(&self) -> String {
        let mut text = self.editor.lines().join("\n");
        text.push('\n');
        text
    }

    /// Estado del motor (los picos se vacían al leerlo).
    #[cfg(test)]
    pub fn controller_status_for_test(&self) -> engine::Status {
        self.controller.status()
    }

    #[cfg(test)]
    pub fn path_for_test(&self) -> &Path {
        &self.path
    }

    #[cfg(test)]
    pub fn text_for_test(&self) -> String {
        self.text()
    }

    pub fn dirty(&self) -> bool {
        self.text() != self.saved
    }

    /// Guarda el editor en disco y lo carga. Si el archivo cambió fuera,
    /// pide confirmación antes de sobrescribirlo.
    pub fn save(&mut self) {
        if self.external.is_some() && !std::mem::replace(&mut self.overwrite_armed, true) {
            self.message = "el archivo cambió fuera del editor: Ctrl-S otra vez para \
                            sobrescribirlo, o F3 para recargarlo"
                .into();
            return;
        }
        self.overwrite_armed = false;
        self.external = None;
        let text = self.text();
        match std::fs::write(&self.path, &text) {
            Ok(()) => {
                self.saved = text;
                self.load();
            }
            Err(e) => {
                self.message = format!("no se pudo guardar: {e}");
            }
        }
    }

    /// Mira si el archivo cambió fuera del editor (otro editor, `git
    /// checkout`...). No toca nada: avisa y ofrece recargar con F3.
    pub fn check_external(&mut self) {
        // Con el editor fuera, el archivo es suyo.
        if self.is_main_window() {
            return;
        }
        let Ok(disk) = std::fs::read_to_string(&self.path) else {
            return;
        };
        if disk == self.saved {
            self.external = None;
        } else if self.external.as_ref() != Some(&disk) {
            self.message =
                "el archivo cambió fuera del editor: F3 recargar · Ctrl-S sobrescribir".into();
            self.external = Some(disk);
            self.overwrite_armed = false;
        }
    }

    /// Ruta donde se guarda el texto sin guardar al recargar del disco.
    pub fn unsaved_backup_path(&self) -> PathBuf {
        let stem = self
            .path
            .file_stem()
            .map_or_else(|| "script".into(), |s| s.to_string_lossy());
        self.path.with_file_name(format!("{stem}.sin-guardar.lua"))
    }

    /// Recarga el archivo cambiado fuera. Si había texto sin guardar, se
    /// escribe antes en `<nombre>.sin-guardar.lua` para no perderlo.
    pub fn reload_external(&mut self) {
        let Some(disk) = self.external.take() else {
            self.message = "el archivo no ha cambiado fuera del editor".into();
            return;
        };
        let mut note = String::new();
        if self.dirty() {
            let backup = self.unsaved_backup_path();
            if let Err(e) = std::fs::write(&backup, self.text()) {
                self.external = Some(disk);
                self.message = format!("no se pudo guardar tu texto en {}: {e}", backup.display());
                return;
            }
            note = format!(" (tu texto sin guardar está en {})", backup.display());
        }
        self.editor = TextArea::new(disk.lines().map(String::from).collect());
        self.saved = disk;
        self.overwrite_armed = false;
        self.load();
        if self.error.is_none() {
            self.message = format!("recargado del disco{note}");
        } else {
            self.message = format!("recargado del disco, con errores{note}");
        }
    }

    /// Carga el contenido del editor (sin guardar). Entra en el siguiente
    /// compás; si falla, sigue sonando lo anterior y el error va al editor.
    pub fn load(&mut self) {
        // En el editor separado, carga la ventana principal.
        if self.link_load() {
            return;
        }
        let name = self.file_name();
        let base = self.path.parent().unwrap_or(Path::new(".")).to_path_buf();
        let text = self.text();
        match self.host.load(&mut self.controller, &text, &name, &base) {
            Ok(song) => {
                self.loads += 1;
                self.error = None;
                self.loaded_text = Some(text.clone());
                self.message = "cargado: entra en el siguiente compás".into();
                for warning in &song.warnings {
                    self.message.push_str(&format!(" · ⚠ {warning}"));
                }
                if !self.started && self.controller.start().is_ok() {
                    self.started = true;
                }
                let count = self.tracks().len();
                if self.selected >= count {
                    self.selected = 0;
                }
                self.update_waveforms();
                self.sync_mix();
                if let Some(on) = self.host.song().and_then(|s| s.metronome) {
                    self.set_metronome(on);
                }
                // Una versión en el historial (si el disco falla, se avisa).
                if let Some(history) = self.history()
                    && let Err(e) = history.record(&text, SystemTime::now())
                {
                    self.message.push_str(&format!(
                        " · ⚠ historial: {e} ({})",
                        history.dir().display()
                    ));
                }
            }
            Err(e) => {
                self.message = "error: sigue sonando lo anterior".into();
                self.error = Some(e);
            }
        }
    }

    /// Calcula los resúmenes de los samples nuevos (y las notas de sus
    /// cortes) y olvida los que ya no se usan. Se hace al cargar, no al
    /// dibujar.
    fn update_waveforms(&mut self) {
        self.update_slice_notes();
        let Some(song) = self.host.song() else { return };
        let mut next = HashMap::new();
        let all = song
            .tracks
            .iter()
            .chain(song.sections.iter().flat_map(|s| s.tracks.iter()));
        for track in all {
            let key = Arc::as_ptr(&track.sample) as usize;
            let entry = self
                .waveforms
                .remove(&key)
                .unwrap_or_else(|| (track.sample.clone(), Arc::new(Waveform::of(&track.sample))));
            next.insert(key, entry);
        }
        self.waveforms = next;
    }

    /// Detecta la nota de los cortes nuevos (por sample y cortes: mover un
    /// corte la vuelve a calcular) y olvida los que ya no se usan.
    fn update_slice_notes(&mut self) {
        let Some(song) = self.host.song() else { return };
        let mut next = HashMap::new();
        let all = song
            .tracks
            .iter()
            .chain(song.sections.iter().flat_map(|s| s.tracks.iter()));
        for track in all {
            let key = slice_notes_key(track);
            if next.contains_key(&key) {
                continue;
            }
            let entry = self.slice_notes.remove(&key).unwrap_or_else(|| {
                let notes = if track.instrument.is_some_and(|i| i.synth.is_some()) {
                    Vec::new()
                } else {
                    engine::detect_notes(&track.sample, &track.slices)
                };
                (track.sample.clone(), notes)
            });
            next.insert(key, entry);
        }
        self.slice_notes = next;
    }

    /// Arranca (en el cue, si hay uno) o para el secuenciador.
    pub fn toggle_transport(&mut self) {
        let running = self.status.is_some_and(|s| s.running) || self.controller.is_playing();
        let result = match self.cue {
            _ if running => self.controller.stop(),
            Some(bar) => self.controller.start_at(bar),
            None => self.controller.start(),
        };
        self.message = match result {
            Ok(()) if running => "parado".into(),
            Ok(()) => match self.cue {
                Some(bar) => format!("en marcha desde el cue: {}", self.cue_text(bar)),
                None => "en marcha".into(),
            },
            Err(e) => format!("motor: {e}"),
        };
    }

    /// Lleva el cursor a la línea del error, si la hay.
    pub fn goto_error(&mut self) {
        let name = self.file_name();
        if let Some(line) = self
            .error
            .as_ref()
            .and_then(|e| error_line(&e.message, &name))
        {
            let row = (line.saturating_sub(1)).min(u16::MAX as usize) as u16;
            self.editor.move_cursor(CursorMove::Jump(row, 0));
        }
    }

    /// `linea N`: lleva el cursor al principio de la línea N (desde 1). Más
    /// allá del final, a la última.
    fn goto_line(&mut self, number: &str) {
        let Ok(line) = number.parse::<usize>() else {
            self.message = format!("linea N: «{number}» no es un número de línea");
            return;
        };
        let lines = self.editor.lines().len();
        let row = line.clamp(1, lines) - 1;
        self.editor.cancel_selection();
        self.editor
            .move_cursor(CursorMove::Jump(row.min(u16::MAX as usize) as u16, 0));
        self.message = if line > lines {
            format!("el script tiene {lines} líneas: a la última")
        } else {
            String::new()
        };
    }

    /// `buscar TEXTO`: resalta todas las veces que sale y va a la primera
    /// desde el cursor. Sin texto, quita el resaltado.
    fn find(&mut self, text: &str) {
        self.editor.cancel_selection();
        if text.is_empty() {
            let _ = self.editor.set_search_pattern("");
            self.message = "búsqueda quitada".into();
            return;
        }
        if self
            .editor
            .set_search_pattern(editing::search_pattern(text))
            .is_err()
        {
            self.message = format!("no se puede buscar «{text}»");
            return;
        }
        self.search = Some(text.to_string());
        if self.editor.search_forward(true) {
            self.search_message(text);
        } else {
            let _ = self.editor.set_search_pattern("");
            self.message = format!("«{text}» no sale en el script");
        }
    }

    /// La siguiente o la anterior de la última búsqueda.
    fn find_again(&mut self, forward: bool) {
        let Some(text) = self.search.clone() else {
            self.message = "no hay nada buscado: Ctrl-F busca".into();
            return;
        };
        if self.editor.search_pattern().is_none() {
            let _ = self
                .editor
                .set_search_pattern(editing::search_pattern(&text));
        }
        self.editor.cancel_selection();
        let found = if forward {
            self.editor.search_forward(false)
        } else {
            self.editor.search_back(false)
        };
        if found {
            self.search_message(&text);
        } else {
            self.message = format!("«{text}» ya no sale en el script");
        }
    }

    fn search_message(&mut self, text: &str) {
        let (total, current) = editing::search_count(&self.editor);
        self.message = format!("«{text}»: {current} de {total} (F6 siguiente, F7 anterior)");
    }

    /// La cola por la que llegan los mensajes del puerto MIDI de entrada.
    #[cfg_attr(not(feature = "midi"), allow(dead_code))]
    pub fn set_midi_input(&mut self, input: rtrb::Consumer<crate::midi::TimedEvent>) {
        self.midi_in = Some(input);
    }

    /// Adónde mandar las notas MIDI de las pistas.
    #[cfg_attr(not(feature = "midi"), allow(dead_code))]
    pub fn set_midi_output(&mut self, sink: Box<dyn crate::midi::MidiSink>) {
        self.midi_out = Some(sink);
    }

    /// Manda las notas MIDI que han salido del motor (si no hay destino,
    /// se tiran: la cola no se queda llena).
    fn flush_midi(&mut self) {
        let notes: Vec<engine::MidiNote> = self.controller.midi_events().collect();
        if let Some(sink) = &mut self.midi_out {
            for note in notes {
                sink.send(&note.bytes());
            }
        }
    }

    /// Atiende los mensajes MIDI que han llegado.
    pub fn poll_midi(&mut self) {
        while let Some(event) = self.midi_in.as_mut().and_then(|q| q.pop().ok()) {
            self.midi_event(event);
        }
        self.flush_expression();
    }

    /// Manda al motor la expresión que ha cambiado (lo último de cada eje
    /// de cada nota) de las notas en vivo de un sintetizador.
    fn flush_expression(&mut self) {
        crate::expression::coalesce(&mut self.expr_changes);
        for change in self.expr_changes.drain(..) {
            if let Some(held) = self.held.iter().find(|h| h.key == change.key && !h.grain) {
                let _ = self
                    .controller
                    .note_mod(held.slot, change.key, change.axis, change.value);
            }
        }
    }

    /// Un mensaje MIDI: una nota dispara un corte de la pista seleccionada
    /// (la misma ruta que los pads del modo tocar, con su velocidad como
    /// volumen); con `reloj = true`, el reloj pone el tempo (entra al
    /// compás) y Start/Stop la marcha.
    pub fn midi_event(&mut self, event: crate::midi::TimedEvent) {
        use crate::midi::MidiEvent;
        match event.event {
            MidiEvent::NoteOn {
                channel,
                note,
                velocity,
            } => {
                let gain = velocity as f32 / 127.0;
                // Con MPE las notas llegan por todos los canales.
                let filtered = !self.midi.mpe && self.midi.channel.is_some_and(|c| c != channel);
                if filtered {
                } else if let Some(root) = self.selected_instrument().map(|i| i.root) {
                    // En un instrumento, la nota es la que suena (con lo que
                    // su canal trae de expresión).
                    let now = Instant::now();
                    let expr = self.expression.note_on(channel, note);
                    self.live_note_on_expr(note, (note as i32 - root) as f32, gain, now, expr);
                } else if self.selected_granular() {
                    // En una pista con granos, la nota abre una nube.
                    let offset = note as i32 - self.midi.base_note as i32;
                    self.live_grain_on(note, offset, gain, Instant::now());
                } else if let Some(index) = self.midi.slice_for(channel, note) {
                    self.play_pad_with_gain(index, gain);
                }
            }
            MidiEvent::NoteOff { channel, note } => {
                if self.midi.mpe || !self.midi.channel.is_some_and(|c| c != channel) {
                    self.expression.note_off(note);
                    self.live_note_off(note);
                }
            }
            MidiEvent::Controller {
                channel,
                number,
                value,
            } if self.midi_macro(channel, number, value) => {}
            MidiEvent::PitchBend { .. }
            | MidiEvent::ChannelPressure { .. }
            | MidiEvent::PolyPressure { .. }
            | MidiEvent::Controller { .. } => {
                let changes = self.expression.event(&event.event);
                self.expr_changes.extend(changes);
            }
            _ if !self.midi.clock => {}
            MidiEvent::Clock => {
                if let Some(bpm) = self.clock.pulse(event.micros)
                    && self.controller.set_bpm(bpm).is_ok()
                {
                    self.message = format!("reloj MIDI: {bpm} BPM (entra en el siguiente compás)");
                }
            }
            MidiEvent::Start | MidiEvent::Continue => {
                self.clock.reset();
                let _ = self.controller.start();
                self.message = "reloj MIDI: en marcha".into();
            }
            MidiEvent::Stop => {
                let _ = self.controller.stop();
                self.message = "reloj MIDI: parado".into();
            }
        }
    }

    /// Dispara ya el corte `index` (desde 0) de la pista seleccionada. En
    /// un instrumento, toca la nota `index` semitonos sobre la raíz, que
    /// suena hasta que se suelta la tecla.
    pub fn play_pad(&mut self, index: usize) {
        self.press_pad_at(index, Instant::now());
    }

    /// El pad `index` pulsado en `now` (el instante se pasa para probar los
    /// tiempos sin esperar).
    pub fn press_pad_at(&mut self, index: usize, now: Instant) {
        let granular = self.selected_granular();
        if self.selected_instrument().is_none() && !granular {
            self.play_pad_with_gain(index, 1.0);
            return;
        }
        let key = PAD_KEY + index as u8;
        // Una tecla que ya suena: es su repetición (mantenerla pulsada).
        if let Some(held) = self.held.iter_mut().find(|h| h.key == key) {
            held.last = now;
            held.repeated = true;
            return;
        }
        if granular {
            self.live_grain_on(key, index as i32, 1.0, now);
        } else {
            self.live_note_on(key, index as f32, 1.0, now);
        }
    }

    /// Se soltó una tecla (solo llega si la terminal lo dice): el pad suelta
    /// su nota.
    pub fn key_release(&mut self, input: Input) {
        if let Key::Char(c) = input.key
            && self.mode == Mode::Play
            && let Some(index) = PADS.iter().position(|&p| p == c.to_ascii_lowercase())
        {
            self.live_note_off(PAD_KEY + index as u8);
        }
    }

    /// La terminal avisa al soltar las teclas (protocolo de teclado
    /// mejorado): los pads de un instrumento sueltan su nota con ese aviso,
    /// no por tiempo.
    /// El ratón en el editor (`[editor] raton`): la terminal lo pide.
    pub fn mouse_enabled(&self) -> bool {
        self.mouse_enabled
    }

    pub fn set_key_releases(&mut self, on: bool) {
        self.key_releases = on;
    }

    /// Sin avisos de soltar: suelta las notas de los pads cuyas
    /// repeticiones de teclado han dejado de llegar en `now`.
    pub fn release_expired_pads(&mut self, now: Instant) {
        if self.key_releases {
            return;
        }
        let expired: Vec<u8> = self
            .held
            .iter()
            .filter(|h| h.key >= PAD_KEY)
            .filter(|h| {
                let hold = if h.repeated {
                    PAD_REPEAT_HOLD
                } else {
                    PAD_FIRST_HOLD
                };
                now.saturating_duration_since(h.last) > hold
            })
            .map(|h| h.key)
            .collect();
        for key in expired {
            self.live_note_off(key);
        }
    }

    /// Suelta las notas de los pads (al salir del modo tocar o cambiar de
    /// pista); las del MIDI siguen hasta su note off.
    fn release_pads(&mut self) {
        let pads: Vec<u8> = self
            .held
            .iter()
            .filter(|h| h.key >= PAD_KEY)
            .map(|h| h.key)
            .collect();
        for key in pads {
            self.live_note_off(key);
        }
    }

    /// Las pistas que suenan: las de la sección en curso (sin secciones o
    /// sin línea de tiempo, las de la canción). Vacío sin nada cargado.
    /// Los tokens de la mini-notación que se oyen ahora (ver
    /// [`crate::highlight`]): nada si el texto del editor no es el de la
    /// última carga buena (marcaría mal) o con el secuenciador parado.
    fn highlights(&self) -> Vec<crate::highlight::Highlight> {
        if !self.highlighting {
            return Vec::new();
        }
        // En el editor separado, lo que manda la principal.
        if let Some(remote) = self.remote_highlights() {
            return remote;
        }
        let (Some(song), Some(status)) = (self.host.song(), self.status) else {
            return Vec::new();
        };
        if !status.running || self.loaded_text.as_deref() != Some(self.text().as_str()) {
            return Vec::new();
        }
        let selected = self.tracks().get(self.selected).map(|t| t.name.as_str());
        // Lo que se suma a mano al retardo de la salida, en pasos.
        let extra = self.highlight_delay_ms * status.bpm * 4.0 / 60_000.0;
        crate::highlight::highlights(
            song,
            &self.cursors,
            status.heard.map(|h| h - extra),
            |name| self.host.track_slot(name),
            selected,
        )
    }

    pub fn tracks(&self) -> &[script::TrackDef] {
        let Some(song) = self.host.song() else {
            return &[];
        };
        let section = self
            .status
            .and_then(|s| s.section)
            .and_then(|place| song.section_at(place.entry));
        song.section_tracks(section)
    }

    /// Los nombres de las secciones de la línea de tiempo, para un mensaje;
    /// sin `arrange()`, `None` (y lo dice).
    fn section_names(&mut self) -> Option<String> {
        let names = self
            .host
            .song()
            .filter(|s| !s.timeline.is_empty())
            .map(|song| {
                let mut names: Vec<&str> = Vec::new();
                for &(s, _) in &song.timeline {
                    let name = song.sections[s].name.as_str();
                    if !names.contains(&name) {
                        names.push(name);
                    }
                }
                names.join(", ")
            });
        if names.is_none() {
            self.message =
                "no hay línea de tiempo: defínela con arrange({ {\"sección\", compases}, ... })"
                    .into();
        }
        names
    }

    /// Salta a la entrada siguiente (o anterior) de la línea de tiempo: entra
    /// en el compás siguiente.
    pub fn step_section(&mut self, forward: bool) {
        if self.section_names().is_none() {
            return;
        }
        let len = self.host.song().map_or(1, |s| s.timeline.len());
        let current = self.status.and_then(|s| s.section).map_or(0, |p| p.entry);
        let entry = if forward {
            (current + 1) % len
        } else {
            (current + len - 1) % len
        };
        self.jump_to_entry(entry);
    }

    /// `seccion NOMBRE`: salta a la primera entrada de esa sección.
    pub fn jump_to_section(&mut self, name: &str) {
        let Some(names) = self.section_names() else {
            return;
        };
        let entry = self.host.song().and_then(|song| {
            song.timeline
                .iter()
                .position(|&(s, _)| song.sections[s].name == name)
        });
        match entry {
            Some(entry) => self.jump_to_entry(entry),
            None if name.is_empty() => self.message = format!("seccion NOMBRE (hay: {names})"),
            None => {
                self.message =
                    format!("la sección «{name}» no está en la línea de tiempo (hay: {names})")
            }
        }
    }

    fn jump_to_entry(&mut self, entry: usize) {
        let name = self
            .host
            .song()
            .and_then(|s| s.section_at(entry))
            .unwrap_or_default()
            .to_string();
        self.message = match self.controller.jump_to(entry) {
            Ok(()) => format!("salta a «{name}» en el compás siguiente"),
            Err(e) => format!("no se pudo saltar: {e}"),
        };
    }

    /// Pasa a la otra variante (A/B): entra al empezar el compás siguiente.
    pub fn toggle_variant(&mut self) {
        match self.host.toggle_variant(&mut self.controller) {
            Ok(song) => {
                self.loads += 1;
                let letter = song.variant.unwrap_or('A');
                self.message = format!("variante {letter}: entra en el compás siguiente");
                let count = self.tracks().len();
                if self.selected >= count {
                    self.selected = 0;
                }
                self.update_waveforms();
                self.sync_mix();
            }
            Err(e) => self.message = e.to_string(),
        }
    }

    /// Repite la sección que suena, o vuelve a la línea de tiempo.
    pub fn toggle_section_loop(&mut self) {
        let looping = self
            .status
            .and_then(|s| s.section)
            .is_some_and(|p| p.looping);
        self.set_section_loop(!looping);
    }

    fn set_section_loop(&mut self, on: bool) {
        if self.section_names().is_none() {
            return;
        }
        let name = self
            .status
            .and_then(|s| s.section)
            .and_then(|p| self.host.song().and_then(|s| s.section_at(p.entry)))
            .unwrap_or_default()
            .to_string();
        self.message = match self.controller.loop_section(on) {
            Ok(()) if on => format!("repite «{name}» hasta que vuelvas a la línea de tiempo (l)"),
            Ok(()) => "sigue la línea de tiempo".into(),
            Err(e) => format!("no se pudo: {e}"),
        };
        // El estado lo dice ya, sin esperar al motor.
        if let Some(place) = self.status.as_mut().and_then(|s| s.section.as_mut()) {
            place.looping = on;
        }
    }

    /// El instrumento de la pista seleccionada, si es un instrumento.
    fn selected_instrument(&self) -> Option<script::InstrumentDef> {
        self.tracks().get(self.selected).and_then(|t| t.instrument)
    }

    /// La pista seleccionada toca en granos (`:grains()`, `:freeze()`).
    fn selected_granular(&self) -> bool {
        self.tracks()
            .get(self.selected)
            .is_some_and(|t| t.granular.is_some())
    }

    /// Una nube de granos en vivo en la pista seleccionada, que sigue hasta
    /// soltar la tecla `key` (la tecla elige el corte, o el pitch si la
    /// pista tiene un solo corte: ver `n`).
    ///
    /// `n` es el número de pad o la distancia de la nota MIDI a la nota
    /// base. Con varios cortes, la tecla `n` abre la nube del corte `n`
    /// (desde 0), como el número de un paso; con uno solo (lo normal en una
    /// textura), las teclas son semitonos sobre el pitch de la nube, como
    /// las de un instrumento. La posición, el tamaño, la densidad y los
    /// demás parámetros son los de la pista (lo que diga el script).
    fn live_grain_on(&mut self, key: u8, n: i32, gain: f32, now: Instant) {
        let Some(track) = self.tracks().get(self.selected) else {
            return;
        };
        let Some(slot) = self.host.track_slot(&track.name) else {
            return;
        };
        let (sample, name, slices) = (track.sample.clone(), track.name.clone(), track.slices.len());
        let single = slices == 1;
        let (range, semitones, index, text) = if single {
            let range = track.slices.get(0).unwrap_or(0..sample.frames());
            (range, n as f32, 0, format!("{n:+} st"))
        } else {
            let Some(range) = usize::try_from(n).ok().and_then(|i| {
                let shape = track.shapes.get(i).copied().unwrap_or_default();
                track.slices.get(i).map(|r| shape.range(r))
            }) else {
                self.message = format!("{name} no tiene corte {} (tiene {slices})", n + 1);
                return;
            };
            let i = n as usize;
            let label = track
                .labels
                .get(i)
                .cloned()
                .flatten()
                .map_or(String::new(), |l| format!(" ({l})"));
            (range, 0.0, i, format!("corte {}{label}", i + 1))
        };
        match self
            .controller
            .grain_on(slot, &sample, range, semitones, gain, key)
        {
            Ok(()) => {
                // La misma tecla otra vez (MIDI sin note off): la sustituye.
                self.held.retain(|h| h.key != key);
                self.selected_slice = index;
                let position = self.controller.position();
                let recorded = match (&mut self.recording, position) {
                    (Some(rec), Some(position)) if rec.track == name && single => {
                        rec.note(position, semitones, gain)
                    }
                    (Some(rec), Some(position)) if rec.track == name => {
                        rec.hit_held(position, index, gain)
                    }
                    _ => None,
                };
                self.held.push(HeldNote {
                    key,
                    slot,
                    track: name.clone(),
                    last: now,
                    repeated: false,
                    recorded,
                    grain: true,
                });
                self.message = format!("⁘ {name}: {text}");
            }
            Err(e) => self.message = format!("{name}: {text}: {e}"),
        }
    }

    /// Una nota en vivo (`semitones` sobre la raíz) en el instrumento de la
    /// pista seleccionada, con la tecla `key` del motor.
    fn live_note_on(&mut self, key: u8, semitones: f32, gain: f32, now: Instant) {
        self.live_note_on_expr(key, semitones, gain, now, [0.0; engine::synth::AXES]);
    }

    /// Como [`App::live_note_on`], con la expresión con la que empieza la
    /// nota (la de su canal MIDI; en un instrumento de sample no cuenta).
    fn live_note_on_expr(
        &mut self,
        key: u8,
        semitones: f32,
        gain: f32,
        now: Instant,
        expr: [f32; engine::synth::AXES],
    ) {
        let Some(track) = self.tracks().get(self.selected) else {
            return;
        };
        let (Some(instrument), Some(slot)) = (track.instrument, self.host.track_slot(&track.name))
        else {
            return;
        };
        let (sample, range, name) = (
            track.sample.clone(),
            track.slices.get(0).unwrap_or(0..track.sample.frames()),
            track.name.clone(),
        );
        let note = script::patterns::note_name(instrument.root + semitones as i32);
        // Un sintetizador toca su patch; un instrumento, su sample.
        let sent = match instrument.synth {
            Some(synth) => {
                self.controller
                    .synth_note_on_expr(slot, synth.patch, semitones, gain, key, expr)
            }
            None => self.controller.note_on(
                slot,
                &sample,
                range,
                instrument.engine,
                semitones,
                gain,
                key,
            ),
        };
        match sent {
            Ok(()) => {
                // La misma tecla otra vez (MIDI sin note off): la sustituye.
                self.held.retain(|h| h.key != key);
                let position = self.controller.position();
                let recorded = match (&mut self.recording, position) {
                    (Some(rec), Some(position)) if rec.track == name => {
                        rec.note(position, semitones, gain)
                    }
                    _ => None,
                };
                self.held.push(HeldNote {
                    key,
                    slot,
                    track: name.clone(),
                    last: now,
                    repeated: false,
                    recorded,
                    grain: false,
                });
                self.message = format!("♪ {name}: {note}");
            }
            Err(e) => self.message = format!("{name}: {note}: {e}"),
        }
    }

    /// Suelta la nota en vivo de la tecla `key`: empieza su release.
    fn live_note_off(&mut self, key: u8) {
        let Some(at) = self.held.iter().position(|h| h.key == key) else {
            return;
        };
        let held = self.held.remove(at);
        let _ = if held.grain {
            self.controller.grain_off(held.slot, key)
        } else {
            self.controller.note_off(held.slot, key)
        };
        if let (Some(rec), Some(step), Some(position)) = (
            &mut self.recording,
            held.recorded,
            self.controller.position(),
        ) && rec.track == held.track
        {
            rec.release(step, position);
        }
    }

    /// Como [`App::play_pad`], con un volumen para el golpe.
    fn play_pad_with_gain(&mut self, index: usize, gain: f32) {
        let Some(track) = self.tracks().get(self.selected) else {
            self.message = "no hay pistas".into();
            return;
        };
        let Some(slot) = self.host.track_slot(&track.name) else {
            return;
        };
        let (sample, slices, name) = (
            track.sample.clone(),
            track.slices.clone(),
            track.name.clone(),
        );
        let shape = track.shapes.get(index).copied().unwrap_or_default();
        let label = track
            .labels
            .get(index)
            .cloned()
            .flatten()
            .map_or(String::new(), |l| format!(" ({l})"));
        self.message = match self
            .controller
            .play_slice_with_gain(slot, &sample, &slices, index, shape, gain)
        {
            Ok(()) => {
                self.selected_slice = index;
                // Grabando esta pista: se apunta en el paso más cercano.
                let position = self.controller.position();
                if let (Some(rec), Some(position)) = (&mut self.recording, position)
                    && rec.track == name
                {
                    rec.hit(position, index, gain);
                }
                format!("▶ {name}: corte {}{label}", index + 1)
            }
            Err(engine::SendError::NoSuchSlice { count, .. }) => {
                format!("{name} no tiene corte {} (tiene {count})", index + 1)
            }
            Err(e) => format!("motor: {e}"),
        };
    }

    /// Carpeta donde `sample()` busca los WAV: la del proyecto o la del
    /// script.
    fn sample_folder(&self) -> PathBuf {
        self.host.sample_dir().map_or_else(
            || self.path.parent().unwrap_or(Path::new(".")).to_path_buf(),
            Path::to_path_buf,
        )
    }

    /// Etiquetas de corte conocidas: las de lo último cargado, las del
    /// proyecto y las que el texto pone con `:name()` (aún sin cargar).
    fn known_labels(&self) -> Vec<String> {
        let mut labels: Vec<String> = Vec::new();
        let mut add = |name: &str| {
            if !labels.iter().any(|l| l == name) {
                labels.push(name.to_string());
            }
        };
        if let Some(song) = self.host.song() {
            let all = song
                .tracks
                .iter()
                .chain(song.sections.iter().flat_map(|s| s.tracks.iter()));
            for name in all.flat_map(|t| t.labels.iter().flatten()) {
                add(name);
            }
        }
        for label in self.host.labels() {
            add(&label.name);
        }
        for name in self.remote_labels() {
            add(name);
        }
        for name in complete::text_labels(&self.editor.lines().join("\n")) {
            add(&name);
        }
        labels
    }

    /// Autocompletado en el cursor. Si no es `manual` (F4), solo se abre
    /// con algo escrito o justo tras `:` o unas comillas, y no para una
    /// única opción que ya está escrita entera y no añade nada (una palabra
    /// clave, sí; una función con su plantilla, no).
    fn completion_at(&self, manual: bool) -> Option<Completion> {
        let cursor = self.editor.cursor();
        let (row, col) = (cursor.0, cursor.1);
        let folder = self.sample_folder();
        let labels = self.known_labels();
        let context = complete::context(
            self.editor.lines(),
            row,
            col,
            || list_wavs(&folder),
            &labels,
            &self.blocks.blocks,
        )?;
        let matches = context.matches();
        if matches.is_empty() {
            return None;
        }
        if !manual {
            let before: Vec<char> = self.editor.lines()[row].chars().take(col).collect();
            let trigger = before
                .len()
                .checked_sub(context.prefix.chars().count() + 1)
                .map(|i| before[i]);
            if context.prefix.is_empty() && !matches!(trigger, Some(':' | '"' | '\'')) {
                return None;
            }
            // (Un efecto escrito entero sigue abierto: aceptarlo regenera
            // sus parámetros.)
            if matches.len() == 1
                && matches[0].label == context.prefix
                && matches[0].insert == context.prefix
                && matches[0].kind != "efecto"
                && matches[0].kind != "bus"
            {
                return None;
            }
        }
        Some(Completion {
            prefix: context.prefix,
            matches,
            selected: 0,
            manual,
            navigated: false,
        })
    }

    /// F4: abre el autocompletado aunque no haya nada escrito.
    pub fn open_completion(&mut self) {
        self.completion = self.completion_at(true);
        if self.completion.is_none() {
            self.message = "nada que completar aquí".into();
        }
    }

    /// Tras una tecla que llega al editor: recalcula el autocompletado
    /// abierto, o lo abre si se escribió algo que lo pide.
    fn refresh_completion(&mut self, typed: Option<char>) {
        match self.completion.take() {
            Some(old) => {
                let mut next = self.completion_at(old.manual);
                if let (Some(next), Some(selected)) = (&mut next, old.matches.get(old.selected)) {
                    next.selected = next
                        .matches
                        .iter()
                        .position(|c| c.label == selected.label)
                        .unwrap_or(0);
                }
                self.completion = next;
            }
            None => {
                if typed.is_some_and(|c| c.is_alphanumeric() || "_:\"'".contains(c)) {
                    self.completion = self.completion_at(false);
                }
            }
        }
    }

    fn move_completion(&mut self, down: bool) {
        if let Some(c) = &mut self.completion {
            c.navigated = true;
            let n = c.matches.len();
            c.selected = if down {
                (c.selected + 1) % n
            } else {
                (c.selected + n - 1) % n
            };
        }
    }

    /// Sustituye lo escrito por la opción elegida. Las plantillas de las
    /// funciones dejan marcado el primer parámetro (lo que se escriba lo
    /// sustituye); Tab salta al siguiente y, tras el último, al final de
    /// la plantilla.
    pub fn accept_completion(&mut self) {
        let Some(completion) = self.completion.take() else {
            return;
        };
        let Some(candidate) = completion.matches.get(completion.selected) else {
            return;
        };
        self.message = candidate.signature.clone();
        let replace = completion.prefix.chars().count();
        let insert = candidate.insert.clone();
        // Un efecto dentro de `fx("...", { ... })`: la tabla se regenera
        // con los parámetros del efecto nuevo.
        let effect = match candidate.kind.as_str() {
            "efecto" => api::effect(&candidate.label),
            // `bus("...", { ... })`: la tabla de parámetros del bus.
            "bus" => api::bus(&candidate.label),
            _ => None,
        };
        if let Some(effect) = effect {
            let fields = self.grouped(|editor| {
                editing::insert_template(editor, replace, &insert);
                editing::replace_fx_table(editor, effect)
            });
            if let Some(fields) = fields {
                self.marks = fields;
                self.mark_selected = self.editor.is_selecting();
            }
            return;
        }
        self.insert_template(replace, &insert);
    }

    /// Inserta una plantilla en lugar de los `replace` caracteres antes
    /// del cursor (se deshace de una vez). Si tiene campos, el primero
    /// queda marcado; si no (un nombre suelto, como un efecto escrito sobre
    /// un campo), se conservan los campos pendientes.
    fn insert_template(&mut self, replace: usize, template: &str) {
        if let Some(fields) =
            self.grouped(|editor| editing::insert_template(editor, replace, template))
        {
            self.marks = fields;
            self.mark_selected = self.editor.is_selecting();
        }
    }

    /// Inserta un snippet en el cursor.
    pub fn insert_snippet(&mut self, name: &str) {
        let Some(snippet) = api::snippet(name) else {
            return;
        };
        self.completion = None;
        self.insert_template(0, snippet.template);
        self.message = format!("snippet {name}: Tab salta al siguiente campo");
    }

    /// Inserta un bloque de la biblioteca en el cursor, como un snippet.
    pub fn insert_block(&mut self, name: &str) {
        let Some(block) = self.blocks.get(name).cloned() else {
            self.message = format!("no hay ningún bloque «{name}»");
            return;
        };
        self.completion = None;
        self.insert_template(0, &block.body);
        self.message = if api::expand_template(&block.body).1.is_empty() {
            format!("bloque {name} insertado")
        } else {
            format!("bloque {name}: Tab salta al siguiente campo")
        };
    }

    /// Abre el selector de bloques (relee la biblioteca antes).
    pub fn open_block_picker(&mut self, mode: BlockMode) {
        let body = match mode {
            BlockMode::Update => match editing::selected_text(&self.editor) {
                Some(text) => editing::dedent(&text),
                None => {
                    self.message = "selecciona antes el texto nuevo del bloque".into();
                    return;
                }
            },
            _ => String::new(),
        };
        self.blocks.reload();
        self.completion = None;
        if self.blocks.blocks.is_empty() {
            self.message = "no hay bloques: selecciona texto y «Guardar la selección como \
                            bloque» en la paleta"
                .into();
            return;
        }
        self.message = match self.blocks.warnings.as_slice() {
            [] => String::new(),
            [one] => format!("⚠ {one}"),
            [first, rest @ ..] => format!("⚠ {first} (+{} más)", rest.len()),
        };
        self.block_picker = Some(BlockPicker {
            mode,
            query: String::new(),
            selected: 0,
            body,
            armed: None,
        });
    }

    /// Bloques que coinciden con lo escrito en el selector.
    fn picker_matches(&self) -> Vec<&blocks::Block> {
        let query = self.block_picker.as_ref().map_or("", |p| p.query.as_str());
        self.blocks.filter(query)
    }

    /// Tecla con el selector de bloques abierto: escribir filtra, flechas
    /// eligen, Enter hace la acción y Esc cierra.
    fn handle_block_picker(&mut self, input: Input) -> Flow {
        let matches: Vec<String> = self
            .picker_matches()
            .iter()
            .map(|b| b.name.clone())
            .collect();
        let count = matches.len();
        let Some(picker) = &mut self.block_picker else {
            return Flow::Continue;
        };
        match input.key {
            Key::Esc => self.block_picker = None,
            Key::Enter => {
                let Some(name) = matches.get(picker.selected).cloned() else {
                    return Flow::Continue;
                };
                // Actualizar y borrar piden confirmación: Enter otra vez.
                if picker.mode != BlockMode::Insert && picker.armed.as_ref() != Some(&name) {
                    picker.armed = Some(name);
                    return Flow::Continue;
                }
                let picker = self.block_picker.take().unwrap();
                match picker.mode {
                    BlockMode::Insert => self.insert_block(&name),
                    BlockMode::Update => self.update_block(&name, &picker.body),
                    BlockMode::Delete => self.delete_block(&name),
                }
            }
            Key::Up if count > 0 => {
                picker.selected = (picker.selected + count - 1) % count;
                picker.armed = None;
            }
            Key::Down if count > 0 => {
                picker.selected = (picker.selected + 1) % count;
                picker.armed = None;
            }
            Key::Backspace => {
                picker.query.pop();
                picker.selected = 0;
                picker.armed = None;
            }
            Key::Char(c) if !input.ctrl && !input.alt => {
                picker.query.push(c);
                picker.selected = 0;
                picker.armed = None;
            }
            _ => {}
        }
        Flow::Continue
    }

    /// Sustituye el texto de un bloque (conserva su cabecera y su archivo).
    fn update_block(&mut self, name: &str, body: &str) {
        self.message = match self.blocks.update(name, body) {
            Ok(block) => format!("bloque «{name}» actualizado ({})", block.path.display()),
            Err(e) => format!("no se pudo actualizar el bloque: {e}"),
        };
    }

    /// Borra el archivo de un bloque.
    fn delete_block(&mut self, name: &str) {
        self.message = match self.blocks.delete(name) {
            Ok(block) => {
                let mut message = format!("bloque «{name}» borrado ({})", block.path.display());
                if let Some(other) = self.blocks.get(name) {
                    message.push_str(&format!(
                        "; queda el {} del mismo nombre",
                        other.scope.label()
                    ));
                }
                message
            }
            Err(e) => format!("no se pudo borrar el bloque: {e}"),
        };
    }

    /// Carpeta de los historiales de versiones (cada script, la suya
    /// dentro). `None`: sin historial.
    pub fn set_history_root(&mut self, root: Option<PathBuf>) {
        self.history_root = root;
    }

    /// El historial del script abierto.
    fn history(&self) -> Option<crate::history::History> {
        let root = self.history_root.as_ref()?;
        Some(crate::history::History::for_script(root, &self.path))
    }

    /// Abre el selector de versiones: las del historial, de la más nueva a
    /// la más antigua, con su texto.
    pub fn open_versions(&mut self) {
        let Some(history) = self.history() else {
            self.message = "sin historial (no hay carpeta de estado)".into();
            return;
        };
        let listed = history.list().and_then(|versions| {
            versions
                .into_iter()
                .map(|v| history.read(&v).map(|text| (v, text)))
                .collect::<std::io::Result<Vec<_>>>()
        });
        match listed {
            Ok(versions) if versions.is_empty() => {
                self.message =
                    "el historial está vacío: cada carga buena guarda una versión".into();
            }
            Ok(versions) => {
                self.completion = None;
                self.message = String::new();
                self.versions = Some(VersionPicker {
                    query: String::new(),
                    selected: 0,
                    versions,
                });
            }
            Err(e) => {
                self.message = format!("⚠ historial: {e} ({})", history.dir().display());
            }
        }
    }

    /// Las versiones que coinciden con lo escrito (en su fecha o su texto).
    fn version_matches(&self) -> Vec<usize> {
        let Some(picker) = &self.versions else {
            return Vec::new();
        };
        let query = picker.query.to_lowercase();
        (0..picker.versions.len())
            .filter(|&i| {
                let (version, text) = &picker.versions[i];
                query.is_empty()
                    || crate::history::date(version.time).contains(&query)
                    || text.to_lowercase().contains(&query)
            })
            .collect()
    }

    /// Tecla con el selector de versiones abierto: escribir filtra, flechas
    /// eligen, Enter restaura y Esc cierra.
    fn handle_versions(&mut self, input: Input) -> Flow {
        let matches = self.version_matches();
        let count = matches.len();
        let Some(picker) = &mut self.versions else {
            return Flow::Continue;
        };
        match input.key {
            Key::Esc => self.versions = None,
            Key::Enter => {
                if let Some(&index) = matches.get(picker.selected) {
                    let picker = self.versions.take().unwrap();
                    let (version, text) = &picker.versions[index];
                    self.restore_version(text, version.time);
                }
            }
            Key::Up if count > 0 => picker.selected = (picker.selected + count - 1) % count,
            Key::Down if count > 0 => picker.selected = (picker.selected + 1) % count,
            Key::Backspace => {
                picker.query.pop();
                picker.selected = 0;
            }
            Key::Char(c) if !input.ctrl && !input.alt => {
                picker.query.push(c);
                picker.selected = 0;
            }
            _ => {}
        }
        Flow::Continue
    }

    /// Analiza los choques de frecuencia de lo último cargado (cada pista en
    /// solo, offline, en motores aparte: lo que suena no se toca) y abre el
    /// informe; sin choques, lo dice en la línea de estado.
    pub fn open_masking(&mut self) {
        let Some(song) = self.host.song() else {
            self.message = "no hay nada cargado que analizar".into();
            return;
        };
        if song.track_names().len() < 2 {
            self.message = "choques de frecuencia: hace falta más de una pista".into();
            return;
        }
        let options = engine::masking::MaskingOptions::default();
        let report = match song.analyze_masking(self.sample_rate, &options) {
            Ok(report) => report,
            Err(e) => {
                self.message = format!("no se pudo analizar: {e}");
                return;
            }
        };
        let note = if self.dirty() || self.error.is_some() {
            " (de lo último cargado bien, no del texto actual)"
        } else {
            ""
        };
        let clashes = report.clashes();
        self.last_masking = Some((report.clone(), self.loads));
        if clashes.is_empty() {
            self.message = format!(
                "sin choques de frecuencia entre {} pistas en {}{note}",
                report.tracks.len(),
                script::bars_text(report.bars)
            );
            return;
        }
        self.completion = None;
        self.message = format!(
            "{} choques en {}{note}",
            clashes.len(),
            script::bars_text(report.bars)
        );
        self.masking = Some(MaskingPicker {
            query: String::new(),
            selected: 0,
            report,
            clashes,
            confirm: None,
        });
    }

    /// Los choques del informe que coinciden con lo escrito (en la pareja o
    /// la banda).
    fn masking_matches(&self) -> Vec<usize> {
        let Some(picker) = &self.masking else {
            return Vec::new();
        };
        let query = picker.query.to_lowercase();
        (0..picker.clashes.len())
            .filter(|&i| {
                let (pair, band, _) = picker.report.clash_line(&picker.clashes[i]);
                query.is_empty()
                    || pair.to_lowercase().contains(&query)
                    || band.to_lowercase().contains(&query)
            })
            .collect()
    }

    /// Tecla con el informe de choques abierto: escribir filtra, flechas
    /// eligen, Enter propone el EQ de la pista que cede en el elegido y
    /// Enter otra vez lo inserta en el script; Esc cancela la propuesta o
    /// cierra.
    fn handle_masking(&mut self, input: Input) -> Flow {
        let matches = self.masking_matches();
        let count = matches.len();
        let Some(picker) = &mut self.masking else {
            return Flow::Continue;
        };
        match input.key {
            Key::Esc if picker.confirm.is_some() => picker.confirm = None,
            Key::Esc => self.masking = None,
            Key::Enter => {
                let Some(&index) = matches.get(picker.selected) else {
                    return Flow::Continue;
                };
                let loser = picker.report.loser(&picker.clashes[index]);
                if picker.confirm == Some(loser) {
                    let picker = self.masking.take().unwrap();
                    self.apply_suggestion(&picker.report, loser);
                } else {
                    picker.confirm = Some(loser);
                }
            }
            Key::Up if count > 0 => {
                picker.selected = (picker.selected + count - 1) % count;
                picker.confirm = None;
            }
            Key::Down if count > 0 => {
                picker.selected = (picker.selected + 1) % count;
                picker.confirm = None;
            }
            Key::Backspace => {
                picker.query.pop();
                picker.selected = 0;
                picker.confirm = None;
            }
            Key::Char(c) if !input.ctrl && !input.alt => {
                picker.query.push(c);
                picker.selected = 0;
                picker.confirm = None;
            }
            _ => {}
        }
        Flow::Continue
    }

    /// Efectos que ya tiene la pista `name` en lo último cargado (en la
    /// canción o en la sección que más tenga).
    fn fx_count(&self, name: &str) -> usize {
        let Some(song) = self.host.song() else {
            return 0;
        };
        song.tracks
            .iter()
            .chain(song.sections.iter().flat_map(|s| s.tracks.iter()))
            .filter(|t| t.name == name)
            .map(|t| t.fx.len())
            .max()
            .unwrap_or(0)
    }

    /// Inserta en el editor la línea del EQ sugerido para la pista `track`
    /// del informe, justo después de donde se define la pista (y de las
    /// líneas de su cadena, las que empiezan por `:`), con su sangría; si no
    /// se encuentra, al final. Es un cambio más (Ctrl-Z lo deshace de una
    /// vez); no se guarda ni se carga.
    fn apply_suggestion(&mut self, report: &script::masking::MaskingReport, track: usize) {
        let Some(suggestion) = report.suggestions.iter().find(|s| s.track == track) else {
            return;
        };
        let name = &report.tracks[track];
        let line = report.lua_line(suggestion);
        let lines = self.editor.lines().to_vec();
        let code = |l: &str| l.split("--").next().unwrap_or("").to_string();
        let head = format!("track(\"{name}\"");
        let defined = lines
            .iter()
            .position(|l| code(l).contains(&format!("{head},")))
            .or_else(|| lines.iter().position(|l| code(l).contains(&head)));
        self.completion = None;
        self.marks.clear();
        let row = match defined {
            Some(start) => {
                let mut end = start;
                while lines
                    .get(end + 1)
                    .is_some_and(|l| l.trim_start().starts_with(':'))
                {
                    end += 1;
                }
                let indent: String = lines[start]
                    .chars()
                    .take_while(|c| c.is_whitespace())
                    .collect();
                self.grouped(|editor| {
                    editor.move_cursor(CursorMove::Jump(end as u16, 0));
                    editor.move_cursor(CursorMove::End);
                    editor.insert_newline();
                    editor.insert_str(format!("{indent}{line}"));
                });
                end + 1
            }
            None => {
                let last = lines.len() - 1;
                let blank = lines[last].trim().is_empty();
                self.grouped(|editor| {
                    editor.move_cursor(CursorMove::Bottom);
                    editor.move_cursor(CursorMove::End);
                    if !blank {
                        editor.insert_newline();
                    }
                    editor.insert_str(&line);
                });
                if blank { last } else { last + 1 }
            }
        };
        self.message = format!(
            "EQ de «{name}» insertado en la línea {}, sin guardar (Ctrl-Z lo deshace; Ctrl-S lo guarda y lo carga)",
            row + 1
        );
    }

    /// Pone una versión en el editor como un cambio más (Ctrl-Z lo deshace
    /// de una vez); el archivo no se toca hasta guardar.
    fn restore_version(&mut self, text: &str, time: SystemTime) {
        let text = text.strip_suffix('\n').unwrap_or(text).to_string();
        self.marks.clear();
        self.grouped(|editor| {
            editor.select_all();
            editor.insert_str(&text);
        });
        self.editor.move_cursor(CursorMove::Top);
        self.message = format!(
            "versión del {} en el editor, sin guardar (Ctrl-Z la deshace; Ctrl-S la guarda y la carga)",
            crate::history::date(time)
        );
    }

    /// Carpeta de las ranuras (`ranuras/` en la de configuración): lee las
    /// que haya. Un archivo que no se puede leer se avisa y la ranura se
    /// queda vacía; si la carpeta falta, se crea al guardar la primera.
    pub fn set_slots_dir(&mut self, dir: Option<PathBuf>) {
        self.slots_dir = dir;
        let Some(dir) = &self.slots_dir else { return };
        let mut problems = Vec::new();
        for (i, slot) in self.slots.iter_mut().enumerate() {
            let path = dir.join(format!("{}.txt", i + 1));
            match std::fs::read_to_string(&path) {
                Ok(text) => *slot = Some(text),
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                Err(e) => problems.push(format!("{}: {e}", path.display())),
            }
        }
        if !problems.is_empty() {
            self.message = format!("⚠ ranuras: {}", problems.join(" · "));
        }
    }

    /// Guarda la ranura `slot` (desde 0) en su archivo. Devuelve el aviso
    /// si no se pudo (la ranura sigue llena en esta sesión).
    fn save_slot(&self, slot: usize) -> Option<String> {
        let dir = self.slots_dir.as_ref()?;
        let text = self.slots[slot].as_deref().unwrap_or("");
        std::fs::create_dir_all(dir)
            .and_then(|()| std::fs::write(dir.join(format!("{}.txt", slot + 1)), text))
            .err()
            .map(|e| format!(" (no se pudo guardar en {}: {e})", dir.display()))
    }

    /// «Guardar en una ranura» / «Pegar una ranura»: espera el número.
    pub fn start_slot(&mut self, op: SlotOp) {
        if op == SlotOp::Paste && self.slots.iter().all(Option::is_none) {
            self.message = "las ranuras están vacías: «Guardar en una ranura» primero".into();
            return;
        }
        self.completion = None;
        self.slot_prompt = Some(op);
        self.message = match op {
            SlotOp::Yank => {
                "guardar la selección (o la línea) en la ranura: pulsa 1-9 · Esc cancela"
            }
            SlotOp::Paste => "pegar la ranura: pulsa 1-9 · Esc cancela",
        }
        .into();
    }

    /// Guarda en la ranura `n` (desde 1) la selección o la línea del
    /// cursor, o la pega en el cursor (se deshace de una vez).
    pub fn run_slot(&mut self, op: SlotOp, n: usize) {
        let Some(slot) = n.checked_sub(1).filter(|&i| i < SLOTS) else {
            return;
        };
        match op {
            SlotOp::Yank => {
                let text = editing::selection_or_line(&self.editor);
                let lines = text.lines().count();
                self.slots[slot] = Some(text);
                let note = self.save_slot(slot).unwrap_or_default();
                self.message = format!(
                    "ranura {n}: {lines} {}{note}",
                    if lines == 1 { "línea" } else { "líneas" }
                );
            }
            SlotOp::Paste => {
                let Some(text) = self.slots[slot].clone() else {
                    self.message = format!("la ranura {n} está vacía");
                    return;
                };
                self.marks.clear();
                self.grouped(|editor| editing::paste(editor, &text));
                self.message = format!("ranura {n} pegada");
            }
        }
    }

    /// Marca el siguiente campo pendiente.
    fn next_mark(&mut self) -> bool {
        match self.marks.next(&mut self.editor) {
            Some(selected) => {
                self.mark_selected = selected == editing::Selected::Field;
                true
            }
            None => false,
        }
    }

    /// Entradas de la paleta que coinciden con lo escrito.
    fn palette_matches(&self) -> Vec<palette::Entry> {
        let query = self.palette.as_ref().map_or("", |p| p.query.as_str());
        palette::filter(&palette::entries_with(self.last_palette, &self.keys), query)
    }

    /// F1: abre la paleta de comandos.
    pub fn open_palette(&mut self) {
        self.completion = None;
        self.palette = Some(Palette::default());
    }

    /// Tecla con la paleta abierta: escribir filtra, flechas eligen, Enter
    /// ejecuta y Esc cierra.
    fn handle_palette(&mut self, input: Input) -> Flow {
        let matches = self.palette_matches();
        let count = matches.len();
        let Some(palette) = &mut self.palette else {
            return Flow::Continue;
        };
        match input.key {
            Key::Esc => self.palette = None,
            Key::Enter => {
                let entry = matches.get(palette.selected).cloned();
                self.palette = None;
                if let Some(entry) = entry {
                    self.last_palette = Some(entry.item);
                    return self.run_item(entry.item);
                }
            }
            Key::Up if count > 0 => palette.selected = (palette.selected + count - 1) % count,
            Key::Down if count > 0 => palette.selected = (palette.selected + 1) % count,
            Key::Backspace => {
                palette.query.pop();
                palette.selected = 0;
            }
            Key::Char(c) if !input.ctrl && !input.alt => {
                palette.query.push(c);
                palette.selected = 0;
            }
            _ => {}
        }
        Flow::Continue
    }

    /// Ejecuta una entrada de la paleta.
    pub fn run_item(&mut self, item: Item) -> Flow {
        match item {
            Item::Action(action) => self.run_action(action),
            Item::Snippet(name) => {
                self.insert_snippet(name);
                Flow::Continue
            }
        }
    }

    /// Ejecuta una acción (de la paleta o de un atajo).
    pub fn run_action(&mut self, action: Action) -> Flow {
        if let Some(flow) = self.link_action(action) {
            return flow;
        }
        let edits = matches!(
            action,
            Action::Undo
                | Action::Redo
                | Action::ToggleComment
                | Action::DuplicateLines
                | Action::MoveLinesUp
                | Action::MoveLinesDown
        );
        if edits {
            self.completion = None;
            self.marks.clear();
        }
        match action {
            Action::Undo => {
                if !self.undo() {
                    self.message = "nada que deshacer".into();
                }
            }
            Action::Redo => {
                if !self.redo() {
                    self.message = "nada que rehacer".into();
                }
            }
            Action::ToggleComment => self.grouped(editing::toggle_comment),
            Action::DuplicateLines => self.grouped(editing::duplicate_lines),
            Action::MoveLinesUp => {
                if !self.grouped(|e| editing::move_lines(e, true)) {
                    self.message = "ya está arriba del todo".into();
                }
            }
            Action::MoveLinesDown => {
                if !self.grouped(|e| editing::move_lines(e, false)) {
                    self.message = "ya está abajo del todo".into();
                }
            }
            Action::TextStart => editing::jump_to_edge(&mut self.editor, false, false),
            Action::TextEnd => editing::jump_to_edge(&mut self.editor, true, false),
            Action::GotoLine => {
                self.prompt = Some("linea ".into());
                let lines = self.editor.lines().len();
                self.message = format!("linea N: lleva el cursor a la línea N (1 a {lines})");
            }
            Action::Find => {
                self.prompt = Some("buscar ".into());
                self.message =
                    "buscar TEXTO: va al primero desde el cursor (sin texto, quita el resaltado)"
                        .into();
            }
            Action::FindNext => self.find_again(true),
            Action::FindPrevious => self.find_again(false),
            Action::Complete => self.open_completion(),
            Action::SaveAndLoad => self.save(),
            Action::Load => self.load(),
            Action::Transport => self.toggle_transport(),
            Action::Cue => self.set_cue(),
            Action::ToggleVim => self.toggle_vim(),
            Action::ClearCue => self.clear_cue(),
            Action::GotoError => self.goto_error(),
            Action::NextTrack => self.next_track(),
            Action::PlayMode => self.toggle_mode(),
            Action::ReloadExternal => self.reload_external(),
            Action::Export => self.export(&[]),
            Action::Commands => self.prompt = Some(String::new()),
            Action::LabelSlice => self.start_labeling(),
            Action::SaveBlock => self.start_save_block(),
            Action::InsertBlock => self.open_block_picker(BlockMode::Insert),
            Action::UpdateBlock => self.open_block_picker(BlockMode::Update),
            Action::DeleteBlock => self.open_block_picker(BlockMode::Delete),
            Action::YankSlot => self.start_slot(SlotOp::Yank),
            Action::PasteSlot => self.start_slot(SlotOp::Paste),
            Action::Mute => self.toggle_live(false),
            Action::Solo => self.toggle_live(true),
            Action::ClearMuteSolo => self.clear_live(),
            Action::ResetMeters => self.reset_meters(),
            Action::Metronome => self.toggle_metronome(),
            Action::Highlight => self.toggle_highlight(),
            Action::DetectBpm => self.detect_bpm(),
            Action::SliceNotes => self.slice_notes(),
            Action::Preview => self.preview_selected(),
            Action::SliceEarlier => self.adjust_slice(Some(-5.0)),
            Action::SliceLater => self.adjust_slice(Some(5.0)),
            Action::ClearSliceAdjust => self.adjust_slice(None),
            Action::NextSlice => self.select_slice(true),
            Action::SplitSlice => self.split_selected(),
            Action::Resample => self.start_resample(),
            Action::Browser => self.open_browser(),
            Action::EditTracker => self.open_tracker_edit(),
            Action::TapTempo => self.start_tap(Instant::now()),
            Action::Macros => self.open_macros(),
            Action::Record => {
                if self.recording.is_some() {
                    self.finish_recording();
                } else {
                    self.prompt = Some("grabar 1".into());
                    self.message = "grabar N: los compases que se graban (Enter empieza en el compás siguiente)".into();
                }
            }
            Action::JoinSlice => self.join_selected(),
            Action::NextSection => self.step_section(true),
            Action::PreviousSection => self.step_section(false),
            Action::JumpSection => {
                if let Some(names) = self.section_names() {
                    self.prompt = Some("seccion ".into());
                    self.message = format!("seccion NOMBRE: salta al compás siguiente ({names})");
                }
            }
            Action::LoopSection => self.toggle_section_loop(),
            Action::FollowTimeline => self.set_section_loop(false),
            Action::Versions => self.open_versions(),
            Action::AnalyzeMasking => self.open_masking(),
            Action::ToggleVariant => self.toggle_variant(),
            Action::PreviousSlice => self.select_slice(false),
            Action::EditorWindow => self.launch_editor(),
            Action::EditorBack => self.reclaim_editor(),
            Action::Quit => {
                let armed = std::mem::replace(&mut self.quit_armed, false);
                return self.request_quit(armed);
            }
        }
        Flow::Continue
    }

    /// Hace una acción compuesta de varios cambios del editor y la apunta
    /// para deshacerla de una vez.
    fn grouped<R>(&mut self, f: impl FnOnce(&mut TextArea<'static>) -> R) -> R {
        let before = self.editor.lines().to_vec();
        let result = f(&mut self.editor);
        let after = self.editor.lines().to_vec();
        if after != before {
            self.undo_groups.push(EditGroup { before, after });
            self.redo_groups.clear();
        }
        result
    }

    /// Deshace un cambio. Si el texto es el resultado de una acción
    /// compuesta, la deshace entera.
    fn undo(&mut self) -> bool {
        undo_in(
            &mut self.editor,
            &mut self.undo_groups,
            &mut self.redo_groups,
        )
    }

    /// Rehace un cambio (una acción compuesta, entera).
    fn redo(&mut self) -> bool {
        redo_in(
            &mut self.editor,
            &mut self.undo_groups,
            &mut self.redo_groups,
        )
    }

    /// Salir: si hay cambios sin guardar, la primera vez solo avisa.
    fn request_quit(&mut self, armed: bool) -> Flow {
        if self.is_main_window() {
            return self.quit_main(armed);
        }
        if self.dirty() && !armed {
            self.quit_armed = true;
            self.message = "hay cambios sin guardar: Ctrl-Q otra vez para salir".into();
            Flow::Continue
        } else {
            Flow::Quit
        }
    }

    fn toggle_mode(&mut self) {
        self.completion = None;
        self.marks.clear();
        self.mode = match self.mode {
            Mode::Edit => Mode::Play,
            Mode::Play => Mode::Edit,
        };
        self.release_pads();
        self.message = match self.mode {
            Mode::Play => {
                "modo tocar: 1-0 y q-i disparan cortes (en un instrumento, notas desde la raíz) · ←→ recorren y preescuchan · espacio preescucha · m mute · s solo · [ ] sección · l repetir sección · v variante A/B · Esc vuelve al editor"
            }
            Mode::Edit => "modo editor",
        }
        .into();
    }

    pub fn next_track(&mut self) {
        let count = self.tracks().len();
        if count > 0 {
            self.release_pads();
            self.selected = (self.selected + 1) % count;
            self.selected_slice = 0;
        }
    }

    /// Envía al motor el mute y el solo en vivo de cada pista (solo lo que
    /// ha cambiado: las pistas pueden haber cambiado de sitio al recargar).
    fn sync_mix(&mut self) {
        let mut want = [(false, false); MAX_TRACKS];
        if let Some(song) = self.host.song() {
            for name in song.track_names() {
                if let Some(slot) = self.host.track_slot(name) {
                    let name = name.to_string();
                    want[slot] = (self.muted.contains(&name), self.soloed.contains(&name));
                }
            }
        }
        for (slot, &(mute, solo)) in want.iter().enumerate() {
            if mute != self.sent_mix[slot].0 && self.controller.set_mute(slot, mute).is_ok() {
                self.sent_mix[slot].0 = mute;
            }
            if solo != self.sent_mix[slot].1 && self.controller.set_solo(slot, solo).is_ok() {
                self.sent_mix[slot].1 = solo;
            }
        }
    }

    /// Mute (o, con `solo`, solo) en vivo de la pista seleccionada: entra ya,
    /// sin esperar al compás.
    pub fn toggle_live(&mut self, solo: bool) {
        let Some(name) = self.tracks().get(self.selected).map(|t| t.name.clone()) else {
            self.message = "no hay pistas".into();
            return;
        };
        let list = if solo {
            &mut self.soloed
        } else {
            &mut self.muted
        };
        let on = match list.iter().position(|n| *n == name) {
            Some(i) => {
                list.remove(i);
                false
            }
            None => {
                list.push(name.clone());
                true
            }
        };
        self.sync_mix();
        let what = if solo { "solo" } else { "mute" };
        self.message = format!("{name}: {what} {}", if on { "sí" } else { "no" });
    }

    /// Enciende o apaga el metrónomo (inmediato).
    fn set_metronome(&mut self, on: bool) {
        if on != self.metronome && self.controller.set_metronome(on).is_ok() {
            self.metronome = on;
        }
    }

    /// «Resaltado en vivo»: lo enciende o lo apaga.
    pub fn toggle_highlight(&mut self) {
        self.highlighting = !self.highlighting;
        self.message = if self.highlighting {
            "resaltado en vivo: encendido"
        } else {
            "resaltado en vivo: apagado"
        }
        .into();
    }

    pub fn toggle_metronome(&mut self) {
        let on = !self.metronome;
        self.set_metronome(on);
        self.message = if self.metronome {
            "metrónomo encendido (no entra en lo exportado salvo «exportar ... metronomo»)"
        } else {
            "metrónomo apagado"
        }
        .into();
    }

    /// «Detectar el BPM del sample»: propone el tempo del sample de la
    /// pista seleccionada y deja escrita la orden `bpm N` para ponerlo.
    pub fn detect_bpm(&mut self) {
        let Some(track) = self.tracks().get(self.selected) else {
            self.message = "no hay pistas".into();
            return;
        };
        let name = track.name.clone();
        let Some(guess) = engine::detect_bpm(&track.sample) else {
            self.message = format!("{name}: no se encontró un tempo (¿no es un loop de compases?)");
            return;
        };
        let bars = if guess.bars == 1 {
            "compás"
        } else {
            "compases"
        };
        let mut message = format!(
            "{name}: {} BPM, el sample dura {} {bars} (encaje {:.0} %)",
            guess.rounded(),
            guess.bars,
            guess.score * 100.0
        );
        if let Some((bpm, n)) = guess.alternative {
            message.push_str(&format!(
                "; también {} con {n}",
                engine::bpm::round_bpm(bpm)
            ));
        }
        message.push_str(" · Enter pone bpm() en el script");
        self.message = message;
        self.prompt = Some(format!("bpm {}", guess.rounded()));
    }

    /// «Notas de los cortes»: la nota de cada corte de la pista
    /// seleccionada (`1 C4 · 2 E4 +25¢ · 3 —`).
    pub fn slice_notes(&mut self) {
        let Some(track) = self.view().tracks.into_iter().nth(self.selected) else {
            self.message = "no hay pistas".into();
            return;
        };
        if track.notes.iter().all(Option::is_none) {
            self.message = format!("{}: ningún corte tiene un tono claro", track.name);
            return;
        }
        let list: Vec<String> = track
            .notes
            .iter()
            .enumerate()
            .map(|(i, note)| match note {
                Some(n) if n.cents().abs() >= CENTS_SHOWN => {
                    format!("{} {} {:+}¢", i + 1, n.name(), n.cents())
                }
                Some(n) => format!("{} {}", i + 1, n.name()),
                None => format!("{} —", i + 1),
            })
            .collect();
        self.message = format!("{}: {}", track.name, list.join(" · "));
    }

    /// `bpm N`: cambia la primera línea `bpm(...)` del script por `bpm(N)`,
    /// o la añade al principio. No guarda (se deshace con Ctrl-Z).
    fn set_bpm_line(&mut self, value: Option<&str>) {
        let Some(bpm) = value
            .and_then(|v| v.parse::<f64>().ok())
            .filter(|b| engine::valid_bpm(*b))
        else {
            self.message = "uso: bpm N (de 20 a 400)".into();
            return;
        };
        let line = format!("bpm({bpm})");
        let row = self
            .editor
            .lines()
            .iter()
            .position(|l| l.trim_start().starts_with("bpm("));
        self.completion = None;
        self.grouped(|editor| match row {
            Some(row) => {
                let len = editor.lines()[row].chars().count();
                editor.move_cursor(CursorMove::Jump(row as u16, 0));
                editor.delete_str(len);
                editor.insert_str(&line);
            }
            None => {
                editor.move_cursor(CursorMove::Jump(0, 0));
                editor.insert_str(format!("{line}\n"));
            }
        });
        self.message = format!("{line} en el script: Ctrl-S para guardar y cargar");
    }

    /// Vacía los medidores y los avisos de saturación.
    pub fn reset_meters(&mut self) {
        self.meters = [0.0; MAX_TRACKS];
        self.group_meters = [0.0; engine::MAX_GROUPS];
        self.master_meter = 0.0;
        self.limiting_until = None;
        self.clipping_until = None;
        self.track_clip_until = [None; MAX_TRACKS];
        self.message = "medidores a cero".into();
    }

    /// Quita todos los mute y solo en vivo.
    pub fn clear_live(&mut self) {
        self.muted.clear();
        self.soloed.clear();
        self.sync_mix();
        self.message = "sin mute ni solo en vivo".into();
    }

    /// Selecciona el corte siguiente (o el anterior) de la pista
    /// seleccionada, sin tocarlo.
    pub fn select_slice(&mut self, forward: bool) {
        let Some(track) = self.tracks().get(self.selected).cloned() else {
            self.message = "no hay pistas".into();
            return;
        };
        let count = track.slices.len().max(1);
        let current = self.selected_slice.min(count - 1);
        self.selected_slice = if forward {
            (current + 1) % count
        } else {
            (current + count - 1) % count
        };
        let label = track
            .labels
            .get(self.selected_slice)
            .cloned()
            .flatten()
            .map_or(String::new(), |l| format!(" ({l})"));
        self.message = format!(
            "{}: corte {}{label} seleccionado",
            track.name,
            self.selected_slice + 1
        );
    }

    /// Preescucha el corte seleccionado en la voz aparte (no toca las
    /// pistas ni el script).
    pub fn preview_selected(&mut self) {
        let Some(track) = self.tracks().get(self.selected) else {
            self.message = "no hay pistas".into();
            return;
        };
        let index = self
            .selected_slice
            .min(track.slices.len().saturating_sub(1));
        let (sample, slices, name) = (
            track.sample.clone(),
            track.slices.clone(),
            track.name.clone(),
        );
        let label = track
            .labels
            .get(index)
            .cloned()
            .flatten()
            .map_or(String::new(), |l| format!(" ({l})"));
        self.message = match self.controller.preview_slice(&sample, &slices, index) {
            Ok(()) => format!("preescucha {name}: corte {}{label}", index + 1),
            Err(e) => format!("motor: {e}"),
        };
    }

    /// Recorre los cortes (←/→ en el modo tocar): selecciona el siguiente o
    /// el anterior y lo preescucha.
    pub fn browse_slice(&mut self, forward: bool) {
        self.select_slice(forward);
        self.preview_selected();
    }

    /// Abre la línea de órdenes para etiquetar el corte seleccionado.
    pub fn start_labeling(&mut self) {
        let Some(track) = self.tracks().get(self.selected).cloned() else {
            self.message = "no hay pistas".into();
            return;
        };
        let slice = self
            .selected_slice
            .min(track.slices.len().saturating_sub(1))
            + 1;
        self.prompt = Some(format!("etiquetar {slice} "));
        self.message = format!("escribe la etiqueta del corte {slice} de {}", track.name);
    }

    /// Atiende una tecla: atajos propios; si no, en modo tocar dispara
    /// cortes y en modo editor escribe.
    pub fn handle(&mut self, input: Input) -> Flow {
        if let Some(prompt) = &mut self.prompt {
            match input.key {
                Key::Enter => {
                    let line = self.prompt.take().unwrap_or_default();
                    self.run_command(&line);
                }
                Key::Esc => {
                    self.prompt = None;
                    self.message = "orden cancelada".into();
                }
                Key::Backspace => {
                    prompt.pop();
                }
                Key::Char(c) if !input.ctrl && !input.alt => prompt.push(c),
                _ => {}
            }
            return Flow::Continue;
        }
        if self.palette.is_some() {
            return self.handle_palette(input);
        }
        if self.block_picker.is_some() {
            return self.handle_block_picker(input);
        }
        if self.versions.is_some() {
            return self.handle_versions(input);
        }
        if self.masking.is_some() {
            return self.handle_masking(input);
        }
        if self.browser.is_some() {
            return self.handle_browser(input);
        }
        if self.tapping.is_some() {
            return self.handle_tap(input);
        }
        if self.macro_panel.is_some() {
            return self.handle_macros(input);
        }
        if let Some(op) = self.slot_prompt.take() {
            match input.key {
                Key::Char(c @ '1'..='9') if !input.ctrl && !input.alt => {
                    self.run_slot(op, c as usize - '0' as usize);
                }
                _ => self.message = "ranura cancelada".into(),
            }
            return Flow::Continue;
        }
        // F1 (y la tecla que se le dé) abre siempre la paleta.
        let bound = self.keys.lookup(&input);
        if bound == Some(Bound::Palette) {
            self.open_palette();
            return Flow::Continue;
        }
        // El cursor del tracker se queda las teclas, salvo deshacer,
        // rehacer, cargar, marcha y salir.
        if self.tracker_edit.is_some()
            && !matches!(
                bound,
                Some(Bound::Action(
                    Action::Undo
                        | Action::Redo
                        | Action::Load
                        | Action::SaveAndLoad
                        | Action::Transport
                        | Action::Cue
                        | Action::Quit
                ))
            )
        {
            return self.handle_tracker_edit(input);
        }
        // El modo vim: en Normal, las letras son órdenes.
        if self.mode == Mode::Edit
            && let Some(flow) = self.handle_vim(&input, bound.is_some())
        {
            return flow;
        }
        // Con el autocompletado abierto, las flechas lo recorren, Tab acepta,
        // Enter acepta solo tras moverse con las flechas (si no, cierra y es
        // un salto de línea) y Esc solo lo cierra (no entra en modo tocar).
        if let Some(completion) = &self.completion
            && !input.ctrl
            && !input.alt
        {
            match input.key {
                Key::Up => return self.then(|app| app.move_completion(false)),
                Key::Down => return self.then(|app| app.move_completion(true)),
                Key::Tab => return self.then(App::accept_completion),
                Key::Enter if completion.navigated => return self.then(App::accept_completion),
                Key::Enter => self.completion = None,
                Key::Esc => return self.then(|app| app.completion = None),
                _ => {}
            }
        }
        // Moverse con un parámetro marcado lo deja tal cual (sin extender
        // la selección).
        let marked = std::mem::replace(&mut self.mark_selected, false);
        if marked
            && matches!(
                input.key,
                Key::Left | Key::Right | Key::Up | Key::Down | Key::Home | Key::End
            )
        {
            self.editor.cancel_selection();
        }
        // Tab salta al siguiente parámetro de la plantilla (o a su final).
        if self.mode == Mode::Edit
            && input.key == Key::Tab
            && !input.ctrl
            && !input.alt
            && self.next_mark()
        {
            return Flow::Continue;
        }
        let quit_armed = std::mem::replace(&mut self.quit_armed, false);
        let action = match bound {
            Some(Bound::Action(action)) => Some(action),
            _ => None,
        };
        if action != Some(Action::SaveAndLoad) {
            self.overwrite_armed = false;
        }
        // Los atajos (los del archivo de configuración o los de siempre).
        match action {
            Some(Action::Quit) => return self.request_quit(quit_armed),
            // Deshacer y rehacer pasan por la app (acciones compuestas).
            Some(a @ (Action::Undo | Action::Redo)) => return self.run_action(a),
            Some(Action::Complete) if self.mode == Mode::Edit => {
                self.open_completion();
                return Flow::Continue;
            }
            Some(Action::Complete) => {}
            Some(action) => {
                self.run_action(action);
            }
            None => return self.handle_plain(input),
        }
        // Los atajos cierran el autocompletado.
        self.completion = None;
        Flow::Continue
    }

    /// Una tecla que no es un atajo: en modo tocar dispara cortes y demás;
    /// en el editor, escribe.
    fn handle_plain(&mut self, input: Input) -> Flow {
        match (input.key, input.ctrl) {
            (Key::Char(c), false) if self.mode == Mode::Play && !input.alt => {
                if let Some(index) = PADS.iter().position(|&p| p == c.to_ascii_lowercase()) {
                    self.play_pad(index);
                } else if c == ' ' {
                    self.preview_selected();
                } else if let Some(delta) = match c {
                    ',' => Some(-5.0),
                    '.' => Some(5.0),
                    '<' => Some(-1.0),
                    '>' => Some(1.0),
                    _ => None,
                } {
                    self.adjust_slice(Some(delta));
                } else if c == 'm' {
                    self.toggle_live(false);
                } else if c == 's' {
                    self.toggle_live(true);
                } else if c == ']' {
                    self.step_section(true);
                } else if c == '[' {
                    self.step_section(false);
                } else if c == 'l' {
                    self.toggle_section_loop();
                } else if c == 'v' {
                    self.toggle_variant();
                }
            }
            (Key::Left, _) if self.mode == Mode::Play => self.browse_slice(false),
            (Key::Right, _) if self.mode == Mode::Play => self.browse_slice(true),
            // En modo tocar el texto no se toca.
            _ if self.mode == Mode::Play => {}
            // Ctrl-Inicio y Ctrl-Fin: principio y final del texto (el widget
            // los tomaría por Inicio y Fin de la línea).
            (Key::Home | Key::End, true) if !input.alt => {
                editing::jump_to_edge(&mut self.editor, input.key == Key::End, input.shift);
            }
            // Copiar y cortar: también al portapapeles del sistema.
            (Key::Char('c' | 'x'), true) if !input.alt => {
                let selected = self.editor.selection_range().is_some();
                self.editor.input(input);
                if selected {
                    let text = self.editor.yank_text();
                    self.clipboard.set(&text);
                }
            }
            // Pegar: el del sistema si se puede leer; si no, el interno.
            (Key::Char('y'), true) if !input.alt => match self.clipboard.get() {
                Some(text) if !text.is_empty() => {
                    self.editor.set_yank_text(text.clone());
                    self.paste(&text);
                }
                _ => {
                    self.editor.input(input);
                }
            },
            _ => {
                let typed = match input.key {
                    Key::Char(c) if !input.ctrl && !input.alt => Some(c),
                    _ => None,
                };
                self.editor.input(input);
                self.refresh_completion(typed);
                return Flow::Continue;
            }
        }
        // Los atajos cierran el autocompletado.
        self.completion = None;
        Flow::Continue
    }

    /// Teclas que llegaron juntas. Si son una ráfaga de texto con saltos
    /// de línea en el editor (un pegado en un terminal sin bracketed
    /// paste), se pegan; si no, se atienden una a una.
    pub fn handle_keys(&mut self, inputs: Vec<Input>) -> Flow {
        if let Some(text) = editing::burst_text(&inputs).filter(|_| self.pastes_in_editor()) {
            self.paste(&text);
            return Flow::Continue;
        }
        for input in inputs {
            if self.handle(input) == Flow::Quit {
                return Flow::Quit;
            }
        }
        Flow::Continue
    }

    /// El texto pegado va al editor: modo editar y sin nada encima.
    fn pastes_in_editor(&self) -> bool {
        self.mode == Mode::Edit && self.nothing_on_top()
    }

    /// Sin nada abierto encima de los paneles (paleta, línea de órdenes...).
    fn nothing_on_top(&self) -> bool {
        self.prompt.is_none()
            && self.palette.is_none()
            && self.block_picker.is_none()
            && self.browser.is_none()
            && self.tapping.is_none()
            && self.slot_prompt.is_none()
    }

    /// Pega texto del terminal en el cursor tal cual (líneas y sangría),
    /// sin autocompletado, plantillas ni atajos; se deshace de una vez.
    /// En la línea de órdenes entra solo la primera línea.
    pub fn paste(&mut self, text: &str) {
        let text = editing::normalize_newlines(text);
        if let Some(prompt) = &mut self.prompt {
            prompt.push_str(text.lines().next().unwrap_or_default());
            return;
        }
        if !self.pastes_in_editor() {
            self.message = "pegar: solo en el editor".into();
            return;
        }
        self.completion = None;
        self.marks.clear();
        self.mark_selected = false;
        self.quit_armed = false;
        self.grouped(|editor| editor.insert_str(text));
    }

    fn then(&mut self, f: impl FnOnce(&mut App)) -> Flow {
        f(self);
        Flow::Continue
    }

    /// Lee el estado del motor, actualiza medidores y recoge la basura.
    pub fn tick(&mut self) {
        self.poll_link();
        if self.last_external_check.elapsed() >= EXTERNAL_CHECK {
            self.last_external_check = Instant::now();
            self.check_external();
            // Los bloques pueden cambiar fuera (otro editor, git...).
            self.blocks.reload();
        }
        self.flush_midi();
        self.flush_expression();
        self.release_expired_pads(Instant::now());
        self.watch_recording();
        let status = self.controller.status();
        self.cursors.observe(&status);
        let now = Instant::now();
        let hold = Some(now + WARNING_HOLD);
        for (t, track) in status.tracks.iter().enumerate() {
            self.meters[t] = track.peak.max(self.meters[t] * METER_DECAY).min(1.0);
            if track.peak > 1.0 {
                self.track_clip_until[t] = hold;
            }
        }
        for (meter, peak) in self.group_meters.iter_mut().zip(status.groups) {
            *meter = peak.max(*meter * METER_DECAY).min(1.0);
        }
        self.master_meter = status.master.peak.max(self.master_meter * METER_DECAY);
        if status.master.limited_frames > 0 {
            self.limiting_until = hold;
        }
        if status.master.peak > 1.0 {
            self.clipping_until = hold;
        }
        self.status = Some(status);
        self.controller.collect_garbage();
    }

    /// Modelo de vista para los paneles.
    pub fn view(&self) -> ViewModel {
        let name = self.file_name();
        let status = self.status;
        let transport = status.map_or(TransportView::default(), |s| TransportView {
            running: s.running,
            bpm: s.bpm,
            bar: s.bar,
            step: s.step,
        });
        let tracks: Vec<TrackView> = Some(self.tracks())
            .map(|tracks| {
                tracks
                    .iter()
                    .map(|t| {
                        let slot = self.host.track_slot(&t.name);
                        let track_status = slot.and_then(|s| status.map(|st| st.tracks[s]));
                        let playing_slice = track_status
                            .and_then(|ts| ts.hit_start)
                            .and_then(|start| t.slices.iter().position(|r| r.start == start));
                        // Vuelta y paso de esta pista (con su longitud) en el
                        // último paso que sonó.
                        let global = status.map_or(0, section_global);
                        let (lap, position) = t.lap(global);
                        let waveform = self
                            .waveforms
                            .get(&(Arc::as_ptr(&t.sample) as usize))
                            .map_or_else(Default::default, |(_, w)| w.clone());
                        TrackView {
                            name: t.name.clone(),
                            slot,
                            sample_name: t
                                .path
                                .file_name()
                                .map_or_else(String::new, |n| n.to_string_lossy().into()),
                            sample_frames: t.sample.frames(),
                            sample_rate: t.sample.sample_rate(),
                            waveform,
                            slices: t.slices.iter().collect(),
                            cycle: t.bars.len(),
                            length: t.length,
                            stretch: t.stretch.map(|s| (s.source_bpm, s.ratio)),
                            steps: t.lap_steps(lap).clone(),
                            position: transport
                                .step
                                .filter(|_| transport.running)
                                .map(|_| position),
                            playing_slice,
                            level: slot.map_or(0.0, |s| self.meters[s]),
                            gain: t.gain,
                            pan: t.pan,
                            mix_moving: t.mix_curves.map(|c| c.is_some()),
                            fx: t.fx.clone(),
                            automated: t
                                .automation
                                .iter()
                                .filter_map(|a| {
                                    let name = t.fx.get(a.fx)?.params().get(a.param)?;
                                    Some((a.fx, *name))
                                })
                                .collect(),
                            labels: t.labels.clone(),
                            notes: self
                                .slice_notes
                                .get(&slice_notes_key(t))
                                .map_or_else(Vec::new, |(_, n)| n.clone()),
                            adjusted: t.adjusted.clone(),
                            swing: t.swing,
                            gate: t.gate,
                            choke: t.choke,
                            midi: t.midi,
                            sends: t.sends,
                            clipping: slot.is_some_and(|s| {
                                self.track_clip_until[s].is_some_and(|u| Instant::now() < u)
                            }),
                            mute: t.mute || self.muted.contains(&t.name),
                            solo: t.solo || self.soloed.contains(&t.name),
                            instrument: t.instrument.map(|i| InstrumentView {
                                root: i.root,
                                voices: i.engine.voices,
                                sounding: track_status.map_or(0, |ts| ts.voices),
                            }),
                            synth: t.instrument.and_then(|i| i.synth).map(|s| SynthView {
                                kind: s.kind,
                                params: crate::model::synth_text(&s),
                            }),
                            grains: t.granular.as_ref().map(crate::model::grains_text),
                            formant: t.formant.or(t.instrument.and_then(|i| i.engine.formant)),
                            slice_fx: t
                                .slice_fx
                                .iter()
                                .enumerate()
                                .filter(|(_, c)| !c.fx.is_empty())
                                .map(|(slice, c)| crate::model::SliceFxView {
                                    slice,
                                    fx: c.fx.clone(),
                                    automated: c
                                        .automation
                                        .iter()
                                        .filter_map(|a| {
                                            let name = c.fx.get(a.fx)?.params().get(a.param)?;
                                            Some((a.fx, *name))
                                        })
                                        .collect(),
                                })
                                .collect(),
                            group: self.host.song().and_then(|song| {
                                song.groups
                                    .iter()
                                    .find(|g| g.tracks.contains(&t.name))
                                    .map(|g| g.name.clone())
                            }),
                        }
                    })
                    .collect()
            })
            .unwrap_or_default();
        let selected = (!tracks.is_empty()).then(|| self.selected.min(tracks.len() - 1));
        // La marca que se arrastra con el ratón, donde va (aún sin cargar).
        let mut tracks = tracks;
        if let Some(drag) = &self.slice_drag
            && let Some(track) = selected.and_then(|i| tracks.get_mut(i))
            && track.name == drag.track
            && drag.index < track.slices.len()
        {
            track.slices[drag.index].start = drag.at;
            if drag.index > 0 {
                track.slices[drag.index - 1].end = drag.at;
            }
        }
        let selected_slice = selected
            .and_then(|t| tracks[t].slices.len().checked_sub(1))
            .map(|last| self.selected_slice.min(last));
        let timeline = self
            .host
            .song()
            .filter(|s| !s.timeline.is_empty())
            .map(|song| {
                let place = status.and_then(|s| s.section);
                TimelineView {
                    entries: song
                        .timeline
                        .iter()
                        .map(|&(s, bars)| (song.sections[s].name.clone(), bars))
                        .collect(),
                    // (Hasta que entre el arreglo nuevo, la entrada puede
                    // ser de la versión anterior.)
                    current: place.map_or(0, |p| p.entry).min(song.timeline.len() - 1),
                    bar: place.map_or(0, |p| p.bar),
                    looping: place.is_some_and(|p| p.looping),
                    cue: self.cue_entry(),
                }
            });
        let groups = self.host.song().map_or_else(Vec::new, |song| {
            song.groups
                .iter()
                .enumerate()
                .map(|(g, def)| GroupView {
                    name: def.name.clone(),
                    tracks: def.tracks.clone(),
                    gain: def.gain,
                    pan: def.pan,
                    sends: def.sends,
                    mix_moving: def.mix_curves.map(|c| c.is_some()),
                    fx: def.fx.clone(),
                    automated: def
                        .automation
                        .iter()
                        .filter_map(|a| {
                            let name = def.fx.get(a.fx)?.params().get(a.param)?;
                            Some((a.fx, *name))
                        })
                        .collect(),
                    level: self.group_meters.get(g).copied().unwrap_or(0.0),
                })
                .collect()
        });
        let mut view = ViewModel {
            groups,
            macros: self
                .host
                .macros()
                .iter()
                .map(|m| (m.name.clone(), m.value))
                .collect(),
            macro_panel: self.macro_panel,
            vim: self.vim.as_ref().map(vim::Vim::label),
            timeline,
            cue: self.cue.map(|bar| self.cue_text(bar)),
            variant: self.host.variant(),
            error: self.error.as_ref().map(|e| ErrorView {
                line: error_line(&e.message, &name),
                message: e.message.clone(),
            }),
            diagnostics: diagnose::steps_diagnostics(self.editor.lines(), &self.known_labels()),
            highlights: self.highlights(),
            help: (self.mode == Mode::Edit
                && self.palette.is_none()
                && self.block_picker.is_none()
                && self.browser.is_none()
                && self.slot_prompt.is_none())
            .then(|| {
                let cursor = self.editor.cursor();
                complete::help_at(self.editor.lines(), cursor.0, cursor.1)
            })
            .flatten(),
            file: name,
            dirty: self.dirty(),
            playing_mode: self.mode == Mode::Play,
            palette: self.palette.as_ref().map(|p| {
                let matches = self.palette_matches();
                PaletteView {
                    query: p.query.clone(),
                    description: matches
                        .get(p.selected)
                        .map_or(String::new(), |e| e.description.clone()),
                    items: matches.into_iter().map(|e| (e.name, e.keys)).collect(),
                    selected: p.selected,
                }
            }),
            slots: self.slot_prompt.map(|op| SlotsView {
                title: match op {
                    SlotOp::Yank => "guardar en la ranura",
                    SlotOp::Paste => "pegar la ranura",
                }
                .into(),
                items: self
                    .slots
                    .iter()
                    .map(|s| s.as_ref().map(|t| t.lines().map(String::from).collect()))
                    .collect(),
            }),
            tracker_cursor: self.tracker_edit.map(|c| (c.track, c.row)),
            browser: self.browser.as_ref().map(|b| {
                let items = self.browser_matches();
                BrowserView {
                    query: b.query.clone(),
                    selected: b.selected.min(items.len().saturating_sub(1)),
                    items,
                    info: b.info.clone(),
                    folder: self.sample_folder().display().to_string(),
                }
            }),
            block_picker: self.block_picker.as_ref().map(|p| {
                let matches = self.picker_matches();
                let selected = matches.get(p.selected);
                BlockPickerView {
                    title: match p.mode {
                        BlockMode::Insert => "insertar un bloque",
                        BlockMode::Update => "actualizar un bloque con la selección",
                        BlockMode::Delete => "borrar un bloque",
                    }
                    .into(),
                    confirm: p.armed.as_ref().map_or(String::new(), |name| match p.mode {
                        BlockMode::Update => {
                            format!(
                                "Enter otra vez para sustituir el texto de «{name}» · Esc cancela"
                            )
                        }
                        _ => format!("Enter otra vez para borrar «{name}» · Esc cancela"),
                    }),
                    query: p.query.clone(),
                    items: matches
                        .iter()
                        .map(|b| {
                            let tags: Vec<String> =
                                b.tags.iter().map(|t| format!("#{t}")).collect();
                            (b.name.clone(), b.scope.label().into(), tags.join(" "))
                        })
                        .collect(),
                    selected: p.selected,
                    description: selected.map_or(String::new(), |b| b.description.clone()),
                    preview: selected.map_or_else(Vec::new, |b| {
                        b.preview().lines().map(String::from).collect()
                    }),
                }
            }),
            versions: self.versions.as_ref().map(|p| {
                let matches = self.version_matches();
                let now = SystemTime::now();
                let selected = matches.get(p.selected).map(|&i| &p.versions[i]);
                BlockPickerView {
                    title: "versiones del script (Enter restaura en el editor)".into(),
                    confirm: String::new(),
                    query: p.query.clone(),
                    items: matches
                        .iter()
                        .map(|&i| {
                            let (version, text) = &p.versions[i];
                            let lines = text.lines().count();
                            (
                                crate::history::date(version.time),
                                format!("{lines} líneas"),
                                crate::history::ago(version.time, now),
                            )
                        })
                        .collect(),
                    selected: p.selected,
                    description: selected.map_or(String::new(), |(v, _)| {
                        if crate::history::hash(&self.text()) == v.hash {
                            "igual que el texto del editor".into()
                        } else {
                            String::new()
                        }
                    }),
                    preview: selected.map_or_else(Vec::new, |(_, text)| {
                        text.lines().map(String::from).collect()
                    }),
                }
            }),
            masking: self.masking.as_ref().map(|p| {
                let matches = self.masking_matches();
                let selected = matches.get(p.selected).map(|&i| &p.clashes[i]);
                let suggestion = selected.and_then(|c| {
                    let loser = p.report.loser(c);
                    p.report.suggestions.iter().find(|s| s.track == loser)
                });
                let mut preview = Vec::new();
                if let Some(s) = suggestion {
                    preview.push(format!(
                        "sugerencia para {} (cede en este choque):",
                        p.report.tracks[s.track]
                    ));
                    preview.push(format!("  {}", p.report.lua_line(s)));
                    preview.extend(p.report.cut_lines(s).iter().map(|l| format!("    {l}")));
                    preview.push(String::new());
                }
                preview.extend(p.report.table().lines().map(String::from));
                let confirm = match p.confirm {
                    None => String::new(),
                    Some(t) => {
                        let name = &p.report.tracks[t];
                        let count = self.fx_count(name);
                        if count >= engine::MAX_FX {
                            format!(
                                "«{name}» ya tiene {count} efectos (máximo {}): el EQ no cabrá al cargar · Enter lo inserta igual · Esc cancela",
                                engine::MAX_FX
                            )
                        } else {
                            format!(
                                "Enter otra vez inserta el EQ de «{name}» en el script (sin guardar) · Esc cancela"
                            )
                        }
                    }
                };
                BlockPickerView {
                    title: format!(
                        "choques de frecuencia · {} · {} pistas · Enter propone el EQ",
                        script::bars_text(p.report.bars),
                        p.report.tracks.len()
                    ),
                    confirm,
                    query: p.query.clone(),
                    items: matches
                        .iter()
                        .map(|&i| {
                            let clash = &p.clashes[i];
                            let (pair, band, _) = p.report.clash_line(clash);
                            (
                                format!("{pair}  {band}"),
                                script::masking::score_text(clash.clash.score),
                                String::new(),
                            )
                        })
                        .collect(),
                    selected: p.selected,
                    description: selected.map_or(String::new(), |c| p.report.summary(c)),
                    preview,
                }
            }),
            clash_table: self.last_masking.as_ref().map(|(report, loads)| {
                let (bands, rows) = report.grid();
                let name = |t: usize| report.tracks[t].clone();
                ClashTableView {
                    bars: report.bars,
                    tracks: report.tracks.len(),
                    stale: *loads != self.loads,
                    bands: bands
                        .iter()
                        .map(|&b| engine::masking::BAND_NAMES[b].to_string())
                        .collect(),
                    rows: rows
                        .into_iter()
                        .map(|((a, b), cells)| ClashRow {
                            a: name(a),
                            b: name(b),
                            cells,
                        })
                        .collect(),
                    clashes: report
                        .clashes()
                        .iter()
                        .map(|c| {
                            (
                                name(c.a),
                                name(c.b),
                                engine::masking::BAND_NAMES[c.band].to_string(),
                                c.clash.score,
                            )
                        })
                        .collect(),
                }
            }),
            completion: self.completion.as_ref().map(|c| {
                let selected = &c.matches[c.selected];
                CompletionView {
                    items: c
                        .matches
                        .iter()
                        .map(|m| (m.label.clone(), m.kind.clone()))
                        .collect(),
                    selected: c.selected,
                    signature: selected.signature.clone(),
                    description: selected.description.clone(),
                    prefix_len: c.prefix.chars().count(),
                }
            }),
            external_change: self.external.is_some(),
            key_help: self.keys.help(self.mode == Mode::Play, true),
            prompt: self.prompt.clone(),
            project: self
                .project
                .as_ref()
                .and_then(|p| p.file_name())
                .map(|n| n.to_string_lossy().into()),
            message: self.message.clone(),
            transport,
            metronome: self.metronome,
            recording: self
                .recording
                .as_ref()
                .map(|rec| match self.controller.position() {
                    Some(p) if rec.waiting(p) => {
                        format!("REC {}: al compás", rec.track)
                    }
                    Some(p) => format!(
                        "REC {} {}/{}",
                        rec.track,
                        rec.bar(p),
                        rec.steps / status.map_or(engine::STEPS, |s| s.bar_steps.max(1))
                    ),
                    None => format!("REC {}", rec.track),
                }),
            tracks,
            selected,
            selected_slice,
            master: MasterView {
                level: self.master_meter,
                limiting: self.limiting_until.is_some_and(|u| Instant::now() < u),
                clipping: self.clipping_until.is_some_and(|u| Instant::now() < u),
            },
            link: None,
            editor_outside: false,
        };
        self.link_view(&mut view);
        view
    }
}

/// Deshace un cambio de `editor`; si su texto es el resultado de una
/// acción compuesta (`undo`), la deshace entera y la pasa a `redo`.
fn undo_in(
    editor: &mut TextArea<'static>,
    undo: &mut Vec<EditGroup>,
    redo: &mut Vec<EditGroup>,
) -> bool {
    if let Some(group) = undo.pop() {
        if editor.lines() == group.after {
            while editor.lines() != group.before && editor.undo() {}
            redo.push(group);
            return true;
        }
        undo.push(group);
    }
    editor.undo()
}

/// Rehace un cambio de `editor` (una acción compuesta, entera).
fn redo_in(
    editor: &mut TextArea<'static>,
    undo: &mut Vec<EditGroup>,
    redo: &mut Vec<EditGroup>,
) -> bool {
    if let Some(group) = redo.pop() {
        if editor.lines() == group.before {
            while editor.lines() != group.after && editor.redo() {}
            undo.push(group);
            return true;
        }
        redo.push(group);
    }
    editor.redo()
}

/// Archivos WAV de una carpeta, ordenados.
fn list_wavs(dir: &Path) -> Vec<String> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut wavs: Vec<String> = entries
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|e| e.eq_ignore_ascii_case("wav")))
        .filter_map(|p| p.file_name().map(|n| n.to_string_lossy().into_owned()))
        .collect();
    wavs.sort();
    wavs
}

/// Texto de un script y lo que hay guardado en disco (vacío si no existe:
/// se abre la plantilla, con cambios sin guardar).
fn read_script(path: &Path) -> std::io::Result<(String, String)> {
    match std::fs::read_to_string(path) {
        Ok(text) => Ok((text.clone(), text)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            Ok((TEMPLATE.to_string(), String::new()))
        }
        Err(e) => Err(e),
    }
}

/// Clave de la caché de notas: el sample (su puntero) y sus cortes.
type SliceNotesKey = (usize, Vec<std::ops::Range<usize>>);

fn slice_notes_key(track: &script::TrackDef) -> SliceNotesKey {
    (
        Arc::as_ptr(&track.sample) as usize,
        track.slices.iter().collect(),
    )
}

/// Semicorchea del último paso contada desde el inicio de la sección que
/// suena (sin línea de tiempo, desde el arranque): la que leen sus pistas.
fn section_global(status: engine::Status) -> u64 {
    let Some(place) = status.section else {
        return status.global;
    };
    let back = place.bar as u64 * status.bar_steps as u64 + status.step.unwrap_or(0) as u64;
    status.global.saturating_sub(back)
}
