use std::fmt;
use std::ops::Range;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use rtrb::{Consumer, Producer, RingBuffer};

use crate::automation::mix;
use crate::bus::{BUSES, Buses, Rack};
use crate::fx::{Fx, FxState, MAX_FX};
use crate::limiter::{LIMIT_THRESHOLD, soft_limit};
use crate::metronome::Metronome;
use crate::sample::Sample;
use crate::sequencer::{Pattern, STEPS, Transport, Trigger, valid_bpm};
use crate::slice::Slices;
use crate::status::{SharedStatus, Status};

const COMMAND_CAPACITY: usize = 64;
// A la basura solo va un `Arc` cuando el motor no guarda otra referencia al
// mismo sample (ver `Engine::discard`), así que las voces que dispara el
// secuenciador no la llenan: cada elemento sale de un comando y el
// controlador la recoge antes de cada envío.
const GARBAGE_CAPACITY: usize = COMMAND_CAPACITY * 2;
/// Número máximo de pistas. Cada una tiene su patrón y su voz.
pub const MAX_TRACKS: usize = 8;
/// Frames por trozo de mezcla. Los buffers de pista y maestro tienen este
/// tamaño fijo; los bloques del callback se procesan en trozos.
pub(crate) const CHUNK: usize = 256;
/// Constante de tiempo del suavizado del volumen de pista.
const GAIN_SMOOTHING_SECONDS: f32 = 0.005;
/// Frames entre recálculos de los parámetros automatizados.
const MOD_BLOCK: usize = 32;
/// Duración de los fundidos de entrada y salida de cada voz.
const FADE_SECONDS: f32 = 0.0015;

// `Pattern` va dentro del comando y no en una `Box`: liberar la `Box` en el
// hilo de audio rompería la regla de no liberar memoria en el callback.
#[allow(clippy::large_enum_variant)]
enum Command {
    /// Reproduce los frames `start..end` del sample en la pista 0.
    Play {
        track: usize,
        sample: Arc<Sample>,
        start: usize,
        end: usize,
        playback: Playback,
    },
    /// Patrón de una pista para el siguiente compás (`None` la silencia).
    SetPattern {
        track: usize,
        pattern: Option<Pattern>,
    },
    /// BPM para el siguiente compás.
    SetBpm(f64),
    /// Parámetros de los buses de envío para el siguiente compás.
    SetBuses(Buses),
    /// Empieza un lote: los cambios se guardan aparte hasta `CommitBatch`.
    BeginBatch,
    /// Hace efectivos a la vez todos los cambios del lote.
    CommitBatch,
    /// Arranca el secuenciador desde el paso 0.
    Start,
    /// Para el secuenciador y la voz que suena.
    Stop,
    /// Mute en vivo de una pista (inmediato, no espera al compás).
    SetMute { track: usize, on: bool },
    /// Solo en vivo de una pista (inmediato).
    SetSolo { track: usize, on: bool },
    /// Enciende o apaga el metrónomo (inmediato).
    SetMetronome(bool),
    /// Preescucha `start..end` del sample en la voz aparte (inmediato).
    Preview {
        sample: Arc<Sample>,
        start: usize,
        end: usize,
    },
    /// Corta la preescucha (con fundido).
    StopPreview,
}

/// Cómo se reproduce un rango: ganancia, velocidad de lectura (pitch),
/// sentido y la forma del corte (envolvente; ver [`SliceShape`]).
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Playback {
    pub gain: f32,
    /// Frames del sample por frame de salida: 2.0 es una octava arriba (y
    /// dura la mitad), 0.5 una octava abajo (y dura el doble).
    pub rate: f64,
    pub reverse: bool,
    pub shape: SliceShape,
}

impl Playback {
    pub const NORMAL: Playback = Playback {
        gain: 1.0,
        rate: 1.0,
        reverse: false,
        shape: SliceShape::DEFAULT,
    };
}

/// Cómo se lee un corte.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PlayMode {
    /// Una vez, de principio a fin.
    #[default]
    Once,
    /// En bucle dentro del corte hasta que otro golpe (o la puerta, o el
    /// choke) lo corte.
    Loop,
    /// En bucle de ida y vuelta.
    PingPong,
}

/// Forma de un corte: su envolvente (ataque y cola, en ms; con 0, el
/// fundido mínimo de siempre, 1,5 ms, que evita los clics), su posición
/// en el estéreo, qué parte suena y cómo se lee. Se fija por corte
/// (`Pattern::with_shapes`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SliceShape {
    /// Rampa de entrada, en ms.
    pub attack_ms: f32,
    /// Rampa de salida al final del corte (o al cerrarse la puerta en el
    /// modo gate), en ms.
    pub release_ms: f32,
    /// Panorama, de -1 (izquierda) a 1 (derecha); 0, al centro sin tocar
    /// nada (ley de balance: el lado contrario baja, el propio no sube).
    pub pan: f32,
    /// Parte del corte que suena, desde su inicio: de 0,01 a 1 (entero).
    pub length: f32,
    pub play: PlayMode,
}

impl SliceShape {
    pub const DEFAULT: SliceShape = SliceShape {
        attack_ms: 0.0,
        release_ms: 0.0,
        pan: 0.0,
        length: 1.0,
        play: PlayMode::Once,
    };

    pub fn validate(&self) -> Result<(), &'static str> {
        if !(0.0..=MAX_ATTACK_MS).contains(&self.attack_ms) {
            return Err("ataque fuera de rango (0 a 2000 ms)");
        }
        if !(0.0..=MAX_RELEASE_MS).contains(&self.release_ms) {
            return Err("cola fuera de rango (0 a 5000 ms)");
        }
        if !(-1.0..=1.0).contains(&self.pan) {
            return Err("pan fuera de rango (-1 a 1)");
        }
        if !(0.01..=1.0).contains(&self.length) {
            return Err("length fuera de rango (0.01 a 1: la parte del corte que suena)");
        }
        Ok(())
    }

    /// El rango que suena de un corte (`length`).
    pub fn range(&self, range: Range<usize>) -> Range<usize> {
        if self.length >= 1.0 {
            return range;
        }
        let frames = ((range.len() as f64 * self.length as f64).round() as usize).max(1);
        range.start..(range.start + frames).min(range.end)
    }
}

impl Default for SliceShape {
    fn default() -> Self {
        Self::DEFAULT
    }
}

/// Ataque y cola máximos de un corte.
pub const MAX_ATTACK_MS: f32 = 2000.0;
pub const MAX_RELEASE_MS: f32 = 5000.0;
/// Grupos de choke: de 1 a este.
pub const MAX_CHOKE: u8 = 16;

