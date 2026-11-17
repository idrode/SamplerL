//! La ventana del espectro estéreo vista desde la principal (Ventanas ›
//! «Espectro»): escucha en su socket, lanza la terminal y, solo mientras
//! la ventana está conectada y ha saludado, enciende la cola estéreo del
//! motor, calcula el espectro de cada canal y el paneo por banda
//! ([`crate::stereo`]) al ritmo de `[interfaz] visuales_hz` (que baja con
//! `bajo_consumo`) y le manda el cuadro ya reducido. Sin ventana, nada: ni
//! el motor copia, ni se calcula, ni se manda.
//!
//! Si la ventana se retrasa (no ha leído el cuadro anterior), el nuevo se
//! tira en vez de acumularlo. En silencio, con el último cuadro ya caído,
//! no se calcula ni se manda nada (la ventana no redibuja).

use std::collections::VecDeque;
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::time::Duration;

use super::App;
use crate::link::{self, Conn, FromSpectrum, Message, Server, ToSpectrum};
use crate::stereo::StereoAnalyzer;

/// Muestras por canal que se guardan: las que mira la FFT.
const HISTORY: usize = crate::viz::spectrum::SIZE;

/// La ventana del espectro, vista desde la principal.
#[derive(Default)]
pub struct SpectrumState {
    link: Option<SpectrumLink>,
}

struct SpectrumLink {
    server: Server,
    child: Option<Child>,
    command: String,
    /// La ventana saludó con la versión buena: se le manda y el motor
    /// copia.
    ready: bool,
    left: VecDeque<f32>,
    right: VecDeque<f32>,
    scratch: (Vec<f32>, Vec<f32>),
    /// Frames llegados desde el último cuadro.
    pending: usize,
    analyzer: StereoAnalyzer,
    /// Lo último que se mandó era silencio ya caído: no se repite.
    settled: bool,
    /// Cuadros mandados y tirados por no haber leído la ventana el
    /// anterior.
    sent: u64,
    skipped: u64,
}

impl SpectrumLink {
    fn new(server: Server) -> SpectrumLink {
        SpectrumLink {
            server,
            child: None,
            command: String::new(),
            ready: false,
            left: VecDeque::with_capacity(HISTORY),
            right: VecDeque::with_capacity(HISTORY),
            scratch: (Vec::new(), Vec::new()),
            pending: 0,
            analyzer: StereoAnalyzer::new(),
            settled: false,
            sent: 0,
            skipped: 0,
        }
    }
}

impl App {
    /// Ventanas › «Espectro»: escucha en su socket y abre la ventana en una
    /// terminal nueva (`[editor] terminal`). Si ya hay una, lo dice.
    pub(super) fn launch_spectrum_window(&mut self) {
        let target = crate::target::full_path(self.project.as_deref().unwrap_or(&self.path));
        if let Some(link) = &self.spectrum.link {
            self.message = if link.server.connected() {
                "la ventana del espectro ya está abierta (Ventanas › «Cerrar la ventana del \
                 espectro» la cierra)"
                    .into()
            } else if link.child.is_some() {
                format!("la ventana del espectro se está abriendo: {}", link.command)
            } else {
                format!(
                    "esperando la ventana del espectro: tui --window-spectrum {}",
                    target.display()
                )
            };
            return;
        }
        let dir = match self.socket_dir.clone() {
            Some(dir) => Ok(dir),
            None => link::runtime_dir(),
        };
        let replaced =
            ToSpectrum::Bye("se ha conectado otra ventana del espectro a esta principal".into());
        let server = dir.and_then(|dir| {
            Server::bind_with(
                &link::spectrum_socket_path(&dir, &self.path),
                replaced.encode(),
            )
        });
        let server = match server {
            Ok(server) => server,
            Err(e) => {
                self.message = format!("no se pudo abrir la ventana del espectro: {e}");
                return;
            }
        };
        self.spectrum.link = Some(SpectrumLink::new(server));
        let terminal = self.editor_terminal.clone().filter(|t| !t.is_empty());
        let Some(terminal) = terminal else {
            self.message = format!(
                "sin [editor] terminal en config.toml: ábrela en otra terminal con tui \
                 --window-spectrum {}",
                target.display()
            );
            return;
        };
        if let Some(problem) = crate::target::terminal_problem(&terminal) {
            self.spectrum.link = None;
            self.message = format!("{problem}: la ventana del espectro no se abre");
            return;
        }
        let exe = std::env::current_exe().unwrap_or_else(|_| "tui".into());
        let args = spectrum_command(&terminal, &exe, &target);
        let spawned = Command::new(&args[0])
            .args(&args[1..])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn();
        match spawned {
            Ok(child) => {
                if let Some(link) = &mut self.spectrum.link {
                    link.child = Some(child);
                    link.command = args.join(" ");
                    self.message = format!("abriendo el espectro: {}", link.command);
                }
            }
            Err(e) => {
                self.spectrum.link = None;
                self.message = format!(
                    "no se pudo lanzar «{}» ({e}): la ventana del espectro no se abre",
                    args[0]
                );
            }
        }
    }

