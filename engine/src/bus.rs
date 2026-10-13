//! Envíos: un delay sincronizado al tempo y una reverb, compartidos por
//! todas las pistas. Cada pista manda una parte de su salida (después de su
//! volumen, mute y solo) con `Pattern::with_sends`; lo que devuelven los
//! buses se suma al master antes del limitador.
//!
//! Los parámetros ([`Buses`]) entran en un lote, al empezar el compás, como
//! el tempo. Los buffers se reservan al crear el motor (o al cambiar su
//! frecuencia, en el hilo de control): en el callback no se reserva nada.
//! Un bus que no recibe nada y cuya cola ya se ha apagado se duerme: no se
//! procesa ni se suma (así, sin envíos, la salida es la de siempre bit a
//! bit, y no gasta CPU).

use crate::sequencer::{MIN_BPM, STEPS};

/// Nivel máximo de un envío.
pub const MAX_SEND: f32 = 1.0;
/// Realimentación máxima del delay (por debajo de 1: las repeticiones se
/// apagan siempre).
pub const MAX_FEEDBACK: f32 = 0.95;
/// Buses: el delay es el 0 y la reverb el 1 (índices de los envíos).
pub const BUSES: usize = 2;
pub const DELAY: usize = 0;
pub const REVERB: usize = 1;

/// Tiempo del delay en figuras del compás.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Division {
    /// Negra: 4 pasos.
    Quarter,
    /// Corchea con puntillo: 3 pasos.
    DottedEighth,
    /// Corchea: 2 pasos.
    Eighth,
    /// Semicorchea: 1 paso.
    Sixteenth,
}

impl Division {
    /// Todas, con su nombre en el script.
    pub const ALL: [(Division, &'static str); 4] = [
        (Division::Quarter, "1/4"),
        (Division::DottedEighth, "3/16"),
        (Division::Eighth, "1/8"),
        (Division::Sixteenth, "1/16"),
    ];

    /// Pasos (semicorcheas) que dura.
    pub fn steps(self) -> u32 {
        match self {
            Division::Quarter => 4,
            Division::DottedEighth => 3,
            Division::Eighth => 2,
            Division::Sixteenth => 1,
        }
    }

    pub fn parse(text: &str) -> Option<Division> {
        Self::ALL.iter().find(|(_, n)| *n == text).map(|(d, _)| *d)
    }

    pub fn name(self) -> &'static str {
        Self::ALL.iter().find(|(d, _)| *d == self).unwrap().1
    }
}

/// Delay: repeticiones cada `division`, cada una `feedback` veces la
/// anterior; vuelve al master con `level`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DelayParams {
    pub division: Division,
    pub feedback: f32,
    pub level: f32,
}

impl Default for DelayParams {
    fn default() -> Self {
        DelayParams {
            division: Division::Eighth,
            feedback: 0.4,
            level: 0.8,
        }
    }
}

/// Reverb (Freeverb): `size` alarga la cola, `damp` apaga los agudos de la
/// cola; vuelve al master con `level`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ReverbParams {
    pub size: f32,
    pub damp: f32,
    pub level: f32,
}

impl Default for ReverbParams {
    fn default() -> Self {
        ReverbParams {
            size: 0.7,
            damp: 0.5,
            level: 0.8,
        }
    }
}

/// Parámetros de los dos buses.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Buses {
    pub delay: DelayParams,
    pub reverb: ReverbParams,
}

impl Buses {
    pub fn validate(&self) -> Result<(), &'static str> {
        let unit = 0.0..=1.0;
        if !(0.0..=MAX_FEEDBACK).contains(&self.delay.feedback) {
            return Err("feedback del delay fuera de rango (0 a 0.95)");
        }
        if !unit.contains(&self.delay.level) || !unit.contains(&self.reverb.level) {
            return Err("level de un bus fuera de rango (0 a 1)");
        }
        if !unit.contains(&self.reverb.size) || !unit.contains(&self.reverb.damp) {
            return Err("size y damp de la reverb van de 0 a 1");
        }
        Ok(())
    }
}

