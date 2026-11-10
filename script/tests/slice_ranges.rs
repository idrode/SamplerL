//! Cortes independientes en el script: `sample:slice({ {inicio, fin}, ...
//! })`, cada corte con su inicio y su final (huecos y solapes), sus errores,
//! y dónde está escrita la llamada que corta (para reescribirla desde la
//! onda).

use std::path::{Path, PathBuf};

use engine::offline;
use script::{ScriptHost, Song};

const RATE: u32 = 48_000;

fn fixture(test: &str) -> PathBuf {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!("slice_ranges_{test}"));
    std::fs::create_dir_all(&dir).unwrap();
    // 0,5 s: cuatro golpes, cada 125 ms (6000 frames).
    let mut four = vec![0.0f32; 24_000];
    for k in 0..4 {
        four[k * 6000..k * 6000 + 600].fill(0.2 + 0.1 * k as f32);
    }
    offline::write_wav(dir.join("four.wav"), &four, 1, RATE).unwrap();
    dir
}

fn eval(dir: &Path, source: &str) -> Result<Song, String> {
    ScriptHost::new(RATE)
        .eval(source, "song.lua", dir)
        .map_err(|e| e.to_string())
}

fn slices(song: &Song, track: &str) -> Vec<std::ops::Range<usize>> {
    song.tracks
        .iter()
        .find(|t| t.name == track)
        .unwrap()
        .slices
        .iter()
        .collect()
}

#[test]
fn each_slice_has_its_own_start_and_end() {
    let dir = fixture("own");
    let song = eval(
        &dir,
        "local s = sample(\"four.wav\"):slice({ {0, 100}, {150, 400}, {300, 500, \"top\"} })\n\
         track(\"a\", s):steps(\"1 2 top .  . . . .  . . . .  . . . .\")\n",
    )
    .unwrap();
    // Un hueco entre el 1 y el 2, y el 3 se solapa con el 2.
    assert_eq!(
        slices(&song, "a"),
        vec![0..4800, 7200..19_200, 14_400..24_000]
    );
    let def = &song.tracks[0];
    assert_eq!(def.labels, vec![None, None, Some("top".into())]);
    assert!(def.origin.explicit);
    let call = def.origin.call.as_ref().unwrap();
    assert_eq!(call.ranges, slices(&song, "a"));
    assert_eq!(call.span.start, (0, 29));
}

#[test]
fn contiguous_ranges_sound_like_the_detector() {
    let dir = fixture("same");
    let steps = "track(\"a\", s):steps(\"1 2 3 4  4 3 2 1  1 . 3 .  . 2 . 4\")\n";
    let detected = eval(
        &dir,
        &format!("local s = sample(\"four.wav\"):slice(4)\n{steps}"),
    )
    .unwrap();
    let ranges = slices(&detected, "a");
    let ms = |f: usize| f as f64 * 1000.0 / RATE as f64;
    let table: Vec<String> = ranges
        .iter()
        .map(|r| format!("{{{}, {}}}", ms(r.start), ms(r.end)))
        .collect();
    let explicit = eval(
        &dir,
        &format!(
            "local s = sample(\"four.wav\"):slice({{ {} }})\n{steps}",
            table.join(", ")
        ),
    )
    .unwrap();
    assert_eq!(slices(&explicit, "a"), ranges);
    assert_eq!(
        explicit.render(RATE, 2).unwrap(),
        detected.render(RATE, 2).unwrap()
    );
}

#[test]
fn invalid_ranges_say_which_slice_and_why() {
    let dir = fixture("errors");
    let cases = [
        (
            "{ {200, 100} }",
            "el corte 1 acaba (100 ms) antes de empezar (200 ms)",
        ),
        (
            "{ {0, 100}, {100, 100} }",
            "el corte 2 acaba (100 ms) antes de empezar",
        ),
        (
            "{ {0, 900} }",
            "el corte 1 acaba en 900 ms, pasado el final de four.wav (500.0 ms)",
        ),
        ("{ {-5, 100} }", "el corte 1 empieza antes del principio"),
        ("{ {0, 0.5} }", "el corte 1 dura menos de 1 ms"),
        ("{}", "hace falta al menos un corte"),
        ("{ {0} }", "el corte 1 no es {inicio, fin} con dos números"),
        ("{ 100 }", "el corte 1 no es una tabla"),
        (
            "{ {0, 10, 3} }",
            "el corte 1 tiene un nombre que no es texto",
        ),
        (
            "{ {0, 10, \"a\"}, {10, 20, \"a\"} }",
            "el corte 2 se llama «a», como el corte 1",
        ),
        ("{ a = {0, 10} }", "solo una lista de cortes"),
    ];
    for (table, expected) in cases {
        let error = eval(
            &dir,
            &format!("local s = sample(\"four.wav\"):slice({table})\ntrack(\"a\", s)\n"),
        )
        .err()
        .unwrap();
        assert!(error.contains("song.lua:1:"), "{table}: {error}");
        assert!(error.contains(expected), "{table}: {error}");
    }
    let error = eval(&dir, "sample(\"four.wav\"):slice(\"8\")")
        .err()
        .unwrap();
    assert!(error.contains("una tabla de cortes"), "{error}");
}

