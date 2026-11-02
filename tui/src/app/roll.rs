//! El piano roll en la aplicación (ver [`crate::roll`] y
//! [`crate::roll_panel`]): la pista seleccionada en una cuadrícula y las
//! ediciones que se escriben en el texto Lua con la técnica del tracker
//! editable (solo los caracteres de los tokens afectados, un paso de
//! deshacer, y se carga como Ctrl-L: entra al compás siguiente).
//!
//! Se ve como panel de la ventana principal o en su propia ventana
//! (`tui --window-roll`, por un socket aparte: ver `crate::roll_window`).
//! La principal es la dueña: tiene la canción y el texto, calcula la vista
//! y aplica las teclas y el ratón que le llegan de la ventana. Así el
//! editor (aquí o en su ventana) y el piano roll ven siempre el mismo texto.
//!
//! El ratón coloca, selecciona, mueve y alarga notas sin que suenen; solo
//! la orden de preescucha (Espacio) suena, y el pánico la corta.

use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use ratatui::layout::Rect;
use ratatui_textarea::CursorMove;

use super::mouse::MouseAction;
use super::{App, Flow, Input, Key};
use crate::config::{ROLL_SNAPS, RollConfig};
use crate::link::{self, Conn, FromRoll, Message, Server, ToRoll};
use crate::roll::{self, Axis, Change, Edit, Roll, Style};
use crate::roll_keys::{RollKeys, RollOp};
use crate::roll_panel::{self, RollView, Zoom};

/// Estado del piano roll.
#[derive(Default)]
pub struct RollState {
    /// Se ve (la paleta lo enseña y lo esconde).
    pub visible: bool,
    /// El cursor (columna del ciclo y nota), si tiene las teclas.
    pub cursor: Option<(usize, i32)>,
    /// Dónde empezó la selección, y si la empezó `s` (sigue con las
    /// flechas) o Shift (se quita al moverse sin él).
    anchor: Option<usize>,
    sticky: bool,
    zoom: Zoom,
    /// El zoom lo elige el panel (hasta que se cambia a mano).
    zoom_auto: bool,
    /// Pasos de la rejilla y si está encendida.
    snap: usize,
    snap_on: bool,
    preview_on_insert: bool,
    /// Lo que se ve: primera columna y nota de arriba (`None`: la
    /// tesitura).
    first: usize,
    top: Option<i32>,
    /// Dónde se dibujó el panel por última vez (aquí o en su ventana):
    /// para el ratón y para desplazar siguiendo al cursor.
    pub area: Option<Rect>,
    /// La pista que se enseñaba (otra pista: otra vista).
    track: String,
    /// Lo que se arrastra con el ratón.
    drag: Option<Drag>,
    /// La preescucha que suena y cuándo se suelta.
    preview: Option<(Instant, Vec<u8>)>,
    keys: RollKeys,
    /// El último modelo calculado: se rehace solo con otra carga (otra
    /// etiqueta de la pista), otra pista o otro texto.
    cache: std::cell::RefCell<Option<(u32, u64, std::rc::Rc<Roll>)>>,
    /// En su ventana.
    link: Option<RollLink>,
    terminal: Option<Vec<String>>,
}

/// Un arrastre del ratón: desde dónde, la nota (cabeza y duración) y si
/// mueve o alarga.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Drag {
    from: (usize, i32),
    head: usize,
    len: usize,
    resize: bool,
    /// Dónde va ahora (columna y nota).
    to: (usize, i32),
}

/// Teclas de la preescucha en el motor (fuera de las de MIDI y pads).
const PREVIEW_KEYS: u8 = 200;
/// Lo menos que suena una preescucha.
const PREVIEW_MIN: Duration = Duration::from_millis(150);

/// La ventana del piano roll, vista desde la principal.
pub struct RollLink {
    server: Server,
    sent: Option<RollView>,
    child: Option<Child>,
    command: String,
}

/// Teclas que el piano roll deja pasar a la aplicación.
pub(super) fn passes_through(action: crate::palette::Action) -> bool {
    use crate::palette::Action;
    matches!(
        action,
        Action::Undo
            | Action::Redo
            | Action::Load
            | Action::SaveAndLoad
            | Action::Transport
            | Action::Cue
            | Action::NextTrack
            | Action::Quit
    )
}

impl App {
    /// Lo de `[roll]` en `config.toml`.
    pub(super) fn apply_roll_config(&mut self, config: &RollConfig) {
        let r = &mut self.roll;
        r.visible = config.visible;
        r.zoom = config.zoom.unwrap_or_default();
        r.zoom_auto = config.zoom.is_none();
        r.snap = config.snap.max(1);
        r.snap_on = config.snap_on;
        r.preview_on_insert = config.preview_on_insert;
        r.keys = config.keys.clone();
        r.terminal = config.terminal.clone();
    }

