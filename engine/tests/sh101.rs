//! El sh101: voz mono con prioridad a la última nota y legato,
//! portamento (`off`, `on`, `auto`), una sola envolvente para el filtro y
//! el VCA (o la puerta), el sub en sus tres modos, el filtro (autooscila
//! afinado, estable, `quality` 1 y 2, `res_comp`, `kbd`, `hpf`), el LFO
//! (formas, destinos y `lfo_sync`), el PWM y la deriva.

use engine::synth::{
    self, CUTOFF, ENV_AMOUNT, GlideMode, HELD, LfoWave, NoteEvent, PARAM_COUNT, Patch, PwmSource,
    RESONANCE, SUB, SongClock, Synth, SynthCurves, VcaMode, Wave, sh101,
};

const RATE: u32 = 48_000;
const NO_CURVES: SynthCurves = [None; PARAM_COUNT];

fn left(stereo: &[f32]) -> Vec<f32> {
    stereo.iter().step_by(2).copied().collect()
}

/// Un sh101 con el filtro abierto del todo y sin resonancia ni envolvente
/// al filtro: se oye el oscilador con el VCA.
fn open() -> Patch {
    let mut p = sh101::patch(36.0)
        .with(CUTOFF, 20_000.0)
        .with(RESONANCE, 0.0)
        .with(ENV_AMOUNT, 0.0);
    p.seed = 7;
    p
}

fn set(patch: &mut Patch, name: &str, value: f32) {
    sh101::param(name)
        .expect("parámetro del sh101")
        .set(patch, value);
}

fn with_glide(mut patch: Patch, mode: GlideMode, ms: f32) -> Patch {
    set(&mut patch, "glide", ms);
    patch.analog.as_mut().unwrap().glide = mode;
    patch
}

/// Lo que se toca en vivo: tecla pulsada (con su nota) o soltada.
#[derive(Clone, Copy)]
enum Live {
    On(u8, f32),
    Off(u8),
}

/// Renderiza notas en vivo (frame, evento) con el transporte parado.
fn live(patch: &Patch, events: &[(usize, Live)], frames: usize) -> Vec<f32> {
    let mut synth = Synth::new(RATE);
    let mut out = vec![0.0f32; 2 * frames];
    let clock = SongClock::stopped(RATE as f64 / 8.0, 2 * RATE as u64);
    let mut at = 0;
    let mut next = 0;
    while at < frames {
        while next < events.len() && events[next].0 <= at {
            match events[next].1 {
                Live::On(key, pitch) => {
                    let ev = NoteEvent {
                        key: Some(key),
                        ..NoteEvent::new(pitch, HELD)
                    };
                    synth.note_on(patch, ev);
                }
                Live::Off(key) => synth.note_off(patch, key),
            }
            next += 1;
        }
        let until = events
            .get(next)
            .map_or(frames, |e| e.0)
            .min(at + 256)
            .min(frames);
        synth.mix_into(&mut out[2 * at..2 * until], patch, &NO_CURVES, &clock);
        at = until;
    }
    out
}

/// Instantes (en frames, con decimales) en que `y` cruza el cero subiendo.
fn crossings(y: &[f32]) -> Vec<f64> {
    y.windows(2)
        .enumerate()
        .filter(|(_, w)| w[0] <= 0.0 && w[1] > 0.0)
        .map(|(i, w)| i as f64 + (w[0] / (w[0] - w[1])) as f64)
        .collect()
}

/// La altura (nota MIDI) que suena cerca del frame `at`: el periodo medio
/// de los cruces de una ventana de `width` frames que empieza ahí.
fn pitch_at(y: &[f32], at: usize, width: usize) -> f64 {
    let c = crossings(&y[at..at + width]);
    assert!(c.len() >= 3, "pocos cruces en {at}");
    let period = (c[c.len() - 1] - c[0]) / (c.len() - 1) as f64;
    let hz = RATE as f64 / period;
    69.0 + 12.0 * (hz / 440.0).log2()
}

/// Pico de `y` en una ventana.
fn peak(y: &[f32]) -> f32 {
    y.iter().fold(0.0f32, |m, v| m.max(v.abs()))
}

