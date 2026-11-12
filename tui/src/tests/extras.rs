//! Las comodidades del editor: la pareja del paréntesis resaltada, el
//! cierre de pares, las guías de sangría y los números relativos.

use super::*;
use crate::config::Config;

#[test]
fn the_pair_of_the_bracket_at_the_cursor_is_highlighted() {
    let (_, mut app, _engine) = open("extras_pair", Some(SONG));
    // SONG: la línea 1 es `bpm(120)`; el cursor sobre su `(`.
    app.editor
        .move_cursor(ratatui_textarea::CursorMove::Jump(0, 3));
    let buffer = draw(&mut app, 120, 40);
    let (x, y) = find(&buffer, "bpm(120)").unwrap();
    assert_eq!(buffer[(x + 3, y)].bg, crate::ui::accent());
    assert_eq!(buffer[(x + 7, y)].bg, crate::ui::accent());
    assert_ne!(buffer[(x + 4, y)].bg, crate::ui::accent());
    // Apagado: nada.
    app.set_config(&Config::parse("[editor]\npareja = false\n", true, "c.toml"));
    let buffer = draw(&mut app, 120, 40);
    assert_ne!(buffer[(x + 7, y)].bg, crate::ui::accent());
}
