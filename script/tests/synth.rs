//! `synth("acid", {...})`: suena igual que el motor con lo mismo, sus
//! parámetros admiten curvas, la semilla sale del nombre, accent y slide
//! en `steps`, `melody`, `every` y `chance`, y los errores dicen la línea.

use std::path::Path;

use engine::synth::{self, ACCENT, CUTOFF, ENV_AMOUNT, GLIDE, RESONANCE, acid};
use engine::{Curve, LfoRate, Pattern, Sample, Slices, new_engine, offline};
use script::{ScriptHost, Song};

const RATE: u32 = 48_000;
/// 120 BPM: 6000 frames por paso.
const STEP: usize = 6000;

fn song(source: &str) -> Result<Song, String> {
    let (mut controller, _engine) = new_engine(RATE);
    let mut host = ScriptHost::new(RATE);
    let source = format!("bpm(120)\n{source}");
    host.load(&mut controller, &source, "song.lua", Path::new("."))
        .cloned()
        .map_err(|e| e.to_string())
}

fn song_ok(source: &str) -> Song {
    song(source).unwrap_or_else(|e| panic!("{source}: {e}"))
}

/// El acid de un script y el mismo patch puesto a mano en el motor: los
/// mismos pasos (con accent y slide) y el mismo audio, muestra a muestra.
#[test]
fn the_script_sounds_like_the_engine() {
    let song = song_ok(
        "local bass = synth(\"acid\", {\n\
           cutoff = 500, res = 0.85, envmod = 0.7, decay = 200, accent = 0.9,\n\
           dist = 0.3, glide = 80, wave = \"square\", seed = 5,\n\
         })\n\
         track(\"bass\", bass):melody(\"c c! eb/ c  . c bb1 c!/  g1 . c c/  eb! f/ eb c\")",
    );
    let def = song.tracks[0].instrument.unwrap();
    assert_eq!(def.root, 36);
    let mut patch = acid::patch(36.0);
    patch.wave = synth::Wave::Square;
    patch.seed = 5;
    for (name, value) in [
        ("cutoff", 500.0),
        ("res", 0.85),
        ("envmod", 0.7),
        ("decay", 200.0),
        ("accent", 0.9),
        ("dist", 0.3),
        ("glide", 80.0),
    ] {
        acid::param(name).unwrap().set(&mut patch, value);
    }
    assert_eq!(def.synth.unwrap().patch, patch);
    let steps =
        engine::notation::parse_steps("1 1! 1^3/ 1  . 1 1^-2 1!/  1^-5 . 1 1/  1^3! 1^5/ 1^3 1")
            .unwrap();
    let silence = std::sync::Arc::new(Sample::silence(RATE));
    let slices = Slices::grid(&silence, 1);
    let pattern = Pattern::from_steps(silence, &slices, steps)
        .unwrap()
        .with_synth(Some(patch), [None; synth::PARAM_COUNT])
        .unwrap();
    let (mut controller, mut engine) = new_engine(RATE);
    controller.set_bpm(120.0).unwrap();
    controller.set_track_pattern(0, Some(pattern)).unwrap();
    controller.start().unwrap();
    let frames = 2 * engine::bar_frames(120.0, RATE) as usize;
    let direct = offline::render(&mut engine, frames, 2);
    let scripted = song.render(RATE, 2).unwrap();
    assert!(direct.iter().any(|v| v.abs() > 0.1), "no suena");
    assert!(scripted == direct, "el script no suena como el motor");
}