/// Un sh101 cuya envolvente cae deprisa a un sustain bajo: se ve si una
/// nota vuelve a disparar el ataque.
fn decaying() -> Patch {
    let mut p = open();
    set(&mut p, "a", 2.0);
    set(&mut p, "d", 40.0);
    set(&mut p, "s", 0.2);
    set(&mut p, "r", 30.0);
    p
}

/// En vivo, una tecla que llega con otra pulsada se liga (la envolvente
/// sigue en su sustain, solo cambia la altura); con todas soltadas, la
/// siguiente vuelve a disparar el ataque.
#[test]
fn held_keys_tie_and_released_ones_retrigger() {
    let patch = decaying();
    let ms = |m: usize| m * RATE as usize / 1000;
    let events = [
        (0, Live::On(1, 45.0)),
        (ms(200), Live::On(2, 52.0)),
        (ms(400), Live::Off(1)),
        (ms(400), Live::Off(2)),
        (ms(600), Live::On(3, 52.0)),
    ];
    let y = left(&live(&patch, &events, ms(800)));
    let sustain = peak(&y[ms(150)..ms(195)]);
    let tied = peak(&y[ms(200)..ms(260)]);
    let fresh = peak(&y[ms(600)..ms(660)]);
    println!("sustain {sustain:.3}, ligada {tied:.3}, nueva {fresh:.3}");
    assert!(
        tied < 1.3 * sustain,
        "la ligada no redispara: {tied} vs {sustain}"
    );
    assert!(
        fresh > 3.0 * sustain,
        "la separada sí: {fresh} vs {sustain}"
    );
    assert!((pitch_at(&y, ms(260), ms(60)) - 52.0).abs() < 0.1);
}

/// En un patrón, la nota con slide (que empieza antes de que acabe la
/// anterior) se liga y la que no lo lleva vuelve a disparar.
#[test]
fn slides_tie_in_a_pattern() {
    let patch = decaying();
    let ms = |m: usize| m * RATE as usize / 1000;
    let mut slide = NoteEvent::new(52.0, ms(150) as u64);
    slide.slide = true;
    let notes = [
        (0, NoteEvent::new(45.0, ms(210) as u64)),
        (ms(200), slide),
        (ms(500), NoteEvent::new(45.0, ms(100) as u64)),
    ];
    let y = left(&synth::render(&patch, &NO_CURVES, &notes, ms(700), RATE));
    let sustain = peak(&y[ms(150)..ms(195)]);
    let tied = peak(&y[ms(200)..ms(260)]);
    let fresh = peak(&y[ms(500)..ms(560)]);
    assert!(
        tied < 1.3 * sustain,
        "con slide no redispara: {tied} vs {sustain}"
    );
    assert!(fresh > 3.0 * sustain, "sin slide sí: {fresh} vs {sustain}");
}

/// Prioridad a la última nota: al soltar la última, suena la que sigue
/// pulsada, ligada; al soltarlas todas, el release.
#[test]
fn the_last_note_has_priority() {
    let patch = decaying();
    let ms = |m: usize| m * RATE as usize / 1000;
    let events = [
        (0, Live::On(1, 45.0)),
        (ms(150), Live::On(2, 57.0)),
        (ms(300), Live::Off(2)),
        (ms(450), Live::Off(1)),
    ];
    let y = left(&live(&patch, &events, ms(600)));
    assert!((pitch_at(&y, ms(200), ms(80)) - 57.0).abs() < 0.1);
    assert!((pitch_at(&y, ms(330), ms(100)) - 45.0).abs() < 0.1);
    let sustain = peak(&y[ms(250)..ms(295)]);
    let back = peak(&y[ms(300)..ms(360)]);
    assert!(
        back < 1.3 * sustain,
        "vuelve sin redisparar: {back} vs {sustain}"
    );
    assert!(peak(&y[ms(560)..]) < 1e-3, "suelta todas: release");
}

