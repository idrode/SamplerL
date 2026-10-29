//! Los dispositivos desde la aplicación: lo que hace `crate::devices` se
//! aplica aquí (la cola nueva de la entrada MIDI, el destino nuevo de las
//! notas y los avisos en la línea de estado), el panel encima de la
//! ventana principal y el panel en su propia ventana (`--window-devices`,
//! por un socket aparte del del editor; ver `crate::link`).

use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::Instant;

use ratatui_textarea::Input;

use super::{App, Flow};
use crate::devices::{Change, Devices, DevicesView, Kind};
use crate::devices_panel::{self, Cursor, PanelKey};
use crate::link::{self, Conn, FromDevices, Message, Server, ToDevices};
use crate::model::DevicesPopup;

/// El panel de dispositivos en su ventana, visto desde la principal.
pub struct DevicesLink {
    server: Server,
    /// Lo último que se le mandó (para mandar solo lo que cambia).
    sent: Option<DevicesView>,
    /// La terminal lanzada desde aquí, mientras no se conecte.
    child: Option<Child>,
    /// El comando con que se lanzó (para el mensaje si falla).
    command: String,
}

/// La línea de órdenes que abre el panel: el comando de la terminal y
/// detrás `tui --window-devices <script>`.
pub fn devices_command(terminal: &[String], exe: &Path, target: &Path) -> Vec<String> {
    let mut args = terminal.to_vec();
    args.push(exe.display().to_string());
    args.push("--window-devices".into());
    args.push(target.display().to_string());
    args
}

impl App {
    /// Los dispositivos de esta ventana principal, con lo que haya que
    /// aplicar al abrirlos (las colas del MIDI y los avisos).
    pub fn set_devices(&mut self, devices: Devices, changes: Vec<Change>) {
        self.devices = Some(devices);
        self.apply_device_changes(changes);
    }

    /// Los avisos al arrancar (los de la configuración y los de los
    /// dispositivos), todos en la línea de estado.
    pub fn set_startup_warnings(&mut self, warnings: &[String]) {
        let parts: Vec<String> = warnings.iter().map(|w| format!("⚠ {w}")).collect();
        self.message = parts.join(" · ");
    }

    fn apply_device_changes(&mut self, changes: Vec<Change>) {
        for change in changes {
            match change {
                Change::Message(text) => self.message = text,
                Change::Input(queue) => self.midi_in = Some(queue),
                Change::Output(sink) => self.midi_out = Some(sink),
            }
        }
    }

    /// Dónde guardar lo que se elija (`config.toml`).
    pub fn set_devices_config(&mut self, path: Option<PathBuf>) {
        self.devices_config = path;
    }

    /// Elige el dispositivo `name` de la lista `kind`. Si cambia lo
    /// elegido, se guarda en `[dispositivos]` (solo lo que elige el
    /// usuario: volver a otro porque el elegido se fue no se guarda, para
    /// que al enchufarlo otra vez vuelva a ser el bueno).
    pub fn choose_device(&mut self, kind: Kind, name: &str) {
        let Some(devices) = &mut self.devices else {
            self.message = "esta ventana no tiene dispositivos".into();
            return;
        };
        let before = devices.choice().get(kind).map(String::from);
        let changes = devices.choose(kind, name);
        let after = devices.choice().get(kind).map(String::from);
        self.apply_device_changes(changes);
        if before != after {
            self.save_device(kind, after);
        }
    }

    /// Guarda en `config.toml` lo elegido en la lista `kind` (solo esa: lo
    /// que se pidió por la línea de órdenes para esta vez no se guarda).
    /// Si no se puede, lo dice.
    fn save_device(&mut self, kind: Kind, value: Option<String>) {
        let Some(path) = &self.devices_config else {
            return;
        };
        let mut saved = crate::config::Config::load(path, false).devices;
        saved.set(kind, value);
        if let Err(e) = crate::config::save_devices(path, &saved) {
            self.message = format!(
                "{} (no se pudo guardar en {}: {e})",
                self.message,
                path.display()
            );
        }
    }

    /// Vuelve a leer las listas de dispositivos.
    pub fn refresh_devices(&mut self) {
        let Some(devices) = &mut self.devices else {
            self.message = "esta ventana no tiene dispositivos".into();
            return;
        };
        let changes = devices.refresh();
        self.apply_device_changes(changes);
    }

    /// En cada vuelta: termina un cambio de audio y se recupera si un
    /// dispositivo se fue.
    pub(super) fn poll_devices(&mut self) {
        let failed = self.midi_out.as_ref().is_some_and(|s| s.failed());
        let Some(devices) = &mut self.devices else {
            return;
        };
        let changes = devices.poll(failed);
        let activity = devices.activity();
        self.devices_blink.observe(activity, Instant::now());
        self.apply_device_changes(changes);
        self.poll_devices_link();
    }