    /// El piano roll de la pista seleccionada (lo cargado).
    pub(super) fn current_roll(&self) -> Option<std::rc::Rc<Roll>> {
        use std::hash::{Hash, Hasher};
        let song = self.host.song()?;
        let def = self.tracks().get(self.selected)?;
        let lines = self.editor.lines();
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        lines.hash(&mut hasher);
        self.loaded_text.hash(&mut hasher);
        let text = hasher.finish();
        if let Some((tag, hash, roll)) = &*self.roll.cache.borrow()
            && *tag == def.tag
            && *hash == text
            && roll.track == def.name
        {
            return Some(roll.clone());
        }
        let loaded = self.loaded_text.as_deref() == Some(self.text().as_str());
        let roll = std::rc::Rc::new(roll::build(&roll::Source {
            song,
            def,
            lines,
            loaded,
        }));
        *self.roll.cache.borrow_mut() = Some((def.tag, text, roll.clone()));
        Some(roll)
    }

    /// La columna del ciclo que se oye, con la sincronía del resaltado.
    fn roll_playhead(&self, roll: &Roll) -> Option<usize> {
        let status = self.status.filter(|s| s.running)?;
        let def = self.tracks().get(self.selected)?;
        let cycle = roll.steps().max(1) as i64;
        let extra = self.highlight_delay_ms * status.bpm * 4.0 / 60_000.0;
        let heard = status.heard.map(|h| (h - extra).floor() as i64)?;
        let slot = self.host.track_slot(&def.name)?;
        if let Some(c) = self.cursors.at(slot, heard).filter(|c| c.tag == def.tag) {
            return Some((c.index as i64 + heard - c.global as i64).rem_euclid(cycle) as usize);
        }
        // Una carga nueva que aún no ha entrado: por la vuelta.
        let (lap, step) = def.lap(super::section_global(status));
        let laps = def.bars.len().max(1) as u64;
        Some((lap % laps) as usize * def.length.max(1) + step)
    }

    /// La vista del piano roll, si se ve aquí (o la de su ventana).
    pub(super) fn roll_view(&self) -> Option<RollView> {
        let roll = self.current_roll()?;
        let def = self.tracks().get(self.selected)?;
        let r = &self.roll;
        // Un cursor de otra pista no vale aquí (lo pone bien `tick_roll`).
        let cursor = r.cursor.filter(|_| r.track == def.name);
        let kind = match def.instrument {
            Some(i) if i.synth.is_some() => i.synth.map_or("synth", |s| s.kind).to_string(),
            Some(_) => "instrumento".into(),
            None if roll.axis == Axis::Notes => "cortes con melody()".into(),
            None => format!("{} cortes", roll.labels.len()),
        };
        let laps = def.bars.len().max(1);
        let laps_text = if laps == 1 {
            format!("{} pasos", def.length)
        } else {
            format!("{laps} vueltas de {}", def.length)
        };
        let key = self
            .host
            .song()
            .and_then(|s| s.key.map(|k| k.transposed(s.transpose)))
            .map_or(String::new(), |k| {
                format!(" · {} {}", k.tonic_name(), k.scale.name)
            });
        let status = match cursor {
            Some((col, pitch)) => roll.describe(col, pitch),
            None => format!(
                "F1 «Editar en el piano roll» o un clic para editar{}",
                roll.locked
                    .as_ref()
                    .map_or(String::new(), |w| format!(" · {w}"))
            ),
        };
        let snap = if r.snap_on {
            let name = ROLL_SNAPS
                .iter()
                .find(|(_, s)| *s == r.snap)
                .map_or("1/16", |(n, _)| n);
            format!("rejilla {name}")
        } else {
            "rejilla apagada".into()
        };
        let color = self
            .host
            .song()
            .map(|s| s.track_names())
            .and_then(|names| names.iter().position(|n| *n == def.name))
            .unwrap_or(self.selected);
        Some(RollView {
            title: format!("{} · {kind} · {laps_text}{key}", def.name),
            notes_axis: roll.axis == Axis::Notes,
            steps: roll.steps(),
            length: roll.length,
            bar_steps: self.status.map_or(engine::STEPS, |s| s.bar_steps.max(1)),
            labels: match roll.axis {
                Axis::Slices => (0..roll.labels.len())
                    .map(|i| match &roll.labels[i] {
                        Some(l) => format!("{} {l}", i + 1),
                        None => (i + 1).to_string(),
                    })
                    .collect(),
                Axis::Notes => Vec::new(),
            },
            notes: roll.notes.clone(),
            scale: roll.scale,
            range: roll.range(),
            bounds: roll.bounds(),
            cursor,
            selection: cursor.and(self.roll_selection()),
            playhead: self.roll_playhead(&roll),
            zoom: r.zoom,
            zoom_auto: r.zoom_auto,
            first: r.first,
            top: r.top,
            status,
            snap,
            color,
        })
    }

    /// La vista para el modelo de esta ventana: solo si se ve aquí.
    pub(super) fn local_roll_view(&self) -> Option<RollView> {
        if !self.roll.visible || self.roll_outside() {
            return None;
        }
        self.roll_view()
    }

