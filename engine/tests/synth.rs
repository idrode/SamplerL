//! Núcleo de síntesis: aliasing medido, estabilidad del filtro, curvas
//! sin clics, determinismo con la semilla, nivel con 8 voces y unison, y
//! (ignorado, con `--release`) el coste de CPU.

use std::time::Instant;

use engine::synth::{
    self, CUTOFF, DETUNE, DRIVE, ENV_AMOUNT, FilterMode, HELD, NOISE, NoteEvent, PARAM_COUNT,
    Patch, RESONANCE, SPREAD, SUB, SynthCurves, Wave, acid,
};
use engine::{Curve, LfoRate, soft_limit};
use rustfft::{FftPlanner, num_complex::Complex};

const RATE: u32 = 48_000;
const NO_CURVES: SynthCurves = [None; PARAM_COUNT];

/// Nota MIDI de una frecuencia.
fn midi(hz: f64) -> f32 {
    (69.0 + 12.0 * (hz / 440.0).log2()) as f32
}

fn left(stereo: &[f32]) -> Vec<f32> {
    stereo.iter().step_by(2).copied().collect()
}

/// El oscilador casi solo: paso alto a 20 Hz sin resonancia (deja pasar
/// todo lo audible), sin envolvente de filtro y con la amplitud fija.
fn raw(wave: Wave) -> Patch {
    let mut p = Patch::new()
        .with(CUTOFF, 20.0)
        .with(RESONANCE, 0.0)
        .with(ENV_AMOUNT, 0.0);
    p.wave = wave;
    p.filter = FilterMode::Highpass;
    p.amp.sustain = 1.0;
    p
}

/// Espectro de potencia (ventana de Blackman-Harris de 4 términos, lóbulos
/// laterales a -92 dB) de `y`, en dB.
fn spectrum(y: &[f32]) -> Vec<f64> {
    let n = y.len();
    let mut buf: Vec<Complex<f64>> = y
        .iter()
        .enumerate()
        .map(|(i, &v)| {
            let x = std::f64::consts::TAU * i as f64 / n as f64;
            let w =
                0.35875 - 0.48829 * x.cos() + 0.14128 * (2.0 * x).cos() - 0.01168 * (3.0 * x).cos();
            Complex::new(v as f64 * w, 0.0)
        })
        .collect();
    FftPlanner::new().plan_fft_forward(n).process(&mut buf);
    buf[..n / 2]
        .iter()
        .map(|c| 10.0 * (c.norm_sqr() + 1e-30).log10())
        .collect()
}

/// La componente que no es armónica más fuerte entre 20 Hz y 20 kHz, en
/// dB respecto a la fundamental, de una onda de `f0` Hz a `rate` Hz.
fn worst_alias(y: &[f32], f0: f64, rate: u32) -> f64 {
    let db = spectrum(y);
    let bin = rate as f64 / y.len() as f64;
    let at = |hz: f64| (hz / bin).round() as usize;
    // Lóbulo principal de la ventana: ±4 bins; un poco más de margen.
    let near_harmonic = |k: usize| {
        let hz = k as f64 * bin;
        let h = (hz / f0).round();
        h >= 1.0 && (hz - h * f0).abs() <= 8.0 * bin
    };
    let fundamental = (at(f0) - 6..=at(f0) + 6)
        .map(|k| db[k])
        .fold(f64::MIN, f64::max);
    let worst = (at(20.0)..at(20_000.0).min(db.len()))
        .filter(|&k| !near_harmonic(k))
        .map(|k| db[k])
        .fold(f64::MIN, f64::max);
    worst - fundamental
}

/// Sierra sin antialiasing, a la misma frecuencia (la referencia).
fn naive_saw(f0: f64, frames: usize, rate: u32) -> Vec<f32> {
    let dt = f0 / rate as f64;
    (0..frames)
        .map(|i| (2.0 * (i as f64 * dt).fract() - 1.0) as f32)
        .collect()
}

/// Lo que sale de una nota sostenida de `seconds` segundos, sin el
/// primer cuarto de segundo (el ataque).
fn steady(patch: &Patch, hz: f64, frames: usize, rate: u32) -> Vec<f32> {
    let skip = rate as usize / 4;
    let notes = [(0, NoteEvent::new(midi(hz), HELD))];
    let out = synth::render(patch, &NO_CURVES, &notes, skip + frames, rate);
    left(&out)[skip..].to_vec()
}

