mod app;
mod complete;
mod diagnose;
mod editing;
mod model;
mod palette;
#[cfg(test)]
mod tests;
mod ui;

use std::sync::Arc;
use std::time::Duration;

use engine::{DetectOptions, Detector, Pattern, STEPS, Slices, Step};

const USAGE: &str = "\
uso: tui --edit <canción.lua | proyecto.sampler> [--allow-audio]   (interfaz)
     tui <archivo.wav> --render <salida.wav>
     tui --script <canción.lua> --render <salida.wav> [--bars M]
     tui --script <canción.lua> --allow-audio     (reproduce y recarga al guardar)
     tui <archivo.wav> --render <salida.wav> --pattern \"1 . 3 . ...\" \
[--slices N] [--bpm B] [--bars M] [--min-gap MS] [--detector flux|energy]

  --pattern  16 pasos separados por espacios (mini-notación):
               3      corte 3 (desde 1; \"x\" = 1)      .      silencio
               3'     reverse                         3^7    pitch +7 semitonos (^-5, ^0.5...)
               3@0.5  volumen (0 a 4)                 3r4    retrigger/stutter (1 a 16)
               3'^7@0.5r2  combinables, en cualquier orden
  --slices   cortes a detectar (por defecto 8)
  --bpm      tempo (por defecto 120)
  --bars     compases a renderizar (por defecto 4)
  --min-gap  separación mínima entre cortes en ms (por defecto 40)
  --detector detección de transitorios: flux (flujo espectral, por defecto) o energy

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
    pattern: [Option<Step>; STEPS],
    slices: usize,
    bpm: f64,
    bars: u64,
    detect: DetectOptions,
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
    let mut pattern = None;
    let mut slices = None;
    let mut bpm = None;
    let mut bars = None;
    let mut min_gap_ms = None;
    let mut detector = None;
    let mut script = None;
    let mut edit = None;
    while let Some(arg) = args.next() {
        let mut value = || {
            args.next()
                .ok_or_else(|| format!("falta el valor de {arg}\n{USAGE}"))
        };
        match arg.as_str() {
            "--render" => render_to = Some(value()?),
            "--allow-audio" => allow_audio = true,
            "--pattern" => {
                let steps = engine::notation::parse_steps(&value()?)
                    .map_err(|e| format!("--pattern: {e}"))?;
                pattern = Some(steps);
            }
            "--slices" => slices = Some(parse_number::<usize>("--slices", &value()?)?),
            "--bpm" => bpm = Some(parse_number::<f64>("--bpm", &value()?)?),
            "--bars" => bars = Some(parse_number::<u64>("--bars", &value()?)?),
            "--script" => script = Some(value()?),
            "--edit" => edit = Some(value()?),
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
    if let Some(edit) = edit {
        if path.is_some() || script.is_some() || render_to.is_some() || pattern.is_some() {
            return Err(format!("--edit no se combina con otras opciones\n{USAGE}").into());
        }
        return run_editor(std::path::Path::new(&edit), allow_audio);
    }
    if let Some(script) = script {
        if path.is_some() || pattern.is_some() || slices.is_some() || min_gap_ms.is_some() {
            return Err(format!(
                "con --script, el sample y los patrones van en el script\n{USAGE}"
            )
            .into());
        }
        return match render_to {
            Some(out) => render_script(&script, &out, bars.unwrap_or(DEFAULT_BARS)),
            None if allow_audio => play_script(&script),
            None => Err(format!(
                "no se reproduce por la tarjeta de sonido sin --allow-audio; \
                 para escuchar, renderiza con --render <salida.wav>\n{USAGE}"
            )
            .into()),
        };
    }
    let path = path.ok_or(USAGE)?;

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

    let pattern = Pattern::from_steps(sample, &slices, sequence.pattern).map_err(|e| match e {
        engine::SendError::NoSuchSlice { index, count } => {
            format!("el patrón usa el corte {} pero solo hay {count}", index + 1)
        }
        other => other.to_string(),
    })?;

    let (mut controller, mut engine) = engine::new_engine(RENDER_RATE);
    controller.set_bpm(sequence.bpm)?;
    controller.set_pattern(pattern)?;
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

/// Carga una canción en Lua y renderiza `bars` compases a su tempo.
fn render_script(path: &str, out: &str, bars: u64) -> Result<(), BoxError> {
    let mut host = script::ScriptHost::new(RENDER_RATE);
    let (mut controller, mut engine) = engine::new_engine(RENDER_RATE);
    let song = host.load_file(&mut controller, std::path::Path::new(path))?;
    let bpm = song.bpm.unwrap_or(engine::DEFAULT_BPM);
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
    controller.start()?;

    let frames = bars * engine::bar_frames(bpm, RENDER_RATE);
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
fn run_editor(path: &std::path::Path, allow_audio: bool) -> Result<(), BoxError> {
    use ratatui::crossterm::event::{self, Event, KeyEventKind};

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

    let mut terminal = ratatui::init();
    let result = (|| -> Result<(), BoxError> {
        loop {
            app.tick();
            terminal.draw(|frame| ui::draw(frame, &mut app))?;
            if event::poll(Duration::from_millis(30))? {
                match event::read()? {
                    Event::Key(key) if key.kind != KeyEventKind::Release => {
                        if app.handle(key.into()) == app::Flow::Quit {
                            return Ok(());
                        }
                    }
                    Event::Paste(text) => {
                        app.editor.insert_str(text);
                    }
                    _ => {}
                }
            }
        }
    })();
    ratatui::restore();
    result
}
