mod app;
mod browser;
mod clipboard;
mod complete;
mod config;
mod diagnose;
mod editing;
mod history;
mod link;
mod midi;
#[cfg(feature = "midi")]
mod midi_io;
mod model;
mod palette;
mod record;
mod tap;
#[cfg(test)]
mod tests;
mod theme;
mod ui;

use std::sync::Arc;
use std::time::Duration;

use engine::{DetectOptions, Detector, Pattern, Slices, Step};

const USAGE: &str = "\
uso: tui --edit <canción.lua | proyecto.sampler> [--allow-audio] [--allow-midi]   (interfaz)
     tui --edit <canción.lua | proyecto.sampler> --split [...]  (el editor en otra ventana)
     tui --window-editor <canción.lua | proyecto.sampler>    (editor de una --split en marcha)
     tui <archivo.wav> --render <salida.wav>
     tui <archivo.wav> --detect-bpm               (propone bpm() y compases)
     tui --script <canción.lua> --render <salida.wav> [--bars M] [--metronome] [--track pista]
     tui --script <canción.lua> --allow-audio     (reproduce y recarga al guardar)
     tui --script <canción.lua> --analyze-masking (choques de frecuencia entre pistas)
     tui <archivo.wav> --render <salida.wav> --pattern \"1 . 3 . ...\" \
[--slices N] [--bpm B] [--bars M] [--min-gap MS] [--detector flux|energy] [--metronome]

  --pattern  pasos separados por espacios (mini-notación; 16 = un compás, de 1 a 64):
               3      corte 3 (desde 1; \"x\" = 1)      .      silencio
               3'     reverse                         3^7    pitch +7 semitonos (^-5, ^0.5...)
               3@0.5  volumen (0 a 4)                 3r4    retrigger/stutter (1 a 16)
               3%7    pitch +7 sin cambiar la duración (granos; bien hasta ±12)
               3'^7@0.5r2  combinables, en cualquier orden
  --slices   cortes a detectar (por defecto 8)
  --bpm      tempo (por defecto 120)
  --bars     compases a renderizar (por defecto 4)
  --min-gap  separación mínima entre cortes en ms (por defecto 40)
  --detector detección de transitorios: flux (flujo espectral, por defecto) o energy
  --metronome  añade al render el clic del metrónomo (si no, nunca lo lleva)
  --analyze-masking  renderiza offline cada pista en solo (un ciclo entero de la
               canción, hasta 256 compases) y dice en qué bandas chocan
  --allow-midi abre los puertos MIDI virtuales «sampler entrada» y «sampler salida» (ALSA)
  --split    con --edit: el motor y las vistas aquí y el editor en una terminal nueva
               ([editor] terminal en config.toml); se hablan por un socket local
  --window-editor  abre solo el editor y conecta con la ventana principal de ese script

Sin --render, tui se niega a reproducir por la tarjeta de sonido salvo con
--allow-audio (ver CLAUDE.md: no usarlo en sesiones automáticas).";
/// Frecuencia del motor cuando se renderiza sin dispositivo.
const RENDER_RATE: u32 = 48_000;
/// Límite del render offline.
const MAX_RENDER_SECONDS: usize = 600;
const DEFAULT_SLICES: usize = 8;
const DEFAULT_BARS: u64 = 4;

type BoxError = Box<dyn std::error::Error>;

/// Opciones del secuenciador para `--render`.
struct Sequence {
    pattern: Vec<Option<Step>>,
    slices: usize,
    bpm: f64,
    bars: u64,
    detect: DetectOptions,
    metronome: bool,
}

fn main() -> std::process::ExitCode {
    match run() {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("error: {e}");
            std::process::ExitCode::FAILURE
        }
    }
}

