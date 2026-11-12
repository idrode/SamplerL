//! La pareja del paréntesis, corchete o llave junto al cursor (para
//! resaltarla, `[editor] pareja`). Sin terminal: solo el texto. Las
//! cadenas y los comentarios de cada línea no cuentan (un `)` dentro de
//! `"..."` no es pareja de nada); las cadenas largas de varias líneas, sí
//! (para eso haría falta leer el texto entero en cada cuadro).

/// Líneas que se miran como mucho hacia cada lado.
const MAX_LINES: usize = 500;

/// La pareja del que está bajo el cursor o, si no, del que está justo
/// antes: las posiciones (fila, columna en caracteres) de los dos.
pub fn matching(lines: &[String], (row, col): (usize, usize)) -> Option<[(usize, usize); 2]> {
    let line: Vec<char> = lines.get(row)?.chars().collect();
    let code = mask(&line);
    let at = [Some(col), col.checked_sub(1)]
        .into_iter()
        .flatten()
        .find(|&c| code.get(c).is_some_and(|ch| pair(*ch).is_some()))?;
    let (open, close, forward) = pair(code[at])?;
    let (me, other) = if forward {
        (open, close)
    } else {
        (close, open)
    };
    let mut depth = 0usize;
    let rows: Box<dyn Iterator<Item = usize>> = if forward {
        Box::new(row..lines.len().min(row + MAX_LINES))
    } else {
        Box::new((row.saturating_sub(MAX_LINES)..=row).rev())
    };
    for r in rows {
        let chars: Vec<char> = if r == row {
            code.clone()
        } else {
            mask(&lines[r].chars().collect::<Vec<_>>())
        };
        let cols: Box<dyn Iterator<Item = usize>> = match (forward, r == row) {
            (true, true) => Box::new(at..chars.len()),
            (true, false) => Box::new(0..chars.len()),
            (false, true) => Box::new((0..=at).rev()),
            (false, false) => Box::new((0..chars.len()).rev()),
        };
        for c in cols {
            if chars[c] == me {
                depth += 1;
            } else if chars[c] == other {
                depth -= 1;
                if depth == 0 {
                    return Some([(row, at), (r, c)]);
                }
            }
        }
    }
    None
}

/// `(abre, cierra, hacia delante)` si `c` es uno de los seis.
fn pair(c: char) -> Option<(char, char, bool)> {
    match c {
        '(' => Some(('(', ')', true)),
        '[' => Some(('[', ']', true)),
        '{' => Some(('{', '}', true)),
        ')' => Some(('(', ')', false)),
        ']' => Some(('[', ']', false)),
        '}' => Some(('{', '}', false)),
        _ => None,
    }
}

/// La línea con las cadenas y el comentario en blanco (mismas columnas).
fn mask(line: &[char]) -> Vec<char> {
    let mut out = line.to_vec();
    let mut i = 0;
    while i < line.len() {
        let c = line[i];
        if c == '-' && line.get(i + 1) == Some(&'-') {
            out[i..].fill(' ');
            break;
        }
        if c == '"' || c == '\'' {
            let mut e = i + 1;
            while e < line.len() && line[e] != c {
                if line[e] == '\\' {
                    e += 1;
                }
                e += 1;
            }
            let end = (e + 1).min(line.len());
            out[i..end].fill(' ');
            i = end;
            continue;
        }
        i += 1;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lines(text: &str) -> Vec<String> {
        text.lines().map(String::from).collect()
    }

    #[test]
    fn finds_the_pair_forward_and_backward() {
        let text = lines("track(\"a\"):fx(\"filter\", { cutoff = f(2) })");
        // Sobre el `(` de track.
        assert_eq!(matching(&text, (0, 5)), Some([(0, 5), (0, 9)]));
        // Justo después del `)` de track.
        assert_eq!(matching(&text, (0, 10)), Some([(0, 9), (0, 5)]));
        // La llave, sobre otras parejas dentro.
        let open = text[0].find('{').unwrap();
        let close = text[0].find('}').unwrap();
        assert_eq!(matching(&text, (0, open)), Some([(0, open), (0, close)]));
        // Fuera de toda pareja: nada.
        assert_eq!(matching(&text, (0, 2)), None);
    }

    #[test]
    fn crosses_lines_and_skips_strings_and_comments() {
        let text = lines("every(4, function(t)\n  t:steps(\")\") -- )\nend)");
        assert_eq!(matching(&text, (0, 5)), Some([(0, 5), (2, 3)]));
        assert_eq!(matching(&text, (2, 4)), Some([(2, 3), (0, 5)]));
        // Un paréntesis dentro de una cadena no tiene pareja que buscar.
        assert_eq!(matching(&text, (1, 11)), None);
    }

    #[test]
    fn unbalanced_has_no_pair() {
        assert_eq!(matching(&lines("bpm(120"), (0, 3)), None);
        assert_eq!(matching(&lines(""), (0, 0)), None);
    }
}
