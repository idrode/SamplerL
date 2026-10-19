//! Granular y congelar: una pista toca su sample en granos.
//!
//! Con [`Pattern::with_granular`], cada golpe de la pista abre una nube de
//! granos sobre su corte mientras dura su paso (y sus `_`): cada grano es
//! un trozo del corte con ventana de Hann, leído a su velocidad (pitch), y
//! los granos nacen cada `1 / density` segundos. La posición de cada grano
//! es `pos` (fracción del corte) más lo que ha avanzado la lectura
//! (`speed`: 1 recorre el corte a su velocidad, 0 lo congela en `pos`) más
//! un desplazamiento al azar de hasta `jitter` ms; su pitch, el del paso
//! más `pitch` más hasta `spread` semitonos al azar. Al acabar el paso no
//! nacen más granos y los que suenan acaban su ventana.
//!
//! Cada parámetro es un número o una curva ([`Curve`]: `ramp`, `lfo`) que
//! se evalúa en el frame en que nace el grano: depende solo de la posición
//! de la canción. El azar sale de un generador (splitmix64) con la semilla
//! de la pista y el frame de la canción en que nace el grano: la misma
//! canción suena igual en cada render, y recargar no lo cambia.
//!
//! En vivo (pads y MIDI), una tecla abre una emisión que dura hasta que se
//! suelta ([`Cloud::open_live`], [`Cloud::release_live`]): no depende del
//! paso ni de que el secuenciador corra. Su azar sale de la semilla y de un
//! reloj propio (cuántas emisiones en vivo lleva la pista y cuántos frames
//! lleva esta), no de la posición de la canción: tocar lo mismo en el mismo
//! orden suena igual esté la canción donde esté.
//!
//! Los granos van en un array fijo por pista ([`MAX_GRAINS`]): nada de
//! memoria dinámica en el hilo de audio. Si se llenan (no debería: la
//! validación no deja que se solapen más), el grano nuevo no nace.
//!
//! Nivel: cada grano va por `1 / √(max(1, solape / 2))`, con `solape =
//! densidad · tamaño` (los granos que suenan a la vez de media): una nube
//! densa con jitter suena más o menos como el sample.
//!
//! [`Pattern::with_granular`]: crate::Pattern::with_granular

use std::sync::Arc;

use crate::automation::Curve;
use crate::sample::Sample;

/// Granos que pueden sonar a la vez en una pista.
pub const MAX_GRAINS: usize = 32;

/// Un parámetro de la nube: fijo o una curva de la canción.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Knob {
    Fixed(f32),
    Moving(Curve),
}

impl Knob {
    /// Valor en la posición de la canción (`steps` pasos, `seconds`).
    pub fn at(&self, steps: f64, seconds: f64, log: bool) -> f32 {
        match self {
            Knob::Fixed(v) => *v,
            Knob::Moving(curve) => curve.value(steps, seconds, log),
        }
    }

    /// Los valores extremos que puede tomar.
    pub fn bounds(&self) -> (f32, f32) {
        match self {
            Knob::Fixed(v) => (*v, *v),
            Knob::Moving(curve) => {
                let (a, b) = curve.bounds();
                (a.min(b), a.max(b))
            }
        }
    }
}

/// Parámetros de la nube de granos de una pista.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Granular {
    /// Duración de cada grano, en ms.
    pub size_ms: Knob,
    /// Granos que nacen por segundo.
    pub density: Knob,
    /// Dónde se leen, en fracción del corte (0 = su inicio, 1 = su final).
    pub position: Knob,
    /// Velocidad con que avanza la posición (1: la del sample; 0:
    /// congelada; negativa: hacia atrás).
    pub speed: Knob,
    /// Desplazamiento al azar de la posición de cada grano, en ms (±).
    pub jitter_ms: Knob,
    /// Pitch de los granos, en semitonos (se suma al del paso).
    pub pitch: Knob,
    /// Desplazamiento al azar del pitch de cada grano, en semitonos (±).
    pub spread: Knob,
    /// Semilla del azar.
    pub seed: u64,
}

