//! La ventana del espectro estéreo: la principal calcula y manda solo con
//! la ventana conectada; un tono panoteado cae a su lado; el modo espejo;
//! cerrar la ventana lo apaga todo; versiones y scripts que no casan; y el
//! dibujo en varios tamaños y en ASCII.

use std::time::{Duration, Instant};

use super::split::socket_dir;
use super::*;
use crate::link::{self, Conn, FromSpectrum, Message, Server, ToSpectrum};
use crate::palette::Action;
use crate::spectrum_window::{Leaving, SpectrumWindow};
use crate::viz::stereo::Mode;

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

/// La ventana de ese script, conectada al socket de su principal.
fn window_for(path: &Path, mode: Mode) -> SpectrumWindow {
    let socket = link::spectrum_socket_path(&socket_dir(path), path);
    SpectrumWindow::new(&socket, &crate::target::full_path(path), mode, None)
}

fn pump(app: &mut App, engine: &mut Engine, window: &mut SpectrumWindow) {
    for _ in 0..4 {
        offline::render(engine, 800, 2);
        app.tick();
        window.tick();
    }
}

fn until(
    app: &mut App,
    engine: &mut Engine,
    window: &mut SpectrumWindow,
    what: impl Fn(&App, &SpectrumWindow) -> bool,
) {
    let deadline = Instant::now() + Duration::from_secs(5);
    while !what(app, window) {
        assert!(
            Instant::now() < deadline,
            "no llegó a pasar: {}",
            app.view().message
        );
        pump(app, engine, window);
        std::thread::sleep(Duration::from_millis(5));
    }
}

fn screen(window: &mut SpectrumWindow, width: u16, height: u16) -> String {
    let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
    terminal.draw(|frame| window.draw(frame)).unwrap();
    text(terminal.backend().buffer())
}

