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
//!   un acorde, hasta 4 notas: `1^0^4^7`; solo en instrumentos), `%7` /
//!   `%-12` pitch en semitonos que no cambia la duración (va en vez del
//!   primer `^`; detrás pueden ir las notas del acorde), `@0.5` volumen,
//!   `r4` retrigger/stutter (el paso se repite 4 veces, de 1 a 16) y `~800`
//!   corte del primer filtro de la pista en Hz desde ese golpe (parameter
//!   lock).
//!
//! - Efectos del golpe entre llaves, uno por par de llaves, detrás del
//!   corte y en cualquier orden con los sufijos: `3{lp=800,q=4}{crush=6}`.
//!   Dentro, el efecto, `=` y el valor de su parámetro principal y, tras
//!   comas, otros parámetros `clave=valor`. Un valor puede ser un número,
//!   una palabra (`type=highpass`) o una rampa del golpe: `200>4000` va de
//!   200 a 4000 durante el paso y `200>4000:4` durante 4 pasos. Las llaves
//!   pueden llevar espacios dentro (el paso sigue siendo uno). Qué efectos
//!   y parámetros hay lo sabe el script (el registro de la API): aquí solo
//!   se lee lo escrito ([`WrittenFx`]) y se pasa a quien lo resuelve
//!   ([`parse_pattern_with`]).
//!
//! Ejemplo: `"1 . 3' .  1^7 . 5@0.5 .  . 3'^-12@0.8 . x  . . . ."`.

use std::fmt;

use crate::sequencer::{MAX_CHORD, MAX_STEPS, STEPS, Step};
use crate::slice_fx::SliceFx;

/// Un efecto escrito en un paso (`{lp=800,q=4}`), tal como se escribió.
#[derive(Debug, Clone, PartialEq)]
pub struct WrittenFx {
    /// El nombre del efecto (`lp`).
    pub name: String,
    /// El valor de su parámetro principal (`lp=800`), si lo lleva.
    pub main: Option<FxValue>,
    /// Los demás parámetros, en orden (`q=4`).
    pub params: Vec<(String, FxValue)>,
}

/// Un valor de un efecto de paso.
#[derive(Debug, Clone, PartialEq)]
pub enum FxValue {
    Number(f32),
    /// Rampa del golpe: de `from` a `to` en `steps` pasos (sin `:`, uno).
    Ramp {
        from: f32,
        to: f32,
        steps: Option<f64>,
    },
    /// Una palabra (`highpass`).
    Word(String),
}

/// Quien sabe convertir los efectos escritos en un paso en una cadena (el
/// script, con el registro de la API).
pub type FxResolver<'a> = &'a dyn Fn(&[WrittenFx]) -> Result<SliceFx, String>;

/// Los pasos de un patrón: columnas (en caracteres) de inicio y fin de
/// cada uno, separados por espacios que no estén dentro de llaves.
pub fn step_tokens(text: &str) -> Vec<(usize, usize)> {
    let mut out = Vec::new();
    let mut start = None;
    let mut depth = 0usize;
    let end = text.chars().count();
    for (i, c) in text.chars().chain([' ']).enumerate() {
        match c {
            '{' => depth += 1,
            '}' => depth = depth.saturating_sub(1),
            _ => {}
        }
        let space = c.is_whitespace() && (depth == 0 || i == end);
        match (space, start) {
            (false, None) => start = Some(i),
            (true, Some(s)) => {
                out.push((s, i));
                start = None;
            }
            _ => {}
        }
    }
    out
}

/// El texto de cada paso de un patrón (ver [`step_tokens`]).
pub fn step_texts(text: &str) -> Vec<String> {
    let chars: Vec<char> = text.chars().collect();
    step_tokens(text)
        .into_iter()
        .map(|(a, b)| chars[a..b].iter().collect())
        .collect()
}

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
    parse_pattern_with(text, length, labels, None)
}

