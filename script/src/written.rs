//! Dónde está escrito cada paso de la mini-notación en el texto del script
//! (fase M: el resaltado en vivo del código al sonar). Al llamar a
//! `:steps("...")` o `:melody("...")` se busca la cadena en el texto: en la
//! línea de la llamada (la que da Lua) o, si no está ahí, la única cadena
//! igual de todo el texto (así vale `local p = "..."` con `:steps(p)`). Un
//! patrón generado (`euclid`, `rotate`, concatenaciones...) no está escrito
//! tal cual: sus pasos quedan sin posición y no se resaltan.

use mlua::Lua;

/// Dónde está escrito un paso: fila (desde 0) y columnas en caracteres
/// (`start..end`) del texto cargado.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct TokenSpan {
    pub row: usize,
    pub start: usize,
    pub end: usize,
}

/// El texto que se evalúa (en los datos de la aplicación de Lua) y el
/// nombre de su trozo.
pub(crate) struct ScriptText {
    pub chunk: String,
    pub lines: Vec<Vec<char>>,
}

impl ScriptText {
    pub fn new(source: &str, chunk: String) -> Self {
        ScriptText {
            chunk,
            lines: source.lines().map(|l| l.chars().collect()).collect(),
        }
    }
}

/// Posiciones de los `length` pasos de `text`, escrito en la llamada que
/// está en curso; todas `None` si no se encuentra.
pub(crate) fn spans(lua: &Lua, text: &str, length: usize) -> Vec<Option<TokenSpan>> {
    let found = lua.app_data_ref::<ScriptText>().and_then(|script| {
        let line = calling_line(lua, &script.chunk)?;
        let (row, start) = find_literal(&script.lines, line.checked_sub(1)?, text)?;
        let tokens = tokens(text);
        (tokens.len() == length).then(|| {
            tokens
                .into_iter()
                .map(|(s, e)| {
                    Some(TokenSpan {
                        row,
                        start: start + s,
                        end: start + e,
                    })
                })
                .collect()
        })
    });
    found.unwrap_or_else(|| vec![None; length])
}

/// Línea (desde 1) de la llamada de Lua en curso, si es del trozo del
/// script.
fn calling_line(lua: &Lua, chunk: &str) -> Option<usize> {
    (1..8).find_map(|level| {
        lua.inspect_stack(level, |debug| {
            let source = debug.source();
            if source.what == "C" {
                return None;
            }
            // Una función de otro trozo: no es el texto del editor.
            (source.source.as_deref() == Some(chunk)).then(|| debug.current_line())?
        })
        .flatten()
    })
}

/// Fila y columna (dentro de las comillas) de una cadena con ese
/// contenido: la primera de la fila `row` o, si no hay, la única del
/// texto.
fn find_literal(lines: &[Vec<char>], row: usize, text: &str) -> Option<(usize, usize)> {
    let content: Vec<char> = text.chars().collect();
    let content = content.as_slice();
    let matches = |r: usize| {
        literals(&lines[r])
            .into_iter()
            .filter(move |&(s, e)| lines[r][s..e] == *content)
            .map(move |(s, _)| (r, s))
    };
    if let Some(found) = lines.get(row).and_then(|_| matches(row).next()) {
        return Some(found);
    }
    let mut all = (0..lines.len()).flat_map(matches);
    let first = all.next()?;
    all.next().is_none().then_some(first)
}

/// Columnas de dentro de las comillas de las cadenas de una línea (sin las
/// de los comentarios; una sin cerrar no cuenta).
fn literals(line: &[char]) -> Vec<(usize, usize)> {
    let mut out = Vec::new();
    let mut i = 0;
    while i < line.len() {
        match line[i] {
            '-' if line.get(i + 1) == Some(&'-') => break,
            quote @ ('"' | '\'') => {
                let start = i + 1;
                let mut j = start;
                while j < line.len() && line[j] != quote {
                    // Un escape: lo de dentro no es el texto tal cual, pero
                    // hay que saltarlo para no cortar la cadena.
                    j += if line[j] == '\\' { 2 } else { 1 };
                }
                if j >= line.len() {
                    break;
                }
                out.push((start, j));
                i = j + 1;
            }
            _ => i += 1,
        }
    }
    out
}

/// Columnas (en caracteres) de cada paso separado por espacios (los de
/// dentro de las llaves de los efectos de paso no separan).
fn tokens(text: &str) -> Vec<(usize, usize)> {
    engine::notation::step_tokens(text)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lines(text: &str) -> Vec<Vec<char>> {
        text.lines().map(|l| l.chars().collect()).collect()
    }

    #[test]
    fn literals_skip_comments_escapes_and_unclosed_strings() {
        let line: Vec<char> = r#"a("x . .", 'y') -- "z""#.chars().collect();
        assert_eq!(literals(&line), vec![(3, 8), (12, 13)]);
        let line: Vec<char> = r#"a("x \" y", "q"#.chars().collect();
        assert_eq!(literals(&line), vec![(3, 9)]);
    }

    #[test]
    fn the_call_line_first_then_a_unique_literal() {
        let text =
            lines("local p = \"1 . 2 .\"\nt:steps(\"1 . 2 .\")\nt:steps(p)\nq = \"1 . 2 .\"");
        // En la línea de la llamada.
        assert_eq!(find_literal(&text, 1, "1 . 2 ."), Some((1, 9)));
        // En otra línea: solo si es la única (aquí hay tres).
        assert_eq!(find_literal(&text, 2, "1 . 2 ."), None);
        let text = lines("local p = \"1 . 2 .\"\nt:steps(p)");
        assert_eq!(find_literal(&text, 1, "1 . 2 ."), Some((0, 11)));
        assert_eq!(find_literal(&text, 1, "x . x ."), None);
    }

    #[test]
    fn tokens_are_char_columns() {
        assert_eq!(tokens(" 1  ñ' 3^7 "), vec![(1, 2), (4, 6), (7, 10)]);
    }
}
