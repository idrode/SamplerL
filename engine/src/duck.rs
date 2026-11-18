//! Sidechain entre pistas (ducking): baja el volumen de la pista que lo
//! lleva, entera o solo sus graves, cuando suena otra.
//!
//! Dos disparos:
//! - **Por patrón** ([`DuckMode::Pattern`]): cada golpe del secuenciador de
//!   la pista origen (lo sabe el motor al dispararlo, al principio de un
//!   trozo) lanza una envolvente: llega a `depth_db` en `attack_ms` (en
//!   línea recta desde donde esté) y vuelve a 0 dB en `release_ms` con su
//!   curva. Sin detección de audio: latencia cero y exacto al frame.
//! - **Por audio** ([`DuckMode::Audio`]): un detector RMS (10 ms, calibrado
//!   para que un seno de amplitud A mida 20·log10(A) dB) sobre la salida de
//!   la pista origen (tras sus efectos y antes de su volumen, o después),
//!   con un paso bajo opcional; por encima de `threshold_db`, baja
//!   `1 − 1/ratio` dB por dB de más, con rodilla suave de `knee_db`, como
//!   mucho `−depth_db`, con ataque y relajación (polos de `attack_ms` y
//!   `release_ms`).
//!
//! Se aplica a la pista entera o, con `band_hz`, solo a lo que queda por
//! debajo de ese cruce: un Linkwitz-Riley de 24 dB/octava parte la pista
//! en graves y agudos y la salida es `g·graves + agudos` (las dos mitades
//! suman plano: sin reducción solo queda el giro de fase del cruce).
//!
//! Todo el estado es de tamaño fijo y vive en la ranura del efecto: sin
//! reservas, bloqueos ni E/S; sin NaN ni denormales (lo que cae por debajo
//! de un umbral minúsculo se pone a cero).

use crate::fx::{Biquad, FilterKind};

/// Qué dispara el ducking.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum DuckMode {
    /// Los golpes del secuenciador de la pista origen.
    #[default]
    Pattern,
    /// El nivel de la salida de la pista origen.
    Audio,
}

/// Curva de la vuelta (la relajación) del ducking por patrón.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum DuckCurve {
    /// En línea recta (en dB).
    #[default]
    Linear,
    /// Exponencial: vuelve rápido al principio y despacio al final.
    Exp,
    /// Coseno: despacio al principio y al final.
    Smooth,
}

impl DuckCurve {
    pub fn name(self) -> &'static str {
        match self {
            DuckCurve::Linear => "linear",
            DuckCurve::Exp => "exp",
            DuckCurve::Smooth => "smooth",
        }
    }

    pub fn from_name(name: &str) -> Option<Self> {
        Some(match name {
            "linear" => DuckCurve::Linear,
            "exp" => DuckCurve::Exp,
            "smooth" => DuckCurve::Smooth,
            _ => return None,
        })
    }

    /// Lo que queda de la reducción (de 1 a 0) a la fracción `u` de la
    /// relajación.
    fn shape(self, u: f32) -> f32 {
        let u = u.clamp(0.0, 1.0);
        match self {
            DuckCurve::Linear => 1.0 - u,
            DuckCurve::Exp => {
                let end = (-EXP_K).exp();
                ((-EXP_K * u).exp() - end) / (1.0 - end)
            }
            DuckCurve::Smooth => 0.5 * (1.0 + (std::f32::consts::PI * u).cos()),
        }
    }
}

/// Lo rápida que es la curva `exp` (llega a 0 justo al final).
const EXP_K: f32 = 5.0;

/// De dónde escucha la pista origen.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum DuckTap {
    /// Tras sus efectos y antes de su volumen, pan y mute: en mute (o fuera
    /// del solo) sigue disparando.
    #[default]
    Pre,
    /// Lo que se oye de ella (tras volumen, pan, mute y solo): en mute no
    /// dispara.
    Post,
}

