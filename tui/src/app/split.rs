//! Editor en su propia ventana (fase L, parte 1): la misma `App` en dos
//! procesos. La **principal** es dueña del motor, el script cargado, el
//! MIDI y el modo tocar; el **editor**, del texto, el archivo, deshacer,
//! autocompletado, bloques, ranuras y versiones. Se hablan por
//! [`crate::link`]: el editor manda el texto y las acciones que tocan el
//! motor; la principal le devuelve errores, avisos, estado y los cambios
//! que haga ella en el texto. Sin enlace (`tui --edit`), nada de esto
//! existe y todo es como siempre.

use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use ratatui_textarea::{CursorMove, TextArea};
use script::ScriptError;

use super::{App, Flow, Mode};
use crate::highlight::Highlight;
use crate::link::{self, Client, Conn, Handover, RemoteState, Server, ToEditor, ToMain};
use crate::palette::{ACTIONS, Action};

/// Cada cuánto, como mucho, manda el editor su texto mientras se escribe.
const TEXT_SYNC: Duration = Duration::from_millis(100);
/// Pasos del historial de deshacer que se llevan como mucho al cambiar el
/// editor de ventana, y cuánto texto en total (el resto se pierde, como al
/// pasar del máximo de la propia `TextArea`).
const HANDOVER_STEPS: usize = 200;
const HANDOVER_BYTES: usize = 4 << 20;
/// Lo que espera el editor a que salga su estado antes de cerrarse.
const HANDOVER_SEND: Duration = Duration::from_secs(2);

/// De qué lado está esta ventana.
pub enum Link {
    Main(MainLink),
    Editor(EditorLink),
}

/// La ventana principal, con el editor fuera.
pub struct MainLink {
    server: Server,
    /// Última revisión del editor recibida.
    editor_rev: u64,
    /// Último texto que tienen igual los dos lados.
    synced: String,
    /// Lo último enviado (para mandar solo lo que cambia).
    sent_error: Option<Option<String>>,
    sent_message: Option<String>,
    sent_state: Option<RemoteState>,
    sent_labels: Option<Vec<String>>,
    /// Lo último que se mandó de lo que suena, con la revisión del editor.
    sent_sounding: Option<(u64, Vec<Highlight>)>,
    /// Comando que abre una terminal (`[editor] terminal`), el binario
    /// propio y lo que abre el editor (script o proyecto).
    terminal: Option<Vec<String>>,
    exe: PathBuf,
    /// Editor lanzado desde aquí (para recogerlo al salir).
    child: Option<Child>,
    /// Estado del editor de aquí al sacarlo a su ventana: para el primer
    /// editor que se conecte.
    pending: Option<Handover>,
}

/// El editor, con el motor en la ventana principal.
pub struct EditorLink {
    client: Client,
    /// Revisión del último texto enviado y ese texto.
    rev: u64,
    sent: String,
    last_sync: Instant,
    /// Ya se saludó alguna vez con una principal (la primera vez se coge
    /// su texto; después, se le manda el propio).
    welcomed: bool,
    /// El texto al abrir: si sigue igual al conectar por primera vez, se
    /// coge el de la principal.
    opened: String,
    remote: Option<RemoteState>,
    labels: Vec<String>,
    /// Lo que suena según la principal y la revisión del texto a la que se
    /// refiere.
    sounding: (u64, Vec<Highlight>),
    /// Ya mandó su estado a la principal: la ventana se cierra.
    leaving: bool,
}

/// Acciones que se hacen en el editor separado; las demás van a la
/// principal.
fn editor_side(action: Action) -> bool {
    matches!(
        action,
        Action::ToggleVim
            | Action::Undo
            | Action::Redo
            | Action::ToggleComment
            | Action::DuplicateLines
            | Action::MoveLinesUp
            | Action::MoveLinesDown
            | Action::TextStart
            | Action::TextEnd
            | Action::GotoLine
            | Action::Find
            | Action::FindNext
            | Action::FindPrevious
            | Action::Complete
            | Action::SaveAndLoad
            | Action::Load
            | Action::GotoError
            | Action::ReloadExternal
            | Action::Commands
            | Action::SaveBlock
            | Action::InsertBlock
            | Action::UpdateBlock
            | Action::DeleteBlock
            | Action::YankSlot
            | Action::PasteSlot
            | Action::Versions
            | Action::Highlight
            | Action::Quit
    )
}

