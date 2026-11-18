//! Los solapes de frecuencia en el panel «choques» (ver `crate::clash`): el
//! análisis en vivo, que solo pide muestras al motor mientras el panel se
//! ve, y el cursor del panel (elegir un choque con el teclado o el ratón).

use ratatui_textarea::{Input, Key};

use super::mouse::MouseAction;
use super::{App, Flow};
use crate::clash::{Live, MatrixHit, MatrixView};
use crate::layout::Tab;
use crate::model::{ClashRow, ClashTableView};

/// Lo del panel de choques que vive en la aplicación.
#[derive(Default)]
pub(crate) struct ClashState {
    /// El análisis en vivo, pedido («Solapes de frecuencia en vivo»).
    pub live: Option<Live>,
    /// El cursor está en el panel (se queda las flechas).
    pub cursor: bool,
    /// El choque elegido (de la lista, por gravedad) y su sugerencia
    /// elegida (←→).
    pub selected: usize,
    pub fix: usize,
    /// Muestras llegadas desde el último cuadro del análisis, y si toca.
    pub pending: usize,
    pub fresh: bool,
    /// Se vio en el último cuadro (con dónde quedó cada cosa, para el
    /// ratón).
    pub layout: Option<crate::clash::MatrixLayout>,
}

impl App {
    /// «Solapes de frecuencia en vivo»: enciende el análisis en vivo y lo
    /// enseña en su pestaña, con el cursor en él; otra vez, lo apaga.
    /// Apagado (o con la pestaña sin verse) el motor no copia nada para él
    /// ni se calcula nada.
    pub fn toggle_live_clashes(&mut self) {
        if self.clash.live.take().is_some() {
            self.clash.cursor = false;
            self.clash.pending = 0;
            self.clash.fresh = false;
            if self.dock_tab == Tab::Clash && self.last_masking.is_none() {
                self.show_tab(Tab::None);
            }
            self.message = "solapes de frecuencia en vivo: apagado".into();
            return;
        }
        self.clash.live = Some(Live::new(self.sample_rate, self.viz_hz));
        self.clash.cursor = true;
        self.clash.selected = 0;
        self.show_tab(Tab::Clash);
        self.message = format!(
            "solapes de frecuencia en vivo (los últimos {} s de lo que suena): ↑↓ elige un choque · Esc suelta el panel",
            crate::clash::LIVE_SECONDS
        );
    }

    /// Si el análisis en vivo está pedido y su panel se vio en el último
    /// cuadro (nunca en la ventana del editor, que no tiene motor).
    pub(super) fn clash_shown(&self) -> bool {
        self.clash.live.is_some() && !self.editor_window() && self.clash.layout.is_some()
    }

    /// Las ranuras del motor de las pistas de la canción (en su orden).
    pub(super) fn clash_slots(&self) -> Vec<(String, usize)> {
        let Some(song) = self.host.song() else {
            return Vec::new();
        };
        song.track_names()
            .into_iter()
            .filter_map(|n| Some((n.to_string(), self.host.track_slot(n)?)))
            .collect()
    }

    /// Tras vaciar las colas en un `tick`: una pista de las del análisis
    /// que no ha mandado nada (no suena: el motor no copia el silencio)
    /// recibe ceros, para que su última ventana no se quede para siempre;
    /// y cuenta las muestras para el ritmo del análisis.
    pub(super) fn tick_clash(&mut self, arrived: &[usize; engine::VIZ_SOURCES]) {
        if !self.clash_shown() {
            // Sin verse, nada: lo guardado se olvida (al volver no sale lo
            // de antes).
            if let Some(live) = &mut self.clash.live {
                live.clear();
            }
            self.clash.pending = 0;
            return;
        }
        let slots = self.clash_slots();
        let most = slots.iter().map(|&(_, s)| arrived[s]).max().unwrap_or(0);
        if most == 0 {
            return;
        }
        for &(_, slot) in &slots {
            let history = &mut self.viz_history[slot];
            history.extend(std::iter::repeat_n(0.0, most - arrived[slot]));
            let extra = history.len().saturating_sub(crate::viz::HISTORY);
            history.drain(..extra);
        }
        self.clash.pending += most;
        let every = (self.sample_rate / self.viz_hz.max(1)).max(1) as usize;
        if self.clash.pending >= every {
            self.clash.pending = 0;
            self.clash.fresh = true;
        }
    }