/// Portamento de A2 a A3 con 200 ms: a los 200 ms está a menos de un 2 %
/// del salto (curva exponencial), entre los 10 y los 40 ms aún no ha
/// pasado de la mitad y poco.
/// En `auto`, solo en las ligadas; en `on`, también en las separadas; en
/// `off`, nunca.
#[test]
fn glide_reaches_the_note_in_its_time() {
    let ms = |m: usize| m * RATE as usize / 1000;
    let glide = 200;
    let tied = |mode: GlideMode| {
        let patch = with_glide(open(), mode, glide as f32);
        let events = [(0, Live::On(1, 45.0)), (ms(300), Live::On(2, 57.0))];
        left(&live(&patch, &events, ms(700)))
    };
    let separate = |mode: GlideMode| {
        let patch = with_glide(open(), mode, glide as f32);
        let events = [
            (0, Live::On(1, 45.0)),
            (ms(280), Live::Off(1)),
            (ms(300), Live::On(2, 57.0)),
        ];
        left(&live(&patch, &events, ms(700)))
    };
    let start = ms(300);
    for (mode, y, glides) in [
        (GlideMode::Auto, tied(GlideMode::Auto), true),
        (GlideMode::Auto, separate(GlideMode::Auto), false),
        (GlideMode::On, tied(GlideMode::On), true),
        (GlideMode::On, separate(GlideMode::On), true),
        (GlideMode::Off, tied(GlideMode::Off), false),
    ] {
        // Ventanas de 30 ms: a 110-220 Hz, tres ciclos o más.
        let early = pitch_at(&y, start + ms(10), ms(30));
        let late = pitch_at(&y, start + ms(glide), ms(30));
        println!(
            "{mode:?} (desliza: {glides}): a los 10-40 ms {early:.2}, a los {glide} ms {late:.2}"
        );
        assert!(
            (late - 57.0).abs() < 0.24,
            "{mode:?}: a los {glide} ms, {late:.2}"
        );
        if glides {
            assert!(
                early < 52.5,
                "{mode:?}: a los 10-40 ms ya está en {early:.2}"
            );
        } else {
            assert!(
                (early - 57.0).abs() < 0.1,
                "{mode:?}: no desliza, {early:.2}"
            );
        }
    }
}

/// Con `vca = "gate"`, la amplitud no sigue a la envolvente: suena igual
/// mientras dura la nota aunque la envolvente caiga a 0 (y la envolvente
/// sigue moviendo el filtro); con `env`, cae con ella.
#[test]
fn the_vca_follows_the_gate_or_the_envelope() {
    let ms = |m: usize| m * RATE as usize / 1000;
    let mut patch = decaying();
    set(&mut patch, "s", 0.0);
    let notes = [(0, NoteEvent::new(45.0, ms(300) as u64))];
    let env = left(&synth::render(&patch, &NO_CURVES, &notes, ms(400), RATE));
    patch.analog.as_mut().unwrap().vca = VcaMode::Gate;
    let gate = left(&synth::render(&patch, &NO_CURVES, &notes, ms(400), RATE));
    let (start, end) = (peak(&gate[ms(10)..ms(30)]), peak(&gate[ms(250)..ms(290)]));
    assert!((end / start - 1.0).abs() < 0.05, "puerta: {start} → {end}");
    assert!(
        peak(&env[ms(250)..ms(290)]) < 0.01 * start,
        "envolvente: cae"
    );
    assert!(peak(&gate[ms(310)..]) < 1e-3, "puerta: se cierra al soltar");
}

/// El sub del sh101 en sus tres modos (el bloque sub): cuadrada a -1 y -2
/// octavas y el pulso al 25 % a -2, a f/2, f/4 y f/4.
#[test]
fn the_three_sub_modes() {
    let frames = RATE as usize * 3 / 2;
    let notes = [(0, NoteEvent::new(69.0, HELD))];
    for (div, wave) in [(2, Wave::Square), (4, Wave::Square), (4, Wave::Pulse25)] {
        let mut patch = open().with(SUB, 1.0);
        set(&mut patch, "saw", 0.0);
        set(&mut patch, "s", 1.0);
        patch.sub_div = div;
        patch.sub_wave = wave;
        let y = left(&synth::render(&patch, &NO_CURVES, &notes, frames, RATE));
        let second = &y[RATE as usize / 2..RATE as usize / 2 + RATE as usize];
        let expected = 440.0 / div as f64;
        let measured = crossings(second).len() as f64;
        assert!(
            (measured - expected).abs() <= 1.0,
            "{wave:?} ÷{div}: {measured} ciclos, no {expected}"
        );
    }
}