/// Acciones que en la principal no tienen sentido sin el editor delante
/// (necesitan el cursor o escriben el archivo).
fn needs_the_editor(action: Action) -> bool {
    matches!(
        action,
        Action::ToggleVim
            | Action::Undo
            | Action::Redo
            | Action::ToggleComment
            | Action::DuplicateLines
            | Action::MoveLinesUp
            | Action::MoveLinesDown
            | Action::TextStart
            | Action::TextEnd
            | Action::GotoLine
            | Action::Find
            | Action::FindNext
            | Action::FindPrevious
            | Action::Complete
            | Action::SaveAndLoad
            | Action::ReloadExternal
            | Action::SaveBlock
            | Action::InsertBlock
            | Action::UpdateBlock
            | Action::YankSlot
            | Action::PasteSlot
    )
}

/// Órdenes de F2 que mueven el cursor del editor: se hacen donde esté.
fn moves_the_cursor(command: &str) -> bool {
    matches!(command, "linea" | "línea" | "line" | "buscar" | "find")
}

/// Acciones que abren algo en la ventana principal (hay que ir a ella).
fn opens_in_main(action: Action) -> bool {
    matches!(
        action,
        Action::Browser
            | Action::TapTempo
            | Action::Macros
            | Action::SampleKey
            | Action::SidechainView
            | Action::Freeze
            | Action::Unfreeze
            | Action::MidiLearn
            | Action::MidiKnobs
            | Action::Devices
            | Action::AnalyzeMasking
            | Action::FillGap
            | Action::SuggestFills
            | Action::Record
            | Action::JumpSection
            | Action::Resample
    )
}

/// Nombre de una acción en el protocolo.
fn action_name(action: Action) -> String {
    format!("{action:?}")
}

fn action_named(name: &str) -> Option<Action> {
    ACTIONS
        .iter()
        .map(|d| d.action)
        .find(|a| action_name(*a) == name)
}

/// La línea de órdenes que abre el editor: el comando de la terminal y
/// detrás `tui --window-editor <script>`.
pub fn editor_command(terminal: &[String], exe: &Path, target: &Path) -> Vec<String> {
    let mut args = terminal.to_vec();
    args.push(exe.display().to_string());
    args.push("--window-editor".into());
    args.push(target.display().to_string());
    args
}

impl App {
    /// Abre el editor en su propia ventana (`tui --window-editor`): el
    /// script, o el de un proyecto, sin cargarlo (lo tiene cargado la
    /// principal), y busca la ventana principal de ese script en
    /// `socket_dir`. El `controller` no se usa nunca (su motor no suena).
    pub fn open_editor(
        path: &Path,
        controller: engine::Controller,
        sample_rate: u32,
        socket_dir: &Path,
    ) -> Result<App, Box<dyn std::error::Error>> {
        let is_project = path
            .extension()
            .is_some_and(|e| e == script::project::EXTENSION);
        let mut app = if is_project {
            let project = script::Project::load(path)?;
            let mut app = App::build(&project.script, controller, sample_rate)?;
            app.set_project(path, &project)?;
            app
        } else {
            App::build(path, controller, sample_rate)?
        };
        let socket = link::socket_path(socket_dir, &app.path);
        app.attach(&socket);
        Ok(app)
    }

    /// El script abierto (también con un proyecto).
    pub fn script_path(&self) -> &Path {
        &self.path
    }

    /// Hace de esta la ventana principal con el editor fuera: escucha en
    /// `server` y se queda en modo tocar (el texto se escribe en el
    /// editor).
    pub fn serve(&mut self, server: Server, terminal: Option<Vec<String>>, exe: PathBuf) {
        self.mode = Mode::Play;
        self.link = Some(Link::Main(MainLink {
            server,
            editor_rev: 0,
            synced: self.text(),
            sent_error: None,
            sent_message: None,
            sent_state: None,
            sent_labels: None,
            sent_sounding: None,
            terminal,
            exe,
            child: None,
            pending: None,
        }));
        self.message = "editor en su ventana: F1 «Abrir el editor en su ventana» si no está".into();
    }

    /// Hace de esta el editor de una ventana principal que escucha en
    /// `socket` (si no está, se reintenta cada segundo).
    pub fn attach(&mut self, socket: &Path) {
        self.link = Some(Link::Editor(EditorLink {
            client: Client::new(socket),
            rev: 0,
            sent: self.text(),
            last_sync: Instant::now(),
            welcomed: false,
            opened: self.text(),
            remote: None,
            labels: Vec::new(),
            sounding: (0, Vec::new()),
            leaving: false,
        }));
        self.message = "buscando la ventana principal…".into();
        self.poll_link();
    }

    pub(super) fn is_main_window(&self) -> bool {
        matches!(self.link, Some(Link::Main(_)))
    }

    /// Etiquetas que conoce la principal (en el editor separado).
    pub(super) fn remote_labels(&self) -> &[String] {
        match &self.link {
            Some(Link::Editor(e)) => &e.labels,
            _ => &[],
        }
    }

