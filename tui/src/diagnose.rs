//! Diagnóstico en vivo de la notación de `steps("...")`: mientras se
//! escribe, sin cargar el script. Usa el mismo lector de la notación que el
//! motor ([`engine::notation`]) y los mismos rangos (ganancia, pitch y
//! retrigger) que valida el script al cargar.

use std::ops::Range;

use engine::notation::{parse_step, parse_steps};
use engine::{MAX_GAIN, MAX_RETRIG, MAX_SEMITONES};

/// Un problema en una línea del editor.
#[derive(Debug, Clone, PartialEq)]
pub struct Diagnostic {
    /// Fila (desde 0).
    pub row: usize,
    /// Columnas (en caracteres) que se subrayan.
    pub cols: Range<usize>,
    pub message: String,
}

/// Diagnósticos de todas las cadenas de `steps("...")` del texto (también
/// las de `c:steps` dentro de `every`). Una cadena sin cerrar se mira
/// hasta el final de la línea.
pub fn steps_diagnostics(lines: &[String]) -> Vec<Diagnostic> {
    let mut out = Vec::new();
    for (row, line) in lines.iter().enumerate() {
        let chars: Vec<char> = line.chars().collect();
        let code_end = comment_start(&chars);
        let mut i = 0;
        while let Some(at) = find(&chars[..code_end], "steps", i) {
            i = at + 5;
            // Solo como método: `:steps` (no `mysteps`, ni dentro de una cadena).
            if at == 0 || chars[at - 1] != ':' || in_string(&chars[..at]) {
                continue;
            }
            let mut j = i;
            while j < chars.len() && chars[j] == ' ' {
                j += 1;
            }
            if chars.get(j) != Some(&'(') {
                continue;
            }
            j += 1;
            while j < chars.len() && chars[j] == ' ' {
                j += 1;
            }
            let Some(&quote) = chars.get(j).filter(|c| **c == '"' || **c == '\'') else {
                continue;
            };
            let start = j + 1;
            let end = (start..chars.len())
                .find(|&k| chars[k] == quote)
                .unwrap_or(chars.len());
            let content: String = chars[start..end].iter().collect();
            if let Some((cols, message)) = check(&content) {
                let cols = if cols.is_empty() {
                    // Sin pasos: se subrayan las comillas.
                    start.saturating_sub(1)..(end + 1).min(chars.len())
                } else {
                    start + cols.start..start + cols.end
                };
                out.push(Diagnostic {
                    row,
                    cols,
                    message: format!("steps: {message}"),
                });
            }
            i = end;
        }
    }
    out
}

/// Comprueba un patrón: primero cada paso (así, mientras se escribe, el
/// error de un paso se ve aunque falten pasos), luego cuántos hay y luego
/// los rangos. Devuelve las columnas (dentro del patrón) que se subrayan y
/// el mensaje.
fn check(content: &str) -> Option<(Range<usize>, String)> {
    let tokens = tokens(content);
    let text = |i: usize| -> String {
        content
            .chars()
            .skip(tokens[i].start)
            .take(tokens[i].len())
            .collect()
    };
    for (i, cols) in tokens.iter().enumerate() {
        let token = text(i);
        if let Err(message) = parse_step(&token) {
            return Some((
                cols.clone(),
                format!("paso {} (\"{token}\"): {message}", i + 1),
            ));
        }
    }
    let steps = match parse_steps(content) {
        Ok(steps) => steps,
        Err(e) => {
            let cols = match (tokens.first(), tokens.last()) {
                (Some(a), Some(b)) => a.start..b.end,
                _ => 0..0,
            };
            return Some((cols, e.to_string()));
        }
    };
    steps.iter().enumerate().find_map(|(i, step)| {
        let step = step.as_ref()?;
        let problem = if !(0.0..=MAX_GAIN).contains(&step.gain) {
            "ganancia fuera de rango (0 a 4)"
        } else if !(-MAX_SEMITONES..=MAX_SEMITONES).contains(&step.semitones) {
            "pitch fuera de rango (-36 a 36 semitonos)"
        } else if !(1..=MAX_RETRIG).contains(&step.retrig) {
            "retrigger fuera de rango (1 a 16)"
        } else {
            return None;
        };
        Some((
            tokens[i].clone(),
            format!("paso {} (\"{}\"): {problem}", i + 1, text(i)),
        ))
    })
}

/// Columnas (en caracteres) de cada paso separado por espacios.
fn tokens(content: &str) -> Vec<Range<usize>> {
    let mut out = Vec::new();
    let mut start = None;
    for (i, c) in content.chars().chain([' ']).enumerate() {
        match (c.is_whitespace(), start) {
            (false, None) => start = Some(i),
            (true, Some(s)) => {
                out.push(s..i);
                start = None;
            }
            _ => {}
        }
    }
    out
}

