//! Visualizaciones al estilo de Strudel, por pista o del master: las de
//! rejilla (`punchcard`, `pianoroll`) salen del modelo del piano roll (lo
//! que sonará) con el cursor del resaltado en vivo; las de audio
//! (`scope`, `spectrum`...), de las muestras que copia el motor
//! (`engine::viz`). Todo lo que se calcula (ventanas, FFT, disparo) va en
//! el hilo de la interfaz.

use ratatui::widgets::Widget;

pub mod grid;
pub mod scope;
pub mod spectrum;
pub mod wheel;

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

    /// Por su nombre en Lua.
    pub fn from_name(name: &str) -> Option<VizKind> {
        [
            VizKind::Punchcard,
            VizKind::Pianoroll,
            VizKind::Scope,
            VizKind::Spectrum,
            VizKind::Pitchwheel,
            VizKind::Spiral,
        ]
        .into_iter()
        .find(|k| k.name() == name)
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
    /// Junto al código, en esta fila del texto (`inline = true`).
    pub inline: Option<usize>,
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
    Spectrum(spectrum::SpectrumView),
    Wheel(wheel::WheelView),
    Spiral(wheel::SpiralView),
    /// Ya dibujada, en texto (la que manda la principal a la ventana del
    /// editor): sus filas, con los bordes.
    Text(Vec<String>),
}

/// Una visualización y dónde va: junto al código, en una fila del texto, o
/// (`None`) en su panel.
#[derive(Debug, Clone, PartialEq)]
pub struct Placed {
    pub line: Option<usize>,
    pub view: VizView,
}

/// Columnas de una visualización junto al código.
pub const INLINE_WIDTH: u16 = 40;
/// Columnas con las que se dibujan las de su panel para la ventana del
/// editor.
pub const REMOTE_WIDTH: u16 = 60;

/// `view` dibujada en `width` columnas, como texto (sin colores).
pub fn to_text(view: &VizView, width: u16) -> Vec<String> {
    let height = view.height();
    let area = ratatui::layout::Rect::new(0, 0, width, height);
    let mut buf = ratatui::buffer::Buffer::empty(area);
    VizPanels(std::slice::from_ref(view)).render(area, &mut buf);
    (0..height)
        .map(|y| (0..width).map(|x| buf[(x, y)].symbol()).collect::<String>())
        .collect()
}

impl VizView {
    /// Filas que ocupa (con los bordes).
    pub fn height(&self) -> u16 {
        match self {
            VizView::Grid(g) => g.size + 2,
            VizView::Scope(s) => s.size + 2,
            VizView::Spectrum(s) => s.size + 2,
            VizView::Wheel(w) => w.size + 2,
            VizView::Spiral(s) => s.size + 2,
            VizView::Text(rows) => rows.len() as u16,
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
                VizView::Spectrum(s) => spectrum::SpectrumPanel(s).render(rect, buf),
                VizView::Wheel(w) => wheel::WheelPanel(w).render(rect, buf),
                VizView::Spiral(s) => wheel::SpiralPanel(s).render(rect, buf),
                VizView::Text(rows) => {
                    for (i, row) in rows.iter().enumerate().take(rect.height as usize) {
                        buf.set_stringn(
                            rect.x,
                            rect.y + i as u16,
                            row,
                            rect.width as usize,
                            ratatui::style::Style::new().fg(crate::ui::accent()),
                        );
                    }
                }
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