struct Voice {
    sample: Arc<Sample>,
    start: usize,
    end: usize,
    playback: Playback,
    /// Frames de salida ya renderizados y total que dura la voz.
    rendered: usize,
    length: usize,
    /// Fundido mínimo (el de interrumpir una voz), y rampas de entrada y de
    /// salida (al menos el mínimo; más con ataque o cola).
    fade: usize,
    fade_in: usize,
    fade_out: usize,
    /// Fundido de salida en curso: frames que quedan y total (voz
    /// interrumpida o puerta cerrada en el modo gate).
    release: Option<(usize, usize)>,
    /// Modo gate: frame de salida en el que se cierra la puerta.
    gate_at: Option<usize>,
}

impl Voice {
    fn new(sample: Arc<Sample>, start: usize, end: usize, playback: Playback) -> Self {
        let rate = sample.sample_rate() as f32;
        let fade = ((FADE_SECONDS * rate) as usize).max(1);
        let ms = |ms: f32| (ms / 1000.0 * rate) as usize;
        // Último frame de salida: el que lee el último frame del rango. En
        // bucle, la voz no acaba sola.
        let length = match (end - start, playback.shape.play) {
            (0, _) => 0,
            (_, PlayMode::Loop | PlayMode::PingPong) => usize::MAX,
            (frames, PlayMode::Once) => ((frames - 1) as f64 / playback.rate).floor() as usize + 1,
        };
        Voice {
            sample,
            start,
            end,
            playback,
            rendered: 0,
            length,
            fade,
            fade_in: fade.max(ms(playback.shape.attack_ms)),
            fade_out: fade.max(ms(playback.shape.release_ms)),
            release: None,
            gate_at: None,
        }
    }

    /// Modo gate: la voz empieza su cola tras `frames` frames de salida.
    fn with_gate(mut self, frames: usize) -> Self {
        self.gate_at = Some(frames);
        self
    }

    fn is_active(&self) -> bool {
        self.rendered < self.length && !matches!(self.release, Some((0, _)))
    }

    /// Empieza un fundido de salida aunque el corte no haya terminado.
    fn release(&mut self) {
        if self.release.is_none() {
            self.release = Some((self.fade, self.fade));
        }
    }

    /// Envolvente del frame actual: rampa de entrada desde el inicio de la
    /// reproducción, de salida hacia su final y, si se interrumpió (o se
    /// cerró la puerta), la del release. En tiempo de salida, así es igual
    /// con pitch o reverse.
    fn envelope(&self) -> f32 {
        let from_start = self.rendered as f32 / self.fade_in as f32;
        let mut gain = if self.length == usize::MAX {
            // En bucle no hay final al que llegar.
            from_start.min(1.0)
        } else {
            let to_end = (self.length - 1 - self.rendered) as f32 / self.fade_out as f32;
            from_start.min(to_end).min(1.0)
        };
        if let Some((left, total)) = self.release {
            gain *= (left - 1) as f32 / total as f32;
        }
        gain
    }

    /// Frame del sample para el frame de salida actual, con interpolación
    /// lineal si la posición cae entre dos frames.
    fn read(&self) -> (f32, f32) {
        let mut offset = self.rendered as f64 * self.playback.rate;
        let span = (self.end - self.start) as f64;
        match self.playback.shape.play {
            PlayMode::Once => {}
            PlayMode::Loop => offset = offset.rem_euclid(span),
            PlayMode::PingPong => {
                // Ida y vuelta sin repetir los extremos: 0, 1 … n-1, n-2 … 1.
                let last = span - 1.0;
                offset = if last > 0.0 {
                    let t = offset.rem_euclid(2.0 * last);
                    if t <= last { t } else { 2.0 * last - t }
                } else {
                    0.0
                };
            }
        }
        let pos = if self.playback.reverse {
            (self.end - 1) as f64 - offset
        } else {
            self.start as f64 + offset
        };
        let i0 = pos.floor() as usize;
        let frac = (pos - i0 as f64) as f32;
        let (l0, r0) = self.sample.frame(i0);
        if frac == 0.0 {
            return (l0, r0);
        }
        let (l1, r1) = self.sample.frame((i0 + 1).min(self.end - 1));
        (l0 + frac * (l1 - l0), r0 + frac * (r1 - r0))
    }

