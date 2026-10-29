//! El panel de dispositivos: dibujado en varios tamaños, encima de la
//! principal desde la paleta y en su propia ventana por el socket de verdad
//! (los dos extremos en este proceso), con lanzadores de terminal falsos.

use std::path::Path;
use std::time::{Duration, Instant};

use ratatui::Terminal;
use ratatui::backend::TestBackend;

use super::devices::{SEABOARD, with_devices};
use super::split::socket_dir;
use super::*;
use crate::devices::{DevicesView, Entry, Kind, VIRTUAL_IN};
use crate::devices_panel::{Cursor, DevicesPanel};
use crate::devices_window::DevicesWindow;
use crate::link::{self, Conn, FromDevices, Message, ToDevices};
use crate::palette::Action;

fn esc() -> Input {
    Input {
        key: Key::Esc,
        ..Input::default()
    }
}

fn press(key: Key) -> Input {
    Input {
        key,
        ..Input::default()
    }
}

fn sample_view() -> DevicesView {
    let entry = |name: &str, detail: &str, active, default| Entry {
        name: name.into(),
        detail: detail.into(),
        active,
        default,
    };
    DevicesView {
        audio: vec![
            entry("Altavoces", "2 canales · 44100, 48000 Hz", false, true),
            entry(
                "Auriculares USB",
                "2 canales · 48000, 96000 Hz",
                true,
                false,
            ),
            entry(
                "WH-1000XM4 (Bluetooth)",
                "2 canales · 48000 Hz",
                false,
                false,
            ),
        ],
        midi_in: vec![
            entry(VIRTUAL_IN, "se conecta con aconnect", false, true),
            entry("Seaboard:Seaboard MIDI 1", "", true, false),
        ],
        midi_out: vec![],
        status: "«Auriculares USB» · 48000 Hz · 2 canales · latencia ≈ 10.7 ms".into(),
        notes: [
            String::new(),
            String::new(),
            "MIDI apagado: abre con --allow-midi".into(),
        ],
        activity: 3,
        notice: "audio: suena por «Auriculares USB»".into(),
    }
}

#[test]
fn the_panel_draws_in_any_size() {
    let view = sample_view();
    let panel = |cursor, lit| DevicesPanel {
        view: &view,
        cursor,
        lit,
        title: "dispositivos",
    };
    let cursor = Cursor {
        kind: Kind::Audio,
        row: 2,
    };
    // Una debajo de otra.
    let screen = text(&draw_widget(panel(cursor, true), 90, 24));
    for needle in [
        "Audio de salida",
        "MIDI de entrada",
        "MIDI de salida",
        "▸  WH-1000XM4 (Bluetooth)",
        " ● Auriculares USB",
        "predeterminado · 2 canales · 44100, 48000 Hz",
        "● nota",
        "MIDI apagado: abre con --allow-midi",
        "latencia ≈ 10.7 ms",
        "audio: suena por «Auriculares USB»",
        "r refrescar",
    ] {
        assert!(screen.contains(needle), "falta «{needle}»:\n{screen}");
    }
    let audio = find(&draw_widget(panel(cursor, true), 90, 24), "Audio de salida").unwrap();
    let midi = find(&draw_widget(panel(cursor, true), 90, 24), "MIDI de entrada").unwrap();
    assert_eq!(audio.0, midi.0);
    assert!(midi.1 > audio.1);
    // Apagado, el indicador se ve vacío.
    let screen = text(&draw_widget(panel(cursor, false), 90, 24));
    assert!(
        screen.contains("○ nota") && !screen.contains("● nota"),
        "{screen}"
    );
    // Ancho: una al lado de otra.
    let buffer = draw_widget(panel(cursor, true), 150, 20);
    let audio = find(&buffer, "Audio de salida").unwrap();
    let midi = find(&buffer, "MIDI de entrada").unwrap();
    assert_eq!(audio.1, midi.1, "{}", text(&buffer));
    assert!(midi.0 > audio.0);
    // Bajo: la lista con el cursor se desplaza para que se vea.
    let screen = text(&draw_widget(panel(cursor, true), 60, 10));
    assert!(screen.contains("WH-1000XM4"), "{screen}");
    // Diminuto: no se rompe.
    for (w, h) in [(1, 1), (10, 3), (24, 6), (40, 4)] {
        draw_widget(panel(cursor, true), w, h);
    }
    // Vacío (sin principal todavía).
    let empty = DevicesView::default();
    let screen = text(&draw_widget(
        DevicesPanel {
            view: &empty,
            cursor: Cursor::default(),
            lit: false,
            title: "dispositivos",
        },
        60,
        12,
    ));
    assert!(screen.contains("Audio de salida"), "{screen}");
}

