//! Modelo de vista: todo lo que dibujan los paneles. Se construye a partir
//! del modelo del script ([`script::Song`]), la instantánea del motor
//! ([`engine::Status`]) y el estado del editor. Los paneles solo leen esto.

use std::ops::Range;
use std::sync::Arc;

use engine::{Fx, Sample, Step};

pub use crate::complete::Help;
pub use crate::diagnose::Diagnostic;

#[derive(Debug, Clone, Default)]
pub struct ViewModel {
    /// Nombre del archivo del script.
    pub file: String,
    /// Cursor del tracker al editar los efectos de un paso: (pista, paso).
    pub tracker_cursor: Option<(usize, usize)>,
    /// Hay cambios en el editor sin guardar.
    pub dirty: bool,
    /// Modo tocar: las teclas disparan cortes en vez de escribir.
    pub playing_mode: bool,
    /// Con el modo vim, el suyo (`NORMAL` o `INSERT`).
    pub vim: Option<&'static str>,
    /// El archivo cambió fuera del editor (se ofrece recargar).
    pub external_change: bool,
    /// Línea de órdenes abierta (F2) y su texto.
    pub prompt: Option<String>,
    /// Autocompletado abierto.
    pub completion: Option<CompletionView>,
    /// Paleta de comandos abierta.
    pub palette: Option<PaletteView>,
    /// Selector de bloques abierto.
    pub block_picker: Option<BlockPickerView>,
    /// Selector de versiones del historial abierto (como el de bloques).
    pub versions: Option<BlockPickerView>,
    /// Informe de choques de frecuencia abierto.
    pub masking: Option<BlockPickerView>,
    /// Tabla del último análisis de choques, si se ha hecho (la ventana
    /// principal sin editor la muestra junto al sample).
    pub clash_table: Option<ClashTableView>,
    /// Explorador de samples abierto.
    pub browser: Option<BrowserView>,
    /// Esperando el número de una ranura rápida.
    pub slots: Option<SlotsView>,
    /// Grupos de pistas (`group()`), en su orden.
    pub groups: Vec<GroupView>,
    /// Macros (`macro()`): nombre y valor.
    pub macros: Vec<(String, f32)>,
    /// Panel de las macros abierto, con la elegida.
    pub macro_panel: Option<usize>,
    /// Nombre del proyecto abierto, si lo hay.
    pub project: Option<String>,
    /// Mensaje informativo de la última acción.
    pub message: String,
    /// Atajos principales del modo actual (de la configuración); vacío: los
    /// de siempre.
    pub key_help: String,
    /// Error de la última carga (sigue sonando lo anterior).
    pub error: Option<ErrorView>,
    /// Problemas de la notación de `steps("...")` en el texto actual (en
    /// vivo, sin cargar).
    pub diagnostics: Vec<Diagnostic>,
    /// Tokens de la mini-notación que se oyen ahora, en el orden en que se
    /// dibujan (resaltado en vivo, fase M).
    pub highlights: Vec<crate::highlight::Highlight>,
    /// Ayuda de la función bajo el cursor (en el modo editor).
    pub help: Option<Help>,
    pub transport: TransportView,
    /// El cue (donde empieza el siguiente arranque), como se ve: `compás
    /// 8` o `drop (entrada 3)`.
    pub cue: Option<String>,
    /// Metrónomo encendido.
    pub metronome: bool,
    /// Grabación en curso: «REC drums 1/2» o, hasta que empieza el
    /// compás, «REC drums: al compás».
    pub recording: Option<String>,
    /// Con `variant()`: la variante que suena (`'A'` o `'B'`).
    pub variant: Option<char>,
    /// Con `arrange()`: la línea de tiempo y por dónde va.
    pub timeline: Option<TimelineView>,
    /// Pistas en el orden del script.
    pub tracks: Vec<TrackView>,
    /// Pista seleccionada (índice en `tracks`).
    pub selected: Option<usize>,
    /// Corte seleccionado de esa pista (desde 0).
    pub selected_slice: Option<usize>,
    pub master: MasterView,
    /// Con el editor en su propia ventana: cómo está el enlace (y, en el
    /// editor, por dónde va la principal).
    pub link: Option<String>,
    /// Esta es la ventana principal con el editor en la suya: las vistas
    /// ocupan toda la ventana.
    pub editor_outside: bool,
}