    /// El piano roll se ve en esta ventana.
    pub(super) fn local_roll_view_shown(&self) -> bool {
        self.roll.visible && !self.roll_outside()
    }

    /// Dónde se dibujó el panel (lo pone `ui::draw`).
    pub fn set_roll_area(&mut self, area: Option<Rect>) {
        if !self.roll_outside() {
            self.roll.area = area;
        }
    }

    /// El piano roll está en su ventana.
    fn roll_outside(&self) -> bool {
        self.roll
            .link
            .as_ref()
            .is_some_and(|l| l.server.connected())
    }

    fn roll_selection(&self) -> Option<(usize, usize)> {
        let (col, _) = self.roll.cursor?;
        let a = self.roll.anchor?;
        Some((a.min(col), a.max(col)))
    }

    /// «Piano roll» en la paleta: lo enseña o lo esconde.
    pub(super) fn toggle_roll(&mut self) {
        if self.roll_outside() {
            self.message = "el piano roll está en su ventana (F1 «Volver a meter el piano roll en la principal»)".into();
            return;
        }
        self.roll.visible = !self.roll.visible;
        if !self.roll.visible {
            self.close_roll_cursor();
        }
        self.message = if self.roll.visible {
            "piano roll de la pista seleccionada · F1 «Editar en el piano roll» o un clic para editar"
                .into()
        } else {
            "piano roll oculto".into()
        };
    }

    /// «Editar en el piano roll»: lo enseña y le da las teclas, con el
    /// cursor en el paso que se oye (o el primero) y en su nota.
    pub(super) fn open_roll_cursor(&mut self) {
        if !self.roll_outside() {
            self.roll.visible = true;
        }
        let Some(roll) = self.current_roll() else {
            self.message = "no hay pistas en el piano roll".into();
            return;
        };
        self.tracker_edit = None;
        self.pending_edit = None;
        self.occupancy.cursor = None;
        let col = self.roll_playhead(&roll).unwrap_or(0);
        let pitch = roll
            .notes
            .iter()
            .filter(|n| n.style != Style::Dropped)
            .min_by_key(|n| (n.col as i64 - col as i64).abs())
            .map_or_else(
                || roll.range().map_or(roll.root, |(a, b)| (a + b) / 2),
                |n| n.pitch,
            );
        self.roll.cursor = Some((col, pitch));
        self.roll.track = roll.track.clone();
        self.roll.anchor = None;
        self.message = self.roll_help();
    }

    fn close_roll_cursor(&mut self) {
        self.roll.cursor = None;
        self.roll.anchor = None;
        self.roll.drag = None;
    }

    fn roll_help(&self) -> String {
        let k = |op| self.roll.keys.describe(op);
        format!(
            "piano roll: flechas mueven · {} nota · {} quita · {}/{} semitono · {}/{} octava · {}/{} duración · {}/{} mover · {}/{} volumen · {} duplicar · {} escuchar · {} rejilla · {}/{} zoom · {} sale",
            k(RollOp::Insert),
            k(RollOp::Delete),
            k(RollOp::Up),
            k(RollOp::Down),
            k(RollOp::OctaveUp),
            k(RollOp::OctaveDown),
            k(RollOp::Longer),
            k(RollOp::Shorter),
            k(RollOp::Earlier),
            k(RollOp::Later),
            k(RollOp::Louder),
            k(RollOp::Softer),
            k(RollOp::Duplicate),
            k(RollOp::Preview),
            k(RollOp::ToggleSnap),
            k(RollOp::ZoomIn),
            k(RollOp::ZoomOut),
            k(RollOp::Exit),
        )
    }

    /// El zoom con que se ve ahora (el automático, el que dio el panel).
    fn roll_zoom_now(&self) -> Zoom {
        match (self.roll_view(), self.roll.area) {
            (Some(view), Some(area)) => roll_panel::layout(&view, area).zoom,
            _ => self.roll.zoom,
        }
    }

    /// Pasos que se mueve el cursor (y las notas) con la rejilla.
    fn roll_snap(&self) -> usize {
        if self.roll.snap_on {
            self.roll.snap.max(1)
        } else {
            1
        }
    }