fn run() -> Result<(), BoxError> {
    let mut args = std::env::args().skip(1);
    let mut path = None;
    let mut render_to = None;
    let mut allow_audio = false;
    let mut allow_midi = false;
    let mut pattern = None;
    let mut slices = None;
    let mut bpm = None;
    let mut bars = None;
    let mut min_gap_ms = None;
    let mut detector = None;
    let mut script = None;
    let mut edit = None;
    let mut metronome = false;
    let mut detect = false;
    let mut track = None;
    let mut masking = false;
    let mut split = false;
    let mut window_editor = None;
    while let Some(arg) = args.next() {
        let mut value = || {
            args.next()
                .ok_or_else(|| format!("falta el valor de {arg}\n{USAGE}"))
        };
        match arg.as_str() {
            "--render" => render_to = Some(value()?),
            "--allow-audio" => allow_audio = true,
            "--allow-midi" => allow_midi = true,
            "--metronome" => metronome = true,
            "--detect-bpm" => detect = true,
            "--analyze-masking" => masking = true,
            "--pattern" => {
                let text = value()?;
                let length = text.split_whitespace().count();
                let steps = engine::notation::parse_pattern_labeled(&text, length, &[])
                    .map_err(|e| format!("--pattern: {e}"))?;
                pattern = Some(steps);
            }
            "--slices" => slices = Some(parse_number::<usize>("--slices", &value()?)?),
            "--bpm" => bpm = Some(parse_number::<f64>("--bpm", &value()?)?),
            "--bars" => bars = Some(parse_number::<u64>("--bars", &value()?)?),
            "--script" => script = Some(value()?),
            "--track" => track = Some(value()?),
            "--edit" => edit = Some(value()?),
            "--split" => split = true,
            "--window-editor" => window_editor = Some(value()?),
            "--detector" => {
                detector = Some(match value()?.as_str() {
                    "flux" => Detector::Flux,
                    "energy" => Detector::Energy,
                    other => {
                        return Err(
                            format!("--detector debe ser flux o energy, no \"{other}\"").into()
                        );
                    }
                })
            }
            "--min-gap" => {
                let ms = parse_number::<f32>("--min-gap", &value()?)?;
                if ms.is_nan() || ms < 0.0 {
                    return Err(format!("--min-gap no puede ser negativo: {ms}").into());
                }
                min_gap_ms = Some(ms);
            }
            "-h" | "--help" => {
                println!("{USAGE}");
                return Ok(());
            }
            flag if flag.starts_with("--") => {
                return Err(format!("opción desconocida: {flag}\n{USAGE}").into());
            }
            _ if path.is_none() => path = Some(arg),
            _ => return Err(USAGE.into()),
        }
    }
    if let Some(target) = window_editor {
        if edit.is_some()
            || split
            || allow_audio
            || allow_midi
            || path.is_some()
            || script.is_some()
            || render_to.is_some()
            || pattern.is_some()
        {
            return Err(format!(
                "--window-editor no se combina con otras opciones (el audio y el MIDI son de la \
                 ventana principal)\n{USAGE}"
            )
            .into());
        }
        return run_window_editor(std::path::Path::new(&target));
    }
    if split && edit.is_none() {
        return Err(format!("--split va con --edit\n{USAGE}").into());
    }
    if let Some(edit) = edit {
        if path.is_some() || script.is_some() || render_to.is_some() || pattern.is_some() {
            return Err(format!("--edit no se combina con otras opciones\n{USAGE}").into());
        }
        return run_editor(std::path::Path::new(&edit), allow_audio, allow_midi, split);
    }
    if let Some(script) = script {
        if path.is_some() || pattern.is_some() || slices.is_some() || min_gap_ms.is_some() {
            return Err(format!(
                "con --script, el sample y los patrones van en el script\n{USAGE}"
            )
            .into());
        }
        if masking {
            if render_to.is_some() || allow_audio || track.is_some() || bars.is_some() {
                return Err(format!(
                    "--analyze-masking no se combina con --render, --track, --bars ni --allow-audio\n{USAGE}"
                )
                .into());
            }
            print!("{}", masking_report(std::path::Path::new(&script))?);
            return Ok(());
        }
        return match render_to {
            Some(out) => match track {
                Some(track) => render_track(&script, &out, &track, bars),
                None => render_script(&script, &out, bars, metronome),
            },
            None if allow_audio => play_script(&script),
            None => Err(format!(
                "no se reproduce por la tarjeta de sonido sin --allow-audio; \
                 para escuchar, renderiza con --render <salida.wav>\n{USAGE}"
            )
            .into()),
        };
    }
    let path = path.ok_or(USAGE)?;
    if detect {
        return detect_tempo(&path);
    }

    let sequence = match pattern {
        Some(pattern) => Some(Sequence {
            pattern,
            slices: slices.unwrap_or(DEFAULT_SLICES),
            bpm: bpm.unwrap_or(engine::DEFAULT_BPM),
            bars: bars.unwrap_or(DEFAULT_BARS),
            detect: DetectOptions {
                detector: detector.unwrap_or_default(),
                min_gap_seconds: min_gap_ms
                    .map_or(engine::DEFAULT_MIN_GAP_SECONDS, |ms| ms / 1000.0),
            },
            metronome,
        }),
        None if slices.is_some()
            || bpm.is_some()
            || bars.is_some()
            || min_gap_ms.is_some()
            || detector.is_some() =>
        {
            return Err(format!(
                "--slices, --bpm, --bars, --min-gap y --detector necesitan --pattern\n{USAGE}"
            )
            .into());
        }
        None => None,
    };

    match (render_to, sequence) {
        (Some(out), Some(sequence)) => render_sequence(&path, &out, &sequence),
        (Some(out), None) => render(&path, &out),
        (None, Some(_)) => Err(format!("--pattern solo funciona con --render\n{USAGE}").into()),
        (None, None) if allow_audio => play(&path),
        (None, None) => Err(format!(
            "no se reproduce por la tarjeta de sonido sin --allow-audio; \
             para escuchar, renderiza con --render <salida.wav>\n{USAGE}"
        )
        .into()),
    }
}