    /// «Cerrar la ventana del espectro»: le dice que se vaya y deja de
    /// calcular.
    pub(super) fn close_spectrum_window(&mut self) {
        let Some(mut link) = self.spectrum.link.take() else {
            self.message = "la ventana del espectro no está abierta".into();
            return;
        };
        if let Some(conn) = link.server.client() {
            let _ = conn.send(&ToSpectrum::GiveBack);
            let _ = conn.drain(Duration::from_millis(200));
        }
        self.drop_spectrum(link);
        self.message = "ventana del espectro cerrada".into();
    }

    /// Suelta la ventana: la cola del motor se apaga y no se calcula nada
    /// más (el socket se borra).
    fn drop_spectrum(&mut self, mut link: SpectrumLink) {
        if let Some(mut child) = link.child.take() {
            std::thread::spawn(move || {
                let _ = child.wait();
            });
        }
        if link.ready {
            self.controller.set_stereo_viz(false);
        }
    }

    /// En cada vuelta: conexiones, el saludo y, con la ventana lista, el
    /// cuadro cuando toca. Nunca bloquea.
    pub(super) fn tick_spectrum(&mut self) {
        let Some(link) = &mut self.spectrum.link else {
            return;
        };
        let (fresh, rejected) = link.server.accept();
        if let Some(rejected) = rejected {
            self.message = rejected;
        }
        if fresh && link.ready {
            // Otra ventana sustituye a la de antes: saluda de nuevo.
            link.ready = false;
            self.controller.set_stereo_viz(false);
        }
        // La terminal terminó sin que la ventana haya llegado: si falló, se
        // deja; si salió bien, se sigue escuchando (hay terminales que le
        // pasan la ventana a una instancia que ya estaba abierta y salen).
        if !link.server.connected()
            && let Some(child) = &mut link.child
            && let Ok(Some(status)) = child.try_wait()
        {
            link.child = None;
            if !status.success() {
                let command = std::mem::take(&mut link.command);
                if let Some(link) = self.spectrum.link.take() {
                    self.drop_spectrum(link);
                }
                self.message =
                    format!("la terminal del espectro se cerró sin abrirlo ({status}: {command})");
                return;
            }
        }
        // Sin cliente después de haber tenido uno listo (un envío falló al
        // morir la ventana): se fue, como si hubiera cerrado.
        let gone = link.ready && !link.server.connected();
        let messages = match link.server.client().map(Conn::receive::<FromSpectrum>) {
            None if !gone => return,
            Some(Ok(messages)) => messages,
            None | Some(Err(_)) => {
                if let Some(link) = self.spectrum.link.take() {
                    self.drop_spectrum(link);
                }
                self.message = "ventana del espectro cerrada · Ventanas › «Espectro»".into();
                return;
            }
        };
        let title = self.file_name();
        let path = crate::target::full_path(&self.path);
        let Some(link) = &mut self.spectrum.link else {
            return;
        };
        link.child = None;
        for FromSpectrum::Hello { version } in messages {
            let Some(conn) = link.server.client() else {
                break;
            };
            if version != link::VERSION {
                let _ = conn.send(&ToSpectrum::Bye(format!(
                    "protocolo distinto (principal {}, espectro {version})",
                    link::VERSION
                )));
                let _ = conn.drain(Duration::from_millis(100));
                link.server.drop_client();
                self.message = "una ventana del espectro de otra versión: rechazada".into();
                continue;
            }
            if conn
                .send(&ToSpectrum::Welcome {
                    version: link::VERSION,
                    title: title.clone(),
                    path: path.clone(),
                })
                .is_ok()
            {
                link.ready = true;
                link.left.clear();
                link.right.clear();
                link.pending = 0;
                link.settled = false;
                self.controller.set_stereo_viz(true);
                self.message = "espectro estéreo en su ventana".into();
            }
        }
        self.feed_spectrum();
    }

