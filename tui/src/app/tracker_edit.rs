//! Editar los efectos de un paso desde el tracker (fase N, parte 4). El
//! tracker sigue siendo una vista del texto: un cambio aquí se escribe en
//! el token de ese paso en el código Lua (solo sus caracteres: nada más del
//! texto se mueve ni se reformatea), en un paso de deshacer, y se carga
//! como Ctrl-L (entra al compás siguiente). Sale de las posiciones de cada
//! paso que guarda la carga (fase M): un paso que no está escrito tal cual
//! (`euclid`, `rotate`, una cadena hecha a trozos...) no se puede editar
//! desde aquí, y tampoco si el texto ya no es el cargado.
//!
//! Teclas: ↑↓ paso, ←→ pista, Enter escribe los efectos del paso en la
//! línea de órdenes (con los que tiene), Supr los quita, Esc sale. En la
//! línea, los efectos van como en las llaves, separados por espacios
//! (`lp=800,q=4 crush=6`) o con sus llaves (`{lp=800}{crush=6}`); vacía,
//! los quita.

use ratatui_textarea::CursorMove;
use script::TokenSpan;

use super::{App, Flow, Input, Key};

/// El cursor del tracker: pista (en el orden del script) y paso.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TrackerCursor {
    pub track: usize,
    pub row: usize,
}

/// El paso que se está editando en la línea de órdenes: pista, vuelta del
/// ciclo y paso (los de cuando se pulsó Enter, aunque luego suene otra
/// vuelta).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct PendingEdit {
    track: String,
    lap: usize,
    row: usize,
}

/// Orden de la línea de órdenes que escribe los efectos del paso.
pub const COMMAND: &str = "efectos";

impl App {
    /// Abre el cursor del tracker en la pista seleccionada, en el paso que
    /// suena (o el primero).
    pub fn open_tracker_edit(&mut self) {
        if self.tracks().is_empty() {
            self.message = "no hay pistas que editar".into();
            return;
        }
        let track = self.selected.min(self.tracks().len() - 1);
        let row = self
            .view_position(track)
            .unwrap_or(0)
            .min(self.track_rows(track).saturating_sub(1));
        self.tracker_edit = Some(TrackerCursor { track, row });
        self.occupancy.cursor = None;
        self.message = HELP.into();
    }

    /// Una tecla con el cursor del tracker abierto.
    pub(super) fn handle_tracker_edit(&mut self, input: Input) -> Flow {
        let Some(mut cursor) = self.tracker_edit else {
            return Flow::Continue;
        };
        let tracks = self.tracks().len();
        if tracks == 0 {
            self.tracker_edit = None;
            return Flow::Continue;
        }
        match input.key {
            Key::Esc => {
                self.tracker_edit = None;
                self.pending_edit = None;
                self.message = "fuera del tracker".into();
                return Flow::Continue;
            }
            Key::Up => cursor.row = cursor.row.saturating_sub(1),
            Key::Down => cursor.row += 1,
            Key::Left => cursor.track = (cursor.track + tracks - 1) % tracks,
            Key::Right => cursor.track = (cursor.track + 1) % tracks,
            Key::Enter => {
                self.start_step_edit(cursor);
                return Flow::Continue;
            }
            Key::Delete | Key::Backspace => {
                self.write_step_fx(cursor, None, "");
                return Flow::Continue;
            }
            _ => return Flow::Continue,
        }
        cursor.row = cursor
            .row
            .min(self.track_rows(cursor.track).saturating_sub(1));
        self.selected = cursor.track;
        self.tracker_edit = Some(cursor);
        self.message = self.step_summary(cursor);
        Flow::Continue
    }

    /// Enter: los efectos del paso en la línea de órdenes, para cambiarlos.
    fn start_step_edit(&mut self, cursor: TrackerCursor) {
        let (track, lap) = match self.step_target(cursor) {
            Ok((_, token, track, lap)) => {
                self.prompt = Some(format!("{COMMAND} {}", written_fx(&token)));
                (track, lap)
            }
            Err(why) => {
                self.message = why;
                return;
            }
        };
        self.pending_edit = Some(PendingEdit {
            track,
            lap,
            row: cursor.row,
        });
        self.message = "efectos del paso: lp=800,q=4 crush=6 (o {lp=800}{crush=6}); vacío los quita; Enter escribe y carga, Esc cancela".into();
    }

