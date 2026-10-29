//! «Tonalidad del sample»: el análisis (`engine::detect_key`) en un hilo
//! aparte, el panel que lo enseña (desde la paleta, del sample de la pista
//! seleccionada, y desde el explorador con Ctrl-K, del elegido) y sus
//! acciones, que solo tocan el texto cuando se piden: la raíz del
//! instrumento (`r`), insertar un instrumento con esa raíz (`i`) y
//! proponer la tonalidad de la canción (`k`). Ninguna carga el script: se
//! ve en el editor y Ctrl-S lo aplica (y Ctrl-Z lo deshace de una vez).

use std::sync::Arc;
use std::sync::mpsc::{self, Receiver};

use engine::{KeyAnalysis, Sample};
use ratatui_textarea::{Input, Key};

use super::{App, Flow};
use crate::key_panel::{self, KeyState, KeyView};

/// El panel abierto.
pub(super) struct KeyPanel {
    /// El sample, como se escribe en `sample("...")`.
    sample: String,
    /// El análisis cuando acaba (`None` dentro: nada que analizar).
    result: Option<Option<KeyAnalysis>>,
    /// Por donde llega del hilo.
    pending: Option<Receiver<Option<KeyAnalysis>>>,
    selected: usize,
}

impl App {
    /// «Tonalidad del sample»: la del sample de la pista seleccionada.
    pub fn open_sample_key(&mut self) {
        let Some(track) = self.tracks().get(self.selected) else {
            self.message = "no hay pistas".into();
            return;
        };
        if track.instrument.is_some_and(|i| i.synth.is_some()) {
            self.message = format!("{}: un sintetizador no tiene sample", track.name);
            return;
        }
        let (sample, name) = (track.sample.clone(), track.origin.sample.clone());
        self.analyze_key(name, sample);
    }

