//! Dónde está escrita la llamada que corta un sample (`:slice(8)`,
//! `:slice_grid(16)` o `:slice({ {0, 120}, {120, 250} })`) y cómo se
//! reescribe con los cortes editados en la onda. El código es la fuente de
//! verdad de los cortes: la interfaz cambia esta llamada por la forma
//! explícita, con el inicio y el final de cada corte en ms del sample.

use std::ops::Range;

use mlua::Lua;

use crate::written::{ScriptText, TableSpan, calling_row_in};

/// Una llamada que corta un sample, tal como está escrita, y los cortes que
/// da (para reescribirla).
#[derive(Debug, Clone, PartialEq)]
pub struct SliceCall {
    /// Desde la primera letra del nombre del método hasta su `)` (o la `}`
    /// de `slice{ ... }`), como en [`TableSpan`].
    pub span: TableSpan,
    /// Los cortes que da la llamada (todos, en frames del sample sin
    /// estirar), con los ajustes del proyecto y los cruces por cero.
    pub ranges: Vec<Range<usize>>,
    /// La etiqueta de cada corte de la llamada: la del proyecto o la de su
    /// entrada en la tabla (las de `:name()` ya están en el código).
    pub labels: Vec<Option<String>>,
    /// Lo que va detrás del primer argumento, tal cual (`, { zero_cross =
    /// 2 }`), o nada.
    pub rest: String,
    /// Frecuencia del sample (para pasar los frames a ms).
    pub sample_rate: u32,
}

impl SliceCall {
    /// La llamada con estos cortes (en frames del sample sin estirar),
    /// escrita en la forma explícita. `indent` es la sangría de la línea
    /// donde empieza, para las líneas de la tabla si no cabe en una.
    pub fn rewritten(
        &self,
        ranges: &[Range<usize>],
        labels: &[Option<String>],
        indent: &str,
    ) -> String {
        let ms = |frame: usize| frame_ms_text(frame, self.sample_rate);
        let entries: Vec<String> = ranges
            .iter()
            .enumerate()
            .map(|(i, r)| match labels.get(i).cloned().flatten() {
                Some(name) => format!("{{{}, {}, \"{name}\"}}", ms(r.start), ms(r.end)),
                None => format!("{{{}, {}}}", ms(r.start), ms(r.end)),
            })
            .collect();
        let one_line = format!("slice({{ {} }}{})", entries.join(", "), self.rest);
        if one_line.chars().count() <= 72 {
            return one_line;
        }
        let mut out = String::from("slice({\n");
        for row in entries.chunks(4) {
            out.push_str(indent);
            out.push_str("  ");
            out.push_str(&row.join(", "));
            out.push_str(",\n");
        }
        out.push_str(indent);
        out.push('}');
        out.push_str(&self.rest);
        out.push(')');
        out
    }
}

/// El frame `frame` en ms con los menos decimales (hasta tres) que vuelven
/// a ese mismo frame: `67.2` mejor que `67.208` si los dos dan el 3226.
pub fn frame_ms_text(frame: usize, sample_rate: u32) -> String {
    let rate = sample_rate.max(1) as f64;
    let exact = frame as f64 * 1000.0 / rate;
    for decimals in 0..3 {
        let scale = 10f64.powi(decimals);
        let ms = (exact * scale).round() / scale;
        if (ms / 1000.0 * rate).round() as usize == frame {
            return ms_text(ms);
        }
    }
    ms_text(exact)
}

/// Un número en ms, con hasta tres decimales (al microsegundo: vuelve al
/// mismo frame a cualquier frecuencia de hasta 192 kHz) y sin ceros de más.
pub fn ms_text(ms: f64) -> String {
    let text = format!("{:.3}", (ms * 1000.0).round() / 1000.0);
    let text = text.trim_end_matches('0').trim_end_matches('.');
    if text == "-0" {
        "0".into()
    } else {
        text.into()
    }
}

/// Lo que se espera como primer argumento de la llamada.
pub(crate) enum FirstArg<'a> {
    /// `slice(8)`, `slice_grid(16)`: ese número.
    Count(i64),
    /// `slice({ ... })`: esa tabla de cortes en ms.
    Table(&'a [(f64, f64, Option<String>)]),
}

