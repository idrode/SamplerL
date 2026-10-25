//! Filtros de las voces, en forma TPT (transformada bilineal con la
//! integración trapezoidal resuelta sin retardos): siguen estables aunque
//! el corte y la resonancia cambien en cada muestra, que es lo que hacen
//! con la envolvente de filtro y las curvas.
//!
//! - [`Svf`]: variable de estado de 12 dB/oct (paso bajo, alto o banda).
//! - [`Ladder`]: paso bajo de 4 polos (24 dB/oct) con realimentación, el
//!   del Moog y, con otra voz, el del 303. La realimentación se resuelve
//!   en la misma muestra y pasa por una saturación suave antes de la
//!   cascada: con la resonancia al máximo autooscila, pero la saturación
//!   la acota (nunca diverge).
//! - [`AnalogLadder`]: el de la voz analógica (el `sh101`), con la
//!   saturación también en cada etapa y sin compensar la caída de los
//!   graves al subir la resonancia (salvo que se pida).
//! - [`OnePoleHp`]: paso alto de un polo, sin resonancia.

use super::FilterMode;

/// tanh aproximada (Padé 3/2, recortada en ±3, donde vale 1 con pendiente
/// 0): continua y derivable, siempre en [-1, 1] y solo con sumas,
/// productos y una división (igual en todas las arquitecturas).
#[inline]
pub(crate) fn soft(x: f32) -> f32 {
    let x = x.clamp(-3.0, 3.0);
    let x2 = x * x;
    x * (27.0 + x2) / (27.0 + 9.0 * x2)
}

/// Coeficiente `g = tan(π·corte/fs)` con el corte acotado entre 20 Hz y
/// el 45 % de la frecuencia de muestreo.
#[inline]
pub(crate) fn warp(cutoff_hz: f32, sample_rate: f32) -> f32 {
    let fc = cutoff_hz.clamp(20.0, 0.45 * sample_rate);
    (std::f32::consts::PI * fc / sample_rate).tan()
}

/// Variable de estado (Simper) de 12 dB/oct.
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct Svf {
    ic1: f32,
    ic2: f32,
}

impl Svf {
    /// Filtra `x` con el coeficiente `g` ([`warp`]) y la resonancia
    /// `res` (0 a 1; cerca de 1, un pico estrecho pero sin autooscilar).
    #[inline]
    pub fn next(&mut self, x: f32, g: f32, res: f32, mode: FilterMode) -> f32 {
        let k = 2.0 - 1.96 * res.clamp(0.0, 1.0);
        let a1 = 1.0 / (1.0 + g * (g + k));
        let a2 = g * a1;
        let a3 = g * a2;
        let v3 = x - self.ic2;
        let v1 = a1 * self.ic1 + a2 * v3;
        let v2 = self.ic2 + a2 * self.ic1 + a3 * v3;
        self.ic1 = 2.0 * v1 - self.ic1;
        self.ic2 = 2.0 * v2 - self.ic2;
        match mode {
            FilterMode::Highpass => x - k * v1 - v2,
            FilterMode::Bandpass => v1,
            _ => v2,
        }
    }
}

/// Resonancia del ladder: la realimentación `k` llega a 4 (donde
/// autooscila) con `res` = 1.
const LADDER_MAX_K: f32 = 4.0;

/// Paso bajo de 4 polos con realimentación (Zavalishin, «The Art of VA
/// Filter Design», cap. 5) y saturación en la entrada de la cascada.
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct Ladder {
    s: [f32; 4],
}

