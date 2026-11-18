//! Análisis de choques de frecuencia de una canción (`Song::analyze_masking`)
//! con pistas sintéticas: dos senos a 300 Hz chocan en esa banda; uno a
//! 100 Hz y otro a 5 kHz, no; se analiza el ciclo entero de la canción; y
//! las sugerencias de recorte van a la pista que cede, con `priority()`.

use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use engine::masking::{BAND_NAMES, MaskingOptions};
use engine::{new_engine, offline};
use script::{ScriptHost, Song};

const RATE: u32 = 48_000;

/// Carpeta con senos de 2 s (un compás a 120 BPM) a 100, 300 y 5000 Hz.
fn dir() -> PathBuf {
    static DIR: OnceLock<PathBuf> = OnceLock::new();
    DIR.get_or_init(|| {
        let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join("masking");
        std::fs::create_dir_all(&dir).unwrap();
        for freq in [100, 300, 5000] {
            let mono: Vec<f32> = (0..2 * RATE as usize)
                .map(|i| 0.3 * (std::f32::consts::TAU * freq as f32 * i as f32 / RATE as f32).sin())
                .collect();
            offline::write_wav(dir.join(format!("sine{freq}.wav")), &mono, 1, RATE).unwrap();
        }
        // Cinco senos en zonas separadas (de grave a agudo).
        let mono: Vec<f32> = (0..2 * RATE as usize)
            .map(|i| {
                let t = i as f32 / RATE as f32;
                [63.0, 250.0, 1000.0, 4000.0, 12500.0]
                    .iter()
                    .map(|f| 0.1 * (std::f32::consts::TAU * f * t).sin())
                    .sum::<f32>()
            })
            .collect();
        offline::write_wav(dir.join("multi.wav"), &mono, 1, RATE).unwrap();
        // Un bombo: un golpe de 150 ms que baja de 120 a 50 Hz y se apaga.
        let kick: Vec<f32> = (0..RATE as usize / 4)
            .map(|i| {
                let t = i as f32 / RATE as f32;
                let phase =
                    std::f32::consts::TAU * (50.0 * t + 70.0 * 0.03 * (1.0 - (-t / 0.03).exp()));
                0.8 * (-t / 0.15).exp() * phase.sin()
            })
            .collect();
        offline::write_wav(dir.join("kick.wav"), &kick, 1, RATE).unwrap();
        // Un bajo: 80 Hz sostenido, con un poco de su segundo armónico.
        let bass: Vec<f32> = (0..RATE as usize / 2)
            .map(|i| {
                let t = i as f32 / RATE as f32;
                0.4 * (std::f32::consts::TAU * 80.0 * t).sin()
                    + 0.1 * (std::f32::consts::TAU * 160.0 * t).sin()
            })
            .collect();
        offline::write_wav(dir.join("bass.wav"), &bass, 1, RATE).unwrap();
        dir
    })
    .clone()
}

fn song(source: &str) -> Song {
    let (mut controller, _engine) = new_engine(RATE);
    let mut host = ScriptHost::new(RATE);
    host.load(&mut controller, source, "song.lua", &dir())
        .unwrap()
        .clone()
}

const ONE: &str = "1 . . .  . . . .  . . . .  . . . .";

fn two(a: &str, b: &str) -> String {
    format!(
        "bpm(120)\n\
         track(\"a\", sample(\"{a}\"):slice_grid(1)):steps(\"{ONE}\")\n\
         track(\"b\", sample(\"{b}\"):slice_grid(1)):steps(\"{ONE}\"):gain(0.8)\n"
    )
}

fn band(name: &str) -> usize {
    BAND_NAMES.iter().position(|b| *b == name).unwrap()
}