/// Tabla de choques de frecuencia del último análisis.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ClashTableView {
    /// Compases y pistas analizados.
    pub bars: u64,
    pub tracks: usize,
    /// Se ha cargado otra cosa después de analizar.
    pub stale: bool,
    /// Bandas que chocan en alguna pareja (`315`, `1.25k`), de grave a
    /// agudo.
    pub bands: Vec<String>,
    /// Una fila por pareja con algún choque: las dos pistas y una celda por
    /// banda.
    pub rows: Vec<ClashRow>,
    /// Los choques, del más fuerte al más débil: las dos pistas, la banda
    /// y la puntuación.
    pub clashes: Vec<(String, String, String, f32)>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ClashRow {
    pub a: String,
    pub b: String,
    pub cells: Vec<script::masking::Cell>,
}

/// Medidor del master.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct MasterView {
    /// Pico (con caída) de la suma antes del limitador; puede pasar de 1.
    pub level: f32,
    /// El limitador ha actuado hace poco.
    pub limiting: bool,
    /// La suma ha pasado de ±1 hace poco (el limitador la comprime).
    pub clipping: bool,
}

impl MasterView {
    /// Nivel en dBFS (`None` en silencio).
    pub fn db(&self) -> Option<f32> {
        (self.level > 0.0).then(|| 20.0 * self.level.log10())
    }
}

/// Paleta de comandos.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct PaletteView {
    /// Lo escrito para filtrar.
    pub query: String,
    /// Entradas que coinciden: nombre y atajo.
    pub items: Vec<(String, String)>,
    pub selected: usize,
    /// Descripción de la elegida.
    pub description: String,
}

/// Ranuras rápidas 1-9 mientras se espera el número.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct SlotsView {
    /// «guardar en la ranura» o «pegar la ranura».
    pub title: String,
    /// Líneas de cada ranura (`None`: vacía).
    pub items: Vec<Option<Vec<String>>>,
}

/// Explorador de samples.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct BrowserView {
    pub query: String,
    /// Archivos que pasan el filtro (rutas relativas a `folder`).
    pub items: Vec<String>,
    pub selected: usize,
    /// Duración del elegido (al preescucharlo) o el error al leerlo.
    pub info: String,
    pub folder: String,
}

/// Selector de bloques de la biblioteca.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct BlockPickerView {
    /// Qué se hace con el elegido («insertar un bloque»...).
    pub title: String,
    /// Lo escrito para filtrar.
    pub query: String,
    /// Bloques que coinciden: nombre, carpeta («global», «proyecto») y
    /// etiquetas (`#a #b`).
    pub items: Vec<(String, String, String)>,
    pub selected: usize,
    /// Descripción y texto (sin marcas de campo) del elegido.
    pub description: String,
    pub preview: Vec<String>,
    /// Confirmación pendiente (actualizar o borrar), o vacío.
    pub confirm: String,
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