/// Frames de retardo para una división a un tempo (con el mismo redondeo
/// que los pasos del secuenciador).
pub fn delay_frames(division: Division, bpm: f64, sample_rate: u32) -> usize {
    let step = sample_rate as f64 * 60.0 / (bpm * (STEPS as f64 / 4.0));
    (division.steps() as f64 * step).round().max(1.0) as usize
}

/// Los dos buses en el hilo de audio.
pub(crate) struct Rack {
    pub params: Buses,
    delay: DelayLine,
    reverb: Reverb,
    /// Frames seguidos sin entrada en cada bus, y si está despierto.
    idle: [usize; BUSES],
    awake: [bool; BUSES],
    sample_rate: u32,
}

impl Rack {
    /// Reserva los buffers (hilo de control).
    pub fn new(sample_rate: u32) -> Self {
        Rack {
            params: Buses::default(),
            delay: DelayLine::new(sample_rate),
            reverb: Reverb::new(sample_rate),
            idle: [0; BUSES],
            awake: [false; BUSES],
            sample_rate,
        }
    }

    /// Procesa `n` frames: `inputs[b]` es lo enviado al bus `b` (estéreo
    /// entrelazado) y `fed[b]` si tiene algo distinto de cero. Suma lo que
    /// devuelven los buses despiertos a `master`.
    pub fn mix_into(
        &mut self,
        inputs: &[[f32; 2 * crate::engine::CHUNK]; BUSES],
        fed: [bool; BUSES],
        n: usize,
        bpm: f64,
        master: &mut [f32],
    ) {
        let rate = self.sample_rate;
        let delay = delay_frames(self.params.delay.division, bpm, rate);
        for bus in 0..BUSES {
            if fed[bus] {
                self.awake[bus] = true;
                self.idle[bus] = 0;
            } else {
                self.idle[bus] += n;
            }
            if !self.awake[bus] {
                continue;
            }
            let input = &inputs[bus][..2 * n];
            let peak = match bus {
                DELAY => self.delay.process(input, master, delay, self.params.delay),
                _ => self.reverb.process(input, master, self.params.reverb),
            };
            // A dormir cuando la cola se ha apagado del todo.
            let tail = match bus {
                DELAY => delay * (1 + repeats(self.params.delay.feedback)),
                _ => rate as usize,
            };
            if self.idle[bus] > tail && (bus == DELAY || peak < 1e-6) {
                // Lo que queda dentro ya no se oye (o, con poco feedback, es
                // la entrada vieja): se vacía para que al despertar no suene.
                self.awake[bus] = false;
                match bus {
                    DELAY => self.delay.clear(),
                    _ => self.reverb.clear(),
                }
            }
        }
    }
}

/// Repeticiones del delay hasta quedar por debajo de -120 dB.
fn repeats(feedback: f32) -> usize {
    if feedback <= 0.0 {
        0
    } else {
        ((1e-6f32).ln() / feedback.ln()).ceil().min(10_000.0) as usize
    }
}

/// Línea de retardo estéreo con realimentación.
struct DelayLine {
    buf: Box<[f32]>,
    write: usize,
}

impl DelayLine {
    fn new(sample_rate: u32) -> Self {
        // La negra a 20 BPM, con margen.
        let max = delay_frames(Division::Quarter, MIN_BPM, sample_rate) + 2;
        DelayLine {
            buf: vec![0.0; 2 * max].into_boxed_slice(),
            write: 0,
        }
    }

    fn clear(&mut self) {
        self.buf.fill(0.0);
    }

    /// Devuelve el pico de lo que sale.
    fn process(&mut self, input: &[f32], out: &mut [f32], delay: usize, p: DelayParams) -> f32 {
        let frames = self.buf.len() / 2;
        let delay = delay.min(frames - 1);
        let mut peak = 0.0f32;
        for (x, o) in input
            .as_chunks::<2>()
            .0
            .iter()
            .zip(out.as_chunks_mut::<2>().0)
        {
            let read = (self.write + frames - delay) % frames;
            for c in 0..2 {
                let y = self.buf[2 * read + c];
                self.buf[2 * self.write + c] = flush(x[c] + p.feedback * y);
                let wet = p.level * y;
                o[c] += wet;
                peak = peak.max(wet.abs());
            }
            self.write = (self.write + 1) % frames;
        }
        peak
    }
}

