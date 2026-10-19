//! Efectos de pista: filtro paso bajo/alto, bitcrusher, compresor,
//! distorsión, EQ paramétrico y EQ dinámico (con sidechain de otra pista).
//!
//! La configuración ([`Fx`]) va dentro del `Pattern` y entra con él al
//! empezar el compás. El estado ([`FxState`]) vive en la pista, en el hilo
//! de audio, en tamaño fijo: aplicar un efecto no reserva memoria.

use std::f32::consts::TAU;

/// Efectos por pista como máximo (se aplican en orden).
pub const MAX_FX: usize = 4;

/// Parámetros automatizables por efecto como máximo (los del EQ: frecuencia,
/// ganancia y q de cada banda).
pub const MAX_PARAMS: usize = 3 * EQ_BANDS;

/// Bandas del EQ paramétrico.
pub const EQ_BANDS: usize = 4;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FilterKind {
    LowPass,
    HighPass,
}

/// Forma de una banda del EQ.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EqKind {
    /// Campana alrededor de la frecuencia (sube o baja una zona).
    Bell,
    /// Estante grave: todo lo que queda por debajo de la frecuencia.
    LowShelf,
    /// Estante agudo: todo lo que queda por encima.
    HighShelf,
}

impl EqKind {
    /// Nombre en el script.
    pub fn name(self) -> &'static str {
        match self {
            EqKind::Bell => "bell",
            EqKind::LowShelf => "lowshelf",
            EqKind::HighShelf => "highshelf",
        }
    }

    pub fn from_name(name: &str) -> Option<Self> {
        [EqKind::Bell, EqKind::LowShelf, EqKind::HighShelf]
            .into_iter()
            .find(|k| k.name() == name)
    }
}

/// Una banda del EQ. Con `gain_db` 0 no hace nada (ni gasta CPU).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct EqBand {
    pub kind: EqKind,
    pub freq_hz: f32,
    pub gain_db: f32,
    pub q: f32,
}

/// Frecuencias por defecto de las bandas: una por zona (graves, medios
/// graves, medios agudos y agudos).
pub const EQ_DEFAULT_FREQS: [f32; EQ_BANDS] = [100.0, 500.0, 2000.0, 8000.0];

/// Q por defecto de una banda: una octava y media de ancho en campana, el
/// estante más suave sin rebote en los estantes.
pub const EQ_DEFAULT_Q: f32 = 1.0;

/// Rangos de los parámetros de una banda del EQ.
pub const EQ_FREQ_RANGE: (f32, f32) = (20.0, 20_000.0);
pub const EQ_GAIN_RANGE: (f32, f32) = (-24.0, 24.0);
pub const EQ_Q_RANGE: (f32, f32) = (0.1, 18.0);

impl EqBand {
    /// Banda `i` sin efecto (ganancia 0), con su frecuencia por defecto.
    pub fn flat(i: usize) -> Self {
        EqBand {
            kind: EqKind::Bell,
            freq_hz: EQ_DEFAULT_FREQS[i],
            gain_db: 0.0,
            q: EQ_DEFAULT_Q,
        }
    }

    pub fn bell(freq_hz: f32, gain_db: f32, q: f32) -> Self {
        EqBand {
            kind: EqKind::Bell,
            freq_hz,
            gain_db,
            q,
        }
    }
}

/// Un efecto de pista.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Fx {
    /// Biquad de 12 dB/octava. `q` = 0.707 es la respuesta más plana.
    Filter {
        kind: FilterKind,
        cutoff_hz: f32,
        q: f32,
    },
    /// Reduce la resolución a `bits` y repite cada muestra `downsample`
    /// veces (reducción de frecuencia de muestreo sin filtro).
    Bitcrush { bits: u32, downsample: u32 },
    /// Compresor estéreo enlazado: por encima de `threshold_db` el nivel
    /// sube `1/ratio` dB por cada dB de entrada; la reducción llega en
    /// `attack_ms` y se va en `release_ms`; `makeup_db` sube la salida.
    Compressor {
        threshold_db: f32,
        ratio: f32,
        attack_ms: f32,
        release_ms: f32,
        makeup_db: f32,
    },
    /// Saturación con `tanh(drive · x)`, mezclada con la señal limpia
    /// (`mix` 1 = solo la saturada).
    Distortion { drive: f32, mix: f32 },
    /// EQ paramétrico: hasta [`EQ_BANDS`] bandas en serie (biquads RBJ).
    /// Las que tienen ganancia 0 no se procesan.
    Eq { bands: [EqBand; EQ_BANDS] },
    /// EQ dinámico de una banda: un detector en la misma banda (campana de
    /// paso de banda en `freq_hz` y `q`) escucha la propia pista o, con
    /// `sidechain`, otra pista del motor (su salida tras sus efectos y antes
    /// de su volumen, aunque esté en mute); cuando pasa de `threshold_db`,
    /// la banda baja `1 − 1/ratio` dB por dB de más, como mucho `range_db`,
    /// con `attack_ms` y `release_ms`.
    DynEq(DynEqParams),
}

