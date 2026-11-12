//! La rueda de verdad: `tui --edit` en un pseudoterminal, con las
//! secuencias que manda una terminal (SGR 1006: botones 64-67 y Shift con
//! la rueda), sin tarjeta de sonido. Se lee la pantalla con un intérprete
//! mínimo de lo que escribe ratatui (mover el cursor y escribir) y se
//! comprueba que la vista se desplaza y que el cursor no se movió (lo que se
//! escribe después va a la línea 1).

use std::io::{Read, Write};
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
use std::os::unix::process::CommandExt;
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

const COLS: u16 = 100;
const ROWS: u16 = 30;

/// Una pantalla de `COLS`×`ROWS` y lo que entiende de lo que escribe
/// ratatui por crossterm: `CSI fila;col H`, texto, y el resto se ignora.
struct Screen {
    cells: Vec<Vec<char>>,
    row: usize,
    col: usize,
    /// Bytes de un carácter UTF-8 a medias o de una secuencia a medias.
    pending: Vec<u8>,
}

impl Screen {
    fn new() -> Screen {
        Screen {
            cells: vec![vec![' '; COLS as usize]; ROWS as usize],
            row: 0,
            col: 0,
            pending: Vec::new(),
        }
    }

    fn feed(&mut self, bytes: &[u8]) {
        self.pending.extend_from_slice(bytes);
        let data = std::mem::take(&mut self.pending);
        let mut i = 0;
        while i < data.len() {
            match data[i] {
                0x1b => {
                    let Some(&kind) = data.get(i + 1) else {
                        self.pending = data[i..].to_vec();
                        return;
                    };
                    match kind {
                        b'[' => {
                            let Some(end) =
                                (i + 2..data.len()).find(|&j| (0x40..=0x7e).contains(&data[j]))
                            else {
                                self.pending = data[i..].to_vec();
                                return;
                            };
                            let params = String::from_utf8_lossy(&data[i + 2..end]).to_string();
                            match data[end] {
                                b'H' if !params.starts_with('?') => {
                                    let mut it =
                                        params.split(';').map(|p| p.parse::<usize>().unwrap_or(1));
                                    self.row = it.next().unwrap_or(1).saturating_sub(1);
                                    self.col = it.next().unwrap_or(1).saturating_sub(1);
                                }
                                b'J' if params == "2" => {
                                    for line in &mut self.cells {
                                        line.fill(' ');
                                    }
                                }
                                _ => {}
                            }
                            i = end + 1;
                        }
                        b']' => {
                            // OSC hasta BEL o ST.
                            let Some(end) = (i + 2..data.len()).find(|&j| {
                                data[j] == 0x07
                                    || (data[j] == 0x1b && data.get(j + 1) == Some(&b'\\'))
                            }) else {
                                self.pending = data[i..].to_vec();
                                return;
                            };
                            i = end + if data[end] == 0x07 { 1 } else { 2 };
                        }
                        _ => i += 2,
                    }
                }
                b'\r' => {
                    self.col = 0;
                    i += 1;
                }
                b'\n' => {
                    self.row = (self.row + 1).min(ROWS as usize - 1);
                    i += 1;
                }
                b if b < 0x20 => i += 1,
                b => {
                    let len = match b {
                        0xf0..=0xff => 4,
                        0xe0..=0xef => 3,
                        0xc0..=0xdf => 2,
                        _ => 1,
                    };
                    if i + len > data.len() {
                        self.pending = data[i..].to_vec();
                        return;
                    }
                    let c = std::str::from_utf8(&data[i..i + len])
                        .ok()
                        .and_then(|s| s.chars().next())
                        .unwrap_or('?');
                    if self.row < ROWS as usize && self.col < COLS as usize {
                        self.cells[self.row][self.col] = c;
                    }
                    self.col += 1;
                    i += len;
                }
            }
        }
    }

    fn text(&self) -> String {
        self.cells
            .iter()
            .map(|l| l.iter().collect::<String>())
            .collect::<Vec<_>>()
            .join("\n")
    }
}

/// La interfaz en un pseudoterminal.
struct Term {
    master: Arc<Mutex<std::fs::File>>,
    screen: Arc<Mutex<Screen>>,
    child: Child,
}