/// `ramp()` y `lfo()` en los parámetros continuos, en las unidades del
/// motor (`envmod` de 0 a 1 son de 0 a 4 octavas).
#[test]
fn continuous_params_take_curves() {
    let song = song_ok(
        "track(\"bass\", synth(\"acid\", {\n\
           cutoff = ramp(200, 3000, \"2 compases\"),\n\
           envmod = lfo(\"4 pasos\", 0, 1),\n\
           res = lfo(2, 0.2, 0.9),\n\
         })):steps(\"1 1 1 1  1 1 1 1  1 1 1 1  1 1 1 1\")",
    );
    let curves = song.tracks[0].instrument.unwrap().synth.unwrap().curves;
    assert_eq!(
        curves[CUTOFF],
        Some(Curve::Ramp {
            from: 200.0,
            to: 3000.0,
            steps: 32.0
        })
    );
    assert_eq!(
        curves[ENV_AMOUNT],
        Some(Curve::Lfo {
            min: 0.0,
            max: acid::ENVMOD_OCTAVES,
            rate: LfoRate::Steps(4.0)
        })
    );
    assert!(matches!(curves[RESONANCE], Some(Curve::Lfo { .. })));
    assert!(curves[GLIDE].is_none() && curves[ACCENT].is_none());
    let audio = song.render(RATE, 2).unwrap();
    assert!(audio.iter().all(|v| v.is_finite()));
}

/// Sin semilla, cada pista tiene la suya (de su nombre): dos pistas con el
/// mismo synth no suenan igual; con la misma semilla, sí. Y el mismo
/// script suena igual cada vez.
#[test]
fn the_seed_comes_from_the_track_name() {
    let two = |seeds: &str| {
        song_ok(&format!(
            "local a = synth(\"acid\", {{ {seeds} }})\n\
             track(\"uno\", a):steps(\"1 . . .  . . . .  . . . .  . . . .\")\n\
             track(\"dos\", a):steps(\"1 . . .  . . . .  . . . .  . . . .\")"
        ))
    };
    let solo = |song: &Song, name: &str| {
        let mut song = song.clone();
        song.tracks.retain(|t| t.name == name);
        song.render(RATE, 1).unwrap()
    };
    let named = two("");
    let seed = |s: &Song, i: usize| s.tracks[i].instrument.unwrap().synth.unwrap().patch.seed;
    assert_ne!(seed(&named, 0), seed(&named, 1));
    assert_ne!(solo(&named, "uno"), solo(&named, "dos"));
    let fixed = two("seed = 3");
    assert_eq!(seed(&fixed, 0), 3);
    assert_eq!(solo(&fixed, "uno"), solo(&fixed, "dos"));
    assert_eq!(solo(&two(""), "uno"), solo(&named, "uno"));
}

/// Accent y slide desde `every` (con `c:melody`) y desde `chance` (con
/// semilla, el mismo ciclo cada vez).
#[test]
fn accents_and_slides_from_every_and_chance() {
    let song = song_ok(
        "track(\"bass\", synth(\"acid\"))\n\
           :melody(\"c . c .  c . c .  c . c .  c . c .\")\n\
           :every(2, function(c) c:melody(\"c! . eb/ .  c . c .  c . c .  c . c .\") end)\n\
           :chance(0.5, \"accent\")",
    );
    let track = &song.tracks[0];
    let lap0 = &track.bars[0];
    assert!(lap0[0].unwrap().accent && lap0[2].unwrap().slide);
    let accents = track
        .bars
        .iter()
        .flatten()
        .flatten()
        .filter(|s| s.accent)
        .count();
    let hits = track.bars.iter().flatten().flatten().count();
    assert!(accents > hits / 4 && accents < hits, "{accents} de {hits}");
}