/// Parámetros del EQ dinámico ([`Fx::DynEq`]).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DynEqParams {
    pub kind: EqKind,
    pub freq_hz: f32,
    pub q: f32,
    pub threshold_db: f32,
    pub ratio: f32,
    /// Lo más que baja la banda, en dB (positivo).
    pub range_db: f32,
    pub attack_ms: f32,
    pub release_ms: f32,
    /// Pista del motor que dispara la reducción; `None`, la propia.
    pub sidechain: Option<usize>,
}

impl DynEqParams {
    /// Campana en `freq_hz` con los valores por defecto: q 1, umbral −30 dB,
    /// ratio 4, como mucho 6 dB, ataque 5 ms y release 120 ms, sin
    /// sidechain.
    pub fn new(freq_hz: f32) -> Self {
        DynEqParams {
            kind: EqKind::Bell,
            freq_hz,
            q: EQ_DEFAULT_Q,
            threshold_db: -30.0,
            ratio: 4.0,
            range_db: 6.0,
            attack_ms: 5.0,
            release_ms: 120.0,
            sidechain: None,
        }
    }

    /// La misma, disparada por la pista `track` del motor.
    pub fn with_sidechain(mut self, track: usize) -> Self {
        self.sidechain = Some(track);
        self
    }
}

pub const DEFAULT_Q: f32 = std::f32::consts::FRAC_1_SQRT_2;

impl Fx {
    pub fn lowpass(cutoff_hz: f32) -> Self {
        Fx::Filter {
            kind: FilterKind::LowPass,
            cutoff_hz,
            q: DEFAULT_Q,
        }
    }

    pub fn highpass(cutoff_hz: f32) -> Self {
        Fx::Filter {
            kind: FilterKind::HighPass,
            cutoff_hz,
            q: DEFAULT_Q,
        }
    }

    pub fn bitcrush(bits: u32, downsample: u32) -> Self {
        Fx::Bitcrush { bits, downsample }
    }

    /// Compresor con ataque de 5 ms, release de 100 ms y sin makeup.
    pub fn compressor(threshold_db: f32, ratio: f32) -> Self {
        Fx::Compressor {
            threshold_db,
            ratio,
            attack_ms: 5.0,
            release_ms: 100.0,
            makeup_db: 0.0,
        }
    }

    pub fn distortion(drive: f32, mix: f32) -> Self {
        Fx::Distortion { drive, mix }
    }

    /// EQ con las bandas dadas (como mucho [`EQ_BANDS`]); las demás, planas.
    pub fn eq(bands: &[EqBand]) -> Self {
        Fx::Eq {
            bands: std::array::from_fn(|i| bands.get(i).copied().unwrap_or(EqBand::flat(i))),
        }
    }