/// Los valores muy pequeños, a cero (los subnormales son lentos y no se
/// oyen).
#[inline]
fn flush(x: f32) -> f32 {
    if x.abs() < 1e-20 { 0.0 } else { x }
}

/// Reverb de Schroeder-Moorer (Freeverb, de Jezar): 8 filtros peine con
/// amortiguación en paralelo y 4 pasatodo en serie por canal; el canal
/// derecho, con los retardos un poco más largos para abrir la imagen.
struct Reverb {
    combs: [[Comb; 8]; 2],
    allpasses: [[Allpass; 4]; 2],
}

const COMB_TUNING: [usize; 8] = [1116, 1188, 1277, 1356, 1422, 1491, 1557, 1617];
const ALLPASS_TUNING: [usize; 4] = [556, 441, 341, 225];
const STEREO_SPREAD: usize = 23;
const FIXED_GAIN: f32 = 0.015;
const SCALE_WET: f32 = 3.0;
const SCALE_DAMP: f32 = 0.4;
const SCALE_ROOM: f32 = 0.28;
const OFFSET_ROOM: f32 = 0.7;

impl Reverb {
    fn new(sample_rate: u32) -> Self {
        // Los retardos de Freeverb son para 44,1 kHz.
        let scale = |n: usize, channel: usize| {
            (((n + channel * STEREO_SPREAD) as f64 * sample_rate as f64 / 44_100.0).round()
                as usize)
                .max(1)
        };
        Reverb {
            combs: std::array::from_fn(|ch| {
                std::array::from_fn(|i| Comb::new(scale(COMB_TUNING[i], ch)))
            }),
            allpasses: std::array::from_fn(|ch| {
                std::array::from_fn(|i| Allpass::new(scale(ALLPASS_TUNING[i], ch)))
            }),
        }
    }

    fn clear(&mut self) {
        for comb in self.combs.iter_mut().flatten() {
            comb.buf.fill(0.0);
            comb.store = 0.0;
        }
        for allpass in self.allpasses.iter_mut().flatten() {
            allpass.buf.fill(0.0);
        }
    }

    fn process(&mut self, input: &[f32], out: &mut [f32], p: ReverbParams) -> f32 {
        let feedback = p.size * SCALE_ROOM + OFFSET_ROOM;
        let damp = p.damp * SCALE_DAMP;
        let wet = p.level * SCALE_WET;
        let mut peak = 0.0f32;
        for (x, o) in input
            .as_chunks::<2>()
            .0
            .iter()
            .zip(out.as_chunks_mut::<2>().0)
        {
            let mono = (x[0] + x[1]) * FIXED_GAIN;
            for ((combs, allpasses), o) in self.combs.iter_mut().zip(&mut self.allpasses).zip(o) {
                let mut acc = 0.0;
                for comb in combs {
                    acc += comb.process(mono, feedback, damp);
                }
                for allpass in allpasses {
                    acc = allpass.process(acc);
                }
                let y = wet * acc;
                *o += y;
                peak = peak.max(y.abs());
            }
        }
        peak
    }
}

struct Comb {
    buf: Box<[f32]>,
    index: usize,
    store: f32,
}

impl Comb {
    fn new(len: usize) -> Self {
        Comb {
            buf: vec![0.0; len].into_boxed_slice(),
            index: 0,
            store: 0.0,
        }
    }

    #[inline]
    fn process(&mut self, input: f32, feedback: f32, damp: f32) -> f32 {
        let output = self.buf[self.index];
        self.store = flush(output * (1.0 - damp) + self.store * damp);
        self.buf[self.index] = flush(input + self.store * feedback);
        self.index = (self.index + 1) % self.buf.len();
        output
    }
}

struct Allpass {
    buf: Box<[f32]>,
    index: usize,
}

impl Allpass {
    fn new(len: usize) -> Self {
        Allpass {
            buf: vec![0.0; len].into_boxed_slice(),
            index: 0,
        }
    }

    #[inline]
    fn process(&mut self, input: f32) -> f32 {
        let buffered = self.buf[self.index];
        self.buf[self.index] = flush(input + buffered * 0.5);
        self.index = (self.index + 1) % self.buf.len();
        buffered - input
    }
}
