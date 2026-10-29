//! La vista de ocupación por paso en la aplicación (ver
//! [`crate::occupancy`]): si se ve, la rejilla de lo que suena y rellenar
//! un hueco desde ella.
//!
//! Rellenar es como el tracker editable (fase N): con el cursor sobre un
//! silencio de una pista, Enter abre la línea de órdenes con `rellenar` y
//! un golpe de esa pista (el anterior, tal como está escrito), y la orden
//! lo escribe en el token `.` de ese paso del código Lua (solo esos
//! caracteres), en un paso de deshacer, y carga. Lo que no está escrito tal
//! cual (`euclid`, `rotate`...), los instrumentos y los granos y un texto
//! que no es el cargado no se editan, y la línea de estado dice por qué.
//!
//! Teclas: ←→ paso, ↑↓ pista (la seleccionada lo sigue), Tab el hueco
//! siguiente, Enter rellena, Esc sale.

use ratatui_textarea::CursorMove;
use script::TokenSpan;

use super::tracker_edit::with_step_fx;
use super::{App, Flow, Input, Key};
use crate::model::ViewModel;
use crate::occupancy::{self, Cell, Grid, Source};

/// Estado de la vista de ocupación.
#[derive(Debug, Default)]
pub struct Occupancy {
    /// Se ve (la paleta la enseña y la esconde).
    pub visible: bool,
    /// El cursor para rellenar, si está abierto.
    pub cursor: Option<OccupancyCursor>,
    /// El paso que se rellena en la línea de órdenes.
    pending: Option<PendingFill>,
}

/// El cursor de la vista: fila (pista, en el orden del script) y columna
/// (paso de la página).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OccupancyCursor {
    pub row: usize,
    pub col: usize,
}

/// El paso que se rellena: pista, vuelta del ciclo y paso (los de cuando
/// se pulsó Enter, aunque luego la página siga).
#[derive(Debug, Clone, PartialEq, Eq)]
struct PendingFill {
    track: String,
    lap: usize,
    step: usize,
}

/// Orden de la línea de órdenes que rellena el paso.
pub const COMMAND: &str = "rellenar";

/// Las teclas del cursor.
const HELP: &str =
    "ocupación: ←→ paso · ↑↓ pista · Tab hueco siguiente · Enter rellenar · Esc sale";

impl App {
    /// La rejilla de la página que se oye, si la vista se ve.
    pub(super) fn occupancy_grid(&self, view: &ViewModel) -> Option<Grid> {
        if !self.occupancy.visible {
            return None;
        }
        let song = self.host.song()?;
        let audible: Vec<bool> = (0..view.tracks.len()).map(|i| view.audible(i)).collect();
        // Lo que se oye, con el mismo retardo a mano que el resaltado.
        let heard = self.status.and_then(|s| {
            let extra = self.highlight_delay_ms * s.bpm * 4.0 / 60_000.0;
            s.heard.map(|h| h - extra)
        });
        let slot = |name: &str| self.host.track_slot(name);
        Some(occupancy::grid(&Source {
            song,
            tracks: self.tracks(),
            cursors: &self.cursors,
            status: self.status,
            heard,
            slot: &slot,
            audible: &audible,
        }))
    }
}

impl App {
    /// Enseña o esconde la vista de ocupación.
    pub fn set_occupancy(&mut self, visible: bool) {
        self.occupancy.visible = visible;
    }

    /// «Vista de ocupación» en la paleta: la enseña o la esconde.
    pub(super) fn toggle_occupancy(&mut self) {
        self.set_occupancy(!self.occupancy.visible);
        self.message = if self.occupancy.visible {
            "vista de ocupación: qué pista suena en cada paso; Σ cuántas, los huecos resaltados"
                .into()
        } else {
            "vista de ocupación oculta".into()
        };
    }
}

impl App {
    /// La rejilla tal como se ve ahora (con la vista abierta).
    fn current_grid(&self) -> Option<Grid> {
        self.view().occupancy
    }

    /// «Rellenar un hueco desde la vista de ocupación»: abre la vista y su
    /// cursor en la pista seleccionada, en el primer hueco desde el paso
    /// que se oye (o en ese paso).
    pub(super) fn open_occupancy_cursor(&mut self) {
        self.set_occupancy(true);
        self.tracker_edit = None;
        self.pending_edit = None;
        let Some(grid) = self.current_grid().filter(|g| !g.rows.is_empty()) else {
            self.message = "no hay pistas en la vista de ocupación".into();
            return;
        };
        let from = grid.current.unwrap_or(0);
        let gaps = grid.gaps();
        let col = gaps
            .iter()
            .find(|&&c| c >= from)
            .or(gaps.first())
            .copied()
            .unwrap_or(from);
        let row = self.selected.min(grid.rows.len() - 1);
        let cursor = OccupancyCursor { row, col };
        self.occupancy.cursor = Some(cursor);
        self.message = self.fill_summary(&grid, cursor);
    }