    /// La orden `efectos ...` de la línea: escribe los efectos en el paso
    /// que se estaba editando (o, sin él, en el del cursor).
    pub(super) fn run_step_fx_command(&mut self, rest: &str) {
        let pending = self.pending_edit.take();
        let Some(cursor) = self.tracker_edit else {
            self.message = "efectos: abre antes el tracker («Editar los efectos de un paso en el tracker», en la paleta)".into();
            return;
        };
        let at = pending.map(|p| (p.track, p.lap, p.row));
        let cursor = match &at {
            Some((name, _, row)) => TrackerCursor {
                track: self
                    .tracks()
                    .iter()
                    .position(|t| &t.name == name)
                    .unwrap_or(cursor.track),
                row: *row,
            },
            None => cursor,
        };
        self.write_step_fx(cursor, at.map(|(_, lap, _)| lap), rest);
    }

    /// Escribe `fx` (lo de la línea de órdenes; vacío quita los efectos)
    /// en el token del paso, en la vuelta `lap` (o la que suena), en un
    /// paso de deshacer, y carga.
    fn write_step_fx(&mut self, cursor: TrackerCursor, lap: Option<usize>, fx: &str) {
        let (span, token) = match self.step_target_in(cursor, lap) {
            Ok((span, token, _, _)) => (span, token),
            Err(why) => {
                self.message = why;
                return;
            }
        };
        let groups = match fx_groups(fx) {
            Ok(groups) => groups,
            Err(why) => {
                self.message = format!("efectos: {why}");
                return;
            }
        };
        let new = with_step_fx(&token, &groups);
        if new == token {
            self.message = "el paso ya tiene esos efectos".into();
            return;
        }
        // Que se entienda antes de tocar el texto (con las etiquetas de la
        // pista): una errata no estropea el código.
        let labels = self
            .tracks()
            .get(cursor.track)
            .map(|t| t.labels.clone())
            .unwrap_or_default();
        if let Err(why) =
            engine::notation::parse_step_with(&new, &labels, Some(&script::step_fx::resolve))
        {
            self.message = format!("efectos: {why}");
            return;
        }
        self.completion = None;
        self.marks.clear();
        let (row, start, end) = (span.row as u16, span.start as u16, span.end as u16);
        self.grouped(|editor| {
            editor.cancel_selection();
            editor.move_cursor(CursorMove::Jump(row, start));
            editor.start_selection();
            editor.move_cursor(CursorMove::Jump(row, end));
            editor.cut();
            editor.insert_str(&new);
        });
        self.load();
        if self.error.is_none() {
            self.message = if groups.is_empty() {
                format!(
                    "paso {} sin efectos: «{new}» (Ctrl-Z lo deshace)",
                    cursor.row + 1
                )
            } else {
                format!(
                    "paso {}: «{new}», entra en el compás siguiente (Ctrl-Z lo deshace)",
                    cursor.row + 1
                )
            };
        }
    }

    /// El token del paso del cursor en la vuelta que suena.
    fn step_target(
        &self,
        cursor: TrackerCursor,
    ) -> Result<(TokenSpan, String, String, usize), String> {
        self.step_target_in(cursor, None)
    }

    /// Dónde está escrito el paso del cursor (en la vuelta `lap`, o la que
    /// suena), su texto, la pista y la vuelta; o por qué no se puede
    /// editar.
    fn step_target_in(
        &self,
        cursor: TrackerCursor,
        lap: Option<usize>,
    ) -> Result<(TokenSpan, String, String, usize), String> {
        if self.loaded_text.as_deref() != Some(self.text().as_str()) {
            return Err(
                "el texto no es el que suena: carga antes (Ctrl-L) para editar desde el tracker"
                    .into(),
            );
        }
        let track = self
            .tracks()
            .get(cursor.track)
            .ok_or("esa pista ya no existe")?;
        if track.instrument.is_some() || track.granular.is_some() {
            return Err(format!(
                "«{}» es {}: sus golpes no tienen efectos por paso (usa :fx)",
                track.name,
                match track.instrument {
                    Some(i) if i.synth.is_some() => "un sintetizador",
                    Some(_) => "un instrumento",
                    None => "de granos",
                }
            ));
        }
        let laps = track.bars.len().max(1);
        let lap = lap.unwrap_or_else(|| {
            let global = self.status.map_or(0, super::section_global);
            track.lap(global).0 as usize
        }) % laps;
        let step = track
            .bars
            .get(lap)
            .and_then(|bar| bar.get(cursor.row))
            .ok_or("ese paso no existe")?;
        if step.is_none() {
            return Err(format!(
                "el paso {} es un silencio: escribe antes un golpe en el código",
                cursor.row + 1
            ));
        }
        let span = track
            .spans
            .get(lap)
            .and_then(|spans| spans.get(cursor.row).copied().flatten())
            .ok_or(
                "ese paso no está escrito tal cual en el texto (sale de euclid, rotate, chance, \
                 una cadena hecha a trozos...): cámbialo en el código",
            )?;
        let line: Vec<char> = self
            .editor
            .lines()
            .get(span.row)
            .map(|l| l.chars().collect())
            .unwrap_or_default();
        let token: String = line
            .get(span.start..span.end)
            .unwrap_or(&[])
            .iter()
            .collect();
        Ok((span, token, track.name.clone(), lap))
    }

