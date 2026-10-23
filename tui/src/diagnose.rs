//! Diagnóstico en vivo de la notación de `steps("...")`, de las notas de
//! `melody("...")` y de los nombres de `:fx(...)` (el efecto y, en
//! `cortes:fx(cuál, ...)`, las etiquetas de corte): mientras se escribe,
//! sin cargar el script. La longitud que se pide es la de la
//! pista ([`track_lengths`]): la de `:length(n)` en la misma cadena de
//! `track(...)` o en la definición de esa pista, o 16. Usa el mismo lector de la notación que el
//! motor ([`engine::notation`]) y los mismos rangos (ganancia, pitch y
//! retrigger) que valida el script al cargar.

use std::ops::Range;

use engine::notation::{parse_pattern_with, parse_step_with};
use engine::{MAX_GAIN, MAX_RETRIG, MAX_SEMITONES, STEPS};
use script::api;

/// Un problema en una línea del editor.
#[derive(Debug, Clone, PartialEq)]
pub struct Diagnostic {
    /// Fila (desde 0).
    pub row: usize,
    /// Columnas (en caracteres) que se subrayan.
    pub cols: Range<usize>,
    pub message: String,
}

/// Diagnósticos de todas las cadenas de `steps("...")` y `melody("...")`
/// del texto (también las de `c:steps`/`c:melody` dentro de `every`). Una
/// cadena sin cerrar se mira hasta el final de la línea. `labels` son las
/// etiquetas de corte conocidas (de cualquier pista: si una no es de esa
/// pista, lo dirá la carga). En `melody` no se sabe la raíz, así que lo
/// lejos que queda cada nota de ella (±36 semitonos) lo dice la carga.
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
        for method in ["steps", "melody"] {
            let mut i = 0;
            while let Some(at) = find(&chars[..code_end], method, i) {
                i = at + method.len();
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
                let problem = if method == "melody" {
                    check_melody(&content, length)
                } else {
                    check(&content, &labels, length)
                };
                if let Some((cols, message)) = problem {
                    let cols = if cols.is_empty() {
                        // Sin pasos: se subrayan las comillas.
                        start.saturating_sub(1)..(end + 1).min(chars.len())
                    } else {
                        start + cols.start..start + cols.end
                    };
                    out.push(Diagnostic {
                        row,
                        cols,
                        message: format!("{method}: {message}"),
                    });
                }
                i = end;
            }
        }
        let mut i = 0;
        while let Some(at) = find(&chars[..code_end], ":fx", i) {
            i = at + 3;
            if in_string(&chars[..at]) {
                continue;
            }
            for (cols, message) in check_fx(&chars[..code_end], i, labels_known(&labels)) {
                out.push(Diagnostic { row, cols, message });
            }
        }
    }
    out.sort_by_key(|d| (d.row, d.cols.start));
    out
}

fn labels_known(labels: &[Option<String>]) -> impl Fn(&str) -> bool + '_ {
    |name| labels.iter().flatten().any(|l| l == name)
}

/// Un argumento de `:fx(...)` tal como se lee en la línea.
enum Arg {
    /// Cadena: su texto y las columnas de dentro de las comillas.
    Text(String, Range<usize>),
    /// Lista `{ ... }`: sus cadenas.
    List(Vec<(String, Range<usize>)>),
    /// Otra cosa (un número, una tabla de parámetros, una variable).
    Other,
}

/// Lee una cadena que empieza en `chars[at]` (con su comilla); `None` si
/// no se cierra en la línea.
fn read_string(chars: &[char], at: usize) -> Option<(String, Range<usize>, usize)> {
    let quote = *chars.get(at).filter(|c| **c == '"' || **c == '\'')?;
    let end = (at + 1..chars.len()).find(|&k| chars[k] == quote)?;
    Some((chars[at + 1..end].iter().collect(), at + 1..end, end + 1))
}