#[test]
fn the_palette_opens_the_panel_over_the_main_window() {
    let (mut app, audio, midi) = with_devices("panel_popup");
    app.run_action(Action::Devices);
    let screen = text(&draw(&mut app, 120, 40));
    assert!(screen.contains("dispositivos"), "{screen}");
    assert!(
        screen.contains("▸● Altavoces"),
        "el cursor en el que suena:\n{screen}"
    );
    // ↓ Enter: los auriculares.
    app.handle(press(Key::Down));
    app.handle(press(Key::Enter));
    for _ in 0..20 {
        audio.pump("Altavoces", 256);
        app.tick();
    }
    let screen = text(&draw(&mut app, 120, 40));
    assert!(screen.contains("▸● Auriculares USB"), "{screen}");
    assert!(
        screen.contains("audio: suena por «Auriculares USB»"),
        "{screen}"
    );
    // → la entrada MIDI: el cursor en la que está en uso (la virtual).
    app.handle(press(Key::Right));
    app.handle(press(Key::Down));
    app.handle(press(Key::Enter));
    assert_eq!(midi.0.borrow().input, Some(Some(SEABOARD.into())));
    // Llega una nota: el indicador se enciende.
    midi.receive(&[0x90, 36, 100]);
    app.poll_midi();
    app.tick();
    let screen = text(&draw(&mut app, 120, 40));
    assert!(screen.contains("● nota"), "{screen}");
    // r refresca; Esc cierra.
    app.handle(key('r'));
    assert!(app.view().message.starts_with("dispositivos: lista al día"));
    app.handle(esc());
    assert!(app.view().devices.is_none());
    // Sin dispositivos (el editor en su ventana, o un test), lo dice.
    let (_, mut bare, _engine) = open("panel_bare", Some(SONG));
    bare.run_action(Action::Devices);
    assert!(bare.view().devices.is_none());
    assert!(bare.view().message.contains("ventana principal"));
}

/// Una vuelta de las dos ventanas.
fn pump(main: &mut App, window: &mut DevicesWindow) {
    for _ in 0..4 {
        main.tick();
        window.tick();
    }
}

fn pump_until(
    main: &mut App,
    window: &mut DevicesWindow,
    what: impl Fn(&App, &DevicesWindow) -> bool,
) {
    let deadline = Instant::now() + Duration::from_secs(3);
    while !what(main, window) {
        assert!(Instant::now() < deadline, "no llegó a pasar");
        pump(main, window);
        std::thread::sleep(Duration::from_millis(5));
    }
}

fn window_screen(window: &DevicesWindow, width: u16, height: u16) -> String {
    let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
    terminal.draw(|frame| window.draw(frame)).unwrap();
    text(terminal.backend().buffer())
}

/// La principal con dispositivos falsos, escuchando al panel en su ventana
/// con una «terminal» que no hace nada (el panel se abre a mano).
fn main_with_panel(test: &str) -> (App, engine::device::fake::FakeAudio, PathBuf) {
    let (mut app, audio, _midi) = with_devices(test);
    let path = app.script_path().to_path_buf();
    app.set_socket_dir(socket_dir(&path));
    app.set_terminals(None, Some(vec!["true".into()]));
    app.run_action(Action::DevicesWindow);
    let socket = link::devices_socket_path(&socket_dir(&path), &path);
    (app, audio, socket)
}

