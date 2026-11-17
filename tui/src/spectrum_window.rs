//! El espectro estéreo en su propia ventana (`tui --window-spectrum`):
//! unida a la principal del mismo script por su socket (ver `crate::link`),
//! solo dibuja los cuadros que ella le manda, ya reducidos (unas bandas por
//! canal: ver `crate::stereo`). No toca el audio ni calcula nada; el modo
//! (campo o espejo) es suyo y se guarda en `[interfaz] espectro_modo`.
//!
//! Dibuja solo cuando hay algo nuevo (un cuadro, una tecla, otro tamaño):
//! sin datos, no redibuja. Lo que llega mal formado se tira sin cortar, y
//! de varios cuadros juntos solo cuenta el último. Si la principal se va,
//! lo dice y se cierra; si contesta otra (otra versión, otro script), se
//! queda con el error hasta pulsar Enter.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use ratatui::Frame;
use ratatui::layout::{Constraint, Layout};
use ratatui::style::Stylize;
use ratatui::text::Line;
use ratatui_textarea::{Input, Key};

use crate::app::Flow;
use crate::link::{self, Client, FromSpectrum, Message, ToSpectrum};
use crate::stereo::StereoFrame;
use crate::ui::{accent, warning_color};
use crate::viz::stereo::{Mode, StereoPanel};

/// Línea más larga que se acepta (un cuadro de 256 bandas ocupa ~1,6 KB).
const MAX_LINE: usize = 64 * 1024;
/// Lo que se queda a la vista el aviso de que la principal se fue.
const GOODBYE: Duration = Duration::from_millis(1500);

/// Cómo termina la ventana.
#[derive(Debug, Clone, PartialEq)]
pub enum Leaving {
    /// Sin error: la principal la cerró o se fue.
    Done,
    /// Con error: se queda en pantalla hasta pulsar Enter.
    Failed(String),
}

pub struct SpectrumWindow {
    client: Client,
    /// El script de esta ventana (canónico): la principal tiene que ser
    /// la suya.
    script: PathBuf,
    /// El nombre del script (del saludo).
    title: String,
    frame: Option<StereoFrame>,
    mode: Mode,
    /// Dónde se guarda el modo (`config.toml`; en los tests, ninguno).
    config: Option<PathBuf>,
    status: String,
    /// Llegó a hablar con la principal: si se pierde, es que se fue.
    met: bool,
    goodbye: Option<Instant>,
    leaving: Option<Leaving>,
    /// Hay algo nuevo que dibujar.
    dirty: bool,
    /// Cuadros que llegaron juntos y se saltaron, y líneas tiradas por mal
    /// formadas.
    skipped: u64,
    bad: u64,
}

impl SpectrumWindow {
    /// Busca la principal que escucha en `socket` (la de `script`).
    pub fn new(socket: &Path, script: &Path, mode: Mode, config: Option<PathBuf>) -> Self {
        let hello = FromSpectrum::Hello {
            version: link::VERSION,
        };
        let mut window = SpectrumWindow {
            client: Client::with_hello(socket, hello.encode()),
            script: script.to_path_buf(),
            title: String::new(),
            frame: None,
            mode,
            config,
            status: "buscando la ventana principal…".into(),
            met: false,
            goodbye: None,
            leaving: None,
            dirty: true,
            skipped: 0,
            bad: 0,
        };
        window.tick();
        window
    }

    /// Atiende el enlace: conecta (o reintenta) y lee lo que llega. Nunca
    /// bloquea.
    pub fn tick(&mut self) {
        if self.goodbye.is_some_and(|t| t.elapsed() >= GOODBYE) {
            self.leaving = Some(Leaving::Done);
            return;
        }
        if self.client.connect(false).is_some() {
            self.status = "conectado con la ventana principal".into();
            self.dirty = true;
        }
        let received = match self.client.conn().map(|c| c.receive_lossy(MAX_LINE)) {
            None => return,
            Some(Ok(received)) => received,
            Some(Err(_)) => {
                self.client.disconnect();
                self.lost();
                return;
            }
        };
        let (messages, bad) = received;
        self.bad += bad as u64;
        let mut fresh: Option<StereoFrame> = None;
        for message in messages {
            match message {
                ToSpectrum::Welcome { version, .. } if version != link::VERSION => {
                    self.fail(format!(
                        "protocolo distinto (principal {version}, espectro {}): abre la \
                         ventana desde una principal de la misma versión",
                        link::VERSION
                    ));
                    return;
                }
                ToSpectrum::Welcome { path, .. } if path != self.script => {
                    self.fail(format!(
                        "ha contestado la principal de otro script ({}), no la de {}",
                        path.display(),
                        self.script.display()
                    ));
                    return;
                }
                ToSpectrum::Welcome { title, .. } => {
                    self.title = title;
                    self.met = true;
                    self.status = "conectado con la ventana principal".into();
                    self.dirty = true;
                }
                ToSpectrum::Frame(frame) => {
                    if fresh.replace(frame).is_some() {
                        self.skipped += 1;
                    }
                }
                ToSpectrum::Bye(reason) => {
                    self.fail(reason);
                    return;
                }
                ToSpectrum::GiveBack => {
                    self.leaving = Some(Leaving::Done);
                    return;
                }
            }
        }
        if let Some(frame) = fresh
            && self.met
        {
            self.frame = Some(frame);
            self.dirty = true;
        }
    }