    /// Una tecla con el piano roll al mando.
    pub(super) fn handle_roll(&mut self, input: Input) -> Flow {
        let Some(roll) = self.current_roll() else {
            self.close_roll_cursor();
            return Flow::Continue;
        };
        // Otra pista seleccionada: el cursor vuelve a empezar en ella.
        if roll.track != self.roll.track {
            self.open_roll_cursor();
        }
        let Some((mut col, mut pitch)) = self.roll.cursor else {
            return Flow::Continue;
        };
        let steps = roll.steps().max(1);
        let (lo, hi) = roll.bounds();
        let snap = self.roll_snap();
        let selection = self.roll_selection();
        let arrow = matches!(
            input.key,
            Key::Left
                | Key::Right
                | Key::Up
                | Key::Down
                | Key::PageUp
                | Key::PageDown
                | Key::Home
                | Key::End
        );
        if arrow && !input.ctrl && !input.alt {
            if input.shift && matches!(input.key, Key::Left | Key::Right) {
                if self.roll.anchor.is_none() {
                    self.roll.anchor = Some(col);
                }
            } else if !self.roll.sticky {
                self.roll.anchor = None;
            }
            match input.key {
                Key::Left => {
                    col = if col % snap != 0 {
                        col - col % snap
                    } else {
                        col.saturating_sub(snap)
                    }
                }
                Key::Right => col = ((col / snap + 1) * snap).min(steps - 1),
                Key::Up => pitch = (pitch + 1).min(hi),
                Key::Down => pitch = (pitch - 1).max(lo),
                Key::PageUp => pitch = (pitch + 12).min(hi),
                Key::PageDown => pitch = (pitch - 12).max(lo),
                Key::Home => col = 0,
                Key::End => col = steps - 1,
                _ => {}
            }
            self.roll.cursor = Some((col, pitch));
            self.message = roll.describe(col, pitch);
            return Flow::Continue;
        }
        let Some(op) = self.roll.keys.lookup(&input) else {
            return Flow::Continue;
        };
        let edit = match op {
            RollOp::Exit => {
                if self.roll.anchor.is_some() {
                    self.roll.anchor = None;
                    self.roll.sticky = false;
                    self.message = "sin selección".into();
                } else {
                    self.close_roll_cursor();
                    self.message = "fuera del piano roll (sigue a la vista)".into();
                }
                return Flow::Continue;
            }
            RollOp::Select => {
                if self.roll.anchor.is_some() {
                    self.roll.anchor = None;
                    self.roll.sticky = false;
                    self.message = "sin selección".into();
                } else {
                    self.roll.anchor = Some(col);
                    self.roll.sticky = true;
                    self.message = "selección: ←→ la alargan; +/-, </>, ,/. y d actúan sobre ella; s o Esc la quitan".into();
                }
                return Flow::Continue;
            }
            RollOp::ToggleSnap => {
                self.roll.snap_on = !self.roll.snap_on;
                self.message = if self.roll.snap_on {
                    "rejilla encendida".into()
                } else {
                    "rejilla apagada: de paso en paso".into()
                };
                return Flow::Continue;
            }
            RollOp::CycleSnap => {
                let i = ROLL_SNAPS
                    .iter()
                    .position(|(_, s)| *s == self.roll.snap)
                    .map_or(0, |i| (i + 1) % ROLL_SNAPS.len());
                self.roll.snap = ROLL_SNAPS[i].1;
                self.roll.snap_on = true;
                self.message = format!("rejilla {}", ROLL_SNAPS[i].0);
                return Flow::Continue;
            }
            RollOp::ZoomIn | RollOp::ZoomOut => {
                self.roll.zoom = self.roll_zoom_now().step(op == RollOp::ZoomIn);
                self.roll.zoom_auto = false;
                self.message = format!("zoom {}", self.roll.zoom.name());
                return Flow::Continue;
            }
            RollOp::Fit => {
                self.roll.top = None;
                self.message = "vista ajustada a la tesitura".into();
                return Flow::Continue;
            }
            RollOp::Preview => {
                self.roll_preview(&roll, col, pitch);
                return Flow::Continue;
            }
            RollOp::NextNote | RollOp::PreviousNote => {
                let mut heads: Vec<(usize, i32)> = roll
                    .notes
                    .iter()
                    .filter(|n| n.style != Style::Dropped)
                    .map(|n| (n.col, n.pitch))
                    .collect();
                heads.sort_unstable();
                let next = if op == RollOp::NextNote {
                    heads.iter().find(|h| **h > (col, pitch)).or(heads.first())
                } else {
                    heads
                        .iter()
                        .rev()
                        .find(|h| **h < (col, pitch))
                        .or(heads.last())
                };
                if let Some(&(c, p)) = next {
                    self.roll.cursor = Some((c, p));
                    self.message = roll.describe(c, p);
                }
                return Flow::Continue;
            }
            RollOp::Insert => Edit::Insert { col, pitch },
            RollOp::Delete => Edit::Delete { col, pitch },
            RollOp::Up | RollOp::Down | RollOp::OctaveUp | RollOp::OctaveDown => Edit::Transpose {
                col,
                pitch,
                semitones: match op {
                    RollOp::Up => 1,
                    RollOp::Down => -1,
                    RollOp::OctaveUp => 12,
                    _ => -12,
                },
                selection,
            },
            RollOp::Longer => Edit::Resize {
                col,
                pitch,
                delta: 1,
            },
            RollOp::Shorter => Edit::Resize {
                col,
                pitch,
                delta: -1,
            },
            RollOp::Earlier | RollOp::Later => Edit::Move {
                col,
                pitch,
                delta: if op == RollOp::Later {
                    snap as i32
                } else {
                    -(snap as i32)
                },
                semitones: 0,
                selection,
            },
            RollOp::Louder => Edit::Gain {
                col,
                pitch,
                delta: 0.1,
            },
            RollOp::Softer => Edit::Gain {
                col,
                pitch,
                delta: -0.1,
            },
            RollOp::Duplicate => Edit::Duplicate {
                col,
                pitch,
                selection,
            },
        };
        if self.roll_edit(&roll, edit) && op == RollOp::Insert && self.roll.preview_on_insert {
            // Suena con lo que se acaba de escribir (aún sin cargar).
            self.roll_preview_note(&roll, col, &[pitch], 1, 1.0);
        }
        Flow::Continue
    }

