//! El botón de pánico desde la interfaz: F12 silencia con cualquier cosa
//! abierta, Shift-F12 pone y quita la pausa, se ve en el transporte y
//! las teclas y el límite de voces se configuran.

use super::*;
use crate::config::Config;

fn shift_f12() -> Input {
    Input {
        key: Key::F(12),
        shift: true,
        ..Input::default()
    }
}

/// Lo que suena en los próximos 200 ms (el motor también avanza).
fn heard(engine: &mut Engine) -> bool {
    offline::render(engine, RATE as usize / 5, 2)
        .iter()
        .any(|&s| s != 0.0)
}

#[test]
fn f12_silences_everything_even_with_the_palette_open() {
    let (_, mut app, mut engine) = open("panic_now", Some(SONG));
    // En marcha (SONG toca en los pasos 0, 2, 4...).
    assert!(heard(&mut engine));
    app.open_palette();
    app.handle(special(Key::F(12)));
    assert!(app.view().palette.is_some(), "la paleta sigue abierta");
    assert_eq!(
        app.view().message,
        "pánico: silencio; lo siguiente entra como siempre"
    );
    // El fundido mínimo y nada más hasta el paso siguiente: con el
    // pánico al principio de un trozo, los 3 ms siguientes callan.
    let after = offline::render(&mut engine, 256, 2);
    assert!(after[2 * 100..].iter().all(|&s| s == 0.0));
    assert_eq!(engine.sounding_voices(), 0);
    // Lo siguiente entra: el secuenciador vuelve a sonar.
    assert!(heard(&mut engine));
}

#[test]
fn shift_f12_pauses_until_pressed_again() {
    let (_, mut app, mut engine) = open("panic_pause", Some(SONG));
    assert!(heard(&mut engine));
    app.handle(esc());
    app.handle(shift_f12());
    assert!(
        app.view().message.starts_with("PÁNICO en pausa"),
        "{}",
        app.view().message
    );
    // Un compás entero (2 s) en silencio, aunque se toquen cortes.
    for _ in 0..10 {
        app.handle(key('1'));
        assert!(!heard(&mut engine), "en pausa no suena nada");
    }
    app.tick();
    let screen = text(&draw(&mut app, 140, 40));
    assert!(screen.contains("PÁNICO: pausa"), "{screen}");
    // F12 en pausa la deja puesta.
    app.handle(special(Key::F(12)));
    assert_eq!(
        app.view().message,
        "pánico: silencio; sigue la pausa · Shift-F12 reanuda"
    );
    app.handle(key('1'));
    assert!(!heard(&mut engine));
    // Shift-F12 otra vez: reanuda.
    app.handle(shift_f12());
    assert_eq!(app.view().message, "pánico: reanudado, vuelve a sonar");
    app.handle(key('1'));
    assert!(heard(&mut engine));
    app.tick();
    let screen = text(&draw(&mut app, 140, 40));
    assert!(!screen.contains("PÁNICO"), "{screen}");
}

#[test]
fn the_panic_keys_and_the_voice_limit_come_from_the_config() {
    let (_, mut app, mut engine) = open("panic_config", Some(SONG));
    let config = Config::parse(
        "[teclas]\npanico = \"alt-p\"\n\n[voces]\nlimite = 8\n",
        true,
        "config.toml",
    );
    assert!(config.warnings.is_empty(), "{:?}", config.warnings);
    assert_eq!(config.voices.limit, 8);
    app.set_config(&config);
    assert!(heard(&mut engine));
    app.handle(Input {
        key: Key::Char('p'),
        alt: true,
        ..Input::default()
    });
    assert!(app.view().message.starts_with("pánico: silencio"));

    // Por defecto, 64; fuera de rango, aviso.
    assert_eq!(Config::default().voices.limit, engine::DEFAULT_VOICE_LIMIT);
    for bad in ["limite = 0", "limite = 300", "limite = \"x\"", "otra = 3"] {
        let config = Config::parse(&format!("[voces]\n{bad}\n"), true, "c.toml");
        assert_eq!(config.voices.limit, 64);
        assert_eq!(config.warnings.len(), 1, "{bad}");
        assert!(config.warnings[0].starts_with("[voces]"), "{bad}");
    }
}