/// Una nota con slide hace que la anterior dure hasta ella aunque llegue
/// tarde (con swing): sin slide, la primera suelta al acabar su paso y
/// queda un hueco hasta la segunda; con slide, sigue sonando (en `steps` y
/// en `melody`).
#[test]
fn a_slid_note_keeps_the_previous_one_sounding() {
    let render = |text: &str| {
        song_ok(&format!(
            "track(\"bass\", synth(\"acid\", {{ envmod = 0, cutoff = 2000 }}))\n\
               :steps(\"{text}\"):swing(70)"
        ))
        .render(RATE, 1)
        .unwrap()
    };
    let rms = |a: &[f32]| (a.iter().map(|v| v * v).sum::<f32>() / a.len() as f32).sqrt();
    let left = |a: &[f32]| a.iter().step_by(2).copied().collect::<Vec<_>>();
    // Con swing de 70, el paso 2 llega 0,4 pasos tarde (2400 frames): el
    // release de la primera (12 ms) ya ha acabado a los 800.
    let gap = STEP + 800..STEP + 2000;
    let plain = left(&render("1 1^7 . .  . . . .  . . . .  . . . ."));
    let tied = left(&render("1 1^7/ . .  . . . .  . . . .  . . . ."));
    let (p, t) = (rms(&plain[gap.clone()]), rms(&tied[gap]));
    assert!(t > 20.0 * p, "con slide {t}, sin él {p}");
    let melody = song_ok(
        "track(\"bass\", synth(\"acid\", { envmod = 0, cutoff = 2000 }))\n\
           :melody(\"c g/ . .  . . . .  . . . .  . . . .\"):swing(70)",
    )
    .render(RATE, 1)
    .unwrap();
    assert_eq!(left(&melody), tied);
}

/// Sus notas salen por MIDI como las de un instrumento: raíz + semitonos.
#[test]
fn synth_notes_go_out_by_midi() {
    let (mut controller, mut engine) = new_engine(RATE);
    let mut host = ScriptHost::new(RATE);
    host.load(
        &mut controller,
        "bpm(120)\n\
         track(\"bass\", synth(\"acid\", { root = \"C2\" }))\n\
           :melody(\"c . eb .  . . . .  . . . .  . . . .\"):midi(3)",
        "song.lua",
        Path::new("."),
    )
    .unwrap();
    controller.start().unwrap();
    offline::render(&mut engine, 4 * STEP, 2);
    let got: Vec<(u64, u8, u8)> = controller
        .midi_events()
        .map(|n| (n.frame, n.note, n.velocity))
        .collect();
    let step = STEP as u64;
    assert_eq!(
        got,
        [
            (0, 36, 100),
            (step, 36, 0),
            (2 * step, 39, 100),
            (3 * step, 39, 0)
        ]
    );
}