    /// Lo que dice la línea de estado del paso del cursor.
    fn step_summary(&self, cursor: TrackerCursor) -> String {
        let name = self
            .tracks()
            .get(cursor.track)
            .map_or(String::new(), |t| t.name.clone());
        match self.step_target(cursor) {
            Ok((_, token, _, _)) => {
                format!("{name} · paso {}: «{token}» · {HELP}", cursor.row + 1)
            }
            Err(why) => format!("{name} · paso {}: {why}", cursor.row + 1),
        }
    }

    /// Pasos de la pista `track` (los de su vuelta).
    fn track_rows(&self, track: usize) -> usize {
        self.tracks().get(track).map_or(0, |t| t.length)
    }

    /// Paso de la pista `track` que suena, si suena.
    fn view_position(&self, track: usize) -> Option<usize> {
        let status = self.status.filter(|s| s.running)?;
        let t = self.tracks().get(track)?;
        Some(t.lap(super::section_global(status)).1)
    }
}

/// Las teclas del cursor del tracker.
const HELP: &str =
    "tracker: ↑↓ paso · ←→ pista · Enter efectos del paso · Supr los quita · Esc sale";

/// Los efectos escritos en un token, como se editan en la línea de
/// órdenes: lo de dentro de cada par de llaves, separado por espacios.
pub fn written_fx(token: &str) -> String {
    let mut out = Vec::new();
    let mut rest = token;
    while let Some(open) = rest.find('{') {
        let Some(close) = rest[open..].find('}') else {
            break;
        };
        // Sin espacios: en la línea, los espacios separan los efectos.
        out.push(
            rest[open + 1..open + close]
                .split_whitespace()
                .collect::<String>(),
        );
        rest = &rest[open + close + 1..];
    }
    out.join(" ")
}

/// Lo de la línea de órdenes como llaves: `lp=800,q=4 crush=6` o
/// `{lp=800}{crush=6}` dan `{lp=800,q=4}{crush=6}`; vacío, nada.
pub fn fx_groups(text: &str) -> Result<String, String> {
    let text = text.trim();
    if text.contains('{') || text.contains('}') {
        let compact: String = text.split_whitespace().collect::<Vec<_>>().join("");
        if compact.matches('{').count() != compact.matches('}').count() {
            return Err("las llaves no cierran".into());
        }
        return Ok(compact);
    }
    Ok(text
        .split_whitespace()
        .map(|group| format!("{{{group}}}"))
        .collect())
}

/// El token de un paso con otros efectos: quita sus llaves y pone `groups`
/// donde estaba la primera (o al final, si no tenía). El resto (el corte,
/// los sufijos y su orden) no cambia.
pub fn with_step_fx(token: &str, groups: &str) -> String {
    let mut out = String::new();
    let mut placed = false;
    let mut rest = token;
    while let Some(open) = rest.find('{') {
        out.push_str(&rest[..open]);
        if !placed {
            out.push_str(groups);
            placed = true;
        }
        match rest[open..].find('}') {
            Some(close) => rest = &rest[open + close + 1..],
            None => {
                rest = "";
                break;
            }
        }
    }
    out.push_str(rest);
    if !placed {
        out.push_str(groups);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tokens_keep_everything_but_their_braces() {
        assert_eq!(with_step_fx("3", "{lp=800}"), "3{lp=800}");
        assert_eq!(
            with_step_fx("kick'^7@0.5", "{crush=4}"),
            "kick'^7@0.5{crush=4}"
        );
        // En el sitio de las primeras llaves, y las demás fuera.
        assert_eq!(
            with_step_fx("2'{hp=300}^7{gain=0.5}r2", "{lp=800}"),
            "2'{lp=800}^7r2"
        );
        assert_eq!(with_step_fx("3{lp=800}{crush}", ""), "3");
        assert_eq!(with_step_fx("3{ lp = 800 }^7", ""), "3^7");
    }

    #[test]
    fn the_command_line_reads_both_forms() {
        assert_eq!(
            fx_groups("lp=800,q=4 crush=6").unwrap(),
            "{lp=800,q=4}{crush=6}"
        );
        assert_eq!(
            fx_groups("{lp=800} {crush=6}").unwrap(),
            "{lp=800}{crush=6}"
        );
        assert_eq!(fx_groups("  ").unwrap(), "");
        assert!(fx_groups("{lp=800").is_err());
        assert_eq!(written_fx("2'{hp = 300}^7{gain=0.5}"), "hp=300 gain=0.5");
        assert_eq!(written_fx("3"), "");
    }
}
