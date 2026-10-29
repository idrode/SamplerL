//! Panel «Tonalidad del sample» (sin estado de la aplicación): lo que se
//! enseña ([`KeyView`]), cómo se dibuja ([`KeyPopup`]) y los cambios de
//! texto de sus acciones, puros para probarlos sin terminal: la raíz del
//! `instrument()` de un sample ([`with_instrument_root`]) y la línea
//! `key(...)` ([`with_key_line`]).

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Style, Stylize};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Clear, Paragraph, Widget, Wrap};

use crate::ui::{accent, dim, warning_color};

/// Lo que enseña el panel.
#[derive(Debug, Clone, PartialEq)]
pub struct KeyView {
    /// El sample, como se escribe en `sample("...")`.
    pub sample: String,
    pub state: KeyState,
    /// La candidata elegida (de 0 a 2).
    pub selected: usize,
    /// La tonalidad de la canción (`key()`), si la tiene.
    pub song_key: Option<String>,
}

/// En qué va el análisis.
#[derive(Debug, Clone, PartialEq)]
pub enum KeyState {
    Analyzing,
    /// Nada que analizar (silencio, muy corto) o un error al leerlo.
    Nothing(String),
    Done {
        /// Las tres más probables: nombre («A menor») y correlación.
        candidates: Vec<(String, f32)>,
        confidence: f32,
        low: bool,
        /// La raíz propuesta («A3»).
        root: String,
    },
}

/// Sitio del panel: centrado arriba, del alto de lo que enseña.
pub fn popup_area(screen: Rect, view: &KeyView) -> Rect {
    let width = screen.width.saturating_sub(4).min(64);
    // Bordes y la línea de las teclas; con el análisis, las tres, la raíz,
    // el aviso, un hueco y las tres acciones.
    let lines = match &view.state {
        KeyState::Done { low, .. } => 8 + u16::from(*low),
        _ => 1,
    };
    let height = (lines + 3).min(screen.height);
    Rect {
        x: screen.x + (screen.width - width) / 2,
        y: screen.y + (screen.height - height).min(2),
        width,
        height,
    }
}

/// El panel: las tres tonalidades con una barra de su correlación, la
/// raíz, la confianza (con un aviso si es baja) y las acciones.
pub struct KeyPopup<'a>(pub &'a KeyView);

impl Widget for KeyPopup<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let view = self.0;
        Clear.render(area, buf);
        let block = Block::bordered()
            .border_style(Style::new().fg(accent()))
            .title(format!(" tonalidad · {} ", view.sample));
        let inner = block.inner(area).width as usize;
        let mut lines: Vec<Line> = Vec::new();
        match &view.state {
            KeyState::Analyzing => lines.push(Line::from("analizando…").fg(dim())),
            KeyState::Nothing(why) => lines.push(Line::from(why.as_str()).fg(warning_color())),
            KeyState::Done {
                candidates,
                confidence,
                low,
                root,
            } => {
                let bar = inner.saturating_sub(24);
                for (i, (name, score)) in candidates.iter().enumerate() {
                    let marker = if i == view.selected { "▸" } else { " " };
                    let mut label = Span::from(format!("{name:<10} "));
                    if i == view.selected {
                        label = label.bold().fg(accent());
                    }
                    lines.push(Line::from(vec![
                        Span::from(format!("{marker}{} ", i + 1))
                            .fg(accent())
                            .bold(),
                        label,
                        Span::from(crate::ui::meter(score.clamp(0.0, 1.0), bar)).fg(accent()),
                        Span::raw(format!(" {score:.2}")),
                    ]));
                }
                lines.push(Line::from(vec![
                    Span::raw(format!(" raíz {root} · confianza {confidence:.2}")),
                    Span::from(match &view.song_key {
                        Some(k) => format!(" · canción: {k}"),
                        None => String::new(),
                    })
                    .fg(dim()),
                ]));
                if *low {
                    lines.push(
                        Line::from(
                            " ⚠ confianza baja: material percusivo o sin una tonalidad clara",
                        )
                        .fg(warning_color()),
                    );
                }
                lines.push(Line::from(""));
                for (key, what) in [
                    ("r", "usar como raíz del instrumento"),
                    ("i", "insertar sample:instrument({ root = ... })"),
                    ("k", "proponer como tonalidad de la canción: key(...)"),
                ] {
                    lines.push(Line::from(vec![
                        Span::from(format!(" {key} ")).fg(accent()).bold(),
                        Span::raw(what),
                    ]));
                }
            }
        }
        lines.push(Line::from(" ↑↓ elegir · Esc cerrar").fg(dim()));
        Paragraph::new(lines)
            .wrap(Wrap { trim: false })
            .block(block)
            .render(area, buf);
    }
}

