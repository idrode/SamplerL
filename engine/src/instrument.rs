//! Instrumento: una pista que toca su sample como notas, con polifonía.
//!
//! Cada golpe de una pista con instrumento ([`Pattern::with_instrument`])
//! abre una nota (una por nota del acorde) en una de sus voces, sin cortar
//! las que suenan: si no queda ninguna libre, se roba la más antigua (con
//! el fundido mínimo, aparte, para que no haga clic; ya no cuenta como
//! voz). Cada nota tiene su envolvente ADSR: sube en el ataque, baja en el
//! decay hasta el sustain, y al acabar su paso (o sus `_`) hace el release.
//! Con un bucle sostenido, al llegar a su fin vuelve a su inicio mientras
//! dure la nota (también en el release); los últimos frames antes del fin
//! se funden con los de antes del inicio, así que el salto no hace clic.
//!
//! La envolvente y la posición se calculan desde el número de frames de la
//! nota (no acumulando), así que son exactas y deterministas.
//!
//! [`Pattern::with_instrument`]: crate::Pattern::with_instrument

use std::sync::Arc;

use crate::sample::Sample;
use crate::shift::Shift;

/// Voces máximas de un instrumento.
pub const MAX_VOICES: usize = 16;
/// Ataque, decay y release máximos, en ms.
pub const MAX_ENVELOPE_MS: f32 = 10_000.0;

/// Envolvente de cada nota: ataque, decay y release en ms (con 0, el
/// fundido mínimo de 1,5 ms que evita los clics) y nivel de sustain (0 a
/// 1). Los tramos son rectas.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Adsr {
    pub attack_ms: f32,
    pub decay_ms: f32,
    pub sustain: f32,
    pub release_ms: f32,
}

impl Adsr {
    pub const DEFAULT: Adsr = Adsr {
        attack_ms: 0.0,
        decay_ms: 0.0,
        sustain: 1.0,
        release_ms: 100.0,
    };
}

impl Default for Adsr {
    fn default() -> Self {
        Self::DEFAULT
    }
}

/// Cómo toca una pista su sample como instrumento.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Instrument {
    /// Notas que pueden sonar a la vez (de 1 a [`MAX_VOICES`]).
    pub voices: u8,
    pub adsr: Adsr,
    /// Bucle sostenido, en frames del sample (`inicio..fin`); sin él, la
    /// nota acaba con el sample.
    pub sustain_loop: Option<(usize, usize)>,
    /// Frames del fundido cruzado del salto del bucle: los últimos antes de
    /// su fin se funden con los de antes de su inicio (no puede pasar de
    /// ninguno de los dos).
    pub crossfade: usize,
    /// Las notas conservan la duración del sample al transponerse (granos
    /// alineados, ver `crate::shift`), como `%` en los pasos.
    pub keep: bool,
    /// Formantes de sus notas con `keep`: `Some(0)` donde estaban,
    /// `Some(n)` desplazados `n` semitonos; `None`, con el pitch.
    pub formant: Option<f32>,
}

impl Instrument {
    pub fn new(voices: u8) -> Self {
        Instrument {
            voices,
            adsr: Adsr::DEFAULT,
            sustain_loop: None,
            crossfade: 0,
            keep: false,
            formant: None,
        }
    }

    /// Comprueba los rangos para un sample de `frames` frames.
    pub fn validate(&self, frames: usize) -> Result<(), &'static str> {
        if !(1..=MAX_VOICES).contains(&(self.voices as usize)) {
            return Err("voces fuera de rango (1 a 16)");
        }
        crate::shift::check_formant(self.formant)?;
        if self.formant.is_some() && !self.keep {
            return Err("formant necesita keep (los formantes se corrigen en los granos)");
        }
        let a = &self.adsr;
        let envelope = 0.0..=MAX_ENVELOPE_MS;
        if !(envelope.contains(&a.attack_ms)
            && envelope.contains(&a.decay_ms)
            && envelope.contains(&a.release_ms))
        {
            return Err("ataque, decay y release van de 0 a 10000 ms");
        }
        if !(0.0..=1.0).contains(&a.sustain) {
            return Err("sustain fuera de rango (0 a 1)");
        }
        if let Some((start, end)) = self.sustain_loop {
            if !(start < end && end <= frames) {
                return Err("bucle fuera del sample (o con el fin antes del inicio)");
            }
            if self.crossfade > start || self.crossfade > end - start {
                return Err(
                    "el fundido del bucle no cabe (ni antes del inicio ni dentro del bucle)",
                );
            }
        }
        Ok(())
    }
}

