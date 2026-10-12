use std::fmt;
use std::ops::Range;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use rtrb::{Consumer, Producer, RingBuffer};

use crate::sample::Sample;
use crate::sequencer::{Pattern, Transport, valid_bpm};
use crate::slice::Slices;

const COMMAND_CAPACITY: usize = 64;
// A la basura solo va un `Arc` cuando el motor no guarda otra referencia al
// mismo sample (ver `Engine::discard`), así que las voces que dispara el
// secuenciador no la llenan: cada elemento sale de un comando y el
// controlador la recoge antes de cada envío.
const GARBAGE_CAPACITY: usize = COMMAND_CAPACITY * 2;
/// Duración de los fundidos de entrada y salida de cada voz.
const FADE_SECONDS: f32 = 0.0015;

// `Pattern` va dentro del comando y no en una `Box`: liberar la `Box` en el
// hilo de audio rompería la regla de no liberar memoria en el callback.
#[allow(clippy::large_enum_variant)]
enum Command {
    /// Reproduce los frames `start..end` del sample.
    Play {
        sample: Arc<Sample>,
        start: usize,
        end: usize,
        playback: Playback,
    },
    /// Patrón para el siguiente compás.
    SetPattern(Pattern),
    /// BPM para el siguiente compás.
    SetBpm(f64),
    /// Arranca el secuenciador desde el paso 0.
    Start,
    /// Para el secuenciador y la voz que suena.
    Stop,
}

/// Cómo se reproduce un rango: ganancia, velocidad de lectura (pitch) y
/// sentido.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Playback {
    pub gain: f32,
    /// Frames del sample por frame de salida: 2.0 es una octava arriba (y
    /// dura la mitad), 0.5 una octava abajo (y dura el doble).
    pub rate: f64,
    pub reverse: bool,
}

impl Playback {
    pub const NORMAL: Playback = Playback {
        gain: 1.0,
        rate: 1.0,
        reverse: false,
    };
}

struct Voice {
    sample: Arc<Sample>,
    start: usize,
    end: usize,
    playback: Playback,
    /// Frames de salida ya renderizados y total que dura la voz.
    rendered: usize,
    length: usize,
    fade: usize,
    /// Frames que quedan del fundido de salida forzado (voz interrumpida).
    release: Option<usize>,
}

impl Voice {
    fn new(sample: Arc<Sample>, start: usize, end: usize, playback: Playback) -> Self {
        let fade = ((FADE_SECONDS * sample.sample_rate() as f32) as usize).max(1);
        // Último frame de salida: el que lee el último frame del rango.
        let length = match end - start {
            0 => 0,
            frames => ((frames - 1) as f64 / playback.rate).floor() as usize + 1,
        };
        Voice {
            sample,
            start,
            end,
            playback,
            rendered: 0,
            length,
            fade,
            release: None,
        }
    }

    fn is_active(&self) -> bool {
        self.rendered < self.length && self.release != Some(0)
    }

    /// Empieza un fundido de salida aunque el corte no haya terminado.
    fn release(&mut self) {
        if self.release.is_none() {
            self.release = Some(self.fade);
        }
    }

    /// Envolvente del frame actual: rampa de entrada desde el inicio de la
    /// reproducción, de salida hacia su final y, si se interrumpió, la del
    /// release. En tiempo de salida, así es igual con pitch o reverse.
    fn envelope(&self) -> f32 {
        let from_start = self.rendered;
        let to_end = self.length - 1 - self.rendered;
        let fade = self.fade as f32;
        let mut gain = (from_start.min(to_end) as f32 / fade).min(1.0);
        if let Some(left) = self.release {
            gain *= (left - 1) as f32 / fade;
        }
        gain
    }

