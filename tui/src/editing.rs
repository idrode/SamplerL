//! Operaciones de edición sobre el [`TextArea`], sin terminal: comentar,
//! duplicar y mover líneas. Todo se hace con las operaciones del propio
//! editor (`insert_str`, `delete_str`), así que deshacer y rehacer
//! funcionan con ellas.

use ratatui_textarea::{CursorMove, TextArea};

/// Filas afectadas: las de la selección o la del cursor.
fn selected_rows(editor: &TextArea) -> (usize, usize) {
    match editor.selection_range() {
        Some(((a, _), (b, c))) => {
            let (a, b) = (a.min(b), a.max(b));
            // Una selección que acaba al principio de una línea no la incluye.
            if b > a && c == 0 && editor.cursor().0 == b {
                (a, b - 1)
            } else {
                (a, b)
            }
        }
        None => (editor.cursor().0, editor.cursor().0),
    }
}

/// Comenta o descomenta (`-- `) las líneas seleccionadas o la del cursor.
/// Si todas las que tienen texto están comentadas, las descomenta; si no,
/// las comenta todas, con el `--` a la sangría mínima.
pub fn toggle_comment(editor: &mut TextArea) {
    let (a, b) = selected_rows(editor);
    let (row, col) = (editor.cursor().0, editor.cursor().1);
    let lines: Vec<String> = editor.lines()[a..=b].to_vec();
    let filled: Vec<&String> = lines.iter().filter(|l| !l.trim().is_empty()).collect();
    if filled.is_empty() {
        return;
    }
    let uncomment = filled.iter().all(|l| l.trim_start().starts_with("--"));
    let indent = filled
        .iter()
        .map(|l| l.chars().take_while(|c| c.is_whitespace()).count())
        .min()
        .unwrap_or(0);
    editor.cancel_selection();
    let mut shift = 0isize;
    for (i, line) in lines.iter().enumerate() {
        if line.trim().is_empty() {
            continue;
        }
        let r = (a + i) as u16;
        if uncomment {
            let lead = line.chars().take_while(|c| c.is_whitespace()).count();
            let rest = &line.trim_start()[2..];
            let n = if rest.starts_with(' ') { 3 } else { 2 };
            editor.move_cursor(CursorMove::Jump(r, lead as u16));
            editor.delete_str(n);
            if a + i == row && col > lead {
                shift = -((col - lead).min(n) as isize);
            }
        } else {
            editor.move_cursor(CursorMove::Jump(r, indent as u16));
            editor.insert_str("-- ");
            if a + i == row && col >= indent {
                shift = 3;
            }
        }
    }
    let col = (col as isize + shift).max(0) as u16;
    editor.move_cursor(CursorMove::Jump(row as u16, col));
}

/// Duplica las líneas seleccionadas o la del cursor debajo de sí mismas;
/// el cursor pasa a la copia.
pub fn duplicate_lines(editor: &mut TextArea) {
    let (a, b) = selected_rows(editor);
    let (row, col) = (editor.cursor().0, editor.cursor().1);
    let block = editor.lines()[a..=b].join("\n");
    let end = editor.lines()[b].chars().count();
    editor.cancel_selection();
    editor.move_cursor(CursorMove::Jump(b as u16, end as u16));
    editor.insert_str(format!("\n{block}"));
    let row = row + (b - a + 1);
    editor.move_cursor(CursorMove::Jump(row as u16, col as u16));
}

/// Mueve una línea arriba o abajo las líneas seleccionadas o la del
/// cursor (con el cursor). No hace nada en el borde.
pub fn move_lines(editor: &mut TextArea, up: bool) -> bool {
    let (a, b) = selected_rows(editor);
    let (row, col) = (editor.cursor().0, editor.cursor().1);
    let count = editor.lines().len();
    if (up && a == 0) || (!up && b + 1 >= count) {
        return false;
    }
    editor.cancel_selection();
    if up {
        // Quitar la línea de encima y ponerla debajo del bloque.
        let above = editor.lines()[a - 1].clone();
        editor.move_cursor(CursorMove::Jump((a - 1) as u16, 0));
        editor.delete_str(above.chars().count() + 1);
        let end = editor.lines()[b - 1].chars().count();
        editor.move_cursor(CursorMove::Jump((b - 1) as u16, end as u16));
        editor.insert_str(format!("\n{above}"));
        editor.move_cursor(CursorMove::Jump((row - 1) as u16, col as u16));
    } else {
        // Quitar la línea de debajo y ponerla encima del bloque.
        let below = editor.lines()[b + 1].clone();
        let end = editor.lines()[b].chars().count();
        editor.move_cursor(CursorMove::Jump(b as u16, end as u16));
        editor.delete_str(below.chars().count() + 1);
        editor.move_cursor(CursorMove::Jump(a as u16, 0));
        editor.insert_str(format!("{below}\n"));
        editor.move_cursor(CursorMove::Jump((row + 1) as u16, col as u16));
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    fn editor(text: &str) -> TextArea<'static> {
        TextArea::new(text.lines().map(String::from).collect())
    }

    fn at(editor: &mut TextArea, row: usize, col: usize) {
        editor.move_cursor(CursorMove::Jump(row as u16, col as u16));
    }

    #[test]
    fn comments_and_uncomments_lines() {
        let mut e = editor("a\n  b\n\n  -- c");
        at(&mut e, 1, 3);
        toggle_comment(&mut e);
        assert_eq!(e.lines(), ["a", "  -- b", "", "  -- c"]);
        assert_eq!(e.cursor(), (1, 6), "el cursor sigue sobre el mismo texto");
        toggle_comment(&mut e);
        assert_eq!(e.lines()[1], "  b");
        // Selección de varias líneas: comenta todas a la sangría mínima.
        at(&mut e, 0, 0);
        e.start_selection();
        at(&mut e, 3, 2);
        toggle_comment(&mut e);
        assert_eq!(e.lines(), ["-- a", "--   b", "", "--   -- c"]);
        e.select_all();
        toggle_comment(&mut e);
        assert_eq!(e.lines(), ["a", "  b", "", "  -- c"]);
    }

    #[test]
    fn duplicates_and_moves_lines() {
        let mut e = editor("uno\ndos\ntres");
        at(&mut e, 1, 2);
        duplicate_lines(&mut e);
        assert_eq!(e.lines(), ["uno", "dos", "dos", "tres"]);
        assert_eq!(e.cursor(), (2, 2));
        assert!(move_lines(&mut e, false));
        assert_eq!(e.lines(), ["uno", "dos", "tres", "dos"]);
        assert_eq!(e.cursor(), (3, 2));
        assert!(!move_lines(&mut e, false), "ya está abajo");
        at(&mut e, 0, 1);
        assert!(!move_lines(&mut e, true), "ya está arriba");
        // Un bloque seleccionado se mueve entero.
        at(&mut e, 1, 0);
        e.start_selection();
        at(&mut e, 2, 1);
        assert!(move_lines(&mut e, true));
        assert_eq!(e.lines(), ["dos", "tres", "uno", "dos"]);
        assert_eq!(e.cursor(), (1, 1));
    }

    #[test]
    fn everything_can_be_undone() {
        let mut e = editor("uno\ndos");
        duplicate_lines(&mut e);
        move_lines(&mut e, true);
        toggle_comment(&mut e);
        while e.undo() {}
        assert_eq!(e.lines(), ["uno", "dos"]);
    }
}
