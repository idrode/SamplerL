//! Modelo de vista: todo lo que dibujan los paneles. Se construye a partir
//! del modelo del script ([`script::Song`]), la instantánea del motor
//! ([`engine::Status`]) y el estado del editor. Los paneles solo leen esto.

use std::ops::Range;
use std::sync::Arc;

use engine::{Fx, STEPS, Sample, Step};

#[derive(Debug, Clone, Default)]
pub struct ViewModel {
    /// Nombre del archivo del script.
    pub file: String,
    /// Hay cambios en el editor sin guardar.
    pub dirty: bool,
    /// Modo tocar: las teclas disparan cortes en vez de escribir.
    pub playing_mode: bool,
    /// El archivo cambió fuera del editor (se ofrece recargar).
    pub external_change: bool,
    /// Línea de órdenes abierta (F2) y su texto.
    pub prompt: Option<String>,
    /// Autocompletado abierto.
    pub completion: Option<CompletionView>,
    /// Nombre del proyecto abierto, si lo hay.
    pub project: Option<String>,
    /// Mensaje informativo de la última acción.
    pub message: String,
    /// Error de la última carga (sigue sonando lo anterior).
    pub error: Option<ErrorView>,
    pub transport: TransportView,
    /// Pistas en el orden del script.
    pub tracks: Vec<TrackView>,
    /// Pista seleccionada (índice en `tracks`).
    pub selected: Option<usize>,
}

/// Popup de autocompletado.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct CompletionView {
    /// Opciones: nombre y qué es.
    pub items: Vec<(String, String)>,
    pub selected: usize,
    /// Firma y descripción de la opción elegida.
    pub signature: String,
    pub description: String,
    /// Caracteres ya escritos antes del cursor (el popup se alinea con
    /// ellos).
    pub prefix_len: usize,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ErrorView {
    /// Línea del script (desde 1), si el error la indica.
    pub line: Option<usize>,
    pub message: String,
}

#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct TransportView {
    pub running: bool,
    pub bpm: f64,
    pub bar: u64,
    /// Último paso disparado.
    pub step: Option<usize>,
}

#[derive(Debug, Clone, Default)]
pub struct TrackView {
    pub name: String,
    /// Pista del motor asignada.
    pub slot: Option<usize>,
    pub sample_name: String,
    pub sample_frames: usize,
    pub sample_rate: u32,
    /// Resumen de la forma de onda del sample.
    pub waveform: Arc<Waveform>,
    pub slices: Vec<Range<usize>>,
    /// Compases del ciclo (`every`/`chance`).
    pub cycle: usize,
    /// Pasos del compás que está sonando (variante del ciclo).
    pub steps: [Option<Step>; STEPS],
    /// Corte que sonó por última vez (índice desde 0).
    pub playing_slice: Option<usize>,
    /// Medidor de nivel (pico con caída), de 0 a 1.
    pub level: f32,
    /// Volumen de la pista (lineal).
    pub gain: f32,
    /// Efectos de la pista, en orden.
    pub fx: Vec<Fx>,
}

/// Resumen de una forma de onda: mínimo y máximo (canales mezclados) de
/// cada tramo de igual longitud. Se calcula una vez por sample; los paneles
/// lo reescalan a su ancho sin tocar las muestras.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Waveform {
    pub buckets: Vec<(f32, f32)>,
}

/// Tramos del resumen de forma de onda.
pub const WAVEFORM_BUCKETS: usize = 2048;

impl Waveform {
    pub fn of(sample: &Sample) -> Self {
        let frames = sample.frames();
        let count = WAVEFORM_BUCKETS.min(frames.max(1));
        let buckets = (0..count)
            .map(|b| {
                let range =
                    b * frames / count..((b + 1) * frames / count).max(b * frames / count + 1);
                range
                    .filter(|&i| i < frames)
                    .map(|i| {
                        let (l, r) = sample.frame(i);
                        0.5 * (l + r)
                    })
                    .fold((f32::MAX, f32::MIN), |(lo, hi), x| (lo.min(x), hi.max(x)))
            })
            .map(|(lo, hi)| if lo > hi { (0.0, 0.0) } else { (lo, hi) })
            .collect();
        Waveform { buckets }
    }

