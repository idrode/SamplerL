//! Efectos de modulación: flanger, phaser y chorus.
//!
//! - **Flanger**: una línea de retardo corta (de 0,1 a 25 ms) con
//!   realimentación, cuyo retardo mueve un LFO. Lee con interpolación de
//!   Hermite de 4 puntos (sin el zumbido de la lineal al barrer).
//! - **Phaser**: de 2 a 12 filtros paso-todo de primer orden en serie cuya
//!   frecuencia mueve un LFO (en escala logarítmica alrededor de `center`),
//!   con realimentación y mezcla con la señal seca.
//! - **Chorus**: la línea del flanger con retardos largos (de 10 a 40 ms),
//!   de 1 a 4 voces con su LFO desfasado y sin realimentación.
//!
//! La configuración va en [`crate::Fx`]; el estado, en [`crate::fx`]
//! (`FxState`). El del phaser cabe en el estado (dos números por etapa y
//! canal). La línea del flanger y del chorus no: ocupa [`LINE_FRAMES`]
//! frames estéreo (64 KiB) y el estado de los efectos se copia en cada voz,
//! así que vive aparte, en [`Lines`], una por ranura de efecto de cada
//! pista y de cada grupo, reservada al crear el motor para la frecuencia
//! más alta que se admite ([`MAX_SAMPLE_RATE`]). Por eso flanger y chorus
//! van en la pista y en el grupo, no en un corte ni en un paso.
//!
//! El LFO sigue a la canción: con `sync`, su fase sale de la posición en
//! pasos con aritmética entera (sin deriva en canciones largas); con `rate`
//! (Hz), arranca en la fase que le toca por los segundos de la canción y
//! luego avanza muestra a muestra (así `rate` se puede automatizar sin
//! saltos).

use std::f32::consts::PI;

/// Frecuencia de muestreo más alta para la que se reservan las líneas.
pub const MAX_SAMPLE_RATE: u32 = 192_000;

/// Retardo más largo que se lee (el del chorus más su barrido), en ms.
pub const MAX_LINE_MS: f32 = 40.0;

/// Frames de cada línea: la potencia de dos que cabe [`MAX_LINE_MS`] a
/// [`MAX_SAMPLE_RATE`] más los vecinos de la interpolación.
pub const LINE_FRAMES: usize = 8192;
const LINE_MASK: usize = LINE_FRAMES - 1;
const _: () = assert!(
    (MAX_LINE_MS / 1000.0 * MAX_SAMPLE_RATE as f32) as usize + 4 <= LINE_FRAMES
        && LINE_FRAMES.is_power_of_two()
);

/// Retardo mínimo en frames: la interpolación lee dos frames por delante
/// de la posición y el frame que se escribe ahora aún no está.
const MIN_DELAY_FRAMES: f32 = 3.0;

/// Realimentación máxima (en valor absoluto) del flanger y del phaser.
pub const MAX_FEEDBACK: f32 = 0.95;
/// Velocidad del LFO, en Hz.
pub const RATE_RANGE: (f32, f32) = (0.01, 20.0);
/// Desfase del LFO entre el canal izquierdo y el derecho, en grados.
pub const PHASE_RANGE: (f32, f32) = (0.0, 180.0);
/// Flanger: retardo base y barrido, en ms.
pub const FLANGER_DELAY_RANGE: (f32, f32) = (0.1, 15.0);
pub const FLANGER_DEPTH_RANGE: (f32, f32) = (0.0, 10.0);
/// Phaser: centro del barrido (Hz) y su amplitud (octavas a cada lado).
pub const PHASER_CENTER_RANGE: (f32, f32) = (50.0, 10_000.0);
pub const PHASER_DEPTH_RANGE: (f32, f32) = (0.0, 4.0);
/// Etapas que admite el phaser.
pub const PHASER_STAGES: [u8; 5] = [2, 4, 6, 8, 12];
const MAX_STAGES: usize = 12;
/// Chorus: retardo base y barrido (ms) y voces.
pub const CHORUS_DELAY_RANGE: (f32, f32) = (10.0, 30.0);
pub const CHORUS_DEPTH_RANGE: (f32, f32) = (0.0, 10.0);
pub const MAX_CHORUS_VOICES: u8 = 4;
/// Ciclos del LFO con `sync`: de 1/64 de compás a 64 compases.
pub const MAX_SYNC: u16 = 64;

