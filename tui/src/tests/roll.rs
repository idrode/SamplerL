//! El piano roll: la vista (de lo que toca el secuenciador) y la edición,
//! que escribe cada cambio en sus tokens del código (un paso de deshacer,
//! sin tocar nada más del texto) y carga.

use script::{ScriptHost, Song};

use super::*;
use crate::app::MouseAction;
use crate::palette::Action;
use crate::roll::Style;

pub const ROLL_SONG: &str = r#"bpm(120)
key("A", "minor")
local piano = sample("break.wav"):instrument({ root = "C4" })
-- el bajo, con un acorde
track("bajo", piano)
    :melody("a2 _ . a2  c3 _ . e3  g2 _ _ _  a2 . c3+e3 .") -- fin
local amen = sample("break.wav"):slice(2)
track("drums", amen):steps("1 . 2 .  1 . 2r4 .  1' . 2 .  1 . 2^7 .")
"#;

/// A2, C3, E3, G2, A3 (C4 = 60).
const A2: i32 = 45;
const C3: i32 = 48;
const E3: i32 = 52;
const G2: i32 = 43;
const A3: i32 = 57;

/// La canción de un texto, cargada aparte (como la cargaría otra vez el
/// editor).
fn song_of(path: &Path, text: &str) -> Song {
    let dir = path.parent().unwrap();
    let mut host = ScriptHost::new(RATE);
    host.set_seed(Some(1));
    host.eval(text, "song.lua", dir).unwrap()
}

fn shift(key: Key) -> Input {
    Input {
        key,
        shift: true,
        ..Input::default()
    }
}

/// Abre el piano roll en la pista `track` (por su orden).
fn roll_on(test: &str, song: &str, track: usize) -> (PathBuf, App, Engines) {
    let (path, mut app, engine) = open(test, Some(song));
    ENGINE.with(|e| *e.borrow_mut() = Some(engine));
    for _ in 0..track {
        app.run_action(Action::NextTrack);
    }
    app.run_action(Action::EditRoll);
    (path, app, Engines)
}

/// El motor del test (en `ENGINE`, para que `at` lo mueva).
struct Engines;

impl Engines {
    fn render(&self, frames: usize) -> Vec<f32> {
        ENGINE.with(|e| offline::render(e.borrow_mut().as_mut().unwrap(), frames, 2))
    }
}

/// Pone el cursor en `(col, pitch)` y pulsa `input`; luego el motor
/// atiende lo que le llega (como en la aplicación, entre fotogramas).
fn at(app: &mut App, col: usize, pitch: i32, input: Input) {
    app.roll_cursor_to(col, pitch);
    app.handle(input);
    settle(app);
}

/// El motor de los tests sin hilo de audio: lo que se le manda se queda en
/// la cola hasta que suena un bloque. Aquí se vacía con un motor aparte
/// por test (ver `ENGINES`).
fn settle(app: &mut App) {
    ENGINE.with(|e| {
        if let Some(engine) = e.borrow_mut().as_mut() {
            offline::render(engine, 256, 2);
        }
    });
    app.tick();
}

/// Una tecla, y el motor atiende.
fn press(app: &mut App, input: Input) {
    app.handle(input);
    settle(app);
}

/// El ratón, y el motor atiende.
fn click(app: &mut App, action: MouseAction, x: u16, y: u16) {
    app.mouse(action, x, y);
    settle(app);
}

thread_local! {
    static ENGINE: std::cell::RefCell<Option<Engine>> = const { std::cell::RefCell::new(None) };
}

/// Que el texto sea `song` con `from` cambiado por `to` (una vez) y que
/// la carga haya ido bien.
fn assert_edited(app: &App, song: &str, from: &str, to: &str) -> String {
    let expected = song.replacen(from, to, 1);
    assert_ne!(expected, song, "«{from}» no está en la canción");
    let view = app.view();
    assert_eq!(app.text_for_test(), expected, "{}", view.message);
    assert!(view.error.is_none(), "{:?}", view.error);
    expected
}

/// Ida y vuelta: lo cargado es lo del texto, y releerlo da los mismos
/// pasos y el mismo audio que escribirlo a mano.
fn assert_round_trip(path: &Path, app: &App, expected: &str) {
    let again = song_of(path, &app.text_for_test());
    let by_hand = song_of(path, expected);
    for (loaded, by_hand) in app.tracks().iter().zip(&by_hand.tracks) {
        assert_eq!(loaded.bars, by_hand.bars, "pista {}", loaded.name);
    }
    assert_eq!(
        again.render(RATE, 1).unwrap(),
        by_hand.render(RATE, 1).unwrap()
    );
}