/// Parámetros del ducking ([`crate::Fx::Duck`]).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DuckParams {
    pub mode: DuckMode,
    /// Pista del motor que dispara (la resuelve el script por su nombre);
    /// `None`, ninguna: no baja nunca.
    pub source: Option<usize>,
    /// Por patrón, lo que baja con cada golpe; por audio, lo más que baja
    /// (dB, negativo).
    pub depth_db: f32,
    pub attack_ms: f32,
    pub release_ms: f32,
    pub curve: DuckCurve,
    pub threshold_db: f32,
    pub ratio: f32,
    pub knee_db: f32,
    /// Paso bajo del detector (Hz); 0, sin filtro.
    pub filter_hz: f32,
    pub tap: DuckTap,
    /// Cruce (Hz): solo baja lo que queda por debajo; 0, la pista entera.
    pub band_hz: f32,
}

impl DuckParams {
    /// Por patrón, disparado por `source`: −9 dB, 5 ms de ataque y 180 de
    /// relajación en línea recta, la pista entera, de antes del volumen.
    pub fn new(source: usize) -> Self {
        DuckParams {
            mode: DuckMode::Pattern,
            source: Some(source),
            depth_db: -9.0,
            attack_ms: 5.0,
            release_ms: 180.0,
            curve: DuckCurve::Linear,
            threshold_db: -30.0,
            ratio: 4.0,
            knee_db: 6.0,
            filter_hz: 0.0,
            tap: DuckTap::Pre,
            band_hz: 0.0,
        }
    }

    /// Los mismos, disparados por el audio de la pista origen.
    pub fn audio(mut self) -> Self {
        self.mode = DuckMode::Audio;
        self
    }

    /// Comprueba los rangos.
    pub fn validate(&self) -> Result<(), &'static str> {
        let within = |v: f32, lo: f32, hi: f32| (lo..=hi).contains(&v);
        if !within(self.depth_db, DEPTH_RANGE.0, DEPTH_RANGE.1) {
            return Err("depth fuera de rango (-60 a 0 dB)");
        }
        if !within(self.attack_ms, ATTACK_RANGE.0, ATTACK_RANGE.1) {
            return Err("attack fuera de rango (0 a 500 ms)");
        }
        if !within(self.release_ms, RELEASE_RANGE.0, RELEASE_RANGE.1) {
            return Err("release fuera de rango (1 a 5000 ms)");
        }
        if !within(self.threshold_db, -60.0, 0.0) {
            return Err("threshold fuera de rango (-60 a 0 dB)");
        }
        if !within(self.ratio, 1.0, 20.0) {
            return Err("ratio fuera de rango (1 a 20)");
        }
        if !within(self.knee_db, 0.0, 24.0) {
            return Err("knee fuera de rango (0 a 24 dB)");
        }
        if self.filter_hz != 0.0 && !within(self.filter_hz, 20.0, 20_000.0) {
            return Err("filter fuera de rango (20 a 20000 Hz)");
        }
        if self.band_hz != 0.0 && !within(self.band_hz, BAND_RANGE.0, BAND_RANGE.1) {
            return Err("band fuera de rango (20 a 2000 Hz)");
        }
        if self.source.is_some_and(|t| t >= crate::engine::MAX_TRACKS) {
            return Err("sidechain: esa pista no existe");
        }
        Ok(())
    }
}

pub const DEPTH_RANGE: (f32, f32) = (-60.0, 0.0);
pub const ATTACK_RANGE: (f32, f32) = (0.0, 500.0);
pub const RELEASE_RANGE: (f32, f32) = (1.0, 5000.0);
pub const BAND_RANGE: (f32, f32) = (20.0, 2000.0);

/// Ventana del detector RMS.
pub const RMS_MS: f32 = 10.0;
/// Frames con la misma cuenta de la reducción por audio (la ganancia se
/// interpola dentro).
const BLOCK: usize = 8;
/// Lo que tarda la profundidad en llegar a un valor nuevo (al recargar o
/// automatizarla): sin escalones.
const DEPTH_SMOOTHING_MS: f32 = 5.0;
/// Por debajo, cero (sin denormales).
const TINY: f32 = 1e-20;