#[test]
fn two_sines_at_300_hz_clash_in_that_band() {
    let report = song(&two("sine300.wav", "sine300.wav"))
        .analyze_masking(RATE, &MaskingOptions::default())
        .unwrap();
    assert_eq!(report.tracks, ["a", "b"]);
    assert_eq!(report.bars, 1);
    let score = report.score("a", "b", band("315")).unwrap();
    assert!(score > 0.7, "{score}");
    let clashes = report.clashes();
    assert!(!clashes.is_empty());
    assert!(
        clashes
            .iter()
            .all(|c| ["250", "315", "400"].contains(&BAND_NAMES[c.band])),
        "{}",
        report.text()
    );
    let text = report.text();
    assert!(text.contains("a × b") && text.contains("315 Hz"), "{text}");
    assert!(text.contains("a 2 dB más fuerte"), "{text}");
}

#[test]
fn a_sine_at_100_hz_and_another_at_5_khz_do_not() {
    let report = song(&two("sine100.wav", "sine5000.wav"))
        .analyze_masking(RATE, &MaskingOptions::default())
        .unwrap();
    assert!(report.clashes().is_empty(), "{}", report.text());
    assert!(report.text().contains("sin choques"));
    for b in ["100", "5k"] {
        assert!(report.score("a", "b", band(b)).is_none_or(|s| s < 0.01));
    }
}

#[test]
fn the_whole_cycle_is_analysed() {
    // b solo suena uno de cada tres compases (`every(3)`): el ciclo es de 3
    // compases y chocan en uno. Con la energía como peso, ese compás pesa
    // el doble que cada uno de los otros dos: 2 / (2 + 1 + 1) = 0,5. Con un
    // solo compás analizado saldría 0 o 1.
    let source = format!(
        "bpm(120)\n\
         track(\"a\", sample(\"sine300.wav\"):slice_grid(1)):steps(\"{ONE}\")\n\
         track(\"b\", sample(\"sine300.wav\"):slice_grid(1))\n  \
         :every(3, function(c) c:steps(\"{ONE}\") end)\n"
    );
    let s = song(&source);
    assert_eq!(s.length_bars(1), 3);
    let report = s.analyze_masking(RATE, &MaskingOptions::default()).unwrap();
    assert_eq!(report.bars, 3);
    let score = report.score("a", "b", band("315")).unwrap();
    assert!((score - 0.5).abs() < 0.08, "{score}");
}

#[test]
fn the_quieter_track_gets_the_cut_unless_priority_says_otherwise() {
    // b suena 2 dB más bajo: sin priority(), cede b.
    let source = two("sine300.wav", "sine300.wav");
    let report = song(&source)
        .analyze_masking(RATE, &MaskingOptions::default())
        .unwrap();
    assert!(report.priority.is_empty());
    assert_eq!(report.suggestions.len(), 1, "{}", report.text());
    let s = &report.suggestions[0];
    assert_eq!(report.tracks[s.track], "b");
    assert!(report.suggestion("a").is_none());
    let text = report.text();
    assert!(text.contains("cede b"), "{text}");
    assert!(
        text.contains("track(\"b\"):fx(\"eq\", { freq1 = "),
        "{text}"
    );
    // Un recorte alrededor de 315 Hz, de 2 a 4 dB y con su Q.
    assert_eq!(s.cuts.len(), 1, "{text}");
    let cut = &s.cuts[0];
    assert!((250.0..=400.0).contains(&cut.freq_hz), "{cut:?}");
    assert!((-4.0..=-2.0).contains(&cut.gain_db), "{cut:?}");
    assert!((0.7..=4.4).contains(&cut.q), "{cut:?}");
    assert_eq!(cut.against, [0]);

    // Con priority, manda b aunque suene menos: el recorte va a a.
    let report = song(&format!("{source}priority({{ \"b\", \"a\" }})\n"))
        .analyze_masking(RATE, &MaskingOptions::default())
        .unwrap();
    assert_eq!(report.suggestions.len(), 1);
    assert_eq!(report.tracks[report.suggestions[0].track], "a");
    assert!(
        report.text().contains("priority: b, a"),
        "{}",
        report.text()
    );
    // Basta con nombrar una: la de la lista manda sobre las demás.
    let report = song(&format!("{source}priority({{ \"b\" }})\n"))
        .analyze_masking(RATE, &MaskingOptions::default())
        .unwrap();
    assert_eq!(report.tracks[report.suggestions[0].track], "a");
}

