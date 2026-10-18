//! Pitch sin cambiar la duración: granos alineados por correlación, en el
//! hilo de audio (un WSOLA hecho al vuelo dentro de la voz).
//!
//! La voz avanza por el sample a velocidad 1 (su duración no cambia), pero
//! lo lee en granos con ventana de Hann de [`GRAIN_SECONDS`] que se solapan
//! a la mitad, y cada grano se lee a la velocidad del pitch: el tono sube o
//! baja y el tiempo no. Cada grano nuevo se ancla donde le toca (su frame
//! de salida) desplazado hasta ±[`SEARCH_SECONDS`] para que su forma de
//! onda encaje con lo que seguiría leyendo el anterior (la correlación más
//! alta, normalizada): sin eso, dos granos solapados se cancelan a ratos y
//! un seno sale con huecos y con otra frecuencia aparente. El primer grano
//! empieza en el golpe sin fundido de entrada (el ataque queda entero), y
//! al final ningún grano lee más allá del rango: se ancla antes.
//!
//! Todo depende solo del sample y del número de frame de la voz: es
//! determinista. Sin memoria dinámica (el estado son unos números).
//!
//! Medido en `engine/tests/pitch_keep.rs` frente al WSOLA de
//! `Sample::stretched` seguido de leer a la velocidad del pitch (lo que
//! haría una versión al cargar): la misma precisión de frecuencia (< 0,1 %)
//! y una pureza parecida en un seno (40-50 dB), y la misma desviación de la
//! envolvente en un break. Va bien hasta ±12 semitonos; más allá los golpes
//! se emborronan (un grano lee hasta 4 veces su duración) y la voz suena
//! metálica.
//!
//! Coste: dos lecturas interpoladas por frame y, cada medio grano (15 ms),
//! una búsqueda de unos 150 candidatos por 90 puntos. Medido: una voz con
//! `%` gasta un 2,6 % de un núcleo del i7 de desarrollo (con `^`, 0,6 %).
//!
//! **Formantes.** Leer un grano más deprisa sube también su envolvente
//! espectral (la voz suena a ardilla, la caja a caja pequeña). Con
//! `formant`, cada grano se analiza dos veces con LPC (orden [`ORDER`],
//! autocorrelación con ventana de Hann y Levinson-Durbin en f64): lo que
//! lee el grano y el mismo trozo del original leído a la velocidad de los
//! formantes (1 para dejarlos donde estaban, 2^(n/12) para desplazarlos
//! `n` semitonos). La suma de los dos granos pasa por un filtro que
//! blanquea con el primero y colorea con el segundo, en celosía, con sus
//! coeficientes de reflexión mezclados con el mismo fundido que los granos
//! (mezclar coeficientes de reflexión deja el filtro estable) y la ganancia
//! que iguala los errores de predicción. Medido en `engine/tests/formant.rs`
//! con una vocal sintética: subida una quinta, sus armónicos quedan a
//! 1,3 dB de la envolvente original (sin corrección, a 14 dB; a 0,5 de la
//! movida); bajada una octava, a 1,4 dB. Cuesta otro 2 % de un núcleo por
//! voz.

/// Desplazamiento máximo de los formantes, en semitonos.
pub const MAX_FORMANT: f32 = 24.0;

/// Comprueba el desplazamiento de los formantes.
pub(crate) fn check_formant(formant: Option<f32>) -> Result<(), &'static str> {
    match formant {
        Some(f) if !(-MAX_FORMANT..=MAX_FORMANT).contains(&f) => {
            Err("formant fuera de rango (-24 a 24 semitonos)")
        }
        _ => Ok(()),
    }
}

/// Duración del grano (ventana de Hann; se solapan a la mitad).
const GRAIN_SECONDS: f64 = 0.030;
/// Desplazamiento máximo de cada grano al buscar el encaje.
const SEARCH_SECONDS: f64 = 0.006;
/// Los candidatos se prueban cada tantas muestras (y luego se afina a 1).
const STRIDE: u64 = 4;
/// La correlación compara un punto de cada tantos del medio grano, como
/// mucho [`MAX_POINTS`] (a 48 kHz, 90).
const POINT_STRIDE: u64 = 8;
const MAX_POINTS: usize = 512;

/// El sample en mono en la posición `pos` (interpolación lineal, sin
/// salir de `start..end`): lo que miran la búsqueda del encaje y el
/// análisis de los formantes.
#[inline]
pub(crate) fn mono_at(sample: &crate::sample::Sample, pos: f64, start: usize, end: usize) -> f32 {
    let pos = pos.clamp(start as f64, (end - 1) as f64);
    let i = pos as usize;
    let t = (pos - i as f64) as f32;
    let (l0, r0) = sample.frame(i);
    let (l1, r1) = sample.frame((i + 1).min(end - 1));
    0.5 * ((l0 + r0) + t * ((l1 + r1) - (l0 + r0)))
}