#[test]
fn the_view_shows_notes_with_their_length_volume_chords_and_the_key() {
    let (_, app, _engine) = roll_on("roll_view", ROLL_SONG, 0);
    let view = app.roll_view_for_test().expect("el piano roll");
    assert!(view.notes_axis);
    assert_eq!(view.steps, 16);
    let at = |col: usize, pitch: i32| {
        view.notes
            .iter()
            .find(|n| n.col == col && n.pitch == pitch)
            .copied()
            .unwrap_or_else(|| panic!("sin nota en {col} {pitch}"))
    };
    // `a2 _`: dos pasos; `g2 _ _ _`: cuatro.
    assert_eq!(at(0, A2).len, 2);
    assert_eq!(at(8, G2).len, 4);
    // El acorde, apilado en el mismo paso.
    assert_eq!(at(14, C3).len, 1);
    assert_eq!(at(14, E3).len, 1);
    // La menor: todas dentro; la tónica, A.
    let scale = view.scale.expect("la tonalidad");
    assert_eq!(scale.tonic, 9);
    assert!(view.notes.iter().all(|n| n.style == Style::Scale));
    assert_eq!(view.range, Some((G2, E3)));
}

#[test]
fn the_panel_draws_at_several_sizes() {
    for (w, h) in [(120, 40), (100, 30), (80, 24)] {
        let (_, mut app, _engine) = roll_on(&format!("roll_sizes_{w}"), ROLL_SONG, 0);
        let screen = text(&draw(&mut app, w, h));
        assert!(screen.contains("piano roll · bajo"), "{w}×{h}\n{screen}");
        // Las notas, con su duración (a una o dos celdas por paso, o
        // en media celda si no cabe la tesitura).
        assert!(
            screen.contains('━') || screen.contains('▀') || screen.contains('▄'),
            "{w}×{h}\n{screen}"
        );
        // Los nombres de las notas a la izquierda.
        assert!(
            screen.contains("A2") || screen.contains("C3"),
            "{w}×{h}\n{screen}"
        );
        // El tracker y el sample no se ven: el piano roll está en su sitio.
        assert!(!screen.contains(" tracker "), "{w}×{h}\n{screen}");
    }
    // Sin el piano roll, como siempre.
    let (_, mut app, _engine) = open("roll_sizes_none", Some(ROLL_SONG));
    let screen = text(&draw(&mut app, 100, 30));
    assert!(!screen.contains("piano roll"), "{screen}");
}

#[test]
fn a_slice_track_shows_one_row_per_slice_with_its_marks() {
    let (_, mut app, _engine) = roll_on("roll_slices", ROLL_SONG, 1);
    let view = app.roll_view_for_test().unwrap();
    assert!(!view.notes_axis);
    assert_eq!(view.labels, ["1", "2"]);
    let mark = |col: usize| view.notes.iter().find(|n| n.col == col).unwrap().mark;
    assert_eq!(mark(6), '4'); // 2r4
    assert_eq!(mark(8), '◂'); // 1'
    assert_eq!(mark(14), '▴'); // 2^7
    let screen = text(&draw(&mut app, 120, 40));
    assert!(screen.contains("piano roll · drums · 2 cortes"), "{screen}");
    assert!(screen.contains("◂") && screen.contains('▴'), "{screen}");
}

#[test]
fn each_edit_lands_in_its_tokens_and_reloads_the_same() {
    let (path, mut app, _engine) = roll_on("roll_edits", ROLL_SONG, 0);
    // Una nota nueva en un silencio.
    at(&mut app, 2, C3, special(Key::Enter));
    let song = assert_edited(&app, ROLL_SONG, "a2 _ . a2", "a2 _ c3 a2");
    assert_round_trip(&path, &app, &song);
    // Un semitono arriba: el nombre, con su estilo (sin bemoles: #).
    at(&mut app, 0, A2, key('+'));
    let song = assert_edited(&app, &song, "a2 _ c3", "a#2 _ c3");
    // Una octava abajo.
    at(&mut app, 4, C3, key('<'));
    let song = assert_edited(&app, &song, "c3 _ . e3", "c2 _ . e3");
    // Alargar y acortar con `_`.
    at(&mut app, 4, C3 - 12, key(']'));
    let song = assert_edited(&app, &song, "c2 _ . e3", "c2 _ _ e3");
    at(&mut app, 8, G2, key('['));
    let song = assert_edited(&app, &song, "g2 _ _ _", "g2 _ _ .");
    // Volumen.
    at(&mut app, 7, E3, key('v'));
    let song = assert_edited(&app, &song, "_ e3  g2", "_ e3@0.9  g2");
    // Mover antes: la nota y sus `_` pasan a otro sitio.
    at(&mut app, 12, A2, key(','));
    let song = assert_edited(&app, &song, "g2 _ _ .  a2 . c3+e3", "g2 _ _ a2  . . c3+e3");
    assert_round_trip(&path, &app, &song);
    // Acorde: otra nota en el mismo paso.
    at(&mut app, 14, A3, special(Key::Enter));
    let song = assert_edited(&app, &song, "c3+e3", "c3+e3+a3");
    // Una del acorde un semitono arriba; quitar otra.
    at(&mut app, 14, E3, key('+'));
    let song = assert_edited(&app, &song, "c3+e3+a3", "c3+f3+a3");
    at(&mut app, 14, A3, special(Key::Delete));
    let song = assert_edited(&app, &song, "c3+f3+a3", "c3+f3");
    // Quitar una nota larga: sus `_` también.
    at(&mut app, 4, C3 - 12, special(Key::Delete));
    let song = assert_edited(&app, &song, "c2 _ _ e3", ". . . e3");
    assert_round_trip(&path, &app, &song);
    // Cada cambio, un paso de deshacer.
    app.handle(ctrl('z'));
    assert_eq!(
        app.text_for_test(),
        song.replacen(". . . e3", "c2 _ _ e3", 1)
    );
}

