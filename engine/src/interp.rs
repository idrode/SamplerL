//! Interpolación de las voces cuando el pitch cambia la velocidad de
//! lectura: sinc con ventana de Kaiser (8 cruces por cero a cada lado,
//! β = 8) leída de una tabla.
//!
//! Al subir el pitch (proporción `r` > 1) el corte de la sinc baja a
//! `0,9 / r` del Nyquist del sample: lo que pasaría del Nyquist de la
//! salida se filtra antes de leerlo, en vez de plegarse como aliasing. A
//! +12 y +24 semitonos (proporciones exactas 2 y 4) las posiciones de
//! lectura son enteras, así que la interpolación lineal o la cúbica no
//! cambian nada; el aliasing que se oye ahí es solo ese plegado. Al bajar
//! el pitch (`r` < 1) el corte es el Nyquist del sample y la sinc pasa por
//! las muestras: en una posición entera da la muestra tal cual.
//!
//! Medido con un seno (`engine/tests/interpolation.rs`): las componentes
//! espurias pasan de 0 dB a menos de -75 dB cuando la nota subida pasaría
//! del Nyquist, y de -36 dB a menos de -90 dB con una quinta (+7).
//!
//! Coste: unos `18 · max(1, r)` vecinos por frame y canal (con pitch 0,
//! ninguno: se lee la muestra). Ocho pistas a la vez a +24 semitonos gastan
//! en torno al 14 % de un núcleo del i7 de desarrollo.

use std::sync::LazyLock;

use crate::sample::Sample;

/// Cruces por cero a cada lado del centro.
const ZEROS: usize = 8;
/// Entradas de la tabla por cruce por cero (se interpola entre ellas).
const RESOLUTION: usize = 512;
/// β de la ventana de Kaiser (atenuación de unos 80 dB).
const BETA: f64 = 8.0;
/// Corte al subir el pitch, en fracción del Nyquist de la salida: deja
/// sitio a la banda de transición de la ventana.
const CUTOFF: f64 = 0.9;

/// Núcleo `sinc(u) · kaiser(u / ZEROS)` para `u` de 0 a `ZEROS`, con un
/// cero de más al final para interpolar sin comprobar el límite.
static KERNEL: LazyLock<[f32; ZEROS * RESOLUTION + 2]> = LazyLock::new(|| {
    let mut table = [0.0f32; ZEROS * RESOLUTION + 2];
    let i0_beta = bessel_i0(BETA);
    for (i, value) in table.iter_mut().enumerate().take(ZEROS * RESOLUTION) {
        let u = i as f64 / RESOLUTION as f64;
        // En los enteros, exactamente 0 (y 1 en el centro): la sinc pasa
        // por las muestras.
        let sinc = if i == 0 {
            1.0
        } else if i % RESOLUTION == 0 {
            0.0
        } else {
            (std::f64::consts::PI * u).sin() / (std::f64::consts::PI * u)
        };
        let r = u / ZEROS as f64;
        let window = bessel_i0(BETA * (1.0 - r * r).max(0.0).sqrt()) / i0_beta;
        *value = (sinc * window) as f32;
    }
    table
});

/// Calcula la tabla (en el hilo de control: `new_engine` la pide antes de
/// que el audio la lea).
pub(crate) fn prepare() {
    LazyLock::force(&KERNEL);
}

/// Función de Bessel modificada de orden 0 (serie de potencias).
fn bessel_i0(x: f64) -> f64 {
    let mut sum = 1.0;
    let mut term = 1.0;
    for k in 1..64 {
        term *= x / 2.0 / k as f64;
        sum += term * term;
        if term * term < sum * 1e-18 {
            break;
        }
    }
    sum
}

/// Frame del sample en la posición fraccionaria `pos`, leída a `rate`
/// frames del sample por frame de salida, sin salir de `lo..hi` (los
/// vecinos de fuera repiten el borde). Con `rate` ≤ 1 y una posición
/// entera, la muestra tal cual (con pitch 0, la salida es idéntica bit a
/// bit a leer el sample).
pub(crate) fn read(sample: &Sample, pos: f64, lo: usize, hi: usize, rate: f64) -> (f32, f32) {
    let i0 = pos.floor();
    if pos == i0 && rate <= 1.0 {
        return sample.frame(i0 as usize);
    }
    let table: &[f32] = &*KERNEL;
    // Corte en fracción del Nyquist del sample, y lo que alcanza el núcleo
    // (en frames del sample) a cada lado.
    let cutoff = if rate > 1.0 { CUTOFF / rate } else { 1.0 };
    let reach = ZEROS as f64 / cutoff;
    let first = (pos - reach).floor() as isize + 1;
    let last = (pos + reach).ceil() as isize - 1;
    // Posición en la tabla del primer vecino, y lo que avanza por vecino.
    let mut u = ((first as f64 - pos) * cutoff * RESOLUTION as f64) as f32;
    let du = (cutoff * RESOLUTION as f64) as f32;
    let limit = (ZEROS * RESOLUTION) as f32;
    let (lo, hi) = (lo as isize, hi as isize - 1);
    let (mut l, mut r, mut sum) = (0.0f32, 0.0f32, 0.0f32);
    let mut tap = |frame: (f32, f32), u: f32| {
        let x = u.abs();
        if x < limit {
            let i = x as usize;
            let t = x - i as f32;
            let w = table[i] + t * (table[i + 1] - table[i]);
            l += w * frame.0;
            r += w * frame.1;
            sum += w;
        }
    };
    if first >= lo && last <= hi {
        // La ventana cabe en el rango: sin comprobar los bordes.
        for k in first..=last {
            tap(sample.frame(k as usize), u);
            u += du;
        }
    } else {
        for k in first..=last {
            tap(sample.frame(k.clamp(lo, hi) as usize), u);
            u += du;
        }
    }
    // Normalizado: ganancia exacta 1 en continua sea cual sea la fase.
    (l / sum, r / sum)
}