impl Term {
    fn spawn(dir: &Path, script: &str) -> Term {
        let mut master = -1;
        let mut slave = -1;
        let size = libc::winsize {
            ws_row: ROWS,
            ws_col: COLS,
            ws_xpixel: 0,
            ws_ypixel: 0,
        };
        // SAFETY: punteros válidos a enteros y a un winsize.
        let ok = unsafe {
            libc::openpty(
                &mut master,
                &mut slave,
                std::ptr::null_mut(),
                std::ptr::null(),
                &size,
            )
        };
        assert_eq!(ok, 0, "openpty");
        // SAFETY: openpty acaba de abrirlos y son nuestros.
        let (master, slave) =
            unsafe { (OwnedFd::from_raw_fd(master), OwnedFd::from_raw_fd(slave)) };
        let slave_fd = slave.as_raw_fd();
        let mut command = Command::new(env!("CARGO_BIN_EXE_tui"));
        command
            .args(["--edit", script])
            .current_dir(dir)
            .env("HOME", dir)
            .env("XDG_CONFIG_HOME", dir.join("config"))
            .env("XDG_RUNTIME_DIR", dir)
            .env("TERM", "xterm-256color")
            .env("LANG", "es_ES.UTF-8")
            .stdin(Stdio::from(slave.try_clone().unwrap()))
            .stdout(Stdio::from(slave.try_clone().unwrap()))
            .stderr(Stdio::from(slave));
        // SAFETY: solo llamadas async-signal-safe entre fork y exec.
        unsafe {
            command.pre_exec(move || {
                if libc::setsid() < 0 {
                    return Err(std::io::Error::last_os_error());
                }
                if libc::ioctl(slave_fd, libc::TIOCSCTTY as _, 0) < 0 {
                    return Err(std::io::Error::last_os_error());
                }
                Ok(())
            });
        }
        let child = command.spawn().expect("arranca tui");
        let master = Arc::new(Mutex::new(std::fs::File::from(master)));
        let screen = Arc::new(Mutex::new(Screen::new()));
        let reader = master.lock().unwrap().try_clone().unwrap();
        let (writer, sink) = (master.clone(), screen.clone());
        std::thread::spawn(move || {
            let mut reader = reader;
            let mut buf = [0u8; 1 << 16];
            loop {
                let n = match reader.read(&mut buf) {
                    Ok(0) | Err(_) => return,
                    Ok(n) => n,
                };
                let data = &buf[..n];
                // Las preguntas de la terminal: atributos y posición.
                let contains = |needle: &[u8]| data.windows(needle.len()).any(|w| w == needle);
                if contains(b"\x1b[c") {
                    let _ = writer.lock().unwrap().write_all(b"\x1b[?62;22c");
                }
                if contains(b"\x1b[6n") {
                    let _ = writer.lock().unwrap().write_all(b"\x1b[1;1R");
                }
                sink.lock().unwrap().feed(data);
            }
        });
        Term {
            master,
            screen,
            child,
        }
    }

    fn send(&self, bytes: &[u8]) {
        self.master.lock().unwrap().write_all(bytes).unwrap();
    }

    fn text(&self) -> String {
        self.screen.lock().unwrap().text()
    }

    /// Espera hasta que la pantalla diga `needle` (o falla con lo que ve).
    fn wait_for(&self, needle: &str) {
        let start = Instant::now();
        while start.elapsed() < Duration::from_secs(20) {
            if self.text().contains(needle) {
                return;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        panic!("no sale «{needle}» en la pantalla:\n{}", self.text());
    }

    /// Pasos de rueda sueltos (no una ráfaga) en (`x`, `y`), desde 1.
    fn wheel(&self, button: u8, x: u16, y: u16, times: usize) {
        for _ in 0..times {
            self.send(format!("\x1b[<{button};{x};{y}M").as_bytes());
            std::thread::sleep(Duration::from_millis(60));
        }
    }
}

impl Drop for Term {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

#[test]
fn real_wheel_sequences_scroll_the_editor_without_moving_the_cursor() {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join("pty_wheel");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("config")).unwrap();
    let mut song = String::from("bpm(120)\n");
    for i in 1..=120 {
        if i == 70 {
            song.push_str(&format!("-- ancha {} FIN_DE_LA_ANCHA\n", "=".repeat(200)));
        } else {
            song.push_str(&format!("-- marca_{i:03}\n"));
        }
    }
    std::fs::write(dir.join("song.lua"), &song).unwrap();
    let term = Term::spawn(&dir, "song.lua");
    term.wait_for("marca_001");
    // El editor maximizado (Alt-Z): el centro de la pantalla es suyo.
    term.send(b"\x1bz");
    std::thread::sleep(Duration::from_millis(400));
    let (x, y) = (COLS / 2, ROWS / 2);
    assert!(!term.text().contains("marca_060"));
    // Botón 65: la rueda hacia abajo, 20 pasos de 3 líneas.
    term.wheel(65, x, y, 20);
    term.wait_for("marca_062");
    assert!(!term.text().contains("marca_001"), "{}", term.text());
    // Botón 64: hacia arriba.
    term.wheel(64, x, y, 2);
    term.wait_for("marca_056");
    // Botón 67: la rueda hacia la derecha, hasta el final de la línea ancha.
    assert!(!term.text().contains("FIN_DE_LA_ANCHA"));
    term.wheel(67, x, y, 40);
    term.wait_for("FIN_DE_LA_ANCHA");
    // Shift y la rueda hacia arriba (64 + 4): hacia la izquierda.
    term.wheel(68, x, y, 60);
    term.wait_for("marca_06");
    // El cursor sigue en la línea 1: lo que se escribe va ahí, y la vista
    // vuelve a él.
    term.send(b"Q");
    term.wait_for("Qbpm(120)");
    // Ctrl-S guarda; Ctrl-Q sale.
    term.send(b"\x13");
    let start = Instant::now();
    while !std::fs::read_to_string(dir.join("song.lua"))
        .unwrap()
        .starts_with("Qbpm(120)")
    {
        assert!(start.elapsed() < Duration::from_secs(10), "no se guardó");
        std::thread::sleep(Duration::from_millis(50));
    }
    term.send(b"\x11");
}