/// Fase de la envolvente por patrón.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
enum Phase {
    #[default]
    Idle,
    /// Subiendo desde `from` (en `DuckState::from`).
    Attack,
    Release,
}

/// Estado del ducking en el hilo de audio (tamaño fijo).
#[derive(Debug, Clone, Copy)]
pub(crate) struct DuckState {
    pub(crate) params: DuckParams,
    sample_rate: u32,
    /// Por patrón: fase, frames dentro de ella, desde dónde subió y lo que
    /// baja ahora (fracción de 0 a 1 de la profundidad).
    phase: Phase,
    pos: u32,
    from: f32,
    r: f32,
    /// La profundidad que suena (dB, negativa): sigue a la del patrón.
    depth: f32,
    depth_coef: f32,
    /// Por audio: el filtro del detector, la media de los cuadrados, la
    /// reducción que suena (dB, 0 o más) y sus coeficientes por bloque.
    filter: Biquad,
    filter_z: [f32; 2],
    ms: f32,
    ms_coef: f32,
    applied: f32,
    attack_block: f32,
    release_block: f32,
    /// La ganancia del final del último bloque (lineal).
    gain: f32,
    /// El cruce: paso bajo y paso alto (cada uno, dos Butterworth en
    /// serie) y su memoria por filtro, etapa y canal.
    cross_low: Biquad,
    cross_high: Biquad,
    cross_z: [[[[f32; 2]; 2]; 2]; 2],
}

impl DuckState {
    pub(crate) fn new(params: DuckParams, sample_rate: u32) -> Self {
        let mut state = DuckState {
            params,
            sample_rate,
            phase: Phase::Idle,
            pos: 0,
            from: 0.0,
            r: 0.0,
            depth: params.depth_db,
            depth_coef: 1.0 - (-1.0 / (DEPTH_SMOOTHING_MS / 1000.0 * sample_rate as f32)).exp(),
            filter: Biquad::default(),
            filter_z: [0.0; 2],
            ms: 0.0,
            ms_coef: 1.0 - (-1.0 / (RMS_MS / 1000.0 * sample_rate as f32)).exp(),
            applied: 0.0,
            attack_block: 0.0,
            release_block: 0.0,
            gain: 1.0,
            cross_low: Biquad::default(),
            cross_high: Biquad::default(),
            cross_z: [[[[0.0; 2]; 2]; 2]; 2],
        };
        state.retune();
        state
    }

    /// Recalcula lo que sale de los parámetros (filtros y coeficientes).
    pub(crate) fn retune(&mut self) {
        let p = self.params;
        let rate = self.sample_rate;
        if p.filter_hz > 0.0 {
            self.filter = Biquad::new(
                FilterKind::LowPass,
                p.filter_hz,
                std::f32::consts::FRAC_1_SQRT_2,
                rate,
            );
        }
        if p.band_hz > 0.0 {
            let q = std::f32::consts::FRAC_1_SQRT_2;
            self.cross_low = Biquad::new(FilterKind::LowPass, p.band_hz, q, rate);
            self.cross_high = Biquad::new(FilterKind::HighPass, p.band_hz, q, rate);
        }
        let block = |ms: f32| {
            if ms <= 0.0 {
                0.0
            } else {
                (-(BLOCK as f32) / (ms / 1000.0 * rate as f32)).exp()
            }
        };
        self.attack_block = block(p.attack_ms);
        self.release_block = block(p.release_ms);
    }