/// Cada cuántos frames se rehacen los coeficientes mezclados del filtro.
const MIX_EVERY: u32 = 8;

/// Orden del LPC de la corrección de formantes.
const ORDER: usize = 32;

/// Estado del desplazador de una voz. Las posiciones son del eje de la
/// voz: desplazamientos desde su inicio, en frames del sample (la voz los
/// pasa a posiciones del sample con su bucle y su sentido).
#[derive(Debug, Clone)]
pub(crate) struct Shift {
    /// Velocidad de lectura de cada grano (2 = una octava arriba).
    pub rate: f64,
    /// Medio grano, en frames de salida.
    hop: u64,
    search: u64,
    /// Último grano empezado, su ancla y la del anterior.
    grain: u64,
    anchor: f64,
    previous: f64,
    /// Formantes: velocidad a la que se leería el original para tener la
    /// envolvente buscada (1: donde estaban); `None`, se mueven con el
    /// pitch. La corrección del último grano y la del anterior (`slot` es
    /// la del último), el peso del último en el frame actual y el filtro
    /// que pasa la suma de los dos.
    formant: Option<f64>,
    corrections: [Correction; 2],
    slot: usize,
    mix: f32,
    filter: Lattice,
    /// Los coeficientes mezclados que usa el filtro (se rehacen cada
    /// [`MIX_EVERY`] frames) y los frames que quedan para rehacerlos.
    mixed: Correction,
    stale: u32,
    /// La corrección del primer grano ya está calculada.
    ready: bool,
}

/// Lo que cambia la envolvente de un grano: un filtro que la blanquea (el
/// LPC de lo que lee el grano) y otro que le pone la buscada (el LPC del
/// original leído a la velocidad de los formantes), como coeficientes de
/// reflexión, con la ganancia que iguala sus errores de predicción.
#[derive(Debug, Clone, Copy)]
struct Correction {
    whiten: [f32; ORDER],
    color: [f32; ORDER],
    gain: f32,
}

/// Los dos filtros en celosía (estables también en f32 y con los
/// coeficientes cambiando, mientras cada uno quede en ±1) y su estado por
/// canal.
#[derive(Debug, Clone, Copy)]
struct Lattice {
    fir: [[f32; ORDER]; 2],
    iir: [[f32; ORDER]; 2],
}

impl Lattice {
    /// Pasa un frame estéreo por `A_whiten(z) · gain / A_color(z)` (los
    /// dos canales a la vez).
    fn apply(&mut self, x: [f32; 2], k: &Correction) -> [f32; 2] {
        // Celosía FIR: el residuo de lo leído.
        let [fir_l, fir_r] = &mut self.fir;
        let (mut f, mut b) = (x, x);
        for i in 0..ORDER {
            let ki = k.whiten[i];
            let old = [fir_l[i], fir_r[i]];
            (fir_l[i], fir_r[i]) = (b[0], b[1]);
            for c in 0..2 {
                (f[c], b[c]) = (f[c] + ki * old[c], old[c] + ki * f[c]);
            }
        }
        // Celosía IIR: la envolvente buscada.
        let [iir_l, iir_r] = &mut self.iir;
        let mut f = [k.gain * f[0], k.gain * f[1]];
        for i in (0..ORDER).rev() {
            let ki = k.color[i];
            f[0] -= ki * iir_l[i];
            f[1] -= ki * iir_r[i];
            if i + 1 < ORDER {
                iir_l[i + 1] = iir_l[i] + ki * f[0];
                iir_r[i + 1] = iir_r[i] + ki * f[1];
            }
        }
        iir_l[0] = f[0];
        iir_r[0] = f[1];
        f
    }
}

impl Correction {
    const IDENTITY: Correction = Correction {
        whiten: [0.0; ORDER],
        color: [0.0; ORDER],
        gain: 1.0,
    };

    /// Corrección de lo que lee `read` (blanquear) a lo que lee `target`
    /// (colorear), cada uno sobre sus frames. Sin señal, la identidad.
    fn new(
        (n_read, read): (usize, impl Fn(usize) -> f32),
        (n_target, target): (usize, impl Fn(usize) -> f32),
    ) -> Self {
        let (whiten, e_read) = lpc(n_read, read);
        let (color, e_target) = lpc(n_target, target);
        if e_read <= 1e-15 || e_target <= 1e-15 {
            return Correction::IDENTITY;
        }
        Correction {
            whiten,
            color,
            gain: (e_target / e_read).sqrt() as f32,
        }
    }
}

