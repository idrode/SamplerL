use std::fmt;
use std::ops::Range;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use rtrb::{Consumer, Producer, RingBuffer};

use crate::arrangement::{Arrangement, Place, Playlist, resume};
use crate::automation::mix;
use crate::bus::{BUSES, Buses, Rack};
use crate::fx::{Fx, FxState, MAX_FX};
use crate::grain::{Cloud, Position};
use crate::instrument::{MAX_VOICES, Note, NoteOn};
use crate::limiter::{LIMIT_THRESHOLD, soft_limit};
use crate::metronome::Metronome;
use crate::sample::Sample;
use crate::sequencer::{Meter, Pattern, TempoRamp, Transport, Trigger, valid_bpm};
use crate::shift::Shift;
use crate::slice::Slices;
use crate::status::{SharedStatus, Status};

const COMMAND_CAPACITY: usize = 64;
// A la basura solo va un `Arc` cuando el motor no guarda otra referencia al
// mismo sample (ver `Engine::discard`), así que las voces que dispara el
// secuenciador no la llenan: cada elemento sale de un comando y el
// controlador la recoge antes de cada envío.
const GARBAGE_CAPACITY: usize = COMMAND_CAPACITY * 2;
/// Notas MIDI que caben en la cola de salida sin que nadie las lea (si se
/// llena, las siguientes se pierden: nunca se bloquea ni se reserva).
const MIDI_CAPACITY: usize = 512;

/// Una nota MIDI que sale de una pista (`Pattern::with_midi`), con el frame
/// exacto del motor en que toca. Velocidad 0: note off.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MidiNote {
    /// Frame desde que arrancó el transporte.
    pub frame: u64,
    /// Canal, de 1 a 16.
    pub channel: u8,
    pub note: u8,
    pub velocity: u8,
}

impl MidiNote {
    /// Los tres bytes del mensaje (note on, o note off con velocidad 0).
    pub fn bytes(&self) -> [u8; 3] {
        let channel = (self.channel.clamp(1, 16) - 1) & 0x0f;
        if self.velocity == 0 {
            [0x80 | channel, self.note, 0]
        } else {
            [0x90 | channel, self.note, self.velocity]
        }
    }
}

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
    /// Arreglo (secciones y línea de tiempo) para el siguiente compás;
    /// `None` lo quita (vuelven los patrones sueltos).
    SetArrangement(Option<Arrangement>),
    /// Saltar a esa entrada de la línea de tiempo al empezar el compás
    /// siguiente.
    JumpTo(usize),
    /// Repetir la entrada que suena (o volver a seguir la línea de tiempo).
    LoopSection(bool),
    /// Patrón de una pista para el siguiente compás (`None` la silencia).
    SetPattern {
        track: usize,
        pattern: Option<Pattern>,
    },
    /// BPM para el siguiente compás.
    SetBpm(f64),
    /// Parámetros de los buses de envío para el siguiente compás.
    SetBuses(Buses),
    /// Rampa de tempo que entra en el siguiente compás.
    SetTempoRamp(TempoRamp),
    /// Compás (pasos y pulso) desde el siguiente compás.
    SetMeter(Meter),
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
    /// Nota en vivo de un instrumento (pad o MIDI de entrada): suena hasta
    /// su `NoteOff`. Lleva el sample y el instrumento, como `Play`, así que
    /// suena también con el secuenciador parado.
    NoteOn {
        track: usize,
        sample: Arc<Sample>,
        instrument: crate::instrument::Instrument,
        start: usize,
        end: usize,
        rate: f64,
        gain: f32,
        key: u8,
    },
    /// Suelta la nota en vivo de esa tecla: empieza su release.
    NoteOff { track: usize, key: u8 },
}

/// Cómo se reproduce un rango: ganancia, velocidad de lectura (pitch),
/// sentido y la forma del corte (envolvente; ver [`SliceShape`]).
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Playback {
    pub gain: f32,
    /// Frames del sample por frame de salida: 2.0 es una octava arriba (y
    /// dura la mitad), 0.5 una octava abajo (y dura el doble).
    pub rate: f64,
    /// El pitch no cambia la duración (ver [`crate::shift`]).
    pub keep: bool,
    /// Formantes con `keep` (ver [`crate::shift`]).
    pub formant: Option<f32>,
    pub reverse: bool,
    pub shape: SliceShape,
}