/// Lo que tardan los parámetros en llegar a su valor nuevo (un polo, por
/// muestra): sin clics al recargar o al automatizar a saltos.
const SMOOTHING_SECONDS: f32 = 0.01;
/// Frames de cada trozo con el LFO (y los coeficientes del phaser)
/// interpolados en línea recta: a 20 Hz, el error es de una parte en 10⁵.
const LFO_BLOCK: usize = 16;
/// Por debajo de esto (−120 dB) una muestra cuenta como silencio.
const QUIET: f32 = 1e-6;
/// Lo que sigue sonando el phaser en silencio antes de darlo por callado.
const PHASER_HOLD_SECONDS: f32 = 0.05;
/// Fundido del pánico: la cola se apaga en este tiempo.
const HUSH_SECONDS: f32 = 0.005;

/// Forma del LFO (de 0 a 1, empieza en 0).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum LfoWave {
    #[default]
    Sine,
    Triangle,
}

impl LfoWave {
    pub const ALL: [(LfoWave, &'static str); 2] =
        [(LfoWave::Sine, "sine"), (LfoWave::Triangle, "tri")];

    pub fn from_name(name: &str) -> Option<Self> {
        match name {
            "sine" | "sin" => Some(LfoWave::Sine),
            "tri" | "triangle" => Some(LfoWave::Triangle),
            _ => None,
        }
    }

    pub fn name(self) -> &'static str {
        Self::ALL
            .iter()
            .find(|(w, _)| *w == self)
            .map_or("sine", |(_, n)| n)
    }

    fn at(self, phase: f64) -> f32 {
        let p = phase.rem_euclid(1.0);
        match self {
            LfoWave::Sine => (0.5 - 0.5 * (std::f64::consts::TAU * p).cos()) as f32,
            LfoWave::Triangle => (1.0 - (2.0 * p - 1.0).abs()) as f32,
        }
    }
}

/// Ciclo del LFO atado al compás: `num/den` compases (de 16 pasos).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Sync {
    pub num: u16,
    pub den: u16,
}

impl Sync {
    /// `n` ciclos por compás.
    pub fn per_bar(n: u16) -> Self {
        Sync { num: 1, den: n }
    }

    pub fn validate(&self) -> Result<(), &'static str> {
        if !(1..=MAX_SYNC).contains(&self.num) || !(1..=MAX_SYNC).contains(&self.den) {
            return Err("sync fuera de rango (de 1/64 de compás a 64 compases)");
        }
        Ok(())
    }

    /// Pasos que dura un ciclo.
    pub fn steps(&self) -> f64 {
        crate::sequencer::STEPS as f64 * self.num as f64 / self.den as f64
    }

    /// Fase (en ciclos, de 0 a 1) en la posición `steps` de la canción.
    /// Con aritmética entera sobre los pasos enteros: no deriva por larga
    /// que sea la canción.
    pub(crate) fn phase_at(&self, steps: f64) -> f64 {
        let steps = steps.max(0.0);
        let whole = steps.floor();
        let period = crate::sequencer::STEPS as u64 * self.num as u64;
        let units = (whole as u64 % period) * self.den as u64 % period;
        (units as f64 + (steps - whole) * self.den as f64) / period as f64
    }
}

/// El LFO de un efecto de modulación.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Lfo {
    /// Ciclos por segundo (sin `sync`).
    pub rate_hz: f32,
    /// Atado al compás: manda sobre `rate_hz`.
    pub sync: Option<Sync>,
    pub wave: LfoWave,
    /// Desfase del canal derecho, en grados.
    pub phase_deg: f32,
}

impl Lfo {
    fn validate(&self) -> Result<(), &'static str> {
        if !within(self.rate_hz, RATE_RANGE) {
            return Err("rate fuera de rango (0.01 a 20 Hz)");
        }
        if let Some(s) = self.sync {
            s.validate()?;
        }
        if !within(self.phase_deg, PHASE_RANGE) {
            return Err("phase fuera de rango (0 a 180 grados)");
        }
        Ok(())
    }
}

fn within(v: f32, (lo, hi): (f32, f32)) -> bool {
    (lo..=hi).contains(&v)
}

fn check_feedback(feedback: f32) -> Result<(), &'static str> {
    if !within(feedback, (-MAX_FEEDBACK, MAX_FEEDBACK)) {
        return Err("feedback fuera de rango (-0.95 a 0.95)");
    }
    Ok(())
}

fn check_mix(mix: f32) -> Result<(), &'static str> {
    if !within(mix, (0.0, 1.0)) {
        return Err("mix fuera de rango (0 a 1)");
    }
    Ok(())
}

