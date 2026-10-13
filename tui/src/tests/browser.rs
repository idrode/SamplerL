//! El explorador de samples: filtra, preescucha en la voz aparte (sin tocar
//! las pistas) e inserta `sample("...")`.

use super::*;

const QUIET: &str = "bpm(120)\nlocal amen = sample(\"break.wav\"):slice(2)\n\
                     track(\"drums\", amen):steps(\". . . .  . . . .  . . . .  . . . .\")\n";

#[test]
fn the_browser_filters_previews_and_inserts() {
    let (path, mut app, mut engine) = open("browser", Some(QUIET));
    let dir = path.parent().unwrap();
    std::fs::create_dir_all(dir.join("kits")).unwrap();
    let bombo = vec![0.3f32; 2000];
    offline::write_wav(dir.join("kits/bombo.wav"), &bombo, 1, RATE).unwrap();
    std::fs::write(dir.join("notas.txt"), "no").unwrap();

    palette(&mut app, "explorador");
    let view = app.view().browser.unwrap();
    assert_eq!(view.items, ["break.wav", "kits/bombo.wav"]);
    let screen = text(&draw(&mut app, 120, 40));
    assert!(screen.contains("SAMPLES: escribe para filtrar"), "{screen}");

    // Filtrar no preescucha; las flechas, sí.
    type_query(&mut app, "bom");
    assert_eq!(app.view().browser.unwrap().items, ["kits/bombo.wav"]);
    offline::render(&mut engine, 1000, 2);
    app.handle(special(Key::Down));
    assert_eq!(app.view().browser.unwrap().info, "kits/bombo.wav · 0.04 s");
    let audio = offline::render(&mut engine, 2000, 2);
    // La voz de preescucha: el bombo a nivel completo tras su fundido (el
    // WAV es de 16 bits).
    assert!((audio[2 * 1000] - 0.3).abs() < 1e-4, "{}", audio[2 * 1000]);
    // Las pistas, igual que antes (no han sonado).
    let status = app.controller_status_for_test();
    assert!(status.tracks.iter().all(|t| t.hit_start.is_none()));

    // Enter inserta sample("...") en el cursor y cierra.
    app.handle(special(Key::Enter));
    assert!(app.view().browser.is_none());
    assert!(app.text_for_test().contains("sample(\"kits/bombo.wav\")"));
    // Se deshace de una vez.
    app.handle(ctrl('z'));
    assert!(!app.text_for_test().contains("bombo"));
}