/// El texto con `root = "<root>"` en el `instrument()` del sample
/// `sample`: `sample("x.wav"):instrument(...)`, o una variable de
/// `local v = sample("x.wav")` seguida de `v:instrument(...)`. Cambia la
/// raíz si la hay, o la añade la primera de la tabla (o la tabla, si no
/// hay). `None` si no se encuentra ese `instrument()`.
pub fn with_instrument_root(text: &str, sample: &str, root: &str) -> Option<String> {
    let needle = format!("sample(\"{sample}\"");
    let mut search = 0;
    while let Some(found) = text[search..].find(&needle) {
        let start = search + found;
        search = start + needle.len();
        let open = start + "sample".len();
        let close = matching(text, open)?;
        // Directamente: `sample("x.wav"):instrument(`.
        let after = text[close + 1..].trim_start();
        if let Some(rest) = after.strip_prefix(":instrument(") {
            let paren = text.len() - rest.len() - 1;
            return set_root(text, paren, root);
        }
        // En una variable: `local v = sample("x.wav")...` y `v:instrument(`.
        let line_start = text[..start].rfind('\n').map_or(0, |i| i + 1);
        let head = text[line_start..start].trim();
        let Some(name) = head
            .strip_prefix("local ")
            .and_then(|h| h.strip_suffix('='))
            .map(str::trim)
            .filter(|n| !n.is_empty() && n.chars().all(|c| c.is_alphanumeric() || c == '_'))
        else {
            continue;
        };
        let call = format!("{name}:instrument(");
        let mut from = close;
        while let Some(i) = text[from..].find(&call) {
            let at = from + i;
            from = at + call.len();
            // Que no sea el final de otro nombre (`xv:instrument`).
            let before = text[..at].chars().next_back();
            if before.is_some_and(|c| c.is_alphanumeric() || c == '_') {
                continue;
            }
            return set_root(text, at + call.len() - 1, root);
        }
    }
    None
}

/// El texto con `key("<tónica>", "<modo>")` (la línea que se pasa en
/// `call`): sustituye la primera línea que empieza por `key(`, o la pone
/// detrás de `bpm(...)` (o al principio).
pub fn with_key_line(text: &str, call: &str) -> String {
    let mut lines: Vec<String> = text.lines().map(str::to_string).collect();
    if let Some(row) = lines
        .iter()
        .position(|l| l.trim_start().starts_with("key("))
    {
        lines[row] = call.to_string();
    } else {
        let at = lines
            .iter()
            .position(|l| l.trim_start().starts_with("bpm("))
            .map_or(0, |r| r + 1);
        lines.insert(at, call.to_string());
    }
    let mut out = lines.join("\n");
    if text.ends_with('\n') {
        out.push('\n');
    }
    out
}

/// Pone la raíz en el `instrument(` cuyo paréntesis abre en `paren`.
fn set_root(text: &str, paren: usize, root: &str) -> Option<String> {
    let close = matching(text, paren)?;
    let inside = &text[paren + 1..close];
    let value = format!("\"{root}\"");
    let trimmed = inside.trim();
    if trimmed.is_empty() {
        return Some(format!(
            "{}{{ root = {value} }}{}",
            &text[..paren + 1],
            &text[close..]
        ));
    }
    if !trimmed.starts_with('{') {
        return None;
    }
    let brace = paren + 1 + inside.find('{')?;
    let end = matching(text, brace)?;
    // `root = ...` en el primer nivel de la tabla.
    let mut depth = 0;
    let mut i = brace + 1;
    let bytes = text.as_bytes();
    while i < end {
        match bytes[i] {
            b'{' | b'(' => depth += 1,
            b'}' | b')' => depth -= 1,
            b'"' => i = skip_string(text, i),
            _ if depth == 0 && text[i..].starts_with("root") => {
                let before = text[..i].chars().next_back();
                let rest = text[i + 4..].trim_start();
                if !before.is_some_and(|c| c.is_alphanumeric() || c == '_')
                    && let Some(after) = rest.strip_prefix('=')
                {
                    // `after` acaba donde el texto: su comienzo es su índice.
                    let from = text.len() - after.trim_start().len();
                    let to = value_end(text, from, end);
                    return Some(format!("{}{value}{}", &text[..from], &text[to..]));
                }
            }
            _ => {}
        }
        i += 1;
    }
    // Sin raíz: la primera de la tabla.
    let empty = text[brace + 1..end].trim().is_empty();
    let insert = if empty {
        format!(" root = {value} ")
    } else {
        format!(" root = {value},")
    };
    let rest = &text[brace + 1..];
    let rest = if empty { rest.trim_start() } else { rest };
    Some(format!("{}{insert}{rest}", &text[..brace + 1]))
}