/// Los efectos de un corte, para el panel de efectos.
#[derive(Debug, Clone, PartialEq)]
pub struct SliceFxView {
    pub slice: usize,
    pub fx: Vec<Fx>,
    pub automated: Vec<(usize, &'static str)>,
}

impl SliceFxView {
    /// Lo que se escribe del corte: `3 snare` o `3`.
    pub fn name(&self, labels: &[Option<String>]) -> String {
        match labels.get(self.slice).cloned().flatten() {
            Some(label) => format!("{} {label}", self.slice + 1),
            None => (self.slice + 1).to_string(),
        }
    }
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
    /// Vueltas del ciclo (`every`/`chance`; con 16 pasos, compases).
    pub cycle: usize,
    /// Pasos de una vuelta del patrón (`:length()`, 16 por defecto).
    pub length: usize,
    /// Pasos de la vuelta que está sonando (variante del ciclo).
    pub steps: Vec<Option<Step>>,
    /// Paso de su patrón (desde 0) que sonó el último, con el
    /// secuenciador en marcha.
    pub position: Option<usize>,
    /// Time-stretch del sample: tempo de origen y proporción (duración
    /// nueva / original).
    pub stretch: Option<(f64, f64)>,
    /// Corte que sonó por última vez (índice desde 0).
    pub playing_slice: Option<usize>,
    /// Medidor de nivel (pico con caída), de 0 a 1.
    pub level: f32,
    /// Volumen de la pista (lineal).
    pub gain: f32,
    /// Pan de la pista (de -1 a 1).
    pub pan: f32,
    /// Volumen, pan y envíos que se mueven con `ramp`/`lfo` (índices
    /// `engine::MIX_GAIN`, `MIX_PAN`, `MIX_SEND + bus`).
    pub mix_moving: [bool; engine::MIX_PARAMS],
    /// Efectos de la pista, en orden.
    pub fx: Vec<Fx>,
    /// Parámetros automatizados de cada efecto (`ramp`, `lfo`), por su
    /// índice en `fx`.
    pub automated: Vec<(usize, &'static str)>,
    /// Etiqueta de cada corte (si la tiene).
    pub labels: Vec<Option<String>>,
    /// Nota detectada de cada corte (`engine::detect_note`), si tiene un
    /// tono claro.
    pub notes: Vec<Option<engine::NoteGuess>>,
    /// Cortes con el inicio ajustado a mano.
    pub adjusted: Vec<bool>,
    /// Envíos a los buses: `[delay, reverb]`.
    pub sends: [f32; 2],
    /// Modo gate y grupo de choke.
    pub gate: bool,
    pub choke: Option<u8>,
    /// Notas MIDI de sus golpes: (canal, nota del corte 1).
    pub midi: Option<(u8, u8)>,
    /// Swing en porcentaje (50 = recto; 0 si no se ha dicho).
    pub swing: f32,
    /// La pista sola pasó de ±1 hace poco.
    pub clipping: bool,
    /// Silenciada (por el script o en vivo).
    pub mute: bool,
    /// En solo (por el script o en vivo).
    pub solo: bool,
    /// Toca su sample como instrumento (`sample:instrument()`).
    pub instrument: Option<InstrumentView>,
    /// Un sintetizador (`synth()`): su tipo y sus parámetros, en vez del
    /// sample (ver [`synth_text`]).
    pub synth: Option<SynthView>,
    /// Toca en granos (`:grains()`, `:freeze()`): su resumen
    /// ([`grains_text`]).
    pub grains: Option<String>,
    /// Formantes al transponer sin cambiar la duración (`:formant()` o
    /// el `formant` del instrumento): semitonos de desplazamiento.
    pub formant: Option<f32>,
    /// Efectos de corte (`cortes:fx()`): cada corte que los tiene (desde
    /// 0) con sus efectos y los parámetros que se mueven de cada uno
    /// (índice en sus efectos).
    pub slice_fx: Vec<SliceFxView>,
    /// El grupo al que va (`group()`), si va a uno.
    pub group: Option<String>,
}

/// Un grupo de pistas para el panel de efectos: como una pista, sin
/// sample.
#[derive(Debug, Clone, Default)]
pub struct GroupView {
    pub name: String,
    /// Sus pistas, por nombre.
    pub tracks: Vec<String>,
    pub gain: f32,
    pub pan: f32,
    pub sends: [f32; 2],
    /// Volumen, pan y envíos que se mueven (como en [`TrackView`]).
    pub mix_moving: [bool; engine::MIX_PARAMS],
    pub fx: Vec<Fx>,
    pub automated: Vec<(usize, &'static str)>,
    /// Medidor de nivel (pico con caída), de 0 a 1.
    pub level: f32,
}

/// Resumen de los granos de una pista para el panel de efectos:
/// `❄ 0.4 · 120 ms · 25/s` (congelados) o `⁘ 80 ms · 20/s` (si la lectura
/// avanza); `∿` en lo que se mueve con `ramp` o `lfo`.
pub fn grains_text(g: &engine::Granular) -> String {
    let show = |knob: &engine::Knob, unit: &str| match knob {
        engine::Knob::Fixed(v) => format!("{v}{unit}"),
        engine::Knob::Moving(_) => "∿".to_string(),
    };
    let size = show(&g.size_ms, " ms");
    let density = show(&g.density, "/s");
    match g.speed {
        engine::Knob::Fixed(0.0) => {
            format!("❄ {} · {size} · {density}", show(&g.position, ""))
        }
        _ => format!("⁘ {size} · {density}"),
    }
}

/// Una pista con sintetizador, para el panel del sample (que no tiene).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct SynthView {
    /// El tipo (`acid`).
    pub kind: &'static str,
    /// Sus parámetros ([`synth_text`]).
    pub params: String,
}

/// Los parámetros de un sintetizador como se escriben en `synth()`, con
/// `∿` en los que siguen una curva: `wave saw · cutoff 320 · res 0.82 ·
/// envmod ∿ ...`.
pub fn synth_text(def: &script::SynthDef) -> String {
    let Some(registered) = script::api::synth(def.kind) else {
        return String::new();
    };
    let round = |v: f32| ((v * 100.0).round() / 100.0).to_string();
    // El bloque sub, solo si suena (o se mueve).
    let on = |i: usize| def.patch.params[i] > 0.0 || def.curves[i].is_some();
    let sub_on = on(engine::synth::SUB);
    // El LFO y el PWM de la voz analógica, solo si mueven algo.
    let analog = def.patch.analog;
    let pwm_on = analog.is_some_and(|a| a.pwm != engine::synth::PwmSource::Manual);
    let lfo_on = on(engine::synth::LFO_PITCH)
        || on(engine::synth::LFO_FILTER)
        || analog.is_some_and(|a| a.pwm == engine::synth::PwmSource::Lfo);
    registered
        .params
        .iter()
        .filter(|p| sub_on || !p.name.starts_with("sub"))
        .filter(|p| lfo_on || !p.name.starts_with("lfo_"))
        .filter(|p| pwm_on || p.name != "pwm_amt")
        .filter_map(|p| match p.name {
            "wave" => Some(format!("wave {}", def.patch.wave.name())),
            "glide_mode" => Some(format!("glide_mode {}", analog?.glide.name())),
            "vca" => Some(format!("vca {}", analog?.vca.name())),
            "pwm_src" => Some(format!("pwm_src {}", analog?.pwm.name())),
            "lfo_wave" => Some(format!("lfo_wave {}", analog?.lfo_wave.name())),
            "sub_wave" => Some(format!("sub_wave {}", def.patch.sub_wave.name())),
            "root" => Some(format!(
                "root {}",
                script::patterns::note_name(def.patch.root.round() as i32)
            )),
            name => {
                let p = engine::synth::preset::preset(def.kind)?.param(name)?;
                let moving = match p.target {
                    engine::synth::preset::Target::Param(i) => def.curves[i].is_some(),
                    _ => false,
                };
                Some(if moving {
                    format!("{name} ∿")
                } else {
                    format!("{name} {}", round(p.get(&def.patch)))
                })
            }
        })
        .chain(def.patch.layers.iter().flat_map(|layers| {
            // Las capas que suenan: `o1 ÷1 ↻4`, `s1 ÷3 pan 0.5 level 0.7`.
            layers
                .iter()
                .zip(engine::synth::LAYER_NAMES)
                .filter(|(l, _)| l.level > 0.0)
                .map(move |(l, name)| {
                    let mut text = format!("{name} ÷{}", l.div);
                    if l.rdiv != 1 {
                        text.push_str(&format!(" ↻{}", l.rdiv));
                    }
                    if l.pan != 0.0 {
                        text.push_str(&format!(" pan {}", round(l.pan)));
                    }
                    if l.level != 1.0 {
                        text.push_str(&format!(" level {}", round(l.level)));
                    }
                    text
                })
        }))
        .collect::<Vec<_>>()
        .join(" · ")
}

/// Una pista con instrumento: su raíz y sus voces.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct InstrumentView {
    /// Nota MIDI en la que suena el sample tal cual (C4 = 60).
    pub root: i32,
    /// Voces del instrumento.
    pub voices: u8,
    /// Notas que sonaban a la vez (como mucho) en la última lectura del
    /// motor.
    pub sounding: usize,
}

/// La línea de tiempo de una canción con secciones.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct TimelineView {
    /// (sección, compases) en orden.
    pub entries: Vec<(String, u32)>,
    /// Entrada que suena y su compás (desde 0).
    pub current: usize,
    pub bar: u32,
    /// Se repite la entrada que suena.
    pub looping: bool,
    /// Entrada en la que está el cue.
    pub cue: Option<usize>,
}