    /// Lo que tienen que hacer con una acción los dos lados del enlace:
    /// `Some` si ya está (reenviada o rechazada); `None`, como siempre.
    pub(super) fn link_action(&mut self, action: Action) -> Option<Flow> {
        match &mut self.link {
            None => None,
            Some(Link::Main(_)) => {
                if action == Action::PlayMode {
                    self.message = "con el editor en su ventana, aquí solo se toca".into();
                    return Some(Flow::Continue);
                }
                if needs_the_editor(action) {
                    self.message = "eso se hace en la ventana del editor".into();
                    return Some(Flow::Continue);
                }
                None
            }
            Some(Link::Editor(_)) if action == Action::PlayMode => {
                self.message = "el modo tocar está en la ventana principal".into();
                Some(Flow::Continue)
            }
            Some(Link::Editor(_)) if action == Action::EditorBack => Some(self.return_to_main()),
            Some(Link::Editor(_)) if action == Action::LabelSlice => {
                let slice = self.remote_state().map_or(1, |s| s.slice.max(1));
                self.prompt = Some(format!("etiquetar {slice} "));
                self.message =
                    "escribe la etiqueta del corte (se aplica en la ventana principal)".into();
                Some(Flow::Continue)
            }
            Some(Link::Editor(_)) if editor_side(action) => None,
            Some(Link::Editor(editor)) => {
                let sent = editor
                    .client
                    .conn()
                    .map(|c| c.send(&ToMain::Action(action_name(action))));
                self.message = match sent {
                    Some(Ok(())) if opens_in_main(action) => {
                        "se abre en la ventana principal".into()
                    }
                    Some(Ok(())) => String::new(),
                    Some(Err(_)) => {
                        editor.client.disconnect();
                        "se perdió la ventana principal".into()
                    }
                    None => "sin ventana principal: eso se hace allí".into(),
                };
                Some(Flow::Continue)
            }
        }
    }

    /// Una orden de F2: en el editor separado, solo `bloque`, `linea` y
    /// `buscar` se hacen aquí; las demás van a la principal. En la
    /// principal, `abrir` no (el editor seguiría con el script de antes),
    /// ni `linea` ni `buscar` (no hay cursor que mover).
    pub(super) fn link_command(&mut self, line: &str) -> bool {
        let command = line.split_whitespace().next().unwrap_or_default();
        match &mut self.link {
            Some(Link::Main(_)) if command.starts_with("abrir") || command.starts_with("open") => {
                self.message =
                    "con el editor en su ventana no se abre otro proyecto: cierra las dos y ábrelo"
                        .into();
                true
            }
            Some(Link::Main(_)) if moves_the_cursor(command) => {
                self.message = "eso se hace en la ventana del editor".into();
                true
            }
            Some(Link::Editor(editor))
                if !command.starts_with("bloque") && !moves_the_cursor(command) =>
            {
                let sent = editor
                    .client
                    .conn()
                    .map(|c| c.send(&ToMain::Command(line.to_string())));
                self.message = match sent {
                    Some(Ok(())) => String::new(),
                    Some(Err(_)) => {
                        editor.client.disconnect();
                        "se perdió la ventana principal".into()
                    }
                    None => "sin ventana principal: las órdenes se hacen allí".into(),
                };
                true
            }
            _ => false,
        }
    }

    /// Cargar en el editor separado: el texto va a la principal, que lo
    /// carga como siempre (entra al compás siguiente).
    pub(super) fn link_load(&mut self) -> bool {
        let text = self.text();
        let Some(Link::Editor(editor)) = &mut self.link else {
            return false;
        };
        // Al cargar se busca la principal aunque no toque reintentar (el
        // saludo llega en la siguiente vuelta; la carga va detrás).
        editor.client.connect(true);
        editor.rev += 1;
        let rev = editor.rev;
        let sent = editor.client.conn().map(|c| {
            c.send(&ToMain::Load {
                rev,
                text: text.clone(),
            })
        });
        self.message = match sent {
            Some(Ok(())) => {
                editor.sent = text;
                editor.last_sync = Instant::now();
                "cargando en la ventana principal…".into()
            }
            Some(Err(_)) => {
                editor.client.disconnect();
                "se perdió la ventana principal: no se cargó".into()
            }
            None => "sin ventana principal: no se cargó (arranca tui --edit … --split)".into(),
        };
        true
    }

    /// En la principal: el texto (y la revisión) que manda el editor.
    fn take_editor_text(&mut self, rev: u64, text: String) {
        if let Some(Link::Main(main)) = &mut self.link {
            main.editor_rev = rev;
            main.synced = text.clone();
        }
        if text != self.text() {
            let row = self.editor.cursor().0;
            self.editor = text_area(&text);
            self.editor
                .move_cursor(CursorMove::Jump(row.min(u16::MAX as usize) as u16, 0));
        }
    }