    /// Lo que se ve en el panel de dispositivos.
    #[cfg(test)]
    pub fn devices_view(&self) -> Option<DevicesView> {
        self.devices.as_ref().map(Devices::view)
    }

    /// Lo elegido ahora.
    #[cfg(test)]
    pub fn device_choice(&self) -> Option<crate::devices::Choice> {
        self.devices.as_ref().map(|d| d.choice().clone())
    }
}

impl App {
    /// «Dispositivos»: el panel encima de esta ventana (si está en la suya,
    /// lo dice).
    pub fn open_devices_panel(&mut self) {
        let Some(devices) = &self.devices else {
            self.message = "los dispositivos están en la ventana principal: ábrelo allí".into();
            return;
        };
        if self
            .devices_link
            .as_mut()
            .is_some_and(|l| l.server.connected())
        {
            self.message = "el panel de dispositivos está en su ventana \
                            (F1 «Volver a meter el panel de dispositivos en la principal»)"
                .into();
            return;
        }
        self.devices_panel = Some(Cursor::on_active(&devices.view(), Kind::Audio));
        self.message =
            "dispositivos: ↑↓ elegir · ←→/Tab lista · Enter usar · r refrescar · Esc".into();
    }

    /// Una tecla con el panel de dispositivos abierto aquí.
    pub(super) fn handle_devices_panel(&mut self, input: Input) -> Flow {
        let (Some(devices), Some(cursor)) = (&self.devices, &mut self.devices_panel) else {
            self.devices_panel = None;
            return Flow::Continue;
        };
        match devices_panel::key(&devices.view(), cursor, &input) {
            PanelKey::Choose(kind, name) => self.choose_device(kind, &name),
            PanelKey::Refresh => self.refresh_devices(),
            PanelKey::Close => {
                self.devices_panel = None;
                self.message.clear();
            }
            PanelKey::Moved => {}
        }
        if let (Some(devices), Some(cursor)) = (&self.devices, &mut self.devices_panel) {
            cursor.clamp(&devices.view());
        }
        Flow::Continue
    }

    /// El panel para el modelo de vista (si está abierto aquí).
    pub(super) fn devices_popup(&self) -> Option<DevicesPopup> {
        let cursor = self.devices_panel?;
        Some(DevicesPopup {
            view: self.with_knobs(self.devices.as_ref()?.view()),
            cursor,
            lit: self.devices_blink.lit(Instant::now()),
        })
    }

    /// La vista con la línea de los mandos MIDI.
    fn with_knobs(&self, mut view: DevicesView) -> DevicesView {
        view.knobs = self.knobs_line();
        view
    }