    /// Una tecla con el cursor de la vista abierto.
    pub(super) fn handle_occupancy_cursor(&mut self, input: Input) -> Flow {
        let (Some(mut cursor), Some(grid)) = (self.occupancy.cursor, self.current_grid()) else {
            self.occupancy.cursor = None;
            return Flow::Continue;
        };
        let (rows, cols) = (grid.rows.len(), grid.steps());
        if rows == 0 || cols == 0 {
            self.occupancy.cursor = None;
            return Flow::Continue;
        }
        match input.key {
            Key::Esc => {
                self.occupancy.cursor = None;
                self.occupancy.pending = None;
                self.message = "fuera de la vista de ocupación".into();
                return Flow::Continue;
            }
            Key::Left => cursor.col = (cursor.col + cols - 1) % cols,
            Key::Right => cursor.col = (cursor.col + 1) % cols,
            Key::Up => cursor.row = (cursor.row + rows - 1) % rows,
            Key::Down => cursor.row = (cursor.row + 1) % rows,
            Key::Tab => {
                let gaps = grid.gaps();
                if let Some(&col) = gaps.iter().find(|&&c| c > cursor.col).or(gaps.first()) {
                    cursor.col = col;
                }
            }
            Key::Enter => {
                self.start_fill(&grid, cursor);
                return Flow::Continue;
            }
            _ => return Flow::Continue,
        }
        cursor.row = cursor.row.min(rows - 1);
        cursor.col = cursor.col.min(cols - 1);
        self.selected = cursor.row;
        self.occupancy.cursor = Some(cursor);
        self.message = self.fill_summary(&grid, cursor);
        Flow::Continue
    }

    /// Enter: la línea de órdenes con `rellenar` y el golpe que se propone.
    fn start_fill(&mut self, grid: &Grid, cursor: OccupancyCursor) {
        match self.fill_target(grid, cursor) {
            Ok((_, track, lap, step)) => {
                let hit = self.proposed_hit(&track, lap, step);
                self.prompt = Some(format!("{COMMAND} {hit}"));
                self.occupancy.pending = Some(PendingFill { track, lap, step });
                self.message = "rellenar: el golpe que va en el paso, como en steps() (1, 2@0.5, kick...); Enter escribe y carga, Esc cancela".into();
            }
            Err(why) => self.message = why,
        }
    }

    /// La orden `rellenar <golpe>`: lo escribe en el paso que se estaba
    /// rellenando (o en el del cursor).
    pub(super) fn run_fill_command(&mut self, rest: &str) {
        let pending = self.occupancy.pending.take();
        let target = match pending {
            Some(p) => Ok((p.track, p.lap, p.step)),
            None => match (self.occupancy.cursor, self.current_grid()) {
                (Some(cursor), Some(grid)) => self
                    .fill_target(&grid, cursor)
                    .map(|(_, track, lap, step)| (track, lap, step)),
                _ => Err("rellenar: abre antes el cursor («Rellenar un hueco desde la vista de ocupación», en la paleta)".into()),
            },
        };
        match target {
            Ok((track, lap, step)) => self.write_fill(&track, lap, step, rest.trim()),
            Err(why) => self.message = why,
        }
    }