/// Nombre, rango, valor por defecto y si se interpola en escala
/// logarítmica, de cada parámetro (en el orden de [`Granular::knobs`]).
pub const GRAIN_PARAMS: [(&str, f32, f32, f32, bool); 7] = [
    ("size", 5.0, 1000.0, 80.0, true),
    ("density", 0.5, 400.0, 20.0, true),
    ("pos", 0.0, 1.0, 0.0, false),
    ("speed", -4.0, 4.0, 1.0, false),
    ("jitter", 0.0, 2000.0, 0.0, false),
    ("pitch", -24.0, 24.0, 0.0, false),
    ("spread", 0.0, 24.0, 0.0, false),
];

impl Granular {
    /// Con los valores por defecto y esa semilla.
    pub fn new(seed: u64) -> Self {
        let d = |i: usize| Knob::Fixed(GRAIN_PARAMS[i].3);
        Granular {
            size_ms: d(0),
            density: d(1),
            position: d(2),
            speed: d(3),
            jitter_ms: d(4),
            pitch: d(5),
            spread: d(6),
            seed,
        }
    }

    pub fn knobs(&self) -> [&Knob; 7] {
        [
            &self.size_ms,
            &self.density,
            &self.position,
            &self.speed,
            &self.jitter_ms,
            &self.pitch,
            &self.spread,
        ]
    }

    pub fn knobs_mut(&mut self) -> [&mut Knob; 7] {
        [
            &mut self.size_ms,
            &mut self.density,
            &mut self.position,
            &mut self.speed,
            &mut self.jitter_ms,
            &mut self.pitch,
            &mut self.spread,
        ]
    }

    /// Comprueba los rangos (también los extremos de las curvas) y que los
    /// granos no se solapen más de [`MAX_GRAINS`].
    pub fn validate(&self) -> Result<(), &'static str> {
        const OUT: [&str; 7] = [
            "size fuera de rango (5 a 1000 ms)",
            "density fuera de rango (0.5 a 400 granos por segundo)",
            "pos fuera de rango (0 a 1)",
            "speed fuera de rango (-4 a 4)",
            "jitter fuera de rango (0 a 2000 ms)",
            "pitch fuera de rango (-24 a 24 semitonos)",
            "spread fuera de rango (0 a 24 semitonos)",
        ];
        for ((knob, (_, lo, hi, _, _)), out) in self.knobs().iter().zip(GRAIN_PARAMS).zip(OUT) {
            let (a, b) = knob.bounds();
            if !(a.is_finite() && b.is_finite() && lo <= a && b <= hi) {
                return Err(out);
            }
            if let Knob::Moving(curve) = knob {
                crate::automation::check_curve(curve)?;
            }
        }
        let overlap = self.density.bounds().1 * self.size_ms.bounds().1 / 1000.0;
        if overlap > MAX_GRAINS as f32 {
            return Err("density × size: se solaparían más de 32 granos a la vez");
        }
        Ok(())
    }

    fn values(&self, steps: f64, seconds: f64) -> [f32; 7] {
        let knobs = self.knobs();
        std::array::from_fn(|i| knobs[i].at(steps, seconds, GRAIN_PARAMS[i].4))
    }
}

/// Un grano que suena.
pub(crate) struct Grain {
    pub sample: Arc<Sample>,
    /// Rango del corte (no lee fuera).
    start: usize,
    end: usize,
    /// Posición del primer frame que lee, velocidad (negativa: al revés)
    /// y frames de salida que dura.
    from: f64,
    rate: f64,
    length: u32,
    age: u32,
    gain: f32,
    /// Cortado (choke, paro): fundido mínimo, frames que quedan.
    killed: Option<u32>,
    fade: u32,
}

impl Grain {
    pub fn is_active(&self) -> bool {
        self.age < self.length && self.killed != Some(0)
    }

    fn mix_into(&mut self, frame: &mut [f32; 2]) {
        if !self.is_active() {
            return;
        }
        let t = self.age as f32 / self.length as f32;
        let mut w = self.gain * (0.5 - 0.5 * (std::f32::consts::TAU * t).cos());
        if let Some(left) = &mut self.killed {
            w *= *left as f32 / self.fade as f32;
            *left -= 1;
        }
        let span = (self.end - self.start) as f64;
        let offset = (self.from + self.rate * self.age as f64).rem_euclid(span);
        let (l, r) = crate::interp::read(
            &self.sample,
            self.start as f64 + offset,
            self.start,
            self.end,
            self.rate.abs(),
        );
        frame[0] += w * l;
        frame[1] += w * r;
        self.age += 1;
    }
}

