//! Envolventes ADSR de las voces, con tramos exponenciales (como un
//! condensador que se carga y se descarga): suenan más naturales que las
//! rectas en la amplitud y en el corte del filtro.
//!
//! Cada tramo es un polo que se acerca a su destino. El ataque apunta un
//! poco por encima de 1 (si no, no llegaría nunca) y pasa al decay al
//! llegar; el release sale del nivel en que esté. Al volver a disparar se
//! sale también del nivel actual, sin saltos: una nota nueva en la misma
//! voz no hace clic.

/// Destino del ataque: llega a 1 en el tiempo pedido.
const ATTACK_TARGET: f32 = 1.3;
/// ln(1,3 / 0,3): constantes de tiempo que dura el ataque de 0 a 1.
const ATTACK_TAUS: f32 = 1.466_337;
/// Constantes de tiempo del decay y del release: con 4,6, el tramo cubre
/// el 99 % del camino en el tiempo pedido.
const FALL_TAUS: f32 = 4.605_17;
/// Por debajo de esto (-80 dB), una envolvente en su release ha acabado.
pub(crate) const SILENT: f32 = 1e-4;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Stage {
    Idle,
    Attack,
    Decay,
    Release,
}

/// Coeficientes de una envolvente (se recalculan cuando cambian sus
/// tiempos, no en cada muestra).
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct Rates {
    attack: f32,
    decay: f32,
    release: f32,
    sustain: f32,
}

/// El coeficiente de un polo con `taus` constantes de tiempo en `ms`.
fn pole(ms: f32, taus: f32, sample_rate: f32, min_ms: f32) -> f32 {
    let frames = ms.max(min_ms) / 1000.0 * sample_rate / taus;
    1.0 - (-1.0 / frames.max(1e-3)).exp()
}

impl Rates {
    /// Los coeficientes de un ADSR (ms y nivel de sustain); ningún tramo
    /// dura menos de `min_ms`.
    pub fn new(adsr: &crate::Adsr, decay_ms: f32, sample_rate: f32, min_ms: f32) -> Rates {
        Rates {
            attack: pole(adsr.attack_ms, ATTACK_TAUS, sample_rate, min_ms),
            decay: pole(decay_ms, FALL_TAUS, sample_rate, min_ms),
            release: pole(adsr.release_ms, FALL_TAUS, sample_rate, min_ms),
            sustain: adsr.sustain,
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct Env {
    pub level: f32,
    pub stage: Stage,
}

impl Default for Env {
    fn default() -> Self {
        Env {
            level: 0.0,
            stage: Stage::Idle,
        }
    }
}

impl Env {
    /// Dispara el ataque desde el nivel en que esté.
    pub fn trigger(&mut self) {
        self.stage = Stage::Attack;
    }

    /// Empieza el release (si no había empezado).
    pub fn release(&mut self) {
        if self.stage != Stage::Idle {
            self.stage = Stage::Release;
        }
    }

    /// Ha acabado su release (o no ha empezado nunca).
    pub fn is_idle(&self) -> bool {
        self.stage == Stage::Idle
    }

    /// El nivel de esta muestra, y avanza.
    #[inline]
    pub fn next(&mut self, r: &Rates) -> f32 {
        match self.stage {
            Stage::Idle => {}
            Stage::Attack => {
                self.level += (ATTACK_TARGET - self.level) * r.attack;
                if self.level >= 1.0 {
                    self.level = 1.0;
                    self.stage = Stage::Decay;
                }
            }
            Stage::Decay => self.level += (r.sustain - self.level) * r.decay,
            Stage::Release => {
                self.level -= self.level * r.release;
                if self.level < SILENT {
                    self.level = 0.0;
                    self.stage = Stage::Idle;
                }
            }
        }
        self.level
    }
}