/// Dónde está escrita la llamada en curso a uno de los métodos `names` con
/// ese primer argumento: en la línea que da Lua o en las de alrededor (una
/// cadena de métodos va en varias; primero las de después), o por qué no
/// se puede reescribir.
pub(crate) fn find(
    lua: &Lua,
    names: &[&str],
    first: FirstArg,
) -> Result<(TableSpan, String), String> {
    let script = lua
        .app_data_ref::<ScriptText>()
        .ok_or("no hay texto del script")?;
    let row = calling_row_in(lua, &script.chunk).ok_or("la llamada no está en el script")?;
    let lines = &script.lines;
    let mut rows: Vec<usize> = (row..lines.len().min(row + 4)).collect();
    rows.extend((row.saturating_sub(3)..row).rev());
    for r in rows {
        let mut found = Vec::new();
        for name in names {
            for col in method_columns(&lines[r], name) {
                if let Some((span, arg, rest)) = arguments(lines, (r, col), name)
                    && accepts(&first, &arg)
                {
                    found.push((span, rest));
                }
            }
        }
        match found.len() {
            0 => continue,
            1 => return Ok(found.remove(0)),
            _ => return Err("hay dos llamadas iguales en la misma línea".into()),
        }
    }
    Err(match first {
        FirstArg::Count(_) => "el número de cortes no está escrito tal cual (sale de una variable o una cuenta)".into(),
        FirstArg::Table(_) => "la tabla de cortes no está escrita tal cual dentro de la llamada (lleva variables, cuentas o comentarios)".into(),
    })
}

/// Si el primer argumento escrito, `arg`, es el que llegó.
fn accepts(first: &FirstArg, arg: &str) -> bool {
    match first {
        FirstArg::Count(n) => arg.trim().parse::<f64>().is_ok_and(|v| v == *n as f64),
        FirstArg::Table(entries) => parse_ranges(arg).is_some_and(|read| read == *entries),
    }
}

/// Columnas de una línea donde empieza `:name` como método (fuera de
/// cadenas y comentarios): la de la primera letra del nombre.
fn method_columns(line: &[char], name: &str) -> Vec<usize> {
    let name: Vec<char> = name.chars().collect();
    let code = code_mask(line);
    let mut out = Vec::new();
    for i in 0..line.len() {
        if line.len() < i + name.len() || line[i..i + name.len()] != name[..] || !code[i] {
            continue;
        }
        let after = line.get(i + name.len());
        if after.is_some_and(|c| c.is_alphanumeric() || *c == '_') {
            continue;
        }
        let before = line[..i].iter().rev().find(|c| !c.is_whitespace());
        if before == Some(&':') {
            out.push(i);
        }
    }
    out
}

/// Qué columnas de la línea son código (no cadena ni comentario).
fn code_mask(line: &[char]) -> Vec<bool> {
    let mut mask = vec![true; line.len()];
    let mut quote: Option<char> = None;
    let mut i = 0;
    while i < line.len() {
        let c = line[i];
        match quote {
            Some(q) => {
                mask[i] = false;
                if c == '\\' {
                    if let Some(m) = mask.get_mut(i + 1) {
                        *m = false;
                    }
                    i += 1;
                } else if c == q {
                    quote = None;
                }
            }
            None if c == '-' && line.get(i + 1) == Some(&'-') => {
                mask[i..].fill(false);
                break;
            }
            None if c == '"' || c == '\'' => {
                mask[i] = false;
                quote = Some(c);
            }
            None => {}
        }
        i += 1;
    }
    mask
}

/// Los argumentos de la llamada cuyo nombre empieza en `at`: el sitio de la
/// llamada entera, el primer argumento y lo que va detrás (con su coma).
fn arguments(
    lines: &[Vec<char>],
    at: (usize, usize),
    name: &str,
) -> Option<(TableSpan, String, String)> {
    let open = skip_space(lines, (at.0, at.1 + name.chars().count()))?;
    let opening = lines[open.0][open.1];
    if opening == '{' {
        // `slice{ ... }`: la tabla es el único argumento.
        let close = closing(lines, open)?;
        let text = text_between(lines, open, (close.0, close.1 + 1))?;
        return Some((
            TableSpan {
                start: at,
                end: (close.0, close.1 + 1),
            },
            text,
            String::new(),
        ));
    }
    if opening != '(' {
        return None;
    }
    let close = closing(lines, open)?;
    let inside = text_between(lines, (open.0, open.1 + 1), close)?;
    if inside.contains("--") {
        return None;
    }
    let split = top_level_comma(&inside).unwrap_or(inside.len());
    let (arg, rest) = inside.split_at(split);
    Some((
        TableSpan {
            start: at,
            end: (close.0, close.1 + 1),
        },
        arg.to_string(),
        rest.trim_end().to_string(),
    ))
}