    /// Escribe `hit` en el token del silencio del paso `step` de la vuelta
    /// `lap` de `track`, en un paso de deshacer, y carga.
    pub(super) fn write_fill(&mut self, track: &str, lap: usize, step: usize, hit: &str) {
        let span = match self.silence_span(track, lap, step) {
            Ok(span) => span,
            Err(why) => {
                self.message = why;
                return;
            }
        };
        if hit.is_empty() || hit.split_whitespace().count() != 1 {
            self.message = "rellenar: un golpe, como en steps() (1, 2@0.5, kick...)".into();
            return;
        }
        // Que se entienda antes de tocar el texto (con las etiquetas de la
        // pista): una errata no estropea el código.
        let (labels, slices) = self
            .tracks()
            .iter()
            .find(|t| t.name == track)
            .map_or((Vec::new(), 0), |t| (t.labels.clone(), t.slices.len()));
        match engine::notation::parse_step_with(hit, &labels, Some(&script::step_fx::resolve)) {
            Ok(Some(parsed)) if parsed.slice >= slices => {
                self.message = format!(
                    "rellenar: «{hit}» es el corte {}, y {track} tiene {slices}",
                    parsed.slice + 1
                );
                return;
            }
            Ok(Some(_)) => {}
            Ok(None) => {
                self.message = format!("rellenar: «{hit}» es un silencio; escribe un golpe");
                return;
            }
            Err(why) => {
                self.message = format!("rellenar: {why}");
                return;
            }
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
            editor.insert_str(hit);
        });
        self.load();
        if self.error.is_none() {
            self.message = format!(
                "{track} · paso {}: «{hit}», entra en el compás siguiente (Ctrl-Z lo deshace)",
                step + 1
            );
        }
    }

    /// La pista, la vuelta y el paso de la celda del cursor, si se puede
    /// rellenar; si no, por qué.
    fn fill_target(
        &self,
        grid: &Grid,
        cursor: OccupancyCursor,
    ) -> Result<(TokenSpan, String, usize, usize), String> {
        let row = grid.rows.get(cursor.row).ok_or("esa pista ya no existe")?;
        let cell = *row.cells.get(cursor.col).ok_or("ese paso no existe")?;
        let (lap, step) = row.at[cursor.col];
        match cell {
            Cell::Silence => {}
            Cell::Hit { .. } => {
                return Err(format!(
                    "{} ya suena en el paso {}: cámbialo en el tracker",
                    row.name,
                    step + 1
                ));
            }
            Cell::Held => {
                return Err(format!(
                    "el paso {} de {} sigue la nota de antes (_): cámbialo en el código",
                    step + 1,
                    row.name
                ));
            }
            Cell::Dropped => {
                return Err(format!(
                    "el paso {} de {} tiene un golpe escrito que chance o every quitan en esta vuelta",
                    step + 1,
                    row.name
                ));
            }
        }
        let span = self.silence_span(&row.name, lap, step)?;
        Ok((span, row.name.clone(), lap, step))
    }

    /// Dónde está escrito el silencio del paso; o por qué no se puede
    /// escribir en él.
    fn silence_span(&self, track: &str, lap: usize, step: usize) -> Result<TokenSpan, String> {
        if self.loaded_text.as_deref() != Some(self.text().as_str()) {
            return Err(
                "el texto no es el que suena: carga antes (Ctrl-L) para rellenar desde la vista"
                    .into(),
            );
        }
        let def = self
            .tracks()
            .iter()
            .find(|t| t.name == track)
            .ok_or("esa pista ya no existe")?;
        if def.instrument.is_some() || def.granular.is_some() {
            return Err(format!(
                "«{}» es {}: sus pasos se escriben en el código",
                def.name,
                match def.instrument {
                    Some(i) if i.synth.is_some() => "un sintetizador",
                    Some(_) => "un instrumento",
                    None => "de granos",
                }
            ));
        }
        let lap = lap % def.bars.len().max(1);
        let slot = def
            .bars
            .get(lap)
            .and_then(|bar| bar.get(step))
            .ok_or("ese paso no existe")?;
        if slot.is_some() {
            return Err(format!("{track} ya suena en el paso {}", step + 1));
        }
        let span = def
            .spans
            .get(lap)
            .and_then(|spans| spans.get(step).copied().flatten())
            .ok_or(
                "ese paso no está escrito tal cual en el texto (sale de euclid, rotate, \
                 una cadena hecha a trozos...): cámbialo en el código",
            )?;
        let token = self.span_text(span);
        if token != "." {
            return Err(format!(
                "el paso {} está escrito «{token}», no «.»: cámbialo en el código",
                step + 1
            ));
        }
        Ok(span)
    }

    /// El texto de un token del editor.
    fn span_text(&self, span: TokenSpan) -> String {
        self.editor
            .lines()
            .get(span.row)
            .map(|l| {
                l.chars()
                    .skip(span.start)
                    .take(span.end.saturating_sub(span.start))
                    .collect()
            })
            .unwrap_or_default()
    }

    /// El golpe que se propone para rellenar: el anterior de la pista en
    /// esa vuelta (o el siguiente), como está escrito y sin sus efectos de
    /// paso; si no hay, `1`.
    fn proposed_hit(&self, track: &str, lap: usize, step: usize) -> String {
        let Some(def) = self.tracks().iter().find(|t| t.name == track) else {
            return "1".into();
        };
        let lap = lap % def.bars.len().max(1);
        let (Some(bar), Some(spans)) = (def.bars.get(lap), def.spans.get(lap)) else {
            return "1".into();
        };
        let length = bar.len();
        (1..length)
            .map(|d| (step + length - d) % length)
            .find_map(|k| {
                bar[k]?;
                let span = spans.get(k).copied().flatten()?;
                let token = with_step_fx(&self.span_text(span), "");
                (!token.is_empty()).then_some(token)
            })
            .unwrap_or_else(|| "1".into())
    }

    /// Lo que dice la línea de estado de la celda del cursor.
    fn fill_summary(&self, grid: &Grid, cursor: OccupancyCursor) -> String {
        let name = grid
            .rows
            .get(cursor.row)
            .map_or(String::new(), |r| r.name.clone());
        let count = grid.counts.get(cursor.col).copied().unwrap_or(0);
        let here = format!(
            "{name} · paso {} de la página · {}",
            cursor.col + 1,
            match count {
                0 => "hueco".to_string(),
                1 => "suena 1 pista".to_string(),
                n => format!("suenan {n} pistas"),
            }
        );
        match self.fill_target(grid, cursor) {
            Ok(_) => format!("{here} · {HELP}"),
            Err(why) => format!("{here}: {why}"),
        }
    }
}