fn parse_number<T: std::str::FromStr>(flag: &str, text: &str) -> Result<T, String> {
    text.parse()
        .map_err(|_| format!("valor no válido para {flag}: \"{text}\""))
}

fn load(path: &str, rate: u32) -> Result<Arc<engine::Sample>, engine::LoadError> {
    let sample = engine::Sample::load_wav(path, rate)?;
    println!(
        "{path}: {} frames a {} Hz ({:.2} s)",
        sample.frames(),
        sample.sample_rate(),
        sample.frames() as f64 / sample.sample_rate() as f64
    );
    Ok(Arc::new(sample))
}

fn play(path: &str) -> Result<(), BoxError> {
    // `open_output` ajusta la frecuencia del motor a la del dispositivo.
    let (mut controller, engine) = engine::new_engine(RENDER_RATE);
    let output = engine::open_output(engine)?;
    controller.play(load(path, output.sample_rate)?)?;
    while controller.is_playing() {
        std::thread::sleep(Duration::from_millis(50));
        controller.collect_garbage();
    }
    Ok(())
}

fn render(path: &str, out: &str) -> Result<(), BoxError> {
    let (mut controller, mut engine) = engine::new_engine(RENDER_RATE);
    controller.play(load(path, RENDER_RATE)?)?;
    let max_frames = MAX_RENDER_SECONDS * RENDER_RATE as usize;
    let audio = engine::offline::render_until_stopped(&controller, &mut engine, 2, max_frames);
    engine::offline::write_wav(out, &audio, 2, RENDER_RATE)?;
    println!("{out}: {} frames renderizados", audio.len() / 2);
    Ok(())
}

