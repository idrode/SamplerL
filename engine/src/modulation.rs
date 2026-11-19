//! Modulación por golpe: cada vez que suena un corte o una nota, su conjunto
//! de modulación (*mod set*) arranca desde cero con sus envolventes, sus
//! LFO y sus pasos, de forma polifónica (cada voz con su estado).
//!
//! La modulación global (`ramp`, `lfo`, macros, lanes: [`crate::Curve`])
//! sigue el reloj de la canción; esta sigue el tiempo del golpe, desde que
//! nace, y su note-off.
//!
//! - Un **mod set** ([`ModSet`]) tiene una cadena por dominio de destino
//!   ([`DOMAINS`]: volumen, pan, pitch y corte del filtro).
//! - Una **cadena** ([`Chain`]) es un valor inicial y hasta [`MAX_DEVICES`]
//!   dispositivos en orden; cada uno da un valor que su operador ([`Op`])
//!   combina con lo acumulado.
//! - Los sets de la canción van en una **tabla** de capacidad fija
//!   ([`ModTable`], [`MAX_MOD_SETS`]) que entra entera por el lote al
//!   compás, como las lanes; los patrones apuntan a un set por su índice.
//! - Cada voz lleva su estado ([`ModVoice`], de tamaño fijo) y el número de
//!   la tabla con la que nació: si entra otra tabla mientras suena, la voz
//!   sigue con la suya ([`ModTables`] guarda las anteriores hasta que nadie
//!   las usa y las devuelve por la cola de basura; nada se libera en el
//!   hilo de audio).
//!
//! Los dominios, normalizados: el volumen multiplica el del golpe (1, igual;
//! de 0 a 4); el pan se suma al del golpe (de -1 a 1); el pitch es una
//! fracción de `range` semitonos que se suma a la nota; el corte va de 0
//! (20 Hz) a 1 (20 kHz) en escala logarítmica (en un sintetizador, 1 es su
//! propio corte y cada 0,1 menos, una octava abajo).
//!
//! Se evalúa a tasa de control: cada [`CONTROL_FRAMES`] frames desde que
//! nace la voz (así no depende del tamaño de bloque) y en línea recta entre
//! medias.

use crate::automation::{Curve, CurveEnv};

/// Sets que caben en la tabla.
pub const MAX_MOD_SETS: usize = 16;
/// Dispositivos de una cadena, como mucho.
pub const MAX_DEVICES: usize = 8;
/// Dominios de destino.
pub const DOMAINS: usize = 4;
pub const VOL: usize = 0;
pub const PAN: usize = 1;
pub const PITCH: usize = 2;
pub const CUTOFF: usize = 3;
/// Sus nombres (las claves de `mod()` en Lua).
pub const DOMAIN_NAMES: [&str; DOMAINS] = ["vol", "pan", "pitch", "cutoff"];
/// Frames entre dos evaluaciones de las cadenas.
pub const CONTROL_FRAMES: u32 = crate::engine::MOD_BLOCK as u32;
/// El rango del pitch, en semitonos, como mucho.
pub const MAX_PITCH_RANGE: f32 = 48.0;
/// Lo más que dura una fase de una envolvente, en ms.
pub const MAX_ENV_MS: f32 = 60_000.0;
/// Lo más que escala un dispositivo (`amount`).
pub const MAX_AMOUNT: f32 = 64.0;
/// Volumen máximo que deja la modulación (como `:gain()`).
pub const MAX_MOD_GAIN: f32 = crate::sequencer::MAX_GAIN;
/// Tablas anteriores que se guardan mientras sus voces suenan.
pub const OLD_TABLES: usize = 3;
/// Frecuencias del corte normalizado: 0 es esta, 1 es mil veces más.
pub const CUTOFF_LOW_HZ: f32 = 20.0;

/// El corte normalizado (`v` de 0 a 1) en Hz: `20 · 1000^v`.
pub fn cutoff_hz(v: f32) -> f32 {
    CUTOFF_LOW_HZ * 1000f32.powf(v.clamp(0.0, 1.0))
}

/// Lo contrario: los Hz (de 20 a 20000) en el corte normalizado.
pub fn cutoff_norm(hz: f32) -> f32 {
    ((hz.max(CUTOFF_LOW_HZ) / CUTOFF_LOW_HZ).ln() / 1000f32.ln()).clamp(0.0, 1.0)
}