    /// Parámetros que se pueden automatizar (con `ramp`, `lfo` o, el corte,
    /// por paso): su nombre en el script, en el orden de su índice.
    pub fn params(&self) -> &'static [&'static str] {
        match self {
            Fx::Filter { .. } => &["cutoff", "q"],
            Fx::Compressor { .. } => &["threshold", "makeup"],
            Fx::Distortion { .. } => &["drive", "mix"],
            Fx::Bitcrush { .. } => &[],
            Fx::Eq { .. } => &[
                "freq1", "gain1", "q1", "freq2", "gain2", "q2", "freq3", "gain3", "q3", "freq4",
                "gain4", "q4",
            ],
            Fx::DynEq(_) => &["freq", "q", "threshold", "range"],
        }
    }

    /// Pista del motor que escucha este efecto (sidechain), si escucha
    /// otra.
    pub fn sidechain(&self) -> Option<usize> {
        match self {
            Fx::DynEq(p) => p.sidechain,
            _ => None,
        }
    }

    /// Índice del parámetro automatizable `name`.
    pub fn param_index(&self, name: &str) -> Option<usize> {
        self.params().iter().position(|p| *p == name)
    }

    /// Rango válido del parámetro `index` y si se mueve en escala
    /// logarítmica (las frecuencias).
    pub fn param_range(&self, index: usize) -> Option<(f32, f32, bool)> {
        Some(match (self, index) {
            (Fx::Filter { .. }, 0) => (20.0, 20_000.0, true),
            (Fx::Filter { .. }, 1) => (0.1, 20.0, true),
            (Fx::Compressor { .. }, 0) => (-60.0, 0.0, false),
            (Fx::Compressor { .. }, 1) => (0.0, 24.0, false),
            (Fx::Distortion { .. }, 0) => (1.0, 50.0, true),
            (Fx::Distortion { .. }, 1) => (0.0, 1.0, false),
            (Fx::Eq { .. }, i) if i < MAX_PARAMS => match i % 3 {
                0 => (EQ_FREQ_RANGE.0, EQ_FREQ_RANGE.1, true),
                1 => (EQ_GAIN_RANGE.0, EQ_GAIN_RANGE.1, false),
                _ => (EQ_Q_RANGE.0, EQ_Q_RANGE.1, true),
            },
            (Fx::DynEq(_), 0) => (EQ_FREQ_RANGE.0, EQ_FREQ_RANGE.1, true),
            (Fx::DynEq(_), 1) => (EQ_Q_RANGE.0, EQ_Q_RANGE.1, true),
            (Fx::DynEq(_), 2) => (-60.0, 0.0, false),
            (Fx::DynEq(_), 3) => (0.0, 24.0, false),
            _ => return None,
        })
    }

    /// Valor fijo del parámetro `index` (el que se escribió en la tabla).
    pub fn param_value(&self, index: usize) -> Option<f32> {
        Some(match (*self, index) {
            (Fx::Filter { cutoff_hz, .. }, 0) => cutoff_hz,
            (Fx::Filter { q, .. }, 1) => q,
            (Fx::Compressor { threshold_db, .. }, 0) => threshold_db,
            (Fx::Compressor { makeup_db, .. }, 1) => makeup_db,
            (Fx::Distortion { drive, .. }, 0) => drive,
            (Fx::Distortion { mix, .. }, 1) => mix,
            (Fx::Eq { bands }, i) if i < MAX_PARAMS => {
                let band = bands[i / 3];
                [band.freq_hz, band.gain_db, band.q][i % 3]
            }
            (Fx::DynEq(p), i) if i < 4 => [p.freq_hz, p.q, p.threshold_db, p.range_db][i],
            _ => return None,
        })
    }

    /// Comprueba los rangos. El corte se limita además por debajo de la
    /// frecuencia de Nyquist al aplicarse.
    pub fn validate(&self) -> Result<(), &'static str> {
        match *self {
            Fx::Filter { cutoff_hz, q, .. } => {
                if !(20.0..=20_000.0).contains(&cutoff_hz) {
                    return Err("frecuencia de corte fuera de rango (20 a 20000 Hz)");
                }
                if !(0.1..=20.0).contains(&q) {
                    return Err("resonancia (q) fuera de rango (0.1 a 20)");
                }
            }
            Fx::Bitcrush { bits, downsample } => {
                if !(1..=16).contains(&bits) {
                    return Err("bits fuera de rango (1 a 16)");
                }
                if !(1..=64).contains(&downsample) {
                    return Err("downsample fuera de rango (1 a 64)");
                }
            }
            Fx::Compressor {
                threshold_db,
                ratio,
                attack_ms,
                release_ms,
                makeup_db,
            } => {
                if !(-60.0..=0.0).contains(&threshold_db) {
                    return Err("threshold fuera de rango (-60 a 0 dB)");
                }
                if !(1.0..=20.0).contains(&ratio) {
                    return Err("ratio fuera de rango (1 a 20)");
                }
                if !(0.1..=200.0).contains(&attack_ms) {
                    return Err("attack fuera de rango (0.1 a 200 ms)");
                }
                if !(1.0..=2000.0).contains(&release_ms) {
                    return Err("release fuera de rango (1 a 2000 ms)");
                }
                if !(0.0..=24.0).contains(&makeup_db) {
                    return Err("makeup fuera de rango (0 a 24 dB)");
                }
            }
            Fx::Distortion { drive, mix } => {
                if !(1.0..=50.0).contains(&drive) {
                    return Err("drive fuera de rango (1 a 50)");
                }
                if !(0.0..=1.0).contains(&mix) {
                    return Err("mix fuera de rango (0 a 1)");
                }
            }
            Fx::Eq { bands } => {
                let within = |v: f32, (lo, hi): (f32, f32)| (lo..=hi).contains(&v);
                for band in bands {
                    if !within(band.freq_hz, EQ_FREQ_RANGE) {
                        return Err("freq fuera de rango (20 a 20000 Hz)");
                    }
                    if !within(band.gain_db, EQ_GAIN_RANGE) {
                        return Err("gain fuera de rango (-24 a 24 dB)");
                    }
                    if !within(band.q, EQ_Q_RANGE) {
                        return Err("q fuera de rango (0.1 a 18)");
                    }
                }
            }
            Fx::DynEq(p) => {
                let within = |v: f32, (lo, hi): (f32, f32)| (lo..=hi).contains(&v);
                if !within(p.freq_hz, EQ_FREQ_RANGE) {
                    return Err("freq fuera de rango (20 a 20000 Hz)");
                }
                if !within(p.q, EQ_Q_RANGE) {
                    return Err("q fuera de rango (0.1 a 18)");
                }
                if !(-60.0..=0.0).contains(&p.threshold_db) {
                    return Err("threshold fuera de rango (-60 a 0 dB)");
                }
                if !(1.0..=20.0).contains(&p.ratio) {
                    return Err("ratio fuera de rango (1 a 20)");
                }
                if !(0.0..=24.0).contains(&p.range_db) {
                    return Err("range fuera de rango (0 a 24 dB)");
                }
                if !(0.1..=200.0).contains(&p.attack_ms) {
                    return Err("attack fuera de rango (0.1 a 200 ms)");
                }
                if !(1.0..=2000.0).contains(&p.release_ms) {
                    return Err("release fuera de rango (1 a 2000 ms)");
                }
                if p.sidechain.is_some_and(|t| t >= crate::engine::MAX_TRACKS) {
                    return Err("sidechain: esa pista no existe");
                }
            }
        }
        Ok(())
    }
}