    /// Salir de la principal: con el editor conectado no se pierde nada
    /// (el texto está allí); sin él, avisa si lo que hay aquí no está en el
    /// disco (lo último del editor que se cerró, o un cambio de aquí).
    pub(super) fn quit_main(&mut self, armed: bool) -> Flow {
        let Some(Link::Main(main)) = &self.link else {
            return Flow::Quit;
        };
        let on_disk = std::fs::read_to_string(&self.path).is_ok_and(|d| d == self.text());
        if main.server.connected() || armed || on_disk {
            return Flow::Quit;
        }
        self.quit_armed = true;
        self.message =
            "sin editor, el texto de aquí no está guardado: Ctrl-Q otra vez para salir (o abre \
             el editor y guárdalo)"
                .into();
        Flow::Continue
    }

    /// En la principal: abre el editor en una terminal nueva con el
    /// comando de `[editor] terminal`.
    pub fn launch_editor(&mut self) {
        let target = self.project.clone().unwrap_or_else(|| self.path.clone());
        if self.link.is_none() {
            self.detach_editor();
            return;
        }
        let Some(Link::Main(main)) = &mut self.link else {
            return;
        };
        if main.server.connected() {
            self.message = "el editor ya está abierto en su ventana".into();
            return;
        }
        let Some(terminal) = main.terminal.clone().filter(|t| !t.is_empty()) else {
            self.message = format!(
                "sin [editor] terminal en config.toml: ábrelo a mano con tui --window-editor {}",
                target.display()
            );
            return;
        };
        let args = editor_command(&terminal, &main.exe, &target);
        let spawned = Command::new(&args[0])
            .args(&args[1..])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn();
        self.message = match spawned {
            Ok(child) => {
                main.child = Some(child);
                format!("abriendo el editor: {}", args.join(" "))
            }
            Err(e) => format!("no se pudo abrir «{}»: {e}", args[0]),
        };
    }

    /// Estado de la principal (en el editor separado).
    fn remote_state(&self) -> Option<&RemoteState> {
        match &self.link {
            Some(Link::Editor(e)) => e.remote.as_ref(),
            _ => None,
        }
    }

    /// Atiende el enlace: lo que llega y lo que hay que mandar. Se llama
    /// en cada vuelta del bucle; nunca bloquea.
    pub fn poll_link(&mut self) {
        match &self.link {
            Some(Link::Main(_)) => self.poll_main(),
            Some(Link::Editor(_)) => self.poll_editor(),
            None => {}
        }
    }

    fn poll_main(&mut self) {
        let Some(Link::Main(main)) = &mut self.link else {
            return;
        };
        if let Some(child) = &mut main.child
            && !matches!(child.try_wait(), Ok(None))
        {
            main.child = None;
        }
        let (fresh, rejected) = main.server.accept();
        if fresh {
            let welcome = self.welcome();
            let self_text = self.text();
            let Some(Link::Main(main)) = &mut self.link else {
                return;
            };
            main.editor_rev = 0;
            main.synced = self_text;
            main.sent_error = None;
            main.sent_message = None;
            main.sent_state = None;
            main.sent_labels = None;
            main.sent_sounding = None;
            let pending = main.pending.take();
            if let Some(conn) = main.server.client() {
                let _ = conn.send(&welcome);
                if let Some(handover) = pending {
                    let _ = conn.send(&ToEditor::History(handover));
                }
            }
            // (El aviso es para esta ventana: el editor ya dice lo suyo.)
            self.message = "editor conectado".into();
            main.sent_message = Some(self.message.clone());
        }
        if let Some(rejected) = rejected {
            self.message = rejected;
        }
        let Some(Link::Main(main)) = &mut self.link else {
            return;
        };
        let messages = match main.server.client().map(Conn::receive::<ToMain>) {
            None => return,
            Some(Ok(messages)) => messages,
            Some(Err(_)) => {
                main.server.drop_client();
                self.message =
                    "el editor se cerró: sigue sonando · F1 «Abrir el editor en su ventana»".into();
                return;
            }
        };
        let mut loaded = false;
        for message in messages {
            match message {
                ToMain::Hello { version } if version != link::VERSION => {
                    if let Some(Link::Main(main)) = &mut self.link {
                        if let Some(conn) = main.server.client() {
                            let _ = conn.send(&ToEditor::Bye(format!(
                                "protocolo distinto (principal {}, editor {version})",
                                link::VERSION
                            )));
                        }
                        main.server.drop_client();
                    }
                    self.message = "un editor de otra versión: rechazado".into();
                    return;
                }
                ToMain::Hello { .. } => {}
                ToMain::Text { rev, text } => self.take_editor_text(rev, text),
                ToMain::Load { rev, text } => {
                    self.take_editor_text(rev, text);
                    self.load();
                    loaded = true;
                }
                // (Salir no llega: lo decide cada ventana.)
                ToMain::Action(name) => match action_named(&name) {
                    Some(action) => {
                        self.run_action(action);
                    }
                    None => self.message = format!("acción desconocida del editor: {name}"),
                },
                ToMain::Command(line) => self.run_command(&line),
                ToMain::Return(handover) => {
                    self.take_editor_back(handover);
                    return;
                }
            }
        }
        if loaded && let Some(Link::Main(main)) = &mut self.link {
            // Tras cada carga, el resultado aunque no haya cambiado.
            main.sent_error = None;
            main.sent_message = None;
        }
        self.push_to_editor();
    }

