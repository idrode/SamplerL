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
    let clock = synth::SongClock::stopped(6000.0, 96_000);
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

/// Cada preset: su patch es válido y sus parámetros caben en los del
/// núcleo; el acid es mono con el ladder, el sub mono con el paso bajo.
#[test]
fn the_presets_are_valid() {
    for preset in synth::preset::PRESETS {
        let patch = preset.patch(preset.root as f32);
        patch.validate().unwrap();
        assert!(patch.mono, "{}", preset.name);
        assert_eq!(patch.wave, preset.waves[0]);
        for p in preset.params {
            assert_eq!(p.get(&patch), p.default, "{}.{}", preset.name, p.name);
            if let synth::preset::Target::Param(i) = p.target {
                let core = &synth::PARAMS[i];
                assert!(
                    core.min <= p.min * p.scale && p.max * p.scale <= core.max,
                    "{}.{}",
                    preset.name,
                    p.name
                );
            }
        }
    }
    assert_eq!(acid::patch(36.0).filter, FilterMode::Ladder);
    assert_eq!(synth::sub::patch(24.0).filter, FilterMode::Lowpass);
}

/// El sub, limpio: un seno de 55 Hz con sus armónicos 2 a 5 por debajo
/// de -60 dB. Con `drive`, el 3 sube (se oye en altavoces pequeños) pero
/// lo que pasa de `tone` sigue cayendo.
#[test]
fn the_sub_is_clean_and_drive_adds_low_harmonics() {
    let frames = 1 << 16;
    let hz = 55.0;
    let clean = steady(&synth::sub::patch(24.0), hz, frames, RATE);
    for k in 2..=5 {
        let level = harmonic(&clean, hz, k, RATE);
        assert!(level < -60.0, "armónico {k}: {level:.1} dB");
    }
    let mut driven = synth::sub::patch(24.0);
    synth::sub::param("drive").unwrap().set(&mut driven, 0.6);
    let driven = steady(&driven, hz, frames, RATE);
    let third = harmonic(&driven, hz, 3, RATE);
    let high = harmonic(&driven, hz, 21, RATE);
    println!("sub con drive 0.6: armónico 3 {third:.1} dB, 21 (1155 Hz) {high:.1} dB");
    assert!(third > -30.0, "armónico 3: {third:.1} dB");
    assert!(high < third - 30.0, "armónico 21: {high:.1} dB");
}

/// Una línea de sub con silencios, notas pegadas sin slide (otra altura,
/// la misma voz), una ligada y la última que suelta: ninguna muestra salta
/// más que lo que sube un seno de la nota más aguda (con un 20 % de
/// margen): ni el arranque desde el silencio (en el cruce por cero) ni el
/// cambio de nota (sigue su fase) ni el release hacen clic.
#[test]
fn the_sub_starts_and_changes_notes_without_clicks() {
    let patch = synth::sub::patch(24.0);
    let step = 6000;
    let pitches = [24.0, 24.0, 31.0, 36.0, 29.0];
    let mut notes = vec![(0, NoteEvent::new(pitches[0], step as u64))];
    // Pegadas: cada una empieza cuando acaba la anterior.
    for (i, &p) in pitches.iter().enumerate().skip(1) {
        let mut ev = NoteEvent::new(p, step as u64);
        ev.slide = i == 3;
        notes.push((2 * step + i * step, ev));
    }
    let y = left(&synth::render(&patch, &NO_CURVES, &notes, 9 * step, RATE));
    let peak = y.iter().fold(0.0f32, |m, v| m.max(v.abs()));
    let top = 440.0 * ((36.0f32 - 69.0) / 12.0).exp2();
    let bound = std::f32::consts::TAU * top / RATE as f32 * peak;
    let step_max = max_step(&y);
    assert!(
        step_max < 1.2 * bound,
        "salto {step_max} (un seno: {bound})"
    );
    // Empieza en silencio y sube poco a poco (la primera muestra, casi 0).
    assert!(y[0].abs() < 1e-3 && y[1].abs() < 1e-3);
    // Con la fase al azar (el núcleo por defecto) sí saltaría.
    let mut random = patch;
    random.zero_phase = false;
    random.seed = 11;
    let jumpy = left(&synth::render(&random, &NO_CURVES, &notes[..1], step, RATE));
    let first = max_step(&jumpy[..480]);
    let calm = max_step(&y[..480]);
    assert!(first > calm, "al azar {first}, en el cero {calm}");
}