/// Los errores dicen la línea (la 3: la 1 es `bpm`, la 2 un `local`).
#[test]
fn errors_say_the_line() {
    let acid = "synth(\"acid\")";
    let cases = [
        (
            "synth(\"moog\")",
            "no hay ese sintetizador (hay: acid, sub, subharm)",
        ),
        (
            "synth(\"acid\", { cutof = 300 })",
            "clave desconocida \"cutof\"",
        ),
        (
            "synth(\"acid\", { cutoff = 5 })",
            "cutoff = 5 fuera de rango (20 a 20000)",
        ),
        (
            "synth(\"acid\", { res = 2 })",
            "res = 2 fuera de rango (0 a 1)",
        ),
        (
            "synth(\"acid\", { wave = \"sine\" })",
            "wave es \"saw\" o \"square\"",
        ),
        ("synth(\"acid\", { root = \"H2\" })", "no es una nota"),
        ("synth(\"acid\", { seed = -1 })", "seed es un entero"),
        (
            "synth(\"acid\", { wave = ramp(0, 1, \"1 compás\") })",
            "wave no admite ramp() ni lfo()",
        ),
        (
            "synth(\"acid\", { res = ramp(0, 3, \"1 compás\") })",
            "la curva se sale del rango (0 a 1)",
        ),
        (
            "track(\"b\", synth(\"acid\")):steps(\"1^0^4 . . .  . . . .  . . . .  . . . .\")",
            "es un sintetizador monofónico",
        ),
        (
            "track(\"b\", synth(\"acid\")):steps(\"1{lp=800} . . .  . . . .  . . . .  . . . .\")",
            "es un sintetizador: sus notas no tienen efectos por paso",
        ),
        (
            "track(\"b\", synth(\"acid\")):melody(\"c+e . . .  . . . .  . . . .  . . . .\")",
            "es un sintetizador monofónico",
        ),
        (
            "synth(\"sub\", { wave = \"saw\" })",
            "wave es \"sine\" o \"triangle\"",
        ),
        (
            "synth(\"sub\", { tone = 10 })",
            "tone = 10 fuera de rango (40 a 2000)",
        ),
        (
            "synth(\"sub\", { attack = ramp(1, 50, \"1 compás\") })",
            "attack no admite ramp() ni lfo(): la envolvente se fija",
        ),
        ("synth(\"sub\", { seed = 1 })", "clave desconocida \"seed\""),
        (
            "synth(\"acid\"):fx(\"lowpass\", {})",
            "los efectos de un sintetizador van en la pista",
        ),
    ];
    for (line, expected) in cases {
        let Err(err) = song(&format!("local x = {acid}\n{line}")) else {
            panic!("{line}: debería fallar");
        };
        assert!(err.contains("song.lua:3:"), "{line}: {err}");
        assert!(err.contains(expected), "{line}: {err}");
    }
    // Accent y slide en una pista que no es un sintetizador: con la línea.
    let dir = std::env::temp_dir();
    let (mut controller, _engine) = new_engine(RATE);
    let mut host = ScriptHost::new(RATE);
    let wav = dir.join("synth-errors.wav");
    offline::write_wav(&wav, &vec![0.25f32; 4800], 1, RATE).unwrap();
    for (line, expected) in [
        (
            "track(\"d\", s):steps(\"1! . . .  . . . .  . . . .  . . . .\")",
            "! (accent) es de los sintetizadores",
        ),
        (
            "track(\"d\", s):steps(\"1 . 1/ .  . . . .  . . . .  . . . .\")",
            "/ (slide) es de los sintetizadores",
        ),
        (
            "track(\"d\", s):chance(0.5, \"slide\")",
            "\"slide\" es de los sintetizadores",
        ),
    ] {
        let source = format!("bpm(120)\nlocal s = sample(\"synth-errors.wav\"):slice(1)\n{line}");
        let err = host
            .load(&mut controller, &source, "song.lua", &dir)
            .map(|_| ())
            .unwrap_err()
            .to_string();
        assert!(err.contains("song.lua:3:"), "{line}: {err}");
        assert!(err.contains(expected), "{line}: {err}");
    }
}

/// Un script con un error en el synth no cambia lo que suena: el modelo
/// sigue siendo el anterior.
#[test]
fn an_invalid_synth_keeps_the_previous_song() {
    let (mut controller, _engine) = new_engine(RATE);
    let mut host = ScriptHost::new(RATE);
    let good = "bpm(120)\ntrack(\"b\", synth(\"acid\", { cutoff = 300 })):melody(\"c . . .  . . . .  . . . .  . . . .\")";
    host.load(&mut controller, good, "song.lua", Path::new("."))
        .unwrap();
    let bad = "bpm(120)\ntrack(\"b\", synth(\"acid\", { cutoff = 3 })):melody(\"c . . .  . . . .  . . . .  . . . .\")";
    assert!(
        host.load(&mut controller, bad, "song.lua", Path::new("."))
            .is_err()
    );
    let kept = host.song().unwrap().tracks[0]
        .instrument
        .unwrap()
        .synth
        .unwrap();
    assert_eq!(kept.patch.params[CUTOFF], 300.0);
}