/// Primera aparición de `needle` desde `from`.
fn find(chars: &[char], needle: &str, from: usize) -> Option<usize> {
    let needle: Vec<char> = needle.chars().collect();
    (from..chars.len().saturating_sub(needle.len() - 1)).find(|&i| chars[i..].starts_with(&needle))
}

/// ¿Termina `chars` dentro de una cadena?
fn in_string(chars: &[char]) -> bool {
    let mut quote = None;
    let mut escaped = false;
    for &c in chars {
        match quote {
            Some(_) if escaped => escaped = false,
            Some(_) if c == '\\' => escaped = true,
            Some(q) if c == q => quote = None,
            None if c == '"' || c == '\'' => quote = Some(c),
            _ => {}
        }
    }
    quote.is_some()
}

/// Columna donde empieza un comentario `--` (fuera de cadenas), o el final.
fn comment_start(chars: &[char]) -> usize {
    (0..chars.len())
        .find(|&i| chars[i..].starts_with(&['-', '-']) && !in_string(&chars[..i]))
        .unwrap_or(chars.len())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn diagnose(text: &str) -> Vec<Diagnostic> {
        let lines: Vec<String> = text.lines().map(String::from).collect();
        steps_diagnostics(&lines)
    }

    #[test]
    fn valid_patterns_have_no_diagnostics() {
        assert!(
            diagnose(r#"track("d", a):steps("1 . 2 .  3 . 4 .  5 6 2 .  7 . 8 .")"#).is_empty()
        );
        assert!(
            diagnose(
                r#"x:every(2, function(c) c:steps("x . . . . . . . . . . . . . . 3'^7@0.5r2") end)"#
            )
            .is_empty()
        );
    }

    #[test]
    fn underlines_the_wrong_step() {
        let line = r#"  :steps("1 . 2 .  3 . 4 .  5 6 5^ .  7 . 8 .")"#;
        let d = diagnose(line);
        assert_eq!(d.len(), 1);
        let underlined: String = line
            .chars()
            .skip(d[0].cols.start)
            .take(d[0].cols.len())
            .collect();
        assert_eq!(underlined, "5^");
        assert_eq!(
            d[0].message,
            "steps: paso 11 (\"5^\"): falta el número después de ^"
        );
        assert_eq!(d[0].row, 0);
    }

    #[test]
    fn ranges_are_checked_like_on_load() {
        let line = r#":steps("1 . . .  . . . .  . . . .  . . . 1@5")"#;
        let d = diagnose(line);
        assert_eq!(
            d[0].message,
            "steps: paso 16 (\"1@5\"): ganancia fuera de rango (0 a 4)"
        );
        let d = diagnose(r#":steps("1^40 . . .  . . . .  . . . .  . . . .")"#);
        assert!(d[0].message.contains("pitch fuera de rango"));
        assert_eq!(d[0].cols, 8..12);
        let d = diagnose(r#":steps("1r17 . . .  . . . .  . . . .  . . . .")"#);
        assert!(d[0].message.contains("retrigger fuera de rango"));
    }

    #[test]
    fn wrong_step_counts_underline_the_whole_pattern() {
        let line = r#"track("d", a):steps("1 . 2 .")"#;
        let d = diagnose(line);
        assert_eq!(d[0].message, "steps: el patrón necesita 16 pasos, tiene 4");
        let underlined: String = line
            .chars()
            .skip(d[0].cols.start)
            .take(d[0].cols.len())
            .collect();
        assert_eq!(underlined, "1 . 2 .");
        // Vacío: las comillas.
        let line = r#":steps("")"#;
        assert_eq!(diagnose(line)[0].cols, 7..9);
    }

    #[test]
    fn unclosed_strings_comments_and_other_names_are_handled() {
        // Escribiendo: aún sin cerrar la cadena.
        let d = diagnose(r#":steps("1 . 2 . z"#);
        assert_eq!(
            d[0].message,
            "steps: paso 5 (\"z\"): debe empezar por un número de corte (desde 1), \"x\" o \".\""
        );
        assert!(diagnose(r#"-- :steps("1 . 2")"#).is_empty());
        assert!(diagnose(r#"print(":steps('1')")"#).is_empty());
        assert!(diagnose(r#"x:mysteps("1")"#).is_empty());
        // Varias en una línea, y la fila correcta.
        let d = diagnose("bpm(1)\na:steps(\"1\") b:steps(\"2\")");
        assert_eq!(d.len(), 2);
        assert_eq!(d[1].row, 1);
    }
}