/// Renderiza exactamente `bars` compases del patrón (el WAV se puede poner en loop).
fn render_sequence(path: &str, out: &str, sequence: &Sequence) -> Result<(), BoxError> {
    let sample = load(path, RENDER_RATE)?;
    let slices = Slices::detect_with(&sample, sequence.slices, &sequence.detect);
    println!(
        "{} cortes detectados (pedidos {}):",
        slices.len(),
        sequence.slices
    );
    for (i, range) in slices.iter().enumerate() {
        let ms = |frame: usize| frame as f64 * 1000.0 / RENDER_RATE as f64;
        println!(
            "  {:>2}: {:>8.1} ms - {:>8.1} ms",
            i + 1,
            ms(range.start),
            ms(range.end)
        );
    }

    let pattern = Pattern::from_steps(sample, &slices, &sequence.pattern).map_err(|e| match e {
        engine::SendError::NoSuchSlice { index, count } => {
            format!("el patrón usa el corte {} pero solo hay {count}", index + 1)
        }
        other => other.to_string(),
    })?;

    let (mut controller, mut engine) = engine::new_engine(RENDER_RATE);
    controller.set_bpm(sequence.bpm)?;
    controller.set_pattern(pattern)?;
    if sequence.metronome {
        controller.set_metronome(true)?;
    }
    controller.start()?;

    let frames = sequence.bars * engine::bar_frames(sequence.bpm, RENDER_RATE);
    if frames > (MAX_RENDER_SECONDS as u64) * RENDER_RATE as u64 {
        return Err(format!("render demasiado largo (máximo {MAX_RENDER_SECONDS} s)").into());
    }
    let audio = engine::offline::render(&mut engine, frames as usize, 2);
    engine::offline::write_wav(out, &audio, 2, RENDER_RATE)?;
    println!(
        "{out}: {} compases a {} BPM, {} frames ({:.2} s)",
        sequence.bars,
        sequence.bpm,
        frames,
        frames as f64 / RENDER_RATE as f64
    );
    Ok(())
}

/// `--detect-bpm`: propone el tempo y los compases de un loop.
fn detect_tempo(path: &str) -> Result<(), BoxError> {
    let sample = load(path, RENDER_RATE)?;
    let guess = engine::detect_bpm(&sample)
        .ok_or("no se encontró un tempo (¿sin golpes, o no es un loop de compases?)")?;
    let bars = if guess.bars == 1 {
        "compás"
    } else {
        "compases"
    };
    println!(
        "{path}: {:.2} BPM, {} {bars} (encaje {:.0} %) → bpm({})",
        guess.bpm,
        guess.bars,
        guess.score * 100.0,
        guess.rounded()
    );
    if let Some((bpm, n)) = guess.alternative {
        println!(
            "  también encaja {} BPM con {n} compases",
            engine::bpm::round_bpm(bpm)
        );
    }
    Ok(())
}

/// Carga una canción en Lua y renderiza `bars` compases a su tempo.
/// `--script canción.lua --render salida.wav --track pista`: solo esa
/// pista (remuestreo), por defecto lo que tarda en repetirse.
fn render_track(path: &str, out: &str, track: &str, bars: Option<u64>) -> Result<(), BoxError> {
    let mut host = script::ScriptHost::new(RENDER_RATE);
    let (mut controller, _engine) = engine::new_engine(RENDER_RATE);
    let song = host
        .load_file(&mut controller, std::path::Path::new(path))?
        .clone();
    let bars = bars
        .or(song.track_length_bars(track))
        .unwrap_or(DEFAULT_BARS);
    let audio = song
        .render_track(RENDER_RATE, bars, track)
        .ok_or_else(|| format!("--track: no hay ninguna pista \"{track}\""))??;
    engine::offline::write_wav(out, &audio, 2, RENDER_RATE)?;
    println!("{out}: pista \"{track}\", {bars} compases");
    Ok(())
}

/// `--script canción.lua --analyze-masking`: el informe de choques de
/// frecuencia (con los avisos de la carga delante). Todo offline, en
/// motores aparte: nunca abre la tarjeta.
fn masking_report(path: &std::path::Path) -> Result<String, BoxError> {
    let mut host = script::ScriptHost::new(RENDER_RATE);
    let (mut controller, _engine) = engine::new_engine(RENDER_RATE);
    let song = host.load_file(&mut controller, path)?.clone();
    let report = song.analyze_masking(RENDER_RATE, &engine::masking::MaskingOptions::default())?;
    let mut out: String = song
        .warnings
        .iter()
        .map(|w| format!("aviso: {w}\n"))
        .collect();
    out.push_str(&report.text());
    Ok(out)
}