/// Estado de un efecto en el hilo de audio.
// El EQ ocupa más que los demás, pero en una `Box` reservaría memoria al
// cambiar de patrón en el hilo de audio: tamaño fijo para todos.
#[allow(clippy::large_enum_variant)]
#[derive(Debug, Clone, Copy, Default)]
pub(crate) enum FxState {
    #[default]
    Off,
    Biquad {
        kind: FilterKind,
        cutoff_hz: f32,
        q: f32,
        coefs: Biquad,
        /// Memoria (z1, z2) de cada canal.
        z: [[f32; 2]; 2],
    },
    Crush {
        levels: f32,
        downsample: u32,
        counter: u32,
        held: [f32; 2],
    },
    Comp {
        threshold_db: f32,
        /// dB que se quitan por dB por encima del umbral: `1 - 1/ratio`.
        slope: f32,
        /// Coeficientes de la envolvente (por muestra) al subir y al bajar.
        attack: f32,
        release: f32,
        makeup_db: f32,
        /// Reducción actual, en dB (0 o más).
        reduction: f32,
    },
    Drive {
        drive: f32,
        mix: f32,
    },
    Eq {
        bands: [EqState; EQ_BANDS],
        sample_rate: u32,
    },
    Dyn(DynState),
}

/// Estado del EQ dinámico: el detector (paso de banda y envolvente), la
/// reducción que suena y la campana que la aplica.
#[derive(Debug, Clone, Copy)]
pub(crate) struct DynState {
    params: DynEqParams,
    sample_rate: u32,
    /// Coeficientes (por muestra) de la envolvente al subir y al bajar.
    attack: f32,
    release: f32,
    detect: Biquad,
    detect_z: [f32; 2],
    /// Envolvente del detector (lineal).
    envelope: f32,
    /// Reducción que suena ahora, en dB (0 o más).
    applied: f32,
    /// Frames que la campana, ya plana, sigue filtrando para que su memoria
    /// se asiente antes de saltársela (si no, su retraso de unos ms sería un
    /// escalón).
    settling: u32,
    coefs: Biquad,
    z: [[f32; 2]; 2],
}

/// Lo que sigue filtrando la campana plana del EQ dinámico antes de
/// saltársela: más de diez veces lo que tarda en asentarse a 20 Hz con q 1.
const DYN_SETTLE_SECONDS: f32 = 0.05;

impl DynState {
    fn new(params: DynEqParams, sample_rate: u32) -> Self {
        let coef = |ms: f32| (-1.0 / (ms / 1000.0 * sample_rate as f32)).exp();
        DynState {
            params,
            sample_rate,
            attack: coef(params.attack_ms),
            release: coef(params.release_ms),
            detect: Biquad::bandpass(params.freq_hz, params.q, sample_rate),
            detect_z: [0.0; 2],
            envelope: 0.0,
            applied: 0.0,
            settling: 0,
            coefs: Biquad::eq(params.kind, [params.freq_hz, 0.0, params.q], sample_rate),
            z: [[0.0; 2]; 2],
        }
    }

    /// Recalcula los filtros con la frecuencia, la q y la reducción de
    /// ahora.
    fn retune(&mut self) {
        let p = self.params;
        self.detect = Biquad::bandpass(p.freq_hz, p.q, self.sample_rate);
        self.coefs = Biquad::eq(p.kind, [p.freq_hz, -self.applied, p.q], self.sample_rate);
    }