/// La misma semilla da el mismo audio bit a bit; otra, otras fases.
#[test]
fn same_seed_same_audio() {
    let notes = [
        (0, NoteEvent::new(40.0, 20_000)),
        (24_000, NoteEvent::new(47.0, 9000)),
    ];
    let mut patch = sh101::patch(36.0).with(SUB, 0.5);
    set(&mut patch, "noise", 0.3);
    set(&mut patch, "pulse", 0.6);
    let a = synth::render(&patch, &NO_CURVES, &notes, 40_000, RATE);
    let b = synth::render(&patch, &NO_CURVES, &notes, 40_000, RATE);
    assert!(a.iter().zip(&b).all(|(x, y)| x.to_bits() == y.to_bits()));
    patch.seed = 99;
    let c = synth::render(&patch, &NO_CURVES, &notes, 40_000, RATE);
    assert!(a != c);
}

/// Los valores por defecto del preset caben en los rangos del núcleo.
#[test]
fn the_preset_is_valid() {
    assert_eq!(sh101::patch(36.0).validate(), Ok(()));
    for p in sh101::PARAMS {
        let mut patch = sh101::patch(36.0);
        p.set(&mut patch, p.min);
        assert_eq!(patch.validate(), Ok(()), "{} al mínimo", p.name);
        p.set(&mut patch, p.max);
        assert_eq!(patch.validate(), Ok(()), "{} al máximo", p.name);
    }
}

/// Amplitud de la componente de `hz` en `y` (una sola frecuencia de la
/// DFT).
fn amplitude(y: &[f32], hz: f64) -> f64 {
    let w = std::f64::consts::TAU * hz / RATE as f64;
    let (mut re, mut im) = (0.0, 0.0);
    for (n, &v) in y.iter().enumerate() {
        re += v as f64 * (w * n as f64).cos();
        im -= v as f64 * (w * n as f64).sin();
    }
    2.0 * (re * re + im * im).sqrt() / y.len() as f64
}

fn db(x: f64) -> f64 {
    20.0 * x.log10()
}

/// Un sh101 sin oscilador, con la resonancia al máximo y un poco de ruido
/// que lo despierte: lo que suena es el filtro autooscilando en `hz`.
fn ringing(hz: f32) -> Patch {
    let mut p = open().with(CUTOFF, hz).with(RESONANCE, 1.0);
    set(&mut p, "saw", 0.0);
    set(&mut p, "noise", 0.01);
    set(&mut p, "s", 1.0);
    p
}

/// La frecuencia que suena en `y` (de sus cruces por cero subiendo).
fn frequency(y: &[f32]) -> f64 {
    let c = crossings(y);
    RATE as f64 * (c.len() - 1) as f64 / (c[c.len() - 1] - c[0])
}

/// Con `res` = 1 el filtro autooscila cerca de su corte: en grave (110
/// Hz), en medio (1 kHz) y cerca de Nyquist (18 kHz a 48 kHz), con
/// `quality` 1 y 2, con el error de afinación acotado (y medido, en
/// cents).
#[test]
fn the_filter_self_oscillates_at_its_cutoff() {
    let notes = [(0, NoteEvent::new(36.0, HELD))];
    let frames = RATE as usize;
    for quality in [1.0, 2.0] {
        for (hz, cents) in [(110.0, 25.0), (1000.0, 25.0), (18_000.0, 100.0)] {
            let mut patch = ringing(hz);
            set(&mut patch, "quality", quality);
            let y = left(&synth::render(&patch, &NO_CURVES, &notes, frames, RATE));
            let tail = &y[frames / 2..];
            let measured = frequency(tail);
            let error = 1200.0 * (measured / hz as f64).log2();
            let level = db(peak(tail) as f64);
            println!(
                "quality {quality}: autooscila a {hz} Hz: {measured:.1} Hz ({error:+.1} cents), {level:.1} dBFS"
            );
            let who = format!("quality {quality}, {hz} Hz");
            assert!(error.abs() < cents, "{who}: {error:+.1} cents");
            assert!(level > -30.0, "{who}: no suena ({level:.1} dBFS)");
        }
    }
}

