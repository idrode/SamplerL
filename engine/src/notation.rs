//! Mini-notación de patrones: 16 pasos separados por espacios.
//!
//! - `.` silencio (lo que sonaba sigue sonando).
//! - `3` corte número 3 (desde 1). `x` equivale a `1`.
//! - Sufijos combinables, en cualquier orden y cada uno una vez:
//!   `'` reverse, `^7` / `^-5` / `^0.5` pitch en semitonos, `@0.5` volumen,
//!   `r4` retrigger/stutter (el paso se repite 4 veces, de 1 a 16).
//!
//! Ejemplo: `"1 . 3' .  1^7 . 5@0.5 .  . 3'^-12@0.8 . x  . . . ."`.

use std::fmt;

use crate::sequencer::{STEPS, Step};

#[derive(Debug, Clone, PartialEq)]
pub struct NotationError {
    /// Paso donde está el error (desde 1), si es de un paso concreto.
    pub step: Option<usize>,
    pub token: String,
    pub message: String,
}

impl fmt::Display for NotationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.step {
            Some(step) => write!(f, "paso {step} (\"{}\"): {}", self.token, self.message),
            None => write!(f, "{}", self.message),
        }
    }
}

impl std::error::Error for NotationError {}

/// Lee un patrón de 16 pasos.
pub fn parse_steps(text: &str) -> Result<[Option<Step>; STEPS], NotationError> {
    let tokens: Vec<&str> = text.split_whitespace().collect();
    if tokens.len() != STEPS {
        return Err(NotationError {
            step: None,
            token: String::new(),
            message: format!("el patrón necesita {STEPS} pasos, tiene {}", tokens.len()),
        });
    }
    let mut steps = [None; STEPS];
    for (i, (slot, token)) in steps.iter_mut().zip(&tokens).enumerate() {
        *slot = parse_step(token).map_err(|message| NotationError {
            step: Some(i + 1),
            token: token.to_string(),
            message,
        })?;
    }
    Ok(steps)
}

/// Lee un paso: `.`, o corte con sufijos opcionales.
pub fn parse_step(token: &str) -> Result<Option<Step>, String> {
    if token == "." {
        return Ok(None);
    }
    let mut rest = token;

    let slice = if let Some(after) = rest.strip_prefix('x') {
        rest = after;
        1
    } else {
        let digits = rest.len() - rest.trim_start_matches(|c: char| c.is_ascii_digit()).len();
        if digits == 0 {
            return Err("debe empezar por un número de corte (desde 1), \"x\" o \".\"".into());
        }
        let n: usize = rest[..digits]
            .parse()
            .map_err(|_| "número de corte demasiado grande".to_string())?;
        if n == 0 {
            return Err("los cortes se numeran desde 1".into());
        }
        rest = &rest[digits..];
        n
    };

    let mut step = Step::new(slice - 1);
    let (mut seen_reverse, mut seen_pitch, mut seen_gain) = (false, false, false);
    let mut seen_retrig = false;
    while let Some(c) = rest.chars().next() {
        rest = &rest[c.len_utf8()..];
        match c {
            '\'' => {
                once(&mut seen_reverse, "reverse (')")?;
                step.reverse = true;
            }
            '^' => {
                once(&mut seen_pitch, "pitch (^)")?;
                let (value, after) = number(rest, true).ok_or("falta el número después de ^")?;
                step.semitones = value;
                rest = after;
            }
            '@' => {
                once(&mut seen_gain, "volumen (@)")?;
                let (value, after) = number(rest, false).ok_or("falta el número después de @")?;
                step.gain = value;
                rest = after;
            }
            'r' => {
                once(&mut seen_retrig, "retrigger (r)")?;
                let digits =
                    rest.len() - rest.trim_start_matches(|c: char| c.is_ascii_digit()).len();
                let count: u8 = rest[..digits]
                    .parse()
                    .map_err(|_| "falta el número de repeticiones después de r (1 a 16)")?;
                step.retrig = count;
                rest = &rest[digits..];
            }
            other => {
                return Err(format!(
                    "carácter inesperado '{other}' \
                     (efectos: ' reverse, ^n pitch, @n volumen, rN retrigger)"
                ));
            }
        }
    }
    Ok(Some(step))
}

fn once(seen: &mut bool, what: &str) -> Result<(), String> {
    if std::mem::replace(seen, true) {
        return Err(format!("{what} repetido"));
    }
    Ok(())
}

/// Lee un número decimal al principio de `text` (con signo si `signed`).
/// Devuelve el valor y el resto del texto.
fn number(text: &str, signed: bool) -> Option<(f32, &str)> {
    let mut end = 0;
    let bytes = text.as_bytes();
    if signed && matches!(bytes.first(), Some(b'+' | b'-')) {
        end = 1;
    }
    let digits_start = end;
    while end < bytes.len() && (bytes[end].is_ascii_digit() || bytes[end] == b'.') {
        end += 1;
    }
    let body = &text[digits_start..end];
    if !body.bytes().any(|b| b.is_ascii_digit()) || body.matches('.').count() > 1 {
        return None;
    }
    text[..end].parse().ok().map(|v| (v, &text[end..]))
}