/// Lo que mira un parámetro con curva al evaluarse: la posición de la
/// canción y lo que se mueve en vivo (macros, lanes).
#[derive(Clone, Copy)]
pub struct ModAt<'a> {
    pub steps: f64,
    pub seconds: f64,
    pub frames_per_step: f64,
    pub env: &'a CurveEnv<'a>,
}

impl ModAt<'static> {
    /// Sin canción (la vista de la interfaz, los tests): todo en 0.
    pub const IDLE: ModAt<'static> = ModAt {
        steps: 0.0,
        seconds: 0.0,
        frames_per_step: 6000.0,
        env: &CurveEnv::IDLE,
    };
}

impl ModAt<'_> {
    /// La misma, `frames` frames más tarde.
    fn later(&self, frames: f64, sample_rate: u32) -> Self {
        ModAt {
            steps: self.steps + frames / self.frames_per_step.max(1e-9),
            seconds: self.seconds + frames / sample_rate as f64,
            ..*self
        }
    }
}

/// Un parámetro de un dispositivo: un número o una curva de la canción
/// (`ramp`, `lfo`, una macro, una lane), que se mira en cada punto de
/// control.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ModValue {
    pub value: f32,
    pub curve: Option<Curve>,
}

impl ModValue {
    pub const fn fixed(value: f32) -> Self {
        ModValue { value, curve: None }
    }

    /// Su valor ahora.
    #[inline]
    pub fn at(&self, at: &ModAt) -> f32 {
        match self.curve {
            None => self.value,
            Some(c) => c.value_in(at.steps, at.seconds, 0.0, at.env, false),
        }
    }

    /// Que no se salga de `lo..=hi` (con curva, ninguno de sus extremos).
    fn check(&self, (lo, hi): (f32, f32), what: &'static str) -> Result<(), &'static str> {
        let inside = |v: f32| v.is_finite() && (lo..=hi).contains(&v);
        let ok = match self.curve {
            None => inside(self.value),
            Some(c) => {
                let (a, b) = c.bounds();
                inside(a) && inside(b)
            }
        };
        if ok { Ok(()) } else { Err(what) }
    }
}

/// Cómo combina un dispositivo su valor con lo acumulado.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Op {
    Add,
    Sub,
    Mul,
    Div,
}

impl Op {
    pub const ALL: [(Op, &'static str); 4] = [
        (Op::Add, "add"),
        (Op::Sub, "sub"),
        (Op::Mul, "mul"),
        (Op::Div, "div"),
    ];

    pub fn from_name(name: &str) -> Option<Op> {
        Self::ALL.iter().find(|(_, n)| *n == name).map(|(o, _)| *o)
    }

    pub fn name(self) -> &'static str {
        Self::ALL
            .iter()
            .find(|(o, _)| *o == self)
            .map_or("add", |(_, n)| n)
    }

    /// El de un dominio si el dispositivo no dice otro: multiplicar en el
    /// volumen y el corte, sumar en el pan y el pitch.
    pub fn default_for(domain: usize) -> Op {
        match domain {
            VOL | CUTOFF => Op::Mul,
            _ => Op::Add,
        }
    }

    #[inline]
    fn apply(self, acc: f32, x: f32) -> f32 {
        match self {
            Op::Add => acc + x,
            Op::Sub => acc - x,
            Op::Mul => acc * x,
            // Entre casi cero, nada (ni infinitos ni NaN).
            Op::Div if x.abs() < 1e-6 => acc,
            Op::Div => acc / x,
        }
    }
}

/// Forma de un tramo de envolvente: `c` de -1 a 1; 0 es la recta, positivo
/// va deprisa al principio (exponencial, como una caída natural), negativo
/// despacio al principio. `u` de 0 a 1.
#[inline]
pub fn shape(u: f32, c: f32) -> f32 {
    let u = u.clamp(0.0, 1.0);
    if c.abs() < 1e-3 {
        return u;
    }
    let k = 6.0 * c;
    (1.0 - (-k * u).exp()) / (1.0 - (-k).exp())
}