    /// Abre el panel y analiza `sample` en un hilo aparte.
    pub(super) fn analyze_key(&mut self, name: String, sample: Arc<Sample>) {
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            let _ = tx.send(engine::detect_key(&sample));
        });
        self.completion = None;
        self.key_panel = Some(KeyPanel {
            sample: name,
            result: None,
            pending: Some(rx),
            selected: 0,
        });
        self.message = "analizando la tonalidad…".into();
    }

    /// Recoge el análisis si ya acabó (en cada `tick`).
    pub(super) fn poll_key_analysis(&mut self) {
        let Some(panel) = &mut self.key_panel else {
            return;
        };
        let Some(rx) = &panel.pending else {
            return;
        };
        let result = match rx.try_recv() {
            Ok(result) => result,
            Err(mpsc::TryRecvError::Empty) => return,
            Err(mpsc::TryRecvError::Disconnected) => None,
        };
        panel.pending = None;
        self.message = match &result {
            Some(a) if a.low_confidence() => format!(
                "{}: {} (confianza baja, {:.2})",
                panel.sample,
                a.best().label(),
                a.confidence
            ),
            Some(a) => format!(
                "{}: {} · raíz {} · r raíz · i instrumento · k key()",
                panel.sample,
                a.best().label(),
                engine::note_name(a.root)
            ),
            None => format!("{}: nada que analizar", panel.sample),
        };
        panel.result = Some(result);
    }

    /// Espera al análisis (para los tests: sin hilo de por medio no se
    /// sabe cuándo acaba).
    #[cfg(test)]
    pub fn wait_key_analysis(&mut self) {
        let start = std::time::Instant::now();
        while self.key_panel.as_ref().is_some_and(|p| p.pending.is_some()) {
            assert!(start.elapsed().as_secs() < 30, "el análisis no acaba");
            std::thread::sleep(std::time::Duration::from_millis(5));
            self.poll_key_analysis();
        }
    }

    /// La tonalidad de lo cargado, como se escribe (para los tests).
    #[cfg(test)]
    pub fn host_song_key_for_test(&self) -> Option<String> {
        self.host.song().and_then(|s| s.key).map(|k| k.call())
    }

    /// Una tecla con el panel abierto.
    pub(super) fn handle_key_panel(&mut self, input: Input) -> Flow {
        let Some(panel) = &mut self.key_panel else {
            return Flow::Continue;
        };
        match input.key {
            Key::Esc | Key::Char('q') => {
                self.key_panel = None;
                self.message.clear();
            }
            Key::Up => panel.selected = panel.selected.saturating_sub(1),
            Key::Down => panel.selected = (panel.selected + 1).min(2),
            Key::Char(c @ ('r' | 'i' | 'k')) if !input.ctrl && !input.alt => {
                let Some(Some(analysis)) = panel.result.clone() else {
                    self.message = "todavía no hay tonalidad".into();
                    return Flow::Continue;
                };
                let (sample, selected) = (panel.sample.clone(), panel.selected);
                match c {
                    'r' => self.use_key_root(&sample, &analysis, selected),
                    'i' => self.insert_key_instrument(&sample, &analysis, selected),
                    _ => self.propose_song_key(&analysis, selected),
                }
            }
            _ => {}
        }
        Flow::Continue
    }

    /// La raíz que se propone con la candidata `selected`: la tónica en la
    /// octava donde más suena (la de la mejor, movida a la tónica elegida
    /// por el camino más corto).
    fn key_root(analysis: &KeyAnalysis, selected: usize) -> String {
        let tonic = analysis.candidates[selected.min(2)].tonic as i32;
        let mut shift = (tonic - analysis.root).rem_euclid(12);
        if shift > 6 {
            shift -= 12;
        }
        engine::note_name(analysis.root + shift)
    }

    /// `r`: la raíz en el `instrument()` de ese sample en el script.
    fn use_key_root(&mut self, sample: &str, analysis: &KeyAnalysis, selected: usize) {
        let root = Self::key_root(analysis, selected);
        let text = self.editor.lines().join("\n");
        let Some(new) = key_panel::with_instrument_root(&text, sample, &root) else {
            self.message = format!(
                "no encuentro el instrument() de {sample} en el script: i inserta uno con la raíz {root}"
            );
            return;
        };
        self.replace_text(&new);
        self.key_panel = None;
        self.message = format!(
            "root = \"{root}\" en el instrument() de {sample}: Ctrl-S para cargar (Ctrl-Z lo deshace)"
        );
    }

    /// `i`: inserta `sample("x"):instrument({ root = "A3" })` en el cursor.
    fn insert_key_instrument(&mut self, sample: &str, analysis: &KeyAnalysis, selected: usize) {
        let root = Self::key_root(analysis, selected);
        let text = format!("sample(\"{sample}\"):instrument({{ root = \"{root}\" }})");
        self.marks.clear();
        self.grouped(|editor| editor.insert_str(&text));
        self.key_panel = None;
        self.message = format!("{text} en el script");
    }

    /// `k`: pone `key(...)` con la candidata elegida (sustituye la que
    /// haya, o va detrás de `bpm()`). No carga: se aplica con Ctrl-S.
    fn propose_song_key(&mut self, analysis: &KeyAnalysis, selected: usize) {
        let guess = analysis.candidates[selected.min(2)];
        let call = format!("key(\"{}\", \"{}\")", guess.tonic_name(), guess.mode());
        let text = self.editor.lines().join("\n");
        self.replace_text(&key_panel::with_key_line(&text, &call));
        self.key_panel = None;
        self.message = format!("{call} en el script: Ctrl-S para aplicarla (Ctrl-Z lo deshace)");
    }

    /// Lo que enseña el panel.
    pub(super) fn key_view(&self) -> Option<KeyView> {
        let panel = self.key_panel.as_ref()?;
        let state = match &panel.result {
            None => KeyState::Analyzing,
            Some(None) => KeyState::Nothing("nada que analizar (silencio o muy corto)".into()),
            Some(Some(a)) => KeyState::Done {
                candidates: a.candidates.iter().map(|g| (g.label(), g.score)).collect(),
                confidence: a.confidence,
                low: a.low_confidence(),
                root: Self::key_root(a, panel.selected),
            },
        };
        let song_key = self
            .host
            .song()
            .and_then(|s| s.key)
            .map(|k| format!("{} {}", k.tonic_name(), k.scale.name));
        Some(KeyView {
            sample: panel.sample.clone(),
            state,
            selected: panel.selected,
            song_key,
        })
    }
}