    /// La principal ya no está.
    fn lost(&mut self) {
        if self.met {
            self.status = "✗ la ventana principal se ha cerrado: esta también se cierra".into();
            self.goodbye = Some(Instant::now());
        } else {
            self.status = "✗ sin ventana principal: reintentando…".into();
        }
        self.dirty = true;
    }

    fn fail(&mut self, why: String) {
        self.client.disconnect();
        self.client.stopped = true;
        self.leaving = Some(Leaving::Failed(why));
    }

    /// Si hay que cerrar la ventana, y cómo.
    pub fn leaving(&self) -> Option<&Leaving> {
        self.leaving.as_ref()
    }

    /// Si hay algo nuevo que dibujar (y lo da por dibujado).
    pub fn take_dirty(&mut self) -> bool {
        std::mem::take(&mut self.dirty)
    }

    /// Algo cambió fuera (otro tamaño de terminal): hay que redibujar.
    pub fn touch(&mut self) {
        self.dirty = true;
    }

    /// Falló un dibujo: se dice en la línea de estado.
    pub fn draw_failed(&mut self, why: &str) {
        self.status = format!("✗ dibujo: {why}");
        self.dirty = true;
    }

    /// El modo (en los tests).
    #[cfg(test)]
    pub fn mode(&self) -> Mode {
        self.mode
    }

    /// El último cuadro, los saltados y los mal formados (en los tests).
    #[cfg(test)]
    pub fn seen(&self) -> (Option<&StereoFrame>, u64, u64) {
        (self.frame.as_ref(), self.skipped, self.bad)
    }

    /// Una tecla: `m` cambia de modo (y lo guarda), `r` reintenta ya, `q`,
    /// Esc o Ctrl-Q cierran (la principal sigue).
    pub fn handle(&mut self, input: Input) -> Flow {
        match input.key {
            Key::Char('q') | Key::Esc => return Flow::Quit,
            Key::Char('c') if input.ctrl => return Flow::Quit,
            Key::Char('m') | Key::Char('M') => {
                self.mode = self.mode.next();
                self.status = format!("modo {}", self.mode.name());
                if let Some(path) = &self.config {
                    let value = format!("\"{}\"", self.mode.name());
                    if let Err(e) =
                        crate::config::save_keys(path, "interfaz", &[("espectro_modo", value)])
                    {
                        self.status = format!("✗ no se pudo guardar en {}: {e}", path.display());
                    }
                }
                self.dirty = true;
            }
            Key::Char('r') if !self.client.connected() => {
                if self.client.connect(true).is_some() {
                    self.status = "conectado con la ventana principal".into();
                }
                self.dirty = true;
            }
            _ => {}
        }
        Flow::Continue
    }

    pub fn draw(&mut self, frame: &mut Frame) {
        let [panel, status] =
            Layout::vertical([Constraint::Min(1), Constraint::Length(1)]).areas(frame.area());
        let title = if self.title.is_empty() {
            format!(" espectro estéreo · {} ", self.mode.name())
        } else {
            format!(" espectro estéreo · {} · {} ", self.mode.name(), self.title)
        };
        frame.render_widget(
            StereoPanel {
                frame: self.frame.as_ref(),
                mode: self.mode,
                title: &title,
            },
            panel,
        );
        let color = if self.status.starts_with('✗') {
            warning_color()
        } else {
            accent()
        };
        let help = format!(" · m: {} · q cierra", self.mode.next().name());
        frame.render_widget(
            Line::from(format!("{}{help}", self.status)).fg(color),
            status,
        );
    }
}