/// Dónde acaba el valor que empieza en `from` (hasta una coma o el cierre
/// de la tabla en `end`, en su nivel), sin el espacio de detrás.
fn value_end(text: &str, from: usize, end: usize) -> usize {
    let bytes = text.as_bytes();
    let mut depth = 0;
    let mut i = from;
    while i < end {
        match bytes[i] {
            b'{' | b'(' => depth += 1,
            b'}' | b')' => depth -= 1,
            b'"' => i = skip_string(text, i),
            b',' | b'\n' if depth == 0 => break,
            _ => {}
        }
        i += 1;
    }
    from + text[from..i].trim_end().len()
}

/// El índice de la comilla que cierra la cadena que abre en `i`.
fn skip_string(text: &str, i: usize) -> usize {
    let bytes = text.as_bytes();
    let mut j = i + 1;
    while j < bytes.len() {
        match bytes[j] {
            b'\\' => j += 1,
            b'"' => return j,
            _ => {}
        }
        j += 1;
    }
    bytes.len()
}

/// El índice del paréntesis o la llave que cierra el que abre en `open`.
fn matching(text: &str, open: usize) -> Option<usize> {
    let bytes = text.as_bytes();
    let (up, down) = match bytes.get(open)? {
        b'(' => (b'(', b')'),
        b'{' => (b'{', b'}'),
        _ => return None,
    };
    let mut depth = 0;
    let mut i = open;
    while i < bytes.len() {
        match bytes[i] {
            b'"' => i = skip_string(text, i),
            c if c == up => depth += 1,
            c if c == down => {
                depth -= 1;
                if depth == 0 {
                    return Some(i);
                }
            }
            _ => {}
        }
        i += 1;
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_root_is_changed_or_added_in_the_instrument() {
        let cases = [
            (
                "local p = sample(\"a.wav\"):instrument({ root = \"C3\", voices = 2 })\n",
                "local p = sample(\"a.wav\"):instrument({ root = \"A3\", voices = 2 })\n",
            ),
            (
                "local p = sample(\"a.wav\"):instrument({ voices = 2 })",
                "local p = sample(\"a.wav\"):instrument({ root = \"A3\", voices = 2 })",
            ),
            (
                "local p = sample(\"a.wav\"):instrument()",
                "local p = sample(\"a.wav\"):instrument({ root = \"A3\" })",
            ),
            (
                "local p = sample(\"a.wav\"):instrument({})",
                "local p = sample(\"a.wav\"):instrument({ root = \"A3\" })",
            ),
            // En varias líneas, con un número MIDI y opciones en sample().
            (
                "local p = sample(\"a.wav\", { trim = true }):instrument({\n  loop = { 1, 2 },\n  root = 57,\n})",
                "local p = sample(\"a.wav\", { trim = true }):instrument({\n  loop = { 1, 2 },\n  root = \"A3\",\n})",
            ),
            // Por una variable.
            (
                "local s = sample(\"a.wav\")\nlocal p = s:instrument({ adsr = { 1, 2, 1, 3 } })",
                "local s = sample(\"a.wav\")\nlocal p = s:instrument({ root = \"A3\", adsr = { 1, 2, 1, 3 } })",
            ),
        ];
        for (text, expected) in cases {
            assert_eq!(
                with_instrument_root(text, "a.wav", "A3").as_deref(),
                Some(expected),
                "{text}"
            );
        }
        // Otro sample, o sin instrument(): nada.
        assert_eq!(
            with_instrument_root("local p = sample(\"b.wav\"):instrument()", "a.wav", "A3"),
            None
        );
        assert_eq!(
            with_instrument_root("local p = sample(\"a.wav\"):slice(8)", "a.wav", "A3"),
            None
        );
        // `sroot` no es `root`.
        assert_eq!(
            with_instrument_root(
                "local p = sample(\"a.wav\"):instrument({ sroot = 1 })",
                "a.wav",
                "A3"
            )
            .as_deref(),
            Some("local p = sample(\"a.wav\"):instrument({ root = \"A3\", sroot = 1 })")
        );
    }

    #[test]
    fn the_key_line_is_replaced_or_goes_after_bpm() {
        assert_eq!(
            with_key_line("bpm(120)\ntrack(\"a\")\n", "key(\"A\", \"minor\")"),
            "bpm(120)\nkey(\"A\", \"minor\")\ntrack(\"a\")\n"
        );
        assert_eq!(
            with_key_line(
                "bpm(120)\n  key(\"C\")\ntrack(\"a\")",
                "key(\"A\", \"minor\")"
            ),
            "bpm(120)\nkey(\"A\", \"minor\")\ntrack(\"a\")"
        );
        assert_eq!(
            with_key_line("track(\"a\")\n", "key(\"D\", \"major\")"),
            "key(\"D\", \"major\")\ntrack(\"a\")\n"
        );
    }
}