/// Flanger ([`crate::Fx::Flanger`]).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FlangerParams {
    pub lfo: Lfo,
    /// Lo que barre el LFO por encima del retardo base, en ms.
    pub depth_ms: f32,
    /// Retardo base, en ms.
    pub delay_ms: f32,
    pub feedback: f32,
    pub mix: f32,
    /// La señal retardada sale con la polaridad invertida (los huecos van
    /// donde antes estaban los picos: suena más hueco).
    pub invert: bool,
}

impl FlangerParams {
    pub const DEFAULT: FlangerParams = FlangerParams {
        lfo: Lfo {
            rate_hz: 0.25,
            sync: None,
            wave: LfoWave::Sine,
            phase_deg: 90.0,
        },
        depth_ms: 2.0,
        delay_ms: 1.0,
        feedback: 0.5,
        mix: 0.5,
        invert: false,
    };

    pub fn validate(&self) -> Result<(), &'static str> {
        self.lfo.validate()?;
        if !within(self.depth_ms, FLANGER_DEPTH_RANGE) {
            return Err("depth fuera de rango (0 a 10 ms)");
        }
        if !within(self.delay_ms, FLANGER_DELAY_RANGE) {
            return Err("delay fuera de rango (0.1 a 15 ms)");
        }
        check_feedback(self.feedback)?;
        check_mix(self.mix)
    }
}

/// Phaser ([`crate::Fx::Phaser`]).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PhaserParams {
    pub lfo: Lfo,
    /// Octavas que barre el LFO a cada lado de `center_hz`.
    pub depth_oct: f32,
    pub center_hz: f32,
    pub feedback: f32,
    /// Filtros paso-todo en serie (2, 4, 6, 8 o 12): cada dos, un hueco
    /// más en el espectro.
    pub stages: u8,
    pub mix: f32,
}

impl PhaserParams {
    pub const DEFAULT: PhaserParams = PhaserParams {
        lfo: Lfo {
            rate_hz: 0.5,
            sync: None,
            wave: LfoWave::Sine,
            phase_deg: 90.0,
        },
        depth_oct: 2.0,
        center_hz: 800.0,
        feedback: 0.3,
        stages: 4,
        mix: 0.5,
    };

    pub fn validate(&self) -> Result<(), &'static str> {
        self.lfo.validate()?;
        if !within(self.depth_oct, PHASER_DEPTH_RANGE) {
            return Err("depth fuera de rango (0 a 4 octavas)");
        }
        if !within(self.center_hz, PHASER_CENTER_RANGE) {
            return Err("center fuera de rango (50 a 10000 Hz)");
        }
        if !PHASER_STAGES.contains(&self.stages) {
            return Err("stages: 2, 4, 6, 8 o 12");
        }
        check_feedback(self.feedback)?;
        check_mix(self.mix)
    }
}

/// Chorus ([`crate::Fx::Chorus`]).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ChorusParams {
    pub lfo: Lfo,
    /// Lo que barre el LFO por encima del retardo base, en ms.
    pub depth_ms: f32,
    /// Retardo base, en ms.
    pub delay_ms: f32,
    /// Voces (de 1 a 4), cada una con su LFO desfasado.
    pub voices: u8,
    pub mix: f32,
}

impl ChorusParams {
    pub const DEFAULT: ChorusParams = ChorusParams {
        lfo: Lfo {
            rate_hz: 0.8,
            sync: None,
            wave: LfoWave::Sine,
            phase_deg: 90.0,
        },
        depth_ms: 3.0,
        delay_ms: 15.0,
        voices: 2,
        mix: 0.5,
    };

    pub fn validate(&self) -> Result<(), &'static str> {
        self.lfo.validate()?;
        if !within(self.depth_ms, CHORUS_DEPTH_RANGE) {
            return Err("depth fuera de rango (0 a 10 ms)");
        }
        if !within(self.delay_ms, CHORUS_DELAY_RANGE) {
            return Err("delay fuera de rango (10 a 30 ms)");
        }
        if !(1..=MAX_CHORUS_VOICES).contains(&self.voices) {
            return Err("voices fuera de rango (1 a 4)");
        }
        check_mix(self.mix)
    }
}

/// Dónde está la canción al empezar un trozo (para los LFO).
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct LfoAt {
    pub steps: f64,
    pub seconds: f64,
    pub frames_per_step: f64,
}

/// Lo que puede apartarse (en ciclos) la fase de un LFO con `sync` de la
/// que le toca por la posición de la canción antes de volver a tomarla: un
/// salto del transporte, no el redondeo ni una rampa de tempo.
const SYNC_SLACK: f64 = 1e-4;