#[test]
fn at_most_four_cuts_from_two_to_four_db() {
    let report = song(&two("multi.wav", "multi.wav"))
        .analyze_masking(RATE, &MaskingOptions::default())
        .unwrap();
    // Cinco zonas que chocan, cuatro recortes (los más fuertes), de grave
    // a agudo.
    let zones: std::collections::BTreeSet<usize> =
        report.clashes().iter().map(|c| c.band / 4).collect();
    assert!(zones.len() >= 5, "{}", report.text());
    let cuts = &report.suggestions[0].cuts;
    assert_eq!(cuts.len(), 4, "{}", report.text());
    assert!(cuts.windows(2).all(|w| w[0].freq_hz < w[1].freq_hz));
    for cut in cuts {
        assert!((-4.0..=-2.0).contains(&cut.gain_db), "{cut:?}");
        assert_eq!(cut.gain_db * 2.0, (cut.gain_db * 2.0).round());
    }
    // La línea de Lua tiene las cuatro bandas y carga.
    let line = report.lua_line(&report.suggestions[0]);
    assert!(
        line.contains("freq4 = ") && !line.contains("freq5"),
        "{line}"
    );
    song(&format!("{}{line}\n", two("multi.wav", "multi.wav")));
}

#[test]
fn priority_errors_and_warnings() {
    let (mut controller, _engine) = new_engine(RATE);
    let mut host = ScriptHost::new(RATE);
    let mut load = |extra: &str| {
        host.load(
            &mut controller,
            &format!("{}{extra}", two("sine300.wav", "sine300.wav")),
            "song.lua",
            &dir(),
        )
        .map(|s| s.warnings.clone())
        .map_err(|e| e.to_string())
    };
    let warnings = load("priority({ \"a\", \"bajo\" })\n").unwrap();
    assert!(
        warnings
            .iter()
            .any(|w| w.contains("priority: no hay ninguna pista \"bajo\"")),
        "{warnings:?}"
    );
    let e = load("priority({ \"a\", \"a\" })\n").unwrap_err();
    assert!(
        e.contains("song.lua:4: priority: \"a\" está dos veces"),
        "{e}"
    );
    let e = load("priority({ 1 })\n").unwrap_err();
    assert!(e.contains("entre comillas"), "{e}");
    let e = load("priority({})\n").unwrap_err();
    assert!(e.contains("falta la lista"), "{e}");
    let e = load("section(\"x\", function() priority({ \"a\" }) end)\n").unwrap_err();
    assert!(e.contains("priority"), "{e}");
}

#[test]
fn applying_the_suggestion_lowers_the_score() {
    let source = two("sine300.wav", "sine300.wav");
    let options = MaskingOptions::default();
    let before = song(&source).analyze_masking(RATE, &options).unwrap();
    let band = band("315");
    let score = before.score("a", "b", band).unwrap();
    let line = before.lua_line(&before.suggestions[0]);
    let after = song(&format!("{source}{line}\n"))
        .analyze_masking(RATE, &options)
        .unwrap();
    let lower = after.score("a", "b", band).unwrap();
    // El recorte (−4 dB sobre los 2 que ya los separaban) la baja mucho.
    assert!(lower < score - 0.3, "{score} → {lower}\n{line}");
    // Y no queda nada que sugerir más fuerte que antes en ninguna banda.
    let worst =
        |r: &script::masking::MaskingReport| r.clashes().first().map_or(0.0, |c| c.clash.score);
    assert!(worst(&after) < worst(&before), "{}", after.text());
}