    /// Una ventana más del análisis en vivo, si le toca (una vez por cuadro
    /// dibujado, a `[interfaz] visuales_hz`).
    pub(super) fn frame_clash(&mut self) {
        if !std::mem::take(&mut self.clash.fresh) || !self.clash_shown() {
            return;
        }
        let slots = self.clash_slots();
        let priority = self
            .host
            .song()
            .map_or_else(Vec::new, |s| s.priority.clone());
        let history = &self.viz_history;
        if let Some(live) = &mut self.clash.live {
            live.frame(
                slots
                    .iter()
                    .map(|(name, slot)| (name.as_str(), &history[*slot])),
                &priority,
            );
            if let (Some(report), Some(song)) = (live.report_mut(), self.host.song()) {
                report.set_song(song);
            }
        }
    }

    /// El informe que enseña el panel: el en vivo si está pedido; si no,
    /// el del último análisis offline.
    pub(super) fn clash_report(&self) -> Option<&script::masking::MaskingReport> {
        match &self.clash.live {
            Some(live) => live.report(),
            None => self.last_masking.as_ref().map(|(r, _)| r),
        }
    }

    /// La matriz del panel (del informe que enseña), con el elegido si el
    /// cursor está en él.
    pub(super) fn clash_matrix(&self) -> Option<MatrixView> {
        if self.clash.live.is_none() && self.last_masking.is_none() {
            return None;
        }
        let Some(report) = self.clash_report() else {
            return Some(MatrixView::default());
        };
        let selected = self.clash.cursor.then_some(self.clash.selected);
        let fixes = selected
            .map(|_| self.clash_fixes(report))
            .unwrap_or_default()
            .iter()
            .enumerate()
            .map(|(i, f)| {
                let lead = if i == self.clash.fix { "▸ " } else { "  " };
                format!("{lead}{}", f.line)
            })
            .collect();
        let mut view = MatrixView::of(report, selected, fixes);
        view.chosen_fix = self.clash.fix;
        Some(view)
    }

    /// Lo que dibuja la pestaña «choques»: la tabla pareja × banda y la
    /// matriz del informe que enseña (`None`: ni en vivo ni analizado).
    pub(super) fn clash_table_view(&self) -> Option<ClashTableView> {
        let live = self.clash.live.is_some();
        let stale = !live
            && self
                .last_masking
                .as_ref()
                .is_some_and(|(_, loads)| *loads != self.loads);
        let matrix = self.clash_matrix()?;
        let Some(report) = self.clash_report() else {
            return Some(ClashTableView {
                live,
                matrix: Some(matrix),
                ..Default::default()
            });
        };
        let (bands, rows) = report.grid();
        let name = |t: usize| report.tracks[t].clone();
        Some(ClashTableView {
            bars: report.bars,
            tracks: report.tracks.len(),
            stale,
            bands: bands
                .iter()
                .map(|&b| engine::masking::BAND_NAMES[b].to_string())
                .collect(),
            rows: rows
                .into_iter()
                .map(|((a, b), cells)| ClashRow {
                    a: name(a),
                    b: name(b),
                    cells,
                })
                .collect(),
            clashes: report
                .clashes()
                .iter()
                .map(|c| {
                    (
                        name(c.a),
                        name(c.b),
                        engine::masking::BAND_NAMES[c.band].to_string(),
                        c.clash.score,
                    )
                })
                .collect(),
            live,
            seconds: self
                .clash
                .live
                .as_ref()
                .map_or(0.0, |l| l.seconds(self.viz_hz)),
            matrix: Some(matrix),
        })
    }

    /// Las sugerencias del choque elegido de `report` (ver
    /// [`script::masking::MaskingReport::fixes_for`]).
    pub(super) fn clash_fixes(
        &self,
        report: &script::masking::MaskingReport,
    ) -> Vec<script::masking::Fix> {
        report
            .groups()
            .get(self.clash.selected)
            .map_or_else(Vec::new, |g| report.fixes_for(g))
    }