/// AHDSR: sube en `attack`, se queda arriba `hold`, baja hasta `sustain`
/// en `decay` y ahí se queda hasta el note-off; desde ahí baja a 0 en
/// `release`. Tiempos en ms; `curves`, la forma del ataque, del decay y
/// del release (ver [`shape`]).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Ahdsr {
    pub attack: ModValue,
    pub hold: ModValue,
    pub decay: ModValue,
    pub sustain: ModValue,
    pub release: ModValue,
    pub curves: [f32; 3],
}

impl Ahdsr {
    pub const DEFAULT: Ahdsr = Ahdsr {
        attack: ModValue::fixed(5.0),
        hold: ModValue::fixed(0.0),
        decay: ModValue::fixed(200.0),
        sustain: ModValue::fixed(0.5),
        release: ModValue::fixed(150.0),
        curves: [0.0, 0.5, 0.5],
    };

    fn validate(&self) -> Result<(), &'static str> {
        let ms = (0.0, MAX_ENV_MS);
        self.attack
            .check(ms, "attack fuera de rango (0 a 60000 ms)")?;
        self.hold.check(ms, "hold fuera de rango (0 a 60000 ms)")?;
        self.decay
            .check(ms, "decay fuera de rango (0 a 60000 ms)")?;
        self.release
            .check(ms, "release fuera de rango (0 a 60000 ms)")?;
        self.sustain
            .check((0.0, 1.0), "sustain fuera de rango (0 a 1)")?;
        if self
            .curves
            .iter()
            .any(|c| !(c.is_finite() && (-1.0..=1.0).contains(c)))
        {
            return Err("las curvas de la envolvente van de -1 a 1");
        }
        Ok(())
    }

    /// El nivel con la puerta abierta, a los `t` ms del golpe.
    fn gate_level(&self, t: f32, at: &ModAt) -> f32 {
        let a = self.attack.at(at);
        let h = self.hold.at(at);
        let d = self.decay.at(at);
        let s = self.sustain.at(at).clamp(0.0, 1.0);
        if t < a {
            return shape(t / a, self.curves[0]);
        }
        let t = t - a;
        if t < h {
            return 1.0;
        }
        let t = t - h;
        if t < d {
            return 1.0 - (1.0 - s) * shape(t / d, self.curves[1]);
        }
        s
    }

    /// El nivel a los `t` ms del golpe con el note-off a los `off` ms.
    pub fn level(&self, t: f32, off: Option<f32>, at: &ModAt) -> f32 {
        match off {
            Some(off) if t >= off => {
                let from = self.gate_level(off, at);
                let r = self.release.at(at);
                if r <= 0.0 {
                    return 0.0;
                }
                from * (1.0 - shape((t - off) / r, self.curves[2]))
            }
            _ => self.gate_level(t, at),
        }
    }
}

/// Forma de un LFO por golpe.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModWave {
    Sine,
    Triangle,
    /// Diente de sierra que sube.
    Saw,
    Square,
    /// Un valor al azar en cada ciclo (sample & hold), el mismo en cada
    /// render.
    Random,
}

impl ModWave {
    pub const ALL: [(ModWave, &'static str); 5] = [
        (ModWave::Sine, "sine"),
        (ModWave::Triangle, "tri"),
        (ModWave::Saw, "saw"),
        (ModWave::Square, "square"),
        (ModWave::Random, "random"),
    ];

    pub fn from_name(name: &str) -> Option<ModWave> {
        Self::ALL.iter().find(|(_, n)| *n == name).map(|(w, _)| *w)
    }

    pub fn name(self) -> &'static str {
        Self::ALL
            .iter()
            .find(|(w, _)| *w == self)
            .map_or("sine", |(_, n)| n)
    }

    /// De -1 a 1 en la fase `phase` (ciclos); `seed`, el azar del golpe.
    fn at(self, phase: f64, seed: u64) -> f32 {
        let p = phase.rem_euclid(1.0) as f32;
        match self {
            ModWave::Sine => (std::f32::consts::TAU * p).sin(),
            ModWave::Triangle => 1.0 - 4.0 * (p - 0.25 - (p - 0.25).round()).abs(),
            ModWave::Saw => 2.0 * p - 1.0,
            ModWave::Square => {
                if p < 0.5 {
                    1.0
                } else {
                    -1.0
                }
            }
            ModWave::Random => {
                let cycle = phase.floor() as i64 as u64;
                let mut x = seed ^ cycle.wrapping_mul(0x9e37_79b9_7f4a_7c15);
                x ^= x >> 33;
                x = x.wrapping_mul(0xff51_afd7_ed55_8ccd);
                x ^= x >> 33;
                (x >> 40) as f32 / (1u64 << 23) as f32 - 1.0
            }
        }
    }
}

