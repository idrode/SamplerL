//! Operaciones de edición sobre el [`TextArea`], sin terminal: insertar
//! plantillas con campos (autocompletado y snippets) y saltar entre ellos,
//! y comentar, duplicar y mover líneas. Todo se hace con las operaciones
//! del propio editor (`insert_str`, `delete_str`), así que deshacer y
//! rehacer funcionan con ellas.

use ratatui_textarea::{CursorMove, Input, Key, TextArea};
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
    let mut fields = build_fields("", &text, &marks);
    editor.move_cursor(CursorMove::Jump(row as u16, start as u16));
    fields.next(editor);
    Some(fields)
}

/// Campos de un texto insertado con sus marcas; `lead` es lo que hay
/// entre el cursor y el principio del texto. El último es el final.
fn build_fields(lead: &str, text: &str, marks: &[std::ops::Range<usize>]) -> Fields {
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
    if let Some(first) = fields.first_mut() {
        first.gap.insert_str(0, lead);
    }
    fields.push(Field {
        gap: chars[end..].iter().collect(),
        text: String::new(),
    });
    Fields(fields)
}

/// Con el cursor al final del nombre de un efecto dentro de
/// `fx("nombre", { ... })`, sustituye la tabla de parámetros que sigue en
/// la línea por la del efecto nuevo ([`api::Effect::table_with`]: conserva
/// los valores de los parámetros que tienen los dos, y los nuevos salen
/// por defecto) y selecciona su primer campo. Lo que quede del nombre anterior hasta
/// las comillas se borra. Si tras el nombre no hay `", { ... }` en la
/// línea, no toca nada y devuelve `None`.
pub fn replace_fx_table(editor: &mut TextArea, effect: &api::Effect) -> Option<Fields> {
    let (row, col) = (editor.cursor().0, editor.cursor().1);
    let chars: Vec<char> = editor.lines()[row].chars().collect();
    let skip_spaces = |mut i: usize| {
        while chars.get(i) == Some(&' ') {
            i += 1;
        }
        i
    };
    // Resto del nombre anterior, comillas, coma y la tabla.
    let mut name_end = col;
    while chars
        .get(name_end)
        .is_some_and(|c| c.is_ascii_alphanumeric() || *c == '_')
    {
        name_end += 1;
    }
    if !matches!(chars.get(name_end), Some('"' | '\'')) {
        return None;
    }
    let comma = skip_spaces(name_end + 1);
    if chars.get(comma) != Some(&',') {
        return None;
    }
    let open = skip_spaces(comma + 1);
    if chars.get(open) != Some(&'{') {
        return None;
    }
    let mut depth = 0;
    let close = (open..chars.len()).find(|&i| {
        match chars[i] {
            '{' => depth += 1,
            '}' => depth -= 1,
            _ => {}
        }
        depth == 0
    })?;

    let old: String = chars[open + 1..close].iter().collect();
    let table = effect.table_with(&api::table_items(&old));
    let (text, marks) = api::expand_template(&table);
    editor.cancel_selection();
    editor.move_cursor(CursorMove::Jump(row as u16, open as u16));
    editor.delete_str(close - open + 1);
    editor.insert_str(&text);
    editor.move_cursor(CursorMove::Jump(row as u16, col as u16));
    editor.delete_str(name_end - col);
    let lead: String = chars[name_end..open].iter().collect();
    let mut fields = build_fields(&lead, &text, &marks);
    // El final queda tras el `)` de `fx(...)`, como en su plantilla.
    if chars.get(close + 1) == Some(&')')
        && let Some(end) = fields.0.last_mut()
    {
        end.gap.push(')');
    }
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

/// Texto seleccionado (puede ocupar varias líneas), o `None` si no hay
/// selección o está vacía.
pub fn selected_text(editor: &TextArea) -> Option<String> {
    let (a, b) = editor.selection_range()?;
    let ((r1, c1), (r2, c2)) = if a <= b { (a, b) } else { (b, a) };
    let mut text = String::new();
    for (row, line) in editor.lines().iter().enumerate().take(r2 + 1).skip(r1) {
        let chars: Vec<char> = line.chars().collect();
        let end = if row == r2 {
            c2.min(chars.len())
        } else {
            chars.len()
        };
        let start = if row == r1 { c1.min(end) } else { 0 };
        text.extend(&chars[start..end]);
        if row < r2 {
            text.push('\n');
        }
    }
    (!text.is_empty()).then_some(text)
}

/// La selección o, si no hay, la línea del cursor entera (con su salto de
/// línea, como `yy` de vim).
pub fn selection_or_line(editor: &TextArea) -> String {
    selected_text(editor).unwrap_or_else(|| {
        let row = editor.cursor().0;
        format!("{}\n", editor.lines().get(row).map_or("", String::as_str))
    })
}

/// Quita la sangría común de las líneas con texto.
pub fn dedent(text: &str) -> String {
    let indent = text
        .lines()
        .filter(|l| !l.trim().is_empty())
        .map(|l| l.chars().take_while(|c| c.is_whitespace()).count())
        .min()
        .unwrap_or(0);
    let mut out: Vec<String> = text
        .lines()
        .map(|l| l.chars().skip(indent).collect())
        .collect();
    if text.ends_with('\n') {
        out.push(String::new());
    }
    out.join("\n")
}

/// Pega texto en el cursor. Si acaba en salto de línea (líneas enteras),
/// va al principio de la línea del cursor y la empuja hacia abajo, como
/// `P` de vim.
pub fn paste(editor: &mut TextArea, text: &str) {
    editor.cancel_selection();
    if text.ends_with('\n') {
        editor.move_cursor(CursorMove::Head);
    }
    editor.insert_str(text);
}

/// Saltos de línea de cualquier terminal (`\r\n` y `\r`) como `\n`.
pub fn normalize_newlines(text: &str) -> String {
    text.replace("\r\n", "\n").replace('\r', "\n")
}

/// Texto pegado sin bracketed paste: el terminal manda lo pegado como
/// teclas, todas en la misma ráfaga. Si la ráfaga es solo texto (letras,
/// Enter, Ctrl-J o Tab) y trae algún salto de línea o tabulador, es un
/// pegado: Enter (`\r`) y Ctrl-J (`\n`) son saltos de línea. Una tecla
/// sola no lo es nunca (se escribe a mano).
pub fn burst_text(inputs: &[Input]) -> Option<String> {
    if inputs.len() < 2 {
        return None;
    }
    let mut text = String::new();
    let mut breaks = false;
    for input in inputs {
        match input.key {
            Key::Enter if !input.ctrl && !input.alt => text.push('\n'),
            Key::Char('j' | 'm') if input.ctrl && !input.alt => text.push('\n'),
            Key::Tab if !input.ctrl && !input.alt => text.push('\t'),
            Key::Char(c) if !input.ctrl && !input.alt => {
                text.push(c);
                continue;
            }
            _ => return None,
        }
        breaks = true;
    }
    breaks.then_some(text)
}

/// Lleva el cursor al principio del texto o a su final (Ctrl-Inicio y
/// Ctrl-Fin). Con `select`, extiende la selección (o la empieza); sin él,
/// la quita.
pub fn jump_to_edge(editor: &mut TextArea, end: bool, select: bool) {
    if !select {
        editor.cancel_selection();
    } else if editor.selection_range().is_none() {
        editor.start_selection();
    }
    if end {
        editor.move_cursor(CursorMove::Bottom);
        editor.move_cursor(CursorMove::End);
    } else {
        editor.move_cursor(CursorMove::Top);
        editor.move_cursor(CursorMove::Head);
    }
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
    fn effect_tables_are_regenerated() {
        let mut e = editor("t:fx(\"bitcrush\", { type = \"lowpass\", cutoff = 3000 }):gain(1)");
        at(&mut e, 0, 14);
        let bitcrush = api::effect("bitcrush").unwrap();
        let mut fields = replace_fx_table(&mut e, bitcrush).unwrap();
        assert_eq!(
            e.lines(),
            ["t:fx(\"bitcrush\", { bits = 6, downsample = 2 }):gain(1)"]
        );
        assert_eq!(selected(&e), "6");
        assert_eq!(fields.next(&mut e), Some(Selected::Field));
        assert_eq!(selected(&e), "2");
        assert_eq!(fields.next(&mut e), Some(Selected::End));
        let end = e.lines()[0].find("}):").unwrap() + 2;
        assert_eq!(e.cursor(), (0, end));
        // El resto del nombre anterior tras el cursor se borra.
        // y los parámetros comunes conservan su valor.
        let mut e = editor("fx(\"highpasster\", { type = \"lowpass\", cutoff = 3000, q = 4 })");
        at(&mut e, 0, 12);
        let highpass = api::effect("highpass").unwrap();
        let mut fields = replace_fx_table(&mut e, highpass).unwrap();
        assert_eq!(e.lines(), ["fx(\"highpass\", { cutoff = 3000, q = 4 })"]);
        assert_eq!(selected(&e), "3000");
        fields.next(&mut e);
        assert_eq!(selected(&e), "4");
        // Sin tabla detrás, nada.
        let mut e = editor("fx(\"lowpass\")");
        at(&mut e, 0, 11);
        assert!(replace_fx_table(&mut e, highpass).is_none());
        assert_eq!(e.lines(), ["fx(\"lowpass\")"]);
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
