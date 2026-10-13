//! Tap tempo desde la paleta.

use std::time::{Duration, Instant};

use super::*;

#[test]
fn tap_tempo_averages_the_taps_and_writes_bpm() {
    let (_, mut app, _engine) = open("tap", Some(SONG));
    let t0 = Instant::now();
    app.start_tap(t0);
    assert!(app.view().message.starts_with("tap tempo"));
    for k in 1..=4 {
        app.tap_at(t0 + Duration::from_millis(350 * k));
    }
    assert!(
        app.view().message.contains("171.4 BPM (5 golpes)"),
        "{}",
        app.view().message
    );
    app.handle(special(Key::Enter));
    assert!(app.text_for_test().starts_with("bpm(171.4)\n"));
    // Esc cancela sin tocar nada.
    app.start_tap(t0);
    app.handle(esc());
    assert_eq!(app.view().message, "tap tempo cancelado");
    assert!(!app.view().playing_mode);
}