#[test]
fn a_failing_table_keeps_what_was_playing() {
    let dir = fixture("keeps");
    let (mut controller, _engine) = engine::new_engine(RATE);
    let mut host = ScriptHost::new(RATE);
    let good = "local s = sample(\"four.wav\"):slice({ {0, 100}, {200, 300} })\n\
                track(\"a\", s):steps(\"1 . 2 .  . . . .  . . . .  . . . .\")\n";
    host.load(&mut controller, good, "song.lua", &dir).unwrap();
    let bad = good.replace("{200, 300}", "{300, 200}");
    let error = host
        .load(&mut controller, &bad, "song.lua", &dir)
        .err()
        .unwrap();
    assert!(error.message.contains("el corte 2 acaba"), "{error:?}");
    // La canción buena sigue siendo la del host.
    assert_eq!(
        host.song().unwrap().tracks[0].slices.get(1),
        Some(9600..14_400)
    );
}

#[test]
fn the_call_is_found_where_it_is_written() {
    let dir = fixture("call");
    let source = "bpm(120)\n\
                  local a = sample(\"four.wav\")\n  :slice(4)\n\
                  local b = sample(\"four.wav\")\n  :slice(4, { zero_cross = 1 })\n\
                  local c = sample(\"four.wav\"):slice_grid(2) -- :slice(4)\n\
                  local n = 3\n\
                  local d = sample(\"four.wav\"):slice(n)\n\
                  local e = sample(\"four.wav\"):slice{ {0, 10},\n  {10, 20} }\n\
                  track(\"a\", a) track(\"b\", b) track(\"c\", c) track(\"d\", d) track(\"e\", e)\n";
    let song = eval(&dir, source).unwrap();
    let call = |name: &str| {
        song.tracks
            .iter()
            .find(|t| t.name == name)
            .unwrap()
            .origin
            .call
            .clone()
    };
    let a = call("a").unwrap();
    assert_eq!((a.span.start, a.span.end), ((2, 3), (2, 11)));
    assert_eq!(a.rest, "");
    let b = call("b").unwrap();
    assert_eq!((b.span.start, b.span.end), ((4, 3), (4, 31)));
    assert_eq!(b.rest, ", { zero_cross = 1 }");
    let c = call("c").unwrap();
    assert_eq!(c.span.start, (5, 29));
    let d = call("d").unwrap_err();
    assert!(d.contains("no está escrito tal cual"), "{d}");
    let e = call("e").unwrap();
    assert_eq!((e.span.start, e.span.end), ((8, 29), (9, 12)));
    // Dos iguales en la misma línea: no se sabe cuál.
    let song = eval(
        &dir,
        "local a, b = sample(\"four.wav\"):slice(4), sample(\"four.wav\"):slice(4)\n\
         track(\"a\", a)\n",
    )
    .unwrap();
    let why = song.tracks[0].origin.call.clone().unwrap_err();
    assert!(why.contains("dos llamadas iguales"), "{why}");
}

#[test]
fn the_rewritten_call_reads_back_the_same_slices() {
    let dir = fixture("rewrite");
    let source = "local s = sample(\"four.wav\"):slice(4)\ntrack(\"a\", s)\n";
    let song = eval(&dir, source).unwrap();
    let call = song.tracks[0].origin.call.clone().unwrap();
    let mut ranges = call.ranges.clone();
    ranges[0].end -= 1000;
    ranges[2].start += 333;
    let labels = vec![None, Some("snare".to_string()), None, None];
    let text = call.rewritten(&ranges, &labels, "");
    let line = source.lines().next().unwrap();
    let chars: Vec<char> = line.chars().collect();
    let new_line: String = chars[..call.span.start.1].iter().collect::<String>()
        + &text
        + &chars[call.span.end.1..].iter().collect::<String>();
    let again = eval(&dir, &format!("{new_line}\ntrack(\"a\", s)\n")).unwrap();
    assert_eq!(slices(&again, "a"), ranges);
    assert_eq!(again.tracks[0].labels, labels);
    let call = again.tracks[0].origin.call.clone().unwrap();
    assert_eq!(call.ranges, ranges);
}