/// La primera coma fuera de llaves, paréntesis y cadenas (en bytes).
fn top_level_comma(text: &str) -> Option<usize> {
    let mut depth = 0i32;
    let mut quote: Option<char> = None;
    let mut escaped = false;
    for (i, c) in text.char_indices() {
        match quote {
            Some(_) if escaped => escaped = false,
            Some(_) if c == '\\' => escaped = true,
            Some(q) if c == q => quote = None,
            Some(_) => {}
            None => match c {
                '"' | '\'' => quote = Some(c),
                '{' | '(' | '[' => depth += 1,
                '}' | ')' | ']' => depth -= 1,
                ',' if depth == 0 => return Some(i),
                _ => {}
            },
        }
    }
    None
}

/// La siguiente posición que no es un espacio (pasando de línea).
fn skip_space(lines: &[Vec<char>], (mut row, mut col): (usize, usize)) -> Option<(usize, usize)> {
    loop {
        let line = lines.get(row)?;
        while col < line.len() && line[col].is_whitespace() {
            col += 1;
        }
        if col < line.len() {
            return Some((row, col));
        }
        row += 1;
        col = 0;
    }
}

/// El cierre de la `(` o la `{` de `open` (contando las de dentro y sin
/// las de las cadenas).
fn closing(lines: &[Vec<char>], open: (usize, usize)) -> Option<(usize, usize)> {
    let mut depth = 0usize;
    let mut quote: Option<char> = None;
    for (r, line) in lines.iter().enumerate().skip(open.0) {
        let mut c = if r == open.0 { open.1 } else { 0 };
        while c < line.len() {
            let ch = line[c];
            match quote {
                Some(q) if ch == q => quote = None,
                Some(_) if ch == '\\' => c += 1,
                Some(_) => {}
                None => match ch {
                    '"' | '\'' => quote = Some(ch),
                    '(' | '{' | '[' => depth += 1,
                    ')' | '}' | ']' => {
                        depth = depth.checked_sub(1)?;
                        if depth == 0 {
                            return Some((r, c));
                        }
                    }
                    _ => {}
                },
            }
            c += 1;
        }
    }
    None
}

/// El texto de `from` a `to` (sin incluir `to`), con sus saltos de línea.
fn text_between(lines: &[Vec<char>], from: (usize, usize), to: (usize, usize)) -> Option<String> {
    let mut out = String::new();
    for r in from.0..=to.0 {
        let line = lines.get(r)?;
        let a = if r == from.0 { from.1 } else { 0 };
        let b = if r == to.0 { to.1 } else { line.len() };
        out.extend(line.get(a..b)?);
        if r != to.0 {
            out.push('\n');
        }
    }
    Some(out)
}