    /// El saludo para un editor que se conecta.
    fn welcome(&self) -> ToEditor {
        ToEditor::Welcome {
            version: link::VERSION,
            path: self.path.clone(),
            samples: self.sample_folder(),
            rev: 0,
            text: self.text(),
        }
    }

    /// Manda al editor lo que haya cambiado aquí.
    fn push_to_editor(&mut self) {
        let text = self.text();
        let error = self.error.as_ref().map(|e| e.message.clone());
        let message = self.message.clone();
        let labels = self.known_labels();
        let sounding = self.highlights();
        let state = self.status.map(|s| RemoteState {
            running: s.running,
            bpm: s.bpm,
            bar: s.bar,
            step: s.step,
            track: self
                .tracks()
                .get(self.selected)
                .map_or(String::new(), |t| t.name.clone()),
            slice: self.selected_slice + 1,
            cue: self.cue.map_or(String::new(), |bar| self.cue_text(bar)),
        });
        let Some(Link::Main(main)) = &mut self.link else {
            return;
        };
        let Some(conn) = main.server.client() else {
            return;
        };
        let mut out = Vec::new();
        if main.sent_error.as_ref() != Some(&error) {
            out.push(ToEditor::Error(error.clone()));
            main.sent_error = Some(error);
        }
        if main.sent_message.as_ref() != Some(&message) {
            out.push(ToEditor::Notice(message.clone()));
            main.sent_message = Some(message);
        }
        if let Some(state) = state
            && main.sent_state.as_ref() != Some(&state)
        {
            out.push(ToEditor::State(state.clone()));
            main.sent_state = Some(state);
        }
        if main.sent_labels.as_ref() != Some(&labels) {
            out.push(ToEditor::Labels(labels.clone()));
            main.sent_labels = Some(labels);
        }
        // Lo que suena, sobre el texto de la última revisión del editor (la
        // que tiene aquí).
        let sounding = (main.editor_rev, sounding);
        if main.sent_sounding.as_ref() != Some(&sounding) {
            out.push(ToEditor::Sounding {
                base: sounding.0,
                marks: sounding.1.clone(),
            });
            main.sent_sounding = Some(sounding);
        }
        // El texto, lo último: si choca con lo que se escribe allí, su aviso
        // no lo tapa el mensaje de aquí.
        if text != main.synced {
            out.push(ToEditor::Text {
                base: main.editor_rev,
                text: text.clone(),
            });
            main.synced = text;
        }
        for message in &out {
            if conn.send(message).is_err() {
                main.server.drop_client();
                self.message = "el editor se cerró: sigue sonando".into();
                return;
            }
        }
        if out.is_empty() && conn.flush().is_err() {
            main.server.drop_client();
        }
    }

    fn poll_editor(&mut self) {
        let text = self.text();
        let Some(Link::Editor(editor)) = &mut self.link else {
            return;
        };
        editor.client.connect(false);
        let messages = match editor.client.conn().map(Conn::receive::<ToEditor>) {
            None => return,
            Some(Ok(messages)) => messages,
            Some(Err(_)) => {
                editor.client.disconnect();
                editor.remote = None;
                editor.sounding.1.clear();
                self.message =
                    "se perdió la ventana principal: sigue editando; se reconecta sola".into();
                return;
            }
        };
        for message in messages {
            self.editor_message(message);
        }
        // El texto, mientras se escribe (para que lo que cambie la
        // principal parta de él).
        let Some(Link::Editor(editor)) = &mut self.link else {
            return;
        };
        if text != editor.sent && editor.last_sync.elapsed() >= TEXT_SYNC {
            editor.rev += 1;
            let rev = editor.rev;
            let sent = editor.client.conn().map(|c| {
                c.send(&ToMain::Text {
                    rev,
                    text: text.clone(),
                })
            });
            match sent {
                Some(Ok(())) => {
                    editor.sent = text;
                    editor.last_sync = Instant::now();
                }
                Some(Err(_)) => editor.client.disconnect(),
                None => {}
            }
        } else if let Some(conn) = editor.client.conn()
            && conn.flush().is_err()
        {
            editor.client.disconnect();
        }
    }

