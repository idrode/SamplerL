//! El linter en el editor (el análisis está en `script::lint`): se lanza
//! tras una pausa al escribir (`[editor] linter_ms`), solo si el texto
//! cambió desde el último análisis, en un hilo aparte, y nunca toca lo que
//! suena. Sin cambios y en reposo no cuesta nada: la huella del texto se
//! toma al construir la vista de un cuadro que se dibuja (como la caché de
//! los avisos de la notación), y `tick` solo compara dos números.
//!
//! Si el análisis entra en pánico, se atrapa (`script::panics::catch`, que
//! además calla el aviso en la terminal) y no se marca nada. Si tarda más
//! que el límite de tiempo del script, se abandona sin esperarlo (y ese
//! texto no se vuelve a mirar); tras [`MAX_ABANDONED`] abandonados, el
//! linter se apaga hasta reiniciar.

use std::cell::Cell;
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::time::{Duration, Instant};

use ratatui_textarea::CursorMove;
use script::lint::{Lint, Severity};

use super::App;

/// Análisis abandonados (colgados) tras los que el linter se apaga.
const MAX_ABANDONED: usize = 4;
/// Lo que se espera de más sobre el límite de tiempo del script.
const GRACE: Duration = Duration::from_millis(500);

/// Lo que se sabe del linter.
#[derive(Default)]
pub(crate) struct LintState {
    /// Lo que dijo el último análisis.
    pub(crate) results: Vec<Lint>,
    /// La huella del texto del último cuadro dibujado y desde cuándo es esa.
    seen: Cell<Option<(u64, Instant)>>,
    /// La huella del texto de `results` (o del que se abandonó).
    analyzed: Option<u64>,
    /// El análisis en marcha: su huella, cuándo empezó y por dónde llega.
    running: Option<(u64, Instant, Receiver<Vec<Lint>>)>,
    /// Cambia con cada resultado (el dibujo en reposo lo mira).
    pub(crate) generation: u64,
    /// Análisis lanzados (los tests miran el debounce).
    pub(crate) launched: usize,
    abandoned: usize,
}

impl LintState {
    /// El texto que se ve ahora tiene esta huella (al construir la vista).
    pub(crate) fn note_text(&self, hash: u64, now: Instant) {
        if self.seen.get().is_none_or(|(h, _)| h != hash) {
            self.seen.set(Some((hash, now)));
        }
    }
}

/// Un problema del texto: del linter o de la notación.
#[derive(Debug, Clone, PartialEq)]
pub struct Problem {
    pub row: usize,
    pub cols: std::ops::Range<usize>,
    pub severity: Severity,
    pub message: String,
}

impl App {
    /// La huella de las líneas del editor.
    pub(crate) fn text_hash(&self) -> u64 {
        use std::hash::{Hash, Hasher};
        let mut h = std::collections::hash_map::DefaultHasher::new();
        self.editor.lines().hash(&mut h);
        h.finish()
    }

    /// Recoge el resultado o lanza el análisis si toca (en cada vuelta).
    pub(crate) fn poll_lint(&mut self) {
        self.poll_lint_at(Instant::now());
    }