/// La fase de un LFO en el hilo de audio. Avanza en trozos de
/// [`LFO_BLOCK`] frames que siguen de un bloque de audio al siguiente: el
/// resultado no depende de en cuántos bloques se calcule.
#[derive(Debug, Clone, Copy)]
struct LfoRun {
    /// Fase (en ciclos) al empezar el trozo en curso.
    seg: f64,
    /// Frames hechos del trozo en curso (0: hay que empezar otro).
    pos: usize,
    /// Ciclos por frame.
    inc: f64,
    /// Ya tiene su fase de la canción.
    anchored: bool,
    /// Pasos de la canción en el último `tick` con `sync` (si no avanzan,
    /// el transporte está parado y el LFO sigue solo).
    last_steps: f64,
}

impl LfoRun {
    fn new(lfo: &Lfo, sample_rate: u32) -> Self {
        LfoRun {
            seg: 0.0,
            pos: 0,
            inc: lfo.rate_hz as f64 / sample_rate as f64,
            anchored: false,
            last_steps: f64::NAN,
        }
    }

    /// Al empezar un bloque de audio: con `sync`, la velocidad sale del
    /// tempo y la fase, de la posición de la canción si es la primera vez o
    /// si se ha apartado (un salto); sin él, la fase sale de los segundos de
    /// la canción la primera vez y luego avanza sola.
    fn tick(&mut self, lfo: &Lfo, at: &LfoAt) {
        match lfo.sync {
            Some(sync) => {
                self.inc = 1.0 / (sync.steps() * at.frames_per_step.max(1e-9));
                let moving = at.steps != self.last_steps;
                self.last_steps = at.steps;
                let want = sync.phase_at(at.steps);
                let now = self.seg + self.inc * self.pos as f64;
                let off = (now - want).rem_euclid(1.0);
                if !self.anchored || (moving && off.min(1.0 - off) > SYNC_SLACK) {
                    self.seg = want;
                    self.pos = 0;
                    self.anchored = true;
                }
            }
            None => {
                if !self.anchored {
                    self.seg = (at.seconds.max(0.0) * lfo.rate_hz as f64).rem_euclid(1.0);
                    self.pos = 0;
                    self.anchored = true;
                }
            }
        }
    }

    /// Fase al principio y al final del trozo en curso.
    fn segment(&self) -> (f64, f64) {
        (self.seg, self.seg + self.inc * LFO_BLOCK as f64)
    }

    /// Dónde va el frame que toca dentro de su trozo (de 0 a 1, el último
    /// en 1), y avanza.
    #[inline]
    fn step(&mut self) -> f32 {
        self.pos += 1;
        let t = self.pos as f32 / LFO_BLOCK as f32;
        if self.pos == LFO_BLOCK {
            self.pos = 0;
            self.seg = (self.seg + self.inc * LFO_BLOCK as f64).rem_euclid(1.0);
        }
        t
    }
}

/// Un parámetro que se acerca a su destino muestra a muestra.
#[derive(Debug, Clone, Copy)]
struct Smooth {
    now: f32,
    target: f32,
}

impl Smooth {
    fn new(v: f32) -> Self {
        Smooth { now: v, target: v }
    }

    #[inline]
    fn step(&mut self, k: f32) -> f32 {
        if self.now != self.target {
            self.now += (self.target - self.now) * k;
            if (self.target - self.now).abs() <= 1e-6 * self.target.abs().max(1e-3) {
                self.now = self.target;
            }
        }
        self.now
    }

    fn snap(&mut self) {
        self.now = self.target;
    }
}

/// Coeficiente por muestra de [`SMOOTHING_SECONDS`].
fn smoothing(sample_rate: u32) -> f32 {
    1.0 - (-1.0 / (SMOOTHING_SECONDS * sample_rate as f32)).exp()
}

/// Recorte suave de la realimentación: lineal hasta 1, y después tiende a
/// 2 sin pasar (con la misma pendiente en 1: sin esquina).
#[inline]
fn soft(v: f32) -> f32 {
    let a = v.abs();
    if a <= 1.0 {
        v
    } else {
        v.signum() * (2.0 - 1.0 / a)
    }
}

/// Lo minúsculo, a cero: sin denormales en las memorias.
#[inline]
fn flush(v: f32) -> f32 {
    if v.abs() < 1e-25 { 0.0 } else { v }
}

/// Interpolación de Hermite de 4 puntos (`t` entre `x0` y `x1`).
#[inline]
fn hermite(xm1: f32, x0: f32, x1: f32, x2: f32, t: f32) -> f32 {
    let c1 = 0.5 * (x1 - xm1);
    let c2 = xm1 - 2.5 * x0 + 2.0 * x1 - 0.5 * x2;
    let c3 = 0.5 * (x2 - xm1) + 1.5 * (x0 - x1);
    ((c3 * t + c2) * t + c1) * t + x0
}

