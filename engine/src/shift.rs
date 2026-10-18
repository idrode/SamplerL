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

/// Estado del desplazador de una voz. Las posiciones son del eje de la
/// voz: desplazamientos desde su inicio, en frames del sample (la voz los
/// pasa a posiciones del sample con su bucle y su sentido).
#[derive(Debug, Clone, Copy)]
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
}

impl Shift {
    pub fn new(rate: f64, sample_rate: u32) -> Self {
        let hop = ((GRAIN_SECONDS / 2.0 * sample_rate as f64) as u64).max(STRIDE);
        Shift {
            rate,
            hop,
            search: ((SEARCH_SECONDS * sample_rate as f64) as u64).max(1),
            grain: 0,
            anchor: 0.0,
            previous: 0.0,
        }
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
        while self.grain < k {
            self.next_grain(fit, &mono);
        }
        let m = (n - k * self.hop) as f64;
        if k == 0 {
            // El primer grano, sin fundido de entrada: el golpe entero.
            return [(self.anchor + self.rate * m, 1.0), (0.0, 0.0)];
        }
        let w = 0.5 - 0.5 * (std::f64::consts::PI * m / self.hop as f64).cos() as f32;
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
    }
}
