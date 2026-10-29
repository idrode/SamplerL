//! Efectos por corte: una cadena de efectos (los de pista, hasta
//! [`MAX_FX`]) que solo suena en los golpes de ese corte.
//!
//! La cadena ([`SliceFx`]) va en el patrón, una por corte
//! (`Pattern::with_slice_fx`), y entra con él al empezar el compás. Cada
//! golpe de ese corte copia la cadena en su voz con su propio estado
//! ([`VoiceFx`], de tamaño fijo: nada de memoria dinámica en el hilo de
//! audio), así que el filtro de un golpe no arrastra la memoria del
//! anterior y la cola de un golpe cortado sigue con sus efectos mientras
//! hace su fundido.
//!
//! Orden: la voz (con su volumen, su envolvente y su pan), los efectos de
//! su corte, los efectos de la pista, el volumen de la pista y la mezcla.
//! Un corte sin efectos no pasa por aquí: suena bit a bit como siempre.
//!
//! Los efectos de un paso (`3{lp=800}` en la notación) van por el mismo
//! camino: su cadena se junta con la del corte del golpe
//! ([`SliceFx::with_step`]: manda el paso) y suena en la voz de ese golpe.

use crate::automation::{Automation, MAX_AUTOMATIONS};
use crate::engine::{Clock, GAIN_SMOOTHING_SECONDS, MOD_BLOCK, drive_block};
use crate::fx::{Fx, FxState, MAX_FX, MAX_PARAMS};

/// Lo que siguen sonando los efectos de un golpe después de acabar su
/// voz (la resonancia de un filtro o la cola de un EQ), para no cortarlos
/// en seco.
const TAIL_SECONDS: f32 = 0.1;

/// Cadena de efectos de un corte, con sus parámetros automatizados.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SliceFx {
    pub(crate) fx: [Option<Fx>; MAX_FX],
    pub(crate) automation: [Option<Automation>; MAX_AUTOMATIONS],
}

impl SliceFx {
    /// Cadena con `chain` (hasta [`MAX_FX`] efectos, en orden) y las
    /// automatizaciones de sus parámetros (`Automation::fx` es el índice
    /// en `chain`). El EQ dinámico no puede escuchar otra pista.
    pub fn new(chain: &[Fx], automation: &[Automation]) -> Result<Self, &'static str> {
        if chain.len() > MAX_FX {
            return Err("demasiados efectos (máximo 4 por corte)");
        }
        if automation.len() > MAX_AUTOMATIONS {
            return Err("demasiadas automatizaciones (máximo 8 por corte)");
        }
        let mut fx = [None; MAX_FX];
        for (slot, f) in fx.iter_mut().zip(chain) {
            f.validate()?;
            if f.sidechain().is_some() {
                return Err("el EQ dinámico de un corte no puede escuchar otra pista (sidechain)");
            }
            *slot = Some(*f);
        }
        let mut slots = [None; MAX_AUTOMATIONS];
        for (slot, a) in slots.iter_mut().zip(automation) {
            a.validate_in_voice(&fx)?;
            *slot = Some(*a);
        }
        Ok(SliceFx {
            fx,
            automation: slots,
        })
    }

    /// Los efectos de la cadena, en orden.
    pub fn effects(&self) -> impl Iterator<Item = &Fx> {
        self.fx.iter().flatten()
    }

    /// Las automatizaciones de la cadena (`Automation::fx`: el índice del
    /// efecto en [`SliceFx::effects`]).
    pub fn automations(&self) -> impl Iterator<Item = &Automation> {
        self.automation.iter().flatten()
    }

    /// La cadena de un golpe con efectos de corte (`self`) y de paso
    /// (`step`): manda el paso. Un efecto del paso del mismo tipo que uno
    /// del corte (el mismo filtro, el mismo bitcrusher...) lo sustituye, con
    /// sus curvas, en su sitio; los demás van detrás de los del corte. Sin
    /// memoria dinámica (vale en el hilo de audio); error si no cabe (más de
    /// [`MAX_FX`] efectos o [`MAX_AUTOMATIONS`] curvas).
    pub fn with_step(&self, step: &SliceFx) -> Result<SliceFx, &'static str> {
        let mut out = *self;
        for (i, fx) in step.fx.iter().enumerate() {
            let Some(fx) = fx else { continue };
            let slot = match out
                .fx
                .iter()
                .position(|f| f.is_some_and(|f| same_kind(&f, fx)))
            {
                Some(slot) => {
                    // Las curvas del efecto sustituido ya no valen.
                    for a in &mut out.automation {
                        if a.is_some_and(|a| a.fx == slot) {
                            *a = None;
                        }
                    }
                    slot
                }
                None => out
                    .fx
                    .iter()
                    .position(Option::is_none)
                    .ok_or("el paso y su corte suman más de 4 efectos")?,
            };
            out.fx[slot] = Some(*fx);
            for a in step.automation.iter().flatten().filter(|a| a.fx == i) {
                let free = out
                    .automation
                    .iter()
                    .position(Option::is_none)
                    .ok_or("el paso y su corte suman más de 8 curvas")?;
                out.automation[free] = Some(Automation { fx: slot, ..*a });
            }
        }
        Ok(out)
    }
}