/// LFO por golpe: su fase vuelve a empezar en cada golpe (salvo `free`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ModLfo {
    pub wave: ModWave,
    /// Ciclos por segundo (sin `sync`).
    pub rate: ModValue,
    /// Atado al compás: un ciclo dura lo que dice (manda sobre `rate`).
    pub sync: Option<crate::modfx::Sync>,
    /// Fase al empezar, en grados.
    pub phase: f32,
    /// Ms en silencio (en 0) antes de empezar.
    pub delay: f32,
    /// Libre: la fase sigue la canción (no vuelve a empezar con el golpe).
    pub free: bool,
}

impl ModLfo {
    pub const DEFAULT: ModLfo = ModLfo {
        wave: ModWave::Sine,
        rate: ModValue::fixed(4.0),
        sync: None,
        phase: 0.0,
        delay: 0.0,
        free: false,
    };

    fn validate(&self) -> Result<(), &'static str> {
        self.rate
            .check((0.01, 50.0), "rate fuera de rango (0.01 a 50 Hz)")?;
        if let Some(s) = self.sync {
            s.validate()?;
        }
        if !(self.phase.is_finite() && (0.0..=360.0).contains(&self.phase)) {
            return Err("phase fuera de rango (0 a 360 grados)");
        }
        if !(self.delay.is_finite() && (0.0..=MAX_ENV_MS).contains(&self.delay)) {
            return Err("delay fuera de rango (0 a 60000 ms)");
        }
        Ok(())
    }

    /// De -1 a 1 en el punto de control `t` (frames desde el golpe; se
    /// llama con `t` = 0, 32, 64...). `state.phase` lleva la fase.
    fn value(&self, t: u32, state: &mut DevState, seed: u64, at: &ModAt, sr: u32) -> f32 {
        let start = self.phase as f64 / 360.0;
        if self.free {
            let phase = match self.sync {
                Some(sync) => sync.phase_at(at.steps),
                None => at.seconds.max(0.0) * self.rate.at(at) as f64,
            };
            return self.wave.at(start + phase, seed);
        }
        let delay = (self.delay as f64 / 1000.0 * sr as f64) as u32;
        if t < delay {
            return 0.0;
        }
        let since = (t - delay) as f64;
        let phase = match (self.sync, self.rate.curve) {
            // Atado al compás: de la cuenta de frames, sin acumular.
            (Some(sync), _) => since / (sync.steps() * at.frames_per_step.max(1e-9)),
            // A una velocidad fija, igual.
            (None, None) => since * self.rate.value as f64 / sr as f64,
            // Con la velocidad moviéndose, acumulada punto a punto.
            (None, Some(_)) => {
                if !state.started {
                    state.started = true;
                    state.phase = 0.0;
                } else {
                    state.phase += self.rate.at(at) as f64 * CONTROL_FRAMES as f64 / sr as f64;
                }
                state.phase
            }
        };
        self.wave.at(start + phase, seed)
    }
}

/// Cómo va un fader de un valor a otro.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FaderScale {
    Linear,
    /// Despacio al principio.
    Exp,
    /// Deprisa al principio.
    Log,
}

impl FaderScale {
    pub const ALL: [(FaderScale, &'static str); 3] = [
        (FaderScale::Linear, "linear"),
        (FaderScale::Exp, "exp"),
        (FaderScale::Log, "log"),
    ];

    pub fn from_name(name: &str) -> Option<FaderScale> {
        Self::ALL.iter().find(|(_, n)| *n == name).map(|(s, _)| *s)
    }

    fn shape(self, u: f32) -> f32 {
        match self {
            FaderScale::Linear => u,
            FaderScale::Exp => shape(u, -0.7),
            FaderScale::Log => shape(u, 0.7),
        }
    }
}

/// Fader: de `from` a `to` en `time` ms, tras `delay` ms.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Fader {
    pub from: f32,
    pub to: f32,
    pub time: f32,
    pub delay: f32,
    pub scale: FaderScale,
}