    /// Frame del sample para el frame de salida actual, con interpolación
    /// lineal si la posición cae entre dos frames.
    fn read(&self) -> (f32, f32) {
        let offset = self.rendered as f64 * self.playback.rate;
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

    /// Suma la voz en `out` (entrelazado, `channels` canales).
    fn mix_into(&mut self, out: &mut [f32], channels: usize) {
        for frame in out.chunks_exact_mut(channels) {
            if !self.is_active() {
                break;
            }
            let gain = self.playback.gain * self.envelope();
            let (l, r) = self.read();
            match frame {
                [mono] => *mono += 0.5 * gain * (l + r),
                [a, b, ..] => {
                    *a += gain * l;
                    *b += gain * r;
                }
                [] => {}
            }
            self.rendered += 1;
            if let Some(left) = &mut self.release {
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
}

impl fmt::Display for SendError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SendError::QueueFull => write!(f, "cola de comandos llena"),
            SendError::NoSuchSlice { index, count } => {
                write!(f, "no existe el corte {index} (hay {count})")
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

/// Lado de control (UI/script). Envía comandos al hilo de audio sin bloquear.
pub struct Controller {
    commands: Producer<Command>,
    garbage: Consumer<Arc<Sample>>,
    playing: Arc<AtomicBool>,
}

/// Lado de audio. Se mueve al callback de audio (o se usa para renderizar
/// offline). `process` no reserva memoria, no bloquea y no hace E/S.
pub struct Engine {
    commands: Consumer<Command>,
    garbage: Producer<Arc<Sample>>,
    playing: Arc<AtomicBool>,
    sample_rate: u32,
    transport: Transport,
    voice: Option<Voice>,
    /// Voz interrumpida que todavía está haciendo su fundido de salida.
    fading: Option<Voice>,
}

/// Crea el par controlador/motor. `sample_rate` es la frecuencia a la que
/// renderiza el motor (la de los samples cargados para él).
pub fn new_engine(sample_rate: u32) -> (Controller, Engine) {
    let (cmd_tx, cmd_rx) = RingBuffer::new(COMMAND_CAPACITY);
    let (garbage_tx, garbage_rx) = RingBuffer::new(GARBAGE_CAPACITY);
    let playing = Arc::new(AtomicBool::new(false));
    (
        Controller {
            commands: cmd_tx,
            garbage: garbage_rx,
            playing: playing.clone(),
        },
        Engine {
            commands: cmd_rx,
            garbage: garbage_tx,
            playing,
            sample_rate,
            transport: Transport::new(),
            voice: None,
            fading: None,
        },
    )
}

impl Controller {
    /// Reproduce el sample entero desde el principio.
    pub fn play(&mut self, sample: Arc<Sample>) -> Result<(), SendError> {
        let frames = sample.frames();
        self.play_range(sample, 0..frames)
    }

    /// Reproduce el corte número `index` (empezando en 0).
    pub fn play_slice(
        &mut self,
        sample: &Arc<Sample>,
        slices: &Slices,
        index: usize,
    ) -> Result<(), SendError> {
        let range = slices.get(index).ok_or(SendError::NoSuchSlice {
            index,
            count: slices.len(),
        })?;
        self.play_range(sample.clone(), range)
    }

    /// Reproduce los frames `range` del sample (se recorta a su longitud).
    pub fn play_range(
        &mut self,
        sample: Arc<Sample>,
        range: Range<usize>,
    ) -> Result<(), SendError> {
        let end = range.end.min(sample.frames());
        let start = range.start.min(end);
        let playback = Playback::NORMAL;
        let cmd = Command::Play {
            sample,
            start,
            end,
            playback,
        };
        self.send(cmd, Some(start < end))
    }

    /// Cambia el patrón. Entra al empezar el siguiente compás (o al arrancar).
    pub fn set_pattern(&mut self, pattern: Pattern) -> Result<(), SendError> {
        self.send(Command::SetPattern(pattern), None)
    }

    /// Cambia el tempo. Entra al empezar el siguiente compás (o al arrancar).
    pub fn set_bpm(&mut self, bpm: f64) -> Result<(), SendError> {
        if !valid_bpm(bpm) {
            return Err(SendError::InvalidBpm(bpm));
        }
        self.send(Command::SetBpm(bpm), None)
    }

    /// Arranca el secuenciador en el paso 0, aplicando lo pendiente.
    pub fn start(&mut self) -> Result<(), SendError> {
        self.send(Command::Start, Some(true))
    }

    /// Para el secuenciador y la voz que suena (con fundido).
    pub fn stop(&mut self) -> Result<(), SendError> {
        self.send(Command::Stop, Some(false))
    }

    pub fn is_playing(&self) -> bool {
        self.playing.load(Ordering::Relaxed)
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
    pub(crate) fn set_sample_rate(&mut self, sample_rate: u32) {
        self.sample_rate = sample_rate;
    }

    /// Rellena `out` (entrelazado, `channels` canales) con el siguiente bloque.
    pub fn process(&mut self, out: &mut [f32], channels: usize) {
        while let Ok(cmd) = self.commands.pop() {
            self.handle(cmd);
        }

        out.fill(0.0);
        let frames = out.len() / channels;
        let mut offset = 0;
        while offset < frames {
            let mut n = frames - offset;
            if self.transport.running {
                let next = self.transport.next_step_frame(self.sample_rate);
                if next <= self.transport.frame {
                    self.step();
                    continue;
                }
                // Partir el bloque en el siguiente paso: disparo exacto al frame.
                n = n.min((next - self.transport.frame) as usize);
            }
            self.mix(
                &mut out[offset * channels..(offset + n) * channels],
                channels,
            );
            offset += n;
            if self.transport.running {
                self.transport.frame += n as u64;
            }
        }

        // La voz principal terminada se queda en `self.voice` hasta que otro
        // comando o paso la reemplace.
        let playing = self.transport.running
            || self.voice.as_ref().is_some_and(Voice::is_active)
            || self.fading.is_some();
        self.playing.store(playing, Ordering::Relaxed);
    }

    fn handle(&mut self, cmd: Command) {
        match cmd {
            Command::Play {
                sample,
                start,
                end,
                playback,
            } => self.trigger(Some(Voice::new(sample, start, end, playback))),
            Command::SetPattern(pattern) => {
                if let Some(old) = self.transport.pending_pattern.replace(pattern) {
                    self.discard(old.sample);
                }
            }
            Command::SetBpm(bpm) => self.transport.pending_bpm = Some(bpm),
            Command::Start => self.transport.start(),
            Command::Stop => {
                self.transport.running = false;
                self.trigger(None);
            }
        }
    }

    /// Dispara el paso actual del secuenciador y avanza.
    fn step(&mut self) {
        let (replaced, hit) = self.transport.advance(self.sample_rate);
        if let Some(old) = replaced {
            self.discard(old.sample);
        }
        if let (Some(hit), Some(pattern)) = (hit, &self.transport.pattern) {
            let voice = Voice::new(pattern.sample.clone(), hit.start, hit.end, hit.playback);
            self.trigger(Some(voice));
        }
    }

    /// Sustituye la voz principal; la anterior hace su fundido de salida.
    fn trigger(&mut self, voice: Option<Voice>) {
        if let Some(old) = std::mem::replace(&mut self.voice, voice) {
            self.fade_out(old);
        }
    }

    fn mix(&mut self, out: &mut [f32], channels: usize) {
        if let Some(voice) = &mut self.voice {
            voice.mix_into(out, channels);
        }
        if let Some(fading) = &mut self.fading {
            fading.mix_into(out, channels);
        }
        if self.fading.as_ref().is_some_and(|v| !v.is_active())
            && let Some(done) = self.fading.take()
        {
            self.discard(done.sample);
        }
    }

    /// Pasa una voz interrumpida a la ranura de fundido. Si ya había otra
    /// fundiéndose, esa se descarta (le quedaba como mucho un fundido).
    fn fade_out(&mut self, mut voice: Voice) {
        if !voice.is_active() {
            self.discard(voice.sample);
            return;
        }
        voice.release();
        if let Some(previous) = self.fading.replace(voice) {
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
        if let Err(rtrb::PushError::Full(sample)) = self.garbage.push(sample) {
            // No debería ocurrir; antes filtrar memoria que liberarla en el callback.
            std::mem::forget(sample);
        }
    }

    fn holds(&self, sample: &Arc<Sample>) -> bool {
        let same = |other: &Arc<Sample>| Arc::ptr_eq(sample, other);
        self.voice.as_ref().is_some_and(|v| same(&v.sample))
            || self.fading.as_ref().is_some_and(|v| same(&v.sample))
            || self
                .transport
                .pattern
                .as_ref()
                .is_some_and(|p| same(&p.sample))
            || self
                .transport
                .pending_pattern
                .as_ref()
                .is_some_and(|p| same(&p.sample))
    }
}