#[test]
fn stretched_explicit_slices_keep_their_own_ends() {
    let dir = fixture("stretch");
    // 0,5 s a 120 BPM estirado a 150 (×0,8): cada límite, en su sitio.
    let song = eval(
        &dir,
        "bpm(150)\n\
         local s = sample(\"four.wav\"):stretch(120):slice({ {0, 100}, {150, 400}, {300, 500} })\n\
         track(\"a\", s):steps(\"1 2 3 .  . . . .  . . . .  . . . .\")\n",
    )
    .unwrap();
    let scaled = |ms: f64| (ms * 48.0 * 0.8).round() as usize;
    assert_eq!(
        slices(&song, "a"),
        vec![
            0..scaled(100.0),
            scaled(150.0)..scaled(400.0),
            scaled(300.0)..scaled(500.0)
        ]
    );
    // La llamada guarda los del sample sin estirar (para reescribirla).
    let call = song.tracks[0].origin.call.clone().unwrap();
    assert_eq!(call.ranges, vec![0..4800, 7200..19_200, 14_400..24_000]);
}

#[test]
fn move_with_an_end_or_on_a_table_leaves_the_neighbours() {
    let dir = fixture("move");
    let steps = "track(\"a\", s):steps(\"1 2 3 4  . . . .  . . . .  . . . .\")\n";
    // Del detector, con final: solo ese corte.
    let song = eval(
        &dir,
        &format!("local s = sample(\"four.wav\"):slice(4)\ns:move(2, 130, 200)\n{steps}"),
    )
    .unwrap();
    let got = slices(&song, "a");
    assert_eq!(got[1], 6240..9600);
    assert_eq!(got[0], 0..6000, "el anterior no se mueve");
    assert_eq!(got[2].start, 12_000);
    // De una tabla, sin final: solo su inicio.
    let song = eval(
        &dir,
        &format!(
            "local s = sample(\"four.wav\"):slice({{ {{0, 125}}, {{125, 250}}, {{250, 375}}, {{375, 500}} }})\n\
             s:move(2, 140)\n\
             local a, b = s:range(2)\n\
             assert(a == 140 and b == 250, a .. \" \" .. b)\n{steps}"
        ),
    )
    .unwrap();
    assert_eq!(slices(&song, "a")[..2], [0..6000, 6720..12_000]);
    for (code, expected) in [
        ("s:move(2, 140, 140.5)", "al menos 1 ms"),
        ("s:move(2, 300, 200)", "al menos 1 ms"),
        ("s:move(2, 100, 900)", "pasado el final del sample"),
        ("s:move(9, 100)", "el corte no existe (hay 4)"),
        ("s:range(0)", "range(0): el corte no existe"),
    ] {
        let error = eval(
            &dir,
            &format!("local s = sample(\"four.wav\"):slice(4)\n{code}\n"),
        )
        .err()
        .unwrap_or_else(|| panic!("{code}: sin error"));
        assert!(
            error.contains("song.lua:2:") && error.contains(expected),
            "{code}: {error}"
        );
    }
}

#[test]
fn everything_about_a_slice_uses_its_own_bounds() {
    let dir = fixture("features");
    let table = "{ {0, 125}, {150, 400}, {300, 500, \"top\"} }";
    let source = format!(
        "bpm(120)\n\
         local s = sample(\"four.wav\"):slice({table})\n\
         s:set(2, {{ dur = \"1/16\", fit = \"loop\", rev = true }})\n\
         s:fx(\"top\", \"lowpass\", {{ cutoff = 2000 }})\n\
         assert(s:note(1) == nil or type(s:note(1)) == \"string\")\n\
         local a, b = s:range(3)\n\
         assert(a == 300 and b == 500)\n\
         track(\"a\", s):steps(\"1 2 top .  2 . 1 .  . . . .  . . . .\")\n\
         track(\"b\", s:slice_at(2)):steps(\"x . . .  . . . .  . . . .  . . . .\")\n\
         track(\"c\", s:slice_at(3):instrument({{ root = \"C4\" }})):melody(\"c4 . e4 .  . . . .  . . . .  . . . .\")\n"
    );
    let song = eval(&dir, &source).unwrap();
    assert_eq!(slices(&song, "b"), vec![7200..19_200]);
    assert_eq!(slices(&song, "c"), vec![14_400..24_000]);
    let audio = song.render(RATE, 2).unwrap();
    assert!(audio.iter().all(|s| s.is_finite()));
    assert!(audio.iter().any(|&s| s != 0.0));
}