/// Una nota que suena en una voz.
pub(crate) struct Note {
    pub sample: Arc<Sample>,
    /// Rango del sample que toca (el corte), velocidad de lectura y sentido.
    start: usize,
    end: usize,
    rate: f64,
    reverse: bool,
    gain: f32,
    pan: f32,
    /// Bucle sostenido (en frames del sample) y su fundido.
    sustain_loop: Option<(usize, usize)>,
    crossfade: usize,
    /// Envolvente en frames de salida (cada tramo, al menos el fundido
    /// mínimo) y sustain.
    attack: u64,
    decay: u64,
    sustain: f32,
    release: u64,
    fade: u64,
    /// Frames de salida ya renderizados.
    rendered: u64,
    /// Frames que dura sin bucle (hasta el final del rango).
    length: u64,
    /// Frame de salida en que acaba su paso y empieza el release.
    gate: u64,
    /// Orden de llegada en la pista (para robar la más antigua).
    pub born: u64,
    /// Robada o cortada: fundido mínimo en curso (frames que quedan).
    killed: Option<u64>,
    /// Tecla que la toca en vivo (pad o nota MIDI): su note off la suelta.
    pub key: Option<u8>,
    /// Pitch que conserva la duración: los granos (con pitch 0, ninguno).
    shift: Option<Shift>,
}

/// Lo que necesita una nota para empezar.
pub(crate) struct NoteOn {
    pub start: usize,
    pub end: usize,
    pub rate: f64,
    /// El pitch no cambia la duración (del paso con `%` o del instrumento).
    pub keep: bool,
    /// Formantes (ver [`crate::shift`]): del patrón; si no, los del
    /// instrumento.
    pub formant: Option<f32>,
    pub reverse: bool,
    pub gain: f32,
    pub pan: f32,
    /// Frames de salida hasta el release ([`HELD`]: hasta su note off).
    pub gate: u64,
    pub born: u64,
    /// Tecla que la toca en vivo, si es en vivo.
    pub key: Option<u8>,
}

/// `gate` de una nota tocada en vivo: suena hasta su note off
/// ([`Note::release_now`]).
pub(crate) const HELD: u64 = u64::MAX;

impl Note {
    pub fn new(sample: Arc<Sample>, instrument: &Instrument, on: NoteOn, fade: u64) -> Self {
        let rate_hz = sample.sample_rate() as f32;
        let frames = |ms: f32| ((ms / 1000.0 * rate_hz) as u64).max(fade);
        let a = &instrument.adsr;
        let span = (on.end - on.start) as u64;
        let keep = on.keep || instrument.keep;
        let formant = on.formant.or(instrument.formant);
        let shift = (keep && Shift::needed(on.rate, formant))
            .then(|| Shift::new(on.rate, sample.sample_rate(), formant));
        // Con los granos, la nota avanza por el sample a velocidad 1.
        let speed = if shift.is_some() { 1.0 } else { on.rate };
        let length = if span == 0 {
            0
        } else {
            ((span - 1) as f64 / speed).floor() as u64 + 1
        };
        // Al revés no hay bucle: se lee de fin a principio una vez.
        let sustain_loop = instrument.sustain_loop.filter(|_| !on.reverse);
        Note {
            sample,
            start: on.start,
            end: on.end,
            rate: on.rate,
            reverse: on.reverse,
            gain: on.gain,
            pan: on.pan,
            sustain_loop,
            crossfade: instrument.crossfade,
            attack: frames(a.attack_ms),
            decay: frames(a.decay_ms),
            sustain: a.sustain,
            release: frames(a.release_ms),
            fade,
            rendered: 0,
            length,
            gate: on.gate,
            born: on.born,
            killed: None,
            key: on.key,
            shift,
        }
    }

    /// Sigue sonando (aunque sea en su release o en el fundido de robo).
    pub fn is_active(&self) -> bool {
        if self.killed == Some(0) {
            return false;
        }
        let released = self.rendered >= self.gate.saturating_add(self.release);
        let ended = self.sustain_loop.is_none() && self.rendered >= self.length;
        !(released || ended)
    }

    /// Cuenta como voz: activa y no robada ni cortada.
    pub fn is_voice(&self) -> bool {
        self.killed.is_none() && self.is_active()
    }

    /// Note off: el release empieza ya (desde el nivel en que esté). Si ya
    /// había empezado, nada.
    pub fn release_now(&mut self) {
        self.gate = self.gate.min(self.rendered);
    }

    /// Corta la nota con el fundido mínimo.
    pub fn kill(&mut self) {
        if self.killed.is_none() {
            self.killed = Some(self.fade);
        }
    }

    /// ADSR en el frame de salida `n`.
    fn adsr(&self, n: u64) -> f32 {
        let held = |n: u64| -> f32 {
            if n < self.attack {
                n as f32 / self.attack as f32
            } else if n < self.attack + self.decay {
                1.0 - (1.0 - self.sustain) * (n - self.attack) as f32 / self.decay as f32
            } else {
                self.sustain
            }
        };
        if n < self.gate {
            held(n)
        } else {
            let from = held(self.gate);
            let k = (n - self.gate) as f32 / self.release as f32;
            from * (1.0 - k).max(0.0)
        }
    }

