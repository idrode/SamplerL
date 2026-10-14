//! Mini-notación de patrones: pasos separados por espacios (16 por
//! defecto; tantos como la longitud de la pista, ver [`parse_pattern_labeled`]).
//!
//! - `.` silencio (lo que sonaba sigue sonando).
//! - `_` alarga un paso más el golpe de antes (en el modo gate y en los
//!   instrumentos, la nota sigue sonando; en one-shot no cambia nada):
//!   `"1 _ _ _"` es una nota de cuatro pasos.
//! - `3` corte número 3 (desde 1). `x` equivale a `1`.
//! - `kick` el corte con esa etiqueta (ver [`parse_steps_labeled`] y
//!   [`check_label`]). Lleva los mismos sufijos: `kick^7@0.5`, `kick'r2`.
//! - Sufijos combinables, en cualquier orden y cada uno una vez (salvo `^`):
//!   `'` reverse, `^7` / `^-5` / `^0.5` pitch en semitonos (varios `^` son
//!   un acorde, hasta 4 notas: `1^0^4^7`; solo en instrumentos), `@0.5` volumen,
//!   `r4` retrigger/stutter (el paso se repite 4 veces, de 1 a 16) y `~800`
//!   corte del primer filtro de la pista en Hz desde ese golpe (parameter
//!   lock).
//!
//! Ejemplo: `"1 . 3' .  1^7 . 5@0.5 .  . 3'^-12@0.8 . x  . . . ."`.

use std::fmt;

use crate::sequencer::{MAX_CHORD, MAX_STEPS, STEPS, Step};

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

/// Lee un patrón de 16 pasos (solo números de corte y `x`).
pub fn parse_steps(text: &str) -> Result<[Option<Step>; STEPS], NotationError> {
    parse_steps_labeled(text, &[])
}

/// Lee un patrón de 16 pasos en el que los cortes también se pueden
/// nombrar por su etiqueta: `labels[i]` es la del corte `i` (desde 0).
pub fn parse_steps_labeled(
    text: &str,
    labels: &[Option<String>],
) -> Result<[Option<Step>; STEPS], NotationError> {
    let steps = parse_pattern_labeled(text, STEPS, labels)?;
    Ok(steps.try_into().expect("tiene STEPS pasos"))
}

/// Lee un patrón de `length` pasos (de 1 a [`MAX_STEPS`]; la longitud de la
/// pista), con etiquetas como [`parse_steps_labeled`].
pub fn parse_pattern_labeled(
    text: &str,
    length: usize,
    labels: &[Option<String>],
) -> Result<Vec<Option<Step>>, NotationError> {
    let error = |message: String| NotationError {
        step: None,
        token: String::new(),
        message,
    };
    if !(1..=MAX_STEPS).contains(&length) {
        return Err(error(format!(
            "la longitud del patrón va de 1 a {MAX_STEPS} pasos (es {length})"
        )));
    }
    let tokens: Vec<&str> = text.split_whitespace().collect();
    if tokens.len() != length {
        return Err(error(format!(
            "el patrón necesita {length} pasos, tiene {}",
            tokens.len()
        )));
    }
    let mut steps: Vec<Option<Step>> = Vec::with_capacity(length);
    // Último golpe que un `_` puede alargar.
    let mut held: Option<usize> = None;
    for (i, token) in tokens.iter().enumerate() {
        let step = parse_step_labeled(token, labels).map_err(|message| NotationError {
            step: Some(i + 1),
            token: token.to_string(),
            message,
        })?;
        if *token == HOLD {
            let Some(j) = held else {
                return Err(NotationError {
                    step: Some(i + 1),
                    token: token.to_string(),
                    message: "_ alarga el golpe de antes y aquí no hay ninguno".into(),
                });
            };
            if let Some(hit) = &mut steps[j] {
                hit.hold += 1;
            }
        } else {
            held = step.is_some().then_some(i);
        }
        steps.push(step);
    }
    Ok(steps)
}

/// Paso que alarga el golpe anterior.
pub const HOLD: &str = "_";

/// Lee un paso: `.`, `_` (los dos sin golpe: `_` lo cuenta el patrón), o
/// corte con sufijos opcionales.
pub fn parse_step(token: &str) -> Result<Option<Step>, String> {
    parse_step_labeled(token, &[])
}