/// El sub de un script (con su alias) y el mismo patch a mano en el
/// motor: los mismos pasos y el mismo audio; su raíz por defecto es C1.
#[test]
fn the_sub_sounds_like_the_engine() {
    let song = song_ok(
        "local sub = synth(\"subbass\", {\n\
           tone = 180, drive = 0.2, glide = 60, attack = 8, release = 150, wave = \"triangle\",\n\
         })\n\
         track(\"sub\", sub):melody(\"c _ _ c!  . . g0/ _  c _ eb .  f/ _ _ _\")",
    );
    let def = song.tracks[0].instrument.unwrap();
    assert_eq!(def.root, 24);
    let synth_def = def.synth.unwrap();
    assert_eq!(synth_def.kind, "sub");
    let mut patch = synth::sub::patch(24.0);
    patch.wave = synth::Wave::Triangle;
    for (name, value) in [
        ("tone", 180.0),
        ("drive", 0.2),
        ("glide", 60.0),
        ("attack", 8.0),
        ("release", 150.0),
    ] {
        synth::sub::param(name).unwrap().set(&mut patch, value);
    }
    patch.seed = synth_def.patch.seed;
    assert_eq!(synth_def.patch, patch);
    let steps =
        engine::notation::parse_steps("1 _ _ 1!  . . 1^-5/ _  1 _ 1^3 .  1^5/ _ _ _").unwrap();
    let silence = std::sync::Arc::new(Sample::silence(RATE));
    let slices = Slices::grid(&silence, 1);
    let pattern = Pattern::from_steps(silence, &slices, steps)
        .unwrap()
        .with_synth(Some(patch), [None; synth::PARAM_COUNT])
        .unwrap();
    let (mut controller, mut engine) = new_engine(RATE);
    controller.set_bpm(120.0).unwrap();
    controller.set_track_pattern(0, Some(pattern)).unwrap();
    controller.start().unwrap();
    let frames = 2 * engine::bar_frames(120.0, RATE) as usize;
    let direct = offline::render(&mut engine, frames, 2);
    let scripted = song.render(RATE, 2).unwrap();
    assert!(direct.iter().any(|v| v.abs() > 0.1), "no suena");
    assert!(scripted == direct, "el script no suena como el motor");
}

/// El bloque sub: en la tabla de cualquier synth (con curvas en lo
/// continuo) y por paso entre llaves, que manda sobre la tabla en su nota.
#[test]
fn the_sub_block_in_the_table_and_per_step() {
    let song = song_ok(
        "local bass = synth(\"acid\", {\n\
           sub = lfo(\"4 pasos\", 0, 0.8), sub_div = 3, sub_wave = \"tri\",\n\
           sub_pan = -0.4, sub_drive = 0.2,\n\
         })\n\
         track(\"bass\", bass)\n\
           :melody(\"c c{sub=0} eb{sub=0.9,sub_div=2}{sub_wave=sine} .  . . . .  . . . .  . . . .\")",
    );
    let def = song.tracks[0].instrument.unwrap().synth.unwrap();
    assert_eq!(def.patch.sub_div, 3);
    assert_eq!(def.patch.sub_wave, synth::Wave::Triangle);
    assert_eq!(def.patch.params[synth::SUB_PAN], -0.4);
    assert!(def.curves[synth::SUB].is_some());
    let bar = &song.tracks[0].bars[0];
    assert_eq!(bar[0].unwrap().sub, None);
    assert_eq!(bar[1].unwrap().sub.unwrap().level, Some(0.0));
    let third = bar[2].unwrap().sub.unwrap();
    assert_eq!(
        (third.level, third.div, third.wave),
        (Some(0.9), Some(2), Some(synth::Wave::Sine))
    );
    // Con el sub, suena distinto que sin él.
    let plain =
        song_ok("track(\"bass\", synth(\"acid\")):melody(\"c c eb .  . . . .  . . . .  . . . .\")");
    assert_ne!(
        song.render(RATE, 1).unwrap(),
        plain.render(RATE, 1).unwrap()
    );
}