/// Las líneas de retardo de una cadena de efectos: una por ranura, de
/// [`LINE_FRAMES`] frames estéreo. Se reservan fuera del hilo de audio.
pub(crate) struct Lines {
    buf: Box<[f32]>,
}

impl Lines {
    pub fn new() -> Self {
        Lines {
            buf: vec![0.0; crate::fx::MAX_FX * LINE_FRAMES * 2].into_boxed_slice(),
        }
    }

    /// La línea de la ranura `slot`.
    pub fn slot(&mut self, slot: usize) -> &mut [f32] {
        &mut self.buf[slot * LINE_FRAMES * 2..(slot + 1) * LINE_FRAMES * 2]
    }
}

impl std::fmt::Debug for Lines {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Lines")
    }
}

/// Lee la línea `line` en el canal `ch`, `delay` frames por detrás de
/// `write`.
#[inline]
fn read(line: &[f32], write: usize, ch: usize, delay: f32) -> f32 {
    let delay = delay.max(MIN_DELAY_FRAMES);
    let whole = delay.floor();
    let t = 1.0 - (delay - whole);
    // La posición es write - delay = (write - whole - 1) + t.
    let i = write.wrapping_sub(whole as usize + 1);
    let at = |k: usize| line[((i.wrapping_add(k)) & LINE_MASK) * 2 + ch];
    hermite(at(usize::MAX), at(0), at(1), at(2), t)
}

/// Lo común del estado de un efecto con línea o con memoria: cuánto lleva
/// en silencio y el fundido del pánico.
#[derive(Debug, Clone, Copy)]
struct Tail {
    /// Frames seguidos en silencio (entrada y salida).
    quiet: u32,
    /// Los que hacen falta para darlo por callado.
    hold: u32,
    /// La memoria tiene algo que no es cero.
    dirty: bool,
    /// Ganancia de la parte mojada (1, o bajando con el pánico).
    hush: f32,
    hush_step: f32,
    hushing: bool,
}

impl Tail {
    fn new(hold: u32, sample_rate: u32) -> Self {
        Tail {
            quiet: hold,
            hold,
            dirty: false,
            hush: 1.0,
            hush_step: 1.0 / (HUSH_SECONDS * sample_rate as f32).max(1.0),
            hushing: false,
        }
    }

    fn ringing(&self) -> bool {
        self.quiet < self.hold && !(self.hushing && self.hush == 0.0)
    }

    /// Tras un trozo de `frames` con este pico (de entrada y salida).
    fn after(&mut self, frames: usize, peak: f32) {
        if peak < QUIET {
            self.quiet = self.quiet.saturating_add(frames as u32);
        } else {
            self.quiet = 0;
            self.dirty = true;
        }
    }

    #[inline]
    fn wet_gain(&mut self) -> f32 {
        if self.hushing {
            self.hush = (self.hush - self.hush_step).max(0.0);
        }
        self.hush
    }

    /// El fundido del pánico ha llegado a cero: la memoria ya no suena.
    fn hushed(&self) -> bool {
        self.hushing && self.hush == 0.0
    }

    fn rest(&mut self) {
        self.quiet = self.hold;
        self.hush = 1.0;
        self.hushing = false;
    }
}

/// Estado del flanger y del chorus (la línea va aparte, en [`Lines`]).
#[derive(Debug, Clone, Copy)]
pub(crate) struct DelayState {
    kind: DelayKind,
    lfo_params: Lfo,
    lfo: LfoRun,
    depth: Smooth,
    delay: Smooth,
    feedback: Smooth,
    mix: Smooth,
    phase: Smooth,
    invert: bool,
    voices: u8,
    write: usize,
    k: f32,
    frames_per_ms: f32,
    /// El LFO de cada canal y voz al principio y al final del trozo en
    /// curso.
    ends: [[(f32, f32); MAX_CHORUS_VOICES as usize]; 2],
    tail: Tail,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DelayKind {
    Flanger,
    Chorus,
}

impl DelayState {
    fn with(
        kind: DelayKind,
        lfo: Lfo,
        (depth, delay, feedback, mix): (f32, f32, f32, f32),
        invert: bool,
        voices: u8,
        sample_rate: u32,
    ) -> Self {
        let frames_per_ms = sample_rate as f32 / 1000.0;
        let max_ms = match kind {
            DelayKind::Flanger => FLANGER_DELAY_RANGE.1 + FLANGER_DEPTH_RANGE.1,
            DelayKind::Chorus => CHORUS_DELAY_RANGE.1 + CHORUS_DEPTH_RANGE.1,
        };
        // Callado cuando lleva en silencio lo que dura la línea entera.
        let hold = (max_ms * frames_per_ms) as u32 + 4;
        DelayState {
            kind,
            lfo_params: lfo,
            lfo: LfoRun::new(&lfo, sample_rate),
            depth: Smooth::new(depth),
            delay: Smooth::new(delay),
            feedback: Smooth::new(feedback),
            mix: Smooth::new(mix),
            phase: Smooth::new(lfo.phase_deg),
            invert,
            voices,
            write: 0,
            k: smoothing(sample_rate),
            frames_per_ms,
            ends: [[(0.0, 0.0); MAX_CHORUS_VOICES as usize]; 2],
            tail: Tail::new(hold, sample_rate),
        }
    }