/// Nivel del armónico `k` de una onda de `f0` Hz, en dB respecto a la
/// fundamental.
fn harmonic(y: &[f32], f0: f64, k: usize, rate: u32) -> f64 {
    let db = spectrum(y);
    let bin = rate as f64 / y.len() as f64;
    let peak = |hz: f64| {
        let c = (hz / bin).round() as usize;
        (c - 6..=c + 6).map(|i| db[i]).fold(f64::MIN, f64::max)
    };
    peak(k as f64 * f0) - peak(f0)
}

/// Umbral del aliasing de la sierra a 4 kHz: la componente espuria más
/// fuerte entre 20 Hz y 20 kHz, respecto a la fundamental. Medido: -52,3
/// dB (con PolyBLEP sin sobremuestrear, -30 dB; sin nada, -16,7 dB).
const ALIAS_THRESHOLD_DB: f64 = -50.0;

/// Sierra a 4 kHz: a 44,1 kHz, porque a 48 kHz los armónicos de 4 kHz
/// se pliegan justo sobre otros armónicos (48 000 es múltiplo de 4000) y
/// el aliasing no se vería. Los armónicos de verdad siguen en su sitio
/// (el diezmado no se come los agudos).
#[test]
fn a_saw_at_4_khz_keeps_its_aliasing_below_the_threshold() {
    let rate = 44_100;
    let frames = 1 << 16;
    let saw = steady(&raw(Wave::Saw), 4000.0, frames, rate);
    let alias = worst_alias(&saw, 4000.0, rate);
    let naive = worst_alias(&naive_saw(4000.0, frames, rate), 4000.0, rate);
    println!("sierra a 4 kHz: {alias:.1} dB (sin antialiasing, {naive:.1} dB)");
    assert!(naive > -30.0, "la referencia ingenua aliasa: {naive:.1} dB");
    assert!(alias < ALIAS_THRESHOLD_DB, "aliasing: {alias:.1} dB");
    for k in 2..=4 {
        let level = harmonic(&saw, 4000.0, k, rate);
        let ideal = -20.0 * (k as f64).log10();
        assert!(
            (level - ideal).abs() < 1.5,
            "armónico {k} ({} Hz): {level:.1} dB, debería ser {ideal:.1}",
            4000 * k
        );
    }
}

/// La cuadrada (también con otro ancho de pulso) y la triangular, a 4 kHz,
/// cada una con su umbral (medidos: -57,6, -49,2 y -83,2 dB).
#[test]
fn square_and_triangle_are_band_limited_too() {
    let rate = 44_100;
    let frames = 1 << 16;
    for (name, wave, pw, threshold) in [
        ("cuadrada", Wave::Square, 0.5, -55.0),
        ("pulso 25 %", Wave::Square, 0.25, -47.0),
        ("triangular", Wave::Triangle, 0.5, -80.0),
    ] {
        let patch = raw(wave).with(synth::PW, pw);
        let y = steady(&patch, 4000.0, frames, rate);
        let alias = worst_alias(&y, 4000.0, rate);
        println!("{name} a 4 kHz: {alias:.1} dB");
        assert!(alias < threshold, "{name}: {alias:.1} dB");
    }
}

/// Los filtros con la resonancia al máximo y el corte en los extremos (y
/// moviéndose a 50 Hz de un extremo a otro), con todo lo que puede entrar
/// en una voz (unison de 7, sub y ruido, ±3): ni un NaN y nada que crezca
/// sin límite. El ladder no pasa de 2 (la saturación de su entrada lo
/// acota; con el corte cerca del Nyquist cada polo rebota un poco); los de 12 dB/oct tienen un pico de unas 25 veces en el corte
/// (grande, pero acotado). Con la saturación de salida, nada pasa de 1.
#[test]
fn the_filters_stay_stable_at_full_resonance_and_extreme_cutoffs() {
    let frames = 2 * RATE as usize;
    for mode in FilterMode::ALL {
        for cutoff in [20.0, 20_000.0, 0.0] {
            for drive in [0.0, 1.0] {
                let mut patch = Patch::new()
                    .with(RESONANCE, 1.0)
                    .with(ENV_AMOUNT, 8.0)
                    .with(SUB, 1.0)
                    .with(NOISE, 1.0)
                    .with(DETUNE, 50.0)
                    .with(SPREAD, 1.0)
                    .with(DRIVE, drive);
                patch.filter = mode;
                patch.unison = 7;
                let mut curves = NO_CURVES;
                if cutoff == 0.0 {
                    curves[CUTOFF] = Some(Curve::Lfo {
                        min: 20.0,
                        max: 20_000.0,
                        rate: LfoRate::Hz(50.0),
                    });
                } else {
                    patch.params[CUTOFF] = cutoff;
                }
                // Una nota cada vez (con su release, sin solaparse).
                let notes: Vec<_> = (0..4)
                    .map(|i| (i * 24_000, NoteEvent::new(24.0 + 24.0 * i as f32, 6000)))
                    .collect();
                let out = synth::render(&patch, &curves, &notes, frames, RATE);
                let what = format!("{mode:?} a {cutoff} Hz, drive {drive}");
                assert!(out.iter().all(|v| v.is_finite()), "{what}: NaN o infinito");
                let peak = out.iter().fold(0.0f32, |m, v| m.max(v.abs()));
                let limit = match (mode, drive) {
                    (_, 1.0) => 1.0001,
                    (FilterMode::Ladder, _) => 2.0,
                    _ => 100.0,
                };
                assert!(peak < limit, "{what}: pico {peak}");
            }
        }
    }
}