#[test]
fn the_panel_in_its_window_lists_and_chooses_through_the_main_window() {
    let (mut main, audio, socket) = main_with_panel("panel_window");
    assert!(
        main.view().message.starts_with("abriendo el panel"),
        "{}",
        main.view().message
    );
    let mut window = DevicesWindow::new(&socket);
    pump_until(&mut main, &mut window, |_, w| {
        window_screen(w, 100, 30).contains("Auriculares USB")
    });
    assert_eq!(main.view().message, "panel de dispositivos en su ventana");
    let screen = window_screen(&window, 100, 30);
    assert!(screen.contains("dispositivos · song.lua"), "{screen}");
    assert!(
        screen.contains("conectado con la ventana principal"),
        "{screen}"
    );
    assert!(screen.contains("▸● Altavoces"), "{screen}");
    // En la principal, «Dispositivos» dice que está fuera.
    main.run_action(Action::Devices);
    assert!(main.view().devices.is_none());
    assert!(main.view().message.contains("en su ventana"));

    // ↓ Enter en la ventana: la principal cambia de dispositivo.
    window.handle(press(Key::Down));
    window.handle(press(Key::Enter));
    pump_until(&mut main, &mut window, |m, _| {
        m.view().message.starts_with("audio: cambiando")
    });
    for _ in 0..20 {
        audio.pump("Altavoces", 256);
        main.tick();
    }
    pump_until(&mut main, &mut window, |_, w| {
        window_screen(w, 100, 30).contains("▸● Auriculares USB")
    });
    assert_eq!(audio.open_streams(), ["Auriculares USB"]);
    // r: refresca en la principal.
    window.handle(key('r'));
    pump_until(&mut main, &mut window, |m, _| {
        m.view().message.starts_with("dispositivos: lista al día")
    });
    // Se ve en varios tamaños.
    for (w, h) in [(40, 10), (80, 24), (160, 40)] {
        let screen = window_screen(&window, w, h);
        assert!(screen.contains("dispositivos"), "{w}x{h}:\n{screen}");
    }

    // «Volver a meterlo en la principal»: la ventana se cierra y el panel
    // se abre en la principal.
    main.run_action(Action::DevicesBack);
    assert!(main.view().devices.is_some());
    pump_until(&mut main, &mut window, |_, w| w.leaving());
    assert!(!socket.exists(), "el socket se borra");
}

#[test]
fn closing_the_window_keeps_the_main_going_and_it_can_come_back() {
    let (mut main, _audio, socket) = main_with_panel("panel_close");
    let mut window = DevicesWindow::new(&socket);
    pump_until(&mut main, &mut window, |_, w| {
        window_screen(w, 80, 24).contains("Altavoces")
    });
    assert_eq!(window.handle(ctrl('q')), Flow::Quit);
    drop(window);
    let deadline = Instant::now() + Duration::from_secs(3);
    while !main
        .view()
        .message
        .starts_with("el panel de dispositivos se cerró")
    {
        assert!(Instant::now() < deadline, "{}", main.view().message);
        main.tick();
        std::thread::sleep(Duration::from_millis(5));
    }
    // Otra vez: se conecta al mismo socket.
    let mut window = DevicesWindow::new(&socket);
    pump_until(&mut main, &mut window, |_, w| {
        window_screen(w, 80, 24).contains("Altavoces")
    });
}

