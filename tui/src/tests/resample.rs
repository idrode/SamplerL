//! Remuestrear desde la interfaz: un WAV nuevo en la carpeta de samples que
//! luego se usa en el script.

use super::*;

#[test]
fn a_track_is_resampled_into_the_sample_folder_and_reused() {
    let (path, mut app, _engine) = open("resample", Some(SONG));
    let dir = path.parent().unwrap().to_path_buf();
    palette(&mut app, "remuestrear");
    assert_eq!(app.view().prompt.as_deref(), Some("remuestrear drums "));
    type_query(&mut app, "2");
    app.handle(special(Key::Enter));
    let file = dir.join("drums-remuestreo.wav");
    assert!(
        app.view()
            .message
            .contains("sample(\"drums-remuestreo.wav\")"),
        "{}",
        app.view().message
    );
    let (spec, audio) = read_wav(&file);
    assert_eq!(spec.channels, 2);
    assert_eq!(
        audio.len(),
        2 * 2 * engine::bar_frames(120.0, RATE) as usize
    );

    // No se sobrescribe sin «!».
    command(&mut app, "remuestrear drums 1");
    assert!(
        app.view().message.contains("ya existe"),
        "{}",
        app.view().message
    );
    assert_eq!(read_wav(&file).1.len(), audio.len());
    command(&mut app, "remuestrear! drums 1");
    assert_eq!(read_wav(&file).1.len(), audio.len() / 2);

    // El master, con nombre propio.
    command(&mut app, "remuestrear master 1 mezcla.wav");
    assert!(dir.join("mezcla.wav").exists());

    // Y el remuestreo se usa como cualquier sample.
    let song = "bpm(120)\nlocal r = sample(\"drums-remuestreo.wav\"):slice_grid(4)\n\
                track(\"r\", r):steps(\"1 . 2 .  3 . 4 .  1 . 2 .  3 . 4 .\")\n";
    replace_all(&mut app, song);
    app.handle(ctrl('s'));
    assert!(app.view().error.is_none(), "{:?}", app.view().error);
    assert_eq!(app.view().tracks[0].slices.len(), 4);
}
