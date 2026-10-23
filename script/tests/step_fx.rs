//! Efectos por paso en Lua (`steps("1 . 2{crush=4} .")`): suenan igual
//! que el motor con la misma cadena, solo cambian su golpe, valen en
//! `every`, mandan sobre los del corte, entran al compás y los errores
//! dicen la línea sin dejar de sonar lo anterior.

use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock};

use engine::{Fx, Pattern, Sample, Slices, Step, new_engine, offline};
use script::{ScriptHost, Song};

const RATE: u32 = 48_000;
const STEPS: &str = "1 . 2 .  1 . 2 2  . 1 . 2  1 . . 2";
const HEAD: &str = "bpm(120)\nlocal b = sample(\"break.wav\"):slice(2)\n";

fn dir() -> PathBuf {
    static DIR: OnceLock<PathBuf> = OnceLock::new();
    DIR.get_or_init(|| {
        let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join("step_fx");
        std::fs::create_dir_all(&dir).unwrap();
        // Dos golpes de ruido (determinista) con silencio detrás.
        let mut x = 7u32;
        let mut noise = || {
            x ^= x << 13;
            x ^= x >> 17;
            x ^= x << 5;
            (x as f32 / u32::MAX as f32 - 0.5) * 0.6
        };
        let mut mono = vec![0.0f32; 12_000];
        for s in &mut mono[..2500] {
            *s = noise();
        }
        for s in &mut mono[6000..8500] {
            *s = 0.5 * noise();
        }
        offline::write_wav(dir.join("break.wav"), &mono, 1, RATE).unwrap();
        dir
    })
    .clone()
}

fn song(source: &str) -> Result<Song, String> {
    let (mut controller, _engine) = new_engine(RATE);
    let mut host = ScriptHost::new(RATE);
    host.load(
        &mut controller,
        &format!("{HEAD}{source}"),
        "song.lua",
        &dir(),
    )
    .cloned()
    .map_err(|e| e.to_string())
}

fn error(source: &str) -> String {
    song(source).err().expect("debería fallar")
}

fn bar() -> usize {
    engine::bar_frames(120.0, RATE) as usize
}

#[test]
fn step_effects_sound_like_the_engine_and_only_change_their_hit() {
    let with = "1 . 2 .  1 . 2{crush=4,downsample=3}{lp=900,q=5} 2  . 1 . 2  1 . . 2";
    let fx = song(&format!("track(\"d\", b):steps(\"{with}\")")).unwrap();
    let plain = song(&format!("track(\"d\", b):steps(\"{STEPS}\")")).unwrap();
    // La cadena del paso llega al modelo.
    let step = fx.tracks[0].steps[6].unwrap();
    let chain: Vec<Fx> = step.fx.unwrap().effects().copied().collect();
    assert_eq!(
        chain,
        [
            Fx::bitcrush(4, 3),
            Fx::Filter {
                kind: engine::FilterKind::LowPass,
                cutoff_hz: 900.0,
                q: 5.0
            }
        ]
    );
    // Igual que el motor con esos pasos.
    let sample = Arc::new(Sample::load_wav(dir().join("break.wav"), RATE).unwrap());
    let slices = Slices::detect(&sample, 2);
    let mut steps = engine::notation::parse_steps(STEPS).unwrap();
    steps[6] = Some(Step::new(1).with_fx(step.fx));
    let pattern = Pattern::from_steps(sample, &slices, steps).unwrap();
    let (mut controller, mut engine) = new_engine(RATE);
    controller.set_bpm(120.0).unwrap();
    controller.set_pattern(pattern).unwrap();
    controller.start().unwrap();
    let expected = offline::render(&mut engine, 2 * bar(), 2);
    let audio = fx.render(RATE, 2).unwrap();
    assert_eq!(audio, expected);
    // Solo cambia el golpe del paso 6 (y la cola de su filtro, 100 ms); lo
    // demás, bit a bit.
    let base = plain.render(RATE, 2).unwrap();
    let step_len = bar() / 16;
    let (from, to) = (6 * step_len, 7 * step_len + 4800 + 1000);
    assert_eq!(audio[..2 * from], base[..2 * from]);
    assert_ne!(audio[2 * from..2 * to], base[2 * from..2 * to]);
    assert_eq!(audio[2 * to..2 * bar()], base[2 * to..2 * bar()]);
    // Y en el segundo compás, igual que en el primero.
    assert_eq!(audio[2 * bar()..], audio[..2 * bar()]);
}