#[test]
fn a_panel_of_another_version_is_turned_away() {
    let (mut main, _audio, socket) = main_with_panel("panel_version");
    let stream = std::os::unix::net::UnixStream::connect(&socket).unwrap();
    let mut conn = Conn::new(stream).unwrap();
    conn.send(&FromDevices::Hello { version: 3 }).unwrap();
    let deadline = Instant::now() + Duration::from_secs(3);
    let mut got = Vec::new();
    while !got.iter().any(|m| matches!(m, ToDevices::Bye(_))) {
        assert!(Instant::now() < deadline);
        main.tick();
        got.extend(conn.receive::<ToDevices>().unwrap_or_default());
        std::thread::sleep(Duration::from_millis(5));
    }
    assert!(matches!(&got[0], ToDevices::Welcome { version, .. } if *version == link::VERSION));
    assert_eq!(
        main.view().message,
        "un panel de dispositivos de otra versión: rechazado"
    );
    // Lo que manda la principal se entiende al revés también.
    assert!(ToDevices::decode(got.last().unwrap().encode().trim_end()).is_ok());
}

#[test]
fn the_launcher_uses_its_terminal_and_falls_back_to_the_popup() {
    // Una «terminal» que apunta sus argumentos en un archivo.
    let (mut app, _audio, _midi) = with_devices("panel_launch");
    let path = app.script_path().to_path_buf();
    app.set_socket_dir(socket_dir(&path));
    let out = path.with_file_name("lanzado.txt");
    let terminal = vec![
        "sh".into(),
        "-c".into(),
        format!("printf '%s\\n' \"$@\" > '{}'", out.display()),
        "sh".into(),
    ];
    assert_eq!(
        crate::app::devices_command(&terminal, Path::new("/bin/tui"), &path)[4..],
        ["/bin/tui", "--window-devices", &path.display().to_string()]
    );
    // Sin [dispositivos] terminal, la del editor.
    app.set_terminals(Some(terminal), None);
    app.run_action(Action::DevicesWindow);
    let deadline = Instant::now() + Duration::from_secs(5);
    while std::fs::read_to_string(&out).map_or(true, |t| t.lines().count() < 3) {
        assert!(Instant::now() < deadline, "no se lanzó");
        std::thread::sleep(Duration::from_millis(20));
    }
    let args = std::fs::read_to_string(&out).unwrap();
    assert!(
        args.ends_with(&format!("--window-devices\n{}\n", path.display())),
        "{args}"
    );
    // La «terminal» se cerró sin que se conectara nadie: el panel, aquí.
    let deadline = Instant::now() + Duration::from_secs(5);
    while app.view().devices.is_none() {
        assert!(Instant::now() < deadline, "no volvió al panel de aquí");
        app.tick();
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(
        app.view().message.contains("se cerró sin abrirlo"),
        "{}",
        app.view().message
    );
    app.handle(esc());

    // Una terminal que no existe: error claro y el panel aquí.
    app.set_terminals(None, Some(vec!["/no/existe/kitty".into(), "-e".into()]));
    app.run_action(Action::DevicesWindow);
    let message = app.view().message;
    assert!(
        message.contains("no se pudo lanzar «/no/existe/kitty»"),
        "{message}"
    );
    assert!(app.view().devices.is_some());
    app.handle(esc());

    // Sin ninguna terminal configurada: cómo abrirlo a mano, y aquí.
    app.set_terminals(None, None);
    app.run_action(Action::DevicesWindow);
    let message = app.view().message;
    assert!(message.contains("tui --window-devices"), "{message}");
    assert!(app.view().devices.is_some());
}

/// Para ver el panel sin terminal:
/// `cargo test -p tui pantalla_dispositivos -- --ignored --nocapture`.
#[test]
#[ignore]
fn pantalla_dispositivos() {
    let view = sample_view();
    let cursor = Cursor {
        kind: Kind::MidiIn,
        row: 1,
    };
    for (w, h) in [(90, 22), (150, 16)] {
        let panel = DevicesPanel {
            view: &view,
            cursor,
            lit: true,
            title: "dispositivos · song.lua",
        };
        println!("{}\n", text(&draw_widget(panel, w, h)));
    }
}