    /// Procesa `buf` con la señal de detección `key` (la de otra pista, o
    /// `None`: la propia, antes del EQ), por trozos de [`DYN_BLOCK`] frames:
    /// la envolvente de cada trozo decide la reducción con la que suena.
    fn process(&mut self, buf: &mut [f32], key: Option<&[f32]>) {
        let p = self.params;
        let slope = 1.0 - 1.0 / p.ratio;
        for (i, sub) in buf.chunks_mut(2 * DYN_BLOCK).enumerate() {
            let from = i * 2 * DYN_BLOCK;
            let source: &[f32] = match key {
                Some(k) => k.get(from..from + sub.len()).unwrap_or(&[]),
                None => sub,
            };
            let d = self.detect;
            let mut envelope = self.envelope;
            let mut z = self.detect_z;
            for frame in source.as_chunks::<2>().0 {
                let x = 0.5 * (frame[0] + frame[1]);
                let y = d.b0 * x + z[0];
                z[0] = d.b1 * x - d.a1 * y + z[1];
                z[1] = d.b2 * x - d.a2 * y;
                let level = y.abs();
                let coef = if level > envelope {
                    self.attack
                } else {
                    self.release
                };
                envelope = level + coef * (envelope - level);
            }
            // Sin señal de detección en este trozo (la clave calla): la
            // envolvente cae como en silencio.
            if source.is_empty() {
                let frames = sub.len() / 2;
                envelope *= self.release.powi(frames as i32);
                z = [0.0; 2];
            }
            self.envelope = envelope;
            self.detect_z = z;
            let over = 20.0 * envelope.max(1e-9).log10() - p.threshold_db;
            let target = if over > 0.0 {
                (over * slope).min(p.range_db)
            } else {
                0.0
            };
            // Medio centésimo de dB no se oye: no se recalcula.
            if (target - self.applied).abs() > 0.005 || (target == 0.0 && self.applied != 0.0) {
                self.applied = target;
                if target == 0.0 {
                    self.settling = (DYN_SETTLE_SECONDS * self.sample_rate as f32) as u32;
                }
                self.retune();
            }
            if self.applied > 0.0 || self.settling > 0 {
                self.coefs.run(sub, &mut self.z);
                if self.applied == 0.0 {
                    self.settling = self.settling.saturating_sub((sub.len() / 2) as u32);
                    if self.settling == 0 {
                        // Ya asentada: fuera de camino, sin memoria.
                        self.z = [[0.0; 2]; 2];
                    }
                }
            }
        }
    }
}

/// Estado de una banda del EQ: los parámetros a los que va (`target`, los
/// del patrón o la automatización) y los que suenan (`current`), que los
/// siguen suavizados (~5 ms) para que recargar con otra ganancia no haga
/// clic.
#[derive(Debug, Clone, Copy)]
pub(crate) struct EqState {
    kind: EqKind,
    target: [f32; 3],
    current: [f32; 3],
    coefs: Biquad,
    z: [[f32; 2]; 2],
}

impl EqState {
    fn new(band: EqBand, sample_rate: u32) -> Self {
        let params = [band.freq_hz, band.gain_db, band.q];
        EqState {
            kind: band.kind,
            target: params,
            current: params,
            coefs: Biquad::eq(band.kind, params, sample_rate),
            z: [[0.0; 2]; 2],
        }
    }

    /// Sin ganancia ni camino hacia ella: no se procesa.
    fn bypassed(&self) -> bool {
        self.current[1] == 0.0 && self.target[1] == 0.0
    }

    fn settled(&self) -> bool {
        self.current == self.target
    }

    /// Un paso del suavizado (`keep`: lo que se conserva del valor actual);
    /// lo que queda a menos de una milésima llega ya a su destino.
    fn glide(&mut self, keep: f32, sample_rate: u32) {
        let before_bypass = self.current[1] != 0.0;
        for i in 0..3 {
            let (from, to) = (self.current[i], self.target[i]);
            let next = if i == 1 {
                to + keep * (from - to)
            } else {
                crate::automation::mix(to, from, keep, true)
            };
            let close = if i == 1 {
                (next - to).abs() < 1e-3
            } else {
                (next / to - 1.0).abs() < 1e-3
            };
            self.current[i] = if close { to } else { next };
        }
        if before_bypass && self.bypassed() {
            self.z = [[0.0; 2]; 2];
        }
        self.coefs = Biquad::eq(self.kind, self.current, sample_rate);
    }
}

/// Frames entre recálculos de la reducción del EQ dinámico: corto, para que
/// la campana cambie a pasos que no se oyen (con 32 se medía un escalón).
const DYN_BLOCK: usize = 8;

/// Frames entre recálculos de los coeficientes del EQ mientras se suaviza.
const EQ_BLOCK: usize = 32;