    pub fn flanger(p: &FlangerParams, sample_rate: u32) -> Self {
        Self::with(
            DelayKind::Flanger,
            p.lfo,
            (p.depth_ms, p.delay_ms, p.feedback, p.mix),
            p.invert,
            1,
            sample_rate,
        )
    }

    pub fn chorus(p: &ChorusParams, sample_rate: u32) -> Self {
        Self::with(
            DelayKind::Chorus,
            p.lfo,
            (p.depth_ms, p.delay_ms, 0.0, p.mix),
            false,
            p.voices,
            sample_rate,
        )
    }

    /// El mismo efecto con los valores nuevos de `next` (al entrar otro
    /// patrón): la línea, la fase y el silencio siguen; los parámetros van
    /// hacia los nuevos suavizados.
    pub fn take_over(&mut self, next: &DelayState) {
        if self.kind != next.kind || self.k != next.k {
            *self = *next;
            return;
        }
        self.depth.target = next.depth.target;
        self.delay.target = next.delay.target;
        self.feedback.target = next.feedback.target;
        self.mix.target = next.mix.target;
        self.phase.target = next.phase.target;
        self.invert = next.invert;
        self.voices = next.voices;
        let sync_changed = self.lfo_params.sync != next.lfo_params.sync;
        self.lfo_params = next.lfo_params;
        self.lfo.inc = next.lfo.inc;
        if sync_changed {
            self.lfo.anchored = self.lfo_params.sync.is_none() && self.lfo.anchored;
            self.lfo.last_steps = f64::NAN;
        }
    }

    pub fn same_kind(&self, other: &DelayState) -> bool {
        self.kind == other.kind
    }

    /// Parámetro automatizable `index` (ver [`crate::Fx::params`]).
    pub fn set_param(&mut self, index: usize, value: f32, sample_rate: u32) {
        let (rate, depth, delay, feedback, mix, phase) = match self.kind {
            DelayKind::Flanger => (0, 1, 2, 3, 4, 5),
            DelayKind::Chorus => (0, 1, 2, usize::MAX, 3, 4),
        };
        match index {
            i if i == rate => {
                self.lfo_params.rate_hz = value;
                if self.lfo_params.sync.is_none() {
                    self.lfo.inc = value as f64 / sample_rate as f64;
                }
            }
            i if i == depth => self.depth.target = value,
            i if i == delay => self.delay.target = value,
            i if i == feedback => self.feedback.target = value,
            i if i == mix => self.mix.target = value,
            i if i == phase => self.phase.target = value,
            _ => {}
        }
    }

    pub fn tick(&mut self, at: &LfoAt) {
        self.lfo.tick(&self.lfo_params, at);
    }

    pub fn ringing(&self) -> bool {
        self.tail.ringing()
    }

    pub fn hush(&mut self) {
        if self.tail.ringing() {
            self.tail.hushing = true;
        }
    }

    /// Silencio: la línea a cero (si tenía algo), los parámetros en su
    /// destino y el LFO vuelve a tomar su fase de la canción.
    pub fn reset(&mut self, line: &mut [f32]) {
        if self.tail.dirty {
            line.fill(0.0);
            self.tail.dirty = false;
        }
        self.tail.rest();
        for s in [
            &mut self.depth,
            &mut self.delay,
            &mut self.feedback,
            &mut self.mix,
            &mut self.phase,
        ] {
            s.snap();
        }
        if self.lfo_params.sync.is_none() {
            self.lfo.anchored = false;
        }
    }