/// Lee una tabla de cortes escrita tal cual: `{ {0, 120.5}, {120.5, 250,
/// "snare"} }` (con espacios, saltos de línea y una coma final), o nada si
/// lleva otra cosa (variables, cuentas, comentarios).
pub fn parse_ranges(text: &str) -> Option<Vec<(f64, f64, Option<String>)>> {
    let mut rest = text.trim().strip_prefix('{')?.trim_start();
    let mut out = Vec::new();
    loop {
        if let Some(after) = rest.strip_prefix('}') {
            return after.trim().is_empty().then_some(out);
        }
        let inner = rest.strip_prefix('{')?;
        let close = inner.find('}')?;
        let (entry, after) = inner.split_at(close);
        let fields: Vec<&str> = entry.split(',').map(str::trim).collect();
        let fields = match fields.as_slice() {
            [a, b, ""] | [a, b] => (*a, *b, None),
            [a, b, name] | [a, b, name, ""] => (*a, *b, Some(*name)),
            _ => return None,
        };
        let number = |t: &str| t.parse::<f64>().ok().filter(|v| v.is_finite());
        let name = match fields.2 {
            None => None,
            Some(quoted) => {
                let q = quoted.chars().next()?;
                if q != '"' && q != '\'' || quoted.len() < 2 || !quoted.ends_with(q) {
                    return None;
                }
                let name = &quoted[1..quoted.len() - 1];
                if name.contains(['\\', '"', '\'']) {
                    return None;
                }
                Some(name.to_string())
            }
        };
        out.push((number(fields.0)?, number(fields.1)?, name));
        rest = after[1..].trim_start();
        rest = rest.strip_prefix(',').unwrap_or(rest).trim_start();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lines(text: &str) -> Vec<Vec<char>> {
        text.lines().map(|l| l.chars().collect()).collect()
    }

    #[test]
    fn frames_are_written_with_the_fewest_decimals_that_read_back() {
        assert_eq!(frame_ms_text(3226, 48_000), "67.2");
        assert_eq!(frame_ms_text(3000, 48_000), "62.5");
        assert_eq!(frame_ms_text(1, 48_000), "0.02");
        assert_eq!(frame_ms_text(44_100, 44_100), "1000");
        for rate in [22_050, 44_100, 48_000, 96_000, 192_000] {
            for frame in (0..20_000).step_by(7) {
                let text = frame_ms_text(frame, rate);
                let back = (text.parse::<f64>().unwrap() / 1000.0 * rate as f64).round() as usize;
                assert_eq!(back, frame, "{rate} {frame} {text}");
            }
        }
    }

    #[test]
    fn ms_have_up_to_three_decimals() {
        assert_eq!(ms_text(0.0), "0");
        assert_eq!(ms_text(62.5), "62.5");
        assert_eq!(ms_text(1000.0 / 48.0), "20.833");
        assert_eq!(ms_text(-0.0001), "0");
    }

    #[test]
    fn tables_of_ranges_are_read_only_when_literal() {
        assert_eq!(
            parse_ranges("{ {0, 120.5}, {120.5, 250, \"snare\"}, }"),
            Some(vec![
                (0.0, 120.5, None),
                (120.5, 250.0, Some("snare".into()))
            ])
        );
        assert_eq!(
            parse_ranges("{\n  {0, 1},\n  {2, 3}\n}"),
            Some(vec![(0.0, 1.0, None), (2.0, 3.0, None)])
        );
        assert_eq!(parse_ranges("{ {0, x} }"), None);
        assert_eq!(parse_ranges("{ {0, 1 + 2} }"), None);
        assert_eq!(parse_ranges("cortes"), None);
        assert_eq!(parse_ranges("{ {0, 1} } x"), None);
    }

    #[test]
    fn methods_are_found_outside_strings_and_comments() {
        let line: Vec<char> = "a:slice(8) -- b:slice(8)".chars().collect();
        assert_eq!(method_columns(&line, "slice"), vec![2]);
        let line: Vec<char> = "x = \":slice(8)\" y:slice_grid(4)".chars().collect();
        assert_eq!(method_columns(&line, "slice"), Vec::<usize>::new());
        assert_eq!(method_columns(&line, "slice_grid"), vec![18]);
        let line: Vec<char> = "  :slice(8)".chars().collect();
        assert_eq!(method_columns(&line, "slice"), vec![3]);
    }

    #[test]
    fn the_arguments_split_at_the_first_top_level_comma() {
        let text = lines("s:slice({ {0, 1}, {1, 2} }, { zero_cross = 2 })");
        let (span, arg, rest) = arguments(&text, (0, 2), "slice").unwrap();
        assert_eq!(span.start, (0, 2));
        assert_eq!(span.end, (0, 47));
        assert_eq!(arg, "{ {0, 1}, {1, 2} }");
        assert_eq!(rest, ", { zero_cross = 2 }");
        let text = lines("s:slice{\n {0, 1},\n}\n");
        let (span, arg, rest) = arguments(&text, (0, 2), "slice").unwrap();
        assert_eq!(span.end, (2, 1));
        assert_eq!(arg, "{\n {0, 1},\n}");
        assert_eq!(rest, "");
    }

    #[test]
    fn the_rewritten_call_fits_in_a_line_or_wraps() {
        let call = SliceCall {
            span: TableSpan {
                start: (0, 0),
                end: (0, 0),
            },
            ranges: Vec::new(),
            labels: Vec::new(),
            rest: String::new(),
            sample_rate: 1000,
        };
        assert_eq!(
            call.rewritten(&[0..120, 120..250], &[None, Some("snare".into())], ""),
            "slice({ {0, 120}, {120, 250, \"snare\"} })"
        );
        let many: Vec<Range<usize>> = (0..8).map(|i| i * 1000..i * 1000 + 999).collect();
        let text = call.rewritten(&many, &[], "  ");
        assert!(text.starts_with("slice({\n    {0, 999}, "), "{text}");
        assert!(text.ends_with("\n  })"), "{text}");
        let read = parse_ranges(&text["slice(".len()..text.len() - 1]).unwrap();
        assert_eq!(read.len(), 8);
    }
}