/// Constante de tiempo del suavizado del EQ (la de la automatización).
const EQ_SMOOTHING_SECONDS: f32 = 0.005;

#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct Biquad {
    b0: f32,
    b1: f32,
    b2: f32,
    a1: f32,
    a2: f32,
}

impl Biquad {
    /// Coeficientes RBJ (Audio EQ Cookbook), normalizados por a0.
    fn new(kind: FilterKind, cutoff_hz: f32, q: f32, sample_rate: u32) -> Self {
        let cutoff = cutoff_hz.min(0.45 * sample_rate as f32);
        let w0 = TAU * cutoff / sample_rate as f32;
        let (sin, cos) = w0.sin_cos();
        let alpha = sin / (2.0 * q);
        let a0 = 1.0 + alpha;
        let (b0, b1, b2) = match kind {
            FilterKind::LowPass => ((1.0 - cos) / 2.0, 1.0 - cos, (1.0 - cos) / 2.0),
            FilterKind::HighPass => ((1.0 + cos) / 2.0, -(1.0 + cos), (1.0 + cos) / 2.0),
        };
        Biquad {
            b0: b0 / a0,
            b1: b1 / a0,
            b2: b2 / a0,
            a1: -2.0 * cos / a0,
            a2: (1.0 - alpha) / a0,
        }
    }

    /// Campana o estante del Audio EQ Cookbook (RBJ) con `[frecuencia,
    /// ganancia en dB, q]`.
    fn eq(kind: EqKind, [freq_hz, gain_db, q]: [f32; 3], sample_rate: u32) -> Self {
        let freq = freq_hz.min(0.45 * sample_rate as f32);
        let w0 = TAU * freq / sample_rate as f32;
        let (sin, cos) = w0.sin_cos();
        let a = 10f32.powf(gain_db / 40.0);
        let alpha = sin / (2.0 * q);
        let (b0, b1, b2, a0, a1, a2) = match kind {
            EqKind::Bell => (
                1.0 + alpha * a,
                -2.0 * cos,
                1.0 - alpha * a,
                1.0 + alpha / a,
                -2.0 * cos,
                1.0 - alpha / a,
            ),
            EqKind::LowShelf | EqKind::HighShelf => {
                let sq = 2.0 * a.sqrt() * alpha;
                // El agudo es el grave con cos cambiado de signo (espejo).
                let (c, s1) = if kind == EqKind::LowShelf {
                    (cos, 1.0)
                } else {
                    (-cos, -1.0)
                };
                (
                    a * ((a + 1.0) - (a - 1.0) * c + sq),
                    s1 * 2.0 * a * ((a - 1.0) - (a + 1.0) * c),
                    a * ((a + 1.0) - (a - 1.0) * c - sq),
                    (a + 1.0) + (a - 1.0) * c + sq,
                    s1 * -2.0 * ((a - 1.0) + (a + 1.0) * c),
                    (a + 1.0) + (a - 1.0) * c - sq,
                )
            }
        };
        Biquad {
            b0: b0 / a0,
            b1: b1 / a0,
            b2: b2 / a0,
            a1: a1 / a0,
            a2: a2 / a0,
        }
    }

    /// Paso de banda (ganancia 1 en el centro) del Audio EQ Cookbook: el
    /// detector del EQ dinámico.
    fn bandpass(freq_hz: f32, q: f32, sample_rate: u32) -> Self {
        let freq = freq_hz.min(0.45 * sample_rate as f32);
        let w0 = TAU * freq / sample_rate as f32;
        let (sin, cos) = w0.sin_cos();
        let alpha = sin / (2.0 * q);
        let a0 = 1.0 + alpha;
        Biquad {
            b0: alpha / a0,
            b1: 0.0,
            b2: -alpha / a0,
            a1: -2.0 * cos / a0,
            a2: (1.0 - alpha) / a0,
        }
    }

    /// Filtra `buf` (estéreo entrelazado) con la memoria `z` de cada canal.
    fn run(self, buf: &mut [f32], z: &mut [[f32; 2]; 2]) {
        for frame in buf.as_chunks_mut::<2>().0 {
            for (x, z) in frame.iter_mut().zip(z.iter_mut()) {
                // Forma directa II transpuesta.
                let y = self.b0 * *x + z[0];
                z[0] = self.b1 * *x - self.a1 * y + z[1];
                z[1] = self.b2 * *x - self.a2 * y;
                *x = y;
            }
        }
    }
}