#[test]
fn comments_and_indentation_stay_exactly_as_they_were() {
    let song = r#"bpm(120)
local piano = sample("break.wav"):instrument({ root = "C4" })
    -- un comentario con "c4 . . ." dentro
track("lead", piano)
        :melody(  "c4 . e4 .  g4 . . .  c5 _ . .  . . . ."  )   -- otro
--[[ largo
  "c4 . e4 ."
]]
"#;
    let (path, mut app, _engine) = roll_on("roll_comments", song, 0);
    at(&mut app, 2, 64, key('+'));
    let edited = assert_edited(&app, song, "c4 . e4 .  g4", "c4 . f4 .  g4");
    // Solo esos caracteres: el resto, línea a línea, igual.
    let before: Vec<&str> = song.lines().collect();
    let after: Vec<&str> = edited.lines().collect();
    assert_eq!(before.len(), after.len());
    let changed: Vec<usize> = (0..before.len())
        .filter(|&i| before[i] != after[i])
        .collect();
    assert_eq!(changed, [4]);
    assert_eq!(&after[4][..18], &before[4][..18]);
    assert!(after[4].ends_with(r#"  )   -- otro"#));
    assert_round_trip(&path, &app, &edited);
}

#[test]
fn steps_tracks_with_an_instrument_and_slice_tracks_are_written_as_steps() {
    let song = r#"bpm(120)
local piano = sample("break.wav"):instrument({ root = "C4" })
track("keys", piano):steps("1 . 1^7 .  1^0^4^7 . . .  1@0.5 . . .  . . . .")
local amen = sample("break.wav"):slice(2)
amen:name(2, "snare")
track("drums", amen):steps("1 . snare .  1 . 2r4 .  1' . 2 .  1 . 2^7 .")
"#;
    let (path, mut app, _engine) = roll_on("roll_steps", song, 0);
    // En `steps()`, la nota es `^n` sobre la raíz.
    at(&mut app, 3, 62, special(Key::Enter));
    let s = assert_edited(&app, song, "1 . 1^7 .", "1 . 1^7 1^2");
    at(&mut app, 2, 67, key('+'));
    let s = assert_edited(&app, &s, "1^7 1^2", "1^8 1^2");
    at(&mut app, 4, 64, key('-'));
    let s = assert_edited(&app, &s, "1^0^4^7", "1^0^3^7");
    at(&mut app, 8, 60, key('V'));
    let s = assert_edited(&app, &s, "1@0.5", "1@0.6");
    assert_round_trip(&path, &app, &s);
    // La pista de cortes: + y - son otro corte (con su etiqueta).
    app.run_action(Action::NextTrack);
    app.tick();
    at(&mut app, 0, 0, key('+'));
    let s = assert_edited(&app, &s, "steps(\"1 . snare", "steps(\"snare . snare");
    at(&mut app, 6, 1, key('-'));
    let s = assert_edited(&app, &s, "2r4", "1r4");
    // Y la octava es el `^`.
    at(&mut app, 14, 1, key('>'));
    let s = assert_edited(&app, &s, "2^7", "2^19");
    // Un golpe nuevo en un silencio, en la fila de su corte.
    at(&mut app, 1, 1, special(Key::Enter));
    let s = assert_edited(&app, &s, "snare . snare", "snare snare snare");
    assert_round_trip(&path, &app, &s);
}

#[test]
fn lengths_laps_and_every_write_where_each_lap_is_written() {
    let song = r#"bpm(120)
local piano = sample("break.wav"):instrument({ root = "C4" })
track("lead", piano):length(12):melody("c4 . e4 .  g4 _ . .  c5 . . .")
  :every(2, function(c) c:melody("c4 . . .  e4 . . .  g4 . . .") end)
"#;
    let (path, mut app, _engine) = roll_on("roll_laps", song, 0);
    let view = app.roll_view_for_test().unwrap();
    // Dos vueltas de 12.
    assert_eq!(view.steps, 24);
    assert_eq!(view.length, 12);
    // La vuelta 0 es la de `every` (otro estilo), la 1 la de siempre.
    let style = |col: usize| view.notes.iter().find(|n| n.col == col).unwrap().style;
    assert_eq!(style(4), Style::Variation);
    assert_eq!(style(16), Style::Scale);
    // Cada una se escribe en su cadena.
    at(&mut app, 4, 64, key('+'));
    let s = assert_edited(&app, song, "c4 . . .  e4", "c4 . . .  f4");
    at(&mut app, 12 + 4, 67, key(']'));
    let s = assert_edited(&app, &s, "g4 _ . .", "g4 _ _ .");
    assert_round_trip(&path, &app, &s);
    // Una nota no cruza el final de su vuelta.
    at(&mut app, 8, 67, key('.'));
    at(&mut app, 9, 67, key('.'));
    at(&mut app, 10, 67, key('.'));
    at(&mut app, 11, 67, key('.'));
    let message = app.view().message;
    assert!(
        message.contains("no cruza el final de la vuelta"),
        "{message}"
    );
}

#[test]
fn what_cannot_be_rewritten_safely_says_why_and_leaves_the_text_alone() {
    let song = r#"bpm(120)
local piano = sample("break.wav"):instrument({ root = "C4" })
track("gen", piano):steps(euclid(3, 8, "1", 16))
track("var", piano):melody("c4 . e4 .  g4 . . .  c5 . . .  . . . .")
  :every(2, function(c) c:pitch(7) end)
track("mute", piano):melody("c4 . . .  . . . .  . . . .  . . . ."):chance(1, "mute")
local p = "1 . . .  . . . .  . . . .  . . . ."
local amen = sample("break.wav"):slice(2)
track("a", amen):steps(p)
track("b", amen):steps(p)
"#;
    let (_, mut app, _engine) = roll_on("roll_refuse", song, 0);
    let check = |app: &mut App, col: usize, pitch: i32, input: Input, why: &str| {
        at(app, col, pitch, input);
        assert_eq!(app.text_for_test(), song);
        let message = app.view().message;
        assert!(message.contains(why), "esperaba «{why}»: {message}");
    };
    // Generado: se ve, no se edita.
    let view = app.roll_view_for_test().unwrap();
    assert!(view.notes.iter().all(|n| n.style == Style::Generated));
    check(&mut app, 0, 60, key('+'), "no está escrito tal cual");
    check(
        &mut app,
        1,
        60,
        special(Key::Enter),
        "no está escrito tal cual",
    );
    // Una vuelta que `every` cambia: la otra sí.
    app.run_action(Action::NextTrack);
    app.tick();
    check(&mut app, 0, 67, key('+'), "every, chance o fill");
    at(&mut app, 16, 60, key('+'));
    assert!(app.text_for_test().contains("c#4 . e4"));
    let song = app.text_for_test();
    // Lo que quita `chance`: se ve con su ×.
    app.run_action(Action::NextTrack);
    app.tick();
    let view = app.roll_view_for_test().unwrap();
    assert!(view.notes.iter().any(|n| n.style == Style::Dropped));
    at(&mut app, 0, 60, key('+'));
    assert_eq!(app.text_for_test(), song);
    assert!(
        app.view().message.contains("no hay ninguna nota"),
        "{}",
        app.view().message
    );
    at(&mut app, 0, 60, special(Key::Enter));
    assert_eq!(app.text_for_test(), song);
    assert!(
        app.view().message.contains("quitan este golpe"),
        "{}",
        app.view().message
    );
    // Una cadena compartida por dos pistas.
    app.run_action(Action::NextTrack);
    app.tick();
    at(&mut app, 1, 0, special(Key::Enter));
    assert_eq!(app.text_for_test(), song);
    assert!(
        app.view().message.contains("la usa también la pista «b»"),
        "{}",
        app.view().message
    );
    // Un texto sin cargar.
    app.run_action(Action::NextTrack);
    app.tick();
    app.handle(esc());
    app.editor.insert_str("-- nada\n");
    app.run_action(Action::EditRoll);
    let text = app.text_for_test();
    at(&mut app, 1, 0, special(Key::Enter));
    assert_eq!(app.text_for_test(), text);
    assert!(
        app.view().message.contains("carga antes"),
        "{}",
        app.view().message
    );
}

#[test]
fn selections_move_transpose_and_duplicate_together() {
    let song = r#"bpm(120)
local piano = sample("break.wav"):instrument({ root = "C4" })
track("lead", piano):melody("c4 e4 g4 .  . . . .  . . . .  . . . .")
"#;
    let (path, mut app, _engine) = roll_on("roll_selection", song, 0);
    // Shift y → seleccionan tres pasos.
    app.roll_cursor_to(0, 60);
    press(&mut app, shift(Key::Right));
    press(&mut app, shift(Key::Right));
    assert_eq!(app.roll_cursor().unwrap().1, Some((0, 2)));
    // Todo un tono arriba.
    press(&mut app, key('+'));
    press(&mut app, key('+'));
    let s = assert_edited(&app, song, "c4 e4 g4 .", "d4 f#4 a4 .");
    // Duplicado justo detrás, sobre los silencios.
    press(&mut app, key('d'));
    let s = assert_edited(&app, &s, "d4 f#4 a4 .  . .", "d4 f#4 a4 d4  f#4 a4");
    assert_eq!(app.roll_cursor().unwrap().1, Some((3, 5)));
    // La copia, dos pasos después (con la rejilla a 1/8).
    press(&mut app, key('G'));
    press(&mut app, key('.'));
    let s = assert_edited(&app, &s, "a4 d4  f#4 a4 . .", "a4 .  . d4 f#4 a4");
    assert_round_trip(&path, &app, &s);
    // Sin sitio detrás: no duplica y lo dice.
    app.roll_cursor_to(0, 62);
    press(&mut app, shift(Key::Right));
    press(&mut app, key('d'));
    assert_eq!(app.text_for_test(), s);
    assert!(
        app.view().message.contains("ya tiene algo"),
        "{}",
        app.view().message
    );
}

#[test]
fn degrees_stay_degrees_when_the_note_is_in_the_key() {
    let song = r#"bpm(120)
key("A", "minor")
local piano = sample("break.wav"):instrument({ root = "C4" })
track("lead", piano):melody("1 . 3 .  5 . 1+3+5 .  . . . .  . . . .")
transpose(2)
"#;
    let (path, mut app, _engine) = roll_on("roll_degrees", song, 0);
    // Con transpose(2), el grado 1 (A3) suena en B3 (59).
    let view = app.roll_view_for_test().unwrap();
    assert!(view.notes.iter().any(|n| n.col == 0 && n.pitch == 59));
    // Un semitono arriba del grado 3 (C4, que suena D4): C#4 no es de la
    // escala, así que va por su nombre; otro más y es el grado 4.
    at(&mut app, 2, 62, key('+'));
    let s = assert_edited(&app, song, "1 . 3 .", "1 . c#4 .");
    at(&mut app, 2, 63, key('+'));
    let s = assert_edited(&app, &s, "1 . c#4 .", "1 . d4 .");
    // En un acorde de grados, la nota que sube sigue siendo un grado.
    // En un acorde de grados, la nota que sale de la escala va por su
    // nombre y las demás siguen siendo grados.
    at(&mut app, 6, 59, key('+'));
    let s = assert_edited(&app, &s, "1+3+5", "a#3+3+5");
    assert_round_trip(&path, &app, &s);
}

#[test]
fn moving_or_resizing_never_sounds_and_preview_does_until_panic() {
    let (_, mut app, engine) = roll_on("roll_silent", ROLL_SONG, 0);
    // Parado: lo que suene es del piano roll.
    app.handle(ctrl('t'));
    engine.render(4096);
    app.tick();
    let silent = |engine: &Engines| {
        let audio = engine.render(8192);
        audio.iter().all(|s| s.abs() < 1e-6)
    };
    assert!(silent(&engine));
    // Teclas: mover, alargar, otra nota.
    at(&mut app, 12, A2, key(','));
    at(&mut app, 11, A2, key(']'));
    at(&mut app, 4, C3, key('+'));
    at(&mut app, 2, C3, special(Key::Enter));
    app.tick();
    assert!(silent(&engine), "{}", app.view().message);
    // Ratón: arrastrar una nota y un clic en un hueco.
    let mut term = draw(&mut app, 120, 40);
    let (x, y) = cell_of(&mut app, &mut term, 0, A2);
    app.mouse(MouseAction::Down, x, y);
    app.mouse(MouseAction::Drag, x, y.saturating_sub(1));
    app.mouse(MouseAction::Up, x, y.saturating_sub(1));
    app.tick();
    assert!(silent(&engine), "{}", app.view().message);
    // La preescucha sí suena, y el pánico la corta.
    app.roll_cursor_to(4, C3 + 1);
    app.handle(key(' '));
    let audio = engine.render(4096);
    assert!(
        audio.iter().any(|s| s.abs() > 1e-3),
        "{}",
        app.view().message
    );
    app.run_action(Action::Panic);
    engine.render(4096);
    assert!(silent(&engine));
}

/// La celda de la pantalla de `(col, pitch)` en el piano roll que se ve.
fn cell_of(app: &mut App, _buffer: &mut Buffer, col: usize, pitch: i32) -> (u16, u16) {
    let view = app.roll_view_for_test().unwrap();
    let area = app.roll_area_for_test().expect("el piano roll dibujado");
    let lay = crate::roll_panel::layout(&view, area);
    (lay.x_of(col).unwrap(), lay.y_of(pitch).unwrap())
}

#[test]
fn the_mouse_places_moves_lengthens_and_deletes_notes() {
    let (path, mut app, _engine) = roll_on("roll_mouse", ROLL_SONG, 0);
    let mut screen = draw(&mut app, 120, 40);
    // Arrastrar la cabeza: un semitono arriba y dos pasos después.
    let (x, y) = cell_of(&mut app, &mut screen, 12, A2);
    let (x2, y2) = cell_of(&mut app, &mut screen, 13, A2 + 1);
    click(&mut app, MouseAction::Down, x, y);
    click(&mut app, MouseAction::Drag, x2, y2);
    click(&mut app, MouseAction::Up, x2, y2);
    let s = assert_edited(&app, ROLL_SONG, "a2 . c3+e3", ". a#2 c3+e3");
    // Arrastrar la cola de una nota larga: más larga.
    let mut screen = draw(&mut app, 120, 40);
    let (x, y) = cell_of(&mut app, &mut screen, 5, C3);
    let (x2, _) = cell_of(&mut app, &mut screen, 6, C3);
    click(&mut app, MouseAction::Down, x, y);
    click(&mut app, MouseAction::Drag, x2, y);
    click(&mut app, MouseAction::Up, x2, y);
    let s = assert_edited(&app, &s, "c3 _ . e3", "c3 _ _ e3");
    // Un clic en un hueco pone una nota; el derecho la quita.
    let mut screen = draw(&mut app, 120, 40);
    let (x, y) = cell_of(&mut app, &mut screen, 2, E3);
    click(&mut app, MouseAction::Down, x, y);
    click(&mut app, MouseAction::Up, x, y);
    let s = assert_edited(&app, &s, "a2 _ . a2", "a2 _ e3 a2");
    click(&mut app, MouseAction::RightDown, x, y);
    let s = assert_edited(&app, &s, "a2 _ e3 a2", "a2 _ . a2");
    assert_round_trip(&path, &app, &s);
    // La rueda desplaza y Ctrl y la rueda acercan y alejan.
    let before = app.roll_view_for_test().unwrap();
    click(&mut app, MouseAction::ScrollUp, x, y);
    let after = app.roll_view_for_test().unwrap();
    assert_ne!(before.top, after.top);
    click(&mut app, MouseAction::ZoomOut, x, y);
    let view = app.roll_view_for_test().unwrap();
    assert!(!view.zoom_auto);
}

#[test]
fn snap_and_zoom_keys() {
    let (_, mut app, _engine) = roll_on("roll_snap", ROLL_SONG, 0);
    app.roll_cursor_to(0, A2);
    app.handle(special(Key::Right));
    assert_eq!(app.roll_cursor().unwrap().0, (1, A2));
    // La rejilla a 1/8: de dos en dos, a las corcheas.
    app.handle(key('G'));
    app.handle(special(Key::Right));
    assert_eq!(app.roll_cursor().unwrap().0, (2, A2));
    app.handle(special(Key::Right));
    assert_eq!(app.roll_cursor().unwrap().0, (4, A2));
    // Apagada, de paso en paso.
    app.handle(key('g'));
    app.handle(special(Key::Right));
    assert_eq!(app.roll_cursor().unwrap().0, (5, A2));
    // El zoom a mano.
    app.handle(key('Z'));
    let view = app.roll_view_for_test().unwrap();
    assert!(!view.zoom_auto);
    let screen = text(&draw(&mut app, 100, 30));
    assert!(
        screen.contains("1/8") || screen.contains("1/16"),
        "{screen}"
    );
    // Esc suelta las teclas: vuelven al editor.
    app.handle(esc());
    assert!(app.roll_cursor().is_none());
}

// ---------------------------------------------------------------------
// En su ventana.

mod window {
    use std::os::unix::fs::PermissionsExt;
    use std::time::{Duration, Instant};

    use super::super::split::{editor_window, main_window, socket_dir};
    use super::*;
    use crate::link::{self, Conn, FromRoll, Message, ToRoll};
    use crate::roll_window::RollWindow;

    /// La principal (con el editor en su ventana) y el piano roll en la
    /// suya, conectados por sus sockets en el mismo proceso.
    fn three_windows(test: &str, song: &str) -> (PathBuf, App, App, RollWindow, Engine, Engine) {
        let path = fixture(test, Some(song));
        let (mut main, main_engine) = main_window(&path, None);
        main.set_socket_dir(socket_dir(&path));
        let (editor, editor_engine) = editor_window(&path);
        // Una «terminal» que no hace nada: la ventana se abre a mano.
        main.set_roll_terminal(Some(vec!["true".into()]));
        main.run_action(Action::RollWindow);
        let socket = link::roll_socket_path(&socket_dir(&path), &path);
        let roll = RollWindow::new(&socket);
        (path, main, editor, roll, main_engine, editor_engine)
    }

    fn pump(main: &mut App, editor: &mut App, roll: &mut RollWindow, engine: &mut Engine) {
        for _ in 0..4 {
            offline::render(engine, 256, 2);
            main.tick();
            editor.tick();
            roll.tick();
        }
    }

    fn until(
        main: &mut App,
        editor: &mut App,
        roll: &mut RollWindow,
        engine: &mut Engine,
        what: impl Fn(&App, &App, &RollWindow) -> bool,
    ) {
        let deadline = Instant::now() + Duration::from_secs(5);
        while !what(main, editor, roll) {
            assert!(
                Instant::now() < deadline,
                "no llegó a pasar: {} · {}",
                main.view().message,
                screen(roll, 100, 30).lines().last().unwrap_or_default()
            );
            pump(main, editor, roll, engine);
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    fn screen(roll: &mut RollWindow, width: u16, height: u16) -> String {
        let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
        terminal.draw(|frame| roll.draw(frame)).unwrap();
        text(terminal.backend().buffer())
    }

    #[test]
    fn the_roll_in_its_window_edits_the_text_the_editor_window_sees() {
        let (path, mut main, mut editor, mut roll, mut engine, _e) =
            three_windows("roll_window", ROLL_SONG);
        // El socket, solo del usuario.
        let socket = link::roll_socket_path(&socket_dir(&path), &path);
        let mode = std::fs::metadata(&socket).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o600);
        let dir = std::fs::metadata(socket_dir(&path))
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(dir & 0o777, 0o700);
        // Dibujar una vez le dice a la principal su tamaño.
        screen(&mut roll, 100, 30);
        until(&mut main, &mut editor, &mut roll, &mut engine, |_, _, r| {
            r.view().is_some()
        });
        let s = screen(&mut roll, 100, 30);
        assert!(s.contains("piano roll · bajo"), "{s}");
        assert!(
            s.contains("song.lua · conectado con la ventana principal"),
            "{s}"
        );
        // En la principal ya no se ve (está fuera).
        assert!(main.view().roll.is_none());

        // Una tecla en la ventana del piano roll: abre su cursor; el
        // cursor va al hueco del paso 3 y Enter pone una nota.
        roll.handle(special(Key::Right));
        until(&mut main, &mut editor, &mut roll, &mut engine, |_, _, r| {
            r.view().is_some_and(|v| v.cursor.is_some())
        });
        let (col, _) = roll.view().unwrap().cursor.unwrap();
        for _ in col..2 {
            roll.handle(special(Key::Right));
        }
        for _ in 2..col {
            roll.handle(special(Key::Left));
        }
        until(&mut main, &mut editor, &mut roll, &mut engine, |_, _, r| {
            r.view().and_then(|v| v.cursor).is_some_and(|c| c.0 == 2)
        });
        let (_, pitch) = roll.view().unwrap().cursor.unwrap();
        roll.handle(special(Key::Enter));
        let note = crate::roll::tests_note_text(pitch);
        let expected = ROLL_SONG.replacen("a2 _ . a2", &format!("a2 _ {note} a2"), 1);
        until(&mut main, &mut editor, &mut roll, &mut engine, |m, _, _| {
            m.text_for_test() == expected
        });
        // El editor en su ventana lo recibe como un paso de deshacer.
        until(&mut main, &mut editor, &mut roll, &mut engine, |_, e, _| {
            e.text_for_test() == expected
        });
        // Y el piano roll lo enseña.
        until(&mut main, &mut editor, &mut roll, &mut engine, |_, _, r| {
            r.view()
                .is_some_and(|v| v.notes.iter().any(|n| n.col == 2 && n.pitch == pitch))
        });

        // Escribir en el editor deja el piano roll sin editar hasta cargar.
        editor.handle(esc());
        editor.handle(esc());
        editor.editor.insert_str("-- x\n");
        until(&mut main, &mut editor, &mut roll, &mut engine, |m, _, _| {
            m.text_for_test().starts_with("-- x")
        });
        roll.handle(special(Key::Right));
        roll.handle(special(Key::Enter));
        until(&mut main, &mut editor, &mut roll, &mut engine, |_, _, r| {
            r.view().is_some_and(|v| v.status.contains("carga antes"))
        });
        // Sin tocar el texto.
        assert_eq!(main.text_for_test(), format!("-- x\n{expected}"));

        // El ratón en la ventana: Ctrl y la rueda, el zoom.
        roll.mouse(MouseAction::ZoomOut, 20, 10);
        until(&mut main, &mut editor, &mut roll, &mut engine, |_, _, r| {
            r.view().is_some_and(|v| !v.zoom_auto)
        });
        // En varios tamaños.
        for (w, h) in [(60, 16), (100, 30), (160, 50)] {
            let s = screen(&mut roll, w, h);
            assert!(s.contains("piano roll"), "{w}×{h}\n{s}");
        }

        // Volver a meterlo: la ventana se cierra y se ve en la principal.
        main.run_action(Action::RollBack);
        until(&mut main, &mut editor, &mut roll, &mut engine, |_, _, r| {
            r.leaving()
        });
        assert!(!socket.exists(), "el socket se borra");
        assert!(main.view().roll.is_some());
    }

    #[test]
    fn a_roll_of_another_version_is_turned_away() {
        let (path, mut main, _editor, _roll, _engine, _e) =
            three_windows("roll_version", ROLL_SONG);
        let socket = link::roll_socket_path(&socket_dir(&path), &path);
        // La de la prueba se cuela antes: la primera conexión se la queda
        // ella; esta es otra.
        drop(_roll);
        let stream = std::os::unix::net::UnixStream::connect(&socket).unwrap();
        let mut conn = Conn::new(stream).unwrap();
        conn.send(&FromRoll::Hello { version: 5 }).unwrap();
        let deadline = Instant::now() + Duration::from_secs(3);
        let mut got = Vec::new();
        while !got.iter().any(|m| matches!(m, ToRoll::Bye(_))) {
            assert!(Instant::now() < deadline, "{} {got:?}", main.view().message);
            main.tick();
            got.extend(conn.receive::<ToRoll>().unwrap_or_default());
            std::thread::sleep(Duration::from_millis(5));
        }
        assert!(
            main.view().message.contains("otra versión"),
            "{}",
            main.view().message
        );
    }

    #[test]
    fn the_launcher_falls_back_to_the_panel_here() {
        let (path, mut app, _engine) = open("roll_launch", Some(ROLL_SONG));
        app.set_socket_dir(socket_dir(&path));
        app.set_roll_terminal(None);
        app.run_action(Action::RollWindow);
        let message = app.view().message;
        assert!(message.contains("tui --window-roll"), "{message}");
        assert!(app.view().roll.is_some());
        // Una terminal que no existe.
        app.set_roll_terminal(Some(vec!["/no/existe/foot".into()]));
        app.run_action(Action::RollWindow);
        assert!(
            app.view().message.contains("no se pudo lanzar"),
            "{}",
            app.view().message
        );
        assert_eq!(
            crate::app::roll_command(&["foot".into(), "-e".into()], Path::new("/bin/tui"), &path)
                [2..],
            ["/bin/tui", "--window-roll", &path.display().to_string()]
        );
    }

    #[test]
    fn the_messages_round_trip() {
        let view = {
            let (_, app, _e) = roll_on("roll_messages", ROLL_SONG, 0);
            app.roll_view_for_test().unwrap()
        };
        let back = ToRoll::decode(ToRoll::View(view.clone()).encode().trim_end()).unwrap();
        assert_eq!(back, ToRoll::View(view));
        for m in [
            FromRoll::Size {
                width: 100,
                height: 30,
            },
            FromRoll::Key(Input {
                key: Key::Char('V'),
                shift: true,
                ..Input::default()
            }),
            FromRoll::Key(special(Key::PageDown)),
            FromRoll::Mouse {
                action: MouseAction::ZoomIn,
                x: 3,
                y: 4,
            },
        ] {
            assert_eq!(FromRoll::decode(m.encode().trim_end()).unwrap(), m);
        }
        assert!(FromRoll::decode("raton\tvolar\t1\t2").is_err());
    }
}

/// Lo que cuesta el piano roll en cada fotograma, en release:
/// `cargo test --release -p tui coste_roll -- --ignored --nocapture`.
#[test]
#[ignore]
fn coste_roll() {
    let song = r#"bpm(170)
local piano = sample("break.wav"):instrument({ root = "C4" })
track("lead", piano):length(64)
  :melody("c4 _ e4 g4  c5+e5+g5 _ _ .  a4 . c5 .  e5 _ _ _  c4 _ e4 g4  c5+e5+g5 _ _ .  a4 . c5 .  e5 _ _ _  c4 _ e4 g4  c5+e5+g5 _ _ .  a4 . c5 .  e5 _ _ _  c4 _ e4 g4  c5+e5+g5 _ _ .  a4 . c5 .  e5 _ _ _")
  :every(4, function(c) c:pitch(7) end)
  :chance(0.3, "gain", 0.5)
local amen = sample("break.wav"):slice(2)
for i = 1, 7 do
  track("d" .. i, amen):steps("1 . 2 .  1 . 2 .  1 . 2 .  1 . 2 .")
end
"#;
    let (_, mut app, _engine) = roll_on("coste_roll", song, 0);
    let roll = app.current_roll_for_test().unwrap();
    println!(
        "ciclo: {} columnas, {} notas; Column {} B, Note {} B, Step {} B",
        roll.steps(),
        roll.notes.len(),
        std::mem::size_of::<crate::roll::Column>(),
        std::mem::size_of::<crate::roll::Note>(),
        std::mem::size_of::<engine::Step>()
    );
    let n = 300;
    let t = std::time::Instant::now();
    for _ in 0..n {
        std::hint::black_box(app.view());
    }
    let with = t.elapsed() / n;
    let t = std::time::Instant::now();
    for _ in 0..n {
        std::hint::black_box(draw(&mut app, 100, 30));
    }
    let drawn = t.elapsed() / n;
    app.run_action(Action::PianoRoll);
    let t = std::time::Instant::now();
    for _ in 0..n {
        std::hint::black_box(app.view());
    }
    let without = t.elapsed() / n;
    let t = std::time::Instant::now();
    for _ in 0..n {
        std::hint::black_box(draw(&mut app, 100, 30));
    }
    let drawn_without = t.elapsed() / n;
    app.run_action(Action::PianoRoll);
    let view = app.roll_view_for_test().unwrap();
    let line = crate::link::Message::encode(&crate::link::ToRoll::View(view));
    println!(
        "view: {with:?} con el piano roll, {without:?} sin él; dibujar 100×30: {drawn:?} con, {drawn_without:?} sin; vista por el socket: {} bytes",
        line.len()
    );
}
