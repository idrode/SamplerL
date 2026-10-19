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
use crate::link::{self, Client, Conn, RemoteState, Server, ToEditor, ToMain};
use crate::palette::{ACTIONS, Action};

/// Cada cuánto, como mucho, manda el editor su texto mientras se escribe.
const TEXT_SYNC: Duration = Duration::from_millis(100);

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
    /// Comando que abre una terminal (`[editor] terminal`), el binario
    /// propio y lo que abre el editor (script o proyecto).
    terminal: Option<Vec<String>>,
    exe: PathBuf,
    /// Editor lanzado desde aquí (para recogerlo al salir).
    child: Option<Child>,
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
}

/// Acciones que se hacen en el editor separado; las demás van a la
/// principal.
fn editor_side(action: Action) -> bool {
    matches!(
        action,
        Action::Undo
            | Action::Redo
            | Action::ToggleComment
            | Action::DuplicateLines
            | Action::MoveLinesUp
            | Action::MoveLinesDown
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
            | Action::Quit
    )
}

/// Acciones que en la principal no tienen sentido sin el editor delante
/// (necesitan el cursor o escriben el archivo).
fn needs_the_editor(action: Action) -> bool {
    matches!(
        action,
        Action::Undo
            | Action::Redo
            | Action::ToggleComment
            | Action::DuplicateLines
            | Action::MoveLinesUp
            | Action::MoveLinesDown
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

/// Acciones que abren algo en la ventana principal (hay que ir a ella).
fn opens_in_main(action: Action) -> bool {
    matches!(
        action,
        Action::Browser
            | Action::TapTempo
            | Action::AnalyzeMasking
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
            terminal,
            exe,
            child: None,
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

    /// Una orden de F2: en el editor separado, solo `bloque` se hace aquí;
    /// las demás van a la principal. En la principal, `abrir` no (el
    /// editor seguiría con el script de antes).
    pub(super) fn link_command(&mut self, line: &str) -> bool {
        let command = line.split_whitespace().next().unwrap_or_default();
        match &mut self.link {
            Some(Link::Main(_)) if command.starts_with("abrir") || command.starts_with("open") => {
                self.message =
                    "con el editor en su ventana no se abre otro proyecto: cierra las dos y ábrelo"
                        .into();
                true
            }
            Some(Link::Editor(editor)) if !command.starts_with("bloque") => {
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
        let Some(Link::Main(main)) = &mut self.link else {
            self.message =
                "para tener el editor en su ventana, arranca con tui --edit … --split".into();
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
            if let Some(conn) = main.server.client() {
                let _ = conn.send(&welcome);
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
            ToEditor::Labels(labels) => editor.labels = labels,
            ToEditor::Bye(reason) => {
                editor.client.disconnect();
                editor.client.stopped = true;
                editor.remote = None;
                self.message = format!("la ventana principal cerró la conexión: {reason}");
            }
        }
    }

    /// Pone otro texto en el editor; se deshace de una vez, como cualquier
    /// acción compuesta. El cursor se queda en su línea.
    fn replace_text(&mut self, text: &str) {
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
                        format!(
                            "⇄ principal {play} {:.0} BPM · compás {} paso {step}{track}",
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
                }
            }
        }
    }
}

fn text_area(text: &str) -> TextArea<'static> {
    TextArea::new(text.lines().map(String::from).collect())
}
