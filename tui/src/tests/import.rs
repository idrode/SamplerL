//! Las opciones de `sample(ruta, { ... })` en el autocompletado y la ayuda.

use super::*;

#[test]
fn sample_options_complete_and_explain_themselves() {
    let (mut app, _engine) = open_at_end("import_complete");
    type_text(&mut app, "local s = sample(\"break.wav\", { ");
    app.handle(f(4));
    assert_eq!(
        completion_items(&app),
        ["trim", "normalize", "fade_in", "fade_out"]
    );
    app.handle(esc());
    assert_eq!(app.view().help.unwrap().signature, "sample(ruta, { ... })");
    type_text(&mut app, "trim = true })");
    app.handle(ctrl('s'));
    assert!(app.view().error.is_none(), "{:?}", app.view().error);
}
