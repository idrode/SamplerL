//! Dónde está escrito cada paso (fase M, resaltado en vivo): las
//! posiciones de los tokens de `:steps()`, `:melody()` y `c:steps()` en el
//! texto cargado, y el token que se resalta en cada paso
//! (`TrackDef::sounding_span`).

use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use engine::{new_engine, offline};
use script::{ScriptHost, Song, TokenSpan};

const RATE: u32 = 48_000;

fn dir() -> PathBuf {
    static DIR: OnceLock<PathBuf> = OnceLock::new();
    DIR.get_or_init(|| {
        let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join("spans");
        std::fs::create_dir_all(&dir).unwrap();
        let mut mono = vec![0.0f32; 6000];
        mono[..800].fill(0.5);
        mono[3000..3800].fill(0.25);
        offline::write_wav(dir.join("break.wav"), &mono, 1, RATE).unwrap();
        dir
    })
    .clone()
}

/// Dos líneas de cabecera: la fila 2 del texto es la primera de `source`.
fn song(source: &str) -> Song {
    let (mut controller, _engine) = new_engine(RATE);
    let mut host = ScriptHost::new(RATE);
    host.set_seed(Some(3));
    let source = format!("bpm(120)\nlocal b = sample(\"break.wav\"):slice(2)\n{source}");
    host.load(&mut controller, &source, "song.lua", &dir())
        .cloned()
        .unwrap()
}

/// El texto que marca cada paso del ciclo de la pista (`-` donde nada).
fn lit(song: &Song, track: &str, source: &str) -> Vec<String> {
    let lines: Vec<&str> = source.lines().collect();
    let t = song.tracks.iter().find(|t| t.name == track).unwrap();
    (0..t.length * t.bars.len())
        .map(|i| match t.sounding_span(i) {
            None => "-".into(),
            Some(TokenSpan { row, start, end }) => {
                let line: Vec<char> = lines[row - 2].chars().collect();
                format!("{row}:{}", line[start..end].iter().collect::<String>())
            }
        })
        .collect()
}

fn strings(list: &[&str]) -> Vec<String> {
    list.iter().map(|s| s.to_string()).collect()
}

#[test]
fn each_step_points_at_its_token_holds_at_their_head_and_rests_at_nothing() {
    let source = "track(\"d\", b):length(6):mode(\"gate\"):steps(\"1 _ _ . 2r3 2'^7@0.5\")";
    let song = song(source);
    assert_eq!(
        lit(&song, "d", source),
        strings(&["2:1", "2:1", "2:1", "-", "2:2r3", "2:2'^7@0.5"])
    );
    // Las columnas son las del texto (en caracteres).
    let t = &song.tracks[0];
    let at = source.find("2r3").unwrap();
    assert_eq!(
        t.sounding_span(4),
        Some(TokenSpan {
            row: 2,
            start: at,
            end: at + 3
        })
    );
}

#[test]
fn chains_over_several_lines_and_patterns_in_a_variable() {
    let source = "local p = \"x . x .\"\n\
                  track(\"d\", b)\n  :length(4)\n  :steps(\"1 . 2 .\")\n\
                  track(\"k\", b):length(4)\n  :steps(p)";
    let song = song(source);
    assert_eq!(lit(&song, "d", source), strings(&["5:1", "-", "5:2", "-"]));
    // En una variable: donde se escribió (es la única cadena igual).
    assert_eq!(lit(&song, "k", source), strings(&["2:x", "-", "2:x", "-"]));
}

#[test]
fn generated_patterns_and_repeated_literals_elsewhere_have_no_place() {
    let source = "track(\"e\", b):length(8):steps(euclid(3, 8, \"x\", 8))\n\
                  local p = \"1 . . .\"\nlocal q = \"1 . . .\"\n\
                  track(\"r\", b):length(4):steps(p)\n\
                  track(\"c\", b):length(4):steps(\"1 . \" .. \"2 .\")";
    let song = song(source);
    for track in ["e", "r", "c"] {
        let marks = lit(&song, track, source);
        assert!(marks.iter().all(|m| m == "-"), "{track}: {marks:?}");
    }
}

#[test]
fn every_keeps_the_base_tokens_unless_it_writes_its_own() {
    let source = "track(\"d\", b):length(4):steps(\"1 . 2 .\")\n\
                  \x20 :every(2, function(c) c:steps(\". 2 . 1\") end)\n\
                  track(\"r\", b):length(4):steps(\"1 . 2 .\")\n\
                  \x20 :every(2, function(c) c:reverse():retrig(2) end)\n\
                  \x20 :fill(\"mute\", 2, 1)";
    let song = song(source);
    // La vuelta 0 (con `every`) marca sus tokens; la 1, los de la pista.
    assert_eq!(
        lit(&song, "d", source),
        strings(&["-", "3:2", "-", "3:1", "2:1", "-", "2:2", "-"])
    );
    // Reverse y retrig no mueven los pasos; el fill quita el último golpe
    // de la vuelta 1 (aquí no hay: el 4.º es un silencio).
    assert_eq!(
        lit(&song, "r", source),
        strings(&["4:1", "-", "4:2", "-", "4:1", "-", "4:2", "-"])
    );
}

#[test]
fn melody_and_sections_point_at_their_own_text() {
    let source = "section(\"a\", function()\n\
                  \x20 track(\"m\", b):length(4):melody(\"c _ . c\")\n\
                  end)";
    let song = song(source);
    let t = &song.sections[0].tracks[0];
    let lines: Vec<&str> = source.lines().collect();
    let marks: Vec<String> = (0..4)
        .map(|i| {
            t.sounding_span(i).map_or("-".into(), |s| {
                let line: Vec<char> = lines[s.row - 2].chars().collect();
                line[s.start..s.end].iter().collect()
            })
        })
        .collect();
    assert_eq!(marks, strings(&["c", "c", "-", "c"]));
    assert_eq!(t.sounding_span(3).unwrap().row, 3);
}