#[test]
fn every_chance_and_generated_patterns_keep_step_effects() {
    let song = song(
        "track(\"d\", b):steps(\"1 . 2 .  1 . 2 2  . 1 . 2  1 . . 2\")\n  \
         :every(2, function(c) c:steps(\"1{drive=12} . . .  . . . .  . . . .  . . . .\") end)\n\
         track(\"e\", b):steps(rotate(\"1{gain=0.3} . . .  2 . . .  . . . .  . . . .\", 4))",
    )
    .unwrap();
    let d = &song.tracks[0];
    assert!(d.bars[0][0].unwrap().fx.is_some());
    assert!(d.bars[1][0].unwrap().fx.is_none());
    let e = &song.tracks[1];
    assert!(e.steps[4].unwrap().fx.is_some());
}

#[test]
fn the_step_wins_over_its_slice() {
    // El corte 2 tiene un paso bajo de 300; el paso 6 pone uno de 6000: en
    // ese golpe suena el de 6000 (como si el corte lo tuviera), en los
    // demás golpes del corte 2, el de 300.
    let both = song(
        "b:fx(2, \"lowpass\", { cutoff = 300 })\n\
         track(\"d\", b):steps(\"1 . 2 .  1 . 2{lp=6000} 2  . 1 . 2  1 . . 2\")",
    )
    .unwrap();
    let only_step = song(&format!(
        "b:fx(2, \"lowpass\", {{ cutoff = 6000 }})\n\
         track(\"d\", b):steps(\"{STEPS}\")"
    ))
    .unwrap();
    let slice_only = song(&format!(
        "b:fx(2, \"lowpass\", {{ cutoff = 300 }})\n\
         track(\"d\", b):steps(\"{STEPS}\")"
    ))
    .unwrap();
    let (a, b6, s) = (
        both.render(RATE, 1).unwrap(),
        only_step.render(RATE, 1).unwrap(),
        slice_only.render(RATE, 1).unwrap(),
    );
    let step = bar() / 16;
    let at = |audio: &[f32], k: usize| audio[2 * k * step..2 * (k + 1) * step].to_vec();
    assert_eq!(at(&a, 6), at(&b6, 6));
    assert_eq!(at(&a, 2), at(&s, 2));
    // Que no quepan: error con la línea.
    let message = error(
        "b:fx(2, \"lowpass\", { cutoff = 300 })\nb:fx(2, \"gain\", { level = 1 })\n\
         b:fx(2, \"comp\", {})\n\
         track(\"d\", b):steps(\"1 . 2{crush}{drive} .  . . . .  . . . .  . . . .\")",
    );
    assert!(
        message.starts_with("song.lua:6: steps: paso 3: ") && message.contains("más de 4 efectos"),
        "{message}"
    );
    // También si el corte recibe sus efectos después de los pasos.
    let message = error(
        "track(\"d\", b):steps(\"1 . 2{crush}{drive} .  . . . .  . . . .  . . . .\")\n\
         b:fx(2, \"lowpass\", { cutoff = 300 })\nb:fx(2, \"gain\", { level = 1 })\n\
         b:fx(2, \"comp\", {})",
    );
    assert!(
        message.starts_with("song.lua:3: steps: paso 3: "),
        "{message}"
    );
}