/// Ningún NaN ni inf en los extremos: corte en 20 Hz y en 20 kHz (y más
/// arriba con el teclado y la envolvente), resonancia al máximo, todo el
/// oscilador, el sub saturado y ruido a la vez, notas muy agudas y muy
/// graves; con `quality` 1 y 2.
#[test]
fn the_filter_is_stable_at_the_extremes() {
    for quality in [1.0, 2.0] {
        for cutoff in [20.0, 20_000.0] {
            for env in [-1.0, 1.0] {
                let mut p = open()
                    .with(CUTOFF, cutoff)
                    .with(RESONANCE, 1.0)
                    .with(SUB, 1.0)
                    .with(synth::SUB_DRIVE, 1.0);
                for (name, value) in [
                    ("pulse", 1.0),
                    ("noise", 1.0),
                    ("kbd", 1.0),
                    ("res_comp", 1.0),
                    ("env_amt", env),
                    ("a", 0.0),
                    ("d", 5.0),
                    ("quality", quality),
                ] {
                    set(&mut p, name, value);
                }
                let notes = [
                    (0, NoteEvent::new(127.0, 6000)),
                    (6000, NoteEvent::new(0.0, 6000)),
                    (12_000, NoteEvent::new(100.0, 6000)),
                ];
                let y = synth::render(&p, &NO_CURVES, &notes, 20_000, RATE);
                let who = format!("quality {quality}, corte {cutoff}, env {env}");
                assert!(y.iter().all(|v| v.is_finite()), "{who}");
                assert!(peak(&y) < 4.0, "{who}: {}", peak(&y));
            }
        }
    }
}

/// Como en el original, los graves caen al subir la resonancia; con
/// `res_comp` = 1 se quedan casi donde estaban.
#[test]
fn resonance_thins_the_bass_unless_compensated() {
    let notes = [(0, NoteEvent::new(33.0, HELD))];
    let fundamental = |res: f32, comp: f32| {
        let mut p = open().with(CUTOFF, 800.0).with(RESONANCE, res);
        set(&mut p, "res_comp", comp);
        set(&mut p, "s", 1.0);
        let y = left(&synth::render(&p, &NO_CURVES, &notes, RATE as usize, RATE));
        amplitude(&y[RATE as usize / 2..], 55.0)
    };
    let drop = db(fundamental(0.8, 0.0) / fundamental(0.0, 0.0));
    let kept = db(fundamental(0.8, 1.0) / fundamental(0.0, 1.0));
    println!("55 Hz con res 0,8: {drop:+.1} dB sin compensar, {kept:+.1} dB con res_comp 1");
    assert!(drop < -6.0, "sin compensar: {drop:+.1} dB");
    assert!(kept.abs() < 2.0, "compensado: {kept:+.1} dB");
}

/// El paso alto: cada posición quita más de un grave de 55 Hz, y la 0 no
/// toca nada.
#[test]
fn the_highpass_positions() {
    let notes = [(0, NoteEvent::new(33.0, HELD))];
    let at = |hpf: f32| {
        let mut p = open();
        set(&mut p, "hpf", hpf);
        set(&mut p, "s", 1.0);
        let y = left(&synth::render(&p, &NO_CURVES, &notes, RATE as usize, RATE));
        amplitude(&y[RATE as usize / 2..], 55.0)
    };
    let levels: Vec<f64> = (0..4).map(|h| db(at(h as f32) / at(0.0))).collect();
    println!("55 Hz con hpf 0 a 3: {levels:.1?} dB");
    assert!(levels.windows(2).all(|w| w[1] < w[0] - 2.0), "{levels:?}");
    assert!(levels[3] < -15.0, "{levels:?}");
}

/// El seguimiento de teclado: con `kbd` = 1, una octava más arriba de la
/// raíz, el filtro autooscila una octava más arriba; con 0, igual.
#[test]
fn the_keyboard_tracks_the_cutoff() {
    let frames = RATE as usize;
    let at = |kbd: f32, note: f32| {
        let mut p = ringing(500.0);
        set(&mut p, "kbd", kbd);
        let notes = [(0, NoteEvent::new(note, HELD))];
        let y = left(&synth::render(&p, &NO_CURVES, &notes, frames, RATE));
        frequency(&y[frames / 2..])
    };
    let root = at(1.0, 36.0);
    let up = at(1.0, 48.0);
    let fixed = at(0.0, 48.0);
    assert!(
        (up / root - 2.0).abs() < 0.02,
        "kbd 1: {root:.1} → {up:.1} Hz"
    );
    assert!(
        (fixed / root - 1.0).abs() < 0.01,
        "kbd 0: {root:.1} → {fixed:.1} Hz"
    );
}