/// El ladder autooscila con la resonancia al máximo (sin nada que entre,
/// un seno en el corte) y no se va: la saturación lo acota.
#[test]
fn the_ladder_self_oscillates_bounded_at_full_resonance() {
    let mut patch = raw(Wave::Sine)
        .with(CUTOFF, 1000.0)
        .with(RESONANCE, 1.0)
        .with(synth::SUB, 0.0);
    patch.filter = FilterMode::Ladder;
    // Una nota muy grave casi no excita el filtro: lo que suena es él.
    let y = steady(&patch, 30.0, RATE as usize, RATE);
    let tail = &y[y.len() / 2..];
    let peak = tail.iter().fold(0.0f32, |m, v| m.max(v.abs()));
    assert!(peak > 0.05, "no autooscila: pico {peak}");
    assert!(peak <= 2.0, "se va: pico {peak}");
}

/// Mayor segunda diferencia (`y[n+1] − 2·y[n] + y[n−1]`): en un seno de
/// frecuencia ω (radianes por muestra) y amplitud A no pasa de ω²·A; un
/// clic (un salto) da del orden del salto entero.
fn max_curvature(y: &[f32]) -> f32 {
    y.windows(3)
        .map(|w| (w[2] - 2.0 * w[1] + w[0]).abs())
        .fold(0.0, f32::max)
}

/// Lo más que se puede curvar un seno de 110 Hz de amplitud `peak` (con
/// sus primeros armónicos, que la saturación del ladder añade).
fn sine_curvature(peak: f32) -> f32 {
    let w = std::f32::consts::TAU * 110.0 / RATE as f32;
    w * w * peak
}

/// Un seno de 110 Hz por el ladder con el corte moviéndose deprisa (una
/// rampa de un paso de 100 a 8000 Hz, un LFO de 20 Hz en la resonancia):
/// la salida sigue siendo un seno de 110 Hz de amplitud variable, así que
/// no se curva más que un seno de 110 Hz de la amplitud máxima (con un
/// margen, [`CLICK_RATIO`]). Un clic lo rompería.
#[test]
fn ramps_and_lfos_do_not_click() {
    let mut patch = raw(Wave::Sine);
    patch.filter = FilterMode::Ladder;
    let mut curves = NO_CURVES;
    curves[CUTOFF] = Some(Curve::Ramp {
        from: 100.0,
        to: 8000.0,
        steps: 1.0,
    });
    curves[RESONANCE] = Some(Curve::Lfo {
        min: 0.0,
        max: 0.6,
        rate: LfoRate::Hz(20.0),
    });
    let y = steady_with(&patch, &curves, 110.0, RATE as usize);
    let peak = y.iter().fold(0.0f32, |m, v| m.max(v.abs()));
    let ratio = max_curvature(&y) / sine_curvature(peak);
    println!("rampa y LFO: curvatura {ratio:.2} veces la de un seno");
    assert!(
        ratio < CLICK_RATIO,
        "curvatura {ratio:.2} veces la de un seno"
    );
}

/// Curvatura máxima permitida, en veces la de un seno de 110 Hz: la
/// saturación del ladder y los cambios de amplitud la suben algo; un
/// clic, cientos de veces.
const CLICK_RATIO: f32 = 4.0;

/// Mayor salto entre dos muestras seguidas.
fn max_step(y: &[f32]) -> f32 {
    y.windows(2)
        .map(|w| (w[1] - w[0]).abs())
        .fold(0.0, f32::max)
}