    fn editor_message(&mut self, message: ToEditor) {
        let text = self.text();
        let Some(Link::Editor(editor)) = &mut self.link else {
            return;
        };
        match message {
            ToEditor::Welcome {
                version,
                path,
                samples,
                rev,
                text: theirs,
            } => {
                if version != link::VERSION {
                    editor.client.disconnect();
                    editor.client.stopped = true;
                    self.message = format!(
                        "la ventana principal habla otro protocolo ({version}, este {})",
                        link::VERSION
                    );
                    return;
                }
                editor.rev = rev;
                let first = !std::mem::replace(&mut editor.welcomed, true);
                self.host.set_sample_dir(Some(samples));
                if first && text == editor.opened {
                    // Recién abierto: el texto es el de la principal (puede
                    // tener cambios sin guardar).
                    editor.sent = theirs.clone();
                    if theirs != text {
                        self.editor = text_area(&theirs);
                    }
                    self.message =
                        format!("conectado con la ventana principal ({})", path.display());
                } else {
                    // Reconectado: el texto es el de aquí; se le manda.
                    editor.sent = theirs.clone();
                    editor.last_sync = Instant::now()
                        .checked_sub(TEXT_SYNC)
                        .unwrap_or_else(Instant::now);
                    self.message = if theirs == text {
                        "reconectado con la ventana principal".into()
                    } else {
                        "reconectado: la principal tiene cargado otro texto; F5 carga el tuyo"
                            .into()
                    };
                }
            }
            ToEditor::Text { base, text: theirs } => {
                if base == editor.rev && text == editor.sent {
                    editor.sent = theirs.clone();
                    self.replace_text(&theirs);
                } else {
                    self.message = "la ventana principal cambió el texto mientras escribías: \
                                    se queda el tuyo (rehaz allí el cambio)"
                        .into();
                    // Se le manda el de aquí en la siguiente vuelta.
                    editor.sent.clear();
                }
            }
            ToEditor::Error(error) => {
                self.error = error.map(|message| ScriptError { message });
            }
            ToEditor::Notice(text) => {
                if !text.is_empty() {
                    self.message = text;
                }
            }
            ToEditor::State(state) => editor.remote = Some(state),
            ToEditor::Sounding { base, marks } => editor.sounding = (base, marks),
            ToEditor::Labels(labels) => editor.labels = labels,
            ToEditor::Bye(reason) => {
                editor.client.disconnect();
                editor.client.stopped = true;
                editor.remote = None;
                editor.sounding.1.clear();
                self.message = format!("la ventana principal cerró la conexión: {reason}");
            }
            ToEditor::History(handover) => {
                // Solo si el texto es el que tenía la principal (recién
                // abierto con su texto).
                if handover.text == text {
                    self.take_handover(handover);
                    self.message =
                        "el editor, en su ventana: con su cursor y su historial de deshacer".into();
                }
            }
            ToEditor::GiveBack => {
                self.return_to_main();
            }
        }
    }

    /// Pone otro texto en el editor; se deshace de una vez, como cualquier
    /// acción compuesta. El cursor se queda en su línea.
    pub(super) fn replace_text(&mut self, text: &str) {
        let cursor = self.editor.cursor();
        let (row, col) = (cursor.0, cursor.1);
        self.grouped(|e| {
            let yank = e.yank_text();
            e.select_all();
            e.cut();
            e.insert_str(text.strip_suffix('\n').unwrap_or(text));
            e.set_yank_text(yank);
            e.move_cursor(CursorMove::Jump(
                row.min(u16::MAX as usize) as u16,
                col.min(u16::MAX as usize) as u16,
            ));
        });
    }

    /// El estado del editor para llevarlo a otra ventana: el texto, el
    /// cursor y lo que deshacen y rehacen Ctrl-Z y Ctrl-Y (sacado de una
    /// copia, deshaciendo y rehaciendo como lo haría el editor).
    pub(super) fn handover(&self) -> Handover {
        let text_of = |e: &TextArea| {
            let mut text = e.lines().join("\n");
            text.push('\n');
            text
        };
        let walk = |forward: bool| {
            let mut editor = self.editor.clone();
            let (mut undo, mut redo) = (self.undo_groups.clone(), self.redo_groups.clone());
            let (mut texts, mut bytes) = (Vec::new(), 0);
            while texts.len() < HANDOVER_STEPS && bytes < HANDOVER_BYTES {
                let moved = if forward {
                    super::redo_in(&mut editor, &mut undo, &mut redo)
                } else {
                    super::undo_in(&mut editor, &mut undo, &mut redo)
                };
                if !moved {
                    break;
                }
                let text = text_of(&editor);
                bytes += text.len();
                texts.push(text);
            }
            texts
        };
        let mut undo = walk(false);
        undo.reverse();
        let cursor = self.editor.cursor();
        Handover {
            text: text_of(&self.editor),
            cursor: (cursor.0, cursor.1),
            undo,
            redo: walk(true),
        }
    }

