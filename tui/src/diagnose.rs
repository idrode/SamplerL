//! Diagnóstico en vivo de la notación de `steps("...")`: mientras se
//! escribe, sin cargar el script. La longitud que se pide es la de la
//! pista ([`track_lengths`]): la de `:length(n)` en la misma cadena de
//! `track(...)` o en la definición de esa pista, o 16. Usa el mismo lector de la notación que el
//! motor ([`engine::notation`]) y los mismos rangos (ganancia, pitch y
//! retrigger) que valida el script al cargar.

use std::ops::Range;

use engine::notation::{parse_pattern_labeled, parse_step_labeled};
use engine::{MAX_GAIN, MAX_RETRIG, MAX_SEMITONES, STEPS};

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
/// hasta el final de la línea. `labels` son las etiquetas de corte
/// conocidas (de cualquier pista: si una no es de esa pista, lo dirá la
/// carga).
pub fn steps_diagnostics(lines: &[String], labels: &[String]) -> Vec<Diagnostic> {
    let labels: Vec<Option<String>> = labels.iter().cloned().map(Some).collect();
    let lengths = track_lengths(lines);
    let mut out = Vec::new();
    let mut line_start = 0;
    for (row, line) in lines.iter().enumerate() {
        let offset = line_start;
        line_start += line.chars().count() + 1;
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
            let length = lengths.at(offset + at);
            if let Some((cols, message)) = check(&content, &labels, length) {
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
/// error de un paso se ve aunque falten pasos), luego cuántos hay (si se
/// sabe la longitud de la pista) y luego los rangos. Devuelve las columnas
/// (dentro del patrón) que se subrayan y el mensaje.
fn check(
    content: &str,
    labels: &[Option<String>],
    length: Option<usize>,
) -> Option<(Range<usize>, String)> {
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
        if let Err(message) = parse_step_labeled(&token, labels) {
            return Some((
                cols.clone(),
                format!("paso {} (\"{token}\"): {message}", i + 1),
            ));
        }
    }
    let length = length.unwrap_or(tokens.len().max(1));
    let steps = match parse_pattern_labeled(content, length, labels) {
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
        } else if step.lock.is_some_and(|hz| !(20.0..=20_000.0).contains(&hz)) {
            "corte (~) fuera de rango (20 a 20000 Hz)"
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

/// Longitud de cada cadena de `track(...)` del texto (de un `track(` al
/// siguiente), para saber cuántos pasos pide cada `steps`.
pub struct Lengths {
    /// Desde dónde (en caracteres del texto, con un salto por línea) vale
    /// cada longitud; `None`: no se sabe (`:length(n)` con algo que no es
    /// un número).
    chains: Vec<(usize, Option<usize>)>,
}

impl Lengths {
    /// Longitud de la pista en la posición `at` del texto (16 antes del
    /// primer `track(`).
    pub fn at(&self, at: usize) -> Option<usize> {
        self.chains
            .iter()
            .rev()
            .find(|(start, _)| *start <= at)
            .map_or(Some(STEPS), |(_, length)| *length)
    }
}

/// Longitudes de las cadenas de `track(...)`: la de su `:length(n)` o, si
/// no tiene, la de otra cadena de la misma pista (`track("d")` recupera
/// una pista definida antes), o 16.
pub fn track_lengths(lines: &[String]) -> Lengths {
    let text: Vec<char> = lines.join("\n").chars().collect();
    let code = mask(&text);
    let mut starts = Vec::new();
    let mut i = 0;
    while let Some(at) = find(&code, "track", i) {
        i = at + 5;
        let ident = |c: char| c.is_alphanumeric() || c == '_';
        let before = at.checked_sub(1).map(|j| code[j]);
        if before.is_some_and(|c| ident(c) || c == '.' || c == ':') {
            continue;
        }
        let mut j = i;
        while code.get(j) == Some(&' ') {
            j += 1;
        }
        if code.get(j) == Some(&'(') {
            starts.push((at, j + 1));
        }
    }
    // Nombre (primer argumento literal) y `:length(n)` de cada cadena.
    let mut chains: Vec<(usize, Option<String>, Option<Option<usize>>)> = Vec::new();
    for (k, &(at, open)) in starts.iter().enumerate() {
        let end = starts.get(k + 1).map_or(code.len(), |s| s.0);
        let mut j = open;
        while text.get(j) == Some(&' ') {
            j += 1;
        }
        let name = text
            .get(j)
            .filter(|c| **c == '"' || **c == '\'')
            .and_then(|q| {
                let close = (j + 1..end).find(|&e| text[e] == *q)?;
                Some(text[j + 1..close].iter().collect::<String>())
            });
        let mut length = None;
        let mut from = open;
        while let Some(l) = find(&code[..end], ":length", from) {
            from = l + 7;
            let rest: String = code[from..end].iter().collect();
            let rest = rest.trim_start();
            let Some(args) = rest.strip_prefix('(') else {
                continue;
            };
            let inner = args.split(')').next().unwrap_or("").trim();
            length = Some(inner.parse::<usize>().ok());
        }
        chains.push((at, name, length));
    }
    let chains = chains
        .iter()
        .map(|(at, name, length)| {
            let length = match length {
                Some(length) => *length,
                // La de la cadena que define esa pista con :length, si hay.
                None => chains
                    .iter()
                    .filter(|(_, other, l)| other.is_some() && other == name && l.is_some())
                    .map(|(_, _, l)| l.unwrap())
                    .next()
                    .unwrap_or(Some(STEPS)),
            };
            (*at, length)
        })
        .collect();
    Lengths { chains }
}

/// El texto con las cadenas y los comentarios cambiados por espacios (los
/// saltos de línea se conservan), para buscar llamadas solo en el código.
fn mask(text: &[char]) -> Vec<char> {
    let mut out = text.to_vec();
    let mut i = 0;
    let blank = |out: &mut Vec<char>, range: Range<usize>| {
        for c in &mut out[range] {
            if *c != '\n' {
                *c = ' ';
            }
        }
    };
    while i < text.len() {
        let c = text[i];
        if text[i..].starts_with(&['-', '-']) {
            // Comentario largo `--[[ ... ]]` o hasta el final de la línea.
            let end = if text[i + 2..].starts_with(&['[', '[']) {
                find(text, "]]", i + 4).map_or(text.len(), |e| e + 2)
            } else {
                (i..text.len())
                    .find(|&e| text[e] == '\n')
                    .unwrap_or(text.len())
            };
            blank(&mut out, i..end);
            i = end;
        } else if c == '"' || c == '\'' {
            let mut e = i + 1;
            while e < text.len() && text[e] != c && text[e] != '\n' {
                if text[e] == '\\' {
                    e += 1;
                }
                e += 1;
            }
            let end = (e + 1).min(text.len());
            blank(&mut out, i..end);
            i = end;
        } else if text[i..].starts_with(&['[', '[']) {
            let end = find(text, "]]", i + 2).map_or(text.len(), |e| e + 2);
            blank(&mut out, i..end);
            i = end;
        } else {
            i += 1;
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
        diagnose_with(text, &[])
    }

    fn diagnose_with(text: &str, labels: &[&str]) -> Vec<Diagnostic> {
        let lines: Vec<String> = text.lines().map(String::from).collect();
        let labels: Vec<String> = labels.iter().map(|l| l.to_string()).collect();
        steps_diagnostics(&lines, &labels)
    }

    #[test]
    fn labels_are_known_and_unknown_ones_underlined() {
        let line = r#":steps("kick . snare .  kick^7@0.5 . snre .  . . . .  . . . .")"#;
        let d = diagnose_with(line, &["kick", "snare"]);
        assert_eq!(d.len(), 1);
        let underlined: String = line
            .chars()
            .skip(d[0].cols.start)
            .take(d[0].cols.len())
            .collect();
        assert_eq!(underlined, "snre");
        assert_eq!(
            d[0].message,
            "steps: paso 7 (\"snre\"): la etiqueta \"snre\" no existe (hay: kick, snare)"
        );
        let fixed = line.replace("snre", "snare");
        assert!(diagnose_with(&fixed, &["kick", "snare"]).is_empty());
        // Rangos también con etiquetas.
        let d = diagnose_with(
            r#":steps("kick@9 . . .  . . . .  . . . .  . . . .")"#,
            &["kick"],
        );
        assert!(
            d[0].message.contains("ganancia fuera de rango"),
            "{}",
            d[0].message
        );
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
    fn the_step_count_follows_the_track_length() {
        // Con :length(12) en la cadena, pide 12.
        let ok = "track(\"d\", a):length(12):steps(\"1 . . .  . . . .  . . . .\")";
        assert!(diagnose(ok).is_empty(), "{:?}", diagnose(ok));
        let d = diagnose("track(\"d\", a)\n  :length(12)\n  :steps(\"1 . . .\")");
        assert_eq!(d[0].message, "steps: el patrón necesita 12 pasos, tiene 4");
        assert_eq!(d[0].row, 2);
        // La siguiente pista vuelve a 16.
        let two = "track(\"a\", a):length(3):steps(\"1 . .\")\n\
                   track(\"b\", a):steps(\"1 . .\")";
        let d = diagnose(two);
        assert_eq!(d.len(), 1);
        assert_eq!(d[0].row, 1);
        assert!(d[0].message.contains("necesita 16 pasos"));
        // every y track("nombre") usan la longitud de la pista.
        let laps = "track(\"a\", a):length(6):steps(\"1 . . . . .\")\n\
                    track(\"b\", a):steps(\"1 . . .  . . . .  . . . .  . . . .\")\n\
                    track(\"a\"):every(2, function(c) c:steps(\"1 . . . . 1\") end)";
        assert!(diagnose(laps).is_empty(), "{:?}", diagnose(laps));
        // Con una longitud que no es un número, no se cuentan los pasos
        // (pero se revisa cada uno).
        let unknown = "track(\"a\", a):length(n):steps(\"1 . ?\")";
        let d = diagnose(unknown);
        assert_eq!(d.len(), 1);
        assert!(d[0].message.contains("paso 3"), "{}", d[0].message);
        assert!(diagnose("track(\"a\", a):length(n):steps(\"1 .\")").is_empty());
        // `track(` en un comentario o una cadena no cuenta.
        let masked = "track(\"a\", a):length(3):steps(\"1 . .\")\n\
                      -- track(\"b\", a)\n\
                      print(\"track(\")\n\
                      track(\"a\"):every(2, function(c) c:steps(\"1 1 1\") end)";
        assert!(diagnose(masked).is_empty(), "{:?}", diagnose(masked));
    }

    #[test]
    fn unclosed_strings_comments_and_other_names_are_handled() {
        // Escribiendo: aún sin cerrar la cadena.
        let d = diagnose(r#":steps("1 . 2 . ?"#);
        assert_eq!(
            d[0].message,
            "steps: paso 5 (\"?\"): debe empezar por un número de corte (desde 1), \
             una etiqueta, \"x\" o \".\""
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