/// Un cambio de patch en medio de una nota (el corte de 100 a 4000 Hz
/// de golpe, como al entrar un patrón nuevo) se suaviza: mientras cambia,
/// ninguna muestra salta más que antes o después (con un 20 % de margen:
/// la amplitud del seno sube en unos milisegundos). Sin resonancia: con
/// ella, barrer cinco octavas en 5 ms hace sonar el pico del filtro (como
/// una envolvente de filtro rápida), y eso no es un clic. Un clic pequeño
/// (un salto de 0,05) sí se vería.
#[test]
fn a_patch_change_mid_note_does_not_click() {
    let mut low = raw(Wave::Sine).with(CUTOFF, 100.0).with(RESONANCE, 0.0);
    low.filter = FilterMode::Ladder;
    let high = low.with(CUTOFF, 4000.0);
    let clock = synth::SongClock {
        steps: 0.0,
        seconds: 0.0,
        frames_per_step: 6000.0,
    };
    let half = RATE as usize / 2;
    let mut s = synth::Synth::new(RATE);
    s.note_on(&low, NoteEvent::new(midi(110.0), HELD));
    let mut out = vec![0.0f32; 4 * half];
    for (i, chunk) in out.chunks_mut(512).enumerate() {
        let patch = if i * 256 >= half { &high } else { &low };
        s.mix_into(chunk, patch, &NO_CURVES, &clock);
    }
    let y = left(&out);
    let quarter = half / 2;
    let around = max_step(&y[quarter..half]).max(max_step(&y[half + quarter..]));
    let during = max_step(&y[half - 64..half + quarter]);
    println!("cambio de patch: salto {during:.4} (antes y después, {around:.4})");
    assert!(
        during < 1.2 * around,
        "salto {during} (antes y después, {around})"
    );
    let mut clicked = y.clone();
    for v in &mut clicked[half + 100..] {
        *v += 0.05;
    }
    assert!(max_step(&clicked[half - 64..half + quarter]) > 1.5 * around);
}

fn steady_with(patch: &Patch, curves: &SynthCurves, hz: f64, frames: usize) -> Vec<f32> {
    let skip = RATE as usize / 4;
    let notes = [(0, NoteEvent::new(midi(hz), HELD))];
    let out = synth::render(patch, curves, &notes, skip + frames, RATE);
    left(&out)[skip..].to_vec()
}

/// Ocho voces con unison de 7, ruido y desafinación: lo que se toca.
fn crowd(seed: u64) -> (Patch, Vec<(usize, NoteEvent)>) {
    let mut patch = Patch::new()
        .with(DETUNE, 25.0)
        .with(SPREAD, 0.8)
        .with(NOISE, 0.2)
        .with(RESONANCE, 0.5)
        .with(ENV_AMOUNT, 3.0);
    patch.filter = FilterMode::Ladder;
    patch.unison = 7;
    patch.voices = 8;
    patch.seed = seed;
    let chord = [36.0, 43.0, 48.0, 52.0, 55.0, 60.0, 64.0, 67.0];
    let notes = chord
        .iter()
        .enumerate()
        .map(|(i, &p)| (i * 64, NoteEvent::new(p, RATE as u64)))
        .collect();
    (patch, notes)
}

/// La misma semilla da el mismo audio, bit a bit; otra semilla, otro (las
/// fases del unison y el ruido salen de ella).
#[test]
fn the_same_seed_gives_the_same_audio() {
    let frames = RATE as usize;
    let (patch, notes) = crowd(7);
    let a = synth::render(&patch, &NO_CURVES, &notes, frames, RATE);
    let b = synth::render(&patch, &NO_CURVES, &notes, frames, RATE);
    assert!(a.iter().zip(&b).all(|(x, y)| x.to_bits() == y.to_bits()));
    let (other, _) = crowd(8);
    let c = synth::render(&other, &NO_CURVES, &notes, frames, RATE);
    assert!(a.iter().zip(&c).any(|(x, y)| x != y));
}