    /// Se queda con el estado de un editor de otra ventana: rehace su
    /// historial paso a paso (cada uno, un cambio que Ctrl-Z deshace de una
    /// vez) desde el más antiguo, deshace los que eran para rehacer y pone
    /// el cursor donde estaba. El portapapeles interno se queda.
    pub(super) fn take_handover(&mut self, handover: Handover) {
        let lines = |text: &str| -> Vec<String> {
            text.strip_suffix('\n')
                .unwrap_or(text)
                .split('\n')
                .map(String::from)
                .collect()
        };
        let yank = self.editor.yank_text();
        let first = handover.undo.first().unwrap_or(&handover.text);
        self.editor = TextArea::new(lines(first));
        self.undo_groups.clear();
        self.redo_groups.clear();
        let steps = handover
            .undo
            .iter()
            .skip(1)
            .chain([&handover.text])
            .chain(&handover.redo);
        for text in steps {
            let target = lines(text);
            self.grouped(|e| change_lines(e, &target));
        }
        for _ in &handover.redo {
            self.undo();
        }
        self.editor.set_yank_text(yank);
        let (row, col) = handover.cursor;
        self.editor.move_cursor(CursorMove::Jump(
            row.min(u16::MAX as usize) as u16,
            col.min(u16::MAX as usize) as u16,
        ));
        self.completion = None;
        self.marks.clear();
    }

    /// En una sola ventana: saca el editor a la suya. Esta pasa a ser la
    /// principal (escucha en el socket del script y se queda en el modo
    /// tocar) y abre el editor como `--split`; el primer editor que se
    /// conecte se lleva el texto, el cursor y el historial.
    fn detach_editor(&mut self) {
        let dir = match self.socket_dir.clone() {
            Some(dir) => Ok(dir),
            None => link::runtime_dir(),
        };
        let server = dir.and_then(|dir| Server::bind(&link::socket_path(&dir, &self.path)));
        let server = match server {
            Ok(server) => server,
            Err(e) => {
                self.message = format!("no se pudo sacar el editor: {e}");
                return;
            }
        };
        let handover = self.handover();
        let exe = std::env::current_exe().unwrap_or_else(|_| "tui".into());
        self.serve(server, self.editor_terminal.clone(), exe);
        if let Some(Link::Main(main)) = &mut self.link {
            main.pending = Some(handover);
        }
        self.launch_editor();
    }

    /// Carpeta de los sockets (para los tests; si no, la de siempre).
    #[cfg(test)]
    pub fn set_socket_dir(&mut self, dir: PathBuf) {
        self.socket_dir = Some(dir);
    }

    /// «Volver a meter el editor en la principal» en la principal: se lo
    /// pide al editor conectado (que contesta con su estado y se cierra);
    /// sin editor, vuelve con el texto que tiene.
    pub(super) fn reclaim_editor(&mut self) {
        let Some(Link::Main(main)) = &mut self.link else {
            self.message = "el editor ya está en esta ventana".into();
            return;
        };
        if let Some(conn) = main.server.client() {
            if conn.send(&ToEditor::GiveBack).is_ok() {
                self.message = "pidiendo el editor a su ventana…".into();
                return;
            }
            main.server.drop_client();
        }
        let handover = Handover {
            text: self.text(),
            cursor: (self.editor.cursor().0, self.editor.cursor().1),
            ..Handover::default()
        };
        self.take_editor_back(handover);
    }

    /// En el editor separado: manda su estado a la principal y se cierra.
    /// Si no hay principal o no le llega, se queda.
    fn return_to_main(&mut self) -> Flow {
        let handover = self.handover();
        let Some(Link::Editor(editor)) = &mut self.link else {
            return Flow::Continue;
        };
        let sent = editor.client.conn().map(|c| {
            c.send(&ToMain::Return(handover))
                .and_then(|()| c.drain(HANDOVER_SEND))
        });
        match sent {
            Some(Ok(())) => {
                editor.leaving = true;
                Flow::Quit
            }
            Some(Err(e)) => {
                editor.client.disconnect();
                self.message = format!("no se pudo volver a la principal: {e}");
                Flow::Continue
            }
            None => {
                self.message = "sin ventana principal: no hay a dónde volver".into();
                Flow::Continue
            }
        }
    }

    /// El editor separado ya volvió a la principal: esta ventana se cierra.
    pub fn leaving(&self) -> bool {
        matches!(&self.link, Some(Link::Editor(e)) if e.leaving)
    }