/// El nivel: con todo el oscilador, el sub, la resonancia alta y
/// compensada, el pico queda por debajo de -1 dBFS; el preset por
/// defecto, por encima de -18.
#[test]
fn the_sh101_level() {
    let notes = [
        (0, NoteEvent::new(36.0, 30_000)),
        (36_000, NoteEvent::new(48.0, 30_000)),
    ];
    let mut loud = sh101::patch(36.0).with(SUB, 1.0).with(RESONANCE, 0.9);
    set(&mut loud, "pulse", 1.0);
    set(&mut loud, "env_amt", 1.0);
    set(&mut loud, "res_comp", 1.0);
    let plain = sh101::patch(36.0);
    let worst = db(peak(&synth::render(&loud, &NO_CURVES, &notes, 70_000, RATE)) as f64);
    let quiet = db(peak(&synth::render(&plain, &NO_CURVES, &notes, 70_000, RATE)) as f64);
    println!("sh101: pico {worst:.1} dBFS con todo, {quiet:.1} dBFS por defecto");
    assert!(worst < -1.0, "con todo: {worst:.1} dBFS");
    assert!(quiet > -18.0, "por defecto: {quiet:.1} dBFS");
}

/// Fracción (en dB) de la energía de `y` que no está en los armónicos de
/// `f0`: lo que la saturación ha plegado. Cada armónico ocupa ±30 bins
/// (±22 Hz): el barrido los ensancha.
fn inharmonic_db(y: &[f32], f0: f64) -> f64 {
    use rustfft::{FftPlanner, num_complex::Complex};
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
    let bin = RATE as f64 / n as f64;
    let (mut total, mut off) = (0.0, 0.0);
    for (i, c) in buf[1..n / 2].iter().enumerate() {
        let hz = (i + 1) as f64 * bin;
        let p = c.norm_sqr();
        total += p;
        let nearest = (hz / f0).round() * f0;
        if (hz - nearest).abs() > 30.0 * bin {
            off += p;
        }
    }
    10.0 * (off / total).log10()
}

/// Con `quality` 2 el filtro pliega menos: un barrido del corte de 500 Hz a
/// 12 kHz con la resonancia alta y la entrada caliente (sierra, pulso y
/// la compensación de graves, que la satura) en una nota de 2,3 kHz deja
/// menos energía fuera de los armónicos que con 1 (medido: -25,1 dB con 1
/// y -32,8 con 2).
#[test]
fn quality_2_aliases_less_than_1() {
    let frames = 1 << 16;
    let note = 69.0 + 12.0 * (2345.6f32 / 440.0).log2();
    // La altura exacta de la nota en f32.
    let f0 = 440.0 * ((note as f64 - 69.0) / 12.0).exp2();
    let mut curves = NO_CURVES;
    curves[CUTOFF] = Some(engine::Curve::Ramp {
        from: 500.0,
        to: 12_000.0,
        steps: 10.0,
    });
    let skip = RATE as usize / 10;
    let alias = |quality: f32| {
        let mut patch = open().with(RESONANCE, 0.9);
        for (name, value) in [("pulse", 1.0), ("pw", 0.3), ("res_comp", 1.0), ("s", 1.0)] {
            set(&mut patch, name, value);
        }
        set(&mut patch, "quality", quality);
        let notes = [(0, NoteEvent::new(note, HELD))];
        let y = left(&synth::render(&patch, &curves, &notes, skip + frames, RATE));
        inharmonic_db(&y[skip..], f0)
    };
    let (one, two) = (alias(1.0), alias(2.0));
    println!("energía fuera de los armónicos: quality 1 {one:.1} dB, quality 2 {two:.1} dB");
    assert!(
        two < one - 5.0,
        "quality 1 {one:.1} dB, quality 2 {two:.1} dB"
    );
}

fn analog(patch: &mut Patch) -> &mut synth::Analog {
    patch.analog.as_mut().expect("voz analógica")
}

/// Las alturas que suenan, en ventanas de `width` frames seguidas.
fn pitch_trace(y: &[f32], from: usize, width: usize) -> Vec<f64> {
    (from..y.len() - width)
        .step_by(width)
        .map(|at| pitch_at(y, at, width))
        .collect()
}

