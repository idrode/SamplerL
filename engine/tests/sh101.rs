//! El sh101: voz mono con prioridad a la última nota y legato,
//! portamento (`off`, `on`, `auto`), una sola envolvente para el filtro y
//! el VCA (o la puerta), y el sub en sus tres modos.

use engine::synth::{
    self, CUTOFF, ENV_AMOUNT, GlideMode, HELD, NoteEvent, PARAM_COUNT, Patch, RESONANCE, SUB,
    SongClock, Synth, SynthCurves, VcaMode, Wave, sh101,
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
/// Hz), en medio (1 kHz) y cerca de Nyquist (18 kHz a 48 kHz), con el
/// error de afinación acotado (y medido, en cents).
#[test]
fn the_filter_self_oscillates_at_its_cutoff() {
    let notes = [(0, NoteEvent::new(36.0, HELD))];
    let frames = RATE as usize;
    for (hz, cents) in [(110.0, 25.0), (1000.0, 25.0), (18_000.0, 100.0)] {
        let y = left(&synth::render(
            &ringing(hz),
            &NO_CURVES,
            &notes,
            frames,
            RATE,
        ));
        let tail = &y[frames / 2..];
        let measured = frequency(tail);
        let error = 1200.0 * (measured / hz as f64).log2();
        let level = db(peak(tail) as f64);
        println!("autooscila a {hz} Hz: {measured:.1} Hz ({error:+.1} cents), {level:.1} dBFS");
        assert!(error.abs() < cents, "{hz} Hz: {error:+.1} cents");
        assert!(level > -30.0, "{hz} Hz: no suena ({level:.1} dBFS)");
    }
}

/// Ningún NaN ni inf en los extremos: corte en 20 Hz y en 20 kHz (y más
/// arriba con el teclado y la envolvente), resonancia al máximo, todo el
/// oscilador, el sub saturado y ruido a la vez, notas muy agudas y muy
/// graves.
#[test]
fn the_filter_is_stable_at_the_extremes() {
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
            ] {
                set(&mut p, name, value);
            }
            let notes = [
                (0, NoteEvent::new(127.0, 6000)),
                (6000, NoteEvent::new(0.0, 6000)),
                (12_000, NoteEvent::new(100.0, 6000)),
            ];
            let y = synth::render(&p, &NO_CURVES, &notes, 20_000, RATE);
            assert!(y.iter().all(|v| v.is_finite()), "corte {cutoff}, env {env}");
            assert!(peak(&y) < 4.0, "corte {cutoff}, env {env}: {}", peak(&y));
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