/// Coeficientes de reflexión del LPC (autocorrelación con ventana de Hann
/// y Levinson-Durbin, en f64: en f32 una envolvente con mucho rango, como
/// la de una vocal, no cabe) de `x(0..n)`, y el error de predicción por
/// muestra.
fn lpc(n: usize, x: impl Fn(usize) -> f32) -> ([f32; ORDER], f64) {
    let mut windowed = [0.0f64; MAX_ANALYSIS];
    let n = n.min(MAX_ANALYSIS);
    for (i, w) in windowed[..n].iter_mut().enumerate() {
        let hann = 0.5 - 0.5 * (std::f64::consts::TAU * (i as f64 + 0.5) / n as f64).cos();
        *w = hann * x(i) as f64;
    }
    let w = &windowed[..n];
    let mut r = [0.0f64; ORDER + 1];
    for (lag, r) in r.iter_mut().enumerate() {
        *r = w[..n.saturating_sub(lag)]
            .iter()
            .zip(&w[lag..])
            .map(|(a, b)| a * b)
            .sum();
    }
    let mut k = [0.0f32; ORDER];
    if r[0] <= 1e-12 {
        return (k, 0.0);
    }
    // Un poco de ruido blanco (-90 dB): el sistema queda bien
    // condicionado sin tapar los valles de la envolvente.
    r[0] *= 1.0 + 1e-9;
    let mut a = [0.0f64; ORDER + 1];
    a[0] = 1.0;
    let mut error = r[0];
    for i in 1..=ORDER {
        let acc: f64 = (0..i).map(|j| a[j] * r[i - j]).sum();
        let ki = -acc / error;
        let previous = a;
        for j in 1..i {
            a[j] = previous[j] + ki * previous[i - j];
        }
        a[i] = ki;
        error *= 1.0 - ki * ki;
        k[i - 1] = ki as f32;
        if error <= 0.0 {
            break;
        }
    }
    // Por muestra (la ventana de Hann, de media 3/8 en potencia).
    (k, error / (0.375 * n as f64))
}

/// Frames que mira el análisis como mucho (un grano a 96 kHz).
const MAX_ANALYSIS: usize = 4096;

impl Shift {
    /// `formant`: `Some(n)` deja los formantes desplazados `n` semitonos
    /// de donde estaban (0: no se mueven), `None` los mueve con el pitch.
    pub fn new(rate: f64, sample_rate: u32, formant: Option<f32>) -> Self {
        let hop = ((GRAIN_SECONDS / 2.0 * sample_rate as f64) as u64).max(STRIDE);
        // Si los formantes van adonde los lleva el pitch, no hay nada que
        // corregir.
        let formant = formant
            .map(|f| 2f64.powf(f as f64 / 12.0))
            .filter(|&f| (f - rate).abs() > 1e-9);
        Shift {
            rate,
            hop,
            search: ((SEARCH_SECONDS * sample_rate as f64) as u64).max(1),
            grain: 0,
            anchor: 0.0,
            previous: 0.0,
            formant,
            corrections: [Correction::IDENTITY; 2],
            slot: 0,
            mix: 1.0,
            filter: Lattice {
                fir: [[0.0; ORDER]; 2],
                iir: [[0.0; ORDER]; 2],
            },
            mixed: Correction::IDENTITY,
            stale: 0,
            ready: false,
        }
    }

    /// Hace falta un desplazador: el pitch cambia o los formantes se
    /// mueven por su cuenta.
    pub fn needed(rate: f64, formant: Option<f32>) -> bool {
        rate != 1.0 || formant.is_some_and(|f| f != 0.0)
    }

    /// La corrección de formantes del grano anclado en `anchor`.
    fn correction(&self, anchor: f64, mono: &impl Fn(f64) -> f32) -> Correction {
        let Some(target) = self.formant else {
            return Correction::IDENTITY;
        };
        let grain = (2 * self.hop) as f64;
        let rate = self.rate;
        // Los dos análisis miran el mismo trozo del original (al menos lo
        // que dura un grano, centrado en el suyo): leído como lo lee el
        // grano y leído a la velocidad de los formantes. Al bajar, la
        // ventana de lo leído se alarga (si no, tendría pocos periodos).
        let span = grain * rate.max(1.0);
        let center = anchor + rate * grain / 2.0;
        let frames = |speed: f64| ((span / speed) as usize).clamp(16, MAX_ANALYSIS);
        let (n_read, n_target) = (frames(rate), frames(target));
        let (from_read, from_target) = (
            center - rate * n_read as f64 / 2.0,
            center - target * n_target as f64 / 2.0,
        );
        Correction::new(
            (n_read, |i| mono(from_read + rate * i as f64)),
            (n_target, |i| mono(from_target + target * i as f64)),
        )
    }