impl Fader {
    fn validate(&self) -> Result<(), &'static str> {
        let ok = |v: f32, (lo, hi): (f32, f32)| v.is_finite() && (lo..=hi).contains(&v);
        if !ok(self.from, (-MAX_AMOUNT, MAX_AMOUNT)) || !ok(self.to, (-MAX_AMOUNT, MAX_AMOUNT)) {
            return Err("from y to van de -64 a 64");
        }
        if !ok(self.time, (0.0, MAX_ENV_MS)) || !ok(self.delay, (0.0, MAX_ENV_MS)) {
            return Err("time y delay van de 0 a 60000 ms");
        }
        Ok(())
    }

    fn value(&self, t_ms: f32) -> f32 {
        let u = if t_ms <= self.delay {
            0.0
        } else if self.time <= 0.0 {
            1.0
        } else {
            ((t_ms - self.delay) / self.time).min(1.0)
        };
        self.from + (self.to - self.from) * self.scale.shape(u)
    }
}

/// Qué es un dispositivo, con sus parámetros.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum DeviceKind {
    Ahdsr(Ahdsr),
    Lfo(ModLfo),
    Fader(Fader),
    /// Un valor fijo (o una curva de la canción).
    Operand(ModValue),
}

impl DeviceKind {
    /// Su nombre en Lua.
    pub fn name(&self) -> &'static str {
        match self {
            DeviceKind::Ahdsr(_) => "ahdsr",
            DeviceKind::Lfo(_) => "lfo",
            DeviceKind::Fader(_) => "fader",
            DeviceKind::Operand(_) => "value",
        }
    }
}

/// Un dispositivo de una cadena.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Device {
    pub kind: DeviceKind,
    /// `None`: el del dominio ([`Op::default_for`]).
    pub op: Option<Op>,
    /// Lo que da de 0 a 1 pasa a ir de -1 a 1.
    pub bipolar: bool,
    /// Apagado: la cadena sigue como si no estuviera.
    pub bypass: bool,
    /// Lo que se escala su valor.
    pub amount: ModValue,
}

impl Device {
    pub fn new(kind: DeviceKind) -> Self {
        Device {
            kind,
            op: None,
            bipolar: false,
            bypass: false,
            amount: ModValue::fixed(1.0),
        }
    }

    pub fn validate(&self) -> Result<(), &'static str> {
        self.amount.check(
            (-MAX_AMOUNT, MAX_AMOUNT),
            "amount fuera de rango (-64 a 64)",
        )?;
        match &self.kind {
            DeviceKind::Ahdsr(a) => a.validate(),
            DeviceKind::Lfo(l) => l.validate(),
            DeviceKind::Fader(f) => f.validate(),
            DeviceKind::Operand(v) => {
                v.check((-MAX_AMOUNT, MAX_AMOUNT), "value fuera de rango (-64 a 64)")
            }
        }
    }
}

/// Lo que sabe la cadena del golpe al evaluarse.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct HitInfo {
    /// De 0 a 1: el volumen del golpe (`@`, la velocidad MIDI / 127); con
    /// accent, 1.
    pub velocity: f32,
    /// La nota (MIDI, con decimales): la del sintetizador, o 60 más el
    /// pitch de un corte.
    pub key: f32,
    /// Número del golpe (siembra el azar de los LFO aleatorios).
    pub born: u64,
}

impl HitInfo {
    pub const DEFAULT: HitInfo = HitInfo {
        velocity: 1.0,
        key: 60.0,
        born: 0,
    };
}

/// La cadena de un dominio: el valor inicial y los dispositivos en orden.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Chain {
    pub input: f32,
    pub devices: [Option<Device>; MAX_DEVICES],
}

impl Chain {
    /// Sin dispositivos, con el valor neutro del dominio.
    pub fn new(domain: usize) -> Self {
        Chain {
            input: neutral(domain),
            devices: [None; MAX_DEVICES],
        }
    }

    pub fn with_device(mut self, device: Device) -> Result<Self, &'static str> {
        let free = self
            .devices
            .iter()
            .position(Option::is_none)
            .ok_or("demasiados dispositivos (máximo 8 por cadena)")?;
        self.devices[free] = Some(device);
        Ok(self)
    }
}

/// El valor que no cambia nada en un dominio.
pub fn neutral(domain: usize) -> f32 {
    match domain {
        VOL | CUTOFF => 1.0,
        _ => 0.0,
    }
}