    /// Mínimo y máximo de la fracción `[from, to)` del sample (0 a 1).
    pub fn range(&self, from: f64, to: f64) -> (f32, f32) {
        let n = self.buckets.len();
        if n == 0 {
            return (0.0, 0.0);
        }
        let a = ((from * n as f64) as usize).min(n - 1);
        let b = ((to * n as f64).ceil() as usize).clamp(a + 1, n);
        self.buckets[a..b]
            .iter()
            .fold((f32::MAX, f32::MIN), |(lo, hi), &(l, h)| {
                (lo.min(l), hi.max(h))
            })
    }
}

/// Línea (desde 1) de un mensaje de error de Lua (`archivo:línea: ...`).
pub fn error_line(message: &str, file: &str) -> Option<usize> {
    let rest = message.strip_prefix(file)?.strip_prefix(':')?;
    let digits: String = rest.chars().take_while(char::is_ascii_digit).collect();
    digits.parse().ok()
}

/// Texto de un paso en mini-notación (`03'^7@0.5r4`), o `---` si es silencio.
pub fn step_text(step: &Option<Step>) -> String {
    let Some(step) = step else {
        return "---".into();
    };
    let mut text = format!("{:02}", step.slice + 1);
    if step.reverse {
        text.push('\'');
    }
    if step.semitones != 0.0 {
        text.push_str(&format!("^{}", step.semitones));
    }
    if step.gain != 1.0 {
        text.push_str(&format!("@{}", step.gain));
    }
    if step.retrig > 1 {
        text.push_str(&format!("r{}", step.retrig));
    }
    text
}

/// Descripción corta de un efecto.
pub fn fx_text(fx: &Fx) -> String {
    match fx {
        Fx::Filter { kind, cutoff_hz, q } => {
            let kind = match kind {
                engine::FilterKind::LowPass => "LP",
                engine::FilterKind::HighPass => "HP",
            };
            format!("{kind} {cutoff_hz:.0} Hz q{q:.2}")
        }
        Fx::Bitcrush { bits, downsample } => format!("crush {bits}b ÷{downsample}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_the_error_line() {
        assert_eq!(error_line("song.lua:12: steps: ...", "song.lua"), Some(12));
        assert_eq!(error_line("otro.lua:3: x", "song.lua"), None);
        assert_eq!(error_line("song.lua: no existe", "song.lua"), None);
    }

    #[test]
    fn waveform_summarizes_min_and_max() {
        let mut mono = vec![0.0f32; 10_000];
        mono[2000..3000].fill(0.5);
        mono[7000..7100].fill(-0.8);
        let sample = Sample::from_interleaved(&mono, 1, 48_000, 48_000).unwrap();
        let w = Waveform::of(&sample);
        assert_eq!(w.buckets.len(), WAVEFORM_BUCKETS);
        assert_eq!(w.range(0.0, 0.1), (0.0, 0.0));
        // Los tramos no caen justo en los bordes: un rango interior.
        assert_eq!(w.range(0.21, 0.29), (0.5, 0.5));
        assert_eq!(w.range(0.6, 0.8), (-0.8, 0.0));
        assert_eq!(w.range(0.0, 1.0), (-0.8, 0.5));
    }

    #[test]
    fn writes_steps_in_notation() {
        assert_eq!(step_text(&None), "---");
        assert_eq!(step_text(&Some(Step::new(2))), "03");
        let fancy = Step::new(0)
            .reversed()
            .with_pitch(-5.0)
            .with_gain(0.5)
            .with_retrig(4);
        assert_eq!(step_text(&Some(fancy)), "01'^-5@0.5r4");
        // Se puede volver a leer con la notación (con el número sin cero a la izquierda).
        let parsed = engine::notation::parse_step("1'^-5@0.5r4").unwrap();
        assert_eq!(parsed, Some(fancy));
    }

    #[test]
    fn describes_effects() {
        assert_eq!(fx_text(&Fx::lowpass(3000.0)), "LP 3000 Hz q0.71");
        assert_eq!(fx_text(&Fx::highpass(120.5)), "HP 120 Hz q0.71");
        assert_eq!(fx_text(&Fx::bitcrush(6, 2)), "crush 6b ÷2");
    }
}