/// La banda más fuerte del último cuadro que ha visto la ventana, y su
/// paneo.
fn loudest_pan(window: &SpectrumWindow) -> f32 {
    let frame = window.seen().0.expect("un cuadro");
    let band = (0..frame.bands())
        .max_by(|&a, &b| frame.energy(a).total_cmp(&frame.energy(b)))
        .unwrap();
    frame.pan[band]
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
fn a_tone_panned_left_right_or_centred_lands_on_its_side() {
    for (name, pan, want) in [
        ("izq", -1.0, -1.0f32),
        ("der", 1.0, 1.0),
        ("centro", 0.0, 0.0),
    ] {
        let (path, mut app, mut engine) = open_panned(&format!("espectro_pan_{name}"), pan);
        app.run_action(Action::SpectrumWindow);
        let mut window = window_for(&path, Mode::Field);
        until(&mut app, &mut engine, &mut window, |a, _| {
            a.spectrum_ready()
        });
        // Unos cuadros (el paneo que se ve se acerca poco a poco).
        until(&mut app, &mut engine, &mut window, |a, _| {
            a.spectrum_cost().0 >= 10
        });
        pump(&mut app, &mut engine, &mut window);
        let got = loudest_pan(&window);
        assert!((got - want).abs() < 0.05, "{name}: {got}");
        let s = screen(&mut window, 80, 24);
        assert!(s.contains("espectro estéreo · campo · song.lua"), "{s}");
        assert!(
            s.contains('L') && s.contains('R') && s.contains("1k"),
            "{s}"
        );
    }
}

#[test]
fn only_with_the_window_open_the_engine_copies_and_the_main_computes() {
    let (path, mut app, mut engine) = open_panned("espectro_coste", -1.0);
    // Sin ventana: nada.
    for _ in 0..40 {
        offline::render(&mut engine, 800, 2);
        app.tick();
    }
    assert!(!app.spectrum_listening());
    assert_eq!(app.spectrum_cost(), (0, 0, 0));
    // Escuchando pero sin ventana conectada: tampoco.
    app.run_action(Action::SpectrumWindow);
    assert!(app.spectrum_listening());
    for _ in 0..40 {
        offline::render(&mut engine, 800, 2);
        app.tick();
    }
    assert!(!app.spectrum_ready());
    assert_eq!(app.spectrum_cost(), (0, 0, 0));
    // Con la ventana: a su ritmo (`visuales_hz`, 15): unos 15 cuadros por
    // segundo de audio, dos FFT cada uno.
    let mut window = window_for(&path, Mode::Field);
    until(&mut app, &mut engine, &mut window, |a, _| {
        a.spectrum_ready()
    });
    let (sent0, _, ffts0) = app.spectrum_cost();
    for _ in 0..60 {
        offline::render(&mut engine, 800, 2);
        app.tick();
        window.tick();
    }
    // 60 × 800 = 48 000 muestras: un segundo.
    let (sent, skipped, ffts) = app.spectrum_cost();
    assert!(
        (13..=16).contains(&(sent + skipped - sent0)),
        "{sent} {skipped}"
    );
    assert_eq!(ffts - ffts0, 2 * (sent - sent0));
    // Abrirla otra vez no duplica: avisa.
    app.run_action(Action::SpectrumWindow);
    assert!(app.view().message.contains("ya está abierta"));
    // La ventana se cierra (q): la principal deja de calcular y apaga la
    // cola del motor.
    assert_eq!(window.handle(key('q')), Flow::Quit);
    drop(window);
    let deadline = Instant::now() + Duration::from_secs(5);
    while app.spectrum_listening() {
        assert!(Instant::now() < deadline);
        offline::render(&mut engine, 800, 2);
        app.tick();
    }
    assert!(
        app.view().message.contains("cerrada"),
        "{}",
        app.view().message
    );
    assert_eq!(app.spectrum_cost(), (0, 0, 0));
    let socket = link::spectrum_socket_path(&socket_dir(&path), &path);
    assert!(!socket.exists(), "el socket se borra");
}

#[test]
fn closing_from_the_main_closes_the_window_and_a_dead_main_is_noticed() {
    let (path, mut app, mut engine) = open_panned("espectro_cerrar", 0.0);
    app.run_action(Action::SpectrumWindow);
    let mut window = window_for(&path, Mode::Mirror);
    until(&mut app, &mut engine, &mut window, |a, w| {
        a.spectrum_ready() && w.seen().0.is_some()
    });
    let s = screen(&mut window, 80, 20);
    assert!(s.contains("espectro estéreo · espejo"), "{s}");
    // Ventanas › «Cerrar la ventana del espectro»: la ventana se va sin
    // error.
    app.run_action(Action::SpectrumBack);
    assert!(!app.spectrum_listening());
    let deadline = Instant::now() + Duration::from_secs(5);
    while window.leaving().is_none() {
        assert!(Instant::now() < deadline);
        window.tick();
    }
    assert_eq!(window.leaving(), Some(&Leaving::Done));

    // Si la principal se cae, la ventana lo dice y se cierra (sin error).
    app.run_action(Action::SpectrumWindow);
    let mut window = window_for(&path, Mode::Field);
    until(&mut app, &mut engine, &mut window, |a, _| {
        a.spectrum_ready()
    });
    pump(&mut app, &mut engine, &mut window);
    drop(app);
    let deadline = Instant::now() + Duration::from_secs(5);
    while window.leaving().is_none() {
        assert!(Instant::now() < deadline);
        window.tick();
        std::thread::sleep(Duration::from_millis(20));
    }
    assert_eq!(window.leaving(), Some(&Leaving::Done));
}

#[test]
fn another_version_or_another_script_is_refused_and_bad_frames_are_dropped() {
    let (path, mut app, _engine) = open_panned("espectro_versiones", 0.0);
    app.run_action(Action::SpectrumWindow);
    let socket = link::spectrum_socket_path(&socket_dir(&path), &path);
    // Una ventana de otra versión: la principal la rechaza y no enciende
    // nada.
    let stream = std::os::unix::net::UnixStream::connect(&socket).unwrap();
    let mut conn = Conn::new(stream).unwrap();
    conn.send(&FromSpectrum::Hello { version: 7 }).unwrap();
    let mut got = Vec::new();
    let deadline = Instant::now() + Duration::from_secs(5);
    while got.is_empty() {
        assert!(Instant::now() < deadline);
        app.tick();
        got.extend(conn.receive::<ToSpectrum>().unwrap_or_default());
    }
    assert!(matches!(&got[0], ToSpectrum::Bye(r) if r.contains("protocolo distinto")));
    assert!(!app.spectrum_ready());
    assert!(app.view().message.contains("otra versión"));
    drop(conn);
    app.run_action(Action::SpectrumBack);

    // Una «principal» falsa de otra versión, o de otro script: la ventana
    // se queda con el error (hasta Enter).
    let dir = socket_dir(&path);
    let fake = dir.join("falsa-espectro.sock");
    for (welcome, why) in [
        (
            ToSpectrum::Welcome {
                version: 7,
                title: "x".into(),
                path: crate::target::full_path(&path),
            },
            "protocolo distinto",
        ),
        (
            ToSpectrum::Welcome {
                version: link::VERSION,
                title: "otra.lua".into(),
                path: "/otra/otra.lua".into(),
            },
            "otro script",
        ),
    ] {
        let mut server = Server::bind(&fake).unwrap();
        let mut window =
            SpectrumWindow::new(&fake, &crate::target::full_path(&path), Mode::Field, None);
        let deadline = Instant::now() + Duration::from_secs(5);
        while !server.accept().0 {
            assert!(Instant::now() < deadline);
        }
        server.client().unwrap().send(&welcome).unwrap();
        while window.leaving().is_none() {
            assert!(Instant::now() < deadline);
            window.tick();
        }
        assert!(
            matches!(window.leaving(), Some(Leaving::Failed(w)) if w.contains(why)),
            "{:?}",
            window.leaving()
        );
    }

    // Cuadros mal formados, con tamaños absurdos o que se amontonan: se
    // tiran, sin pánico ni corte; de varios juntos, solo el último.
    let mut server = Server::bind(&fake).unwrap();
    let mut window =
        SpectrumWindow::new(&fake, &crate::target::full_path(&path), Mode::Field, None);
    let deadline = Instant::now() + Duration::from_secs(5);
    while !server.accept().0 {
        assert!(Instant::now() < deadline);
    }
    let conn = server.client().unwrap();
    conn.send(&ToSpectrum::Welcome {
        version: link::VERSION,
        title: "song.lua".into(),
        path: crate::target::full_path(&path),
    })
    .unwrap();
    let frame = |level: f32| {
        ToSpectrum::Frame(crate::stereo::StereoFrame {
            rate: RATE,
            left: vec![level; 8],
            right: vec![level; 8],
            pan: vec![0.0; 8],
        })
    };
    conn.send(&frame(0.2)).unwrap();
    for bad in [
        "cuadro\t48000\t99999999\t00\t00\t00\n".to_string(),
        "cuadro\tNaN\t1\t00\t00\t80\n".into(),
        format!("cuadro\t48000\t1\t{}\t00\t80\n", "f".repeat(100_000)),
        "\u{1}\u{2}basura\n".into(),
    ] {
        conn.send(&RawLine(bad)).unwrap();
    }
    conn.send(&frame(0.7)).unwrap();
    conn.drain(Duration::from_secs(2)).unwrap();
    while window.seen().2 < 4 || window.seen().0.is_none_or(|f| f.left[0] < 0.6) {
        assert!(Instant::now() < deadline, "{:?}", window.seen());
        window.tick();
    }
    assert!(window.leaving().is_none());
    let (seen, skipped, bad) = window.seen();
    assert!((seen.unwrap().left[0] - 0.7).abs() < 0.01);
    assert_eq!(bad, 4);
    assert!(skipped >= 1, "el de 0,2 se saltó");
}

/// Una línea tal cual (para mandar basura).
struct RawLine(String);

impl Message for RawLine {
    fn encode(&self) -> String {
        self.0.clone()
    }
    fn decode(line: &str) -> Result<Self, String> {
        Ok(RawLine(line.into()))
    }
}

#[test]
fn a_slow_window_loses_frames_instead_of_piling_them_up() {
    let (path, mut app, mut engine) = open_panned("espectro_lenta", 0.0);
    app.run_action(Action::SpectrumWindow);
    let mut window = window_for(&path, Mode::Field);
    until(&mut app, &mut engine, &mut window, |a, _| {
        a.spectrum_ready()
    });
    // La ventana deja de leer: la principal tira cuadros en vez de
    // amontonarlos (lo que espera, como mucho un cuadro y lo que no cupo).
    for _ in 0..2000 {
        offline::render(&mut engine, 800, 2);
        app.tick();
    }
    let (sent, skipped, _) = app.spectrum_cost();
    assert!(sent > 0);
    assert!(skipped > 0 || sent >= 400, "{sent} {skipped}");
    // Cuando vuelve a leer, sigue.
    let before = sent;
    until(&mut app, &mut engine, &mut window, |a, _| {
        a.spectrum_cost().0 > before + 2
    });
}

#[test]
fn the_menu_and_the_palette_open_and_close_the_window() {
    use crate::menu::{Menu, menu_of};
    assert_eq!(menu_of(Action::SpectrumWindow), Some(Menu::Windows));
    assert_eq!(menu_of(Action::SpectrumBack), Some(Menu::Windows));
    let (path, mut app, _engine) = open_panned("espectro_menu", 0.0);
    // Sin terminal: dice cómo abrirla a mano y se queda escuchando.
    app.set_terminals(None, None);
    app.run_action(Action::SpectrumWindow);
    let message = app.view().message;
    assert!(message.contains("tui --window-spectrum"), "{message}");
    assert!(app.spectrum_listening());
    app.run_action(Action::SpectrumBack);
    assert!(!app.spectrum_listening());
    // Una terminal que no existe, o con una carpeta que no existe.
    app.set_terminals(Some(vec!["/no/existe/foot".into()]), None);
    app.run_action(Action::SpectrumWindow);
    assert!(app.view().message.contains("no se pudo lanzar"));
    assert!(!app.spectrum_listening());
    app.set_terminals(
        Some(vec!["kitty".into(), "--directory".into(), "/no/hay".into()]),
        None,
    );
    app.run_action(Action::SpectrumWindow);
    assert!(
        app.view().message.contains("/no/hay"),
        "{}",
        app.view().message
    );
    assert!(!app.spectrum_listening());
    // La orden: la terminal y la ruta completa.
    assert_eq!(
        crate::app::spectrum_command(&["foot".into(), "-e".into()], Path::new("/bin/tui"), &path),
        [
            "foot".to_string(),
            "-e".into(),
            "/bin/tui".into(),
            "--window-spectrum".into(),
            crate::target::full_path(&path).display().to_string(),
        ]
    );
    // Y en `[teclas]`, `ventana_espectro`.
    let config =
        crate::config::Config::parse("[teclas]\nventana_espectro = \"alt-s\"\n", true, "c");
    assert!(config.warnings.is_empty(), "{:?}", config.warnings);
}

#[test]
fn the_window_draws_in_every_size_and_in_ascii() {
    let (path, mut app, mut engine) = open_panned("espectro_tamanos", -0.5);
    app.run_action(Action::SpectrumWindow);
    let mut window = window_for(&path, Mode::Field);
    until(&mut app, &mut engine, &mut window, |a, w| {
        a.spectrum_ready() && w.seen().0.is_some()
    });
    pump(&mut app, &mut engine, &mut window);
    let braille = |s: &str| s.chars().any(|c| ('\u{2801}'..='\u{28FF}').contains(&c));
    for mode in [Mode::Field, Mode::Mirror] {
        for (w, h) in [(200, 60), (80, 24), (40, 12), (24, 9)] {
            let s = screen(&mut window, w, h);
            assert!(braille(&s), "{mode:?} {w}×{h}:\n{s}");
            assert!(!s.contains("muy pequeña"), "{s}");
        }
        // Por debajo del mínimo, el aviso.
        let s = screen(&mut window, 20, 6);
        assert!(s.contains("muy peq"), "{s}");
        let s = screen(&mut window, 10, 3);
        assert!(!braille(&s), "{s}");
        // En ASCII, sin Braille ni bloques.
        crate::theme::set_ascii(true);
        let mut terminal = Terminal::new(TestBackend::new(80, 24)).unwrap();
        terminal.draw(|frame| window.draw(frame)).unwrap();
        let mut buffer = terminal.backend().buffer().clone();
        crate::screen::to_ascii(&mut buffer);
        crate::theme::set_ascii(false);
        let s = text(&buffer);
        assert!(
            s.chars().all(|c| c.is_ascii() || c == '·' || c == 'é'),
            "{s}"
        );
        assert!(s.contains('#') || s.contains('='), "{s}");
        // `m` cambia de modo.
        window.handle(key('m'));
        assert_eq!(window.mode(), mode.next());
    }
}

#[test]
fn the_window_saves_its_mode() {
    let (path, ..) = open_panned("espectro_modo", 0.0);
    let config = path.parent().unwrap().join("config.toml");
    std::fs::write(&config, "[interfaz]\nrefresco = 20\n").unwrap();
    let socket = path.parent().unwrap().join("nadie.sock");
    let mut window = SpectrumWindow::new(&socket, &path, Mode::Field, Some(config.clone()));
    // Sin principal: lo dice, y en blanco hasta que llega algo.
    let s = screen(&mut window, 60, 16);
    assert!(
        s.contains("sin ventana principal") || s.contains("buscando"),
        "{s}"
    );
    assert!(s.contains("esperando el sonido"), "{s}");
    window.handle(key('m'));
    let saved = crate::config::Config::parse(&std::fs::read_to_string(&config).unwrap(), true, "c");
    assert_eq!(saved.interface.stereo_mode, Mode::Mirror);
    assert_eq!(saved.interface.refresh, 20);
    let s = screen(&mut window, 60, 16);
    assert!(s.contains("modo espejo") && s.contains("m: campo"), "{s}");
}

#[test]
#[ignore]
fn pantalla_espectro() {
    for (pan, mode) in [(-0.7, Mode::Field), (0.0, Mode::Field), (0.7, Mode::Mirror)] {
        let (path, mut app, mut engine) = open_panned("espectro_pantalla", pan);
        app.run_action(Action::SpectrumWindow);
        let mut window = window_for(&path, mode);
        until(&mut app, &mut engine, &mut window, |a, w| {
            a.spectrum_ready() && w.seen().0.is_some()
        });
        for _ in 0..10 {
            pump(&mut app, &mut engine, &mut window);
        }
        println!("{}", screen(&mut window, 100, 30));
    }
}