/// Un conjunto de modulación.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ModSet {
    pub chains: [Option<Chain>; DOMAINS],
    /// Semitonos del pitch con su valor en 1.
    pub pitch_range: f32,
}

impl Default for ModSet {
    fn default() -> Self {
        ModSet {
            chains: [None; DOMAINS],
            pitch_range: 12.0,
        }
    }
}

impl ModSet {
    pub fn validate(&self) -> Result<(), &'static str> {
        if !(self.pitch_range.is_finite() && (0.0..=MAX_PITCH_RANGE).contains(&self.pitch_range)) {
            return Err("range del pitch fuera de rango (0 a 48 semitonos)");
        }
        for chain in self.chains.iter().flatten() {
            if !chain.input.is_finite() || chain.input.abs() > MAX_AMOUNT {
                return Err("el valor inicial de una cadena va de -64 a 64");
            }
            for device in chain.devices.iter().flatten() {
                device.validate()?;
            }
        }
        Ok(())
    }

    /// Tiene cadena en el dominio `domain`.
    pub fn has(&self, domain: usize) -> bool {
        self.chains[domain].is_some()
    }

    /// El valor del dominio `domain` a los `t` frames del golpe, con el
    /// note-off a los `off` frames (si lo hay). Sin cadena, el neutro.
    /// Es la función de la vista de la interfaz y la del motor (que le da
    /// además el estado de la voz).
    #[allow(clippy::too_many_arguments)]
    pub fn value(
        &self,
        domain: usize,
        t: u32,
        off: Option<u32>,
        hit: &HitInfo,
        state: &mut [DevState; MAX_DEVICES],
        at: &ModAt,
        sample_rate: u32,
    ) -> f32 {
        let Some(chain) = &self.chains[domain] else {
            return neutral(domain);
        };
        let ms = |frames: u32| frames as f32 * 1000.0 / sample_rate as f32;
        let (t_ms, off_ms) = (ms(t), off.map(ms));
        let mut acc = chain.input;
        for (i, (device, state)) in chain.devices.iter().zip(state.iter_mut()).enumerate() {
            let Some(device) = device else { continue };
            if device.bypass {
                continue;
            }
            // De 0 a 1 (bipolar, de -1 a 1); el LFO, al revés; el fader y
            // el operando, sus valores tal cual.
            let polar = |r: f32| if device.bipolar { 2.0 * r - 1.0 } else { r };
            let raw = match &device.kind {
                DeviceKind::Ahdsr(a) => polar(a.level(t_ms, off_ms, at)),
                DeviceKind::Lfo(l) => {
                    let seed = hit.born ^ ((domain * MAX_DEVICES + i) as u64) << 56;
                    let x = l.value(t, state, seed, at, sample_rate);
                    if device.bipolar { x } else { 0.5 * (x + 1.0) }
                }
                DeviceKind::Fader(f) => f.value(t_ms),
                DeviceKind::Operand(v) => v.at(at),
            };
            let x = raw * device.amount.at(at);
            acc = device.op.unwrap_or(Op::default_for(domain)).apply(acc, x);
        }
        clamp_domain(domain, acc)
    }
}

/// El valor de un dominio dentro de su rango (lo no finito, al neutro).
fn clamp_domain(domain: usize, v: f32) -> f32 {
    if !v.is_finite() {
        return neutral(domain);
    }
    match domain {
        VOL => v.clamp(0.0, MAX_MOD_GAIN),
        PAN | PITCH => v.clamp(-1.0, 1.0),
        _ => v.clamp(0.0, 1.0),
    }
}

/// El estado de un dispositivo en una voz (la fase de un LFO, el valor de
/// un stepper al nacer...).
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct DevState {
    pub phase: f64,
    pub held: f32,
    pub started: bool,
}

/// La tabla de sets de la canción.
#[derive(Debug, Clone, PartialEq)]
pub struct ModTable {
    pub sets: [Option<ModSet>; MAX_MOD_SETS],
}

impl Default for ModTable {
    fn default() -> Self {
        ModTable {
            sets: [None; MAX_MOD_SETS],
        }
    }
}

