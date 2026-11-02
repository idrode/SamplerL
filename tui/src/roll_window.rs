//! El piano roll en su propia ventana (`tui --window-roll`): solo el panel,
//! unido a la ventana principal del mismo script por su socket (ver
//! `crate::link`). No evalúa el script ni toca el audio: manda a la
//! principal su tamaño, las teclas y el ratón, y dibuja la vista que ella
//! le devuelve. Así la principal (dueña de la canción y del texto) aplica
//! cada edición una sola vez y el editor, esté donde esté, la recibe como
//! cualquier cambio. Si la principal no está, reintenta cada segundo.

use std::path::Path;

use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::Stylize;
use ratatui::text::Line;
use ratatui_textarea::{Input, Key};

use crate::app::{Flow, MouseAction};
use crate::link::{self, Client, FromRoll, Message, ToRoll};
use crate::roll_panel::{RollPanel, RollView};
use crate::ui::{accent, warning_color};

pub struct RollWindow {
    client: Client,
    /// El script de la principal (del saludo).
    title: String,
    view: Option<RollView>,
    /// La línea del enlace.
    status: String,
    /// La principal pidió el piano roll de vuelta: la ventana se cierra.
    leaving: bool,
    /// Dónde está el panel y el tamaño que sabe la principal.
    area: Rect,
    sent_size: Option<(u16, u16)>,
}

impl RollWindow {
    /// Busca la ventana principal que escucha en `socket`.
    pub fn new(socket: &Path) -> RollWindow {
        let hello = FromRoll::Hello {
            version: link::VERSION,
        };
        let mut window = RollWindow {
            client: Client::with_hello(socket, hello.encode()),
            title: String::new(),
            view: None,
            status: "buscando la ventana principal…".into(),
            leaving: false,
            area: Rect::default(),
            sent_size: None,
        };
        window.tick();
        window
    }

    /// Atiende el enlace: conecta (o reintenta), manda el tamaño si ha
    /// cambiado y lee lo que llega. Nunca bloquea.
    pub fn tick(&mut self) {
        if self.client.connect(false).is_some() {
            self.status = "conectado con la ventana principal".into();
            self.sent_size = None;
        }
        let messages = match self.client.conn().map(|c| c.receive::<ToRoll>()) {
            None => return,
            Some(Ok(messages)) => messages,
            Some(Err(_)) => {
                self.lost();
                return;
            }
        };
        for message in messages {
            match message {
                ToRoll::Welcome { version, .. } if version != link::VERSION => {
                    self.client.disconnect();
                    self.client.stopped = true;
                    self.status = format!(
                        "✗ protocolo distinto (principal {version}, piano roll {})",
                        link::VERSION
                    );
                }
                ToRoll::Welcome { title, .. } => {
                    self.title = title;
                    self.status = "conectado con la ventana principal".into();
                }
                ToRoll::View(view) => self.view = Some(view),
                ToRoll::Bye(reason) => {
                    self.client.disconnect();
                    self.client.stopped = true;
                    self.view = None;
                    self.status = format!("✗ {reason}");
                }
                ToRoll::GiveBack => self.leaving = true,
            }
        }
        // Después de leer: si la principal se ha ido, lo que mandó antes
        // (su `devuelve`) se ha leído ya.
        let size = (self.area.width, self.area.height);
        if !self.leaving
            && self.sent_size != Some(size)
            && size.0 > 0
            && self.send(&FromRoll::Size {
                width: size.0,
                height: size.1,
            })
        {
            self.sent_size = Some(size);
        }
    }

    fn lost(&mut self) {
        self.client.disconnect();
        self.view = None;
        self.sent_size = None;
        self.status = "✗ sin ventana principal: reintentando…".into();
    }

    /// Manda un mensaje; si no se puede, la principal se ha ido.
    fn send(&mut self, message: &FromRoll) -> bool {
        let Some(conn) = self.client.conn() else {
            return false;
        };
        if conn.send(message).is_err() {
            self.lost();
            return false;
        }
        true
    }

    /// La principal se quedó el piano roll: esta ventana se cierra.
    pub fn leaving(&self) -> bool {
        self.leaving
    }

    /// Lo que se ve (en los tests).
    #[cfg(test)]
    pub fn view(&self) -> Option<&RollView> {
        self.view.as_ref()
    }

    /// Una tecla: Ctrl-Q cierra la ventana (la principal sigue); las demás
    /// van a la principal.
    pub fn handle(&mut self, input: Input) -> Flow {
        if input.ctrl && matches!(input.key, Key::Char('q')) {
            return Flow::Quit;
        }
        if self.view.is_none() {
            if matches!(input.key, Key::Char('r')) && self.client.connect(true).is_some() {
                self.status = "conectado con la ventana principal".into();
            }
            return Flow::Continue;
        }
        self.send(&FromRoll::Key(input));
        Flow::Continue
    }

    /// El ratón en la celda `x`, `y` de la terminal.
    pub fn mouse(&mut self, action: MouseAction, x: u16, y: u16) {
        let inside = x >= self.area.x
            && y >= self.area.y
            && x < self.area.right()
            && y < self.area.bottom();
        if self.view.is_some() && (inside || matches!(action, MouseAction::Drag | MouseAction::Up)) {
            let (x, y) = (
                x.saturating_sub(self.area.x),
                y.saturating_sub(self.area.y),
            );
            self.send(&FromRoll::Mouse { action, x, y });
        }
    }

    pub fn draw(&mut self, frame: &mut Frame) {
        let [panel, status] =
            Layout::vertical([Constraint::Min(3), Constraint::Length(1)]).areas(frame.area());
        self.area = panel;
        match &self.view {
            Some(view) => frame.render_widget(RollPanel(view), panel),
            None => frame.render_widget(
                ratatui::widgets::Block::bordered().title(" piano roll "),
                panel,
            ),
        }
        let color = if self.status.starts_with('✗') {
            warning_color()
        } else {
            accent()
        };
        let title = if self.title.is_empty() {
            String::new()
        } else {
            format!("{} · ", self.title)
        };
        let help = " · Ctrl-Q cierra (la principal sigue)";
        frame.render_widget(
            Line::from(format!("{title}{}{help}", self.status)).fg(color),
            status,
        );
    }
}