impl FxState {
    /// Estado para `fx`. Si el estado anterior es del mismo tipo, conserva
    /// su memoria (así cambiar el corte de un filtro no hace clic).
    pub fn update(self, fx: Option<Fx>, sample_rate: u32) -> Self {
        match (fx, self) {
            (None, _) => FxState::Off,
            (Some(Fx::Filter { kind, cutoff_hz, q }), previous) => {
                let coefs = Biquad::new(kind, cutoff_hz, q, sample_rate);
                let z = match previous {
                    FxState::Biquad { kind: k, z, .. } if k == kind => z,
                    _ => [[0.0; 2]; 2],
                };
                FxState::Biquad {
                    kind,
                    cutoff_hz,
                    q,
                    coefs,
                    z,
                }
            }
            (
                Some(Fx::Compressor {
                    threshold_db,
                    ratio,
                    attack_ms,
                    release_ms,
                    makeup_db,
                }),
                previous,
            ) => {
                let coef = |ms: f32| (-1.0 / (ms / 1000.0 * sample_rate as f32)).exp();
                FxState::Comp {
                    threshold_db,
                    slope: 1.0 - 1.0 / ratio,
                    attack: coef(attack_ms),
                    release: coef(release_ms),
                    makeup_db,
                    // Otro compresor en la misma ranura: sigue su envolvente.
                    reduction: match previous {
                        FxState::Comp { reduction, .. } => reduction,
                        _ => 0.0,
                    },
                }
            }
            (Some(Fx::Distortion { drive, mix }), _) => FxState::Drive { drive, mix },
            (Some(Fx::Eq { bands }), previous) => {
                let old = match previous {
                    FxState::Eq { bands, .. } => Some(bands),
                    _ => None,
                };
                FxState::Eq {
                    sample_rate,
                    bands: std::array::from_fn(|i| {
                        let mut state = EqState::new(bands[i], sample_rate);
                        // Otro EQ en la ranura: sigue sonando lo de antes y
                        // va hacia lo nuevo suavizado (con otra forma, ya).
                        if let Some(old) = old.map(|o| o[i])
                            && old.kind == state.kind
                        {
                            state.current = old.current;
                            state.z = old.z;
                            state.coefs = old.coefs;
                        }
                        state
                    }),
                }
            }
            (Some(Fx::DynEq(params)), previous) => {
                let mut state = DynState::new(params, sample_rate);
                // Otro EQ dinámico en la ranura: sigue con su envolvente, su
                // reducción y su memoria.
                if let FxState::Dyn(old) = previous
                    && old.params.kind == params.kind
                {
                    state.envelope = old.envelope;
                    state.applied = old.applied;
                    state.settling = old.settling;
                    state.detect_z = old.detect_z;
                    state.z = old.z;
                    state.retune();
                }
                FxState::Dyn(state)
            }
            (Some(Fx::Bitcrush { bits, downsample }), previous) => {
                let (counter, held) = match previous {
                    FxState::Crush { counter, held, .. } => (counter % downsample, held),
                    _ => (0, [0.0; 2]),
                };
                FxState::Crush {
                    levels: (1u32 << (bits - 1)) as f32,
                    downsample,
                    counter,
                    held,
                }
            }
        }
    }

    /// Cambia el parámetro automatizable `index` (ver [`Fx::params`]) sin
    /// tocar la memoria del efecto. Solo recalcula lo necesario.
    pub fn set_param(&mut self, index: usize, value: f32, sample_rate: u32) {
        match (self, index) {
            (
                FxState::Biquad {
                    kind,
                    cutoff_hz,
                    q,
                    coefs,
                    ..
                },
                0 | 1,
            ) => {
                let target = if index == 0 { &mut *cutoff_hz } else { &mut *q };
                if *target != value {
                    *target = value;
                    *coefs = Biquad::new(*kind, *cutoff_hz, *q, sample_rate);
                }
            }
            (FxState::Comp { threshold_db, .. }, 0) => *threshold_db = value,
            (FxState::Comp { makeup_db, .. }, 1) => *makeup_db = value,
            (FxState::Drive { drive, .. }, 0) => *drive = value,
            (FxState::Drive { mix, .. }, 1) => *mix = value,
            // La automatización llega ya suavizada: entra tal cual.
            (FxState::Eq { bands, .. }, i) if i < MAX_PARAMS => {
                let band = &mut bands[i / 3];
                if band.current[i % 3] != value || band.target[i % 3] != value {
                    band.target[i % 3] = value;
                    band.current[i % 3] = value;
                    band.coefs = Biquad::eq(band.kind, band.current, sample_rate);
                }
            }
            (FxState::Dyn(state), i) if i < 4 => {
                let p = &mut state.params;
                let target = match i {
                    0 => &mut p.freq_hz,
                    1 => &mut p.q,
                    2 => &mut p.threshold_db,
                    _ => &mut p.range_db,
                };
                if *target != value {
                    *target = value;
                    if i < 2 {
                        state.retune();
                    }
                }
            }
            _ => {}
        }
    }