fn render_script(
    path: &str,
    out: &str,
    bars: Option<u64>,
    metronome: bool,
) -> Result<(), BoxError> {
    let mut host = script::ScriptHost::new(RENDER_RATE);
    let (mut controller, mut engine) = engine::new_engine(RENDER_RATE);
    let song = host.load_file(&mut controller, std::path::Path::new(path))?;
    let bpm = song.bpm.unwrap_or(engine::DEFAULT_BPM);
    // Sin --bars: con línea de tiempo, la canción entera (hasta 256
    // compases); si no, 4.
    let bars = bars.unwrap_or(if song.timeline.is_empty() {
        DEFAULT_BARS
    } else {
        song.length_bars(1)
    });
    for warning in &song.warnings {
        eprintln!("aviso: {warning}");
    }
    for track in &song.tracks {
        println!(
            "pista \"{}\": {} ({} cortes)",
            track.name,
            track.path.display(),
            track.slices.len()
        );
    }
    if !song.timeline.is_empty() {
        let line: Vec<String> = song
            .timeline
            .iter()
            .map(|&(s, bars)| format!("{} {bars}", song.sections[s].name))
            .collect();
        println!("línea de tiempo: {}", line.join(" · "));
    }
    // El metrónomo del script no entra en el render: solo con --metronome.
    if metronome {
        controller.set_metronome(true)?;
    }
    controller.start()?;

    // Con la rampa de tempo y el compás de la canción, como suena.
    let frames = song.frames(RENDER_RATE, bars);
    if frames > (MAX_RENDER_SECONDS as u64) * RENDER_RATE as u64 {
        return Err(format!("render demasiado largo (máximo {MAX_RENDER_SECONDS} s)").into());
    }
    let audio = engine::offline::render(&mut engine, frames as usize, 2);
    engine::offline::write_wav(out, &audio, 2, RENDER_RATE)?;
    println!(
        "{out}: {bars} compases a {bpm} BPM, {frames} frames ({:.2} s)",
        frames as f64 / RENDER_RATE as f64
    );
    Ok(())
}

/// Reproduce una canción en Lua por la tarjeta y la recarga al guardarla.
/// Cada cambio entra en el siguiente compás; si el script falla, se muestra
/// el error y sigue sonando lo anterior. Solo con `--allow-audio`.
fn play_script(path: &str) -> Result<(), BoxError> {
    use script::WatchEvent;

    // `open_output` ajusta la frecuencia del motor a la del dispositivo.
    let (mut controller, engine) = engine::new_engine(RENDER_RATE);
    let output = engine::open_output(engine)?;
    let mut host = script::ScriptHost::new(output.sample_rate);
    let mut watcher = script::ScriptWatcher::new(path);
    let mut started = false;
    println!("{path}: se recarga al guardar (Ctrl-C para salir)");
    loop {
        match watcher.poll(&mut host, &mut controller) {
            WatchEvent::Unchanged => {}
            WatchEvent::Reloaded => {
                println!("recargado: entra en el siguiente compás");
                for warning in host.song().map_or(&[][..], |s| &s.warnings) {
                    eprintln!("aviso: {warning}");
                }
                if !started {
                    controller.start()?;
                    started = true;
                }
            }
            WatchEvent::Failed(e) => eprintln!("error (sigue sonando lo anterior): {e}"),
        }
        controller.collect_garbage();
        std::thread::sleep(Duration::from_millis(200));
    }
}