#[test]
fn a_kick_and_a_bass_clash_in_the_lows_and_group_into_one_range() {
    // El bombo en cada negra y el bajo en cada corchea: los dos entre 50 y
    // 125 Hz.
    let report = song(
        "bpm(120)\n\
         track(\"kick\", sample(\"kick.wav\"):slice_grid(1)):steps(\"1 . . . 1 . . . 1 . . . 1 . . .\")\n\
         track(\"bass\", sample(\"bass.wav\"):slice_grid(1)):steps(\"1 . 1 . 1 . 1 . 1 . 1 . 1 . 1 .\")\n\
         track(\"hat\", sample(\"sine5000.wav\"):slice_grid(1)):steps(\"1 . . .  . . . .  . . . .  . . . .\")\n",
    )
    .analyze_masking(RATE, &MaskingOptions::default())
    .unwrap();
    let groups = report.groups();
    assert!(!groups.is_empty(), "{}", report.text());
    let g = groups[0];
    assert_eq!(
        (report.tracks[g.a].as_str(), report.tracks[g.b].as_str()),
        ("kick", "bass")
    );
    // En los graves de los dos: un rango con 63 Hz, entre 50 y 125 Hz (el
    // bombo baja de 120 a 50 Hz; el bajo, 80 y 160).
    let (lo, hi) = g.bands;
    assert!(
        lo >= band("50") && lo <= band("63") && hi <= band("125"),
        "{}",
        report.text()
    );
    let line = report.group_text(&g);
    assert!(line.starts_with("kick × bass: "), "{line}");
    // Nada con el charles.
    assert!(
        groups.iter().all(|g| g.a != 2 && g.b != 2),
        "{}",
        report.text()
    );
    // El informe lo pone el primero de «por gravedad».
    let text = report.text();
    let first = text
        .lines()
        .skip_while(|l| *l != "por gravedad:")
        .nth(1)
        .unwrap();
    assert_eq!(first.trim(), line);
    // La matriz: los niveles de las dos en sus graves, muy por encima de
    // los del charles.
    for t in 0..2 {
        assert!(
            report.levels[t][band("80")] > -30.0,
            "{:?}",
            report.levels[t]
        );
        assert!(report.levels[2][band("80")] < report.levels[t][band("80")] - 30.0);
    }
}

#[test]
fn the_suggested_sidechain_loads_and_the_bass_gives_way() {
    let base = "bpm(120)\n\
         priority({ \"kick\" })\n\
         track(\"kick\", sample(\"kick.wav\"):slice_grid(1)):steps(\"1 . . . 1 . . . 1 . . . 1 . . .\")\n\
         track(\"bass\", sample(\"bass.wav\"):slice_grid(1)):steps(\"1 . 1 . 1 . 1 . 1 . 1 . 1 . 1 .\")\n";
    let options = MaskingOptions::default();
    let report = song(base).analyze_masking(RATE, &options).unwrap();
    let g = report.groups()[0];
    assert_eq!(report.tracks[g.loser], "bass");
    let fixes = report.fixes_for(&g);
    assert_eq!(fixes.len(), 2);
    let line = &fixes[0].line;
    assert!(
        line.starts_with("track(\"bass\"):sidechain(\"kick\", { mode = \"pattern\""),
        "{line}"
    );
    // El texto del informe las numera bajo su choque.
    assert!(
        report.text().contains(&format!("    1  {line}")),
        "{}",
        report.text()
    );
    // Con la línea, carga y el choque baja.
    let after = song(&format!("{base}{line}\n"))
        .analyze_masking(RATE, &options)
        .unwrap();
    let score = |r: &script::masking::MaskingReport| {
        r.groups()
            .iter()
            .filter(|h| (h.a, h.b) == (g.a, g.b))
            .map(|h| h.score)
            .fold(0.0f32, f32::max)
    };
    assert!(
        score(&after) < score(&report) - 0.05,
        "{} → {}",
        score(&report),
        score(&after)
    );
    // Con el sidechain al revés ya puesto, no sugiere otro (sería un
    // círculo): solo el EQ.
    let reverse = song(&format!(
        "{base}track(\"kick\"):sidechain(\"bass\", {{ depth = -1 }})\n"
    ))
    .analyze_masking(RATE, &options)
    .unwrap();
    let g = reverse.groups()[0];
    let fixes = reverse.fixes_for(&g);
    assert_eq!(fixes.len(), 1);
    assert_eq!(fixes[0].kind, script::masking::FixKind::Eq);
}