    /// Pasa por la corrección de formantes la suma de los dos granos del
    /// frame (lo que devolvió [`Shift::grains`]): un solo filtro, con los
    /// coeficientes del grano anterior y del último mezclados con el mismo
    /// fundido que los granos.
    pub fn correct(&mut self, (l, r): (f32, f32)) -> (f32, f32) {
        if self.formant.is_none() {
            return (l, r);
        }
        if self.stale == 0 {
            let (now, before) = (
                &self.corrections[self.slot],
                &self.corrections[1 - self.slot],
            );
            let t = self.mix;
            let lerp = |a: f32, b: f32| b + t * (a - b);
            self.mixed = Correction {
                whiten: std::array::from_fn(|i| lerp(now.whiten[i], before.whiten[i])),
                color: std::array::from_fn(|i| lerp(now.color[i], before.color[i])),
                gain: lerp(now.gain, before.gain),
            };
            self.stale = MIX_EVERY;
        }
        self.stale -= 1;
        let [l, r] = self.filter.apply([l, r], &self.mixed);
        (l, r)
    }

    /// Lo que lee como mucho un grano desde su ancla (en el eje de la voz).
    pub fn reach(&self) -> f64 {
        self.rate * (2 * self.hop) as f64
    }

    /// Los dos granos que suenan en el frame de salida `n` (posición en el
    /// eje de la voz y peso; los pesos suman 1). `n` solo crece. `fit`: el
    /// ancla más alta que deja leer un grano entero sin salir del rango
    /// (`None` en bucle). `mono(pos)` es el sample (mono) en esa posición
    /// del eje, para buscar el encaje.
    pub fn grains(
        &mut self,
        n: u64,
        fit: Option<f64>,
        mono: impl Fn(f64) -> f32,
    ) -> [(f64, f32); 2] {
        let k = n / self.hop;
        if !self.ready {
            self.ready = true;
            self.corrections[self.slot] = self.correction(self.anchor, &mono);
        }
        while self.grain < k {
            self.next_grain(fit, &mono);
        }
        let m = (n - k * self.hop) as f64;
        if k == 0 {
            // El primer grano, sin fundido de entrada: el golpe entero.
            self.mix = 1.0;
            return [(self.anchor + self.rate * m, 1.0), (0.0, 0.0)];
        }
        let w = 0.5 - 0.5 * (std::f64::consts::PI * m / self.hop as f64).cos() as f32;
        self.mix = w;
        [
            (self.anchor + self.rate * m, w),
            (self.previous + self.rate * (m + self.hop as f64), 1.0 - w),
        ]
    }

    /// Ancla el grano siguiente: donde le toca, desplazado al mejor encaje
    /// con la continuación del anterior.
    fn next_grain(&mut self, fit: Option<f64>, mono: &impl Fn(f64) -> f32) {
        let k = self.grain + 1;
        let hop = self.hop as f64;
        let top = fit.map_or(f64::MAX, |f| f.max(0.0));
        let nominal = (k as f64 * hop).min(top);
        let natural = self.anchor + self.rate * hop;
        let rate = self.rate;
        // Lo que seguiría leyendo el grano anterior, una vez (en la pila).
        let points = ((self.hop / POINT_STRIDE) as usize).min(MAX_POINTS);
        let mut reference = [0.0f32; MAX_POINTS];
        for (i, x) in reference[..points].iter_mut().enumerate() {
            *x = mono(natural + rate * (i as u64 * POINT_STRIDE) as f64);
        }
        let similarity = |candidate: f64| -> f32 {
            let (mut xy, mut yy) = (0.0f32, 1e-9f32);
            for (i, x) in reference[..points].iter().enumerate() {
                let y = mono(candidate + rate * (i as u64 * POINT_STRIDE) as f64);
                xy += x * y;
                yy += y * y;
            }
            xy / yy.sqrt()
        };
        let test = |candidate: f64, best: &mut (f32, f64)| {
            if !(0.0..=top).contains(&candidate) {
                return;
            }
            let s = similarity(candidate);
            if s > best.0 {
                *best = (s, candidate);
            }
        };
        let mut best = (f32::MIN, nominal);
        let search = self.search as i64;
        for d in (-search..=search).step_by(STRIDE as usize) {
            test(nominal + d as f64, &mut best);
        }
        let coarse = best.1;
        for d in 1..STRIDE as i64 {
            test(coarse - d as f64, &mut best);
            test(coarse + d as f64, &mut best);
        }
        self.previous = self.anchor;
        self.anchor = best.1;
        self.grain = k;
        if self.formant.is_some() {
            self.slot = 1 - self.slot;
            // Los coeficientes mezclados se rehacen ya (el grano cambia).
            self.stale = 0;
            self.corrections[self.slot] = self.correction(self.anchor, mono);
        }
    }
}