    /// Inserta en el script la sugerencia elegida del choque elegido, tras
    /// la definición de la pista que cede, en un paso de deshacer; ni
    /// guarda ni carga.
    pub(super) fn insert_clash_fix(&mut self) {
        let Some(report) = self.clash_report().cloned() else {
            return;
        };
        let fixes = self.clash_fixes(&report);
        let Some(fix) = fixes.get(self.clash.fix.min(fixes.len().saturating_sub(1))) else {
            return;
        };
        let name = report.tracks[fix.track].clone();
        let full = self
            .host
            .song()
            .and_then(|s| s.tracks.iter().find(|t| t.name == name))
            .is_some_and(|t| t.fx.len() >= engine::MAX_FX);
        let row = self.insert_after_track(&name, &fix.line);
        let what = match fix.kind {
            script::masking::FixKind::Sidechain => "sidechain",
            script::masking::FixKind::Eq => "EQ",
        };
        let warning = if full {
            format!(" · ojo: «{name}» ya tiene {} efectos", engine::MAX_FX)
        } else {
            String::new()
        };
        self.message = format!(
            "{what} de «{name}» insertado en la línea {}, sin guardar (Ctrl-Z lo deshace; Ctrl-S lo guarda y lo carga){warning}",
            row + 1
        );
    }

    /// Cuántos choques hay en lo que enseña el panel.
    fn clash_groups(&self) -> usize {
        self.clash_report().map_or(0, |r| r.groups().len())
    }

    /// Tecla con el cursor en el panel de choques: ↑↓ eligen un choque, ←→
    /// una de sus sugerencias, Enter la inserta y Esc suelta.
    pub(super) fn handle_clash_cursor(&mut self, input: Input) -> Flow {
        let count = self.clash_groups();
        let fixes = self.clash_report().map_or(0, |r| self.clash_fixes(r).len());
        match input.key {
            Key::Left | Key::Right if fixes > 0 => {
                self.clash.fix = (self.clash.fix.min(fixes - 1) + 1) % fixes;
            }
            Key::Enter => self.insert_clash_fix(),
            Key::Esc => {
                self.clash.cursor = false;
                self.message = "fuera del panel de choques".into();
            }
            Key::Up if count > 0 => {
                self.clash.fix = 0;
                self.clash.selected = (self.clash.selected.min(count - 1) + count - 1) % count;
                self.say_clash();
            }
            Key::Down if count > 0 => {
                self.clash.fix = 0;
                self.clash.selected = (self.clash.selected.min(count - 1) + 1) % count;
                self.say_clash();
            }
            _ => {}
        }
        Flow::Continue
    }

    /// El choque elegido, en la línea de estado.
    fn say_clash(&mut self) {
        let Some(report) = self.clash_report() else {
            return;
        };
        let groups = report.groups();
        if let Some(g) = groups.get(self.clash.selected) {
            self.message = report.group_text(g);
        }
    }

    /// El ratón en el panel de choques: un clic en un choque de la lista lo
    /// elige (y pone el cursor en el panel); en una celda, el choque más
    /// fuerte de esa pista y banda.
    pub(super) fn clash_mouse(&mut self, action: MouseAction, hit: MatrixHit) {
        if action != MouseAction::Down {
            return;
        }
        let Some(report) = self.clash_report() else {
            return;
        };
        let groups = report.groups();
        let chosen = match hit {
            MatrixHit::Group(i) => (i < groups.len()).then_some(i),
            MatrixHit::Cell(t, band) => groups
                .iter()
                .position(|g| (g.a == t || g.b == t) && (g.bands.0..=g.bands.1).contains(&band)),
            MatrixHit::Fix(i) => {
                // Un clic en una sugerencia la inserta.
                self.clash.fix = i;
                self.insert_clash_fix();
                return;
            }
        };
        if let Some(i) = chosen {
            self.clash.cursor = true;
            if i != self.clash.selected {
                self.clash.fix = 0;
            }
            self.clash.selected = i;
            self.say_clash();
        }
    }
}