    /// Los parámetros nuevos de un patrón que entra en la misma ranura:
    /// sigue la envolvente (sin saltos). Un cambio de modo pasa la
    /// reducción de uno al otro.
    pub(crate) fn take_over(&mut self, old: &DuckState) {
        self.phase = old.phase;
        self.pos = old.pos;
        self.from = old.from;
        self.r = old.r;
        self.depth = old.depth;
        self.filter_z = if old.params.filter_hz > 0.0 && self.params.filter_hz > 0.0 {
            old.filter_z
        } else {
            [0.0; 2]
        };
        self.ms = old.ms;
        self.applied = old.applied;
        self.gain = old.gain;
        if old.params.band_hz > 0.0 && self.params.band_hz > 0.0 {
            self.cross_z = old.cross_z;
        }
        match (old.params.mode, self.params.mode) {
            (DuckMode::Pattern, DuckMode::Audio) => {
                self.applied = -old.depth * old.r;
            }
            (DuckMode::Audio, DuckMode::Pattern) => {
                let r = if self.depth < 0.0 {
                    (old.applied / -self.depth).clamp(0.0, 1.0)
                } else {
                    0.0
                };
                self.r = r;
                if r > 0.0 {
                    // Vuelve desde ahí, como si estuviera relajándose.
                    self.phase = Phase::Release;
                    let frames = self.release_frames();
                    self.pos = ((1.0 - r) * frames as f32) as u32;
                } else {
                    self.phase = Phase::Idle;
                }
            }
            _ => {}
        }
    }

    fn attack_frames(&self) -> u32 {
        (self.params.attack_ms / 1000.0 * self.sample_rate as f32).round() as u32
    }

    fn release_frames(&self) -> u32 {
        ((self.params.release_ms / 1000.0 * self.sample_rate as f32).round() as u32).max(1)
    }

    /// Lo que baja ahora (dB, 0 o más), para la vista del sidechain.
    pub(crate) fn reduction(&self) -> f32 {
        match self.params.mode {
            DuckMode::Pattern => -self.depth * self.r,
            DuckMode::Audio => self.applied,
        }
    }

    /// En silencio (la pista que lo lleva no suena): la envolvente y el
    /// detector siguen su curso `frames` frames (un golpe del origen sigue
    /// contando para cuando vuelva a sonar), y el cruce se vacía.
    pub(crate) fn rest(&mut self, frames: usize, key: Option<&[f32]>, hit: bool) {
        self.cross_z = [[[[0.0; 2]; 2]; 2]; 2];
        match self.params.mode {
            DuckMode::Pattern => {
                if hit {
                    self.trigger();
                }
                for _ in 0..frames {
                    if self.phase == Phase::Idle {
                        break;
                    }
                    self.advance();
                }
                self.depth = self.params.depth_db;
            }
            DuckMode::Audio => {
                let n = key.map_or(0, |k| k.len() / 2).min(frames);
                for b in (0..n).step_by(BLOCK) {
                    let end = (b + BLOCK).min(n);
                    let k = key.unwrap_or(&[]);
                    self.detect(&k[2 * b..2 * end]);
                    self.depth = self.params.depth_db;
                    self.smooth_block(end - b);
                }
                if n < frames {
                    // La clave calla: el detector cae como en silencio.
                    self.detect_silence(frames - n);
                    self.smooth_block(frames - n);
                }
                self.gain = db_to_gain(-self.applied);
            }
        }
    }

    /// Un golpe del origen: sube desde donde esté.
    fn trigger(&mut self) {
        self.from = self.r;
        self.pos = 0;
        if self.attack_frames() == 0 {
            self.r = 1.0;
            self.phase = Phase::Release;
        } else {
            self.phase = Phase::Attack;
        }
    }

    /// Un frame de la envolvente por patrón.
    #[inline]
    fn advance(&mut self) {
        match self.phase {
            Phase::Idle => {}
            Phase::Attack => {
                self.pos += 1;
                let n = self.attack_frames().max(1);
                if self.pos >= n {
                    self.r = 1.0;
                    self.phase = Phase::Release;
                    self.pos = 0;
                } else {
                    self.r = self.from + (1.0 - self.from) * self.pos as f32 / n as f32;
                }
            }
            Phase::Release => {
                self.pos += 1;
                let n = self.release_frames();
                if self.pos >= n {
                    self.r = 0.0;
                    self.phase = Phase::Idle;
                    self.pos = 0;
                } else {
                    self.r = self.params.curve.shape(self.pos as f32 / n as f32);
                }
            }
        }
    }

