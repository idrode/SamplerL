use std::fmt;
use std::ops::Range;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use rtrb::{Consumer, Producer, RingBuffer};

use crate::fx::{FxState, MAX_FX};
use crate::limiter::soft_limit;
use crate::sample::Sample;
use crate::sequencer::{Pattern, Transport, Trigger, valid_bpm};
use crate::slice::Slices;

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
const CHUNK: usize = 256;
/// Constante de tiempo del suavizado del volumen de pista.
const GAIN_SMOOTHING_SECONDS: f32 = 0.005;
/// Duración de los fundidos de entrada y salida de cada voz.
const FADE_SECONDS: f32 = 0.0015;

// `Pattern` va dentro del comando y no en una `Box`: liberar la `Box` en el
// hilo de audio rompería la regla de no liberar memoria en el callback.
#[allow(clippy::large_enum_variant)]
enum Command {
    /// Reproduce los frames `start..end` del sample en la pista 0.
    Play {
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
    /// Empieza un lote: los cambios se guardan aparte hasta `CommitBatch`.
    BeginBatch,
    /// Hace efectivos a la vez todos los cambios del lote.
    CommitBatch,
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

    /// Suma la voz en `out` (estéreo entrelazado).
    fn mix_into(&mut self, out: &mut [f32]) {
        for frame in out.as_chunks_mut::<2>().0 {
            if !self.is_active() {
                break;
            }
            let gain = self.playback.gain * self.envelope();
            let (l, r) = self.read();
            frame[0] += gain * l;
            frame[1] += gain * r;
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
    /// Número de pista fuera de rango.
    NoSuchTrack(usize),
    /// Volumen de pista fuera de rango.
    InvalidTrackGain(f32),
    /// Efecto de pista con parámetros fuera de rango.
    InvalidFx(&'static str),
}

impl fmt::Display for SendError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SendError::QueueFull => write!(f, "cola de comandos llena"),
            SendError::NoSuchSlice { index, count } => {
                write!(f, "no existe el corte {index} (hay {count})")
            }
            SendError::InvalidFx(reason) => write!(f, "efecto: {reason}"),
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

    /// Patrón de la pista `track`; `None` la silencia.
    pub fn set_pattern(&mut self, track: usize, pattern: Option<Pattern>) -> &mut Self {
        self.patterns.push((track, pattern));
        self
    }
}

/// Lado de control (UI/script). Envía comandos al hilo de audio sin bloquear.
pub struct Controller {
    commands: Producer<Command>,
    garbage: Consumer<Arc<Sample>>,
    playing: Arc<AtomicBool>,
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
}

/// Retrigger en curso: el paso que empezó en `start` y dura `length` frames
/// se divide en `count` partes; ya se han disparado `done`.
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
        }
    }
}

