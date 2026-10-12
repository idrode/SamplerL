use std::fmt;
use std::ops::Range;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use rtrb::{Consumer, Producer, RingBuffer};

use crate::sample::Sample;
use crate::slice::Slices;

const COMMAND_CAPACITY: usize = 64;
// Cada comando procesado genera como mucho un elemento de basura, y el
// controlador la recoge antes de cada envío, así que esto no se llena.
const GARBAGE_CAPACITY: usize = COMMAND_CAPACITY * 2;

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
    pos: usize,
    end: usize,
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
                Command::Play { sample, start, end } => Some(Voice {
                    sample,
                    pos: start,
                    end,
                }),
                Command::Stop => None,
            };
            if let Some(old) = std::mem::replace(&mut self.voice, new_voice) {
                self.discard(old.sample);
            }
        }

        out.fill(0.0);
        let Some(voice) = &mut self.voice else {
            self.playing.store(false, Ordering::Relaxed);
            return;
        };

        let remaining = voice.end - voice.pos;
        let n = remaining.min(out.len() / channels);
        for (i, frame) in out.chunks_exact_mut(channels).take(n).enumerate() {
            let (l, r) = voice.sample.frame(voice.pos + i);
            match frame {
                [mono] => *mono = 0.5 * (l + r),
                [a, b, ..] => {
                    *a = l;
                    *b = r;
                }
                [] => {}
            }
        }
        voice.pos += n;
        // La voz terminada se queda en `self.voice` (sin soltar el Arc aquí)
        // hasta que otro comando la reemplace.
        self.playing.store(voice.pos < voice.end, Ordering::Relaxed);
    }

    /// Devuelve un sample al hilo de control para que lo libere allí.
    fn discard(&mut self, sample: Arc<Sample>) {
        if let Err(rtrb::PushError::Full(sample)) = self.garbage.push(sample) {
            // No debería ocurrir; antes filtrar memoria que liberarla en el callback.
            std::mem::forget(sample);
        }
    }
}
