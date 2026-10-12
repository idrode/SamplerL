//! Estado y lógica de la interfaz, sin terminal (se prueba sin pantalla).

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use engine::{Controller, MAX_TRACKS, Sample, Status};
use ratatui_textarea::{CursorMove, Input, Key, TextArea};
use script::{Project, ScriptError, ScriptHost, api};

use crate::complete::{self, Candidate};
use crate::diagnose;
use crate::editing;
use crate::model::{
    CompletionView, ErrorView, MasterView, PaletteView, TrackView, TransportView, ViewModel,
    Waveform, error_line,
};
use crate::palette::{self, Action, Item};

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

/// Qué hacer tras una tecla.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Flow {
    Continue,
    Quit,
}

pub struct App {
    pub editor: TextArea<'static>,
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
}

impl App {
    /// Abre el script (o una plantilla si no existe) y lo carga.
    pub fn open(path: &Path, controller: Controller, sample_rate: u32) -> std::io::Result<App> {
        let (text, saved) = read_script(path)?;
        let mut app = App {
            editor: TextArea::new(text.lines().map(String::from).collect()),
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
            selected: 0,
            selected_slice: 0,
            metronome: false,
            muted: Vec::new(),
            soloed: Vec::new(),
            sent_mix: [(false, false); MAX_TRACKS],
            status: None,
            meters: [0.0; MAX_TRACKS],
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
            completion: None,
            marks: editing::Fields::default(),
            mark_selected: false,
            palette: None,
            last_palette: None,
            undo_groups: Vec::new(),
            redo_groups: Vec::new(),
        };
        app.load();
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
        self.load();
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

    /// Ejecuta una orden de la línea de órdenes (F2).
    pub fn run_command(&mut self, line: &str) {
        let mut words = line.split_whitespace();
        let Some(command) = words.next() else { return };
        let args: Vec<&str> = words.collect();
        match command {
            "exportar" | "export" => self.export(&args),
            "guardar" | "save" => self.save_project(args.first().copied()),
            "abrir" | "open" => self.open_project_command(args.first().copied(), false),
            "abrir!" | "open!" => self.open_project_command(args.first().copied(), true),
            "muestras" | "samples" => self.set_samples(args.first().copied()),
            "etiquetar" | "label" => self.label_slice(&args),
            "bpm" => self.set_bpm_line(args.first().copied()),
            other => {
                self.message = format!(
                    "orden desconocida «{other}» (exportar, guardar, abrir, muestras, etiquetar, bpm)"
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
                "exportado {}: {bars} compases a {bpm} BPM ({:.1} s){note}",
                path.display(),
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
        let Some(track) = self.host.song().and_then(|s| s.tracks.get(self.selected)) else {
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
        if let Err(e) = self
            .host
            .set_label(&origin.sample, origin.count, index + 1, name)
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
            .host
            .song()
            .and_then(|s| s.tracks.iter().find(|t| t.name == track_name))
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
        let Some(track) = self.host.song().and_then(|s| s.tracks.get(self.selected)) else {
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
        let rate = track.sample.sample_rate() as f64;
        let current = range.start as f64 * 1000.0 / rate;
        let slice = origin.indices[index] + 1;
        let previous = self
            .host
            .positions()
            .iter()
            .find(|p| p.sample == origin.sample && p.slices == origin.count && p.slice == slice)
            .map(|p| p.ms);
        // Con la resolución de un frame, redondeado a 0,1 ms.
        let target = delta_ms.map(|d| ((current + d).max(0.0) * 10.0).round() / 10.0);
        self.host
            .set_position(&origin.sample, origin.count, slice, target);
        self.load();
        let blocked = format!("el ajuste del corte {slice} de {}", origin.sample);
        let warning = self
            .host
            .song()
            .and_then(|s| s.warnings.iter().find(|w| w.starts_with(&blocked)).cloned());
        if let Some(warning) = warning {
            // Se sale de sus vecinos: se deja como estaba.
            self.host
                .set_position(&origin.sample, origin.count, slice, previous);
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
        let name = self.file_name();
        let base = self.path.parent().unwrap_or(Path::new(".")).to_path_buf();
        let text = self.text();
        match self.host.load(&mut self.controller, &text, &name, &base) {
            Ok(song) => {
                self.error = None;
                self.message = "cargado: entra en el siguiente compás".into();
                for warning in &song.warnings {
                    self.message.push_str(&format!(" · ⚠ {warning}"));
                }
                if !self.started && self.controller.start().is_ok() {
                    self.started = true;
                }
                let count = self.host.song().map_or(0, |s| s.tracks.len());
                if self.selected >= count {
                    self.selected = 0;
                }
                self.update_waveforms();
                self.sync_mix();
                if let Some(on) = self.host.song().and_then(|s| s.metronome) {
                    self.set_metronome(on);
                }
            }
            Err(e) => {
                self.message = "error: sigue sonando lo anterior".into();
                self.error = Some(e);
            }
        }
    }

    /// Calcula los resúmenes de los samples nuevos y olvida los que ya no se
    /// usan. Se hace al cargar, no al dibujar.
    fn update_waveforms(&mut self) {
        let Some(song) = self.host.song() else { return };
        let mut next = HashMap::new();
        for track in &song.tracks {
            let key = Arc::as_ptr(&track.sample) as usize;
            let entry = self
                .waveforms
                .remove(&key)
                .unwrap_or_else(|| (track.sample.clone(), Arc::new(Waveform::of(&track.sample))));
            next.insert(key, entry);
        }
        self.waveforms = next;
    }

    pub fn toggle_transport(&mut self) {
        let running = self.status.is_some_and(|s| s.running) || self.controller.is_playing();
        let result = if running {
            self.controller.stop()
        } else {
            self.controller.start()
        };
        self.message = match result {
            Ok(()) if running => "parado".into(),
            Ok(()) => "en marcha".into(),
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

    /// Dispara ya el corte `index` (desde 0) de la pista seleccionada.
    pub fn play_pad(&mut self, index: usize) {
        let Some(track) = self.host.song().and_then(|s| s.tracks.get(self.selected)) else {
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
        let label = track
            .labels
            .get(index)
            .cloned()
            .flatten()
            .map_or(String::new(), |l| format!(" ({l})"));
        self.message = match self.controller.play_slice_on(slot, &sample, &slices, index) {
            Ok(()) => {
                self.selected_slice = index;
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
            for name in song.tracks.iter().flat_map(|t| t.labels.iter().flatten()) {
                add(name);
            }
        }
        for label in self.host.labels() {
            add(&label.name);
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
        if let Some(effect) = (candidate.kind == "efecto")
            .then(|| api::effect(&candidate.label))
            .flatten()
        {
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
        palette::filter(&palette::entries(self.last_palette), query)
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
            Action::Complete => self.open_completion(),
            Action::SaveAndLoad => self.save(),
            Action::Load => self.load(),
            Action::Transport => self.toggle_transport(),
            Action::GotoError => self.goto_error(),
            Action::NextTrack => self.next_track(),
            Action::PlayMode => self.toggle_mode(),
            Action::ReloadExternal => self.reload_external(),
            Action::Export => self.export(&[]),
            Action::Commands => self.prompt = Some(String::new()),
            Action::LabelSlice => self.start_labeling(),
            Action::Mute => self.toggle_live(false),
            Action::Solo => self.toggle_live(true),
            Action::ClearMuteSolo => self.clear_live(),
            Action::ResetMeters => self.reset_meters(),
            Action::Metronome => self.toggle_metronome(),
            Action::DetectBpm => self.detect_bpm(),
            Action::Preview => self.preview_selected(),
            Action::SliceEarlier => self.adjust_slice(Some(-5.0)),
            Action::SliceLater => self.adjust_slice(Some(5.0)),
            Action::ClearSliceAdjust => self.adjust_slice(None),
            Action::NextSlice => self.select_slice(true),
            Action::PreviousSlice => self.select_slice(false),
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
        if let Some(group) = self.undo_groups.pop() {
            if self.editor.lines() == group.after {
                while self.editor.lines() != group.before && self.editor.undo() {}
                self.redo_groups.push(group);
                return true;
            }
            self.undo_groups.push(group);
        }
        self.editor.undo()
    }

    /// Rehace un cambio (una acción compuesta, entera).
    fn redo(&mut self) -> bool {
        if let Some(group) = self.redo_groups.pop() {
            if self.editor.lines() == group.before {
                while self.editor.lines() != group.after && self.editor.redo() {}
                self.undo_groups.push(group);
                return true;
            }
            self.redo_groups.push(group);
        }
        self.editor.redo()
    }

    /// Salir: si hay cambios sin guardar, la primera vez solo avisa.
    fn request_quit(&mut self, armed: bool) -> Flow {
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
        self.message = match self.mode {
            Mode::Play => {
                "modo tocar: 1-0 y q-i disparan cortes · ←→ recorren y preescuchan · espacio preescucha · m mute · s solo · Esc vuelve al editor"
            }
            Mode::Edit => "modo editor",
        }
        .into();
    }

    pub fn next_track(&mut self) {
        let count = self.host.song().map_or(0, |s| s.tracks.len());
        if count > 0 {
            self.selected = (self.selected + 1) % count;
            self.selected_slice = 0;
        }
    }

    /// Envía al motor el mute y el solo en vivo de cada pista (solo lo que
    /// ha cambiado: las pistas pueden haber cambiado de sitio al recargar).
    fn sync_mix(&mut self) {
        let mut want = [(false, false); MAX_TRACKS];
        if let Some(song) = self.host.song() {
            for track in &song.tracks {
                if let Some(slot) = self.host.track_slot(&track.name) {
                    want[slot] = (
                        self.muted.contains(&track.name),
                        self.soloed.contains(&track.name),
                    );
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
        let Some(name) = self
            .host
            .song()
            .and_then(|s| s.tracks.get(self.selected))
            .map(|t| t.name.clone())
        else {
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
        let Some(track) = self.host.song().and_then(|s| s.tracks.get(self.selected)) else {
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
        let Some(track) = self.host.song().and_then(|s| s.tracks.get(self.selected)) else {
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
        let Some(track) = self.host.song().and_then(|s| s.tracks.get(self.selected)) else {
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
        let Some(track) = self.host.song().and_then(|s| s.tracks.get(self.selected)) else {
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
        if input.key == Key::F(1) {
            self.open_palette();
            return Flow::Continue;
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
        if (input.key, input.ctrl) != (Key::Char('s'), true) {
            self.overwrite_armed = false;
        }
        match (input.key, input.ctrl) {
            (Key::Char('q'), true) => return self.request_quit(quit_armed),
            // Deshacer y rehacer pasan por la app (acciones compuestas).
            (Key::Char('z' | 'u'), true) => return self.run_action(Action::Undo),
            (Key::Char('r'), true) => return self.run_action(Action::Redo),
            (Key::Char('s'), true) => self.save(),
            (Key::Char('l'), true) | (Key::F(5), _) => self.load(),
            (Key::Char('t'), true) | (Key::F(8), _) => self.toggle_transport(),
            (Key::Char('g'), true) => self.goto_error(),
            (Key::Char('o'), true) => self.next_track(),
            (Key::F(3), _) => self.reload_external(),
            (Key::F(2), _) => self.prompt = Some(String::new()),
            (Key::F(4), _) if self.mode == Mode::Edit => {
                self.open_completion();
                return Flow::Continue;
            }
            (Key::Esc, _) => self.toggle_mode(),
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
                }
            }
            (Key::Left, _) if self.mode == Mode::Play => self.browse_slice(false),
            (Key::Right, _) if self.mode == Mode::Play => self.browse_slice(true),
            // En modo tocar el texto no se toca.
            _ if self.mode == Mode::Play => {}
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

    fn then(&mut self, f: impl FnOnce(&mut App)) -> Flow {
        f(self);
        Flow::Continue
    }

    /// Lee el estado del motor, actualiza medidores y recoge la basura.
    pub fn tick(&mut self) {
        if self.last_external_check.elapsed() >= EXTERNAL_CHECK {
            self.last_external_check = Instant::now();
            self.check_external();
        }
        let status = self.controller.status();
        let now = Instant::now();
        let hold = Some(now + WARNING_HOLD);
        for (t, track) in status.tracks.iter().enumerate() {
            self.meters[t] = track.peak.max(self.meters[t] * METER_DECAY).min(1.0);
            if track.peak > 1.0 {
                self.track_clip_until[t] = hold;
            }
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
        let tracks: Vec<TrackView> = self
            .host
            .song()
            .map(|song| {
                song.tracks
                    .iter()
                    .map(|t| {
                        let slot = self.host.track_slot(&t.name);
                        let track_status = slot.and_then(|s| status.map(|st| st.tracks[s]));
                        let playing_slice = track_status
                            .and_then(|ts| ts.hit_start)
                            .and_then(|start| t.slices.iter().position(|r| r.start == start));
                        let bar = (transport.bar % t.bars.len().max(1) as u64) as usize;
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
                            steps: t.bars.get(bar).copied().unwrap_or(t.steps),
                            playing_slice,
                            level: slot.map_or(0.0, |s| self.meters[s]),
                            gain: t.gain,
                            fx: t.fx.clone(),
                            labels: t.labels.clone(),
                            adjusted: t.adjusted.clone(),
                            swing: t.swing,
                            clipping: slot.is_some_and(|s| {
                                self.track_clip_until[s].is_some_and(|u| Instant::now() < u)
                            }),
                            mute: t.mute || self.muted.contains(&t.name),
                            solo: t.solo || self.soloed.contains(&t.name),
                        }
                    })
                    .collect()
            })
            .unwrap_or_default();
        let selected = (!tracks.is_empty()).then(|| self.selected.min(tracks.len() - 1));
        let selected_slice = selected
            .and_then(|t| tracks[t].slices.len().checked_sub(1))
            .map(|last| self.selected_slice.min(last));
        ViewModel {
            error: self.error.as_ref().map(|e| ErrorView {
                line: error_line(&e.message, &name),
                message: e.message.clone(),
            }),
            diagnostics: diagnose::steps_diagnostics(self.editor.lines(), &self.known_labels()),
            help: (self.mode == Mode::Edit && self.palette.is_none())
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
            prompt: self.prompt.clone(),
            project: self
                .project
                .as_ref()
                .and_then(|p| p.file_name())
                .map(|n| n.to_string_lossy().into()),
            message: self.message.clone(),
            transport,
            metronome: self.metronome,
            tracks,
            selected,
            selected_slice,
            master: MasterView {
                level: self.master_meter,
                limiting: self.limiting_until.is_some_and(|u| Instant::now() < u),
                clipping: self.clipping_until.is_some_and(|u| Instant::now() < u),
            },
        }
    }
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