    /// Calcula la edición, la escribe en el texto (un paso de deshacer) y
    /// carga. Si no se puede, lo dice y no toca nada.
    fn roll_edit(&mut self, roll: &Roll, edit: Edit) -> bool {
        match roll::apply(roll, edit) {
            Ok(change) => {
                self.write_roll_change(&change);
                true
            }
            Err(why) => {
                self.message = why;
                false
            }
        }
    }

    fn write_roll_change(&mut self, change: &Change) {
        self.completion = None;
        self.marks.clear();
        // De atrás adelante: lo de delante no se mueve.
        let mut edits = change.edits.clone();
        edits.sort_by_key(|e| std::cmp::Reverse(e.0));
        self.grouped(|editor| {
            for (span, text) in &edits {
                editor.cancel_selection();
                editor.move_cursor(CursorMove::Jump(span.row as u16, span.start as u16));
                editor.delete_str(span.end - span.start);
                editor.insert_str(text);
            }
        });
        self.load();
        self.roll.cursor = Some(change.cursor);
        if let Some((a, b)) = change.selection {
            self.roll.anchor = Some(if change.cursor.0 == a { b } else { a });
        }
        if self.error.is_none() {
            self.message = format!(
                "{} · entra en el compás siguiente (Ctrl-Z lo deshace)",
                change.message
            );
        }
    }

    /// Oye la nota del cursor (o, en un hueco, la de esa fila).
    fn roll_preview(&mut self, roll: &Roll, col: usize, pitch: i32) {
        let (pitches, len, gain) = match roll.note_at(col, pitch) {
            Some(n) => {
                let step = roll.columns[n.col].hit;
                (roll.pitches(n.col), n.len, step.map_or(1.0, |s| s.gain))
            }
            None => (vec![pitch], 1, 1.0),
        };
        self.roll_preview_note(roll, col, &pitches, len, gain);
    }

    fn roll_preview_note(
        &mut self,
        roll: &Roll,
        _col: usize,
        pitches: &[i32],
        len: usize,
        gain: f32,
    ) {
        self.stop_roll_preview();
        let Some(def) = self.tracks().get(self.selected).cloned() else {
            return;
        };
        let bpm = self.status.map_or(engine::DEFAULT_BPM, |s| s.bpm);
        let step = Duration::from_secs_f64(60.0 / bpm / 4.0);
        let length = (step * len as u32).max(PREVIEW_MIN);
        match (roll.axis, def.instrument) {
            (Axis::Notes, Some(_)) => {
                let now = Instant::now();
                let mut keys = Vec::new();
                for (i, p) in pitches.iter().enumerate() {
                    let key = PREVIEW_KEYS + i as u8;
                    self.live_note_on(key, (p - roll.root) as f32, gain, now);
                    keys.push(key);
                }
                self.roll.preview = Some((now + length, keys));
                self.message = format!(
                    "♪ {}: {}",
                    def.name,
                    pitches
                        .iter()
                        .map(|p| engine::note_name(*p))
                        .collect::<Vec<_>>()
                        .join("+")
                );
            }
            (Axis::Slices, _) => {
                let slice = pitches.first().copied().unwrap_or(0).max(0) as usize;
                self.preview_track_slice(&def, slice);
            }
            (Axis::Notes, None) => {
                // melody() en una pista de cortes: el corte 1 suena tal cual.
                self.preview_track_slice(&def, 0);
                self.message.push_str(" (sin el tono: es un corte)");
            }
        }
    }

    fn preview_track_slice(&mut self, def: &script::TrackDef, slice: usize) {
        let result = self
            .controller
            .preview_slice(&def.sample, &def.slices, slice);
        self.message = match result {
            Ok(()) => format!("preescucha {}: corte {}", def.name, slice + 1),
            Err(e) => format!("motor: {e}"),
        };
    }

    /// Suelta la preescucha que suena.
    fn stop_roll_preview(&mut self) {
        if let Some((_, keys)) = self.roll.preview.take() {
            for key in keys {
                self.live_note_off(key);
            }
        }
    }

    /// El pánico: la preescucha ya no hace falta soltarla.
    pub(super) fn forget_roll_preview(&mut self) {
        self.roll.preview = None;
    }