impl ViewModel {
    /// Si la pista `i` se oye: sin mute y, si alguna está en solo, en solo.
    pub fn audible(&self, i: usize) -> bool {
        let any_solo = self.tracks.iter().any(|t| t.solo);
        self.tracks
            .get(i)
            .is_some_and(|t| !t.mute && (!any_solo || t.solo))
    }
}

impl TrackView {
    /// Texto de la celda del tracker de un paso: notas en un instrumento
    /// (`C4+E4+G4`), cortes en las demás (ver [`step_text`]).
    pub fn cell_text(&self, step: &Option<Step>) -> String {
        match self.instrument {
            Some(i) => note_text(step, i.root),
            None => step_text(step, &self.labels),
        }
    }

    /// Si la fila `row` alarga (con `_`) el golpe de una anterior.
    pub fn held(&self, row: usize) -> bool {
        let Some(before) = self.steps.get(..row) else {
            return false;
        };
        match before.iter().rposition(Option::is_some) {
            Some(j) => before[j].is_some_and(|s| s.hold as usize >= row - j),
            None => false,
        }
    }

    /// Etiqueta del corte `i` (desde 0), si la tiene.
    pub fn label(&self, i: usize) -> Option<&str> {
        self.labels.get(i).and_then(|l| l.as_deref())
    }