#[test]
fn errors_point_at_the_line_and_the_old_song_keeps_playing() {
    let steps =
        |s: &str| format!("track(\"d\", b):steps(\"{s} . . .  . . . .  . . . .  . . . .\")");
    for (source, fragment) in [
        (steps("1{zap=1}"), "{zap}: el efecto no existe"),
        (
            steps("1{lp=800,zz=2}"),
            "{lp}: parámetro desconocido \"zz\"",
        ),
        (steps("1{lp=5}"), "{lp}: frecuencia de corte fuera de rango"),
        (
            steps("1{filter,cutoff=900,type=1>2}"),
            "type no admite rampas",
        ),
        (steps("1{lp=800"), "falta } al final"),
        (
            "local i = sample(\"break.wav\"):instrument()\n\
             track(\"m\", i):steps(\"1{lp=800} . . .  . . . .  . . . .  . . . .\")"
                .to_string(),
            "es un instrumento: sus notas no tienen efectos por paso",
        ),
        (
            "track(\"g\", b):grains({}):steps(\"1{lp=800} . . .  . . . .  . . . .  . . . .\")"
                .to_string(),
            "toca en granos",
        ),
        (
            "track(\"g\", b):steps(\"1{lp=800} . . .  . . . .  . . . .  . . . .\"):grains({})"
                .to_string(),
            "toca en granos",
        ),
    ] {
        let message = error(&source);
        let line = if source.contains("instrument") {
            "song.lua:4:"
        } else {
            "song.lua:3:"
        };
        assert!(
            message.starts_with(line) && message.contains(fragment),
            "{source}\n→ {message}"
        );
    }
    // Lo que sonaba sigue sonando: una carga mala no cambia nada.
    let (mut controller, mut engine) = new_engine(RATE);
    let mut host = ScriptHost::new(RATE);
    let good = format!("{HEAD}track(\"d\", b):steps(\"{STEPS}\")");
    host.load(&mut controller, &good, "song.lua", &dir())
        .unwrap();
    controller.start().unwrap();
    let mut heard = offline::render(&mut engine, bar() / 2, 2);
    let bad = format!("{HEAD}{}", steps("1{lp=5}"));
    assert!(
        host.load(&mut controller, &bad, "song.lua", &dir())
            .is_err()
    );
    heard.extend(offline::render(&mut engine, bar() + bar() / 2, 2));
    let (mut c2, mut e2) = new_engine(RATE);
    let mut h2 = ScriptHost::new(RATE);
    h2.load(&mut c2, &good, "song.lua", &dir()).unwrap();
    c2.start().unwrap();
    assert_eq!(heard, offline::render(&mut e2, 2 * bar(), 2));
}

#[test]
fn a_change_with_step_effects_enters_at_the_bar() {
    let (mut controller, mut engine) = new_engine(RATE);
    let mut host = ScriptHost::new(RATE);
    let plain = format!("{HEAD}track(\"d\", b):steps(\"{STEPS}\")");
    let crushed =
        format!("{HEAD}track(\"d\", b):steps(\"1{{crush=3}} . 2 .  1 . 2 2  . 1 . 2  1 . . 2\")");
    host.load(&mut controller, &plain, "song.lua", &dir())
        .unwrap();
    controller.start().unwrap();
    let mut audio = offline::render(&mut engine, bar() / 4, 2);
    host.load(&mut controller, &crushed, "song.lua", &dir())
        .unwrap();
    audio.extend(offline::render(&mut engine, 2 * bar() - bar() / 4, 2));
    // El primer compás, como el de antes; el segundo, con el efecto.
    let old = song(&format!("track(\"d\", b):steps(\"{STEPS}\")")).unwrap();
    let new =
        song("track(\"d\", b):steps(\"1{crush=3} . 2 .  1 . 2 2  . 1 . 2  1 . . 2\")").unwrap();
    let old = old.render(RATE, 2).unwrap();
    let new = new.render(RATE, 2).unwrap();
    assert_eq!(audio[..2 * bar()], old[..2 * bar()]);
    assert_eq!(audio[2 * bar()..], new[2 * bar()..]);
}

#[test]
fn spans_follow_braces_with_spaces_inside() {
    let song = song("track(\"d\", b):steps(\"1{lp = 800, q=2} . 2 .  . . . .  . . . .  . . . .\")")
        .unwrap();
    let spans = &song.tracks[0].spans[0];
    let line = "track(\"d\", b):steps(\"1{lp = 800, q=2} . 2 .  . . . .  . . . .  . . . .\")";
    let start = line.find('1').unwrap();
    let first = spans[0].unwrap();
    assert_eq!((first.row, first.start, first.end), (2, start, start + 16));
    let third = spans[2].unwrap();
    assert_eq!(third.start, start + 19);
}

/// Las llaves en `melody()` (de un instrumento, de una pista de cortes o
/// dentro de `every`) dicen dónde valen, no que falte el script.
#[test]
fn braces_in_a_melody_say_where_they_belong() {
    let notes = "c{lp=800} . . .  . . . .  . . . .  . . . .";
    for source in [
        format!(
            "local i = sample(\"break.wav\"):instrument()\ntrack(\"m\", i):melody(\"{notes}\")"
        ),
        format!("track(\"d\", b):melody(\"{notes}\")"),
        format!(
            "track(\"d\", b):steps(\"{STEPS}\"):every(2, function(c) c:melody(\"{notes}\") end)"
        ),
    ] {
        let message = error(&source);
        assert!(
            message.contains("paso 1 (\"c{lp=800}\")")
                && message.contains(
                    "las llaves ({...}) solo valen en steps(\"...\") de una pista de cortes, \
                     no en melody() ni en un instrumento: usa los efectos de la pista (:fx)"
                )
                && !message.contains("desde el script"),
            "{source}\n→ {message}"
        );
    }
}
