// Hasta que lo pida el script (la API, más adelante), solo lo usan los tests.
#![allow(dead_code)]

//! Visualizaciones al estilo de Strudel, por pista o del master: las de
//! rejilla (`punchcard`, `pianoroll`) salen del modelo del piano roll (lo
//! que sonará) con el cursor del resaltado en vivo; las de audio
//! (`scope`, `spectrum`...), de las muestras que copia el motor
//! (`engine::viz`). Todo lo que se calcula (ventanas, FFT, disparo) va en
//! el hilo de la interfaz.

pub mod grid;
pub mod scope;

/// Qué se enseña.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VizKind {
    Punchcard,
    Pianoroll,
    Scope,
    Spectrum,
    Pitchwheel,
    Spiral,
}

impl VizKind {
    pub fn name(self) -> &'static str {
        match self {
            VizKind::Punchcard => "punchcard",
            VizKind::Pianoroll => "pianoroll",
            VizKind::Scope => "scope",
            VizKind::Spectrum => "spectrum",
            VizKind::Pitchwheel => "pitchwheel",
            VizKind::Spiral => "spiral",
        }
    }

    /// Necesita las muestras del motor.
    pub fn audio(self) -> bool {
        !matches!(self, VizKind::Punchcard | VizKind::Pianoroll)
    }
}

/// De dónde sale.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VizSource {
    Track(String),
    Master,
}

/// Una visualización pedida: qué, de dónde, cuántas filas y (en
/// `spectrum`) cuántas bandas.
#[derive(Debug, Clone, PartialEq)]
pub struct VizSpec {
    pub kind: VizKind,
    pub source: VizSource,
    pub size: u16,
    pub bands: usize,
}

impl VizSpec {
    pub fn title(&self) -> String {
        let source = match &self.source {
            VizSource::Track(name) => name.as_str(),
            VizSource::Master => "master",
        };
        format!(" {} · {source} ", self.kind.name())
    }
}

/// Lo que se dibuja de una visualización.
#[derive(Debug, Clone, PartialEq)]
pub enum VizView {
    Grid(grid::GridView),
    Scope(scope::ScopeView),
}

impl VizView {
    /// Filas que ocupa (con los bordes).
    pub fn height(&self) -> u16 {
        match self {
            VizView::Grid(g) => g.size + 2,
            VizView::Scope(s) => s.size + 2,
        }
    }
}

/// Las visualizaciones, apiladas: cada una en su panel.
pub struct VizPanels<'a>(pub &'a [VizView]);

impl ratatui::widgets::Widget for VizPanels<'_> {
    fn render(self, area: ratatui::layout::Rect, buf: &mut ratatui::buffer::Buffer) {
        let mut y = area.y;
        for view in self.0 {
            if y >= area.bottom() {
                break;
            }
            let height = view.height().min(area.bottom() - y);
            let rect = ratatui::layout::Rect { y, height, ..area };
            match view {
                VizView::Grid(g) => grid::GridPanel(g).render(rect, buf),
                VizView::Scope(s) => scope::ScopePanel(s).render(rect, buf),
            }
            y += height;
        }
    }
}

/// Lo que se guarda de cada fuente de audio (lo último que ha llegado).
pub const HISTORY: usize = 16_384;

/// Lo que piden todas, apiladas.
pub fn total_height(views: &[VizView]) -> u16 {
    views.iter().map(VizView::height).sum()
}
