//! `sample:stretch()` en la interfaz: la vista del sample lo dice y los
//! ajustes de cortes siguen en ms del sample original.

use super::*;

#[test]
fn the_sample_view_shows_the_stretch_and_nudges_in_original_ms() {
    // El break de prueba (6000 frames) a 120, estirado a 150: ×0,8.
    let song = r#"bpm(150)
local amen = sample("break.wav"):stretch(120):slice(2)
track("drums", amen):steps("1 . 2 .  1 . 2 .  1 . 2 .  1 . 2 .")
"#;
    let (path, mut app, _engine) = open("stretch_view", Some(song));
    let view = app.view();
    assert_eq!(view.tracks[0].stretch, Some((120.0, 0.8)));
    assert_eq!(view.tracks[0].sample_frames, 4800);
    let screen = text(&draw(&mut app, 140, 40));
    assert!(
        screen.contains("break.wav · 120→150 BPM · 0.10 s"),
        "{screen}"
    );

    // Corte 2: en 3000 frames del original (62,5 ms); +5 ms → 67,5 ms del
    // original, guardado así en el proyecto.
    command(&mut app, "guardar");
    app.handle(esc());
    app.select_slice(true);
    let screen = text(&draw(&mut app, 160, 40));
    assert!(screen.contains("corte 2 62.5"), "{screen}");
    app.adjust_slice(Some(5.0));
    let project = std::fs::read_to_string(path.with_extension("sampler")).unwrap();
    assert!(project.contains("corte = break.wav 2 2 67.5"), "{project}");
    let start = app.view().tracks[0].slices[1].start;
    assert_eq!(start, (0.0675 * 48_000.0 * 0.8f64).round() as usize);
}