/// Interfaz de terminal. Sin `--allow-audio`, el motor avanza con una salida
/// nula (a ritmo real, sin abrir la tarjeta): se ve todo y no suena nada.
/// Con `split`, esta es la ventana principal y el editor se abre en una
/// terminal nueva.
fn run_editor(
    path: &std::path::Path,
    allow_audio: bool,
    allow_midi: bool,
    split: bool,
) -> Result<(), BoxError> {
    let (controller, engine) = engine::new_engine(RENDER_RATE);
    // Hay que mantener viva la salida mientras dure la interfaz.
    let (_audio, _null, sample_rate);
    if allow_audio {
        let output = engine::open_output(engine)?;
        sample_rate = output.sample_rate;
        _audio = Some(output);
        _null = None;
    } else {
        _null = Some(engine::open_null_output(engine, RENDER_RATE));
        sample_rate = RENDER_RATE;
        _audio = None;
    }
    let is_project = path
        .extension()
        .is_some_and(|e| e == script::project::EXTENSION);
    let mut app = if is_project {
        app::App::open_project(path, controller, sample_rate)?
    } else {
        app::App::open(path, controller, sample_rate)?
    };
    app.set_global_blocks(script::blocks::global_dir());
    // Teclas y colores del usuario (si falta o está roto: por defecto y un
    // aviso).
    let config = match config::dir() {
        Some(dir) => config::Config::load(&dir.join("config.toml"), theme::truecolor()),
        None => config::Config::default(),
    };
    theme::set(config.theme);
    app.set_config(&config);
    // Las ranuras 1-9, de una sesión a otra.
    app.set_slots_dir(config::dir().map(|d| d.join("ranuras")));
    // El historial de versiones del script, fuera del repositorio.
    app.set_history_root(history::root());
    // El editor en su ventana: se escucha antes de abrir la interfaz (si
    // ya hay otra principal de este script, el error sale normal).
    if split {
        let socket = link::socket_path(&link::runtime_dir()?, app.script_path());
        let server = link::Server::bind(&socket)?;
        app.serve(
            server,
            config.editor_terminal.clone(),
            std::env::current_exe()?,
        );
        app.launch_editor();
    }
    // MIDI solo si se pide: abre puertos del sistema.
    #[cfg(feature = "midi")]
    let _midi_in = if allow_midi {
        let (tx, rx) = rtrb::RingBuffer::new(midi_io::QUEUE);
        app.set_midi_input(rx);
        app.set_midi_output(Box::new(
            midi_io::open_output().map_err(|e| format!("MIDI: {e}"))?,
        ));
        Some(midi_io::open_input(tx).map_err(|e| format!("MIDI: {e}"))?)
    } else {
        None
    };
    #[cfg(not(feature = "midi"))]
    if allow_midi {
        return Err("--allow-midi: compilado sin la feature midi".into());
    }
    // Con MIDI se mira la cola más a menudo (menos retraso en las notas).
    let poll = if allow_midi {
        Duration::from_millis(2)
    } else {
        Duration::from_millis(30)
    };

    run_terminal(&mut app, poll, ui::draw)
}

/// `--window-editor`: solo el editor, conectado con la ventana principal
/// del mismo script (si aún no está, se reintenta). Nunca abre la tarjeta
/// ni el MIDI: su motor no se mueve; todo lo que suena es de la principal.
fn run_window_editor(path: &std::path::Path) -> Result<(), BoxError> {
    let (controller, _engine) = engine::new_engine(RENDER_RATE);
    let mut app = app::App::open_editor(path, controller, RENDER_RATE, &link::runtime_dir()?)?;
    app.set_global_blocks(script::blocks::global_dir());
    let config = match config::dir() {
        Some(dir) => config::Config::load(&dir.join("config.toml"), theme::truecolor()),
        None => config::Config::default(),
    };
    theme::set(config.theme);
    app.set_config(&config);
    app.set_slots_dir(config::dir().map(|d| d.join("ranuras")));
    app.set_history_root(history::root());
    run_terminal(&mut app, Duration::from_millis(30), ui::draw_editor_window)
}

