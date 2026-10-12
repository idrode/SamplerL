//! Operaciones de edición sobre el [`TextArea`], sin terminal: insertar
//! plantillas con campos (autocompletado y snippets) y saltar entre ellos,
//! y comentar, duplicar y mover líneas. Todo se hace con las operaciones
//! del propio editor (`insert_str`, `delete_str`), así que deshacer y
//! rehacer funcionan con ellas.

use ratatui_textarea::{CursorMove, TextArea};
use script::api;

/// Campo pendiente de una plantilla insertada.
#[derive(Debug, Clone, PartialEq)]
struct Field {
    /// Texto entre el campo anterior (o el inicio) y este; puede tener
    /// saltos de línea.
    gap: String,
    /// Texto del campo (vacío: el final de la plantilla).
    text: String,
}

/// Campos de la última plantilla que faltan por rellenar.
#[derive(Debug, Clone, Default)]
pub struct Fields(Vec<Field>);

impl Fields {
    #[cfg(test)]
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    pub fn clear(&mut self) {
        self.0.clear();
    }

    /// Va al siguiente campo y lo selecciona (lo que se escriba lo
    /// sustituye). Lo busca justo tras el cursor si el texto intermedio no
    /// ha cambiado o, si no, en el resto del texto. Devuelve si había uno
    /// y lo encontró; si no lo encuentra, olvida los pendientes.
    pub fn next(&mut self, editor: &mut TextArea) -> Option<Selected> {
        if self.0.is_empty() {
            return None;
        }
        let field = self.0.remove(0);
        let rest = text_from_cursor(editor);
        let at = if rest.starts_with(&format!("{}{}", field.gap, field.text)) {
            Some(field.gap.chars().count())
        } else if field.text.is_empty() {
            None
        } else {
            rest.find(&field.text)
                .map(|byte| rest[..byte].chars().count())
        };
        let Some(at) = at else {
            self.0.clear();
            return None;
        };
        let (row, col) = advance(editor, at);
        Some(select(editor, row, col, field.text.chars().count()))
    }
}

/// Si quedó texto seleccionado al ir a un campo.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Selected {
    /// Un campo con texto, seleccionado.
    Field,
    /// El final de la plantilla: solo el cursor.
    End,
}

/// Sustituye los `replace` caracteres antes del cursor por la plantilla
/// (con sus `${...}`, ver [`api::expand_template`]). Las líneas nuevas
/// llevan la sangría de la actual. Si tiene campos, selecciona el primero
/// y devuelve los demás (con el final de la plantilla como último); si no,
/// `None`.
pub fn insert_template(editor: &mut TextArea, replace: usize, template: &str) -> Option<Fields> {
    let (row, col) = (editor.cursor().0, editor.cursor().1);
    let indent: String = editor.lines()[row]
        .chars()
        .take_while(|c| c.is_whitespace())
        .collect();
    let template = template.replace('\n', &format!("\n{indent}"));
    let (text, marks) = api::expand_template(&template);
    let start = col.saturating_sub(replace);
    editor.cancel_selection();
    editor.move_cursor(CursorMove::Jump(row as u16, start as u16));
    editor.delete_str(col - start);
    editor.insert_str(&text);
    if marks.is_empty() {
        return None;
    }
    let chars: Vec<char> = text.chars().collect();
    let mut end = 0;
    let mut fields: Vec<Field> = marks
        .iter()
        .map(|m| {
            let field = Field {
                gap: chars[end..m.start].iter().collect(),
                text: chars[m.clone()].iter().collect(),
            };
            end = m.end;
            field
        })
        .collect();
    fields.push(Field {
        gap: chars[end..].iter().collect(),
        text: String::new(),
    });
    let mut fields = Fields(fields);
    editor.move_cursor(CursorMove::Jump(row as u16, start as u16));
    fields.next(editor);
    Some(fields)
}

/// Texto desde el cursor hasta el final.
fn text_from_cursor(editor: &TextArea) -> String {
    let (row, col) = (editor.cursor().0, editor.cursor().1);
    let lines = editor.lines();
    let mut text: String = lines[row].chars().skip(col).collect();
    for line in &lines[row + 1..] {
        text.push('\n');
        text.push_str(line);
    }
    text
}

/// Posición `chars` caracteres después del cursor (los saltos de línea
/// cuentan uno).
fn advance(editor: &TextArea, chars: usize) -> (usize, usize) {
    let (mut row, mut col) = (editor.cursor().0, editor.cursor().1);
    let lines = editor.lines();
    let mut left = chars;
    loop {
        let len = lines[row].chars().count();
        if col + left <= len || row + 1 == lines.len() {
            return (row, (col + left).min(len));
        }
        left -= len - col + 1;
        row += 1;
        col = 0;
    }
}

/// Selecciona `len` caracteres desde (`row`, `col`), en la misma línea.
fn select(editor: &mut TextArea, row: usize, col: usize, len: usize) -> Selected {
    editor.cancel_selection();
    editor.move_cursor(CursorMove::Jump(row as u16, col as u16));
    if len == 0 {
        return Selected::End;
    }
    editor.start_selection();
    editor.move_cursor(CursorMove::Jump(row as u16, (col + len) as u16));
    Selected::Field
}

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

    fn selected(editor: &TextArea) -> String {
        let ((r1, c1), (r2, c2)) = editor.selection_range().expect("sin selección");
        assert_eq!(r1, r2);
        editor.lines()[r1].chars().skip(c1).take(c2 - c1).collect()
    }

    #[test]
    fn templates_select_each_field_across_lines() {
        let mut e = editor("  am");
        at(&mut e, 0, 4);
        let mut fields = insert_template(&mut e, 2, "a(${1})\nb(\"${x}\")\n${end}").unwrap();
        // Las líneas nuevas llevan la sangría de la actual.
        assert_eq!(e.lines(), ["  a(1)", "  b(\"x\")", "  end"]);
        assert_eq!(selected(&e), "1");
        e.insert_str("22");
        assert_eq!(fields.next(&mut e), Some(Selected::Field));
        assert_eq!(selected(&e), "x");
        assert_eq!(fields.next(&mut e), Some(Selected::Field));
        assert_eq!(selected(&e), "end");
        assert_eq!(fields.next(&mut e), Some(Selected::End));
        assert_eq!(e.cursor(), (2, 5));
        assert!(fields.is_empty());
        assert_eq!(fields.next(&mut e), None);
    }

    #[test]
    fn fields_are_found_further_on_if_the_text_changed() {
        let mut e = editor("");
        let mut fields = insert_template(&mut e, 0, "f(${a}, ${b})").unwrap();
        e.cancel_selection();
        at(&mut e, 0, 2);
        e.insert_str("zz, ");
        assert_eq!(e.lines(), ["f(zz, a, b)"]);
        assert_eq!(fields.next(&mut e), Some(Selected::Field));
        assert_eq!(selected(&e), "b");
        assert!(insert_template(&mut e, 0, "sin campos").is_none());
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