/// Lo que abre un golpe: de dónde salen los granos y durante cuánto.
pub(crate) struct Emission {
    pub sample: Arc<Sample>,
    start: usize,
    end: usize,
    /// Velocidad del pitch del paso y su sentido.
    rate: f64,
    reverse: bool,
    gain: f32,
    /// Frames de salida que quedan de emisión y hasta el siguiente grano.
    left: u64,
    next: f64,
    /// Lo que ha avanzado la lectura (frames del sample).
    scan: f64,
    /// Emisión en vivo: sigue hasta que se suelta su tecla.
    live: Option<Live>,
}

/// Lo propio de una emisión en vivo.
struct Live {
    /// Tecla que la mantiene (la del motor, como las notas de un
    /// instrumento).
    key: u8,
    /// Cuántas emisiones en vivo había abierto la pista antes de esta.
    serial: u64,
    /// Frames de salida desde que se abrió.
    elapsed: u64,
}

/// Duración de una emisión en vivo hasta que se suelta.
const HELD: u64 = u64::MAX;

/// La nube de granos de una pista.
pub(crate) struct Cloud {
    pub grains: [Option<Grain>; MAX_GRAINS],
    pub emission: Option<Emission>,
    /// Granos que han nacido (para el estado).
    pub born: u32,
    /// Emisiones en vivo abiertas (para su azar).
    opened: u64,
    /// Parámetros de una nube en vivo mientras el patrón con granos de su
    /// pista no ha entrado (el que espera al compás): los usa en su lugar.
    pub standby: Option<Granular>,
}

impl Default for Cloud {
    fn default() -> Self {
        Cloud {
            grains: std::array::from_fn(|_| None),
            emission: None,
            born: 0,
            opened: 0,
            standby: None,
        }
    }
}

/// Dónde está la canción en el primer frame de un trozo.
pub(crate) struct Position {
    pub frame: u64,
    pub steps: f64,
    pub seconds: f64,
    pub frames_per_step: f64,
    pub sample_rate: u32,
}

impl Cloud {
    /// Suena (o va a sonar): emite o le quedan granos.
    pub fn is_active(&self) -> bool {
        self.emission.is_some() || self.grains.iter().flatten().any(Grain::is_active)
    }

    /// Un golpe: los granos nacen del rango `start..end` durante `gate`
    /// frames de salida (el primero, ya). Devuelve la emisión anterior
    /// (para soltar su sample fuera del audio).
    #[allow(clippy::too_many_arguments)]
    pub fn open(
        &mut self,
        sample: Arc<Sample>,
        start: usize,
        end: usize,
        rate: f64,
        reverse: bool,
        gain: f32,
        gate: u64,
    ) -> Option<Emission> {
        let emission = (end > start).then_some(Emission {
            sample,
            start,
            end,
            rate,
            reverse,
            gain,
            left: gate.max(1),
            next: 0.0,
            scan: 0.0,
            live: None,
        });
        std::mem::replace(&mut self.emission, emission)
    }

    /// Una tecla en vivo: los granos nacen del rango `start..end` (al pitch
    /// `rate`) hasta que se suelta la tecla `key`, que cierra esta emisión
    /// ([`Cloud::release_live`]). Devuelve la emisión anterior.
    pub fn open_live(
        &mut self,
        sample: Arc<Sample>,
        (start, end): (usize, usize),
        rate: f64,
        gain: f32,
        key: u8,
    ) -> Option<Emission> {
        let serial = self.opened;
        self.opened += 1;
        let old = self.open(sample, start, end, rate, false, gain, HELD);
        // (Con un rango vacío no se abre nada.)
        if let Some(e) = &mut self.emission {
            e.live = Some(Live {
                key,
                serial,
                elapsed: 0,
            });
        }
        old
    }

    /// Se soltó la tecla `key`: si mantenía la emisión, no nacen más granos
    /// (los que suenan acaban su ventana). Devuelve la emisión para soltar
    /// su sample fuera del audio.
    pub fn release_live(&mut self, key: u8) -> Option<Emission> {
        let holds = self
            .emission
            .as_ref()
            .is_some_and(|e| e.live.as_ref().is_some_and(|l| l.key == key));
        if holds { self.emission.take() } else { None }
    }