/// Como [`parse_pattern_labeled`], con efectos por paso (`3{lp=800}`):
/// `fx` los convierte en su cadena. Sin él, unas llaves dan error.
pub fn parse_pattern_with(
    text: &str,
    length: usize,
    labels: &[Option<String>],
    fx: Option<FxResolver>,
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
    let tokens = step_texts(text);
    // Unas llaves sin cerrar se tragan el resto del patrón: el error, en
    // su paso.
    if let Some((i, token)) = tokens
        .iter()
        .enumerate()
        .find(|(_, t)| t.matches('{').count() > t.matches('}').count())
    {
        return Err(NotationError {
            step: Some(i + 1),
            token: token.chars().take(24).collect(),
            message: "falta } al final de los efectos del paso ({...})".into(),
        });
    }
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
        let token = token.as_str();
        let step = parse_step_with(token, labels, fx).map_err(|message| NotationError {
            step: Some(i + 1),
            token: token.to_string(),
            message,
        })?;
        if token == HOLD {
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
    parse_step_with(token, labels, None)
}

/// Como [`parse_step_labeled`], con los efectos del paso (`{...}`)
/// resueltos por `fx`.
pub fn parse_step_with(
    token: &str,
    labels: &[Option<String>],
    fx: Option<FxResolver>,
) -> Result<Option<Step>, String> {
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
    let mut written: Vec<WrittenFx> = Vec::new();
    while let Some(c) = rest.chars().next() {
        rest = &rest[c.len_utf8()..];
        match c {
            '{' => {
                let end = rest
                    .find('}')
                    .ok_or("falta } al final de los efectos del paso ({...})")?;
                written.push(written_fx(&rest[..end])?);
                rest = &rest[end + 1..];
            }
            '}' => return Err("} sin su { (los efectos del paso van entre llaves)".into()),
            '\'' => {
                once(&mut seen_reverse, "reverse (')")?;
                step.reverse = true;
            }
            '%' => {
                if pitches > 0 {
                    return Err(
                        "% (pitch sin cambiar la duración) va en vez del primer ^, no detrás"
                            .into(),
                    );
                }
                let (value, after) = number(rest, true).ok_or("falta el número después de %")?;
                step.semitones = value;
                step.keep = true;
                pitches += 1;
                rest = after;
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
                     (efectos: ' reverse, ^n pitch, %n pitch sin cambiar la duración, \
                     @n volumen, rN retrigger, ~Hz corte, {{efecto=valor}})"
                ));
            }
        }
    }
    if !written.is_empty() {
        let resolve = fx.ok_or(
            "los efectos de un paso ({...}) solo se entienden desde el script (steps(\"...\"))",
        )?;
        step.fx = Some(resolve(&written)?);
    }
    Ok(Some(step))
}

/// Lee lo de dentro de unas llaves: `lp=800,q=4` (con espacios, si los
/// hay, alrededor de cada cosa).
fn written_fx(inner: &str) -> Result<WrittenFx, String> {
    let mut items = inner.split(',').map(str::trim);
    let head = items.next().unwrap_or("");
    if head.is_empty() {
        return Err("efecto vacío: {efecto=valor} (por ejemplo {lp=800})".into());
    }
    let (name, main) = match head.split_once('=') {
        Some((name, value)) => (name.trim(), Some(fx_value(name.trim(), value.trim())?)),
        None => (head, None),
    };
    if !is_word(name) {
        return Err(format!(
            "\"{name}\" no es un nombre de efecto (letras, números y _)"
        ));
    }
    let mut params = Vec::new();
    for item in items {
        let (key, value) = item
            .split_once('=')
            .ok_or_else(|| format!("{{{name}}}: \"{item}\" va como clave=valor"))?;
        let key = key.trim();
        if !is_word(key) {
            return Err(format!(
                "{{{name}}}: \"{key}\" no es un nombre de parámetro"
            ));
        }
        if params.iter().any(|(k, _)| k == key) {
            return Err(format!("{{{name}}}: {key} repetido"));
        }
        params.push((key.to_string(), fx_value(key, value.trim())?));
    }
    Ok(WrittenFx {
        name: name.to_string(),
        main,
        params,
    })
}

fn is_word(text: &str) -> bool {
    !text.is_empty() && text.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
}

/// Un valor: número, palabra o rampa (`a>b` o `a>b:pasos`).
fn fx_value(key: &str, text: &str) -> Result<FxValue, String> {
    if text.is_empty() {
        return Err(format!("falta el valor de {key} (tras =)"));
    }
    if let Some((from, to)) = text.split_once('>') {
        let (to, steps) = match to.split_once(':') {
            Some((to, steps)) => (to, Some(steps)),
            None => (to, None),
        };
        let number = |t: &str| match self::number(t.trim(), true) {
            Some((v, "")) => Some(v),
            _ => None,
        };
        let (Some(from), Some(to)) = (number(from), number(to)) else {
            return Err(format!(
                "{key}={text}: una rampa es desde>hasta (o desde>hasta:pasos), con números"
            ));
        };
        let steps = match steps {
            None => None,
            Some(steps) => match number(steps) {
                Some(n) if n > 0.0 => Some(n as f64),
                _ => {
                    return Err(format!(
                        "{key}={text}: tras : van los pasos que dura la rampa (más de 0)"
                    ));
                }
            },
        };
        return Ok(FxValue::Ramp { from, to, steps });
    }
    match number(text, true) {
        Some((v, "")) => Ok(FxValue::Number(v)),
        _ if is_word(text) => Ok(FxValue::Word(text.to_string())),
        _ => Err(format!(
            "{key}={text}: el valor es un número, una palabra o desde>hasta"
        )),
    }
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