    /// Suma la voz en `out` (estéreo entrelazado).
    fn mix_into(&mut self, out: &mut [f32]) {
        for frame in out.as_chunks_mut::<2>().0 {
            // Modo gate: al cerrarse la puerta empieza la cola.
            if self.gate_at == Some(self.rendered) && self.release.is_none() {
                self.release = Some((self.fade_out, self.fade_out));
            }
            if !self.is_active() {
                break;
            }
            let gain = self.playback.gain * self.envelope();
            let (l, r) = self.read();
            // Ley de balance: al centro, los dos lados por 1 (sin cambios).
            let pan = self.playback.shape.pan;
            frame[0] += (1.0 - pan).min(1.0) * (gain * l);
            frame[1] += (1.0 + pan).min(1.0) * (gain * r);
            self.rendered += 1;
            if let Some((left, _)) = &mut self.release {
                *left -= 1;
            }
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum SendError {
    /// La cola de comandos está llena; el comando no se envió.
    QueueFull,
    /// El número de corte no existe.
    NoSuchSlice { index: usize, count: usize },
    /// BPM fuera de rango.
    InvalidBpm(f64),
    /// Parámetro de un paso fuera de rango.
    InvalidStep { step: usize, reason: &'static str },
    /// Número de pista fuera de rango.
    NoSuchTrack(usize),
    /// Volumen de pista fuera de rango.
    InvalidTrackGain(f32),
    /// Efecto de pista con parámetros fuera de rango.
    InvalidFx(&'static str),
    /// Ciclo de patrón con un número de compases fuera de rango.
    InvalidCycle(usize),
    /// Longitud de patrón fuera de rango (o vueltas de distinta longitud).
    InvalidLength(usize),
    /// Envío o parámetro de un bus fuera de rango.
    InvalidBus(&'static str),
}

impl fmt::Display for SendError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SendError::QueueFull => write!(f, "cola de comandos llena"),
            SendError::NoSuchSlice { index, count } => {
                write!(f, "no existe el corte {index} (hay {count})")
            }
            SendError::InvalidFx(reason) => write!(f, "efecto: {reason}"),
            SendError::InvalidBus(reason) => write!(f, "envío: {reason}"),
            SendError::InvalidCycle(bars) => write!(
                f,
                "ciclo de {bars} compases fuera de rango (1 a {})",
                crate::sequencer::MAX_CYCLE_BARS
            ),
            SendError::InvalidLength(steps) => write!(
                f,
                "patrón de {steps} pasos fuera de rango (1 a {})",
                crate::sequencer::MAX_STEPS
            ),
            SendError::InvalidTrackGain(gain) => {
                write!(f, "volumen de pista {gain} fuera de rango (0 a 4)")
            }
            SendError::NoSuchTrack(track) => {
                write!(f, "no existe la pista {track} (máximo {MAX_TRACKS})")
            }
            SendError::InvalidStep { step, reason } => {
                write!(f, "paso {}: {reason}", step + 1)
            }
            SendError::InvalidBpm(bpm) => write!(
                f,
                "BPM {bpm} fuera de rango ({}-{})",
                crate::sequencer::MIN_BPM,
                crate::sequencer::MAX_BPM
            ),
        }
    }
}

impl std::error::Error for SendError {}

/// Cambios que se aplican juntos, en el mismo inicio de compás. Se prepara
/// en el hilo de control y se envía con [`Controller::apply`].
#[derive(Default)]
pub struct Batch {
    bpm: Option<f64>,
    buses: Option<Buses>,
    patterns: Vec<(usize, Option<Pattern>)>,
}

impl Batch {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn set_bpm(&mut self, bpm: f64) -> &mut Self {
        self.bpm = Some(bpm);
        self
    }

    /// Parámetros de los buses de envío (delay y reverb).
    pub fn set_buses(&mut self, buses: Buses) -> &mut Self {
        self.buses = Some(buses);
        self
    }

    /// Patrón de la pista `track`; `None` la silencia.
    pub fn set_pattern(&mut self, track: usize, pattern: Option<Pattern>) -> &mut Self {
        self.patterns.push((track, pattern));
        self
    }
}

/// Lo que el hilo de audio devuelve al de control para que lo libere allí.
/// Su contenido no se lee: solo se suelta en el hilo de control. El
/// patrón va sin `Box` por lo mismo que en `Command`: meterlo en una caja
/// reservaría memoria en el callback.
#[allow(dead_code, clippy::large_enum_variant)]
enum Garbage {
    Sample(Arc<Sample>),
    /// Un patrón entero: lleva memoria propia (sus compases).
    Pattern(Pattern),
}

/// Lado de control (UI/script). Envía comandos al hilo de audio sin bloquear.
pub struct Controller {
    commands: Producer<Command>,
    garbage: Consumer<Garbage>,
    playing: Arc<AtomicBool>,
    status: Arc<SharedStatus>,
}

/// Una pista en el hilo de audio: su patrón (con cambios pendientes) y su voz.
struct Track {
    pattern: Option<Pattern>,
    /// Cambio que entra en el siguiente compás (`Some(None)` = silenciar).
    pending: Option<Option<Pattern>>,
    /// Cambio de un lote todavía abierto.
    staged: Option<Option<Pattern>>,
    voice: Option<Voice>,
    /// Voz interrumpida que todavía está haciendo su fundido de salida.
    fading: Option<Voice>,
    /// Volumen actual, que se acerca suavemente al del patrón.
    gain: f32,
    /// Estado de los efectos del patrón actual.
    fx: [FxState; MAX_FX],
    /// Subdisparos pendientes del paso actual (retrigger).
    retrig: Option<Retrig>,
    /// Mute y solo en vivo (desde la interfaz).
    live_mute: bool,
    live_solo: bool,
    /// Ganancia de mute/solo que se acerca a 0 o 1 (sin clics).
    audible: f32,
    /// La voz acaba de empezar desde el silencio: el mute/solo entra de
    /// golpe (no hay nada que suavizar).
    fresh: bool,
    /// Nivel de cada envío, que se acerca suavemente al del patrón.
    sends: [f32; BUSES],
    /// Corte del primer filtro puesto por el último golpe (`~`).
    lock: Option<f32>,
    /// Valor (suavizado) de cada parámetro automatizable de cada efecto
    /// mientras se mueve; `None`: el fijo del efecto.
    mods: [[Option<f32>; 2]; MAX_FX],
}

/// Disparos pendientes de una pista: el golpe empieza en `start` (el
/// frame del paso más su retraso) y dura `length` frames (los del paso);
/// se divide en `count` partes (retrigger) y ya se han disparado `done`.
/// Un golpe retrasado empieza con `done` = 0.
#[derive(Clone, Copy)]
struct Retrig {
    hit: Trigger,
    count: u8,
    done: u8,
    start: u64,
    length: u64,
}

impl Retrig {
    /// Frame del siguiente subdisparo: round(done · length / count).
    fn next_frame(&self) -> u64 {
        let count = self.count as u64;
        self.start + (self.done as u64 * self.length * 2 + count) / (2 * count)
    }
}

impl Default for Track {
    fn default() -> Self {
        Track {
            pattern: None,
            pending: None,
            staged: None,
            voice: None,
            fading: None,
            gain: 1.0,
            fx: [FxState::Off; MAX_FX],
            retrig: None,
            live_mute: false,
            live_solo: false,
            audible: 1.0,
            fresh: false,
            sends: [0.0; BUSES],
            lock: None,
            mods: [[None; 2]; MAX_FX],
        }
    }
}

impl Track {
    fn muted(&self) -> bool {
        self.live_mute || self.pattern.as_ref().is_some_and(|p| p.mute)
    }

    fn soloed(&self) -> bool {
        self.live_solo || self.pattern.as_ref().is_some_and(|p| p.solo)
    }
}

/// Lado de audio. Se mueve al callback de audio (o se usa para renderizar
/// offline). `process` no reserva memoria, no bloquea y no hace E/S.
pub struct Engine {
    commands: Consumer<Command>,
    garbage: Producer<Garbage>,
    playing: Arc<AtomicBool>,
    status: Arc<SharedStatus>,
    sample_rate: u32,
    transport: Transport,
    tracks: [Track; MAX_TRACKS],
    /// Buffers de mezcla (estéreo entrelazado), de tamaño fijo.
    track_buf: [f32; 2 * CHUNK],
    master: [f32; 2 * CHUNK],
    batch_open: bool,
    staged_bpm: Option<f64>,
    metronome: Metronome,
    /// Voz de preescucha: aparte de las pistas (sin su volumen, efectos ni
    /// mute), y la que hace su fundido al ser sustituida.
    preview: Option<Voice>,
    preview_fading: Option<Voice>,
    /// Buses de envío (delay y reverb), sus parámetros pendientes y lo que
    /// les mandan las pistas en cada trozo.
    rack: Rack,
    pending_buses: Option<Buses>,
    staged_buses: Option<Buses>,
    send_buf: [[f32; 2 * CHUNK]; BUSES],
}

/// Crea el par controlador/motor. `sample_rate` es la frecuencia a la que
/// renderiza el motor (la de los samples cargados para él).
pub fn new_engine(sample_rate: u32) -> (Controller, Engine) {
    let (cmd_tx, cmd_rx) = RingBuffer::new(COMMAND_CAPACITY);
    let (garbage_tx, garbage_rx) = RingBuffer::new(GARBAGE_CAPACITY);
    let playing = Arc::new(AtomicBool::new(false));
    let status = Arc::new(SharedStatus::new(crate::sequencer::DEFAULT_BPM));
    (
        Controller {
            commands: cmd_tx,
            garbage: garbage_rx,
            playing: playing.clone(),
            status: status.clone(),
        },
        Engine {
            commands: cmd_rx,
            garbage: garbage_tx,
            playing,
            status,
            sample_rate,
            transport: Transport::new(),
            tracks: std::array::from_fn(|_| Track::default()),
            track_buf: [0.0; 2 * CHUNK],
            master: [0.0; 2 * CHUNK],
            batch_open: false,
            staged_bpm: None,
            metronome: Metronome::default(),
            preview: None,
            preview_fading: None,
            rack: Rack::new(sample_rate),
            pending_buses: None,
            staged_buses: None,
            send_buf: [[0.0; 2 * CHUNK]; BUSES],
        },
    )
}

impl Controller {
    /// Reproduce el sample entero desde el principio (en la pista 0).
    pub fn play(&mut self, sample: Arc<Sample>) -> Result<(), SendError> {
        let frames = sample.frames();
        self.play_range(sample, 0..frames)
    }

    /// Reproduce el corte número `index` (empezando en 0) en la pista 0.
    pub fn play_slice(
        &mut self,
        sample: &Arc<Sample>,
        slices: &Slices,
        index: usize,
    ) -> Result<(), SendError> {
        self.play_slice_on(0, sample, slices, index)
    }

    /// Reproduce el corte `index` en la pista `track`, ya (sin esperar al
    /// compás): para tocar en vivo. Suena con el volumen y los efectos de la
    /// pista y corta su voz, como un paso del secuenciador.
    pub fn play_slice_on(
        &mut self,
        track: usize,
        sample: &Arc<Sample>,
        slices: &Slices,
        index: usize,
    ) -> Result<(), SendError> {
        self.play_slice_shaped(track, sample, slices, index, SliceShape::DEFAULT)
    }

    /// Como [`Controller::play_slice_on`], con la forma del corte (la de
    /// su pista: `Pattern::with_shapes`).
    pub fn play_slice_shaped(
        &mut self,
        track: usize,
        sample: &Arc<Sample>,
        slices: &Slices,
        index: usize,
        shape: SliceShape,
    ) -> Result<(), SendError> {
        let range = slices.get(index).ok_or(SendError::NoSuchSlice {
            index,
            count: slices.len(),
        })?;
        shape.validate().map_err(|reason| SendError::InvalidStep {
            step: index,
            reason,
        })?;
        self.play_range_shaped(track, sample.clone(), shape.range(range), shape)
    }

    /// Reproduce los frames `range` del sample (se recorta a su longitud) en
    /// la pista 0.
    pub fn play_range(
        &mut self,
        sample: Arc<Sample>,
        range: Range<usize>,
    ) -> Result<(), SendError> {
        self.play_range_on(0, sample, range)
    }

    /// Reproduce los frames `range` del sample en la pista `track`, ya.
    pub fn play_range_on(
        &mut self,
        track: usize,
        sample: Arc<Sample>,
        range: Range<usize>,
    ) -> Result<(), SendError> {
        self.play_range_shaped(track, sample, range, SliceShape::DEFAULT)
    }

    fn play_range_shaped(
        &mut self,
        track: usize,
        sample: Arc<Sample>,
        range: Range<usize>,
        shape: SliceShape,
    ) -> Result<(), SendError> {
        if track >= MAX_TRACKS {
            return Err(SendError::NoSuchTrack(track));
        }
        let end = range.end.min(sample.frames());
        let start = range.start.min(end);
        let playback = Playback {
            shape,
            ..Playback::NORMAL
        };
        let cmd = Command::Play {
            track,
            sample,
            start,
            end,
            playback,
        };
        self.send(cmd, Some(start < end))
    }

    /// Cambia el patrón de la pista 0. Entra al empezar el siguiente compás
    /// (o al arrancar).
    pub fn set_pattern(&mut self, pattern: Pattern) -> Result<(), SendError> {
        self.set_track_pattern(0, Some(pattern))
    }

    /// Cambia el patrón de una pista (`None` la silencia). Entra al empezar
    /// el siguiente compás (o al arrancar).
    pub fn set_track_pattern(
        &mut self,
        track: usize,
        pattern: Option<Pattern>,
    ) -> Result<(), SendError> {
        if track >= MAX_TRACKS {
            return Err(SendError::NoSuchTrack(track));
        }
        self.send(Command::SetPattern { track, pattern }, None)
    }

    /// Cambia el tempo. Entra al empezar el siguiente compás (o al arrancar).
    pub fn set_bpm(&mut self, bpm: f64) -> Result<(), SendError> {
        if !valid_bpm(bpm) {
            return Err(SendError::InvalidBpm(bpm));
        }
        self.send(Command::SetBpm(bpm), None)
    }

    /// Envía un lote de cambios que entran juntos en el mismo compás. Se
    /// valida entero y se comprueba que cabe en la cola antes de enviar nada:
    /// o entra todo o no entra nada.
    pub fn apply(&mut self, batch: Batch) -> Result<(), SendError> {
        if let Some(bpm) = batch.bpm
            && !valid_bpm(bpm)
        {
            return Err(SendError::InvalidBpm(bpm));
        }
        if let Some(&(track, _)) = batch.patterns.iter().find(|(t, _)| *t >= MAX_TRACKS) {
            return Err(SendError::NoSuchTrack(track));
        }
        if let Some(buses) = &batch.buses {
            buses.validate().map_err(SendError::InvalidBus)?;
        }
        self.collect_garbage();
        let needed = 2
            + batch.patterns.len()
            + usize::from(batch.bpm.is_some())
            + usize::from(batch.buses.is_some());
        if self.commands.slots() < needed {
            return Err(SendError::QueueFull);
        }
        let mut push = |cmd| {
            // No falla: se ha comprobado el espacio y solo este hilo escribe.
            let _ = self.commands.push(cmd);
        };
        push(Command::BeginBatch);
        if let Some(bpm) = batch.bpm {
            push(Command::SetBpm(bpm));
        }
        if let Some(buses) = batch.buses {
            push(Command::SetBuses(buses));
        }
        for (track, pattern) in batch.patterns {
            push(Command::SetPattern { track, pattern });
        }
        push(Command::CommitBatch);
        Ok(())
    }

    /// Cambia los parámetros de los buses de envío. Entran al empezar el
    /// siguiente compás (o al arrancar).
    pub fn set_buses(&mut self, buses: Buses) -> Result<(), SendError> {
        buses.validate().map_err(SendError::InvalidBus)?;
        self.send(Command::SetBuses(buses), None)
    }

    /// Arranca el secuenciador en el paso 0, aplicando lo pendiente.
    pub fn start(&mut self) -> Result<(), SendError> {
        self.send(Command::Start, Some(true))
    }

    /// Silencia (o vuelve a dejar sonar) una pista ya, sin esperar al
    /// compás. Se suma al `:mute()` del script.
    pub fn set_mute(&mut self, track: usize, on: bool) -> Result<(), SendError> {
        if track >= MAX_TRACKS {
            return Err(SendError::NoSuchTrack(track));
        }
        self.send(Command::SetMute { track, on }, None)
    }

    /// Pone (o quita) una pista en solo ya: si alguna lo está, solo suenan
    /// esas. Se suma al `:solo()` del script.
    pub fn set_solo(&mut self, track: usize, on: bool) -> Result<(), SendError> {
        if track >= MAX_TRACKS {
            return Err(SendError::NoSuchTrack(track));
        }
        self.send(Command::SetSolo { track, on }, None)
    }

    /// Preescucha el corte `index` en una voz aparte: no corta ni pasa por
    /// ninguna pista (ni su volumen, ni sus efectos, ni su mute), y suena
    /// también con el secuenciador parado. Inmediato.
    pub fn preview_slice(
        &mut self,
        sample: &Arc<Sample>,
        slices: &Slices,
        index: usize,
    ) -> Result<(), SendError> {
        let range = slices.get(index).ok_or(SendError::NoSuchSlice {
            index,
            count: slices.len(),
        })?;
        let end = range.end.min(sample.frames());
        let start = range.start.min(end);
        let cmd = Command::Preview {
            sample: sample.clone(),
            start,
            end,
        };
        self.send(cmd, Some(start < end))
    }

    /// Corta la preescucha (con fundido).
    pub fn stop_preview(&mut self) -> Result<(), SendError> {
        self.send(Command::StopPreview, None)
    }

    /// Enciende o apaga el metrónomo: un clic en cada pulso, con acento en
    /// el primero del compás. Inmediato.
    pub fn set_metronome(&mut self, on: bool) -> Result<(), SendError> {
        self.send(Command::SetMetronome(on), None)
    }

    /// Para el secuenciador y las voces que suenan (con fundido).
    pub fn stop(&mut self) -> Result<(), SendError> {
        self.send(Command::Stop, Some(false))
    }

    pub fn is_playing(&self) -> bool {
        self.playing.load(Ordering::Relaxed)
    }

    /// Estado del motor para la interfaz (tempo, compás, paso, picos...).
    /// Los picos de las pistas se ponen a cero al leerlos.
    pub fn status(&self) -> Status {
        self.status.snapshot()
    }

    /// Libera aquí (fuera del hilo de audio) los samples que el motor ya no usa.
    pub fn collect_garbage(&mut self) {
        while self.garbage.pop().is_ok() {}
    }

    fn send(&mut self, cmd: Command, playing: Option<bool>) -> Result<(), SendError> {
        self.collect_garbage();
        self.commands.push(cmd).map_err(|_| SendError::QueueFull)?;
        // Estado previsto hasta que el motor procese el comando y lo actualice.
        if let Some(playing) = playing {
            self.playing.store(playing, Ordering::Relaxed);
        }
        Ok(())
    }
}

impl Engine {
    /// Cambia la frecuencia del motor (en el hilo de control, antes de
    /// arrancar la salida): los buffers de los buses se vuelven a reservar.
    pub(crate) fn set_sample_rate(&mut self, sample_rate: u32) {
        self.sample_rate = sample_rate;
        let params = self.rack.params;
        self.rack = Rack::new(sample_rate);
        self.rack.params = params;
    }

    /// Rellena `out` (entrelazado, `channels` canales) con el siguiente bloque.
    pub fn process(&mut self, out: &mut [f32], channels: usize) {
        while let Ok(cmd) = self.commands.pop() {
            self.handle(cmd);
        }

        let frames = out.len() / channels;
        let mut offset = 0;
        while offset < frames {
            let mut n = (frames - offset).min(CHUNK);
            if self.transport.running {
                let now = self.transport.frame;
                let next = self.transport.next_step_frame(self.sample_rate);
                if next <= now {
                    self.step();
                    continue;
                }
                if let Some(t) = (0..MAX_TRACKS)
                    .find(|&t| self.tracks[t].retrig.is_some_and(|r| r.next_frame() <= now))
                {
                    self.fire_retrig(t);
                    continue;
                }
                // Partir el bloque en el siguiente paso o subdisparo: disparo
                // exacto al frame.
                let next_retrig = self
                    .tracks
                    .iter()
                    .filter_map(|t| t.retrig.map(|r| r.next_frame()))
                    .min()
                    .unwrap_or(u64::MAX);
                n = n.min((next.min(next_retrig) - now) as usize);
            }
            self.mix(n);
            write_out(
                &self.master[..2 * n],
                &mut out[offset * channels..(offset + n) * channels],
                channels,
            );
            offset += n;
            if self.transport.running {
                self.transport.frame += n as u64;
            }
        }

        // Las voces terminadas se quedan en su pista hasta que otro comando o
        // paso las reemplace.
        let playing = self.transport.running
            || self
                .tracks
                .iter()
                .any(|t| t.voice.as_ref().is_some_and(Voice::is_active) || t.fading.is_some())
            || self.preview.as_ref().is_some_and(Voice::is_active)
            || self.preview_fading.is_some();
        self.playing.store(playing, Ordering::Relaxed);
        self.status
            .set_transport(self.transport.running, self.transport.bpm());
    }

    fn handle(&mut self, cmd: Command) {
        match cmd {
            Command::Play {
                track,
                sample,
                start,
                end,
                playback,
            } => {
                self.status.set_hit(track, start);
                self.trigger(track, Some(Voice::new(sample, start, end, playback)));
                self.choke(track);
            }
            Command::SetPattern { track, pattern } => {
                let track = &mut self.tracks[track];
                let slot = if self.batch_open {
                    &mut track.staged
                } else {
                    &mut track.pending
                };
                if let Some(Some(old)) = slot.replace(pattern) {
                    self.discard_pattern(old);
                }
            }
            Command::SetBpm(bpm) => {
                if self.batch_open {
                    self.staged_bpm = Some(bpm);
                } else {
                    self.transport.pending_bpm = Some(bpm);
                }
            }
            Command::SetBuses(buses) => {
                if self.batch_open {
                    self.staged_buses = Some(buses);
                } else {
                    self.pending_buses = Some(buses);
                }
            }
            Command::BeginBatch => self.batch_open = true,
            Command::CommitBatch => {
                self.batch_open = false;
                if let Some(bpm) = self.staged_bpm.take() {
                    self.transport.pending_bpm = Some(bpm);
                }
                if let Some(buses) = self.staged_buses.take() {
                    self.pending_buses = Some(buses);
                }
                for t in 0..MAX_TRACKS {
                    if let Some(change) = self.tracks[t].staged.take()
                        && let Some(Some(old)) = self.tracks[t].pending.replace(change)
                    {
                        self.discard_pattern(old);
                    }
                }
            }
            Command::Start => {
                self.transport.start();
                self.status.clear_step();
                for track in &mut self.tracks {
                    track.retrig = None;
                }
            }
            Command::Stop => {
                self.transport.running = false;
                self.metronome.stop();
                for t in 0..MAX_TRACKS {
                    self.tracks[t].retrig = None;
                    self.trigger(t, None);
                }
            }
            Command::SetMute { track, on } => self.tracks[track].live_mute = on,
            Command::SetSolo { track, on } => self.tracks[track].live_solo = on,
            Command::Preview { sample, start, end } => {
                let voice = Voice::new(sample, start, end, Playback::NORMAL);
                if let Some(old) = self.preview.replace(voice) {
                    self.fade_preview(old);
                }
            }
            Command::StopPreview => {
                if let Some(old) = self.preview.take() {
                    self.fade_preview(old);
                }
            }
            Command::SetMetronome(on) => {
                self.metronome.enabled = on;
                if !on {
                    self.metronome.stop();
                }
            }
        }
    }

    /// Dispara el paso actual del secuenciador en todas las pistas y avanza.
    /// Al empezar un compás, antes aplica los patrones pendientes.
    fn step(&mut self) {
        let now = self.transport.frame;
        let (step, bar) = self.transport.advance(self.sample_rate);
        self.status.set_step(bar, step);
        self.metronome.on_step(step, self.sample_rate);
        let next = self.transport.next_step_frame(self.sample_rate);
        // Semicorchea de la canción: cada pista la lee con su longitud.
        let global = bar * STEPS as u64 + step as u64;
        if step == 0
            && let Some(buses) = self.pending_buses.take()
        {
            self.rack.params = buses;
        }
        for t in 0..MAX_TRACKS {
            if step == 0
                && let Some(change) = self.tracks[t].pending.take()
            {
                let track = &mut self.tracks[t];
                let old = std::mem::replace(&mut track.pattern, change);
                // Un efecto de otro tipo en la ranura: su parámetro suavizado
                // ya no vale.
                for (i, mods) in track.mods.iter_mut().enumerate() {
                    let kind = |p: &Option<Pattern>| {
                        p.as_ref()
                            .and_then(|p| p.fx[i])
                            .map(|fx| std::mem::discriminant(&fx))
                    };
                    if kind(&old) != kind(&track.pattern) {
                        *mods = [None; 2];
                    }
                }
                // Los disparos pendientes eran del patrón anterior (quizá de
                // otro sample): no se mezclan con el nuevo.
                track.retrig = None;
                // Si la pista está callada, el volumen nuevo entra ya, sin
                // suavizado: el primer golpe suena con él desde el principio.
                if !track.voice.as_ref().is_some_and(Voice::is_active) && track.fading.is_none() {
                    track.gain = track.pattern.as_ref().map_or(1.0, |p| p.gain);
                    track.sends = track.pattern.as_ref().map_or([0.0; BUSES], |p| p.sends);
                }
                for (i, state) in track.fx.iter_mut().enumerate() {
                    let fx = track.pattern.as_ref().and_then(|p| p.fx[i]);
                    *state = state.update(fx, self.sample_rate);
                }
                if let Some(old) = old {
                    self.discard_pattern(old);
                }
            }
            let hit = self.tracks[t].pattern.as_ref().and_then(|p| p.hit(global));
            let Some(hit) = hit else {
                // Un paso vacío no corta los subdisparos pendientes de un
                // golpe retrasado del paso anterior.
                continue;
            };
            let length = next - now;
            let delay = (hit.delay as f64 * length as f64).round() as u64;
            if delay == 0 {
                // Los subdisparos de un retrigger caben dentro de su paso.
                self.tracks[t].retrig = (hit.retrig > 1).then_some(Retrig {
                    hit,
                    count: hit.retrig,
                    done: 1,
                    start: now,
                    length,
                });
                self.play_hit(t, hit, (length / hit.retrig as u64) as usize);
            } else {
                // Retrasado: se dispara (con sus subdisparos) al llegar a su frame.
                self.tracks[t].retrig = Some(Retrig {
                    hit,
                    count: hit.retrig,
                    done: 0,
                    start: now + delay,
                    length,
                });
            }
        }
    }

    /// Siguiente subdisparo del retrigger de la pista `t`.
    fn fire_retrig(&mut self, t: usize) {
        let Some(mut retrig) = self.tracks[t].retrig else {
            return;
        };
        retrig.done += 1;
        self.tracks[t].retrig = (retrig.done < retrig.count).then_some(retrig);
        let gate = retrig.length / retrig.count as u64;
        self.play_hit(t, retrig.hit, gate as usize);
    }

    /// Dispara `hit` con el sample del patrón actual de la pista. En el
    /// modo gate, la voz empieza su cola tras `gate` frames (lo que dura su
    /// paso o su parte del retrigger). Corta las pistas de su grupo de
    /// choke.
    fn play_hit(&mut self, t: usize, hit: Trigger, gate: usize) {
        let Some(pattern) = &self.tracks[t].pattern else {
            return;
        };
        let mut voice = Voice::new(pattern.sample.clone(), hit.start, hit.end, hit.playback);
        if pattern.gate {
            voice = voice.with_gate(gate);
        }
        self.status.set_hit(t, hit.start);
        self.tracks[t].lock = hit.lock;
        self.trigger(t, Some(voice));
        self.choke(t);
    }

    /// Corta (con el fundido mínimo) las voces de las demás pistas del
    /// grupo de choke de la pista `t`, como el charles abierto que calla
    /// al cerrado.
    fn choke(&mut self, t: usize) {
        let group = |track: &Track| track.pattern.as_ref().and_then(|p| p.choke);
        let Some(g) = group(&self.tracks[t]) else {
            return;
        };
        for u in 0..MAX_TRACKS {
            if u != t && group(&self.tracks[u]) == Some(g) && self.tracks[u].voice.is_some() {
                self.trigger(u, None);
            }
        }
    }

    /// Sustituye la voz principal de una pista; la anterior hace su fundido
    /// de salida.
    fn trigger(&mut self, track: usize, voice: Option<Voice>) {
        let t = &mut self.tracks[track];
        if voice.is_some() && !t.voice.as_ref().is_some_and(Voice::is_active) && t.fading.is_none()
        {
            t.fresh = true;
        }
        if let Some(old) = std::mem::replace(&mut self.tracks[track].voice, voice) {
            self.fade_out(track, old);
        }
    }

    /// Mezcla `n` frames en `self.master`: cada pista en su buffer, con su
    /// volumen, y al final el limitador.
    fn mix(&mut self, n: usize) {
        self.master[..2 * n].fill(0.0);
        let smoothing = 1.0 - (-1.0 / (GAIN_SMOOTHING_SECONDS * self.sample_rate as f32)).exp();
        // Con alguna pista en solo, solo suenan las que lo están.
        let any_solo = self.tracks.iter().any(Track::soloed);
        for buf in &mut self.send_buf {
            buf[..2 * n].fill(0.0);
        }
        let mut fed = [false; BUSES];
        // Posición de la canción al empezar el trozo (para las curvas).
        let rate = self.sample_rate;
        let clock = Clock {
            steps: self.transport.position_steps(rate),
            seconds: self.transport.frame as f64 / rate as f64,
            frames_per_step: crate::sequencer::frames_per_step(self.transport.bpm(), rate),
            sample_rate: rate,
        };
        for t in 0..MAX_TRACKS {
            let track = &mut self.tracks[t];
            let target = track.pattern.as_ref().map_or(1.0, |p| p.gain);
            let target_sends = track.pattern.as_ref().map_or([0.0; BUSES], |p| p.sends);
            let audible = if track.muted() || (any_solo && !track.soloed()) {
                0.0
            } else {
                1.0
            };
            if track.voice.is_none() && track.fading.is_none() {
                track.audible = audible;
                // En silencio no hay nada que suavizar: el siguiente golpe
                // empieza ya con el volumen nuevo y los efectos sin memoria.
                track.gain = target;
                track.sends = target_sends;
                for state in &mut track.fx {
                    state.reset();
                }
                // Los parámetros que se movían vuelven a su valor fijo.
                if track.mods.iter().flatten().any(Option::is_some)
                    && let Some(pattern) = &track.pattern
                {
                    for (i, state) in track.fx.iter_mut().enumerate() {
                        for (p, m) in track.mods[i].iter_mut().enumerate() {
                            if m.take().is_some()
                                && let Some(base) = pattern.fx[i].and_then(|fx| fx.param_value(p))
                            {
                                state.set_param(p, base, rate);
                            }
                        }
                    }
                }
                track.mods = [[None; 2]; MAX_FX];
                continue;
            }
            let fresh = std::mem::take(&mut track.fresh);
            if fresh {
                track.audible = audible;
            }
            let buf = &mut self.track_buf[..2 * n];
            buf.fill(0.0);
            if let Some(voice) = &mut track.voice {
                voice.mix_into(buf);
            }
            if let Some(fading) = &mut track.fading {
                fading.mix_into(buf);
            }
            let moving = track.lock.is_some()
                || track.mods.iter().flatten().any(Option::is_some)
                || track
                    .pattern
                    .as_ref()
                    .is_some_and(|p| p.automation.iter().any(Option::is_some));
            if moving {
                modulate(track, buf, &clock, fresh);
            } else {
                for state in &mut track.fx {
                    state.process(buf);
                }
            }

            let master = &mut self.master[..2 * n];
            let mut peak = 0.0f32;
            let sending: [bool; BUSES] =
                std::array::from_fn(|b| target_sends[b] > 0.0 || track.sends[b] > 0.0);
            for (k, (dst, src)) in master
                .as_chunks_mut::<2>()
                .0
                .iter_mut()
                .zip(buf.as_chunks::<2>().0)
                .enumerate()
            {
                track.gain += (target - track.gain) * smoothing;
                // Mute y solo también se suavizan: las voces siguen su curso
                // (al quitar el mute suenan por donde iban) pero no se oyen.
                track.audible += (audible - track.audible) * smoothing;
                let gain = track.gain * track.audible;
                let (l, r) = (gain * src[0], gain * src[1]);
                dst[0] += l;
                dst[1] += r;
                peak = peak.max(l.abs()).max(r.abs());
                // Envíos después del volumen, el mute y el solo.
                for b in 0..BUSES {
                    if !sending[b] {
                        continue;
                    }
                    track.sends[b] += (target_sends[b] - track.sends[b]) * smoothing;
                    let send = track.sends[b];
                    self.send_buf[b][2 * k] += send * l;
                    self.send_buf[b][2 * k + 1] += send * r;
                    fed[b] |= send > 0.0 && (l != 0.0 || r != 0.0);
                }
            }
            self.status.add_peak(t, peak);

            if track.fading.as_ref().is_some_and(|v| !v.is_active())
                && let Some(done) = track.fading.take()
            {
                self.discard(done.sample);
            }
        }
        // Lo que devuelven los buses (los dormidos no tocan nada).
        self.rack.mix_into(
            &self.send_buf,
            fed,
            n,
            self.transport.bpm(),
            &mut self.master[..2 * n],
        );
        // La preescucha, directa al master.
        let master = &mut self.master[..2 * n];
        if let Some(voice) = &mut self.preview {
            voice.mix_into(master);
        }
        if let Some(voice) = &mut self.preview_fading {
            voice.mix_into(master);
        }
        if self.preview_fading.as_ref().is_some_and(|v| !v.is_active())
            && let Some(done) = self.preview_fading.take()
        {
            self.discard(done.sample);
        }
        self.metronome.mix_into(&mut self.master[..2 * n]);
        // Medidor del master antes del limitador, y cuándo actúa.
        let mut peak = 0.0f32;
        let mut limited = 0u64;
        for frame in self.master[..2 * n].as_chunks_mut::<2>().0 {
            let level = frame[0].abs().max(frame[1].abs());
            peak = peak.max(level);
            if level > LIMIT_THRESHOLD {
                limited += 1;
            }
            frame[0] = soft_limit(frame[0]);
            frame[1] = soft_limit(frame[1]);
        }
        self.status.add_master(peak, limited);
    }

    /// Pasa una voz interrumpida a la ranura de fundido de su pista. Si ya
    /// había otra fundiéndose, esa se descarta (le quedaba como mucho un
    /// fundido).
    fn fade_out(&mut self, track: usize, mut voice: Voice) {
        if !voice.is_active() {
            self.discard(voice.sample);
            return;
        }
        voice.release();
        if let Some(previous) = self.tracks[track].fading.replace(voice) {
            self.discard(previous.sample);
        }
    }

    /// La voz de preescucha sustituida hace su fundido de salida.
    fn fade_preview(&mut self, mut voice: Voice) {
        if !voice.is_active() {
            self.discard(voice.sample);
            return;
        }
        voice.release();
        if let Some(previous) = self.preview_fading.replace(voice) {
            self.discard(previous.sample);
        }
    }

    /// Suelta un `Arc<Sample>` sin liberar memoria en el hilo de audio. Si el
    /// motor guarda otra referencia al mismo sample, soltarlo solo baja el
    /// contador; si no, podría ser la última y se devuelve al hilo de control.
    fn discard(&mut self, sample: Arc<Sample>) {
        if self.holds(&sample) {
            drop(sample);
            return;
        }
        if let Err(rtrb::PushError::Full(garbage)) = self.garbage.push(Garbage::Sample(sample)) {
            // No debería ocurrir; antes filtrar memoria que liberarla en el callback.
            std::mem::forget(garbage);
        }
    }

    /// Devuelve un patrón entero al hilo de control: soltarlo aquí liberaría
    /// sus compases (memoria) en el callback.
    fn discard_pattern(&mut self, pattern: Pattern) {
        if let Err(rtrb::PushError::Full(garbage)) = self.garbage.push(Garbage::Pattern(pattern)) {
            std::mem::forget(garbage);
        }
    }

    fn holds(&self, sample: &Arc<Sample>) -> bool {
        let same = |other: &Arc<Sample>| Arc::ptr_eq(sample, other);
        let same_pattern = |p: &Option<Pattern>| p.as_ref().is_some_and(|p| same(&p.sample));
        self.tracks.iter().any(|t| {
            t.voice.as_ref().is_some_and(|v| same(&v.sample))
                || t.fading.as_ref().is_some_and(|v| same(&v.sample))
                || same_pattern(&t.pattern)
                || t.pending.as_ref().is_some_and(same_pattern)
                || t.staged.as_ref().is_some_and(same_pattern)
        }) || self.preview.as_ref().is_some_and(|v| same(&v.sample))
            || self
                .preview_fading
                .as_ref()
                .is_some_and(|v| same(&v.sample))
    }
}

/// Dónde está la canción al empezar un trozo de mezcla.
struct Clock {
    steps: f64,
    seconds: f64,
    frames_per_step: f64,
    sample_rate: u32,
}

/// Pasa `buf` por los efectos de la pista con sus parámetros movidos:
/// cada `MOD_BLOCK` frames calcula el destino de cada parámetro (la curva
/// de su automatización o, el corte del primer filtro, el del último
/// golpe con `~`), lo suaviza (~5 ms) y se lo pasa al efecto. Con `snap`
/// (la voz acaba de empezar desde el silencio), el primer valor entra ya.
/// Un parámetro que vuelve a su valor fijo deja de moverse.
fn modulate(track: &mut Track, buf: &mut [f32], clock: &Clock, snap: bool) {
    let Some(pattern) = &track.pattern else {
        for state in &mut track.fx {
            state.process(buf);
        }
        return;
    };
    let rate = clock.sample_rate;
    let keep = (-(MOD_BLOCK as f32) / (GAIN_SMOOTHING_SECONDS * rate as f32)).exp();
    let first_filter = pattern
        .fx
        .iter()
        .position(|f| matches!(f, Some(Fx::Filter { .. })));
    for (block, sub) in buf.chunks_mut(2 * MOD_BLOCK).enumerate() {
        let frames = (block * MOD_BLOCK) as f64;
        let steps = clock.steps + frames / clock.frames_per_step;
        let seconds = clock.seconds + frames / rate as f64;
        for (slot, state) in track.fx.iter_mut().enumerate() {
            let Some(fx) = pattern.fx[slot] else { continue };
            for p in 0..fx.params().len() {
                let (base, (_, _, log)) = (
                    fx.param_value(p).unwrap_or(0.0),
                    fx.param_range(p).unwrap_or((0.0, 1.0, false)),
                );
                let curve = pattern
                    .automation
                    .iter()
                    .flatten()
                    .find(|a| a.fx == slot && a.param == p);
                let lock = track.lock.filter(|_| p == 0 && Some(slot) == first_filter);
                let target = match (lock, curve) {
                    (Some(hz), _) => Some(hz),
                    (None, Some(a)) => Some(a.curve.value(steps, seconds, log)),
                    (None, None) => None,
                };
                let current = &mut track.mods[slot][p];
                if target.is_none() && current.is_none() {
                    continue;
                }
                let goal = target.unwrap_or(base);
                let from = current.unwrap_or(if snap { goal } else { base });
                let next = if snap && block == 0 {
                    goal
                } else {
                    mix(goal, from, keep, log)
                };
                // Sin nada que lo mueva y ya en su sitio: vuelve al fijo.
                if target.is_none() && (next - base).abs() <= 1e-4 * base.abs().max(1.0) {
                    *current = None;
                    state.set_param(p, base, rate);
                } else {
                    *current = Some(next);
                    state.set_param(p, next, rate);
                }
            }
        }
        for state in &mut track.fx {
            state.process(sub);
        }
    }
}

/// Copia la mezcla estéreo a la salida con `channels` canales: en mono se
/// promedia; con más de dos, los demás canales quedan en silencio.
fn write_out(master: &[f32], out: &mut [f32], channels: usize) {
    for (src, dst) in master
        .as_chunks::<2>()
        .0
        .iter()
        .zip(out.chunks_exact_mut(channels))
    {
        match dst {
            [mono] => *mono = 0.5 * (src[0] + src[1]),
            [l, r, rest @ ..] => {
                *l = src[0];
                *r = src[1];
                rest.fill(0.0);
            }
            [] => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::notation::parse_steps;
    use crate::offline;

    /// Un lote leído a medias por el motor no se aplica aunque pase un
    /// cambio de compás: solo entra entero, tras `CommitBatch`.
    #[test]
    fn half_read_batch_is_not_applied_until_commit() {
        const RATE: u32 = 48_000;
        let bar = crate::bar_frames(120.0, RATE) as usize;
        let sample = Arc::new(Sample::from_interleaved(&vec![0.5; 4000], 1, RATE, RATE).unwrap());
        let slices = Slices::detect(&sample, 1);
        let every_step = parse_steps("1 1 1 1  1 1 1 1  1 1 1 1  1 1 1 1").unwrap();
        let pattern = || Pattern::from_steps(sample.clone(), &slices, every_step).unwrap();

        let (mut controller, mut engine) = new_engine(RATE);
        controller.start().unwrap();
        // Primera mitad del lote: el motor la lee y renderiza dos compases.
        controller.commands.push(Command::BeginBatch).ok().unwrap();
        let set = |track| Command::SetPattern {
            track,
            pattern: Some(pattern()),
        };
        controller.commands.push(set(0)).ok().unwrap();
        let silent = offline::render(&mut engine, 2 * bar, 2);
        assert!(silent.iter().all(|&s| s == 0.0), "se aplicó medio lote");

        // Segunda mitad y cierre: entra en el siguiente compás, entero.
        controller.commands.push(set(1)).ok().unwrap();
        controller.commands.push(Command::CommitBatch).ok().unwrap();
        let audio = offline::render(&mut engine, bar, 2);
        // Las dos pistas suenan a la vez: 0.5 + 0.5, por el limitador.
        assert_eq!(audio[2 * 200], soft_limit(1.0));
    }
}
