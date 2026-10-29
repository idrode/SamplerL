//! Congelar y descongelar la pista seleccionada desde la paleta: el WAV en
//! `congelado/`, una línea `frozen(...)` al final (un paso de deshacer), la
//! pista marcada en el panel del motor en varios tamaños, y descongelar
//! deja el texto exactamente como estaba.

use super::*;

const SONG_FREEZE: &str = r#"bpm(120)
local amen = sample("break.wav"):slice(2)
track("drums", amen)
  :steps("1 . 2 .  1 . 2 .  1 . 2 .  1 . 2 .")
  :fx("lowpass", { cutoff = 2000 })
  :gain(0.8)
track("kick", amen:slice_at(1)):steps("x . . .  x . . .  x . . .  x . . .")
"#;

#[test]
fn freezing_writes_the_wav_and_one_line_and_unfreezing_restores_the_text() {
    let (path, mut app, mut engine) = open("freeze_palette", Some(SONG_FREEZE));
    let before = app.text_for_test();
    palette(&mut app, "congelar la pista");
    let message = app.view().message;
    assert!(
        message.starts_with("drums congelada en congelado/drums.wav (1 compás)"),
        "{message}"
    );
    let wav = path.with_file_name("congelado").join("drums.wav");
    let spec = hound::WavReader::open(&wav).unwrap().spec();
    assert_eq!(
        (spec.sample_format, spec.bits_per_sample, spec.channels),
        (hound::SampleFormat::Float, 32, 2)
    );
    let text = app.text_for_test();
    assert!(text.starts_with(&before), "{text}");
    let added = &text[before.len()..];
    assert!(
        added.starts_with("frozen(\"drums\", \"congelado/drums.wav\", \"")
            && added.ends_with("\") -- congelada: F1 «Descongelar»\n"),
        "{added}"
    );
    // Cargada: la pista toca el WAV, sin efectos, con su volumen.
    assert!(app.view().error.is_none(), "{:?}", app.view().error);
    let drums = &app.view().tracks[0];
    assert_eq!(drums.frozen, Some(false));
    assert!(text_of_screen(&mut app, 120, 40).contains("❄ congelada"));
    for (width, height) in [(120, 40), (90, 28), (70, 24)] {
        let screen = text_of_screen(&mut app, width, height);
        assert!(screen.contains("drums❄"), "{width}x{height}\n{screen}");
    }
    offline::render(&mut engine, 4 * STEP, 2);

    // Descongelar: el texto, exactamente el de antes, y cargado.
    palette(&mut app, "descongelar");
    assert_eq!(app.text_for_test(), before);
    assert_eq!(app.view().tracks[0].frozen, None);
    assert!(wav.exists(), "el WAV se queda");
    // Ctrl-Z vuelve a congelarla de una vez (el texto).
    app.handle(ctrl('z'));
    assert_eq!(app.text_for_test(), text);
    app.handle(ctrl('z'));
    assert_eq!(app.text_for_test(), before);
}

fn text_of_screen(app: &mut App, width: u16, height: u16) -> String {
    text(&draw(app, width, height))
}

#[test]
fn freezing_needs_the_loaded_text() {
    let (_, mut app, _engine) = open("freeze_unloaded", Some(SONG_FREEZE));
    type_text(&mut app, "-- algo sin cargar");
    let before = app.text_for_test();
    palette(&mut app, "congelar la pista");
    assert!(
        app.view().message.contains("Ctrl-S"),
        "{}",
        app.view().message
    );
    assert_eq!(app.text_for_test(), before);
}

#[test]
fn what_cannot_be_frozen_says_why() {
    let song = "bpm(120)\nlocal amen = sample(\"break.wav\"):slice(2)\n\
        section(\"a\", function() track(\"d\", amen):steps(\"1 . . .  . . . .  . . . .  . . . .\") end)\n";
    let (_, mut app, _engine) = open("freeze_sections", Some(song));
    let before = app.text_for_test();
    palette(&mut app, "congelar la pista");
    assert!(
        app.view().message.contains("con secciones no se congela"),
        "{}",
        app.view().message
    );
    assert_eq!(app.text_for_test(), before);
    palette(&mut app, "descongelar");
    assert!(
        app.view().message.contains("no está congelada"),
        "{}",
        app.view().message
    );
}