    /// Vacía la cola estéreo y, cuando han llegado las muestras de un
    /// cuadro (`visuales_hz`), calcula y manda.
    fn feed_spectrum(&mut self) {
        let rate = self.sample_rate;
        let every = (rate / self.viz_hz.max(1)).max(1) as usize;
        let Some(link) = &mut self.spectrum.link else {
            return;
        };
        if !link.ready {
            return;
        }
        let (l, r) = &mut link.scratch;
        l.clear();
        r.clear();
        self.controller.stereo_samples(l, r);
        link.pending += l.len();
        for (history, fresh) in [(&mut link.left, &*l), (&mut link.right, &*r)] {
            history.extend(fresh.iter().copied());
            let extra = history.len().saturating_sub(HISTORY);
            history.drain(..extra);
        }
        if link.pending < every {
            return;
        }
        link.pending = 0;
        let silent = link.left.iter().chain(&link.right).all(|&x| x == 0.0);
        if silent && link.settled {
            return;
        }
        let Some(conn) = link.server.client() else {
            return;
        };
        // La ventana no ha leído el anterior: este se tira (sin calcularlo).
        if conn.pending() > 0 {
            link.skipped += 1;
            if conn.flush().is_err() {
                link.server.drop_client();
            }
            return;
        }
        let frame =
            link.analyzer
                .analyze(link.left.iter().copied(), link.right.iter().copied(), rate);
        let settled = silent && frame.silent();
        if conn.send(&ToSpectrum::Frame(frame.clone())).is_err() {
            link.server.drop_client();
            return;
        }
        link.sent += 1;
        link.settled = settled;
    }

    /// Si la ventana del espectro está conectada y lista (en los tests).
    #[cfg(test)]
    pub fn spectrum_ready(&self) -> bool {
        self.spectrum.link.as_ref().is_some_and(|l| l.ready)
    }

    /// Si la principal escucha la ventana del espectro (en los tests).
    #[cfg(test)]
    pub fn spectrum_listening(&self) -> bool {
        self.spectrum.link.is_some()
    }

    /// Cuadros mandados, tirados y FFT hechas (en los tests).
    #[cfg(test)]
    pub fn spectrum_cost(&self) -> (u64, u64, u64) {
        self.spectrum
            .link
            .as_ref()
            .map_or((0, 0, 0), |l| (l.sent, l.skipped, l.analyzer.runs()))
    }
}

/// La línea de órdenes que abre la ventana del espectro: la terminal y
/// detrás `tui --window-spectrum <script>`, con la ruta completa (la
/// terminal nueva puede arrancar en otra carpeta).
pub fn spectrum_command(terminal: &[String], exe: &Path, target: &Path) -> Vec<String> {
    let mut args = terminal.to_vec();
    args.push(exe.display().to_string());
    args.push("--window-spectrum".into());
    args.push(crate::target::full_path(target).display().to_string());
    args
}