    /// Nota del corte `i` (desde 0) en cifrado americano, si tiene.
    pub fn note(&self, i: usize) -> Option<String> {
        self.notes.get(i).copied().flatten().map(|n| n.name())
    }
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

/// Texto de un paso en mini-notación (`03'^7@0.5r4`, o con la etiqueta
/// del corte si la tiene: `kick'^7`), o `---` si es silencio.
pub fn step_text(step: &Option<Step>, labels: &[Option<String>]) -> String {
    let Some(step) = step else {
        return "---".into();
    };
    let mut text = match labels.get(step.slice).and_then(|l| l.as_deref()) {
        Some(label) => label.to_string(),
        None => format!("{:02}", step.slice + 1),
    };
    if step.reverse {
        text.push('\'');
    }
    let chord = step.chord.iter().any(Option::is_some);
    if step.semitones != 0.0 || chord || step.keep {
        for (i, st) in step.notes().enumerate() {
            // `%`: el pitch que no cambia la duración (solo el primero).
            let mark = if i == 0 && step.keep { '%' } else { '^' };
            text.push_str(&format!("{mark}{st}"));
        }
    }
    text.push_str(&suffixes(step));
    text
}

/// Texto de un paso de un instrumento con raíz `root`: sus notas por su
/// nombre (`C4`, `C4+E4+G4`) y los demás sufijos, o `---`.
pub fn note_text(step: &Option<Step>, root: i32) -> String {
    let Some(step) = step else {
        return "---".into();
    };
    let notes: Vec<String> = step
        .notes()
        .map(|st| script::patterns::note_name(root + st.round() as i32))
        .collect();
    let mut text = notes.join("+");
    if step.reverse {
        text.push('\'');
    }
    text.push_str(&suffixes(step));
    text
}

/// Volumen, retrigger y corte de un paso (`@0.5r4~800`).
fn suffixes(step: &Step) -> String {
    let mut text = String::new();
    if step.gain != 1.0 {
        text.push_str(&format!("@{}", step.gain));
    }
    if step.retrig > 1 {
        text.push_str(&format!("r{}", step.retrig));
    }
    if let Some(hz) = step.lock {
        text.push_str(&format!("~{hz}"));
    }
    // El sub de la nota, accent y slide (de los sintetizadores).
    if step.sub.is_some() {
        text.push_str("{sub}");
    }
    if step.accent {
        text.push('!');
    }
    if step.slide {
        text.push('/');
    }
    // Efectos del paso: su nombre más corto del registro, con `∿` si algo
    // se mueve (`{lp∿,crush}`).
    if let Some(chain) = &step.fx {
        let names: Vec<String> = chain
            .effects()
            .enumerate()
            .map(|(i, fx)| {
                let moving = chain.automations().any(|a| a.fx == i);
                format!("{}{}", short_name(fx), if moving { "∿" } else { "" })
            })
            .collect();
        if !names.is_empty() {
            text.push_str(&format!("{{{}}}", names.join(",")));
        }
    }
    text
}

/// El nombre más corto (nombre o alias del registro) del efecto de `fx`.
pub fn short_name(fx: &Fx) -> &'static str {
    let name = match fx {
        Fx::Filter {
            kind: engine::FilterKind::LowPass,
            ..
        } => "lowpass",
        Fx::Filter { .. } => "highpass",
        Fx::Bitcrush { .. } => "bitcrush",
        Fx::Compressor { .. } => "compressor",
        Fx::Distortion { .. } => "distortion",
        Fx::Gain { .. } => "gain",
        Fx::Eq { .. } => "eq",
        Fx::DynEq(_) => "dyneq",
    };
    script::api::effect(name).map_or(name, |e| {
        std::iter::once(e.name)
            .chain(e.aliases.iter().copied())
            .min_by_key(|n| n.len())
            .unwrap_or(e.name)
    })
}