/// Los dos primeros argumentos de la llamada que abre en `chars[open]`
/// (`(`), cada uno con dónde acaba.
fn fx_args(chars: &[char], open: usize) -> Vec<Arg> {
    let skip = |mut k: usize| {
        while chars.get(k) == Some(&' ') {
            k += 1;
        }
        k
    };
    let mut args = Vec::new();
    let mut k = skip(open + 1);
    while args.len() < 2 {
        let (arg, next) = match chars.get(k) {
            Some('"' | '\'') => match read_string(chars, k) {
                Some((text, cols, next)) => (Arg::Text(text, cols), next),
                None => break,
            },
            Some('{') => {
                let Some(close) = (k..chars.len()).find(|&j| chars[j] == '}') else {
                    break;
                };
                let mut items = Vec::new();
                let mut j = k + 1;
                while j < close {
                    if let Some((text, cols, next)) = read_string(&chars[..close], j)
                        .filter(|_| chars[j] == '"' || chars[j] == '\'')
                    {
                        items.push((text, cols));
                        j = next;
                    } else {
                        j += 1;
                    }
                }
                // Una lista con `=` es la tabla de parámetros.
                if chars[k..close].contains(&'=') {
                    (Arg::Other, close + 1)
                } else {
                    (Arg::List(items), close + 1)
                }
            }
            Some(_) => {
                let end = (k..chars.len())
                    .find(|&j| chars[j] == ',' || chars[j] == ')')
                    .unwrap_or(chars.len());
                (Arg::Other, end)
            }
            None => break,
        };
        args.push(arg);
        k = skip(next);
        if chars.get(k) != Some(&',') {
            break;
        }
        k = skip(k + 1);
    }
    args
}