impl Ladder {
    /// Filtra `x` con el coeficiente `g` ([`warp`]) y la resonancia `res`
    /// (0 a 1).
    #[inline]
    pub fn next(&mut self, x: f32, g: f32, res: f32) -> f32 {
        let k = LADDER_MAX_K * res.clamp(0.0, 1.0);
        let big_g = g / (1.0 + g);
        // La salida de la cascada es G⁴·u + S, con S lo que aportan los
        // estados; con u = x − k·y, se despeja y en la misma muestra.
        let b = 1.0 - big_g;
        let s = self.s;
        let sum = ((s[0] * b * big_g + s[1] * b) * big_g + s[2] * b) * big_g + s[3] * b;
        let g4 = big_g * big_g * big_g * big_g;
        // Compensa en parte la caída de los graves al subir la resonancia.
        let x = (1.0 + 0.5 * k) * x;
        let y = (g4 * x + sum) / (1.0 + k * g4);
        let u = soft(x - k * y);
        let mut input = u;
        for state in &mut self.s {
            let v = (input - *state) * big_g;
            let out = v + *state;
            *state = out + v;
            input = out;
        }
        input
    }
}

/// Realimentación del ladder analógico con `res` = 1: algo más de 4, así
/// autooscila aunque la saturación de las etapas le quite ganancia.
const ANALOG_MAX_K: f32 = 4.4;

/// Paso bajo de 4 polos como [`Ladder`] (la realimentación resuelta en la
/// misma muestra con la cascada lineal, la saturación en la entrada), con
/// una saturación suave también a la entrada de cada etapa: la del
/// transistor de cada par del original. La saturación de cada etapa se
/// aplica sobre lo que le llega, sin iterar: cada etapa es un polo TPT
/// con la entrada acotada a ±1, que no diverge nunca, sea cual sea el
/// corte o la resonancia.
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct AnalogLadder {
    s: [f32; 4],
}

impl AnalogLadder {
    /// Filtra `x` con el coeficiente `g` ([`warp`]), la resonancia `res`
    /// (0 a 1; cerca de 1 autooscila) y la compensación de graves `comp`
    /// (0, ninguna: los graves caen al subir la resonancia; 1, la que la
    /// deja en su sitio).
    #[inline]
    pub fn next(&mut self, x: f32, g: f32, res: f32, comp: f32) -> f32 {
        let k = ANALOG_MAX_K * res.clamp(0.0, 1.0);
        let big_g = g / (1.0 + g);
        let b = 1.0 - big_g;
        let s = self.s;
        let sum = ((s[0] * b * big_g + s[1] * b) * big_g + s[2] * b) * big_g + s[3] * b;
        let g4 = big_g * big_g * big_g * big_g;
        // En continua la ganancia es 1/(1+k): con `comp` = 1, se devuelve.
        let x = (1.0 + comp.clamp(0.0, 1.0) * k) * x;
        let y = (g4 * x + sum) / (1.0 + k * g4);
        let mut input = soft(x - k * y);
        for (i, state) in self.s.iter_mut().enumerate() {
            let u = if i == 0 { input } else { soft(input) };
            let v = (u - *state) * big_g;
            let out = v + *state;
            *state = out + v;
            input = out;
        }
        input
    }
}

/// Paso alto de un polo (TPT), sin resonancia.
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct OnePoleHp {
    s: f32,
}

impl OnePoleHp {
    /// Filtra `x` con `big_g` = g/(1+g) ([`warp`] del corte).
    #[inline]
    pub fn next(&mut self, x: f32, big_g: f32) -> f32 {
        let v = (x - self.s) * big_g;
        let low = v + self.s;
        self.s = low + v;
        x - low
    }
}

/// El filtro de una voz (uno por canal).
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct Filter {
    svf: Svf,
    ladder: Ladder,
    /// Los de la voz analógica.
    pub analog: AnalogLadder,
    pub hp: OnePoleHp,
}

impl Filter {
    #[inline]
    pub fn next(&mut self, x: f32, g: f32, res: f32, mode: FilterMode) -> f32 {
        match mode {
            FilterMode::Ladder => self.ladder.next(x, g, res),
            _ => self.svf.next(x, g, res, mode),
        }
    }

    /// Algún estado no es finito (no debería pasar: lo comprueban los
    /// tests).
    pub fn is_finite(&self) -> bool {
        self.ladder.s.iter().all(|v| v.is_finite())
            && self.analog.s.iter().all(|v| v.is_finite())
            && self.hp.s.is_finite()
            && self.svf.ic1.is_finite()
            && self.svf.ic2.is_finite()
    }
}