/// Separa la etiqueta del principio de un paso (`kick^7` → `kick`, `^7`).
/// Un `r` y un número al final de la palabra son el retrigger (`kickr4` →
/// `kick`, `r4`), por eso ninguna etiqueta puede acabar así. `None` si el
/// paso no empieza por una letra o `_`.
pub fn split_label(token: &str) -> Option<(&str, &str)> {
    if !token.starts_with(|c: char| c.is_ascii_alphabetic() || c == '_') {
        return None;
    }
    let run = token
        .find(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
        .unwrap_or(token.len());
    let word = &token[..run];
    let digits = word.len() - word.trim_end_matches(|c: char| c.is_ascii_digit()).len();
    let end = match word[..word.len() - digits].strip_suffix('r') {
        Some(base) if digits > 0 && !base.is_empty() => base.len(),
        _ => run,
    };
    Some((&token[..end], &token[end..]))
}

/// Comprueba que un nombre sirve de etiqueta de corte: letras, números y
/// `_`, empezando por letra o `_`; ni `x` ni nada que se confunda con la
/// notación (un número o un final en `r` y un número, como `r4`).
pub fn check_label(name: &str) -> Result<(), String> {
    if name.is_empty() {
        return Err("la etiqueta está vacía".into());
    }
    if name.chars().all(|c| c.is_ascii_digit()) {
        return Err(format!(
            "\"{name}\" es un número: las etiquetas son nombres"
        ));
    }
    if name.starts_with(|c: char| c.is_ascii_digit()) {
        return Err(format!(
            "\"{name}\" empieza por un número (se leería como un corte)"
        ));
    }
    if let Some(c) = name
        .chars()
        .find(|c| !(c.is_ascii_alphanumeric() || *c == '_'))
    {
        return Err(format!(
            "\"{name}\" tiene '{c}': solo letras sin tilde, números y _"
        ));
    }
    if name == "x" {
        return Err("\"x\" ya es el corte 1 en la notación".into());
    }
    if name == HOLD {
        return Err("\"_\" ya es «alargar el golpe» en la notación".into());
    }
    let retrig_like =
        name.len() > 1 && name.starts_with('r') && name[1..].chars().all(|c| c.is_ascii_digit());
    if retrig_like || split_label(name).is_some_and(|(base, _)| base != name) {
        return Err(format!(
            "\"{name}\" acaba en r y un número: se confundiría con el retrigger (r4)"
        ));
    }
    Ok(())
}

/// Como [`parse_step`], pero el corte también puede ser una etiqueta de
/// `labels` (`labels[i]` es la del corte `i`, desde 0).
pub fn parse_step_labeled(token: &str, labels: &[Option<String>]) -> Result<Option<Step>, String> {
    if token == "." || token == HOLD {
        return Ok(None);
    }
    let mut rest = token;

    let slice = if let Some((name, after)) = split_label(token) {
        rest = after;
        if name == "x" {
            1
        } else {
            match labels.iter().position(|l| l.as_deref() == Some(name)) {
                Some(i) => i + 1,
                None => {
                    let known: Vec<&str> = labels.iter().flatten().map(String::as_str).collect();
                    return Err(if known.is_empty() {
                        format!("la etiqueta \"{name}\" no existe (no hay cortes con etiqueta)")
                    } else {
                        format!(
                            "la etiqueta \"{name}\" no existe (hay: {})",
                            known.join(", ")
                        )
                    });
                }
            }
        }
    } else {
        let digits = rest.len() - rest.trim_start_matches(|c: char| c.is_ascii_digit()).len();
        if digits == 0 {
            return Err(
                "debe empezar por un número de corte (desde 1), una etiqueta, \"x\" o \".\"".into(),
            );
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
    let mut pitches = 0;
    let (mut seen_reverse, mut seen_gain) = (false, false);
    let (mut seen_retrig, mut seen_lock) = (false, false);
    while let Some(c) = rest.chars().next() {
        rest = &rest[c.len_utf8()..];
        match c {
            '\'' => {
                once(&mut seen_reverse, "reverse (')")?;
                step.reverse = true;
            }
            '^' => {
                let (value, after) = number(rest, true).ok_or("falta el número después de ^")?;
                // El primero es el pitch; los siguientes, notas del acorde.
                match pitches {
                    0 => step.semitones = value,
                    n if n < MAX_CHORD => step.chord[n - 1] = Some(value - step.semitones),
                    _ => return Err(format!("un acorde tiene como mucho {MAX_CHORD} notas (^)")),
                }
                pitches += 1;
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
            '~' => {
                once(&mut seen_lock, "corte (~)")?;
                let (value, after) =
                    number(rest, false).ok_or("falta el corte en Hz después de ~")?;
                step.lock = Some(value);
                rest = after;
            }
            other => {
                return Err(format!(
                    "carácter inesperado '{other}' \
                     (efectos: ' reverse, ^n pitch, @n volumen, rN retrigger, ~Hz corte)"
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