/// Problemas de una llamada `:fx(...)` cuyo nombre acaba en `chars[after]`:
/// un efecto que no existe y, en `cortes:fx(cuál, "efecto", ...)`, una
/// etiqueta que no conoce ninguna pista ni el texto.
fn check_fx(
    chars: &[char],
    after: usize,
    known: impl Fn(&str) -> bool,
) -> Vec<(Range<usize>, String)> {
    let mut k = after;
    while chars.get(k) == Some(&' ') {
        k += 1;
    }
    if chars.get(k) != Some(&'(') {
        return Vec::new();
    }
    let args = fx_args(chars, k);
    let mut out = Vec::new();
    let effect = |name: &str, cols: Range<usize>, out: &mut Vec<(Range<usize>, String)>| {
        if api::effect(name).is_none() {
            let all: Vec<&str> = api::EFFECTS.iter().map(|e| e.name).collect();
            out.push((
                cols,
                format!("fx: el efecto \"{name}\" no existe ({})", all.join(", ")),
            ));
        }
    };
    let label = |name: &str, cols: Range<usize>, out: &mut Vec<(Range<usize>, String)>| {
        if !known(name) {
            out.push((
                cols,
                format!(
                    "fx: ningún corte se llama \"{name}\" (ponle la etiqueta con cortes:name(i, \"{name}\"))"
                ),
            ));
        }
    };
    match args.as_slice() {
        // `cortes:fx("etiqueta", "efecto", ...)`.
        [Arg::Text(which, which_cols), Arg::Text(name, cols), ..] => {
            label(which, which_cols.clone(), &mut out);
            effect(name, cols.clone(), &mut out);
        }
        [Arg::List(items), rest @ ..] => {
            for (which, cols) in items {
                label(which, cols.clone(), &mut out);
            }
            if let [Arg::Text(name, cols), ..] = rest {
                effect(name, cols.clone(), &mut out);
            }
        }
        // `cortes:fx(3, "efecto", ...)`.
        [Arg::Other, Arg::Text(name, cols), ..] => effect(name, cols.clone(), &mut out),
        // `pista:fx("efecto", { ... })` (o mientras se escribe).
        [Arg::Text(name, cols), ..] if !name.is_empty() => effect(name, cols.clone(), &mut out),
        _ => {}
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
    let steps: Vec<Token> = tokens(content)
        .into_iter()
        .map(|cols| {
            let text: String = content.chars().skip(cols.start).take(cols.len()).collect();
            Token {
                cols,
                notation: text.clone(),
                text,
            }
        })
        .collect();
    for (i, token) in steps.iter().enumerate() {
        if let Err(message) = parse_step_with(&token.text, labels, Some(&script::step_fx::resolve))
        {
            return Some((
                token.cols.clone(),
                format!("paso {} (\"{}\"): {message}", i + 1, token.text),
            ));
        }
    }
    check_pattern(&steps, labels, length, true)
}

/// Como [`check`], para las notas de `melody("...")`: cada nota (o
/// acorde) se lee y se pasa a pasos, y en ellos se comprueba cuántos hay,
/// los `_` y los rangos (menos el pitch, que depende de la raíz).
fn check_melody(content: &str, length: Option<usize>) -> Option<(Range<usize>, String)> {
    let mut steps = Vec::new();
    for (i, cols) in tokens(content).into_iter().enumerate() {
        let text: String = content.chars().skip(cols.start).take(cols.len()).collect();
        match script::patterns::note_step(&text, None) {
            Ok(notation) => steps.push(Token {
                cols,
                text,
                notation,
            }),
            Err(message) => {
                return Some((cols, format!("paso {} (\"{text}\"): {message}", i + 1)));
            }
        }
    }
    check_pattern(&steps, &[], length, false)
}

/// Un paso del patrón: dónde está, cómo se escribió y en mini-notación.
struct Token {
    cols: Range<usize>,
    text: String,
    notation: String,
}

/// Cuántos pasos hay (con la longitud de la pista, si se sabe), los `_` y
/// los rangos (el pitch solo si `pitch`).
fn check_pattern(
    steps: &[Token],
    labels: &[Option<String>],
    length: Option<usize>,
    pitch: bool,
) -> Option<(Range<usize>, String)> {
    let length = length.unwrap_or(steps.len().max(1));
    let notation: Vec<&str> = steps.iter().map(|t| t.notation.as_str()).collect();
    let parsed = match parse_pattern_with(
        &notation.join(" "),
        length,
        labels,
        Some(&script::step_fx::resolve),
    ) {
        Ok(parsed) => parsed,
        Err(e) => {
            // El paso del error (un `_` sin golpe), o todo el patrón.
            let cols = match e.step.and_then(|n| steps.get(n - 1)) {
                Some(token) => token.cols.clone(),
                None => match (steps.first(), steps.last()) {
                    (Some(a), Some(b)) => a.cols.start..b.cols.end,
                    _ => 0..0,
                },
            };
            let message = match e.step.and_then(|n| steps.get(n - 1)) {
                Some(token) => format!(
                    "paso {} (\"{}\"): {}",
                    e.step.unwrap_or(0),
                    token.text,
                    e.message
                ),
                None => e.to_string(),
            };
            return Some((cols, message));
        }
    };
    parsed.iter().enumerate().find_map(|(i, step)| {
        let step = step.as_ref()?;
        let problem = if !(0.0..=MAX_GAIN).contains(&step.gain) {
            "ganancia fuera de rango (0 a 4)"
        } else if pitch
            && step
                .notes()
                .any(|st| !(-MAX_SEMITONES..=MAX_SEMITONES).contains(&st))
        {
            "pitch fuera de rango (-36 a 36 semitonos)"
        } else if !(1..=MAX_RETRIG).contains(&step.retrig) {
            "retrigger fuera de rango (1 a 16)"
        } else if step.lock.is_some_and(|hz| !(20.0..=20_000.0).contains(&hz)) {
            "corte (~) fuera de rango (20 a 20000 Hz)"
        } else {
            return None;
        };
        Some((
            steps[i].cols.clone(),
            format!("paso {} (\"{}\"): {problem}", i + 1, steps[i].text),
        ))
    })
}

/// Columnas (en caracteres) de cada paso separado por espacios (los de
/// dentro de las llaves de los efectos de un paso no separan).
fn tokens(content: &str) -> Vec<Range<usize>> {
    engine::notation::step_tokens(content)
        .into_iter()
        .map(|(a, b)| a..b)
        .collect()
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
    fn fx_underlines_unknown_effects_and_slice_labels() {
        let under = |line: &str, d: &Diagnostic| -> String {
            line.chars().skip(d.cols.start).take(d.cols.len()).collect()
        };
        let known = ["kick", "snare"];
        let ok = [
            r#"amen:fx("snare", "bitcrush", { bits = 6 })"#,
            r#"amen:fx({ "kick", 3 }, "gain", { level = 0.5 })"#,
            r#"amen:fx(3, "lowpass", { cutoff = 300 })"#,
            r#"track("d", amen):fx("filter", { type = "lowpass", cutoff = 300 })"#,
            r#"track("d", amen):fx("bitcrusher")"#,
            r#"t:fx(""#,
            r#"x = "a:fx('nada')" -- :fx("tampoco")"#,
        ];
        for line in ok {
            assert!(diagnose_with(line, &known).is_empty(), "{line}");
        }
        let cases = [
            (r#"amen:fx("snre", "bitcrush", {})"#, vec!["snre"]),
            (r#"amen:fx("snare", "bitcrsh", {})"#, vec!["bitcrsh"]),
            (
                r#"amen:fx({ "kik", 2, "snare" }, "chorus", {})"#,
                vec!["kik", "chorus"],
            ),
            (r#"amen:fx(4, "reverb", {})"#, vec!["reverb"]),
            (
                r#"track("d", amen):fx("lowpas", { cutoff = 3 })"#,
                vec!["lowpas"],
            ),
        ];
        for (line, expected) in cases {
            let d = diagnose_with(line, &known);
            let got: Vec<String> = d.iter().map(|d| under(line, d)).collect();
            assert_eq!(got, expected, "{line}");
        }
        let d = diagnose_with(r#"amen:fx("tom", "gain", {})"#, &known);
        assert_eq!(
            d[0].message,
            "fx: ningún corte se llama \"tom\" (ponle la etiqueta con cortes:name(i, \"tom\"))"
        );
        // Las etiquetas del texto también cuentan (las pasa la interfaz).
        let d = diagnose_with(r#"amen:fx("kick", "gian", {})"#, &known);
        assert!(
            d[0].message
                .starts_with("fx: el efecto \"gian\" no existe (filter,")
        );
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
        // `%`, igual que `^`; y mal puesto, subrayado.
        let d = diagnose(r#":steps("1%40 . . .  . . . .  . . . .  . . . .")"#);
        assert!(d[0].message.contains("pitch fuera de rango"));
        let d = diagnose(r#":steps("1 1^2%7 . .  . . . .  . . . .  . . . .")"#);
        assert!(
            d[0].message.contains("en vez del primer ^"),
            "{}",
            d[0].message
        );
        assert_eq!(d[0].cols, 10..15);
        assert!(diagnose(r#":steps("1%7 1%-12 . .  . . . .  . . . .  . . . .")"#).is_empty());
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

    fn underlined(line: &str, d: &Diagnostic) -> String {
        line.chars().skip(d.cols.start).take(d.cols.len()).collect()
    }

    #[test]
    fn melody_underlines_the_wrong_note() {
        let line = r#"track("lead", piano):melody("c e g e  c+e h _  . . . .  . . . .")"#;
        let d = diagnose(line);
        assert_eq!(d.len(), 1);
        assert_eq!(underlined(line, &d[0]), "h");
        assert_eq!(
            d[0].message,
            "melody: paso 6 (\"h\"): debe ser una nota (c, e, g#, bb3...), un acorde (c+e+g), \".\" o \"_\""
        );
        // Acorde de más, ^ (el tono lo da la nota), sufijo roto y rango.
        for (bad, why) in [
            ("c+e+g+b+d", "como mucho 4 notas"),
            ("e^7", "sin ^"),
            ("g@", "falta el número después de @"),
            ("a@9", "ganancia fuera de rango"),
            ("cr40", "retrigger fuera de rango"),
        ] {
            let line = format!(r#"t:melody("c . {bad} .  . . . .  . . . .  . . . .")"#);
            let d = diagnose(&line);
            assert_eq!(d.len(), 1, "{bad}");
            assert_eq!(underlined(&line, &d[0]), bad);
            assert!(d[0].message.contains(why), "{bad}: {}", d[0].message);
        }
    }

    #[test]
    fn melody_checks_holds_and_the_number_of_steps() {
        // `_` sin nota antes: se subraya ese `_`.
        let line = r#"t:melody("c . _ .  . . . .  . . . .  . . . .")"#;
        let d = diagnose(line);
        assert_eq!(underlined(line, &d[0]), "_");
        assert!(d[0].message.contains("no hay ninguno"), "{}", d[0].message);
        // Pasos de menos: todo el patrón.
        let line = r#"t:melody("c e g")"#;
        let d = diagnose(line);
        assert_eq!(underlined(line, &d[0]), "c e g");
        assert!(d[0].message.contains("16 pasos"), "{}", d[0].message);
        // Con :length(n) en la misma cadena, cuenta esa longitud.
        assert!(diagnose(r#"track("l", piano):length(4):melody("c+e+g _ eb3 .")"#).is_empty());
        // Correctas: notas, octavas, acordes, sufijos, dentro de every. La
        // distancia a la raíz (±36) la dice la carga: aquí no se sabe.
        assert!(
            diagnose(
                r#"t:every(2, function(c) c:melody("c7 _ bb2@0.5 g#r2  c+e+g _ _ _  . . f'~800 .  a-1 . . .") end)"#
            )
            .is_empty()
        );
        // Una nota que también es una etiqueta no es un problema en steps.
        assert!(diagnose(r#"t:steps("1 . 2 .  . . . .  . . . .  . . . .")"#).is_empty());
    }
}
