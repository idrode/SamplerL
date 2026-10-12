//! Estado y lógica de la interfaz, sin terminal (se prueba sin pantalla).

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use engine::{Controller, MAX_TRACKS, Sample, Status};
use ratatui_textarea::{CursorMove, Input, Key, TextArea};
use script::{ScriptError, ScriptHost};

use crate::model::{ErrorView, TrackView, TransportView, ViewModel, Waveform, error_line};

/// Plantilla para un script nuevo.
const TEMPLATE: &str = r#"bpm(170)

local amen = sample("amen.wav"):slice(8)

track("drums", amen)
  :steps("1 . . 1  . . 3 .  1 . 5 .  . 3 . 6")
"#;

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

/// Qué hacer tras una tecla.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Flow {
    Continue,
    Quit,
}

pub struct App {
    pub editor: TextArea<'static>,
    path: PathBuf,
    /// Contenido guardado en disco (para saber si hay cambios).
    saved: String,
    host: ScriptHost,
    controller: Controller,
    error: Option<ScriptError>,
    message: String,
    started: bool,
    selected: usize,
    status: Option<Status>,
    meters: [f32; MAX_TRACKS],
    /// Se pidió salir con cambios sin guardar: otra vez para confirmar.
    quit_armed: bool,
    mode: Mode,
    /// Resúmenes de forma de onda por sample (se guarda el `Arc` para que la
    /// dirección no se reutilice mientras está en la caché).
    waveforms: HashMap<usize, (Arc<Sample>, Arc<Waveform>)>,
}

impl App {
    /// Abre el script (o una plantilla si no existe) y lo carga.
    pub fn open(path: &Path, controller: Controller, sample_rate: u32) -> std::io::Result<App> {
        let (text, saved) = match std::fs::read_to_string(path) {
            Ok(text) => (text.clone(), text),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                (TEMPLATE.to_string(), String::new())
            }
            Err(e) => return Err(e),
        };
        let mut app = App {
            editor: TextArea::new(text.lines().map(String::from).collect()),
            path: path.to_path_buf(),
            saved,
            host: ScriptHost::new(sample_rate),
            controller,
            error: None,
            message: String::new(),
            started: false,
            selected: 0,
            status: None,
            meters: [0.0; MAX_TRACKS],
            quit_armed: false,
            mode: Mode::Edit,
            waveforms: HashMap::new(),
        };
        app.load();
        Ok(app)
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

    pub fn dirty(&self) -> bool {
        self.text() != self.saved
    }

    /// Guarda el editor en disco y lo carga.
    pub fn save(&mut self) {
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

    /// Carga el contenido del editor (sin guardar). Entra en el siguiente
    /// compás; si falla, sigue sonando lo anterior y el error va al editor.
    pub fn load(&mut self) {
        let name = self.file_name();
        let base = self.path.parent().unwrap_or(Path::new(".")).to_path_buf();
        let text = self.text();
        match self.host.load(&mut self.controller, &text, &name, &base) {
            Ok(_) => {
                self.error = None;
                self.message = "cargado: entra en el siguiente compás".into();
                if !self.started && self.controller.start().is_ok() {
                    self.started = true;
                }
                let count = self.host.song().map_or(0, |s| s.tracks.len());
                if self.selected >= count {
                    self.selected = 0;
                }
                self.update_waveforms();
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
        self.message = match self.controller.play_slice_on(slot, &sample, &slices, index) {
            Ok(()) => format!("▶ {name}: corte {}", index + 1),
            Err(engine::SendError::NoSuchSlice { count, .. }) => {
                format!("{name} no tiene corte {} (tiene {count})", index + 1)
            }
            Err(e) => format!("motor: {e}"),
        };
    }

    fn toggle_mode(&mut self) {
        self.mode = match self.mode {
            Mode::Edit => Mode::Play,
            Mode::Play => Mode::Edit,
        };
        self.message = match self.mode {
            Mode::Play => "modo tocar: 1-0 y q-i disparan cortes · Esc vuelve al editor",
            Mode::Edit => "modo editor",
        }
        .into();
    }

    pub fn next_track(&mut self) {
        let count = self.host.song().map_or(0, |s| s.tracks.len());
        if count > 0 {
            self.selected = (self.selected + 1) % count;
        }
    }

    /// Atiende una tecla: atajos propios; si no, en modo tocar dispara
    /// cortes y en modo editor escribe.
    pub fn handle(&mut self, input: Input) -> Flow {
        let quit_armed = std::mem::replace(&mut self.quit_armed, false);
        match (input.key, input.ctrl) {
            (Key::Char('q'), true) => {
                if self.dirty() && !quit_armed {
                    self.quit_armed = true;
                    self.message = "hay cambios sin guardar: Ctrl-Q otra vez para salir".into();
                } else {
                    return Flow::Quit;
                }
            }
            (Key::Char('s'), true) => self.save(),
            (Key::Char('l'), true) | (Key::F(5), _) => self.load(),
            (Key::Char('t'), true) | (Key::F(8), _) => self.toggle_transport(),
            (Key::Char('g'), true) => self.goto_error(),
            (Key::Char('o'), true) => self.next_track(),
            (Key::Esc, _) => self.toggle_mode(),
            (Key::Char(c), false) if self.mode == Mode::Play && !input.alt => {
                if let Some(index) = PADS.iter().position(|&p| p == c.to_ascii_lowercase()) {
                    self.play_pad(index);
                }
            }
            // En modo tocar el texto no se toca.
            _ if self.mode == Mode::Play => {}
            _ => {
                self.editor.input(input);
            }
        }
        Flow::Continue
    }

    /// Lee el estado del motor, actualiza medidores y recoge la basura.
    pub fn tick(&mut self) {
        let status = self.controller.status();
        for (meter, track) in self.meters.iter_mut().zip(&status.tracks) {
            *meter = track.peak.max(*meter * METER_DECAY).min(1.0);
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
                        }
                    })
                    .collect()
            })
            .unwrap_or_default();
        let selected = (!tracks.is_empty()).then(|| self.selected.min(tracks.len() - 1));
        ViewModel {
            error: self.error.as_ref().map(|e| ErrorView {
                line: error_line(&e.message, &name),
                message: e.message.clone(),
            }),
            file: name,
            dirty: self.dirty(),
            playing_mode: self.mode == Mode::Play,
            message: self.message.clone(),
            transport,
            tracks,
            selected,
        }
    }
}