impl ModTable {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn set(&mut self, index: usize, set: Option<ModSet>) -> Result<(), &'static str> {
        if index >= MAX_MOD_SETS {
            return Err("demasiados mod sets (máximo 16)");
        }
        if let Some(s) = &set {
            s.validate()?;
        }
        self.sets[index] = set;
        Ok(())
    }

    pub fn get(&self, index: usize) -> Option<&ModSet> {
        self.sets.get(index).and_then(Option::as_ref)
    }

    pub fn is_empty(&self) -> bool {
        self.sets.iter().all(Option::is_none)
    }
}

/// Las tablas del motor: la que suena (su número, `generation`) y las anteriores
/// que alguna voz aún usa.
#[derive(Default)]
pub struct ModTables {
    current: Option<Box<ModTable>>,
    generation: u32,
    old: [Option<(u32, Box<ModTable>)>; OLD_TABLES],
}

/// Sin tablas (un sintetizador fuera del motor).
pub static NO_MODS: ModTables = ModTables {
    current: None,
    generation: 0,
    old: [None, None, None],
};

impl ModTables {
    /// La tabla número `generation`, si sigue.
    #[inline]
    pub fn get(&self, generation: u32) -> Option<&ModTable> {
        if generation == self.generation {
            return self.current.as_deref();
        }
        self.old
            .iter()
            .flatten()
            .find(|(g, _)| *g == generation)
            .map(|(_, t)| &**t)
    }

    /// La que suena y su número.
    pub fn current(&self) -> (u32, Option<&ModTable>) {
        (self.generation, self.current.as_deref())
    }

    /// Entra `new` (al compás): la de antes pasa a las anteriores. Devuelve
    /// lo que hay que soltar fuera del hilo de audio (la más antigua, si no
    /// cabe: sus voces se quedan con su último valor).
    pub fn install(&mut self, new: Option<Box<ModTable>>) -> Option<Box<ModTable>> {
        let previous = std::mem::replace(&mut self.current, new);
        let generation = self.generation;
        self.generation = self.generation.wrapping_add(1);
        let previous = previous?;
        if let Some(slot) = self.old.iter_mut().find(|s| s.is_none()) {
            *slot = Some((generation, previous));
            return None;
        }
        // Sin sitio: sale la más antigua.
        let oldest = (0..OLD_TABLES)
            .min_by_key(|&i| self.old[i].as_ref().map_or(u32::MAX, |(g, _)| *g))
            .unwrap_or(0);
        let evicted = self.old[oldest].replace((generation, previous));
        evicted.map(|(_, t)| t)
    }

    /// Las anteriores que ninguna voz usa (según `used`), fuera.
    pub fn retire(&mut self, used: impl Fn(u32) -> bool, mut out: impl FnMut(Box<ModTable>)) {
        for slot in &mut self.old {
            if let Some((generation, _)) = slot
                && !used(*generation)
                && let Some((_, table)) = slot.take()
            {
                out(table);
            }
        }
    }

    pub fn has_old(&self) -> bool {
        self.old.iter().any(Option::is_some)
    }
}

/// El estado de la modulación de una voz.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ModVoice {
    /// El set (en su tabla) y la tabla con la que nació.
    pub(crate) slot: u8,
    pub(crate) generation: u32,
    /// Frames desde el golpe del punto de control siguiente.
    t: u32,
    /// Frames desde el golpe del note-off.
    off: Option<u32>,
    hit: HitInfo,
    sample_rate: u32,
    /// Lo que se interpola: el valor de cada dominio en el punto de control
    /// anterior y en el siguiente, y dónde va entre ellos.
    from: [f32; DOMAINS],
    to: [f32; DOMAINS],
    k: u32,
    state: [[DevState; MAX_DEVICES]; DOMAINS],
    /// Semitonos del pitch con su valor en 1 (los del set al nacer).
    pub(crate) range: f32,
}

impl ModVoice {
    /// La modulación de un golpe que nace ahora con el set `slot` de la
    /// tabla número `generation`, con el note-off a los `off` frames (si se sabe).
    pub fn new(
        set: &ModSet,
        slot: u8,
        generation: u32,
        hit: HitInfo,
        off: Option<u32>,
        at: &ModAt,
        sample_rate: u32,
    ) -> ModVoice {
        let mut voice = ModVoice {
            slot,
            generation,
            t: 0,
            off,
            hit,
            sample_rate,
            from: [0.0; DOMAINS],
            to: [0.0; DOMAINS],
            k: 0,
            state: [[DevState::default(); MAX_DEVICES]; DOMAINS],
            range: set.pitch_range,
        };
        voice.to = voice.eval(set, at);
        voice.from = voice.to;
        voice.t = CONTROL_FRAMES;
        voice.to = voice.eval(set, &at.later(CONTROL_FRAMES as f64, sample_rate));
        voice
    }