/// Descripción corta de un efecto.
/// Como [`fx_text`], con el nombre de la pista que escucha un EQ dinámico
/// por sidechain (`dyneq 80 -6 ⟵kick`).
pub fn fx_text_named(fx: &Fx, key: Option<&str>) -> String {
    match (fx, key) {
        (Fx::DynEq(p), Some(name)) => {
            format!("dyneq {} -{:.0} ⟵{name}", hz_label(p.freq_hz), p.range_db)
        }
        _ => fx_text(fx),
    }
}

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
        Fx::Compressor {
            threshold_db,
            ratio,
            makeup_db,
            ..
        } => {
            let makeup = if *makeup_db > 0.0 {
                format!(" +{makeup_db:.0}")
            } else {
                String::new()
            };
            format!("comp {threshold_db:.0} dB {ratio}:1{makeup}")
        }
        Fx::Distortion { drive, mix } => format!("drive ×{drive} mix {mix:.2}"),
        Fx::Gain { level } => format!("gain ×{level:.2}"),
        Fx::DynEq(p) => {
            let key = p
                .sidechain
                .map_or(String::new(), |k| format!(" ⟵pista {}", k + 1));
            format!("dyneq {} -{:.0}{key}", hz_label(p.freq_hz), p.range_db)
        }
        Fx::Eq { bands } => {
            let active: Vec<String> = bands
                .iter()
                .filter(|b| b.gain_db != 0.0)
                .map(|b| {
                    let kind = match b.kind {
                        engine::EqKind::Bell => "",
                        engine::EqKind::LowShelf => "≤",
                        engine::EqKind::HighShelf => "≥",
                    };
                    format!("{kind}{} {:+.1}", hz_label(b.freq_hz), b.gain_db)
                })
                .collect();
            if active.is_empty() {
                "eq plano".into()
            } else {
                format!("eq {}", active.join(" "))
            }
        }
    }
}