/// El LFO triangular a la altura: ±1 semitono a 4 Hz alrededor de la nota;
/// el cuadrado salta entre los dos extremos; el aleatorio da escalones que
/// cambian (y los mismos con la misma semilla).
#[test]
fn the_lfo_moves_the_pitch() {
    let frames = 2 * RATE as usize;
    let notes = [(0, NoteEvent::new(57.0, HELD))];
    let render = |wave: LfoWave| {
        let mut p = open();
        set(&mut p, "s", 1.0);
        set(&mut p, "lfo_rate", 4.0);
        set(&mut p, "lfo_pitch", 1.0);
        analog(&mut p).lfo_wave = wave;
        left(&synth::render(&p, &NO_CURVES, &notes, frames, RATE))
    };
    // Ventanas de 15 ms: más de 3 ciclos de 220 Hz, 16 por ciclo del LFO.
    let width = 3 * RATE as usize / 200;
    let tri = pitch_trace(&render(LfoWave::Tri), RATE as usize / 2, width);
    let (lo, hi) = tri
        .iter()
        .fold((f64::MAX, f64::MIN), |(a, b), &v| (a.min(v), b.max(v)));
    println!("LFO triangular: de {lo:.2} a {hi:.2}");
    assert!(
        (lo - 56.0).abs() < 0.15 && (hi - 58.0).abs() < 0.15,
        "{lo:.2}..{hi:.2}"
    );
    let square = pitch_trace(&render(LfoWave::Square), RATE as usize / 2, width);
    let near = |v: f64, to: f64| (v - to).abs() < 0.05;
    let edges = square
        .iter()
        .filter(|&&v| !near(v, 56.0) && !near(v, 58.0))
        .count();
    assert!(
        edges <= square.len() / 6,
        "cuadrado: {edges} de {} fuera de los extremos",
        square.len()
    );
    let random = render(LfoWave::Random);
    let steps = pitch_trace(&random, RATE as usize / 2, width);
    let distinct = {
        let mut v: Vec<i64> = steps.iter().map(|p| (p * 20.0).round() as i64).collect();
        v.sort();
        v.dedup();
        v.len()
    };
    assert!(distinct >= 4, "aleatorio: {distinct} valores distintos");
    assert!(
        random == render(LfoWave::Random),
        "aleatorio: la misma semilla, lo mismo"
    );
}

/// El LFO al corte: el filtro autooscilando sube y baja con él.
#[test]
fn the_lfo_moves_the_cutoff() {
    let mut p = ringing(1000.0);
    set(&mut p, "lfo_rate", 2.0);
    set(&mut p, "lfo_filter", 1.0);
    let notes = [(0, NoteEvent::new(36.0, HELD))];
    let y = left(&synth::render(
        &p,
        &NO_CURVES,
        &notes,
        2 * RATE as usize,
        RATE,
    ));
    let trace = pitch_trace(&y, RATE as usize / 2, RATE as usize / 50);
    let (lo, hi) = trace
        .iter()
        .fold((f64::MAX, f64::MIN), |(a, b), &v| (a.min(v), b.max(v)));
    // Una octava arriba y abajo de 1 kHz (nota 83,2): ±12 semitonos.
    println!("LFO al corte: de {lo:.1} a {hi:.1}");
    assert!(hi - lo > 20.0, "{lo:.1}..{hi:.1}");
}

/// Con `lfo_sync` = n, el LFO da n ciclos por compás, en fase con él: el
/// cuadrado cambia exactamente en cada 1/(2n) del compás (en `render`, a
/// 120 BPM, compases de 2 s).
#[test]
fn lfo_sync_locks_to_the_bar() {
    let bar = 2 * RATE as usize;
    for n in [1, 3, 4] {
        let mut p = open();
        set(&mut p, "s", 1.0);
        set(&mut p, "lfo_pitch", 12.0);
        set(&mut p, "lfo_rate", 0.05);
        set(&mut p, "lfo_sync", n as f32);
        analog(&mut p).lfo_wave = LfoWave::Square;
        // La nota empieza a mitad de un compás: el LFO sigue al compás.
        let notes = [(bar / 3, NoteEvent::new(57.0, HELD))];
        let y = left(&synth::render(&p, &NO_CURVES, &notes, 2 * bar, RATE));
        let half = bar / (2 * n);
        // 30 ms: más de 3 ciclos de la nota más grave (110 Hz).
        let probe = 3 * RATE as usize / 100;
        for k in 1..(4 * n) {
            let edge = k * half;
            if edge < bar / 3 + 2 * probe {
                continue;
            }
            let before = pitch_at(&y, edge - probe - 200, probe);
            let after = pitch_at(&y, edge + 200, probe);
            let up = if k % 2 == 0 { 69.0 } else { 45.0 };
            let down = if k % 2 == 0 { 45.0 } else { 69.0 };
            assert!(
                (before - down).abs() < 0.2 && (after - up).abs() < 0.2,
                "lfo_sync {n}, marca {k}: {before:.2} → {after:.2}"
            );
        }
    }
}