/// Lado de audio. Se mueve al callback de audio (o se usa para renderizar
/// offline). `process` no reserva memoria, no bloquea y no hace E/S.
pub struct Engine {
    commands: Consumer<Command>,
    garbage: Producer<Arc<Sample>>,
    playing: Arc<AtomicBool>,
    sample_rate: u32,
    transport: Transport,
    tracks: [Track; MAX_TRACKS],
    /// Buffers de mezcla (estéreo entrelazado), de tamaño fijo.
    track_buf: [f32; 2 * CHUNK],
    master: [f32; 2 * CHUNK],
    batch_open: bool,
    staged_bpm: Option<f64>,
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
            tracks: std::array::from_fn(|_| Track::default()),
            track_buf: [0.0; 2 * CHUNK],
            master: [0.0; 2 * CHUNK],
            batch_open: false,
            staged_bpm: None,
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
        let range = slices.get(index).ok_or(SendError::NoSuchSlice {
            index,
            count: slices.len(),
        })?;
        self.play_range(sample.clone(), range)
    }

    /// Reproduce los frames `range` del sample (se recorta a su longitud) en
    /// la pista 0.
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
        self.collect_garbage();
        let needed = 2 + batch.patterns.len() + usize::from(batch.bpm.is_some());
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
        for (track, pattern) in batch.patterns {
            push(Command::SetPattern { track, pattern });
        }
        push(Command::CommitBatch);
        Ok(())
    }

    /// Arranca el secuenciador en el paso 0, aplicando lo pendiente.
    pub fn start(&mut self) -> Result<(), SendError> {
        self.send(Command::Start, Some(true))
    }

    /// Para el secuenciador y las voces que suenan (con fundido).
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
                .any(|t| t.voice.as_ref().is_some_and(Voice::is_active) || t.fading.is_some());
        self.playing.store(playing, Ordering::Relaxed);
    }

    fn handle(&mut self, cmd: Command) {
        match cmd {
            Command::Play {
                sample,
                start,
                end,
                playback,
            } => self.trigger(0, Some(Voice::new(sample, start, end, playback))),
            Command::SetPattern { track, pattern } => {
                let track = &mut self.tracks[track];
                let slot = if self.batch_open {
                    &mut track.staged
                } else {
                    &mut track.pending
                };
                if let Some(Some(old)) = slot.replace(pattern) {
                    self.discard(old.sample);
                }
            }
            Command::SetBpm(bpm) => {
                if self.batch_open {
                    self.staged_bpm = Some(bpm);
                } else {
                    self.transport.pending_bpm = Some(bpm);
                }
            }
            Command::BeginBatch => self.batch_open = true,
            Command::CommitBatch => {
                self.batch_open = false;
                if let Some(bpm) = self.staged_bpm.take() {
                    self.transport.pending_bpm = Some(bpm);
                }
                for t in 0..MAX_TRACKS {
                    if let Some(change) = self.tracks[t].staged.take()
                        && let Some(Some(old)) = self.tracks[t].pending.replace(change)
                    {
                        self.discard(old.sample);
                    }
                }
            }
            Command::Start => {
                self.transport.start();
                for track in &mut self.tracks {
                    track.retrig = None;
                }
            }
            Command::Stop => {
                self.transport.running = false;
                for t in 0..MAX_TRACKS {
                    self.tracks[t].retrig = None;
                    self.trigger(t, None);
                }
            }
        }
    }

    /// Dispara el paso actual del secuenciador en todas las pistas y avanza.
    /// Al empezar un compás, antes aplica los patrones pendientes.
    fn step(&mut self) {
        let now = self.transport.frame;
        let step = self.transport.advance(self.sample_rate);
        let next = self.transport.next_step_frame(self.sample_rate);
        for t in 0..MAX_TRACKS {
            if step == 0
                && let Some(change) = self.tracks[t].pending.take()
            {
                let track = &mut self.tracks[t];
                let old = std::mem::replace(&mut track.pattern, change);
                // Si la pista está callada, el volumen nuevo entra ya, sin
                // suavizado: el primer golpe suena con él desde el principio.
                if !track.voice.as_ref().is_some_and(Voice::is_active) && track.fading.is_none() {
                    track.gain = track.pattern.as_ref().map_or(1.0, |p| p.gain);
                }
                for (i, state) in track.fx.iter_mut().enumerate() {
                    let fx = track.pattern.as_ref().and_then(|p| p.fx[i]);
                    *state = state.update(fx, self.sample_rate);
                }
                if let Some(old) = old {
                    self.discard(old.sample);
                }
            }
            let hit = self.tracks[t].pattern.as_ref().and_then(|p| p.steps[step]);
            // Los subdisparos de un retrigger caben dentro de su paso.
            self.tracks[t].retrig = hit.filter(|h| h.retrig > 1).map(|hit| Retrig {
                hit,
                count: hit.retrig,
                done: 1,
                start: now,
                length: next - now,
            });
            if let Some(hit) = hit {
                self.play_hit(t, hit);
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
        self.play_hit(t, retrig.hit);
    }

    /// Dispara `hit` con el sample del patrón actual de la pista.
    fn play_hit(&mut self, t: usize, hit: Trigger) {
        let Some(pattern) = &self.tracks[t].pattern else {
            return;
        };
        let voice = Voice::new(pattern.sample.clone(), hit.start, hit.end, hit.playback);
        self.trigger(t, Some(voice));
    }

    /// Sustituye la voz principal de una pista; la anterior hace su fundido
    /// de salida.
    fn trigger(&mut self, track: usize, voice: Option<Voice>) {
        if let Some(old) = std::mem::replace(&mut self.tracks[track].voice, voice) {
            self.fade_out(track, old);
        }
    }

    /// Mezcla `n` frames en `self.master`: cada pista en su buffer, con su
    /// volumen, y al final el limitador.
    fn mix(&mut self, n: usize) {
        self.master[..2 * n].fill(0.0);
        let smoothing = 1.0 - (-1.0 / (GAIN_SMOOTHING_SECONDS * self.sample_rate as f32)).exp();
        for t in 0..MAX_TRACKS {
            let track = &mut self.tracks[t];
            let target = track.pattern.as_ref().map_or(1.0, |p| p.gain);
            if track.voice.is_none() && track.fading.is_none() {
                // En silencio no hay nada que suavizar: el siguiente golpe
                // empieza ya con el volumen nuevo y los efectos sin memoria.
                track.gain = target;
                for state in &mut track.fx {
                    state.reset();
                }
                continue;
            }
            let buf = &mut self.track_buf[..2 * n];
            buf.fill(0.0);
            if let Some(voice) = &mut track.voice {
                voice.mix_into(buf);
            }
            if let Some(fading) = &mut track.fading {
                fading.mix_into(buf);
            }
            for state in &mut track.fx {
                state.process(buf);
            }

            let master = &mut self.master[..2 * n];
            for (dst, src) in master
                .as_chunks_mut::<2>()
                .0
                .iter_mut()
                .zip(buf.as_chunks::<2>().0)
            {
                track.gain += (target - track.gain) * smoothing;
                dst[0] += track.gain * src[0];
                dst[1] += track.gain * src[1];
            }

            if track.fading.as_ref().is_some_and(|v| !v.is_active())
                && let Some(done) = track.fading.take()
            {
                self.discard(done.sample);
            }
        }
        for s in &mut self.master[..2 * n] {
            *s = soft_limit(*s);
        }
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
        let same_pattern = |p: &Option<Pattern>| p.as_ref().is_some_and(|p| same(&p.sample));
        self.tracks.iter().any(|t| {
            t.voice.as_ref().is_some_and(|v| same(&v.sample))
                || t.fading.as_ref().is_some_and(|v| same(&v.sample))
                || same_pattern(&t.pattern)
                || t.pending.as_ref().is_some_and(same_pattern)
                || t.staged.as_ref().is_some_and(same_pattern)
        })
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