/// Frecuencia corta: `300`, `2.5k`, `12k`.
pub fn hz_label(hz: f32) -> String {
    if hz >= 10_000.0 {
        format!("{:.0}k", hz / 1000.0)
    } else if hz >= 1000.0 {
        let k = format!("{:.1}", hz / 1000.0);
        format!("{}k", k.trim_end_matches(".0"))
    } else {
        format!("{hz:.0}")
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
    fn writes_chords_and_notes() {
        let chord = Step::new(0).with_chord(&[4.0, 7.0]).with_gain(0.5);
        assert_eq!(step_text(&Some(chord), &[]), "01^0^4^7@0.5");
        let parsed = engine::notation::parse_step("1^0^4^7@0.5").unwrap();
        assert_eq!(parsed, Some(chord));
        // `%`: el pitch que no cambia la duración (también con 0).
        for text in ["01%7", "01%-12^4", "01%0"] {
            let step = engine::notation::parse_step(text).unwrap();
            assert_eq!(step_text(&step, &[]), text);
        }
        // En un instrumento, por su nombre desde la raíz (C3 = 48).
        assert_eq!(note_text(&Some(chord), 48), "C3+E3+G3@0.5");
        let low = Step::new(0).with_pitch(-13.0).with_retrig(2);
        assert_eq!(note_text(&Some(low), 48), "B1r2");
        assert_eq!(note_text(&None, 48), "---");
        // Accent y slide, detrás (`C2!`, `D#2/`).
        let tied = engine::notation::parse_step("1^3!/").unwrap();
        assert_eq!(note_text(&tied, 36), "D#2!/");
        assert_eq!(step_text(&tied, &[]), "01^3!/");
        let sub =
            engine::notation::parse_step_with("1{sub=0.5}!", &[], Some(&script::step_fx::resolve))
                .unwrap();
        assert_eq!(note_text(&sub, 36), "C2{sub}!");
    }

    #[test]
    fn writes_steps_in_notation() {
        assert_eq!(step_text(&None, &[]), "---");
        assert_eq!(step_text(&Some(Step::new(2)), &[]), "03");
        let fancy = Step::new(0)
            .reversed()
            .with_pitch(-5.0)
            .with_gain(0.5)
            .with_retrig(4);
        assert_eq!(step_text(&Some(fancy), &[]), "01'^-5@0.5r4");
        // Con etiqueta, el nombre en vez del número (y se vuelve a leer igual).
        let labels = vec![Some("kick".to_string()), None];
        assert_eq!(step_text(&Some(fancy), &labels), "kick'^-5@0.5r4");
        assert_eq!(step_text(&Some(Step::new(1)), &labels), "02");
        let parsed = engine::notation::parse_step_labeled("kick'^-5@0.5r4", &labels).unwrap();
        assert_eq!(parsed, Some(fancy));
        // Se puede volver a leer con la notación (con el número sin cero a la izquierda).
        let parsed = engine::notation::parse_step("1'^-5@0.5r4").unwrap();
        assert_eq!(parsed, Some(fancy));
    }

    #[test]
    fn describes_effects() {
        assert_eq!(fx_text(&Fx::lowpass(3000.0)), "LP 3000 Hz q0.71");
        assert_eq!(fx_text(&Fx::highpass(120.5)), "HP 120 Hz q0.71");
        assert_eq!(fx_text(&Fx::bitcrush(6, 2)), "crush 6b ÷2");
        assert_eq!(fx_text(&Fx::compressor(-18.0, 4.0)), "comp -18 dB 4:1");
        assert_eq!(fx_text(&Fx::distortion(8.0, 0.5)), "drive ×8 mix 0.50");
        let dyn_eq = Fx::DynEq(engine::DynEqParams::new(80.0).with_sidechain(2));
        assert_eq!(fx_text(&dyn_eq), "dyneq 80 -6 ⟵pista 3");
        assert_eq!(fx_text_named(&dyn_eq, Some("kick")), "dyneq 80 -6 ⟵kick");
        assert_eq!(
            fx_text(&Fx::DynEq(engine::DynEqParams::new(2500.0))),
            "dyneq 2.5k -6"
        );
    }
}