/// Dos efectos del mismo tipo (un paso sustituye el de su corte): el mismo
/// efecto y, en los filtros, el mismo paso bajo o alto.
fn same_kind(a: &Fx, b: &Fx) -> bool {
    match (a, b) {
        (Fx::Filter { kind: x, .. }, Fx::Filter { kind: y, .. }) => x == y,
        _ => std::mem::discriminant(a) == std::mem::discriminant(b),
    }
}

/// Los efectos del corte de una voz, con su estado.
#[derive(Clone, Copy)]
pub(crate) struct VoiceFx {
    chain: SliceFx,
    state: [FxState; MAX_FX],
    /// Valor suavizado de cada parámetro que se mueve (ver `drive_block`).
    mods: [[Option<f32>; MAX_PARAMS]; MAX_FX],
    /// Ya ha procesado algo: el primer valor de una curva entra de golpe.
    started: bool,
    /// Frames que quedan de cola cuando la voz ya ha acabado.
    tail: usize,
    sample_rate: u32,
    /// Paso de la canción en que cayó el golpe (donde empieza la rampa de
    /// un golpe).
    origin: f64,
}

impl VoiceFx {
    /// Los efectos de una voz cuyo paso cayó en `origin` (pasos de la
    /// canción).
    pub fn new(chain: SliceFx, sample_rate: u32, origin: f64) -> Self {
        VoiceFx {
            chain,
            state: std::array::from_fn(|i| FxState::Off.update(chain.fx[i], sample_rate)),
            mods: [[None; MAX_PARAMS]; MAX_FX],
            started: false,
            tail: (TAIL_SECONDS * sample_rate as f32) as usize,
            sample_rate,
            origin,
        }
    }

    /// Le queda cola por sonar.
    pub fn ringing(&self) -> bool {
        self.tail > 0
    }

    /// La voz ya ha acabado y se han procesado `frames` frames más.
    pub fn spend_tail(&mut self, frames: usize) {
        self.tail = self.tail.saturating_sub(frames);
    }

    /// Pasa `buf` (estéreo entrelazado, la voz sola) por la cadena. `clock`
    /// es la posición de la canción al empezar `buf` (para las curvas).
    pub fn process(&mut self, buf: &mut [f32], clock: &Clock) {
        let snap = !std::mem::replace(&mut self.started, true);
        if self.chain.automation.iter().all(Option::is_none) {
            for state in &mut self.state {
                state.process(buf);
            }
            return;
        }
        let rate = self.sample_rate;
        let keep = (-(MOD_BLOCK as f32) / (GAIN_SMOOTHING_SECONDS * rate as f32)).exp();
        for (block, sub) in buf.chunks_mut(2 * MOD_BLOCK).enumerate() {
            let frames = (block * MOD_BLOCK) as f64;
            drive_block(
                &self.chain.fx,
                &self.chain.automation,
                None,
                &mut self.state,
                &mut self.mods,
                clock.steps + frames / clock.frames_per_step,
                clock.seconds + frames / rate as f64,
                self.origin,
                &clock.macros,
                None,
                snap,
                snap && block == 0,
                keep,
                rate,
            );
            for state in &mut self.state {
                state.process(sub);
            }
        }
    }
}