/// El bucle de la interfaz en la terminal: dibuja, lee teclas y pegados y
/// atiende al motor y al enlace en cada vuelta.
fn run_terminal(
    app: &mut app::App,
    poll: Duration,
    draw: fn(&mut ratatui::Frame, &mut app::App),
) -> Result<(), BoxError> {
    use ratatui::crossterm::event::{
        self, EnableBracketedPaste, Event, KeyEventKind, KeyboardEnhancementFlags,
        PushKeyboardEnhancementFlags,
    };

    let mut terminal = ratatui::init();
    // Pegado del terminal (bracketed paste): lo pegado llega de una vez,
    // con sus saltos de línea. Si el terminal no lo sabe, llega como
    // teclas en ráfaga (ver `App::handle_keys`).
    let paste = ratatui::crossterm::execute!(std::io::stdout(), EnableBracketedPaste).is_ok();
    // Si la terminal sabe decir cuándo se suelta una tecla (protocolo de
    // teclado de kitty: kitty, foot, WezTerm, Ghostty...), se le pide: los
    // pads de un instrumento sueltan su nota al soltar la tecla. Si no
    // (tmux, la mayoría), la sueltan cuando dejan de llegar repeticiones.
    let releases = ratatui::crossterm::terminal::supports_keyboard_enhancement().unwrap_or(false)
        && ratatui::crossterm::execute!(
            std::io::stdout(),
            PushKeyboardEnhancementFlags(KeyboardEnhancementFlags::REPORT_EVENT_TYPES)
        )
        .is_ok();
    // Modos del terminal que se piden al entrar y se quitan al salir,
    // también con error o pánico (al soltar la guarda).
    let modes = TerminalModes { paste, releases };
    app.set_key_releases(releases);
    let result = (|| -> Result<(), BoxError> {
        loop {
            app.poll_midi();
            app.tick();
            terminal.draw(|frame| draw(frame, app))?;
            // Lo copiado, al portapapeles de la terminal (OSC 52).
            let osc = app.clipboard.take_osc();
            if !osc.is_empty() {
                use std::io::Write;
                let mut out = std::io::stdout();
                for sequence in osc {
                    let _ = out.write_all(sequence.as_bytes());
                }
                let _ = out.flush();
            }
            if !event::poll(poll)? {
                continue;
            }
            // Lo que ya ha llegado, junto: un pegado sin bracketed paste
            // es una ráfaga de teclas.
            let mut events = vec![event::read()?];
            while events.len() < BURST_MAX && event::poll(Duration::ZERO)? {
                events.push(event::read()?);
            }
            let presses: Option<Vec<_>> = events
                .iter()
                .map(|e| match e {
                    Event::Key(key) if key.kind != KeyEventKind::Release => Some((*key).into()),
                    _ => None,
                })
                .collect();
            if let Some(inputs) = presses {
                if app.handle_keys(inputs) == app::Flow::Quit {
                    return Ok(());
                }
                continue;
            }
            for event in events {
                match event {
                    Event::Key(key) if key.kind == KeyEventKind::Release => {
                        app.key_release(key.into());
                    }
                    Event::Key(key) => {
                        if app.handle(key.into()) == app::Flow::Quit {
                            return Ok(());
                        }
                    }
                    Event::Paste(text) => app.paste(&text),
                    _ => {}
                }
            }
        }
    })();
    drop(modes);
    ratatui::restore();
    result
}

/// Eventos leídos de una vez como mucho (un pegado largo sin bracketed
/// paste llega en varias ráfagas).
const BURST_MAX: usize = 4096;

/// Modos pedidos al terminal; se quitan al soltarla.
struct TerminalModes {
    paste: bool,
    releases: bool,
}

impl Drop for TerminalModes {
    fn drop(&mut self) {
        use ratatui::crossterm::event::{DisableBracketedPaste, PopKeyboardEnhancementFlags};
        if self.releases {
            let _ = ratatui::crossterm::execute!(std::io::stdout(), PopKeyboardEnhancementFlags);
        }
        if self.paste {
            let _ = ratatui::crossterm::execute!(std::io::stdout(), DisableBracketedPaste);
        }
    }
}