impl Playback {
    pub const NORMAL: Playback = Playback {
        gain: 1.0,
        rate: 1.0,
        keep: false,
        formant: None,
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
    /// Pitch que conserva la duración: los granos (con pitch 0, ninguno).
    shift: Option<Shift>,
}

impl Voice {
    fn new(sample: Arc<Sample>, start: usize, end: usize, playback: Playback) -> Self {
        let rate = sample.sample_rate() as f32;
        let fade = ((FADE_SECONDS * rate) as usize).max(1);
        let ms = |ms: f32| (ms / 1000.0 * rate) as usize;
        let shift = (playback.keep && Shift::needed(playback.rate, playback.formant))
            .then(|| Shift::new(playback.rate, sample.sample_rate(), playback.formant));
        // Con los granos, la voz avanza por el sample a velocidad 1.
        let speed = if shift.is_some() { 1.0 } else { playback.rate };
        // Último frame de salida: el que lee el último frame del rango. En
        // bucle, la voz no acaba sola.
        let length = match (end - start, playback.shape.play) {
            (0, _) => 0,
            (_, PlayMode::Loop | PlayMode::PingPong) => usize::MAX,
            (frames, PlayMode::Once) => ((frames - 1) as f64 / speed).floor() as usize + 1,
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
            shift,
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

    /// Frame del sample para el frame de salida actual, interpolado si la
    /// posición cae entre dos frames o el pitch sube (ver [`crate::interp`]).
    /// Con el pitch que conserva la duración, la suma de sus dos granos
    /// (con la corrección de formantes, si la hay).
    fn read(&mut self) -> (f32, f32) {
        let (start, end, playback) = (self.start, self.end, self.playback);
        let sample = &self.sample;
        let read_at = |offset: f64| {
            let pos = voice_position(start, end, &playback, offset);
            crate::interp::read(sample, pos, start, end, playback.rate)
        };
        let Some(shift) = self.shift.as_mut() else {
            return read_at(self.rendered as f64 * playback.rate);
        };
        let span = (end - start) as f64;
        // Sin bucle, ningún grano pasa del final del corte.
        let fit = (playback.shape.play == PlayMode::Once).then(|| span - 1.0 - shift.reach());
        let grains = shift.grains(self.rendered as u64, fit, |offset| {
            crate::shift::mono_at(
                sample,
                voice_position(start, end, &playback, offset),
                start,
                end,
            )
        });
        let (mut l, mut r) = (0.0, 0.0);
        for (offset, weight) in grains {
            if weight == 0.0 || (fit.is_some() && offset > span - 1.0) {
                continue;
            }
            let (gl, gr) = read_at(offset);
            l += weight * gl;
            r += weight * gr;
        }
        shift.correct((l, r))
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

/// Posición en el sample del desplazamiento `offset` desde el inicio de
/// una voz que toca `start..end` así (con su bucle y su sentido).
fn voice_position(start: usize, end: usize, playback: &Playback, mut offset: f64) -> f64 {
    let span = (end - start) as f64;
    match playback.shape.play {
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
    if playback.reverse {
        (end - 1) as f64 - offset
    } else {
        start as f64 + offset
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
    /// Instrumento con parámetros fuera de rango.
    InvalidInstrument(&'static str),
    /// Granos con parámetros fuera de rango.
    InvalidGranular(&'static str),
    /// Arreglo (secciones y línea de tiempo) no válido.
    InvalidArrangement(&'static str),
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
            SendError::InvalidInstrument(reason) => write!(f, "instrumento: {reason}"),
            SendError::InvalidGranular(reason) => write!(f, "granos: {reason}"),
            SendError::InvalidArrangement(reason) => write!(f, "arreglo: {reason}"),
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
    ramp: Option<TempoRamp>,
    meter: Option<Meter>,
    buses: Option<Buses>,
    patterns: Vec<(usize, Option<Pattern>)>,
    arrangement: Option<Option<Arrangement>>,
}

impl Batch {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn set_bpm(&mut self, bpm: f64) -> &mut Self {
        self.bpm = Some(bpm);
        self
    }

    /// Rampa de tempo (en vez de un tempo fijo).
    pub fn set_tempo_ramp(&mut self, ramp: TempoRamp) -> &mut Self {
        self.ramp = Some(ramp);
        self
    }

    /// Compás (3/4, 7/8...).
    pub fn set_meter(&mut self, meter: Meter) -> &mut Self {
        self.meter = Some(meter);
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

    /// Arreglo: secciones y línea de tiempo (ver [`crate::arrangement`]);
    /// con él, los patrones de las pistas son los de sus secciones. `None`
    /// quita el que haya (y mandan los de `set_pattern`).
    pub fn set_arrangement(&mut self, arrangement: Option<Arrangement>) -> &mut Self {
        self.arrangement = Some(arrangement);
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
    /// Un arreglo entero (sus secciones y sus patrones).
    Arrangement(Arrangement),
}

/// Lado de control (UI/script). Envía comandos al hilo de audio sin bloquear.
pub struct Controller {
    commands: Producer<Command>,
    garbage: Consumer<Garbage>,
    midi: Consumer<MidiNote>,
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
    /// Nota MIDI que suena (canal, nota) y el frame de su note off.
    midi_off: Option<(u8, u8, u64)>,
    /// Corte del primer filtro puesto por el último golpe (`~`).
    lock: Option<f32>,
    /// Valor (suavizado) de cada parámetro automatizable de cada efecto
    /// mientras se mueve; `None`: el fijo del efecto.
    mods: [[Option<f32>; 2]; MAX_FX],
    /// Instrumento: las notas que suenan (una por voz) y las robadas o
    /// cortadas que hacen su fundido.
    notes: [Option<Note>; MAX_VOICES],
    tails: [Option<Note>; MAX_VOICES],
    /// Notas abiertas en la pista (orden de llegada, para robar).
    born: u64,
    /// Notas MIDI de un instrumento que siguen encendidas: su note off
    /// sale al acabar su paso, o antes si se roba o se corta la nota.
    midi_notes: [Option<MidiHeld>; MIDI_HELD],
    /// Granos: la nube del último golpe y los granos que suenan.
    cloud: Cloud,
}

/// Notas MIDI encendidas a la vez por pista de instrumento (si se llenan,
/// se apaga la más antigua).
const MIDI_HELD: usize = 2 * MAX_VOICES;

/// Una nota MIDI de un instrumento encendida.
#[derive(Clone, Copy)]
struct MidiHeld {
    channel: u8,
    note: u8,
    /// Frame del note off (el final de su paso y sus `_`).
    at: u64,
    /// La nota del motor que la toca (`Note::born`).
    born: u64,
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
    /// Lo que dura el subdisparo `done` (desde 1): su parte del paso; el
    /// último, además, los pasos que se alarga el golpe (`_`).
    fn gate(&self) -> usize {
        let part = self.length / self.count as u64;
        let hold = if self.done >= self.count {
            self.hit.hold as u64 * self.length
        } else {
            0
        };
        (part + hold) as usize
    }

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
            midi_off: None,
            lock: None,
            mods: [[None; 2]; MAX_FX],
            notes: std::array::from_fn(|_| None),
            tails: std::array::from_fn(|_| None),
            born: 0,
            midi_notes: [None; MIDI_HELD],
            cloud: Cloud::default(),
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

    /// Alguna nota del instrumento suena (o hace su fundido).
    fn has_notes(&self) -> bool {
        self.notes.iter().chain(&self.tails).any(Option::is_some)
    }

    /// Algo suena en la pista: su voz, su fundido, notas o granos.
    fn sounding(&self) -> bool {
        self.voice.as_ref().is_some_and(Voice::is_active)
            || self.fading.is_some()
            || self.has_notes()
            || self.cloud.is_active()
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
    /// En el heap (se crean en `new_engine`): con los filtros de los
    /// formantes de cada voz ocupan cientos de KB, demasiado para la pila.
    tracks: Box<[Track; MAX_TRACKS]>,
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
    /// Cola de las notas MIDI que salen de las pistas.
    midi: Producer<MidiNote>,
    pending_buses: Option<Buses>,
    /// Arreglo que suena (con su posición), el que entra en el siguiente
    /// compás y el de un lote todavía abierto (`Some(None)`: quitarlo).
    playlist: Option<Playlist>,
    pending_arrangement: Option<Option<Arrangement>>,
    staged_arrangement: Option<Option<Arrangement>>,
    staged_buses: Option<Buses>,
    staged_ramp: Option<TempoRamp>,
    staged_meter: Option<Meter>,
    send_buf: [[f32; 2 * CHUNK]; BUSES],
}

/// Crea el par controlador/motor. `sample_rate` es la frecuencia a la que
/// renderiza el motor (la de los samples cargados para él).
pub fn new_engine(sample_rate: u32) -> (Controller, Engine) {
    // La tabla de la interpolación se calcula aquí, no en el hilo de audio.
    crate::interp::prepare();
    let (cmd_tx, cmd_rx) = RingBuffer::new(COMMAND_CAPACITY);
    let (garbage_tx, garbage_rx) = RingBuffer::new(GARBAGE_CAPACITY);
    let (midi_tx, midi_rx) = RingBuffer::new(MIDI_CAPACITY);
    let playing = Arc::new(AtomicBool::new(false));
    let status = Arc::new(SharedStatus::new(crate::sequencer::DEFAULT_BPM));
    (
        Controller {
            commands: cmd_tx,
            garbage: garbage_rx,
            midi: midi_rx,
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
            tracks: (0..MAX_TRACKS)
                .map(|_| Track::default())
                .collect::<Vec<_>>()
                .into_boxed_slice()
                .try_into()
                .unwrap_or_else(|_| unreachable!("son MAX_TRACKS pistas")),
            track_buf: [0.0; 2 * CHUNK],
            master: [0.0; 2 * CHUNK],
            batch_open: false,
            staged_bpm: None,
            metronome: Metronome::default(),
            preview: None,
            preview_fading: None,
            rack: Rack::new(sample_rate),
            midi: midi_tx,
            pending_buses: None,
            playlist: None,
            pending_arrangement: None,
            staged_arrangement: None,
            staged_buses: None,
            staged_ramp: None,
            staged_meter: None,
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
        self.play_slice_with_gain(track, sample, slices, index, shape, 1.0)
    }

    /// Como [`Controller::play_slice_shaped`], con un volumen para el golpe
    /// (de 0 a `MAX_GAIN`; la velocidad de una nota MIDI).
    pub fn play_slice_with_gain(
        &mut self,
        track: usize,
        sample: &Arc<Sample>,
        slices: &Slices,
        index: usize,
        shape: SliceShape,
        gain: f32,
    ) -> Result<(), SendError> {
        if !(0.0..=crate::sequencer::MAX_GAIN).contains(&gain) {
            return Err(SendError::InvalidStep {
                step: index,
                reason: "ganancia fuera de rango (0 a 4)",
            });
        }
        let range = slices.get(index).ok_or(SendError::NoSuchSlice {
            index,
            count: slices.len(),
        })?;
        shape.validate().map_err(|reason| SendError::InvalidStep {
            step: index,
            reason,
        })?;
        self.play_range_shaped(track, sample.clone(), shape.range(range), shape, gain)
    }

    /// Toca ya una nota de un instrumento en la pista `track` (un pad o una
    /// nota MIDI de entrada): `semitones` sobre la raíz, con su volumen. Sin
    /// cortar las que suenan (como las del secuenciador); suena hasta
    /// [`Controller::note_off`] con la misma `key`, que dispara su release.
    #[allow(clippy::too_many_arguments)]
    pub fn note_on(
        &mut self,
        track: usize,
        sample: &Arc<Sample>,
        range: Range<usize>,
        instrument: crate::instrument::Instrument,
        semitones: f32,
        gain: f32,
        key: u8,
    ) -> Result<(), SendError> {
        if track >= MAX_TRACKS {
            return Err(SendError::NoSuchTrack(track));
        }
        let invalid = |reason| SendError::InvalidStep { step: 0, reason };
        if !(0.0..=crate::sequencer::MAX_GAIN).contains(&gain) {
            return Err(invalid("ganancia fuera de rango (0 a 4)"));
        }
        if !(-crate::sequencer::MAX_SEMITONES..=crate::sequencer::MAX_SEMITONES)
            .contains(&semitones)
        {
            return Err(invalid("nota fuera de rango (-36 a 36 semitonos)"));
        }
        instrument
            .validate(sample.frames())
            .map_err(SendError::InvalidInstrument)?;
        let end = range.end.min(sample.frames());
        let start = range.start.min(end);
        let cmd = Command::NoteOn {
            track,
            sample: sample.clone(),
            instrument,
            start,
            end,
            rate: 2f64.powf(semitones as f64 / 12.0),
            gain,
            key,
        };
        self.send(cmd, Some(start < end))
    }

    /// Suelta la nota en vivo de la tecla `key` en la pista `track`: empieza
    /// su release. Una tecla sin nota no hace nada.
    pub fn note_off(&mut self, track: usize, key: u8) -> Result<(), SendError> {
        if track >= MAX_TRACKS {
            return Err(SendError::NoSuchTrack(track));
        }
        self.send(Command::NoteOff { track, key }, None)
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
        self.play_range_shaped(track, sample, range, SliceShape::DEFAULT, 1.0)
    }

    fn play_range_shaped(
        &mut self,
        track: usize,
        sample: Arc<Sample>,
        range: Range<usize>,
        shape: SliceShape,
        gain: f32,
    ) -> Result<(), SendError> {
        if track >= MAX_TRACKS {
            return Err(SendError::NoSuchTrack(track));
        }
        let end = range.end.min(sample.frames());
        let start = range.start.min(end);
        let playback = Playback {
            shape,
            gain,
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
        if let Some(Some(arrangement)) = &batch.arrangement {
            arrangement.validate()?;
        }
        if let Some(ramp) = &batch.ramp
            && !(valid_bpm(ramp.from) && valid_bpm(ramp.to))
        {
            return Err(SendError::InvalidBpm(ramp.from.max(ramp.to)));
        }
        self.collect_garbage();
        let needed = 2
            + batch.patterns.len()
            + usize::from(batch.bpm.is_some())
            + usize::from(batch.ramp.is_some())
            + usize::from(batch.meter.is_some())
            + usize::from(batch.buses.is_some())
            + usize::from(batch.arrangement.is_some());
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
        if let Some(ramp) = batch.ramp {
            push(Command::SetTempoRamp(ramp));
        }
        if let Some(meter) = batch.meter {
            push(Command::SetMeter(meter));
        }
        if let Some(buses) = batch.buses {
            push(Command::SetBuses(buses));
        }
        if let Some(arrangement) = batch.arrangement {
            push(Command::SetArrangement(arrangement));
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

    /// Con un arreglo: salta a la entrada `entry` de la línea de tiempo al
    /// empezar el compás siguiente (y sigue desde ahí).
    pub fn jump_to(&mut self, entry: usize) -> Result<(), SendError> {
        self.send(Command::JumpTo(entry), None)
    }

    /// Con un arreglo: repite la entrada que suena (`true`) o vuelve a
    /// seguir la línea de tiempo (`false`).
    pub fn loop_section(&mut self, on: bool) -> Result<(), SendError> {
        self.send(Command::LoopSection(on), None)
    }

    /// Rampa de tempo: entra al empezar el siguiente compás.
    pub fn set_tempo_ramp(&mut self, ramp: TempoRamp) -> Result<(), SendError> {
        if !(valid_bpm(ramp.from) && valid_bpm(ramp.to)) {
            return Err(SendError::InvalidBpm(ramp.from.max(ramp.to)));
        }
        self.send(Command::SetTempoRamp(ramp), None)
    }

    /// Compás: entra al empezar el siguiente compás.
    pub fn set_meter(&mut self, meter: Meter) -> Result<(), SendError> {
        self.send(Command::SetMeter(meter), None)
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

    /// Dónde va la canción, en pasos con decimales desde que arrancó (la
    /// semicorchea global y la fracción del paso), al acabar el último
    /// bloque de audio; `None` con el secuenciador parado. Para cuantizar
    /// lo que se toca en vivo (grabar). No vacía los picos de `status`.
    pub fn position(&self) -> Option<f64> {
        self.status.position()
    }

    /// Con un arreglo, dónde va la canción en la línea de tiempo (sin
    /// vaciar nada de `status`).
    pub fn section(&self) -> Option<crate::SectionStatus> {
        self.status.section()
    }

    /// Las notas MIDI que han salido de las pistas desde la última vez (en
    /// orden, con su frame), para mandarlas a un puerto.
    pub fn midi_events(&mut self) -> impl Iterator<Item = MidiNote> + '_ {
        std::iter::from_fn(|| self.midi.pop().ok())
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
            self.midi_due(self.transport.frame + n as u64);
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
            || self.tracks.iter().any(|t| {
                t.voice.as_ref().is_some_and(Voice::is_active)
                    || t.fading.is_some()
                    || t.has_notes()
            })
            || self.preview.as_ref().is_some_and(Voice::is_active)
            || self.preview_fading.is_some();
        self.playing.store(playing, Ordering::Relaxed);
        self.status
            .set_transport(self.transport.running, self.transport.bpm());
        self.status.set_position(
            self.transport
                .running
                .then(|| self.transport.position_steps(self.sample_rate)),
        );
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
            Command::SetTempoRamp(ramp) => {
                if self.batch_open {
                    self.staged_ramp = Some(ramp);
                } else {
                    self.transport.pending_ramp = Some(ramp);
                }
            }
            Command::SetMeter(meter) => {
                if self.batch_open {
                    self.staged_meter = Some(meter);
                } else {
                    self.transport.pending_meter = Some(meter);
                }
            }
            Command::SetBuses(buses) => {
                if self.batch_open {
                    self.staged_buses = Some(buses);
                } else {
                    self.pending_buses = Some(buses);
                }
            }
            Command::SetArrangement(arrangement) => {
                let slot = if self.batch_open {
                    &mut self.staged_arrangement
                } else {
                    &mut self.pending_arrangement
                };
                if let Some(Some(old)) = slot.replace(arrangement) {
                    self.discard_arrangement(old);
                }
            }
            Command::JumpTo(entry) => {
                if let Some(playlist) = &mut self.playlist
                    && entry < playlist.arrangement.len()
                {
                    playlist.jump = Some(entry);
                }
            }
            Command::LoopSection(on) => {
                if let Some(playlist) = &mut self.playlist {
                    playlist.looping = on;
                    let place = playlist.place;
                    self.status.set_section(Some((place.entry, place.bar, on)));
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
                if let Some(ramp) = self.staged_ramp.take() {
                    self.transport.pending_ramp = Some(ramp);
                }
                if let Some(meter) = self.staged_meter.take() {
                    self.transport.pending_meter = Some(meter);
                }
                if let Some(change) = self.staged_arrangement.take()
                    && let Some(Some(old)) = self.pending_arrangement.replace(change)
                {
                    self.discard_arrangement(old);
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
                if let Some(playlist) = &mut self.playlist {
                    playlist.restart = true;
                }
                self.status.clear_step();
                for track in self.tracks.iter_mut() {
                    track.retrig = None;
                }
            }
            Command::Stop => {
                self.transport.running = false;
                self.metronome.stop();
                let now = self.transport.frame;
                for t in 0..MAX_TRACKS {
                    self.tracks[t].retrig = None;
                    self.trigger(t, None);
                    self.kill_notes(t);
                    self.midi_note_off(t, now);
                }
            }
            Command::NoteOn {
                track,
                sample,
                instrument,
                start,
                end,
                rate,
                gain,
                key,
            } => {
                // La misma tecla otra vez: la nota anterior se suelta.
                self.note_off(track, key);
                if !self.tracks[track].sounding() {
                    self.tracks[track].fresh = true;
                }
                self.status.set_hit(track, start);
                let on = NoteOn {
                    start,
                    end,
                    rate,
                    keep: false,
                    formant: None,
                    reverse: false,
                    gain,
                    pan: 0.0,
                    gate: crate::instrument::HELD,
                    born: 0,
                    key: Some(key),
                };
                self.open_note(track, &sample, &instrument, on);
                self.choke(track);
                self.discard(sample);
            }
            Command::NoteOff { track, key } => self.note_off(track, key),
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
        let (step, bar, global) = self.transport.advance(self.sample_rate);
        let meter = self.transport.meter();
        self.status.set_step(bar, step, global, meter.steps);
        self.metronome.on_step(step, meter.beat, self.sample_rate);
        let next = self.transport.next_step_frame(self.sample_rate);
        if step == 0
            && let Some(buses) = self.pending_buses.take()
        {
            self.rack.params = buses;
        }
        if step == 0 {
            self.advance_playlist(global, now);
        }
        // Las vueltas de los patrones cuentan desde el inicio de la sección.
        let origin = self.playlist.as_ref().map_or(0, |p| p.origin_step);
        for t in 0..MAX_TRACKS {
            if step == 0
                && let Some(change) = self.tracks[t].pending.take()
            {
                let old = put_pattern(&mut self.tracks[t], change, self.sample_rate);
                if let Some(old) = old {
                    self.discard_pattern(old);
                }
            }
            let hit = self.tracks[t]
                .pattern
                .as_ref()
                .and_then(|p| p.hit(global - origin));
            let Some(hit) = hit else {
                // Un paso vacío no corta los subdisparos pendientes de un
                // golpe retrasado del paso anterior.
                continue;
            };
            let length = next - now;
            let delay = (hit.delay as f64 * length as f64).round() as u64;
            if delay == 0 {
                // Los subdisparos de un retrigger caben dentro de su paso.
                let retrig = Retrig {
                    hit,
                    count: hit.retrig,
                    done: 1,
                    start: now,
                    length,
                };
                self.tracks[t].retrig = (hit.retrig > 1).then_some(retrig);
                self.play_hit(t, hit, retrig.gate());
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
        self.play_hit(t, retrig.hit, retrig.gate());
    }

    /// Dispara `hit` con el sample del patrón actual de la pista. En el
    /// modo gate, la voz empieza su cola tras `gate` frames (lo que dura su
    /// paso o su parte del retrigger). Corta las pistas de su grupo de
    /// choke.
    fn play_hit(&mut self, t: usize, hit: Trigger, gate: usize) {
        let Some(pattern) = &self.tracks[t].pattern else {
            return;
        };
        if let Some(instrument) = pattern.instrument {
            let sample = pattern.sample.clone();
            let midi = pattern.midi;
            self.status.set_hit(t, hit.start);
            self.tracks[t].lock = hit.lock;
            self.play_notes(t, sample, hit, gate, instrument, midi);
            self.choke(t);
            return;
        }
        if pattern.granular.is_some() {
            let sample = pattern.sample.clone();
            let midi = pattern.midi;
            self.status.set_hit(t, hit.start);
            let track = &mut self.tracks[t];
            track.lock = hit.lock;
            if !track.sounding() {
                track.fresh = true;
            }
            let p = hit.playback;
            let old = track.cloud.open(
                sample,
                hit.start,
                hit.end,
                p.rate,
                p.reverse,
                p.gain,
                gate as u64,
            );
            if let Some(old) = old {
                self.discard(old.sample);
            }
            self.choke(t);
            if let Some((channel, base)) = midi {
                self.midi_note_on(t, channel, base, hit, gate);
            }
            return;
        }
        let playback = Playback {
            formant: pattern.formant,
            ..hit.playback
        };
        let mut voice = Voice::new(pattern.sample.clone(), hit.start, hit.end, playback);
        if pattern.gate {
            voice = voice.with_gate(gate);
        }
        let midi = pattern.midi;
        self.status.set_hit(t, hit.start);
        self.tracks[t].lock = hit.lock;
        self.trigger(t, Some(voice));
        self.choke(t);
        if let Some((channel, base)) = midi {
            self.midi_note_on(t, channel, base, hit, gate);
        }
    }

    /// Un golpe de una pista con instrumento: una nota por cada nota del
    /// acorde, cada una en una voz. Si no quedan voces, se roban las más
    /// antiguas (hacen el fundido mínimo aparte y dejan de contar).
    fn play_notes(
        &mut self,
        t: usize,
        sample: Arc<Sample>,
        hit: Trigger,
        gate: usize,
        instrument: crate::instrument::Instrument,
        midi: Option<(u8, u8)>,
    ) {
        if !self.tracks[t].sounding() {
            self.tracks[t].fresh = true;
        }
        let formant = self.tracks[t].pattern.as_ref().and_then(|p| p.formant);
        let rates = std::iter::once(hit.playback.rate).chain(hit.chord.iter().flatten().copied());
        for rate in rates {
            let on = NoteOn {
                start: hit.start,
                end: hit.end,
                rate,
                keep: hit.playback.keep,
                formant,
                reverse: hit.playback.reverse,
                gain: hit.playback.gain,
                pan: hit.playback.shape.pan,
                gate: gate.max(1) as u64,
                born: 0,
                key: None,
            };
            self.open_note(t, &sample, &instrument, on);
            if let Some((channel, root)) = midi {
                // La nota real: la raíz más los semitonos de esta nota.
                let semitones = (12.0 * rate.log2()).round() as i32;
                let born = self.tracks[t].born;
                let gain = hit.playback.gain;
                self.instrument_midi_on(t, channel, root as i32 + semitones, gain, gate, born);
            }
        }
        self.discard(sample);
    }

    /// Note on de una nota de un instrumento (en el frame del golpe) y su
    /// note off para dentro de `gate` frames. Si la pista no se oye, nada.
    fn instrument_midi_on(
        &mut self,
        t: usize,
        channel: u8,
        note: i32,
        gain: f32,
        gate: usize,
        born: u64,
    ) {
        let now = self.transport.frame;
        let any_solo = self.tracks.iter().any(Track::soloed);
        let track = &self.tracks[t];
        if track.muted() || (any_solo && !track.soloed()) {
            return;
        }
        let Ok(note) = u8::try_from(note) else {
            return;
        };
        if note > 127 {
            return;
        }
        // La misma nota aún encendida (o sin sitio): se apaga antes.
        let same = |m: &Option<MidiHeld>| m.is_some_and(|m| m.channel == channel && m.note == note);
        if let Some(i) = self.tracks[t].midi_notes.iter().position(same) {
            self.instrument_midi_off(t, i, now);
        }
        let track = &self.tracks[t];
        let slot = track
            .midi_notes
            .iter()
            .position(Option::is_none)
            .unwrap_or_else(|| {
                (0..MIDI_HELD)
                    .min_by_key(|&i| track.midi_notes[i].map_or(0, |m| m.at))
                    .expect("MIDI_HELD > 0")
            });
        if self.tracks[t].midi_notes[slot].is_some() {
            self.instrument_midi_off(t, slot, now);
        }
        let velocity = (gain * 100.0).round().clamp(1.0, 127.0) as u8;
        let _ = self.midi.push(MidiNote {
            frame: now,
            channel,
            note,
            velocity,
        });
        self.tracks[t].midi_notes[slot] = Some(MidiHeld {
            channel,
            note,
            at: now + gate.max(1) as u64,
            born,
        });
    }

    /// Note off de la nota MIDI de instrumento `i` de la pista, en `frame`.
    fn instrument_midi_off(&mut self, t: usize, i: usize, frame: u64) {
        if let Some(m) = self.tracks[t].midi_notes[i].take() {
            let _ = self.midi.push(MidiNote {
                frame,
                channel: m.channel,
                note: m.note,
                velocity: 0,
            });
        }
    }

    /// Apaga ya la nota MIDI de la nota del motor `born` (robada o cortada).
    fn midi_off_born(&mut self, t: usize, born: u64) {
        let now = self.transport.frame;
        if let Some(i) = self.tracks[t]
            .midi_notes
            .iter()
            .position(|m| m.is_some_and(|m| m.born == born))
        {
            self.instrument_midi_off(t, i, now);
        }
    }

    /// Abre una nota en una voz de la pista. Si no queda ninguna libre,
    /// roba las más antiguas (hacen el fundido mínimo aparte y dejan de
    /// contar).
    fn open_note(
        &mut self,
        t: usize,
        sample: &Arc<Sample>,
        instrument: &crate::instrument::Instrument,
        mut on: NoteOn,
    ) {
        let fade = ((FADE_SECONDS * self.sample_rate as f32) as u64).max(1);
        let voices = instrument.voices as usize;
        let is_voice = |n: &Option<Note>| n.as_ref().is_some_and(Note::is_voice);
        loop {
            let track = &mut self.tracks[t];
            if track.notes.iter().filter(|n| is_voice(n)).count() < voices {
                break;
            }
            let oldest = (0..MAX_VOICES)
                .filter(|&v| is_voice(&track.notes[v]))
                .min_by_key(|&v| track.notes[v].as_ref().map_or(u64::MAX, |n| n.born))
                .expect("hay voces sonando");
            let mut stolen = track.notes[oldest].take().expect("es una voz");
            stolen.kill();
            let born = stolen.born;
            self.park_tail(t, stolen);
            self.midi_off_born(t, born);
        }
        let track = &mut self.tracks[t];
        track.born += 1;
        on.born = track.born;
        let note = Note::new(sample.clone(), instrument, on, fade);
        let free = (0..MAX_VOICES)
            .find(|&v| !is_voice(&track.notes[v]))
            .expect("se acaba de liberar una voz");
        if let Some(old) = track.notes[free].replace(note) {
            // Una nota cortada (choke, paro) que aún hacía su fundido.
            if old.is_active() {
                self.park_tail(t, old);
            } else {
                self.discard(old.sample);
            }
        }
    }

    /// Guarda una nota cortada en las colas de la pista (si están llenas,
    /// sale la más antigua).
    fn park_tail(&mut self, t: usize, note: Note) {
        let track = &mut self.tracks[t];
        let tail = (0..MAX_VOICES)
            .find(|&v| track.tails[v].is_none())
            .unwrap_or_else(|| {
                (0..MAX_VOICES)
                    .min_by_key(|&v| track.tails[v].as_ref().map_or(0, |n| n.born))
                    .expect("MAX_VOICES > 0")
            });
        if let Some(old) = track.tails[tail].replace(note) {
            self.discard(old.sample);
        }
    }

    /// Suelta las notas en vivo de la tecla `key`: empiezan su release.
    fn note_off(&mut self, t: usize, key: u8) {
        for note in self.tracks[t].notes.iter_mut().flatten() {
            if note.key == Some(key) {
                note.release_now();
            }
        }
    }

    /// Corta (con el fundido mínimo) las notas y los granos de una pista.
    fn kill_notes(&mut self, t: usize) {
        for note in self.tracks[t].notes.iter_mut().flatten() {
            note.kill();
        }
        let fade = ((FADE_SECONDS * self.sample_rate as f32) as u32).max(1);
        if let Some(emission) = self.tracks[t].cloud.kill(fade) {
            self.discard(emission.sample);
        }
        // Sus notas MIDI se apagan ya.
        let now = self.transport.frame;
        for i in 0..MIDI_HELD {
            self.instrument_midi_off(t, i, now);
        }
    }

    /// La nota MIDI de un golpe de una pista con `with_midi`: la anterior
    /// se apaga, y esta suena lo que su paso (o su parte del retrigger). Si
    /// la pista no se oye (mute, o solo de otra), no sale nada.
    fn midi_note_on(&mut self, t: usize, channel: u8, base: u8, hit: Trigger, gate: usize) {
        let now = self.transport.frame;
        self.midi_note_off(t, now);
        let any_solo = self.tracks.iter().any(Track::soloed);
        let track = &self.tracks[t];
        if track.muted() || (any_solo && !track.soloed()) {
            return;
        }
        let Some(note) = u8::try_from(base as usize + hit.slice)
            .ok()
            .filter(|n| *n <= 127)
        else {
            return;
        };
        let velocity = (hit.playback.gain * 100.0).round().clamp(1.0, 127.0) as u8;
        let _ = self.midi.push(MidiNote {
            frame: now,
            channel,
            note,
            velocity,
        });
        self.tracks[t].midi_off = Some((channel, note, now + gate.max(1) as u64));
    }

    /// Apaga ya (en `frame`) la nota MIDI que suena en la pista, si hay.
    fn midi_note_off(&mut self, t: usize, frame: u64) {
        if let Some((channel, note, _)) = self.tracks[t].midi_off.take() {
            let _ = self.midi.push(MidiNote {
                frame,
                channel,
                note,
                velocity: 0,
            });
        }
    }

    /// Apaga las notas MIDI cuyo paso ha acabado (con el frame exacto en
    /// que tocaba).
    fn midi_due(&mut self, upto: u64) {
        for t in 0..MAX_TRACKS {
            if let Some((_, _, at)) = self.tracks[t].midi_off
                && at <= upto
            {
                self.midi_note_off(t, at);
            }
            // Las de los instrumentos, en orden de su frame.
            while let Some((i, at)) = self.tracks[t]
                .midi_notes
                .iter()
                .enumerate()
                .filter_map(|(i, m)| m.map(|m| (i, m.at)))
                .filter(|&(_, at)| at <= upto)
                .min_by_key(|&(_, at)| at)
            {
                self.instrument_midi_off(t, i, at);
            }
        }
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
            if u != t && group(&self.tracks[u]) == Some(g) {
                if self.tracks[u].voice.is_some() {
                    self.trigger(u, None);
                }
                self.kill_notes(u);
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
        // Las curvas cuentan desde el inicio de la sección (sin arreglo,
        // desde el arranque).
        let (origin_step, origin_frame) = self
            .playlist
            .as_ref()
            .map_or((0, 0), |p| (p.origin_step, p.origin_frame));
        let clock = Clock {
            steps: self.transport.position_steps(rate) - origin_step as f64,
            seconds: (self.transport.frame - origin_frame) as f64 / rate as f64,
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
            if track.voice.is_none()
                && track.fading.is_none()
                && !track.has_notes()
                && !track.cloud.is_active()
            {
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
            let voices = track
                .notes
                .iter()
                .flatten()
                .filter(|n| n.is_voice())
                .count();
            for note in track.notes.iter_mut().chain(&mut track.tails).flatten() {
                note.mix_into(buf);
            }
            let mut ended = None;
            let born = track.cloud.born;
            if let Some(params) = track.pattern.as_ref().and_then(|p| p.granular) {
                let at = Position {
                    frame: self.transport.frame,
                    steps: clock.steps,
                    seconds: clock.seconds,
                    frames_per_step: clock.frames_per_step,
                    sample_rate: rate,
                };
                let fade = ((FADE_SECONDS * rate as f32) as u32).max(1);
                ended = track.cloud.mix_into(buf, &params, &at, fade);
            } else if track.cloud.is_active() {
                // El patrón ya no tiene granos: no nacen más y los que
                // suenan acaban su ventana.
                let fade = ((FADE_SECONDS * rate as f32) as u32).max(1);
                ended = track.cloud.emission.take();
                let at = Position {
                    frame: self.transport.frame,
                    steps: clock.steps,
                    seconds: clock.seconds,
                    frames_per_step: clock.frames_per_step,
                    sample_rate: rate,
                };
                track
                    .cloud
                    .mix_into(buf, &crate::grain::Granular::new(0), &at, fade);
            }
            let grains_born = track.cloud.born.wrapping_sub(born);
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
            self.status.add_voices(t, voices);
            self.status.add_grains(t, grains_born);

            if track.fading.as_ref().is_some_and(|v| !v.is_active())
                && let Some(done) = track.fading.take()
            {
                self.discard(done.sample);
            }
            // Los granos y las notas que han acabado sueltan su sample.
            if let Some(done) = ended {
                self.discard(done.sample);
            }
            for g in 0..crate::grain::MAX_GRAINS {
                if let Some(done) = self.tracks[t].cloud.grains[g].take_if(|g| !g.is_active()) {
                    self.discard(done.sample);
                }
            }
            for v in 0..MAX_VOICES {
                if let Some(done) = self.tracks[t].notes[v].take_if(|n| !n.is_active()) {
                    self.discard(done.sample);
                }
                if let Some(done) = self.tracks[t].tails[v].take_if(|n| !n.is_active()) {
                    self.discard(done.sample);
                }
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

    /// Devuelve un arreglo entero al hilo de control.
    fn discard_arrangement(&mut self, arrangement: Arrangement) {
        if let Err(rtrb::PushError::Full(garbage)) =
            self.garbage.push(Garbage::Arrangement(arrangement))
        {
            std::mem::forget(garbage);
        }
    }

    /// Al empezar un compás: entra el arreglo pendiente (seguir por la
    /// misma sección si sigue estando) o se avanza por la línea de tiempo;
    /// si cambia la sección, sus patrones pasan a las pistas y los de la
    /// anterior vuelven a su sitio en el arreglo. Publica la posición.
    fn advance_playlist(&mut self, global: u64, now: u64) {
        let bar_steps = self.transport.meter().steps as u64;
        let bar_frames = crate::sequencer::bar_frames_steps(
            bar_steps as usize,
            self.transport.bpm(),
            self.sample_rate,
        );
        if let Some(change) = self.pending_arrangement.take() {
            let old = self.playlist.take();
            let Some(new) = change else {
                // Sin arreglo: los patrones de las pistas son los del lote
                // (llegan como `pending`); los de la sección que sonaba, a la
                // basura con su arreglo.
                if let Some(old) = old {
                    self.discard_arrangement(old.arrangement);
                }
                self.status.set_section(None);
                return;
            };
            // Dónde seguir: lo que iba a sonar ahora en el arreglo anterior,
            // en el nuevo.
            let (place, kept, lost) = match &old {
                Some(old) if !old.restart => {
                    let next = old.next();
                    match resume(&old.arrangement, next, &new) {
                        Some(place) => {
                            let same = place == next && next.bar > 0;
                            (
                                place,
                                same.then_some((old.origin_step, old.origin_frame)),
                                false,
                            )
                        }
                        None => (Place { entry: 0, bar: 0 }, None, true),
                    }
                }
                _ => (Place { entry: 0, bar: 0 }, None, false),
            };
            let (looping, origin) = match &old {
                Some(old) => (old.looping, kept),
                None => (false, None),
            };
            let back = place.bar as u64;
            let (origin_step, origin_frame) = origin.unwrap_or((
                global.saturating_sub(back * bar_steps),
                now.saturating_sub(back * bar_frames),
            ));
            let mut playlist = Playlist {
                current: new.timeline[place.entry].0,
                arrangement: new,
                place,
                origin_step,
                origin_frame,
                looping,
                jump: None,
                restart: false,
            };
            // Los patrones de la sección nueva, a las pistas; los que había
            // (del arreglo anterior), a la basura.
            for t in 0..MAX_TRACKS {
                let incoming = playlist.arrangement.sections[playlist.current].patterns[t].take();
                let outgoing = put_pattern(&mut self.tracks[t], incoming, self.sample_rate);
                if let Some(outgoing) = outgoing {
                    self.discard_pattern(outgoing);
                }
            }
            if let Some(old) = old {
                self.discard_arrangement(old.arrangement);
            }
            self.playlist = Some(playlist);
            self.status.set_lost(lost);
        } else if let Some(playlist) = &mut self.playlist {
            let place = if playlist.restart {
                Place { entry: 0, bar: 0 }
            } else {
                playlist.next()
            };
            playlist.restart = false;
            playlist.jump = None;
            if place.bar == 0 {
                playlist.origin_step = global;
                playlist.origin_frame = now;
            }
            playlist.place = place;
            let section = playlist.arrangement.timeline[place.entry].0;
            if section != playlist.current {
                let from = playlist.current;
                for t in 0..MAX_TRACKS {
                    let incoming = playlist.arrangement.sections[section].patterns[t].take();
                    let outgoing = put_pattern(&mut self.tracks[t], incoming, self.sample_rate);
                    playlist.arrangement.sections[from].patterns[t] = outgoing;
                }
                playlist.current = section;
            }
        }
        if let Some(playlist) = &self.playlist {
            self.status.set_section(Some((
                playlist.place.entry,
                playlist.place.bar,
                playlist.looping,
            )));
        }
    }

    fn holds(&self, sample: &Arc<Sample>) -> bool {
        let same = |other: &Arc<Sample>| Arc::ptr_eq(sample, other);
        let same_pattern = |p: &Option<Pattern>| p.as_ref().is_some_and(|p| same(&p.sample));
        self.tracks.iter().any(|t| {
            t.voice.as_ref().is_some_and(|v| same(&v.sample))
                || t.fading.as_ref().is_some_and(|v| same(&v.sample))
                || t.notes
                    .iter()
                    .chain(&t.tails)
                    .flatten()
                    .any(|n| same(&n.sample))
                || t.cloud.emission.as_ref().is_some_and(|e| same(&e.sample))
                || t.cloud.grains.iter().flatten().any(|g| same(&g.sample))
                || same_pattern(&t.pattern)
                || t.pending.as_ref().is_some_and(same_pattern)
                || t.staged.as_ref().is_some_and(same_pattern)
        }) || [
            self.playlist.as_ref().map(|p| &p.arrangement),
            self.pending_arrangement.as_ref().and_then(Option::as_ref),
            self.staged_arrangement.as_ref().and_then(Option::as_ref),
        ]
        .into_iter()
        .flatten()
        .flat_map(|a| a.sections.iter())
        .flat_map(|s| s.patterns.iter().flatten())
        .any(|p| same(&p.sample))
            || self.preview.as_ref().is_some_and(|v| same(&v.sample))
            || self
                .preview_fading
                .as_ref()
                .is_some_and(|v| same(&v.sample))
    }
}

/// Pone `new` como patrón de la pista (al empezar un compás) y devuelve
/// el que tenía. Lo que dependía del patrón anterior se ajusta: los
/// parámetros suavizados de un efecto de otro tipo, los subdisparos
/// pendientes, el volumen y los envíos si la pista está callada, y el
/// estado de los efectos.
fn put_pattern(track: &mut Track, new: Option<Pattern>, sample_rate: u32) -> Option<Pattern> {
    let old = std::mem::replace(&mut track.pattern, new);
    // Un efecto de otro tipo en la ranura: su parámetro suavizado ya no
    // vale.
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
    // Los disparos pendientes eran del patrón anterior (quizá de otro
    // sample): no se mezclan con el nuevo.
    track.retrig = None;
    // Si la pista está callada, el volumen nuevo entra ya, sin suavizado:
    // el primer golpe suena con él desde el principio.
    if !track.sounding() {
        track.gain = track.pattern.as_ref().map_or(1.0, |p| p.gain);
        track.sends = track.pattern.as_ref().map_or([0.0; BUSES], |p| p.sends);
    }
    for (i, state) in track.fx.iter_mut().enumerate() {
        let fx = track.pattern.as_ref().and_then(|p| p.fx[i]);
        *state = state.update(fx, sample_rate);
    }
    old
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
