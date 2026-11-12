//! Cierre automático de pares (`[editor] cerrar_pares`): al escribir `(`,
//! `[`, `{` o una comilla se pone también su pareja y el cursor queda en
//! medio; escribir el que cierra justo delante de él lo salta en vez de
//! repetirlo, y Retroceso entre una pareja vacía borra las dos.
//!
//! Para no estorbar: el que abre solo se dobla si detrás no hay nada, un
//! espacio o algo que cierra (así no envuelve una palabra ya escrita); una
//! comilla, además, no tras una letra o un número (`don't`) ni dentro de
//! una cadena de esa línea. Con una selección, el widget hace lo de
//! siempre (sustituirla). Lo doblado es un solo paso de deshacer.

use ratatui_textarea::{CursorMove, Input, Key};

use super::App;

/// El que cierra de cada uno que abre.
fn closer(c: char) -> Option<char> {
    match c {
        '(' => Some(')'),
        '[' => Some(']'),
        '{' => Some('}'),
        '"' => Some('"'),
        '\'' => Some('\''),
        _ => None,
    }
}

impl App {
    /// Si la tecla era de un par, lo hace y devuelve `true` (si no, la
    /// escribe el widget como siempre).
    pub(super) fn auto_pair(&mut self, input: &Input) -> bool {
        if !self.editor_config.close_pairs
            || input.ctrl
            || input.alt
            || self.editor.selection_range().is_some()
        {
            return false;
        }
        let (row, col) = {
            let c = self.editor.cursor();
            (c.0, c.1)
        };
        let line: Vec<char> = self.editor.lines()[row].chars().collect();
        let before = col.checked_sub(1).and_then(|i| line.get(i)).copied();
        let after = line.get(col).copied();
        match input.key {
            Key::Backspace => {
                let (Some(b), Some(a)) = (before, after) else {
                    return false;
                };
                if closer(b) != Some(a) {
                    return false;
                }
                // Retroceso entre `(` y `)`: los dos, en un paso.
                self.editor.move_cursor(CursorMove::Back);
                self.editor.delete_str(2);
                true
            }
            Key::Char(c) => {
                let quote = c == '"' || c == '\'';
                // Saltar el que cierra que ya está delante.
                if (matches!(c, ')' | ']' | '}') || quote) && after == Some(c) {
                    let inside = quote && in_string(&line[..col], c);
                    if !quote || inside {
                        self.editor.move_cursor(CursorMove::Forward);
                        return true;
                    }
                }
                let Some(close) = closer(c) else {
                    return false;
                };
                let free_after = after.is_none_or(|a| a.is_whitespace() || ")]},;".contains(a));
                if !free_after {
                    return false;
                }
                if quote
                    && (before.is_some_and(|b| b.is_alphanumeric() || b == '_')
                        || in_string(&line[..col], c))
                {
                    return false;
                }
                self.editor.insert_str(format!("{c}{close}"));
                self.editor.move_cursor(CursorMove::Back);
                true
            }
            _ => false,
        }
    }
}

/// Si en `before` (lo de la línea antes del cursor) hay una cadena abierta
/// con `quote` (fuera de los comentarios).
fn in_string(before: &[char], quote: char) -> bool {
    let mut open: Option<char> = None;
    let mut i = 0;
    while i < before.len() {
        let c = before[i];
        match open {
            Some(q) => {
                if c == '\\' {
                    i += 1;
                } else if c == q {
                    open = None;
                }
            }
            None => {
                if c == '-' && before.get(i + 1) == Some(&'-') {
                    return false;
                }
                if c == '"' || c == '\'' {
                    open = Some(c);
                }
            }
        }
        i += 1;
    }
    open == Some(quote)
}