    /// Procesa `buf` (estéreo entrelazado) en su sitio con su línea.
    pub fn process(&mut self, buf: &mut [f32], line: &mut [f32]) {
        let k = self.k;
        let voices = self.voices.max(1) as usize;
        let mut peak = 0.0f32;
        for frame in buf.as_chunks_mut::<2>().0 {
            if self.lfo.pos == 0 {
                // Un trozo nuevo: el LFO de cada canal y voz en sus extremos.
                let (start, end) = self.lfo.segment();
                let offset = self.phase.now as f64 / 360.0;
                let wave = self.lfo_params.wave;
                for (ch, row) in self.ends.iter_mut().enumerate() {
                    for (v, slot) in row.iter_mut().enumerate().take(voices) {
                        let shift = ch as f64 * offset + v as f64 / voices as f64;
                        *slot = (wave.at(start + shift), wave.at(end + shift));
                    }
                }
            }
            let t = self.lfo.step();
            let depth = self.depth.step(k) * self.frames_per_ms;
            let delay = self.delay.step(k) * self.frames_per_ms;
            let feedback = self.feedback.step(k);
            let mix = self.mix.step(k);
            self.phase.step(k);
            let gain = self.tail.wet_gain();
            let w = self.write & LINE_MASK;
            for (ch, x) in frame.iter_mut().enumerate() {
                let input = *x;
                let mut wet = 0.0;
                for &(a, b) in &self.ends[ch][..voices] {
                    let l = a + (b - a) * t;
                    wet += read(line, w, ch, delay + depth * l);
                }
                wet *= gain / voices as f32;
                line[w * 2 + ch] = flush(input + soft(feedback * wet));
                let wet = if self.invert { -wet } else { wet };
                if mix != 0.0 {
                    *x = input + mix * (wet - input);
                }
                peak = peak.max(input.abs()).max(wet.abs());
            }
            self.write = self.write.wrapping_add(1);
        }
        self.tail.after(buf.len() / 2, peak);
        if self.tail.hushed() {
            // Tras el pánico, la línea vacía: el siguiente golpe suena con
            // su parte mojada desde cero.
            self.reset(line);
        }
    }
}

/// Estado del phaser: la memoria de cada etapa y canal.
#[derive(Debug, Clone, Copy)]
pub(crate) struct PhaserState {
    lfo_params: Lfo,
    lfo: LfoRun,
    depth: Smooth,
    center: Smooth,
    feedback: Smooth,
    mix: Smooth,
    phase: Smooth,
    stages: u8,
    z: [[f32; 2]; MAX_STAGES],
    /// Salida de la última etapa (lo que vuelve con la realimentación).
    last: [f32; 2],
    /// Coeficiente de cada canal al principio y al final del trozo en
    /// curso del LFO.
    coef: [(f32, f32); 2],
    sample_rate: u32,
    k: f32,
    tail: Tail,
}

impl PhaserState {
    pub fn new(p: &PhaserParams, sample_rate: u32) -> Self {
        PhaserState {
            lfo_params: p.lfo,
            lfo: LfoRun::new(&p.lfo, sample_rate),
            depth: Smooth::new(p.depth_oct),
            center: Smooth::new(p.center_hz),
            feedback: Smooth::new(p.feedback),
            mix: Smooth::new(p.mix),
            phase: Smooth::new(p.lfo.phase_deg),
            stages: p.stages,
            z: [[0.0; 2]; MAX_STAGES],
            last: [0.0; 2],
            coef: [(0.0, 0.0); 2],
            sample_rate,
            k: smoothing(sample_rate),
            tail: Tail::new(
                (PHASER_HOLD_SECONDS * sample_rate as f32) as u32,
                sample_rate,
            ),
        }
    }

    pub fn take_over(&mut self, next: &PhaserState) {
        if self.sample_rate != next.sample_rate {
            *self = *next;
            return;
        }
        self.depth.target = next.depth.target;
        self.center.target = next.center.target;
        self.feedback.target = next.feedback.target;
        self.mix.target = next.mix.target;
        self.phase.target = next.phase.target;
        if self.stages != next.stages {
            // Las etapas nuevas empiezan sin memoria.
            for z in &mut self.z[self.stages as usize..] {
                *z = [0.0; 2];
            }
            self.stages = next.stages;
        }
        let sync_changed = self.lfo_params.sync != next.lfo_params.sync;
        self.lfo_params = next.lfo_params;
        self.lfo.inc = next.lfo.inc;
        if sync_changed {
            self.lfo.anchored = self.lfo_params.sync.is_none() && self.lfo.anchored;
            self.lfo.last_steps = f64::NAN;
        }
    }

    pub fn set_param(&mut self, index: usize, value: f32, sample_rate: u32) {
        match index {
            0 => {
                self.lfo_params.rate_hz = value;
                if self.lfo_params.sync.is_none() {
                    self.lfo.inc = value as f64 / sample_rate as f64;
                }
            }
            1 => self.depth.target = value,
            2 => self.center.target = value,
            3 => self.feedback.target = value,
            4 => self.mix.target = value,
            5 => self.phase.target = value,
            _ => {}
        }
    }