/// Lo que no vale del bloque sub dice la línea.
#[test]
fn sub_block_errors_say_the_line() {
    let cases = [
        (
            "synth(\"acid\", { sub = 2 })",
            "sub = 2 fuera de rango (0 a 1)",
        ),
        ("synth(\"acid\", { sub_div = 2.5 })", "sub_div es un entero"),
        (
            "synth(\"acid\", { sub_div = 17 })",
            "fuera de rango (1 a 16)",
        ),
        (
            "synth(\"acid\", { sub_wave = \"saw\" })",
            "sub_wave es \"square\", \"tri\" o \"sine\"",
        ),
        (
            "synth(\"acid\", { sub_div = ramp(1, 4, \"1 compás\") })",
            "sub_div no admite ramp() ni lfo()",
        ),
        (
            "track(\"b\", synth(\"acid\")):steps(\"1{sub_div=2.5} . . .  . . . .  . . . .  . . . .\")",
            "sub_div es un entero de 1 a 16",
        ),
        (
            "track(\"b\", synth(\"acid\")):steps(\"1{sub=0.5,lp=800} . . .  . . . .  . . . .  . . . .\")",
            "el sub de la nota va en sus propias llaves",
        ),
        (
            "track(\"b\", synth(\"acid\")):steps(\"1{sub=0>1} . . .  . . . .  . . . .  . . . .\")",
            "sub no admite rampas",
        ),
        (
            "track(\"b\", synth(\"acid\")):steps(\"1{sub_wave=saw} . . .  . . . .  . . . .  . . . .\")",
            "sub_wave es square, tri o sine",
        ),
    ];
    for (line, expected) in cases {
        let Err(err) = song(&format!("local x = 1\n{line}")) else {
            panic!("{line}: debería fallar");
        };
        assert!(err.contains("song.lua:3:"), "{line}: {err}");
        assert!(err.contains(expected), "{line}: {err}");
    }
    // En una pista de cortes, el sub de un paso no vale.
    let dir = std::env::temp_dir();
    let wav = dir.join("sub-block-errors.wav");
    offline::write_wav(&wav, &vec![0.25f32; 4800], 1, RATE).unwrap();
    let (mut controller, _engine) = new_engine(RATE);
    let mut host = ScriptHost::new(RATE);
    let source = "bpm(120)\nlocal s = sample(\"sub-block-errors.wav\"):slice(1)\n\
                  track(\"d\", s):steps(\"1{sub=0.5} . . .  . . . .  . . . .  . . . .\")";
    let err = host
        .load(&mut controller, source, "song.lua", &dir)
        .map(|_| ())
        .unwrap_err()
        .to_string();
    assert!(
        err.contains("song.lua:3:") && err.contains("{sub=...} es de los sintetizadores"),
        "{err}"
    );
}

/// `subharm{...}` es `synth("subharm", {...})`, sus capas llegan al patch
/// y suena igual que el motor con lo mismo.
#[test]
fn the_subharm_sounds_like_the_engine() {
    let source = "{\n\
                    o1 = { div = 1, rdiv = 4, pan = -0.6 },\n\
                    o2 = { div = 2, level = 0.5, pan = 0.4, rdiv = 3 },\n\
                    s1 = { div = 3, pan = 0.5, level = 0.7, rdiv = 7 },\n\
                    s4 = { div = 5, level = 0.3 },\n\
                    cutoff = 900, res = 0.4, seed = 2, attack = 5, decay = 250,\n\
                  }";
    let braced = song_ok(&format!(
        "track(\"sh\", subharm{source}):melody(\"c _ _ _  _ _ _ _  eb _ _ g/  _ _ _ _\")"
    ));
    let called = song_ok(&format!(
        "track(\"sh\", synth(\"subharm\", {source})):melody(\"c _ _ _  _ _ _ _  eb _ _ g/  _ _ _ _\")"
    ));
    let def = braced.tracks[0].instrument.unwrap();
    assert_eq!(def.root, 36);
    let synth_def = def.synth.unwrap();
    assert_eq!(synth_def.kind, "subharm");
    assert_eq!(
        synth_def.patch,
        called.tracks[0].instrument.unwrap().synth.unwrap().patch
    );
    let mut patch = synth::subharm::patch(36.0);
    let layers = patch.layers.as_mut().unwrap();
    layers[0].pan = -0.6;
    layers[0].rdiv = 4;
    layers[1] = synth::Layer {
        div: 2,
        rdiv: 3,
        pan: 0.4,
        level: 0.5,
    };
    layers[2] = synth::Layer {
        div: 3,
        rdiv: 7,
        pan: 0.5,
        level: 0.7,
    };
    layers[5].level = 0.3;
    layers[5].div = 5;
    synth::subharm::param("cutoff")
        .unwrap()
        .set(&mut patch, 900.0);
    synth::subharm::param("res").unwrap().set(&mut patch, 0.4);
    synth::subharm::param("attack")
        .unwrap()
        .set(&mut patch, 5.0);
    synth::subharm::param("decay")
        .unwrap()
        .set(&mut patch, 250.0);
    patch.seed = 2;
    assert_eq!(synth_def.patch, patch);
    let steps = engine::notation::parse_steps("1 _ _ _  _ _ _ _  1^3 _ _ 1^7/  _ _ _ _").unwrap();
    let silence = std::sync::Arc::new(Sample::silence(RATE));
    let slices = Slices::grid(&silence, 1);
    let pattern = Pattern::from_steps(silence, &slices, steps)
        .unwrap()
        .with_synth(Some(patch), [None; synth::PARAM_COUNT])
        .unwrap();
    let (mut controller, mut engine) = new_engine(RATE);
    controller.set_bpm(120.0).unwrap();
    controller.set_track_pattern(0, Some(pattern)).unwrap();
    controller.start().unwrap();
    let frames = 2 * engine::bar_frames(120.0, RATE) as usize;
    let direct = offline::render(&mut engine, frames, 2);
    let scripted = braced.render(RATE, 2).unwrap();
    assert!(direct.iter().any(|v| v.abs() > 0.05), "no suena");
    assert!(scripted == direct, "el script no suena como el motor");
}