/// El PWM: con `pwm_src` = lfo el ancho de pulso sube y baja; con env,
/// sigue a la envolvente (ancho al disparar, estrecho al caer); con
/// manual, `pwm_amt` no hace nada.
#[test]
fn pwm_follows_its_source() {
    let frames = 2 * RATE as usize;
    let notes = [(0, NoteEvent::new(45.0, HELD))];
    let duty = |src: PwmSource| {
        let mut p = open();
        set(&mut p, "saw", 0.0);
        set(&mut p, "pulse", 1.0);
        set(&mut p, "pw", 0.5);
        set(&mut p, "pwm_amt", 0.8);
        set(&mut p, "lfo_rate", 2.0);
        set(&mut p, "d", 2000.0);
        set(&mut p, "s", 0.0);
        analog(&mut p).pwm = src;
        analog(&mut p).vca = VcaMode::Gate;
        let y = left(&synth::render(&p, &NO_CURVES, &notes, frames, RATE));
        // La fracción del periodo en alto, en ventanas de 50 ms.
        let width = RATE as usize / 20;
        (RATE as usize / 20..frames - width)
            .step_by(width)
            .map(|at| {
                let w = &y[at..at + width];
                let mean = w.iter().sum::<f32>() / w.len() as f32;
                w.iter().filter(|&&v| v > mean).count() as f64 / w.len() as f64
            })
            .collect::<Vec<f64>>()
    };
    let span = |v: &[f64]| {
        v.iter()
            .fold((f64::MAX, f64::MIN), |(a, b), &x| (a.min(x), b.max(x)))
    };
    let (lo, hi) = span(&duty(PwmSource::Lfo));
    println!(
        "PWM por LFO: del {:.0} % al {:.0} %",
        100.0 * lo,
        100.0 * hi
    );
    assert!(lo < 0.25 && hi > 0.75, "lfo: {lo:.2}..{hi:.2}");
    let env = duty(PwmSource::Env);
    assert!(env[0] > 0.75 && env[env.len() - 1] < 0.55, "env: {env:.2?}");
    let (lo, hi) = span(&duty(PwmSource::Manual));
    assert!(hi - lo < 0.03, "manual: {lo:.2}..{hi:.2}");
}

/// La deriva: con 0, el audio es el de siempre bit a bit; con 1, la
/// afinación se mueve despacio, nunca más de ±10 cents, y es la misma con
/// la misma semilla.
#[test]
fn drift_is_small_slow_and_reproducible() {
    let frames = 4 * RATE as usize;
    let notes = [(0, NoteEvent::new(57.0, HELD))];
    let mut p = open();
    set(&mut p, "s", 1.0);
    let plain = synth::render(&p, &NO_CURVES, &notes, frames, RATE);
    set(&mut p, "drift", 0.0);
    assert!(plain == synth::render(&p, &NO_CURVES, &notes, frames, RATE));
    set(&mut p, "drift", 1.0);
    let drifting = synth::render(&p, &NO_CURVES, &notes, frames, RATE);
    assert!(drifting == synth::render(&p, &NO_CURVES, &notes, frames, RATE));
    let trace = pitch_trace(&left(&drifting), RATE as usize / 2, RATE as usize / 10);
    let cents: Vec<f64> = trace.iter().map(|v| 100.0 * (v - 57.0)).collect();
    let (lo, hi) = cents
        .iter()
        .fold((f64::MAX, f64::MIN), |(a, b), &x| (a.min(x), b.max(x)));
    println!("deriva: de {lo:+.1} a {hi:+.1} cents");
    assert!(lo > -10.5 && hi < 10.5, "{lo:+.1}..{hi:+.1} cents");
    assert!(hi - lo > 1.0, "se mueve: {lo:+.1}..{hi:+.1} cents");
}