    /// Para la emisión y corta los granos con el fundido mínimo.
    pub fn kill(&mut self, fade: u32) -> Option<Emission> {
        for grain in self.grains.iter_mut().flatten() {
            if grain.killed.is_none() {
                grain.killed = Some(fade.min(grain.length - grain.age.min(grain.length)));
                grain.fade = fade.max(1);
            }
        }
        self.emission.take()
    }

    /// Suma la nube en `out` (estéreo entrelazado): nacen los granos que
    /// tocan y suenan los que hay. Devuelve la emisión si acaba (para soltar
    /// su sample fuera). Los granos acabados se quedan en su ranura hasta
    /// que el motor los suelta.
    pub fn mix_into(
        &mut self,
        out: &mut [f32],
        params: &Granular,
        at: &Position,
        fade: u32,
    ) -> Option<Emission> {
        let rate = at.sample_rate as f64;
        let mut done = None;
        for (k, frame) in out.as_chunks_mut::<2>().0.iter_mut().enumerate() {
            let steps = at.steps + k as f64 / at.frames_per_step;
            let seconds = at.seconds + k as f64 / rate;
            if let Some(e) = &mut self.emission {
                if e.next <= 0.0 {
                    let [size_ms, density, position, _, jitter_ms, pitch, spread] =
                        params.values(steps, seconds);
                    // Azar de este grano: la semilla y el frame de la canción
                    // (en vivo, el reloj de la emisión).
                    let clock = match &e.live {
                        Some(live) => (live.serial << 40) ^ live.elapsed,
                        None => at.frame + k as u64,
                    };
                    let mut random = Random::new(params.seed, clock);
                    let (u, w) = (random.next(), random.next());
                    let span = (e.end - e.start) as f64;
                    let semitones = pitch as f64 + (2.0 * w - 1.0) * spread as f64;
                    let grain_rate = e.rate * 2f64.powf(semitones / 12.0);
                    let length = (size_ms as f64 / 1000.0 * rate).round().max(2.0);
                    let extent = grain_rate * length;
                    let jitter = (2.0 * u - 1.0) * jitter_ms as f64 / 1000.0 * rate;
                    let mut from = (position as f64 * span + e.scan + jitter).rem_euclid(span);
                    // Que el grano quepa en el corte (si cabe).
                    if extent < span {
                        from = from.min(span - 1.0 - extent);
                    }
                    let (from, signed) = if e.reverse {
                        ((from + extent).min(span - 1.0), -grain_rate)
                    } else {
                        (from, grain_rate)
                    };
                    // En un frame entero: sin pitch, el grano copia el sample
                    // sin interpolar (mucho más barato, y no se oye).
                    let from = from.floor();
                    let overlap = (density * size_ms / 1000.0) as f64;
                    let gain = e.gain / (overlap / 2.0).max(1.0).sqrt() as f32;
                    if let Some(slot) = self.grains.iter_mut().find(|g| g.is_none()) {
                        *slot = Some(Grain {
                            sample: e.sample.clone(),
                            start: e.start,
                            end: e.end,
                            from,
                            rate: signed,
                            length: length as u32,
                            age: 0,
                            gain,
                            killed: None,
                            fade: fade.max(1),
                        });
                        self.born += 1;
                    }
                    e.next += rate / density as f64;
                }
                e.next -= 1.0;
                e.left -= 1;
                if let Some(live) = &mut e.live {
                    live.elapsed += 1;
                }
                // La lectura avanza con la velocidad de ese momento.
                e.scan += params.speed.at(steps, seconds, false) as f64;
                if e.left == 0 {
                    done = self.emission.take();
                }
            }
            for grain in self.grains.iter_mut().flatten() {
                grain.mix_into(frame);
            }
        }
        done
    }
}

/// splitmix64: rápido, sin estado global y con buena mezcla.
struct Random(u64);

impl Random {
    fn new(seed: u64, frame: u64) -> Self {
        Random(seed ^ frame.wrapping_mul(0x9e37_79b9_7f4a_7c15))
    }

    /// Un número en [0, 1).
    fn next(&mut self) -> f64 {
        self.0 = self.0.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        z ^= z >> 31;
        (z >> 11) as f64 / (1u64 << 53) as f64
    }
}