    /// El detector sobre un trozo de la clave (estéreo entrelazado).
    fn detect(&mut self, key: &[f32]) {
        let filtered = self.params.filter_hz > 0.0;
        let f = self.filter;
        let mut z = self.filter_z;
        let mut ms = self.ms;
        let c = self.ms_coef;
        for frame in key.as_chunks::<2>().0 {
            let mut x = 0.5 * (frame[0] + frame[1]);
            // Lo que no es finito no entra (un NaN se quedaría para
            // siempre en el detector); lo enorme, recortado.
            if !x.is_finite() {
                x = 0.0;
            }
            let x = x.clamp(-1e6, 1e6);
            let y = if filtered {
                let y = f.b0 * x + z[0];
                z[0] = f.b1 * x - f.a1 * y + z[1];
                z[1] = f.b2 * x - f.a2 * y;
                y
            } else {
                x
            };
            ms += (y * y - ms) * c;
        }
        for v in &mut z {
            if v.abs() < TINY {
                *v = 0.0;
            }
        }
        if ms < TINY {
            ms = 0.0;
        }
        self.filter_z = z;
        self.ms = ms;
    }

    /// El detector con `frames` de silencio.
    fn detect_silence(&mut self, frames: usize) {
        self.ms *= (1.0 - self.ms_coef).powi(frames as i32);
        if self.ms < TINY {
            self.ms = 0.0;
        }
        self.filter_z = [0.0; 2];
    }

    /// La reducción que pide el nivel de ahora (dB, 0 o más, como mucho la
    /// profundidad).
    fn target(&self) -> f32 {
        let p = self.params;
        if self.ms <= 0.0 {
            return 0.0;
        }
        // Calibrado con un seno: amplitud A, media de cuadrados A²/2.
        let level = 10.0 * (2.0 * self.ms).log10();
        let over = level - p.threshold_db;
        let slope = 1.0 - 1.0 / p.ratio;
        let w = p.knee_db;
        let reduction = if w > 0.0 && 2.0 * over.abs() <= w {
            slope * (over + w / 2.0).powi(2) / (2.0 * w)
        } else if over > 0.0 {
            slope * over
        } else {
            0.0
        };
        reduction.clamp(0.0, (-self.depth).max(0.0))
    }

    /// Un bloque de `frames` de la reducción por audio hacia lo que pide el
    /// detector.
    fn smooth_block(&mut self, frames: usize) {
        let target = self.target();
        let coef = if target > self.applied {
            self.attack_block
        } else {
            self.release_block
        };
        let coef = if frames == BLOCK {
            coef
        } else {
            coef.powf(frames as f32 / BLOCK as f32)
        };
        self.applied = target + coef * (self.applied - target);
        // Medio centésimo de dB no se oye (y la ganancia va interpolada en
        // el bloque): de ahí, a cero, y la pista vuelve a ser la de siempre.
        if self.applied < 0.005 && target == 0.0 {
            self.applied = 0.0;
        }
    }

    /// Procesa `buf` (estéreo entrelazado): por patrón, `hit` dice si el
    /// origen ha dado un golpe al principio de este trozo; por audio, `key`
    /// es lo que se escucha del origen (los mismos frames; `None` o vacía,
    /// calla).
    pub(crate) fn process(&mut self, buf: &mut [f32], key: Option<&[f32]>, hit: bool) {
        match self.params.mode {
            DuckMode::Pattern => self.process_pattern(buf, hit),
            DuckMode::Audio => self.process_audio(buf, key),
        }
    }

    fn process_pattern(&mut self, buf: &mut [f32], hit: bool) {
        if hit {
            self.trigger();
        }
        let target = self.params.depth_db;
        let k = std::f32::consts::LN_10 / 20.0;
        let band = self.params.band_hz > 0.0;
        for frame in buf.as_chunks_mut::<2>().0 {
            if self.phase == Phase::Idle && self.depth == target {
                // Sin reducción: la pista tal cual (con la banda, el cruce
                // sigue filtrando para no arrancar en frío).
                if band {
                    self.apply(frame, 1.0);
                }
                continue;
            }
            self.depth += (target - self.depth) * self.depth_coef;
            if (self.depth - target).abs() < 1e-4 {
                self.depth = target;
            }
            let gain = (k * self.depth * self.r).exp();
            self.apply(frame, gain);
            self.advance();
        }
        self.gain = (k * self.depth * self.r).exp();
    }

