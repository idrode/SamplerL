use std::fmt;
use std::ops::Range;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use rtrb::{Consumer, Producer, RingBuffer};

use crate::sample::Sample;
use crate::slice::Slices;

const COMMAND_CAPACITY: usize = 64;
// Cada voz nace de un comando y va a la basura una sola vez, y el controlador
// la recoge antes de cada envío, así que esto no se llena.
const GARBAGE_CAPACITY: usize = COMMAND_CAPACITY * 2;
/// Duración de los fundidos de entrada y salida de cada voz.
const FADE_SECONDS: f32 = 0.0015;

enum Command {
    /// Reproduce los frames `start..end` del sample.
    Play {
        sample: Arc<Sample>,
        start: usize,
        end: usize,
    },
    Stop,
}

struct Voice {
    sample: Arc<Sample>,
    start: usize,
    pos: usize,
    end: usize,
    fade: usize,
    /// Frames que quedan del fundido de salida forzado (voz interrumpida).
    release: Option<usize>,
}

impl Voice {
    fn new(sample: Arc<Sample>, start: usize, end: usize) -> Self {
        let fade = ((FADE_SECONDS * sample.sample_rate() as f32) as usize).max(1);
        Voice {
            sample,
            start,
            pos: start,
            end,
            fade,
            release: None,
        }
    }

    fn is_active(&self) -> bool {
        self.pos < self.end && self.release != Some(0)
    }

    /// Empieza un fundido de salida aunque el corte no haya terminado.
    fn release(&mut self) {
        if self.release.is_none() {
            self.release = Some(self.fade);
        }
    }

    /// Ganancia del frame actual: rampa de entrada desde el inicio del corte,
    /// de salida hacia su final y, si se interrumpió, la del release.
    fn gain(&self) -> f32 {
        let from_start = self.pos - self.start;
        let to_end = self.end - 1 - self.pos;
        let fade = self.fade as f32;
        let mut gain = (from_start.min(to_end) as f32 / fade).min(1.0);
        if let Some(left) = self.release {
            gain *= (left - 1) as f32 / fade;
        }
        gain
    }

    /// Suma la voz en `out` (entrelazado, `channels` canales).
    fn mix_into(&mut self, out: &mut [f32], channels: usize) {
        for frame in out.chunks_exact_mut(channels) {
            if !self.is_active() {
                break;
            }
            let gain = self.gain();
            let (l, r) = self.sample.frame(self.pos);
            match frame {
                [mono] => *mono += 0.5 * gain * (l + r),
                [a, b, ..] => {
                    *a += gain * l;
                    *b += gain * r;
                }
                [] => {}
            }
            self.pos += 1;
            if let Some(left) = &mut self.release {
                *left -= 1;
            }
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SendError {
    /// La cola de comandos está llena; el comando no se envió.
    QueueFull,
    /// El número de corte no existe.
    NoSuchSlice { index: usize, count: usize },
}

impl fmt::Display for SendError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SendError::QueueFull => write!(f, "cola de comandos llena"),
            SendError::NoSuchSlice { index, count } => {
                write!(f, "no existe el corte {index} (hay {count})")
            }
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
    voice: Option<Voice>,
    /// Voz interrumpida que todavía está haciendo su fundido de salida.
    fading: Option<Voice>,
}

pub fn new_engine() -> (Controller, Engine) {
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
        self.send(Command::Play { sample, start, end }, start < end)
    }

    pub fn stop(&mut self) -> Result<(), SendError> {
        self.send(Command::Stop, false)
    }

    pub fn is_playing(&self) -> bool {
        self.playing.load(Ordering::Relaxed)
    }

    /// Libera aquí (fuera del hilo de audio) los samples que el motor ya no usa.
    pub fn collect_garbage(&mut self) {
        while self.garbage.pop().is_ok() {}
    }

    fn send(&mut self, cmd: Command, playing: bool) -> Result<(), SendError> {
        self.collect_garbage();
        self.commands.push(cmd).map_err(|_| SendError::QueueFull)?;
        // Estado previsto hasta que el motor procese el comando y lo actualice.
        self.playing.store(playing, Ordering::Relaxed);
        Ok(())
    }
}

impl Engine {
    /// Rellena `out` (entrelazado, `channels` canales) con el siguiente bloque.
    pub fn process(&mut self, out: &mut [f32], channels: usize) {
        while let Ok(cmd) = self.commands.pop() {
            let new_voice = match cmd {
                Command::Play { sample, start, end } => Some(Voice::new(sample, start, end)),
                Command::Stop => None,
            };
            if let Some(old) = std::mem::replace(&mut self.voice, new_voice) {
                self.fade_out(old);
            }
        }

        out.fill(0.0);
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

        // La voz principal terminada se queda en `self.voice` hasta que otro
        // comando la reemplace.
        let playing = self.voice.as_ref().is_some_and(Voice::is_active) || self.fading.is_some();
        self.playing.store(playing, Ordering::Relaxed);
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

    /// Devuelve un sample al hilo de control para que lo libere allí.
    fn discard(&mut self, sample: Arc<Sample>) {
        if let Err(rtrb::PushError::Full(sample)) = self.garbage.push(sample) {
            // No debería ocurrir; antes filtrar memoria que liberarla en el callback.
            std::mem::forget(sample);
        }
    }
}