/// Lo que no vale de `subharm` dice la línea.
#[test]
fn subharm_errors_say_the_line() {
    let cases = [
        ("subharm{ o3 = { div = 2 } }", "clave desconocida \"o3\""),
        ("subharm{ o1 = 2 }", "o1 es una tabla"),
        (
            "subharm{ s1 = { div = 2.5 } }",
            "s1.div es un entero de 1 a 16",
        ),
        (
            "subharm{ s2 = { div = 17 } }",
            "s2.div es un entero de 1 a 16",
        ),
        ("subharm{ o2 = { pan = 2 } }", "o2.pan va de -1 a 1"),
        (
            "subharm{ o1 = { rdiv = 0 } }",
            "o1.rdiv es un entero de 1 a 16",
        ),
        (
            "subharm{ s1 = { rdiv = 3.5 } }",
            "s1.rdiv es un entero de 1 a 16",
        ),
        (
            "subharm{ decay = 5 }",
            "decay = 5 fuera de rango (10 a 10000)",
        ),
        ("subharm{ s3 = { level = -1 } }", "s3.level va de 0 a 1"),
        (
            "subharm{ s4 = { tono = 1 } }",
            "s4: clave desconocida \"tono\"",
        ),
        (
            "subharm{ o1 = { level = lfo(1, 0, 1) } }",
            "o1.level no admite ramp() ni lfo()",
        ),
        ("subharm{ voices = 2.5 }", "voices es un entero (1 a 8)"),
        ("subharm{ sub = 0.5 }", "clave desconocida \"sub\""),
        (
            "track(\"s\", subharm{}):steps(\"1{sub=0.5} . . .  . . . .  . . . .  . . . .\")",
            "es un subharm: no tiene bloque sub",
        ),
        (
            "track(\"s\", subharm{}):steps(\"1^0^7 . . .  . . . .  . . . .  . . . .\")",
            "es un sintetizador monofónico",
        ),
    ];
    for (line, expected) in cases {
        let Err(err) = song(&format!("local x = 1\n{line}")) else {
            panic!("{line}: debería fallar");
        };
        assert!(err.contains("song.lua:3:"), "{line}: {err}");
        assert!(err.contains(expected), "{line}: {err}");
    }
    // Con varias voces, acordes sí.
    song_ok(
        "track(\"s\", subharm{ voices = 3 }):steps(\"1^0^7 . . .  . . . .  . . . .  . . . .\")",
    );
}