    fn process_audio(&mut self, buf: &mut [f32], key: Option<&[f32]>) {
        let band = self.params.band_hz > 0.0;
        let target_depth = self.params.depth_db;
        for (i, sub) in buf.chunks_mut(2 * BLOCK).enumerate() {
            let frames = sub.len() / 2;
            let from = i * 2 * BLOCK;
            match key.and_then(|k| k.get(from..from + sub.len())) {
                Some(k) => self.detect(k),
                None => self.detect_silence(frames),
            }
            // La profundidad (el tope) también sin escalones.
            self.depth +=
                (target_depth - self.depth) * (1.0 - (1.0 - self.depth_coef).powi(frames as i32));
            if (self.depth - target_depth).abs() < 1e-4 {
                self.depth = target_depth;
            }
            self.smooth_block(frames);
            let end = db_to_gain(-self.applied);
            let start = self.gain;
            self.gain = end;
            if start == 1.0 && end == 1.0 {
                if band {
                    for frame in sub.as_chunks_mut::<2>().0 {
                        self.apply(frame, 1.0);
                    }
                }
                continue;
            }
            let step = (end - start) / frames as f32;
            for (j, frame) in sub.as_chunks_mut::<2>().0.iter_mut().enumerate() {
                self.apply(frame, start + step * (j + 1) as f32);
            }
        }
    }

    /// Aplica `gain` a un frame: a todo, o solo a lo de debajo del cruce.
    #[inline]
    fn apply(&mut self, frame: &mut [f32; 2], gain: f32) {
        if self.params.band_hz > 0.0 {
            let low = run_twice(self.cross_low, &mut self.cross_z[0], *frame);
            let high = run_twice(self.cross_high, &mut self.cross_z[1], *frame);
            frame[0] = gain * low[0] + high[0];
            frame[1] = gain * low[1] + high[1];
        } else {
            frame[0] *= gain;
            frame[1] *= gain;
        }
    }

    /// Vacía la memoria (tras un NaN de la pista).
    pub(crate) fn reset(&mut self) {
        self.phase = Phase::Idle;
        self.pos = 0;
        self.r = 0.0;
        self.from = 0.0;
        self.depth = self.params.depth_db;
        self.filter_z = [0.0; 2];
        self.ms = 0.0;
        self.applied = 0.0;
        self.gain = 1.0;
        self.cross_z = [[[[0.0; 2]; 2]; 2]; 2];
    }

    /// Cambia un parámetro automatizable (ver `Fx::params`).
    pub(crate) fn set_param(&mut self, index: usize, value: f32) {
        let p = &mut self.params;
        match index {
            0 => p.depth_db = value,
            1 => p.attack_ms = value,
            2 => p.release_ms = value,
            3 => p.threshold_db = value,
            4 => p.ratio = value,
            5 => p.knee_db = value,
            _ => return,
        }
        if matches!(index, 1 | 2) {
            self.retune();
        }
    }
}

/// Un frame por dos etapas del biquad `c` (con la memoria de cada una y
/// cada canal), sin denormales.
#[inline]
fn run_twice(c: Biquad, z: &mut [[[f32; 2]; 2]; 2], frame: [f32; 2]) -> [f32; 2] {
    let mut out = frame;
    for stage in z.iter_mut() {
        for (x, z) in out.iter_mut().zip(stage.iter_mut()) {
            let y = c.b0 * *x + z[0];
            z[0] = c.b1 * *x - c.a1 * y + z[1];
            z[1] = c.b2 * *x - c.a2 * y;
            if z[0].abs() < TINY {
                z[0] = 0.0;
            }
            if z[1].abs() < TINY {
                z[1] = 0.0;
            }
            *x = y;
        }
    }
    out
}

/// dB a ganancia lineal (0 dB, 1 exacto).
fn db_to_gain(db: f32) -> f32 {
    if db == 0.0 {
        1.0
    } else {
        10f32.powf(db / 20.0)
    }
}
