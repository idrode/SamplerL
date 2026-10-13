//! Partir y unir cortes desde la paleta: se guarda en el proyecto y las
//! etiquetas de los cortes siguientes se corren.

use super::*;

#[test]
fn slices_are_split_and_joined_from_the_palette() {
    let song = "bpm(120)\nlocal amen = sample(\"break.wav\"):slice_grid(2)\n\
                track(\"drums\", amen):steps(\"1 . 2 .  1 . 2 .  1 . 2 .  1 . 2 .\")\n";
    let (path, mut app, _engine) = open("grid_edit", Some(song));
    command(&mut app, "guardar");
    command(&mut app, "etiquetar 2 caja");
    // Partir el corte 1 (de 0 a 3000 frames): uno nuevo en su mitad.
    palette(&mut app, "partir el corte");
    let view = app.view();
    assert_eq!(view.tracks[0].slices, [0..1500, 1500..3000, 3000..6000]);
    assert!(
        view.message.contains("ahora hay 3 (el nuevo es el 2)"),
        "{}",
        view.message
    );
    assert_eq!(view.selected_slice, Some(1));
    // La etiqueta del corte 2 pasa al 3.
    assert_eq!(view.tracks[0].labels[2].as_deref(), Some("caja"));
    let project = std::fs::read_to_string(path.with_extension("sampler")).unwrap();
    assert!(
        project.contains("partir = break.wav rejilla2 31.25\n"),
        "{project}"
    );
    assert!(
        project.contains("etiqueta = break.wav rejilla2 3 caja\n"),
        "{project}"
    );

    // Unir el seleccionado (el nuevo) con el anterior: como antes.
    palette(&mut app, "unir el corte");
    let view = app.view();
    assert_eq!(view.tracks[0].slices, [0..3000, 3000..6000]);
    assert_eq!(view.tracks[0].labels[1].as_deref(), Some("caja"));
    // El corte 1 no tiene anterior.
    palette(&mut app, "unir el corte");
    assert!(app.view().message.contains("no tiene anterior"));
}