/// Ocho voces con unison de 7 a la vez, sumadas: tras el limitador del
/// motor, dentro de ±1 y sin NaN. Y el unison no sube el nivel: siete
/// osciladores suenan como uno (dentro de 3 dB).
#[test]
fn eight_voices_with_unison_stay_within_one_after_the_limiter() {
    let frames = RATE as usize;
    let (patch, notes) = crowd(1);
    let out = synth::render(&patch, &NO_CURVES, &notes, frames, RATE);
    let limited: Vec<f32> = out.iter().map(|&v| soft_limit(v)).collect();
    assert!(limited.iter().all(|v| v.is_finite() && v.abs() <= 1.0));
    let rms = |y: &[f32]| (y.iter().map(|v| v * v).sum::<f32>() / y.len() as f32).sqrt();
    let one = |unison: u8| {
        let mut p = Patch::new().with(DETUNE, 25.0);
        p.unison = unison;
        rms(&steady(&p, 110.0, RATE as usize / 2, RATE))
    };
    let ratio_db = 20.0 * (one(7) / one(1)).log10();
    assert!(ratio_db.abs() < 3.0, "unison de 7: {ratio_db:.1} dB");
}

/// El acid: sus parámetros caben en los del núcleo y su patch es válido.
#[test]
fn the_acid_patch_is_valid() {
    let patch = acid::patch(36.0);
    patch.validate().unwrap();
    assert!(patch.mono);
    assert_eq!(patch.filter, FilterMode::Ladder);
    for p in &acid::PARAMS {
        let core = &synth::PARAMS[p.param];
        assert!(
            core.min <= p.min * p.scale && p.max * p.scale <= core.max,
            "{}",
            p.name
        );
    }
}

/// Coste de CPU: 8 voces con unison de 7 por el ladder, 10 segundos.
/// `cargo test -p engine --release --test synth -- --ignored --nocapture`.
#[test]
#[ignore]
fn cpu_cost_of_eight_voices_with_unison_of_seven() {
    let seconds = 10;
    let frames = RATE as usize * seconds;
    let (patch, notes) = crowd(3);
    let notes: Vec<_> = notes
        .into_iter()
        .map(|(at, ev)| (at, NoteEvent { gate: HELD, ..ev }))
        .collect();
    let started = Instant::now();
    let out = synth::render(&patch, &NO_CURVES, &notes, frames, RATE);
    let took = started.elapsed().as_secs_f64();
    assert!(out.iter().all(|v| v.is_finite()));
    let (mono, mono_notes) = (acid::patch(36.0), vec![(0, NoteEvent::new(36.0, HELD))]);
    let started = Instant::now();
    let _ = synth::render(&mono, &NO_CURVES, &mono_notes, frames, RATE);
    let acid_took = started.elapsed().as_secs_f64();
    println!(
        "8 voces × unison 7: {:.1} % de un núcleo ({took:.2} s por {seconds} s); acid: {:.2} %",
        100.0 * took / seconds as f64,
        100.0 * acid_took / seconds as f64
    );
}

/// Nivel del acid: con accent al máximo, la resonancia al máximo, el
/// filtro y la envolvente abiertos y sin o con saturación, no pasa de -1
/// dBFS (no satura el limitador); sin accent, suena bien por encima de -18
/// dBFS.
#[test]
fn the_acid_leaves_headroom() {
    let peak = |cutoff: f32, res: f32, accent: bool, dist: f32| {
        let mut patch = acid::patch(36.0);
        for (name, value) in [
            ("cutoff", cutoff),
            ("res", res),
            ("envmod", 1.0),
            ("accent", 1.0),
            ("dist", dist),
        ] {
            acid::set(&mut patch, acid::param(name).unwrap(), value);
        }
        let notes: Vec<_> = [24.0, 36.0, 48.0, 60.0]
            .iter()
            .enumerate()
            .map(|(i, &p)| {
                let mut ev = NoteEvent::new(p, 6000);
                ev.accent = accent;
                (i * 6000, ev)
            })
            .collect();
        let out = synth::render(&patch, &NO_CURVES, &notes, 24_000, RATE);
        20.0 * out.iter().fold(0.0f32, |m, v| m.max(v.abs())).log10()
    };
    let mut worst = f32::MIN;
    for cutoff in [100.0, 400.0, 2000.0, 20_000.0] {
        for res in [0.0, 0.7, 1.0] {
            for dist in [0.0, 0.5, 1.0] {
                worst = worst.max(peak(cutoff, res, true, dist));
            }
        }
    }
    let plain = peak(400.0, 0.7, false, 0.0);
    println!("acid: pico con accent {worst:.1} dBFS, sin él {plain:.1} dBFS");
    assert!(worst < -1.0, "con accent: {worst:.1} dBFS");
    assert!(plain > -18.0, "sin accent: {plain:.1} dBFS");
}