/// Nivel del sub: sin accent, el pico en unos -6,5 dBFS (el paso bajo de
/// `tone` le quita medio decibelio a 65 Hz); con él, unos 3 dB más (y por
/// debajo de 0 con `drive` al máximo).
#[test]
fn the_sub_level() {
    let peak_db = |accent: bool, drive: f32| {
        let mut patch = synth::sub::patch(24.0);
        synth::sub::param("drive").unwrap().set(&mut patch, drive);
        let mut ev = NoteEvent::new(36.0, HELD);
        ev.accent = accent;
        let out = synth::render(&patch, &NO_CURVES, &[(0, ev)], RATE as usize / 2, RATE);
        20.0 * out.iter().fold(0.0f32, |m, v| m.max(v.abs())).log10()
    };
    let (plain, accented, driven) = (peak_db(false, 0.0), peak_db(true, 0.0), peak_db(true, 1.0));
    println!("sub: pico {plain:.1} dBFS, con accent {accented:.1}, con drive {driven:.1}");
    assert!((plain + 6.5).abs() < 0.5, "{plain:.1} dBFS");
    assert!((accented - plain - 2.9).abs() < 0.5, "{accented:.1} dBFS");
    assert!(driven < 0.0, "{driven:.1} dBFS");
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
    let sub = synth::sub::patch(24.0);
    let started = Instant::now();
    let _ = synth::render(
        &sub,
        &NO_CURVES,
        &[(0, NoteEvent::new(24.0, HELD))],
        frames,
        RATE,
    );
    let sub_took = started.elapsed().as_secs_f64();
    // El subharm a plena carga: 8 voces con las seis capas sonando y
    // redisparándose (poliritmos), el ladder abierto.
    let mut harm = synth::subharm::patch(36.0).with(CUTOFF, 4000.0);
    synth::subharm::param("voices").unwrap().set(&mut harm, 8.0);
    harm.layers = Some(std::array::from_fn(|i| synth::Layer {
        div: [1, 2, 3, 5, 7, 16][i],
        rdiv: [4, 3, 5, 7, 2, 16][i],
        pan: [-0.8, 0.8, -0.4, 0.4, -1.0, 1.0][i],
        level: 1.0,
    }));
    let chord: Vec<_> = (0..8)
        .map(|i| (i * 64, NoteEvent::new(36.0 + 3.0 * i as f32, HELD)))
        .collect();
    let started = Instant::now();
    let _ = synth::render(&harm, &NO_CURVES, &chord, frames, RATE);
    let harm_took = started.elapsed().as_secs_f64();
    println!(
        "subharm, 8 voces × 6 capas: {:.1} % de un núcleo ({:.2} % por voz)",
        100.0 * harm_took / seconds as f64,
        100.0 * harm_took / seconds as f64 / 8.0
    );
    println!(
        "8 voces × unison 7: {:.1} % de un núcleo ({took:.2} s por {seconds} s); acid: {:.2} %; sub: {:.2} %",
        100.0 * took / seconds as f64,
        100.0 * acid_took / seconds as f64,
        100.0 * sub_took / seconds as f64
    );
    // El sh101 con todo sonando (sierra, pulso, sub, ruido, resonancia y
    // paso alto), con el filtro a la frecuencia del motor y al doble.
    for quality in [1.0, 2.0] {
        let mut sh = synth::sh101::patch(36.0).with(SUB, 0.7);
        for (name, value) in [("pulse", 0.5), ("noise", 0.1), ("res", 0.8), ("hpf", 1.0)] {
            synth::sh101::param(name).unwrap().set(&mut sh, value);
        }
        synth::sh101::param("s").unwrap().set(&mut sh, 1.0);
        synth::sh101::param("quality")
            .unwrap()
            .set(&mut sh, quality);
        let started = Instant::now();
        let _ = synth::render(
            &sh,
            &NO_CURVES,
            &[(0, NoteEvent::new(36.0, HELD))],
            frames,
            RATE,
        );
        let took = started.elapsed().as_secs_f64();
        println!(
            "sh101, quality {quality}: {:.2} % de un núcleo",
            100.0 * took / seconds as f64
        );
    }
    println!(
        "memoria: {} KiB por pista ({} KiB las 8 pistas), reservada al crear el motor",
        std::mem::size_of::<synth::Synth>() / 1024,
        8 * std::mem::size_of::<synth::Synth>() / 1024
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
            acid::param(name).unwrap().set(&mut patch, value);
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

/// Un seno sin filtro que oiga, mono, que empieza en el cero, con su sub
/// (seno) al nivel `level` y dividido por `div`.
fn with_sub(level: f32, div: u8, wave: Wave) -> Patch {
    let mut p = raw(Wave::Sine).with(synth::SUB, level);
    p.zero_phase = true;
    p.mono = true;
    p.voices = 1;
    p.sub_div = div;
    p.sub_wave = wave;
    p
}

/// Lo que aporta el sub: la diferencia entre el patch con él y sin él (el
/// paso alto sin resonancia es lineal: la suma se separa).
fn sub_alone(patch: &Patch, notes: &[(usize, NoteEvent)], frames: usize) -> Vec<f32> {
    let off = Patch {
        params: {
            let mut p = patch.params;
            p[synth::SUB] = 0.0;
            p
        },
        ..*patch
    };
    let on = synth::render(patch, &NO_CURVES, notes, frames, RATE);
    let off = synth::render(&off, &NO_CURVES, notes, frames, RATE);
    on.iter().zip(&off).map(|(a, b)| a - b).collect()
}

/// Cruces por cero hacia arriba de `y`.
fn ups(y: &[f32]) -> usize {
    y.windows(2).filter(|w| w[0] <= 0.0 && w[1] > 0.0).count()
}

/// Con `sub_div = n`, el sub suena a f/n: cuenta sus cruces por cero.
#[test]
fn the_sub_sounds_at_f_over_div() {
    let frames = 2 * RATE as usize;
    let notes = [(0, NoteEvent::new(69.0, HELD))];
    for div in [1, 2, 3, 5, 7, 16] {
        for wave in [Wave::Sine, Wave::Square] {
            let sub = left(&sub_alone(&with_sub(0.8, div, wave), &notes, frames));
            // Un segundo, pasado el ataque: 440/n ciclos.
            let second = &sub[RATE as usize / 2..RATE as usize / 2 + RATE as usize];
            let expected = 440.0 / div as f64;
            let measured = ups(second) as f64;
            assert!(
                (measured - expected).abs() <= 1.0,
                "sub_div {div} ({wave:?}): {measured} ciclos en un segundo, no {expected:.1}"
            );
        }
    }
}

/// Los tres subs del SH-101: cuadrada una octava abajo (`sub_div = 2`),
/// cuadrada dos octavas abajo (4) y el pulso al 25 % dos octavas abajo:
/// suenan a f/2, f/4 y f/4, el pulso alto un cuarto del periodo y sin
/// continua.
#[test]
fn the_sh101_sub_modes() {
    let frames = 2 * RATE as usize;
    let notes = [(0, NoteEvent::new(69.0, HELD))];
    for (div, wave) in [(2, Wave::Square), (4, Wave::Square), (4, Wave::Pulse25)] {
        let sub = left(&sub_alone(&with_sub(0.8, div, wave), &notes, frames));
        let second = &sub[RATE as usize / 2..RATE as usize / 2 + RATE as usize];
        let expected = 440.0 / div as f64;
        let measured = ups(second) as f64;
        assert!(
            (measured - expected).abs() <= 1.0,
            "{wave:?} ÷{div}: {measured} ciclos en un segundo, no {expected:.1}"
        );
        let high = second.iter().filter(|&&y| y > 0.0).count() as f64 / second.len() as f64;
        let mean = second.iter().map(|&y| y as f64).sum::<f64>() / second.len() as f64;
        let duty = if wave == Wave::Pulse25 { 0.25 } else { 0.5 };
        assert!(
            (high - duty).abs() < 0.02,
            "{wave:?} ÷{div}: alto {high:.3}"
        );
        assert!(mean.abs() < 0.01, "{wave:?} ÷{div}: continua {mean:.4}");
    }
}

/// Con glide (de A3 a A4 en 300 ms), el sub sigue al oscilador: cuelga de
/// su fase, así que en cada trozo hace exactamente 1/n de sus ciclos.
#[test]
fn the_sub_follows_the_glide() {
    let mut patch = with_sub(0.8, 3, Wave::Sine).with(synth::GLIDE, 300.0);
    patch.amp.release_ms = 5.0;
    let mut second = NoteEvent::new(69.0, HELD);
    second.slide = true;
    let notes = [(0, NoteEvent::new(57.0, HELD)), (12_000, second)];
    let frames = RATE as usize * 3 / 2;
    let sub = left(&sub_alone(&patch, &notes, frames));
    let main = {
        let mut off = patch;
        off.params[synth::SUB] = 0.0;
        left(&synth::render(&off, &NO_CURVES, &notes, frames, RATE))
    };
    let window = 12_000..frames;
    let (m, s) = (ups(&main[window.clone()]), ups(&sub[window]));
    assert!(m.abs_diff(3 * s) <= 3, "oscilador {m} ciclos, sub {s}");
}

/// Con `sub = 0`, nada del sub cambia nada: el divisor, la forma, el pan y
/// su saturación dan el mismo audio bit a bit que el patch por defecto.
#[test]
fn a_silent_sub_changes_nothing() {
    let notes = [
        (0, NoteEvent::new(45.0, 20_000)),
        (24_000, NoteEvent::new(52.0, 9000)),
    ];
    for base in [acid::patch(36.0), synth::sub::patch(24.0), Patch::new()] {
        let mut odd = base
            .with(synth::SUB, 0.0)
            .with(synth::SUB_PAN, 0.8)
            .with(synth::SUB_DRIVE, 0.9);
        odd.sub_div = 7;
        odd.sub_wave = Wave::Sine;
        let a = synth::render(&base, &NO_CURVES, &notes, 40_000, RATE);
        let b = synth::render(&odd, &NO_CURVES, &notes, 40_000, RATE);
        assert!(a.iter().zip(&b).all(|(x, y)| x.to_bits() == y.to_bits()));
    }
}

/// El pan del sub es de potencia constante: L² + R² no cambia al moverlo
/// (y en el centro, los dos lados como sin pan).
#[test]
fn the_sub_pan_keeps_its_power() {
    let notes = [(0, NoteEvent::new(45.0, HELD))];
    let power = |pan: f32| {
        let patch = with_sub(0.8, 2, Wave::Sine).with(synth::SUB_PAN, pan);
        let sub = sub_alone(&patch, &notes, RATE as usize);
        let tail = &sub[RATE as usize / 2..];
        let (mut l, mut r) = (0.0f64, 0.0f64);
        for f in tail.chunks(2) {
            l += (f[0] as f64).powi(2);
            r += (f[1] as f64).powi(2);
        }
        (l, r)
    };
    let (cl, cr) = power(0.0);
    assert!((cl / cr - 1.0).abs() < 1e-6);
    for pan in [-1.0, -0.6, -0.2, 0.3, 0.7, 1.0] {
        let (l, r) = power(pan);
        let total = (l + r) / (cl + cr);
        assert!((total - 1.0).abs() < 0.01, "pan {pan}: potencia {total:.4}");
        assert_eq!(l > r, pan < 0.0, "pan {pan}");
    }
    let (l, r) = power(1.0);
    assert!(l < 1e-6 * r, "a la derecha del todo, nada a la izquierda");
}

/// Una nota con sub y la siguiente (pegada, en la misma voz) sin él: el
/// nivel se suaviza, sin salto (no más que el de la nota sola, con un 20 %
/// de margen). Y abrir el pan del sub a media nota tampoco salta.
#[test]
fn sub_changes_between_notes_do_not_click() {
    let patch = with_sub(0.0, 2, Wave::Sine);
    let mut loud = NoteEvent::new(45.0, 6000);
    loud.sub = synth::SubNote {
        level: Some(1.0),
        ..Default::default()
    };
    let mut quiet = NoteEvent::new(45.0, 6000);
    quiet.sub = synth::SubNote {
        level: Some(0.0),
        pan: Some(0.0),
        ..Default::default()
    };
    let mut wide = NoteEvent::new(45.0, 6000);
    wide.sub = synth::SubNote {
        level: Some(1.0),
        pan: Some(-1.0),
        ..Default::default()
    };
    let notes = [(0, loud), (6000, quiet), (12_000, loud), (18_000, wide)];
    let out = synth::render(&patch, &NO_CURVES, &notes, 24_000, RATE);
    let steady = {
        let only = [(0, loud)];
        let y = synth::render(&patch, &NO_CURVES, &only, 6000, RATE);
        max_step(&left(&y)[1000..])
    };
    for channel in [0, 1] {
        let y: Vec<f32> = out.iter().skip(channel).step_by(2).copied().collect();
        // En el pan abierto, la izquierda sube hasta √2.
        let margin = if channel == 0 { 1.5 } else { 1.2 };
        let step = max_step(&y[1000..]);
        assert!(
            step < margin * steady,
            "canal {channel}: salto {step} (la nota sola, {steady})"
        );
    }
}

/// La cuadrada del sub lleva PolyBLEP en sus saltos: un sub de 2 kHz (una
/// nota de 4 kHz dividida por 2) queda por debajo de -50 dB de aliasing
/// (medido: -56), y el pulso al 25 % de 1 kHz (dividida por 4) también.
#[test]
fn the_square_sub_is_band_limited() {
    let rate = 44_100;
    let frames = 1 << 16;
    let notes = [(0, NoteEvent::new(midi(4000.0), HELD))];
    let skip = rate as usize / 4;
    for (div, wave) in [(2, Wave::Square), (4, Wave::Pulse25)] {
        let patch = with_sub(1.0, div, wave);
        let sub: Vec<f32> = {
            let off = Patch {
                params: {
                    let mut p = patch.params;
                    p[synth::SUB] = 0.0;
                    p
                },
                ..patch
            };
            let on = synth::render(&patch, &NO_CURVES, &notes, skip + frames, rate);
            let off = synth::render(&off, &NO_CURVES, &notes, skip + frames, rate);
            on.iter().zip(&off).map(|(a, b)| a - b).step_by(2).collect()
        };
        let f0 = 4000.0 / div as f64;
        let alias = worst_alias(&sub[skip..], f0, rate);
        println!("sub {wave:?} a {f0} Hz: {alias:.1} dB");
        assert!(alias < -50.0, "{wave:?}: {alias:.1} dB");
    }
}

/// Un patch de capas que se oye tal cual (senos, paso alto a 20 Hz sin
/// resonancia, sostenido) con solo las capas `on` sonando, cada una con
/// su divisor.
fn layered(on: &[(usize, u8)]) -> Patch {
    // La envolvente de las capas, tan larga que no decae en el render.
    let mut p = raw(Wave::Sine).with(synth::LAYER_DECAY, 10_000.0);
    let mut layers = synth::subharm::DEFAULT_LAYERS.map(|l| synth::Layer { level: 0.0, ..l });
    for &(i, div) in on {
        layers[i].div = div;
        layers[i].level = 1.0;
    }
    p.layers = Some(layers);
    p.mono = true;
    p.voices = 1;
    p
}

/// Cada capa suena a su frecuencia: `o1` y `o2` a la nota entre su `div`,
/// y cada sub a la de su oscilador entre el suyo (`s1`, `s2` de `o1`; `s3`,
/// `s4` de `o2`).
#[test]
fn each_layer_sounds_at_its_division() {
    let notes = [(0, NoteEvent::new(81.0, HELD))]; // 880 Hz
    let frames = RATE as usize * 3 / 2;
    // (capa, su div, el div de su oscilador si es un sub)
    for (layer, div, parent_div) in [
        (0, 1, 1),
        (0, 4, 4),
        (1, 3, 3),
        (2, 2, 1),
        (3, 5, 2),
        (4, 7, 1),
        (5, 16, 3),
    ] {
        let parent = synth::LAYER_PARENT[layer];
        let mut on = vec![(layer, div)];
        if layer >= 2 {
            // El oscilador del sub, callado pero con su divisor.
            on.push((parent, parent_div));
        }
        let mut patch = layered(&on);
        if layer >= 2 {
            patch.layers.as_mut().unwrap()[parent].level = 0.0;
        }
        let y = left(&synth::render(&patch, &NO_CURVES, &notes, frames, RATE));
        let second = &y[RATE as usize / 2..RATE as usize / 2 + RATE as usize];
        let expected = if layer < 2 {
            880.0 / div as f64
        } else {
            880.0 / parent_div as f64 / div as f64
        };
        let measured = ups(second) as f64;
        assert!(
            (measured - expected).abs() <= 1.0,
            "capa {}: {measured} ciclos, no {expected:.1}",
            synth::LAYER_NAMES[layer]
        );
    }
}

/// Los subs van atados a su oscilador: con `o1` a f/3 y `s1` a la mitad de
/// él, en todo el render (con un glide de una octava por medio) `o1` hace
/// exactamente el doble de ciclos que `s1`.
#[test]
fn layer_subs_stay_locked_to_their_oscillator() {
    let mut patch = layered(&[(0, 3), (2, 2)]).with(synth::GLIDE, 200.0);
    patch.layers.as_mut().unwrap()[2].level = 0.0;
    let mut slid = NoteEvent::new(69.0, HELD);
    slid.slide = true;
    let notes = [(0, NoteEvent::new(57.0, HELD)), (20_000, slid)];
    let frames = 2 * RATE as usize;
    let main = left(&synth::render(&patch, &NO_CURVES, &notes, frames, RATE));
    let mut only_sub = patch;
    let layers = only_sub.layers.as_mut().unwrap();
    layers[0].level = 0.0;
    layers[2].level = 1.0;
    let sub = left(&synth::render(&only_sub, &NO_CURVES, &notes, frames, RATE));
    let (m, s) = (ups(&main[4000..]), ups(&sub[4000..]));
    assert!(m.abs_diff(2 * s) <= 2, "o1 {m} ciclos, s1 {s}");
}

/// Pan de potencia constante en las seis capas: L² + R² igual con
/// cualquier pan, y en el centro, igual en los dos lados.
#[test]
fn every_layer_pans_with_constant_power() {
    let notes = [(0, NoteEvent::new(57.0, HELD))];
    for layer in 0..synth::LAYERS {
        let power = |pan: f32| {
            let mut on = vec![(layer, 2)];
            if layer >= 2 {
                on.push((synth::LAYER_PARENT[layer], 1));
            }
            let mut patch = layered(&on);
            let layers = patch.layers.as_mut().unwrap();
            if layer >= 2 {
                layers[synth::LAYER_PARENT[layer]].level = 0.0;
            }
            layers[layer].pan = pan;
            let out = synth::render(&patch, &NO_CURVES, &notes, RATE as usize / 2, RATE);
            let (mut l, mut r) = (0.0f64, 0.0f64);
            for f in out[RATE as usize / 4 * 2..].chunks(2) {
                l += (f[0] as f64).powi(2);
                r += (f[1] as f64).powi(2);
            }
            (l, r)
        };
        let (cl, cr) = power(0.0);
        assert!((cl / cr - 1.0).abs() < 1e-6, "capa {layer}");
        for pan in [-1.0, -0.5, 0.25, 0.8, 1.0] {
            let (l, r) = power(pan);
            let total = (l + r) / (cl + cr);
            assert!(
                (total - 1.0).abs() < 0.01,
                "capa {layer}, pan {pan}: {total:.4}"
            );
        }
    }
}

/// Un patch nuevo con otras capas (como al entrar un patrón) a media nota:
/// el nivel y el pan de cada capa se suavizan, sin saltos.
#[test]
fn layer_changes_mid_note_do_not_click() {
    let a = layered(&[(0, 2)]);
    let mut b = layered(&[(0, 2), (1, 3)]);
    let layers = b.layers.as_mut().unwrap();
    layers[0].pan = -1.0;
    layers[0].level = 0.3;
    let clock = synth::SongClock::stopped(6000.0, 96_000);
    let mut s = synth::Synth::new(RATE);
    s.note_on(&a, NoteEvent::new(57.0, HELD));
    let mut out = vec![0.0f32; 2 * RATE as usize];
    for (i, chunk) in out.chunks_mut(512).enumerate() {
        let patch = if i < 40 { &a } else { &b };
        s.mix_into(chunk, patch, &NO_CURVES, &clock);
    }
    for channel in [0, 1] {
        let y: Vec<f32> = out.iter().skip(channel).step_by(2).copied().collect();
        let before = max_step(&y[2000..20_000]);
        let after = max_step(&y[30_000..]);
        let during = max_step(&y[20_000..30_000]);
        assert!(
            during < 1.2 * before.max(after),
            "canal {channel}: {during} (antes {before}, después {after})"
        );
    }
}

/// Ocho voces de subharm con las seis capas al máximo, el filtro abierto,
/// resonancia y accent: tras el limitador, dentro de ±1; y una sola voz
/// así, sin saturar.
#[test]
fn eight_subharm_voices_at_full_load_stay_within_one() {
    let mut patch = synth::subharm::patch(36.0)
        .with(CUTOFF, 20_000.0)
        .with(RESONANCE, 0.8);
    patch.layers = Some(synth::subharm::DEFAULT_LAYERS.map(|l| synth::Layer { level: 1.0, ..l }));
    let mut one = NoteEvent::new(36.0, HELD);
    one.accent = true;
    let single = synth::render(&patch, &NO_CURVES, &[(0, one)], RATE as usize / 2, RATE);
    let peak = single.iter().fold(0.0f32, |m, v| m.max(v.abs()));
    println!(
        "subharm, una voz con todo: pico {:.1} dBFS",
        20.0 * peak.log10()
    );
    assert!(peak < 1.0, "una voz satura: {peak}");
    synth::subharm::param("voices")
        .unwrap()
        .set(&mut patch, 8.0);
    let notes: Vec<_> = (0..8)
        .map(|i| (i * 100, NoteEvent::new(36.0 + 5.0 * i as f32, HELD)))
        .collect();
    let out = synth::render(&patch, &NO_CURVES, &notes, RATE as usize / 2, RATE);
    assert!(
        out.iter()
            .map(|&v| soft_limit(v))
            .all(|v| v.is_finite() && v.abs() <= 1.0)
    );
}
