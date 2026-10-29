//! El panel de dispositivos en su propia ventana (`tui --window-devices`):
//! solo el panel, conectado con la ventana principal del mismo script por
//! su socket (ver `crate::link`). Nunca abre la tarjeta ni el MIDI: lo que
//! elige se lo pide a la principal, que es la dueña de los dispositivos, y
//! enseña lo que ella le manda. Si la principal no está, reintenta cada
//! segundo.

use std::path::Path;
use std::time::Instant;

use ratatui::Frame;
use ratatui::layout::{Constraint, Layout};
use ratatui::style::Stylize;
use ratatui::text::Line;
use ratatui_textarea::{Input, Key};

use crate::app::Flow;
use crate::devices_panel::{self, Blink, Cursor, DevicesPanel, PanelKey};
use crate::link::{self, Client, FromDevices, Message, ToDevices};
use crate::ui::{accent, warning_color};

pub struct DevicesWindow {
    client: Client,
    /// El script de la principal (del saludo).
    title: String,
    view: Option<crate::devices::DevicesView>,
    cursor: Cursor,
    blink: Blink,
    /// La línea del enlace: conectado, buscando, rechazado...
    status: String,
    /// La principal pidió el panel de vuelta: la ventana se cierra.
    leaving: bool,
}

impl DevicesWindow {
    /// Busca la ventana principal que escucha en `socket`.
    pub fn new(socket: &Path) -> DevicesWindow {
        let hello = FromDevices::Hello {
            version: link::VERSION,
        };
        let mut window = DevicesWindow {
            client: Client::with_hello(socket, hello.encode()),
            title: String::new(),
            view: None,
            cursor: Cursor::default(),
            blink: Blink::default(),
            status: "buscando la ventana principal…".into(),
            leaving: false,
        };
        window.tick();
        window
    }

    /// Atiende el enlace en cada vuelta: conecta (o reintenta) y lee lo que
    /// llega. Nunca bloquea.
    pub fn tick(&mut self) {
        if self.client.connect(false).is_some() {
            self.status = "conectado con la ventana principal".into();
        }
        let messages = match self.client.conn().map(|c| c.receive::<ToDevices>()) {
            None => return,
            Some(Ok(messages)) => messages,
            Some(Err(_)) => {
                self.client.disconnect();
                self.view = None;
                self.status = "✗ sin ventana principal: reintentando…".into();
                return;
            }
        };
        for message in messages {
            match message {
                ToDevices::Welcome { version, .. } if version != link::VERSION => {
                    self.client.disconnect();
                    self.client.stopped = true;
                    self.status = format!(
                        "✗ protocolo distinto (principal {version}, panel {})",
                        link::VERSION
                    );
                }
                ToDevices::Welcome { title, .. } => {
                    self.title = title;
                    self.status = "conectado con la ventana principal".into();
                }
                ToDevices::View(view) => {
                    if self.view.is_none() {
                        self.cursor = Cursor::on_active(&view, crate::devices::Kind::Audio);
                    }
                    self.cursor.clamp(&view);
                    self.blink.observe(view.activity, Instant::now());
                    self.view = Some(view);
                }
                ToDevices::Bye(reason) => {
                    self.client.disconnect();
                    self.client.stopped = true;
                    self.view = None;
                    self.status = format!("✗ {reason}");
                }
                ToDevices::GiveBack => self.leaving = true,
            }
        }
    }

    /// La principal se quedó el panel: esta ventana se cierra.
    pub fn leaving(&self) -> bool {
        self.leaving
    }

    /// Una tecla: las del panel (se mandan a la principal) y Ctrl-Q, `q` o
    /// Esc para cerrar la ventana (la principal sigue).
    pub fn handle(&mut self, input: Input) -> Flow {
        if input.ctrl && matches!(input.key, Key::Char('q')) {
            return Flow::Quit;
        }
        let Some(view) = &self.view else {
            if matches!(input.key, Key::Esc | Key::Char('q')) {
                return Flow::Quit;
            }
            // Sin principal: reintentar ya.
            if matches!(input.key, Key::Char('r')) && self.client.connect(true).is_some() {
                self.status = "conectado con la ventana principal".into();
            }
            return Flow::Continue;
        };
        let request = match devices_panel::key(view, &mut self.cursor, &input) {
            PanelKey::Close => return Flow::Quit,
            PanelKey::Moved => return Flow::Continue,
            PanelKey::Refresh => FromDevices::Refresh,
            PanelKey::Choose(kind, name) => FromDevices::Choose { kind, name },
        };
        if let Some(conn) = self.client.conn()
            && conn.send(&request).is_err()
        {
            self.client.disconnect();
            self.status = "✗ sin ventana principal: reintentando…".into();
        }
        Flow::Continue
    }

    pub fn draw(&self, frame: &mut Frame) {
        let [panel, status] =
            Layout::vertical([Constraint::Min(3), Constraint::Length(1)]).areas(frame.area());
        let title = if self.title.is_empty() {
            "dispositivos".to_string()
        } else {
            format!("dispositivos · {}", self.title)
        };
        let empty = crate::devices::DevicesView::default();
        let view = self.view.as_ref().unwrap_or(&empty);
        frame.render_widget(
            DevicesPanel {
                view,
                cursor: self.cursor,
                lit: self.blink.lit(Instant::now()),
                title: &title,
            },
            panel,
        );
        let color = if self.status.starts_with('✗') {
            warning_color()
        } else {
            accent()
        };
        let help = " · Ctrl-Q, q o Esc cierra (la principal sigue)";
        frame.render_widget(
            Line::from(format!("{}{help}", self.status)).fg(color),
            status,
        );
    }
}