    /// En cada vuelta: suelta la preescucha a su hora y atiende la ventana.
    pub(super) fn tick_roll(&mut self) {
        // Otra pista seleccionada: el cursor empieza otra vez en ella.
        if self.roll.cursor.is_some()
            && self
                .tracks()
                .get(self.selected)
                .is_some_and(|t| t.name != self.roll.track)
        {
            self.open_roll_cursor();
        }
        if self
            .roll
            .preview
            .as_ref()
            .is_some_and(|(until, _)| Instant::now() >= *until)
        {
            self.stop_roll_preview();
        }
        self.poll_roll_link();
    }

    /// El ratón sobre el piano roll en `area` (el panel entero): coloca,
    /// selecciona, mueve, alarga y desplaza. Nunca suena. Devuelve si era
    /// suyo.
    pub(super) fn roll_mouse(&mut self, action: MouseAction, x: u16, y: u16, area: Rect) -> bool {
        let Some(view) = self.roll_view() else {
            return false;
        };
        let lay = roll_panel::layout(&view, area);
        let inside = lay.hit(x, y);
        if inside.is_none() && self.roll.drag.is_none() {
            return area.contains(ratatui::layout::Position::new(x, y));
        }
        let Some(roll) = self.current_roll() else {
            return true;
        };
        let snap = self.roll_snap();
        let snapped = |c: usize| c - c % snap;
        match action {
            MouseAction::ScrollUp | MouseAction::ScrollDown => {
                let top = self.roll.top.unwrap_or(lay.top);
                let d = if action == MouseAction::ScrollUp {
                    3
                } else {
                    -3
                };
                let (lo, hi) = roll.bounds();
                self.roll.top = Some((top + d).clamp(lo, hi.max(lo)));
                if let Some((c, p)) = self.roll.cursor {
                    // Que el cursor no tire de la vista hacia atrás.
                    let rows = lay.rows();
                    let top = self.roll.top.unwrap_or(top);
                    self.roll.cursor = Some((c, p.clamp(top - rows + 1, top)));
                }
            }
            MouseAction::ScrollLeft | MouseAction::ScrollRight => {
                let shift = lay.shown().max(4) / 4;
                self.roll.first = if action == MouseAction::ScrollLeft {
                    lay.first.saturating_sub(shift)
                } else {
                    (lay.first + shift).min(roll.steps().saturating_sub(1))
                };
                if let Some((c, p)) = self.roll.cursor {
                    let first = self.roll.first;
                    let last = first + lay.shown().saturating_sub(1);
                    self.roll.cursor = Some((c.clamp(first, last.max(first)), p));
                }
            }
            MouseAction::ZoomIn | MouseAction::ZoomOut => {
                self.roll.zoom = lay.zoom.step(action == MouseAction::ZoomIn);
                self.roll.zoom_auto = false;
                self.message = format!("zoom {}", self.roll.zoom.name());
            }
            MouseAction::Down => {
                let Some((col, pitch)) = inside else {
                    return true;
                };
                if col >= roll.steps() {
                    return true;
                }
                self.tracker_edit = None;
                self.occupancy.cursor = None;
                self.roll.track = roll.track.clone();
                self.roll.anchor = None;
                self.roll.sticky = false;
                match roll.note_at(col, pitch) {
                    Some(n) => {
                        self.roll.cursor = Some((col, pitch));
                        self.roll.drag = Some(Drag {
                            from: (col, pitch),
                            head: n.col,
                            len: n.len,
                            resize: col != n.col,
                            to: (col, pitch),
                        });
                        self.message = roll.describe(col, pitch);
                    }
                    None => {
                        // Un clic en un hueco pone una nota (sin sonar).
                        let col = snapped(col);
                        self.roll.cursor = Some((col, pitch));
                        self.roll_edit(&roll, Edit::Insert { col, pitch });
                    }
                }
            }
            MouseAction::Drag => {
                let Some(mut drag) = self.roll.drag else {
                    return true;
                };
                if let Some((col, pitch)) = inside {
                    drag.to = (col, if drag.resize { drag.from.1 } else { pitch });
                    self.roll.drag = Some(drag);
                    self.roll.cursor = Some(drag.to);
                    self.message = if drag.resize {
                        format!(
                            "alargar hasta el paso {} (al soltar)",
                            col % roll.length + 1
                        )
                    } else {
                        format!(
                            "mover a {} · paso {} (al soltar)",
                            roll.pitch_name(pitch),
                            col % roll.length + 1
                        )
                    };
                }
            }
            MouseAction::Up => {
                let Some(drag) = self.roll.drag.take() else {
                    return true;
                };
                let (to_col, to_pitch) = drag.to;
                if drag.to == drag.from {
                    self.roll.cursor = Some(drag.from);
                    return true;
                }
                let edit = if drag.resize {
                    let tail = drag.head + drag.len - 1;
                    let delta = to_col as i64 - tail as i64;
                    Edit::Resize {
                        col: drag.from.0,
                        pitch: drag.from.1,
                        delta: delta as i32,
                    }
                } else {
                    // La cabeza se mueve lo que el ratón, a la rejilla.
                    let target = drag.head as i64 + to_col as i64 - drag.from.0 as i64;
                    let target = snapped(target.max(0) as usize) as i64;
                    let semitones = to_pitch - drag.from.1;
                    Edit::Move {
                        col: drag.from.0,
                        pitch: drag.from.1,
                        delta: (target - drag.head as i64) as i32,
                        semitones,
                        selection: None,
                    }
                };
                if !self.roll_edit(&roll, edit) {
                    self.roll.cursor = Some(drag.from);
                }
            }
            MouseAction::RightDown => {
                if let Some((col, pitch)) = inside
                    && roll.note_at(col, pitch).is_some()
                {
                    self.roll.cursor = Some((col, pitch));
                    self.roll_edit(&roll, Edit::Delete { col, pitch });
                }
            }
        }
        true
    }
}