    /// «Abrir el panel de dispositivos en su ventana»: escucha en el socket
    /// del panel y lo abre en una terminal nueva (`[dispositivos] terminal`
    /// o, si no, `[editor] terminal`). Si no se puede lanzar, lo dice y lo
    /// abre aquí (y se puede conectar a mano con `tui --window-devices`).
    pub fn launch_devices_window(&mut self) {
        if self.devices.is_none() {
            self.message = "los dispositivos están en la ventana principal: ábrelo allí".into();
            return;
        }
        let target = self.project.clone().unwrap_or_else(|| self.path.clone());
        if self.devices_link.is_none() {
            let dir = match self.socket_dir.clone() {
                Some(dir) => Ok(dir),
                None => link::runtime_dir(),
            };
            let replaced = ToDevices::Bye(
                "se ha conectado otro panel de dispositivos a esta ventana principal".into(),
            );
            let server = dir.and_then(|dir| {
                Server::bind_with(
                    &link::devices_socket_path(&dir, &self.path),
                    replaced.encode(),
                )
            });
            match server {
                Ok(server) => {
                    self.devices_link = Some(DevicesLink {
                        server,
                        sent: None,
                        child: None,
                        command: String::new(),
                    })
                }
                Err(e) => {
                    self.fall_back_to_popup(format!(
                        "no se pudo abrir el panel de dispositivos en su ventana: {e}"
                    ));
                    return;
                }
            }
        }
        let Some(link) = &mut self.devices_link else {
            return;
        };
        if link.server.connected() {
            self.message = "el panel de dispositivos ya está abierto en su ventana".into();
            return;
        }
        let terminal = self
            .devices_terminal
            .clone()
            .or_else(|| self.editor_terminal.clone())
            .filter(|t| !t.is_empty());
        let Some(terminal) = terminal else {
            self.fall_back_to_popup(format!(
                "sin [dispositivos] terminal ni [editor] terminal en config.toml: aquí, o en \
                 otra terminal con tui --window-devices {}",
                target.display()
            ));
            return;
        };
        let exe = std::env::current_exe().unwrap_or_else(|_| "tui".into());
        let args = devices_command(&terminal, &exe, &target);
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
                self.message = format!("abriendo el panel de dispositivos: {}", link.command);
            }
            Err(e) => self.fall_back_to_popup(format!(
                "no se pudo lanzar «{}» ({e}): el panel de dispositivos, aquí",
                args[0]
            )),
        }
    }

    /// Las terminales de `[editor]` y `[dispositivos]` (en los tests; en la
    /// aplicación, de `config.toml`).
    #[cfg(test)]
    pub fn set_terminals(&mut self, editor: Option<Vec<String>>, devices: Option<Vec<String>>) {
        self.editor_terminal = editor;
        self.devices_terminal = devices;
    }

    /// Abre el panel aquí con un mensaje de por qué.
    fn fall_back_to_popup(&mut self, why: String) {
        self.open_devices_panel();
        self.message = why;
    }

    /// «Volver a meter el panel de dispositivos en la principal»: cierra
    /// su ventana y lo abre aquí.
    pub fn reclaim_devices_window(&mut self) {
        let Some(mut link) = self.devices_link.take() else {
            self.message = "el panel de dispositivos no está en su ventana".into();
            return;
        };
        if let Some(conn) = link.server.client() {
            let _ = conn.send(&ToDevices::GiveBack);
            let _ = conn.drain(std::time::Duration::from_millis(200));
        }
        if let Some(mut child) = link.child.take() {
            std::thread::spawn(move || {
                let _ = child.wait();
            });
        }
        // (Al soltar el enlace se borra el socket.)
        drop(link);
        self.open_devices_panel();
        self.message = "el panel de dispositivos vuelve a esta ventana".into();
    }

    /// Atiende el panel en su ventana: conexiones nuevas, lo que pide y lo
    /// que hay que mandarle. Nunca bloquea.
    fn poll_devices_link(&mut self) {
        let Some(link) = &mut self.devices_link else {
            return;
        };
        let (fresh, rejected) = link.server.accept();
        if let Some(rejected) = rejected {
            self.message = rejected;
        }
        let title = self.file_name();
        let Some(link) = &mut self.devices_link else {
            return;
        };
        if fresh {
            link.sent = None;
            if let Some(conn) = link.server.client() {
                let _ = conn.send(&ToDevices::Welcome {
                    version: link::VERSION,
                    title,
                });
            }
            // Ya está fuera: el de aquí se cierra.
            self.devices_panel = None;
            self.message = "panel de dispositivos en su ventana".into();
        }
        let Some(link) = &mut self.devices_link else {
            return;
        };
        // La terminal se cerró sin que el panel llegara a conectarse: no se
        // pudo abrir (la terminal no existe, falló...). Se abre aquí.
        if !link.server.connected()
            && let Some(child) = &mut link.child
            && let Ok(Some(status)) = child.try_wait()
        {
            let command = std::mem::take(&mut link.command);
            link.child = None;
            self.fall_back_to_popup(format!(
                "la terminal del panel de dispositivos se cerró sin abrirlo ({status}: {command}): \
                 el panel, aquí"
            ));
            return;
        }
        if link.server.connected() {
            link.child = None;
        }
        let messages = match link.server.client().map(Conn::receive::<FromDevices>) {
            None => return,
            Some(Ok(messages)) => messages,
            Some(Err(_)) => {
                link.server.drop_client();
                self.message =
                    "el panel de dispositivos se cerró · F1 «Dispositivos» o «Abrir el panel de \
                     dispositivos en su ventana»"
                        .into();
                return;
            }
        };
        for message in messages {
            match message {
                FromDevices::Hello { version } if version != link::VERSION => {
                    if let Some(link) = &mut self.devices_link {
                        if let Some(conn) = link.server.client() {
                            let _ = conn.send(&ToDevices::Bye(format!(
                                "protocolo distinto (principal {}, panel {version})",
                                link::VERSION
                            )));
                        }
                        link.server.drop_client();
                    }
                    self.message = "un panel de dispositivos de otra versión: rechazado".into();
                    return;
                }
                FromDevices::Hello { .. } => {}
                FromDevices::Refresh => self.refresh_devices(),
                FromDevices::Choose { kind, name } => self.choose_device(kind, &name),
            }
        }
        self.push_devices_view();
    }

    /// Manda al panel en su ventana las listas, si han cambiado.
    fn push_devices_view(&mut self) {
        let Some(view) = self.devices.as_ref().map(Devices::view) else {
            return;
        };
        let view = self.with_knobs(view);
        let Some(link) = &mut self.devices_link else {
            return;
        };
        if link.sent.as_ref() == Some(&view) {
            return;
        }
        if let Some(conn) = link.server.client() {
            if conn.send(&ToDevices::View(view.clone())).is_ok() {
                link.sent = Some(view);
            } else {
                link.server.drop_client();
            }
        }
    }
}
