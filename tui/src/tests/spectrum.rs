//! La ventana del espectro estéreo, del lado de la principal: calcula y
//! manda solo con una ventana conectada, y al irse la ventana lo apaga
//! todo.

use std::time::{Duration, Instant};

use super::split::socket_dir;
use super::*;
use crate::link::{self, Conn, FromSpectrum, Message, ToSpectrum};
use crate::palette::Action;

/// Un tono de 1 kHz en bucle en una pista con `pan`.
fn open_panned(test: &str, pan: f32) -> (PathBuf, App, Engine) {
    let path = fixture(test, None);
    let dir = path.parent().unwrap();
    let tone: Vec<f32> = (0..96_000)
        .map(|i| 0.6 * (std::f32::consts::TAU * 1000.0 * i as f32 / RATE as f32).sin())
        .collect();
    offline::write_wav(dir.join("tono.wav"), &tone, 1, RATE).unwrap();
    std::fs::write(
        &path,
        format!(
            "bpm(120)\nlocal t = sample(\"tono.wav\"):slice_grid(1)\n\
             t:set(1, {{ play = \"loop\" }})\n\
             track(\"tono\", t):steps(\"1 . . .  . . . .  . . . .  . . . .\"):pan({pan})\n"
        ),
    )
    .unwrap();
    let (controller, engine) = new_engine(RATE);
    let mut app = App::open(&path, controller, RATE).unwrap();
    assert!(app.view().error.is_none(), "{:?}", app.view().error);
    app.set_socket_dir(socket_dir(&path));
    // Una «terminal» que no hace nada: la ventana se abre a mano.
    app.set_terminals(Some(vec!["true".into()]), None);
    (path, app, engine)
}

/// Una ventana «cruda»: solo el enlace, para ver lo que manda la
/// principal.
fn raw_window(path: &Path) -> Conn {
    let socket = link::spectrum_socket_path(&socket_dir(path), path);
    let stream = std::os::unix::net::UnixStream::connect(&socket).unwrap();
    let mut conn = Conn::new(stream).unwrap();
    conn.send(&FromSpectrum::Hello {
        version: link::VERSION,
    })
    .unwrap();
    conn
}

#[test]
fn the_main_sends_small_reduced_frames_only_to_a_connected_window() {
    let (path, mut app, mut engine) = open_panned("espectro_crudo", 1.0);
    app.run_action(Action::SpectrumWindow);
    let mut conn = raw_window(&path);
    let mut got = Vec::new();
    let deadline = Instant::now() + Duration::from_secs(10);
    let frames = |got: &[ToSpectrum]| {
        got.iter()
            .filter_map(|m| match m {
                ToSpectrum::Frame(f) => Some(f.clone()),
                _ => None,
            })
            .collect::<Vec<_>>()
    };
    while frames(&got).len() < 12 {
        assert!(Instant::now() < deadline, "{}", app.view().message);
        offline::render(&mut engine, 800, 2);
        app.tick();
        got.extend(conn.receive::<ToSpectrum>().unwrap());
    }
    // Primero el saludo, con la ruta canónica del script.
    assert!(
        matches!(&got[0], ToSpectrum::Welcome { path: p, title, .. }
            if *p == crate::target::full_path(&path) && title == "song.lua"),
        "{:?}",
        got[0]
    );
    // Cuadros de tamaño fijo: las bandas de cada canal, menos de 1 KB.
    let last = frames(&got).pop().unwrap();
    assert_eq!(last.bands(), crate::stereo::BANDS);
    assert_eq!(last.rate, RATE);
    assert!(ToSpectrum::Frame(last.clone()).encode().len() < 1024);
    // El tono, panoteado a la derecha.
    let band = (0..last.bands())
        .max_by(|&a, &b| last.energy(a).total_cmp(&last.energy(b)))
        .unwrap();
    assert!(last.pan[band] > 0.95, "{}", last.pan[band]);
    assert!(last.left[band] < last.right[band]);
    // La ventana se va: la principal deja de escuchar, de calcular y de
    // copiar.
    drop(conn);
    while app.spectrum_listening() {
        assert!(Instant::now() < deadline);
        offline::render(&mut engine, 800, 2);
        app.tick();
    }
    assert_eq!(app.spectrum_cost(), (0, 0, 0));
}

#[test]
fn the_menu_and_the_palette_open_and_close_it() {
    use crate::menu::{Menu, menu_of};
    assert_eq!(menu_of(Action::SpectrumWindow), Some(Menu::Windows));
    assert_eq!(menu_of(Action::SpectrumBack), Some(Menu::Windows));
    let (path, mut app, _engine) = open_panned("espectro_menu", 0.0);
    app.set_terminals(None, None);
    app.run_action(Action::SpectrumWindow);
    assert!(app.view().message.contains("tui --window-spectrum"));
    assert!(app.spectrum_listening() && !app.spectrum_ready());
    app.run_action(Action::SpectrumBack);
    assert!(!app.spectrum_listening());
    let command = crate::app::spectrum_command(&["foot".into()], Path::new("/bin/tui"), &path);
    assert_eq!(command[1..3], ["/bin/tui", "--window-spectrum"]);
}