impl App {
    /// «Abrir el piano roll en su ventana»: escucha en su socket y lo abre
    /// en una terminal nueva (`[roll] terminal` o, si no, `[editor]
    /// terminal`). Si no se puede lanzar, lo dice y lo deja aquí.
    pub(super) fn launch_roll_window(&mut self) {
        let target = self.project.clone().unwrap_or_else(|| self.path.clone());
        if self.roll.link.is_none() {
            let dir = match self.socket_dir.clone() {
                Some(dir) => Ok(dir),
                None => link::runtime_dir(),
            };
            let replaced =
                ToRoll::Bye("se ha conectado otro piano roll a esta ventana principal".into());
            let server = dir.and_then(|dir| {
                Server::bind_with(&link::roll_socket_path(&dir, &self.path), replaced.encode())
            });
            match server {
                Ok(server) => {
                    self.roll.link = Some(RollLink {
                        server,
                        sent: None,
                        child: None,
                        command: String::new(),
                    })
                }
                Err(e) => {
                    self.roll_here(format!("no se pudo abrir el piano roll en su ventana: {e}"));
                    return;
                }
            }
        }
        let Some(link) = &mut self.roll.link else {
            return;
        };
        if link.server.connected() {
            self.message = "el piano roll ya está abierto en su ventana".into();
            return;
        }
        let terminal = self
            .roll
            .terminal
            .clone()
            .or_else(|| self.editor_terminal.clone())
            .filter(|t| !t.is_empty());
        let Some(terminal) = terminal else {
            self.roll_here(format!(
                "sin [roll] terminal ni [editor] terminal en config.toml: aquí, o en otra \
                 terminal con tui --window-roll {}",
                target.display()
            ));
            return;
        };
        let exe = std::env::current_exe().unwrap_or_else(|_| "tui".into());
        let args = roll_command(&terminal, &exe, &target);
        let Some(link) = &mut self.roll.link else {
            return;
        };
        let spawned = Command::new(&args[0])
            .args(&args[1..])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn();
        match spawned {
            Ok(child) => {
                link.child = Some(child);
                link.command = args.join(" ");
                self.message = format!("abriendo el piano roll: {}", link.command);
            }
            Err(e) => self.roll_here(format!(
                "no se pudo lanzar «{}» ({e}): el piano roll, aquí",
                args[0]
            )),
        }
    }

    /// Pone el cursor del piano roll (en los tests).
    #[cfg(test)]
    pub fn roll_cursor_to(&mut self, col: usize, pitch: i32) {
        if self.roll.cursor.is_none() {
            self.open_roll_cursor();
        }
        self.roll.cursor = Some((col, pitch));
        self.roll.anchor = None;
        self.roll.sticky = false;
    }

    /// El cursor del piano roll y la selección (en los tests).
    #[cfg(test)]
    #[allow(clippy::type_complexity)]
    pub fn roll_cursor(&self) -> Option<((usize, i32), Option<(usize, usize)>)> {
        Some((self.roll.cursor?, self.roll_selection()))
    }

    /// La vista del piano roll, se vea donde se vea (en los tests).
    #[cfg(test)]
    pub fn roll_view_for_test(&self) -> Option<RollView> {
        self.roll_view()
    }

    /// El modelo del piano roll (en los tests).
    #[cfg(test)]
    pub fn current_roll_for_test(&self) -> Option<std::rc::Rc<Roll>> {
        self.current_roll()
    }

    /// Dónde se dibujó el piano roll (en los tests).
    #[cfg(test)]
    pub fn roll_area_for_test(&self) -> Option<Rect> {
        self.roll.area
    }

    /// La terminal del piano roll (en los tests).
    #[cfg(test)]
    pub fn set_roll_terminal(&mut self, terminal: Option<Vec<String>>) {
        self.roll.terminal = terminal;
    }

    /// Deja el piano roll aquí, con un mensaje de por qué.
    fn roll_here(&mut self, why: String) {
        self.roll.visible = true;
        self.message = why;
    }

