//! Portapapeles del sistema para el editor. Copiar y cortar mandan la
//! selección por dos vías: OSC 52 (la terminal la guarda en el
//! portapapeles; funciona también por SSH y, con `set-clipboard on`, dentro
//! de tmux) y `wl-copy` si está en el PATH (Wayland). Pegar lee con
//! `wl-paste` si está; si no (o si falla o tarda), con el portapapeles
//! interno del editor.
//!
//! Los programas se ejecutan a través de [`Runner`], así que los tests usan
//! uno falso. Nada bloquea la interfaz más de [`READ_TIMEOUT`]: `wl-copy`
//! va en un hilo aparte y `wl-paste` se abandona si no contesta a tiempo.

use std::io::{Read, Write};
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::time::Duration;

/// Lo que se espera a `wl-paste` antes de usar el portapapeles interno.
pub const READ_TIMEOUT: Duration = Duration::from_millis(250);
/// Lo que se espera a `wl-copy` (en su hilo) antes de matarlo.
const WRITE_TIMEOUT: Duration = Duration::from_secs(2);

/// Ejecuta los programas del portapapeles.
pub trait Runner: Send {
    /// El programa está en el PATH.
    fn available(&self, program: &str) -> bool;
    /// Lanza `program args` con `text` por su entrada, sin esperar.
    fn write(&self, program: &str, args: &[&str], text: &str);
    /// Lanza `program args` y devuelve su salida si acaba bien antes de
    /// `timeout`.
    fn read(&self, program: &str, args: &[&str], timeout: Duration) -> Option<String>;
}

/// El de verdad: `std::process`.
pub struct SystemRunner;

impl Runner for SystemRunner {
    fn available(&self, program: &str) -> bool {
        std::env::var_os("PATH")
            .is_some_and(|path| std::env::split_paths(&path).any(|dir| dir.join(program).is_file()))
    }

    fn write(&self, program: &str, args: &[&str], text: &str) {
        let Ok(mut child) = Command::new(program)
            .args(args)
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
        else {
            return;
        };
        let text = text.to_string();
        std::thread::spawn(move || {
            if let Some(mut stdin) = child.stdin.take() {
                let _ = stdin.write_all(text.as_bytes());
            }
            wait_or_kill(&mut child, WRITE_TIMEOUT);
        });
    }

    fn read(&self, program: &str, args: &[&str], timeout: Duration) -> Option<String> {
        let mut child = Command::new(program)
            .args(args)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .ok()?;
        let mut stdout = child.stdout.take()?;
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            let mut out = Vec::new();
            let ok = stdout.read_to_end(&mut out).is_ok();
            let _ = tx.send(ok.then_some(out));
        });
        let out = rx.recv_timeout(timeout).ok().flatten();
        if out.is_none() {
            let _ = child.kill();
        }
        let status = child.wait().ok()?;
        String::from_utf8(out?).ok().filter(|_| status.success())
    }
}

/// Espera a que acabe `child`; si tarda más de `timeout`, lo mata.
fn wait_or_kill(child: &mut std::process::Child, timeout: Duration) {
    let start = std::time::Instant::now();
    while start.elapsed() < timeout {
        if let Ok(Some(_)) = child.try_wait() {
            return;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    let _ = child.kill();
    let _ = child.wait();
}

/// Ninguno: en los tests, para no tocar el portapapeles de quien los corre.
pub struct NoRunner;

impl Runner for NoRunner {
    fn available(&self, _: &str) -> bool {
        false
    }
    fn write(&self, _: &str, _: &[&str], _: &str) {}
    fn read(&self, _: &str, _: &[&str], _: Duration) -> Option<String> {
        None
    }
}

/// El portapapeles del sistema, con lo que hay instalado.
pub struct Clipboard {
    runner: Box<dyn Runner>,
    /// `wl-copy` y `wl-paste` están (se mira una vez, al crearlo).
    copy: bool,
    paste: bool,
    /// Secuencias OSC 52 por escribir en la terminal.
    osc: Vec<String>,
}

impl Clipboard {
    pub fn new(runner: Box<dyn Runner>) -> Self {
        let copy = runner.available("wl-copy");
        let paste = runner.available("wl-paste");
        Clipboard {
            runner,
            copy,
            paste,
            osc: Vec::new(),
        }
    }

    /// El del sistema (en los tests, ninguno).
    pub fn system() -> Self {
        if cfg!(test) {
            Clipboard::new(Box::new(NoRunner))
        } else {
            Clipboard::new(Box::new(SystemRunner))
        }
    }

    /// Manda `text` al portapapeles: OSC 52 y `wl-copy` si está.
    pub fn set(&mut self, text: &str) {
        if text.is_empty() {
            return;
        }
        self.osc.push(osc52(text));
        if self.copy {
            self.runner.write("wl-copy", &[], text);
        }
    }

    /// Lo que hay en el portapapeles del sistema (`wl-paste`), si se
    /// puede leer a tiempo; `None`: usar el interno.
    pub fn get(&self) -> Option<String> {
        if !self.paste {
            return None;
        }
        self.runner
            .read("wl-paste", &["--no-newline"], READ_TIMEOUT)
            .map(|t| crate::editing::normalize_newlines(&t))
    }

    /// Las secuencias OSC 52 pendientes, para escribirlas en la terminal.
    pub fn take_osc(&mut self) -> Vec<String> {
        std::mem::take(&mut self.osc)
    }
}

/// `ESC ] 52 ; c ; <base64> BEL`: pone `text` en el portapapeles de la
/// terminal.
pub fn osc52(text: &str) -> String {
    format!("\x1b]52;c;{}\x07", base64(text.as_bytes()))
}

/// Base64 estándar (con relleno).
fn base64(bytes: &[u8]) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let n = chunk
            .iter()
            .enumerate()
            .fold(0u32, |n, (i, b)| n | (*b as u32) << (16 - 8 * i));
        for i in 0..4 {
            if i <= chunk.len() {
                out.push(ALPHABET[(n >> (18 - 6 * i) & 63) as usize] as char);
            } else {
                out.push('=');
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base64_matches_the_standard() {
        for (text, expected) in [
            ("", ""),
            ("f", "Zg=="),
            ("fo", "Zm8="),
            ("foo", "Zm9v"),
            ("foobar", "Zm9vYmFy"),
            ("ñ", "w7E="),
        ] {
            assert_eq!(base64(text.as_bytes()), expected, "{text}");
        }
        assert_eq!(osc52("hola"), "\x1b]52;c;aG9sYQ==\x07");
    }

    #[test]
    fn the_system_runner_reads_and_gives_up_in_time() {
        let runner = SystemRunner;
        assert!(runner.available("sh"));
        assert!(!runner.available("no-existe-este-programa"));
        let out = runner.read("sh", &["-c", "printf 'a\\nb'"], READ_TIMEOUT);
        assert_eq!(out.as_deref(), Some("a\nb"));
        // Un programa que tarda se abandona a tiempo; uno que falla o no
        // existe, nada.
        let start = std::time::Instant::now();
        assert_eq!(
            runner.read("sleep", &["5"], Duration::from_millis(100)),
            None
        );
        assert!(start.elapsed() < Duration::from_secs(2));
        assert_eq!(runner.read("sh", &["-c", "exit 1"], READ_TIMEOUT), None);
        assert_eq!(
            runner.read("no-existe-este-programa", &[], READ_TIMEOUT),
            None
        );
    }
}