    pub(crate) fn poll_lint_at(&mut self, now: Instant) {
        if let Some((hash, started, rx)) = &self.lint.running {
            match rx.try_recv() {
                Ok(results) => {
                    self.lint.results = results;
                    self.lint.analyzed = Some(*hash);
                    self.lint.running = None;
                    self.lint.generation += 1;
                }
                Err(TryRecvError::Empty) => {
                    let limit = self.script_config.time_limit + GRACE;
                    if now.saturating_duration_since(*started) > limit {
                        self.lint.analyzed = Some(*hash);
                        self.lint.running = None;
                        self.lint.abandoned += 1;
                        self.lint.results.clear();
                        self.lint.generation += 1;
                    }
                }
                Err(TryRecvError::Disconnected) => {
                    self.lint.analyzed = Some(*hash);
                    self.lint.running = None;
                    self.lint.results.clear();
                    self.lint.generation += 1;
                }
            }
            return;
        }
        if !self.editor_config.linter || self.lint.abandoned >= MAX_ABANDONED {
            if !self.lint.results.is_empty() {
                self.lint.results.clear();
                self.lint.generation += 1;
            }
            return;
        }
        let Some((hash, since)) = self.lint.seen.get() else {
            return;
        };
        let wait = Duration::from_millis(self.editor_config.linter_ms as u64);
        if self.lint.analyzed == Some(hash) || now.saturating_duration_since(since) < wait {
            return;
        }
        // El texto del cuadro puede haber cambiado sin dibujarse aún: la
        // pausa empieza otra vez.
        let current = self.text_hash();
        if current != hash {
            self.lint.seen.set(Some((current, now)));
            return;
        }
        let text = self.editor.lines().join("\n");
        let memory = self.script_config.memory_mb << 20;
        let (tx, rx) = mpsc::channel();
        let spawned = std::thread::Builder::new()
            .name("linter".into())
            .stack_size(16 << 20)
            .spawn(move || {
                let results = script::panics::catch(|| script::lint::check(&text, memory))
                    .unwrap_or_default();
                let _ = tx.send(results);
            });
        match spawned {
            Ok(_) => {
                self.lint.running = Some((hash, now, rx));
                self.lint.launched += 1;
            }
            // Sin hilo, sin análisis (y no se reintenta con este texto).
            Err(_) => self.lint.analyzed = Some(hash),
        }
    }

    /// Para los tests: espera al análisis en marcha.
    #[cfg(test)]
    pub(crate) fn wait_lint(&mut self) {
        let start = Instant::now();
        while self.lint.running.is_some() && start.elapsed() < Duration::from_secs(10) {
            std::thread::sleep(Duration::from_millis(5));
            self.poll_lint_at(Instant::now());
        }
    }

    /// Los problemas del texto: los del linter y los avisos de la notación,
    /// por posición.
    pub(crate) fn problems(&self, diagnostics: &[crate::diagnose::Diagnostic]) -> Vec<Problem> {
        let mut all: Vec<Problem> = self
            .lint
            .results
            .iter()
            .map(|l| Problem {
                row: l.row,
                cols: l.cols.clone(),
                severity: l.severity,
                message: l.message.clone(),
            })
            .chain(diagnostics.iter().map(|d| Problem {
                row: d.row,
                cols: d.cols.clone(),
                severity: Severity::Warning,
                message: d.message.clone(),
            }))
            .collect();
        all.sort_by_key(|p| (p.row, p.cols.start, p.severity));
        all
    }

    /// Lleva el cursor al problema siguiente (o al anterior), dando la
    /// vuelta; con `errors_first`, al siguiente error si hay alguno.
    pub(crate) fn goto_problem(&mut self, forward: bool, errors_first: bool) {
        let diagnostics = self.diagnostics();
        let mut problems = self.problems(&diagnostics);
        if problems.is_empty() {
            self.message = "sin diagnósticos en el texto".into();
            return;
        }
        if errors_first && problems.iter().any(|p| p.severity == Severity::Error) {
            problems.retain(|p| p.severity == Severity::Error);
        }
        let cursor = self.editor.cursor();
        let here = (cursor.0, cursor.1);
        let at = |p: &Problem| (p.row, p.cols.start);
        let chosen = if forward {
            problems
                .iter()
                .find(|p| at(p) > here)
                .or_else(|| problems.first())
        } else {
            problems
                .iter()
                .rev()
                .find(|p| at(p) < here)
                .or_else(|| problems.last())
        };
        let Some(p) = chosen.cloned() else { return };
        let index = problems.iter().position(|q| *q == p).unwrap_or(0) + 1;
        self.editor.cancel_selection();
        let clamp = |n: usize| n.min(u16::MAX as usize) as u16;
        self.editor
            .move_cursor(CursorMove::Jump(clamp(p.row), clamp(p.cols.start)));
        self.message = format!(
            "{} {index} de {}, línea {}: {}",
            p.severity.label(),
            problems.len(),
            p.row + 1,
            p.message
        );
    }
}