    /// «Volver a meter el piano roll en la principal».
    pub(super) fn reclaim_roll_window(&mut self) {
        let Some(mut link) = self.roll.link.take() else {
            self.message = "el piano roll no está en su ventana".into();
            return;
        };
        if let Some(conn) = link.server.client() {
            let _ = conn.send(&ToRoll::GiveBack);
            let _ = conn.drain(Duration::from_millis(200));
        }
        if let Some(mut child) = link.child.take() {
            std::thread::spawn(move || {
                let _ = child.wait();
            });
        }
        drop(link);
        self.roll.visible = true;
        self.roll.area = None;
        self.message = "el piano roll vuelve a esta ventana".into();
    }

    /// Atiende la ventana del piano roll: conexiones, lo que pide y lo que
    /// hay que mandarle. Nunca bloquea.
    fn poll_roll_link(&mut self) {
        let Some(link) = &mut self.roll.link else {
            return;
        };
        let (fresh, rejected) = link.server.accept();
        if let Some(rejected) = rejected {
            self.message = rejected;
        }
        let title = self.file_name();
        let Some(link) = &mut self.roll.link else {
            return;
        };
        if fresh {
            link.sent = None;
            if let Some(conn) = link.server.client() {
                let _ = conn.send(&ToRoll::Welcome {
                    version: link::VERSION,
                    title,
                });
            }
            self.roll.area = None;
            self.message = "piano roll en su ventana".into();
        }
        let Some(link) = &mut self.roll.link else {
            return;
        };
        if !link.server.connected()
            && let Some(child) = &mut link.child
            && let Ok(Some(status)) = child.try_wait()
        {
            let command = std::mem::take(&mut link.command);
            link.child = None;
            self.roll_here(format!(
                "la terminal del piano roll se cerró sin abrirlo ({status}: {command}): aquí"
            ));
            return;
        }
        if link.server.connected() {
            link.child = None;
        }
        let messages = match link.server.client().map(Conn::receive::<FromRoll>) {
            None => return,
            Some(Ok(messages)) => messages,
            Some(Err(_)) => {
                link.server.drop_client();
                self.roll.area = None;
                self.message = "el piano roll se cerró · F1 «Piano roll» o «Abrir el piano roll en su ventana»".into();
                return;
            }
        };
        for message in messages {
            match message {
                FromRoll::Hello { version } if version != link::VERSION => {
                    if let Some(link) = &mut self.roll.link {
                        if let Some(conn) = link.server.client() {
                            let _ = conn.send(&ToRoll::Bye(format!(
                                "protocolo distinto (principal {}, piano roll {version})",
                                link::VERSION
                            )));
                        }
                        link.server.drop_client();
                    }
                    self.message = "un piano roll de otra versión: rechazado".into();
                    return;
                }
                FromRoll::Hello { .. } => {}
                FromRoll::Size { width, height } => {
                    self.roll.area = Some(Rect::new(0, 0, width, height));
                }
                FromRoll::Key(input) => self.roll_window_key(input),
                FromRoll::Mouse { action, x, y } => {
                    if let Some(area) = self.roll.area {
                        self.roll_mouse(action, x, y, area);
                    }
                }
            }
        }
        self.push_roll_view();
    }

    /// Una tecla de la ventana del piano roll: las de la aplicación que
    /// deja pasar (deshacer, cargar, marcha, pánico...) y las suyas.
    fn roll_window_key(&mut self, input: Input) {
        use crate::config::Bound;
        use crate::palette::Action;
        match self.keys.lookup(&input) {
            Some(Bound::Action(a @ (Action::Panic | Action::PanicPause))) => {
                self.run_action(a);
            }
            Some(Bound::Action(a)) if passes_through(a) && a != Action::Quit => {
                self.run_action(a);
            }
            _ => {
                if self.roll.cursor.is_none() {
                    self.open_roll_cursor();
                    if input.key == Key::Esc {
                        return;
                    }
                }
                self.handle_roll(input);
            }
        }
    }

    /// Manda la vista a la ventana del piano roll, si ha cambiado.
    fn push_roll_view(&mut self) {
        if !self.roll_outside() {
            return;
        }
        let Some(view) = self.roll_view() else {
            return;
        };
        let Some(link) = &mut self.roll.link else {
            return;
        };
        if link.sent.as_ref() == Some(&view) {
            return;
        }
        if let Some(conn) = link.server.client() {
            if conn.send(&ToRoll::View(view.clone())).is_ok() {
                link.sent = Some(view);
            } else {
                link.server.drop_client();
            }
        }
    }
}

/// La línea de órdenes que abre el piano roll: la terminal y detrás
/// `tui --window-roll <script>`.
pub fn roll_command(terminal: &[String], exe: &Path, target: &Path) -> Vec<String> {
    let mut args = terminal.to_vec();
    args.push(exe.display().to_string());
    args.push("--window-roll".into());
    args.push(target.display().to_string());
    args
}