    /// En la principal: el editor vuelve a esta ventana. Se deja de
    /// escuchar (el socket se borra), se coge su estado y se vuelve al
    /// layout de una sola ventana, en el editor. Lo guardado es lo que haya
    /// en el disco (lo guardó el editor).
    fn take_editor_back(&mut self, handover: Handover) {
        if let Some(Link::Main(main)) = self.link.take()
            && let Some(mut child) = main.child
        {
            // La terminal del editor se cierra sola; se recoge aparte para
            // no esperar aquí.
            std::thread::spawn(move || {
                let _ = child.wait();
            });
        }
        self.take_handover(handover);
        self.saved = std::fs::read_to_string(&self.path).unwrap_or_default();
        self.external = None;
        self.mode = Mode::Edit;
        self.message = "el editor vuelve a estar en esta ventana".into();
    }

    /// En el editor separado, lo que suena según la principal: solo si el
    /// texto de aquí es el de la revisión a la que se refiere (si se ha
    /// escrito algo desde entonces, nada). `None` fuera del editor
    /// separado (lo calcula esta ventana).
    pub(super) fn remote_highlights(&self) -> Option<Vec<Highlight>> {
        let Some(Link::Editor(editor)) = &self.link else {
            return None;
        };
        let (base, marks) = &editor.sounding;
        let current =
            editor.client.connected() && *base == editor.rev && self.text() == editor.sent;
        Some(if current { marks.clone() } else { Vec::new() })
    }

    /// La línea del enlace para las vistas, y el transporte del editor
    /// separado (el de la principal).
    pub(super) fn link_view(&self, view: &mut crate::model::ViewModel) {
        match &self.link {
            None => {}
            Some(Link::Main(main)) => {
                view.link = Some(if main.server.connected() {
                    "⇄ editor en su ventana".into()
                } else {
                    "editor fuera, sin conectar · F1 «Abrir el editor en su ventana»".into()
                });
                view.dirty = self.text() != main.synced;
                view.editor_outside = true;
                // Siempre en el modo tocar, y Esc no lleva a ningún editor.
                view.key_help = self.keys.help(true, false);
            }
            Some(Link::Editor(editor)) => {
                view.link = Some(match (&editor.remote, editor.client.connected()) {
                    (_, false) if editor.client.stopped => "✗ sin ventana principal".into(),
                    (_, false) => "✗ sin ventana principal (se reintenta)".into(),
                    (None, true) => "⇄ principal".into(),
                    (Some(s), true) => {
                        // Como el panel del motor de la principal.
                        let step = s.step.map_or("--".into(), |p| format!("{p:02}"));
                        let play = if s.running { "▶" } else { "■" };
                        let track = if s.track.is_empty() {
                            String::new()
                        } else {
                            format!(" · {} corte {}", s.track, s.slice)
                        };
                        let cue = if s.cue.is_empty() {
                            String::new()
                        } else {
                            format!(" · ⏮ cue {}", s.cue)
                        };
                        format!(
                            "⇄ principal {play} {:.0} BPM · compás {} paso {step}{track}{cue}",
                            s.bpm, s.bar
                        )
                    }
                });
                if let Some(s) = &editor.remote {
                    view.transport = crate::model::TransportView {
                        running: s.running,
                        bpm: s.bpm,
                        bar: s.bar,
                        step: s.step,
                    };
                    view.cue = (!s.cue.is_empty()).then(|| s.cue.clone());
                }
            }
        }
    }
}

/// Cambia el texto de `editor` por `target` tocando solo lo que cambia
/// (lo de en medio entre lo común del principio y del final), para que el
/// cursor quede ahí al deshacerlo.
fn change_lines(editor: &mut TextArea<'static>, target: &[String]) {
    let before: Vec<char> = editor.lines().join("\n").chars().collect();
    let after: Vec<char> = target.join("\n").chars().collect();
    let prefix = before
        .iter()
        .zip(&after)
        .take_while(|(a, b)| a == b)
        .count();
    let room = before.len().min(after.len()) - prefix;
    let suffix = before
        .iter()
        .rev()
        .zip(after.iter().rev())
        .take(room)
        .take_while(|(a, b)| a == b)
        .count();
    // Fila y columna del primer carácter distinto.
    let head = &before[..prefix];
    let row = head.iter().filter(|&&c| c == '\n').count();
    let col = head.iter().rev().take_while(|&&c| c != '\n').count();
    editor.move_cursor(CursorMove::Jump(
        row.min(u16::MAX as usize) as u16,
        col.min(u16::MAX as usize) as u16,
    ));
    editor.delete_str(before.len() - prefix - suffix);
    let middle: String = after[prefix..after.len() - suffix].iter().collect();
    editor.insert_str(middle);
}

fn text_area(text: &str) -> TextArea<'static> {
    TextArea::new(text.lines().map(String::from).collect())
}