    pub fn tick(&mut self, at: &LfoAt) {
        self.lfo.tick(&self.lfo_params, at);
    }

    pub fn ringing(&self) -> bool {
        self.tail.ringing()
    }

    pub fn hush(&mut self) {
        if self.tail.ringing() {
            self.tail.hushing = true;
        }
    }

    pub fn reset(&mut self) {
        self.z = [[0.0; 2]; MAX_STAGES];
        self.last = [0.0; 2];
        self.tail.dirty = false;
        self.tail.rest();
        for s in [
            &mut self.depth,
            &mut self.center,
            &mut self.feedback,
            &mut self.mix,
            &mut self.phase,
        ] {
            s.snap();
        }
        if self.lfo_params.sync.is_none() {
            self.lfo.anchored = false;
        }
    }

    /// Coeficiente del paso-todo con el LFO en `lfo` (de 0 a 1):
    /// `(tan(πf/fs) − 1) / (tan(πf/fs) + 1)`, con `f` acotada por debajo
    /// de Nyquist (la tangente nunca llega a su polo).
    fn coefficient(&self, lfo: f32) -> f32 {
        let nyquist = 0.45 * self.sample_rate as f32;
        let f = self.center.now * (self.depth.now * (2.0 * lfo - 1.0)).exp2();
        let f = f.clamp(10.0, nyquist.max(10.0));
        let t = (PI * f / self.sample_rate as f32).tan();
        (t - 1.0) / (t + 1.0)
    }

    pub fn process(&mut self, buf: &mut [f32]) {
        let k = self.k;
        let stages = self.stages as usize;
        let mut peak = 0.0f32;
        for frame in buf.as_chunks_mut::<2>().0 {
            if self.lfo.pos == 0 {
                // Un trozo nuevo: los coeficientes en sus extremos.
                let (start, end) = self.lfo.segment();
                let offset = self.phase.now as f64 / 360.0;
                let wave = self.lfo_params.wave;
                self.coef = std::array::from_fn(|ch| {
                    let shift = ch as f64 * offset;
                    (
                        self.coefficient(wave.at(start + shift)),
                        self.coefficient(wave.at(end + shift)),
                    )
                });
            }
            let t = self.lfo.step();
            self.depth.step(k);
            self.center.step(k);
            self.phase.step(k);
            let feedback = self.feedback.step(k);
            let mix = self.mix.step(k);
            let gain = self.tail.wet_gain();
            for (ch, x) in frame.iter_mut().enumerate() {
                let input = *x;
                let (from, to) = self.coef[ch];
                let a = from + (to - from) * t;
                let mut v = soft(input + feedback * self.last[ch]);
                for z in &mut self.z[..stages] {
                    let y = a * v + z[ch];
                    z[ch] = flush(v - a * y);
                    v = y;
                }
                let wet = v * gain;
                self.last[ch] = flush(wet);
                if mix != 0.0 {
                    *x = input + mix * (wet - input);
                }
                peak = peak.max(input.abs()).max(wet.abs());
            }
        }
        if peak >= QUIET {
            self.tail.dirty = true;
        }
        self.tail.after(buf.len() / 2, peak);
        if self.tail.hushed() {
            self.reset();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sync_phase_is_exact_far_into_the_song() {
        let s = Sync { num: 3, den: 16 };
        // Un ciclo cada 3 pasos.
        assert_eq!(s.phase_at(0.0), 0.0);
        assert!((s.phase_at(1.5) - 0.5).abs() < 1e-12);
        let far = 3.0 * 1e12 + 1.5;
        assert!((s.phase_at(far) - 0.5).abs() < 1e-3);
        assert_eq!(Sync::per_bar(4).steps(), 4.0);
    }

    #[test]
    fn hermite_passes_through_the_samples() {
        assert_eq!(hermite(1.0, 2.0, 3.0, 4.0, 0.0), 2.0);
        assert!((hermite(1.0, 2.0, 3.0, 4.0, 0.5) - 2.5).abs() < 1e-6);
    }

    #[test]
    fn the_line_reads_whole_delays_exactly() {
        let mut line = vec![0.0f32; LINE_FRAMES * 2];
        for i in 0..100 {
            line[i * 2] = i as f32;
        }
        // Escribiendo en 100, 10 frames atrás es 90.
        assert!((read(&line, 100, 0, 10.0) - 90.0).abs() < 1e-4);
        assert!((read(&line, 100, 0, 10.5) - 89.5).abs() < 1e-4);
    }
}