    /// Vacía la memoria (pista en silencio).
    pub fn reset(&mut self) {
        match self {
            FxState::Off => {}
            FxState::Biquad { z, .. } => *z = [[0.0; 2]; 2],
            FxState::Crush { counter, held, .. } => {
                *counter = 0;
                *held = [0.0; 2];
            }
            FxState::Comp { reduction, .. } => *reduction = 0.0,
            FxState::Drive { .. } => {}
            // En silencio no hay nada que suavizar: los parámetros llegan ya.
            FxState::Eq { bands, sample_rate } => {
                for band in bands {
                    band.z = [[0.0; 2]; 2];
                    if !band.settled() {
                        band.glide(0.0, *sample_rate);
                    }
                }
            }
            FxState::Dyn(state) => {
                state.z = [[0.0; 2]; 2];
                state.detect_z = [0.0; 2];
                state.envelope = 0.0;
                state.settling = 0;
                if state.applied != 0.0 {
                    state.applied = 0.0;
                    state.retune();
                }
            }
        }
    }

    /// Pista del motor que escucha este efecto (sidechain), si escucha
    /// otra.
    pub fn sidechain(&self) -> Option<usize> {
        match self {
            FxState::Dyn(state) => state.params.sidechain,
            _ => None,
        }
    }

    /// Como [`FxState::process`], con la señal de la pista que escucha (el
    /// trozo de los mismos frames que `buf`) para el EQ dinámico con
    /// sidechain; vacía, esa pista calla.
    pub fn process_keyed(&mut self, buf: &mut [f32], key: Option<&[f32]>) {
        match self {
            FxState::Dyn(state) => state.process(buf, key),
            other => other.process(buf),
        }
    }

    /// Procesa `buf` (estéreo entrelazado) en su sitio. El EQ dinámico
    /// escucha aquí la propia pista; con sidechain, ver
    /// [`FxState::process_keyed`].
    pub fn process(&mut self, buf: &mut [f32]) {
        match self {
            FxState::Off => {}
            FxState::Dyn(state) => state.process(buf, None),
            FxState::Biquad { coefs, z, .. } => {
                let c = *coefs;
                for frame in buf.as_chunks_mut::<2>().0 {
                    for (x, z) in frame.iter_mut().zip(z.iter_mut()) {
                        // Forma directa II transpuesta.
                        let y = c.b0 * *x + z[0];
                        z[0] = c.b1 * *x - c.a1 * y + z[1];
                        z[1] = c.b2 * *x - c.a2 * y;
                        *x = y;
                    }
                }
            }
            FxState::Crush {
                levels,
                downsample,
                counter,
                held,
            } => {
                for frame in buf.as_chunks_mut::<2>().0 {
                    if *counter == 0 {
                        for (h, x) in held.iter_mut().zip(frame.iter()) {
                            *h = (*x * *levels).round() / *levels;
                        }
                    }
                    *counter = (*counter + 1) % *downsample;
                    *frame = *held;
                }
            }
            FxState::Comp {
                threshold_db,
                slope,
                attack,
                release,
                makeup_db,
                reduction,
            } => {
                for frame in buf.as_chunks_mut::<2>().0 {
                    // Pico de los dos canales (estéreo enlazado), en dB.
                    let level = frame[0].abs().max(frame[1].abs()).max(1e-9);
                    let over = 20.0 * level.log10() - *threshold_db;
                    let target = if over > 0.0 { over * *slope } else { 0.0 };
                    let coef = if target > *reduction {
                        *attack
                    } else {
                        *release
                    };
                    *reduction = target + coef * (*reduction - target);
                    let gain = 10f32.powf((*makeup_db - *reduction) / 20.0);
                    frame[0] *= gain;
                    frame[1] *= gain;
                }
            }
            FxState::Drive { drive, mix } => {
                for x in buf.iter_mut() {
                    let wet = (*drive * *x).tanh();
                    *x += *mix * (wet - *x);
                }
            }
            FxState::Eq { bands, sample_rate } => {
                let rate = *sample_rate;
                let keep = (-(EQ_BLOCK as f32) / (EQ_SMOOTHING_SECONDS * rate as f32)).exp();
                for band in bands.iter_mut() {
                    if band.bypassed() {
                        continue;
                    }
                    if band.settled() {
                        band.coefs.run(buf, &mut band.z);
                        continue;
                    }
                    // Suavizando: coeficientes nuevos cada EQ_BLOCK frames.
                    for sub in buf.chunks_mut(2 * EQ_BLOCK) {
                        if !band.settled() {
                            band.glide(keep, rate);
                        }
                        if band.bypassed() {
                            break;
                        }
                        band.coefs.run(sub, &mut band.z);
                    }
                }
            }
        }
    }
}
