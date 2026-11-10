//! Cortes independientes en la onda: los huecos y los solapes se ven (con
//! palabras y en ASCII), la geometría dice qué borde se agarra, y un clic
//! derecho en un hueco pone un corte ahí.

use super::*;
use crate::app::MouseAction;
use crate::ui::{WaveHit, WaveLayout};
use ratatui::layout::Rect;

/// break.wav (6000 frames, 125 ms): un hueco entre el 1 y el 2, y el 3 se
/// solapa con el 2.
const GAPS: &str = r#"bpm(120)
local amen = sample("break.wav"):slice({ {0, 40}, {60, 100}, {90, 125} })
track("drums", amen):steps("1 . 2 .  3 . . .  . . . .  . . . .")
"#;

#[test]
fn the_layout_says_which_edge_is_grabbed() {
    // 100 columnas para 1000 frames: 10 frames por columna.
    let layout = WaveLayout::new(Rect::new(10, 5, 100, 4), 0..1000);
    let slices = vec![0..300, 300..600, 700..1000];
    let (x, y) = layout.point_of(300);
    assert_eq!((x, y), (40, 6));
    // En la celda de la marca o a su derecha, el inicio del 2; a su
    // izquierda, el final del 1.
    assert_eq!(
        layout.hit(x, &slices, None),
        WaveHit::Edge {
            slice: 1,
            end: false
        }
    );
    assert_eq!(
        layout.hit(x + 1, &slices, None),
        WaveHit::Edge {
            slice: 1,
            end: false
        }
    );
    assert_eq!(
        layout.hit(x - 1, &slices, None),
        WaveHit::Edge {
            slice: 0,
            end: true
        }
    );
    // Aunque el 1 esté elegido.
    assert_eq!(
        layout.hit(x, &slices, Some(0)),
        WaveHit::Edge {
            slice: 1,
            end: false
        }
    );
    // Lejos de los bordes, el cuerpo; en el hueco, nada.
    assert_eq!(layout.hit(55, &slices, None), WaveHit::Body { slice: 1 });
    assert_eq!(layout.hit(75, &slices, None), WaveHit::Gap);
    // Un final suelto se agarra solo.
    assert_eq!(
        layout.hit(70, &slices, None),
        WaveHit::Edge {
            slice: 1,
            end: true
        }
    );
    // Solapes: el elegido si lo cubre; si no, el que empieza más tarde.
    let overlapping = vec![0..600, 200..800];
    assert_eq!(
        layout.hit(50, &overlapping, None),
        WaveHit::Body { slice: 1 }
    );
    assert_eq!(
        layout.hit(50, &overlapping, Some(0)),
        WaveHit::Body { slice: 0 }
    );
}

#[test]
fn gaps_and_overlaps_show_in_words_and_in_ascii() {
    let (_, mut app, _engine) = open("slice_edit_draw", Some(GAPS));
    let screen = text(&draw(&mut app, 160, 40));
    assert!(screen.contains("1 hueco · 1 solape"), "{screen}");
    assert!(screen.contains("corte 1 0.0–40.0 ms"), "{screen}");
    // El final suelto del 1 lleva `]`, en la fila de los números.
    let layout = app.wave_layout().unwrap();
    let buffer = draw(&mut app, 160, 40);
    let (x, _) = layout.point_of(1919);
    assert_eq!(buffer[(x, layout.area.y)].symbol(), "]");
    // En ASCII, lo mismo sin un solo carácter fuera de ASCII.
    crate::theme::set_ascii(true);
    let buffer = draw(&mut app, 160, 40);
    crate::theme::set_ascii(false);
    let area = layout.area;
    for y in area.y..area.bottom() {
        for x in area.x..area.right() {
            let symbol = buffer[(x, y)].symbol();
            assert!(symbol.is_ascii(), "«{symbol}» en {x},{y}");
        }
    }
    assert!(text(&buffer).contains("1 hueco"));
}

#[test]
fn a_right_click_in_a_gap_puts_a_slice_there() {
    let (_, mut app, _engine) = open("slice_edit_gap", Some(GAPS));
    draw(&mut app, 160, 40);
    let layout = app.wave_layout().unwrap();
    // 50 ms: en el hueco entre el 1 (hasta 40) y el 2 (desde 60).
    let (x, y) = layout.point_of(2400);
    app.mouse(MouseAction::RightDown, x, y);
    let view = app.view();
    assert_eq!(view.tracks[0].slices.len(), 4, "{}", view.message);
    let new = view.tracks[0].slices[1].clone();
    assert!(new.start.abs_diff(2400) <= 40 && new.end == 2880, "{new:?}");
    // Los demás, como estaban.
    assert_eq!(view.tracks[0].slices[0], 0..1920);
    assert_eq!(view.tracks[0].slices[2], 2880..4800);
    assert_eq!(view.tracks[0].slices[3], 4320..6000);
}