    /// Ganancia del frame actual: ADSR, fundido al final del sample (sin
    /// bucle) y el de robo.
    fn envelope(&self) -> f32 {
        let n = self.rendered;
        let mut gain = self.adsr(n);
        if self.sustain_loop.is_none() {
            let to_end = (self.length - 1 - n) as f32 / self.fade as f32;
            gain *= to_end.min(1.0);
        }
        if let Some(left) = self.killed {
            gain *= (left - 1) as f32 / self.fade as f32;
        }
        gain
    }

    /// Cómo lee la nota el sample (copia pequeña, para leer mientras se
    /// toca el desplazador).
    fn axis(&self) -> Axis {
        Axis {
            start: self.start,
            end: self.end,
            rate: self.rate,
            reverse: self.reverse,
            sustain_loop: self.sustain_loop,
            crossfade: self.crossfade,
        }
    }

    /// Frame del sample que toca ahora. Con el pitch que conserva la
    /// duración, la suma de sus dos granos (con la corrección de formantes,
    /// si la hay).
    fn read(&mut self) -> (f32, f32) {
        let axis = self.axis();
        let sample = &self.sample;
        let Some(shift) = self.shift.as_mut() else {
            return axis.read(sample, self.rendered as f64 * axis.rate);
        };
        let span = (axis.end - axis.start) as f64;
        // Sin bucle, ningún grano pasa del final del rango.
        let fit = axis
            .sustain_loop
            .is_none()
            .then(|| span - 1.0 - shift.reach());
        let grains = shift.grains(self.rendered, fit, |offset| {
            crate::shift::mono_at(sample, axis.position(offset), axis.start, axis.end)
        });
        let (mut l, mut r) = (0.0, 0.0);
        for (offset, weight) in grains {
            if weight == 0.0 || (fit.is_some() && offset > span - 1.0) {
                continue;
            }
            let (gl, gr) = axis.read(sample, offset);
            l += weight * gl;
            r += weight * gr;
        }
        shift.correct((l, r))
    }

    /// Suma la nota en `out` (estéreo entrelazado).
    pub fn mix_into(&mut self, out: &mut [f32]) {
        for frame in out.as_chunks_mut::<2>().0 {
            if !self.is_active() {
                break;
            }
            let gain = self.gain * self.envelope();
            let (l, r) = self.read();
            let pan = self.pan;
            frame[0] += (1.0 - pan).min(1.0) * (gain * l);
            frame[1] += (1.0 + pan).min(1.0) * (gain * r);
            self.rendered += 1;
            if let Some(left) = &mut self.killed {
                *left -= 1;
            }
        }
    }
}

/// Cómo lee una nota su sample: rango, velocidad, sentido y bucle.
#[derive(Clone, Copy)]
struct Axis {
    start: usize,
    end: usize,
    rate: f64,
    reverse: bool,
    sustain_loop: Option<(usize, usize)>,
    crossfade: usize,
}

impl Axis {
    fn at(&self, sample: &Sample, pos: f64) -> (f32, f32) {
        crate::interp::read(sample, pos, self.start, self.end, self.rate)
    }

    /// Posición en el sample del desplazamiento `offset` desde el inicio
    /// de la nota (con el bucle, sin su fundido).
    fn position(&self, offset: f64) -> f64 {
        if self.reverse {
            return (self.end - 1) as f64 - offset;
        }
        let pos = self.start as f64 + offset;
        match self.sustain_loop {
            Some((a, b)) if pos >= b as f64 => {
                a as f64 + (pos - a as f64).rem_euclid((b - a) as f64)
            }
            _ => pos,
        }
    }

    /// Frame del sample en el desplazamiento `offset`, con el salto del
    /// bucle fundido.
    fn read(&self, sample: &Sample, offset: f64) -> (f32, f32) {
        if self.reverse {
            return self.at(sample, (self.end - 1) as f64 - offset);
        }
        let pos = self.start as f64 + offset;
        let Some((loop_start, loop_end)) = self.sustain_loop else {
            return self.at(sample, pos);
        };
        let (loop_start, loop_end) = (loop_start as f64, loop_end as f64);
        let span = loop_end - loop_start;
        let pos = if pos >= loop_end {
            loop_start + (pos - loop_start).rem_euclid(span)
        } else {
            pos
        };
        let fade_from = loop_end - self.crossfade as f64;
        if self.crossfade == 0 || pos < fade_from {
            return self.at(sample, pos);
        }
        // Del fin del bucle hacia lo que hay antes de su inicio: al llegar
        // al fin ya suena entero lo que sigue en el inicio.
        let t = ((pos - fade_from) / self.crossfade as f64) as f32;
        let (l0, r0) = self.at(sample, pos);
        let (l1, r1) = self.at(sample, pos - span);
        (l0 + t * (l1 - l0), r0 + t * (r1 - r0))
    }
}