    fn eval(&mut self, set: &ModSet, at: &ModAt) -> [f32; DOMAINS] {
        let (t, off, hit, rate) = (self.t, self.off, self.hit, self.sample_rate);
        std::array::from_fn(|d| set.value(d, t, off, &hit, &mut self.state[d], at, rate))
    }

    /// El valor de cada dominio en el frame que toca ahora (sin avanzar).
    pub fn current(&self) -> [f32; DOMAINS] {
        let u = self.k as f32 / CONTROL_FRAMES as f32;
        std::array::from_fn(|d| self.from[d] + (self.to[d] - self.from[d]) * u)
    }

    /// El último [`ModVoice::next`] empezó un trozo de control nuevo.
    pub fn fresh(&self) -> bool {
        self.k == 1
    }

    /// El trozo de control en curso del dominio `domain`: sus extremos y
    /// dónde va el último frame que se dio (de 0 a 1).
    pub fn segment(&self, domain: usize) -> (f32, f32, f32) {
        (
            self.from[domain],
            self.to[domain],
            self.k.saturating_sub(1) as f32 / CONTROL_FRAMES as f32,
        )
    }

    /// El note-off: ahora (si no lo tenía antes).
    pub fn release(&mut self) {
        let now = self.t.saturating_sub(CONTROL_FRAMES) + self.k;
        if self.off.is_none_or(|off| off > now) {
            self.off = Some(now);
        }
    }

    /// Ya ha pasado su note-off.
    pub fn released(&self) -> bool {
        self.off
            .is_some_and(|off| off <= self.t.saturating_sub(CONTROL_FRAMES) + self.k)
    }

    /// El valor de cada dominio en el frame que toca, y avanza uno. `set`:
    /// el suyo (sin él, la tabla ya no está: se queda donde estaba);
    /// `at()`: la canción ahora (solo se pide en los puntos de control).
    #[inline]
    pub fn next<'a>(
        &mut self,
        set: Option<&ModSet>,
        at: impl FnOnce() -> ModAt<'a>,
    ) -> [f32; DOMAINS] {
        if self.k == CONTROL_FRAMES {
            self.k = 0;
            self.from = self.to;
            match set {
                Some(set) => {
                    self.t = self.t.saturating_add(CONTROL_FRAMES);
                    let at = at().later(CONTROL_FRAMES as f64, self.sample_rate);
                    self.to = self.eval(set, &at);
                }
                None => self.to = self.from,
            }
        }
        let u = self.k as f32 / CONTROL_FRAMES as f32;
        self.k += 1;
        std::array::from_fn(|d| self.from[d] + (self.to[d] - self.from[d]) * u)
    }

    /// Avanza `frames` frames (un sintetizador, por bloques) y da el valor
    /// de cada dominio en el último. `at(i)`: la canción `i` frames después
    /// del primero.
    pub fn advance<'a>(
        &mut self,
        set: Option<&ModSet>,
        frames: u32,
        at: impl Fn(u32) -> ModAt<'a>,
    ) -> [f32; DOMAINS] {
        let mut out = self.from;
        for i in 0..frames {
            out = self.next(set, || at(i));
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shapes_go_from_0_to_1() {
        for c in [-1.0, -0.3, 0.0, 0.4, 1.0] {
            assert_eq!(shape(0.0, c), 0.0);
            assert!((shape(1.0, c) - 1.0).abs() < 1e-6);
        }
        assert!(shape(0.5, 1.0) > 0.5);
        assert!(shape(0.5, -1.0) < 0.5);
    }

    #[test]
    fn cutoff_round_trips() {
        assert_eq!(cutoff_hz(0.0), 20.0);
        assert!((cutoff_hz(1.0) - 20_000.0).abs() < 1.0);
        assert!((cutoff_norm(cutoff_hz(0.37)) - 0.37).abs() < 1e-5);
    }
}
