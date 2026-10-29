mod app;
mod browser;
mod clipboard;
mod complete;
mod config;
mod devices;
mod devices_panel;
mod devices_window;
mod diagnose;
mod editing;
mod expression;
mod highlight;
mod history;
mod key_panel;
mod link;
mod midi;
#[cfg(feature = "midi")]
mod midi_io;
mod model;
mod occupancy;
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
     tui --edit <canción.lua> --new [...]         (un script nuevo, con la plantilla)
     tui --edit <canción.lua | proyecto.sampler> --split [...]  (el editor en otra ventana)
     tui --window-editor <canción.lua | proyecto.sampler>    (editor de una --split en marcha)
     tui --edit <canción.lua> [--allow-audio --audio-device NOMBRE] [--midi-in PUERTO] [--midi-out PUERTO]
     tui --list-devices                           (dispositivos de audio y puertos MIDI, y sale)
     tui --window-devices <canción.lua | proyecto.sampler>   (panel de dispositivos de una principal)
     tui <archivo.wav> --render <salida.wav>
     tui <archivo.wav> --detect-bpm               (propone bpm() y compases)
     tui --script <canción.lua> --render <salida.wav> [--bars M] [--from N] [--metronome] [--track pista]
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
  --bars     compases a renderizar (por defecto 4; con línea de tiempo, hasta su final)
  --from     con --script: empieza en el compás N (desde 0, como la barra de estado),
               como si la canción hubiera llegado hasta él (every, ramp, lfo, secciones)
  --min-gap  separación mínima entre cortes en ms (por defecto 40)
  --detector detección de transitorios: flux (flujo espectral, por defecto) o energy
  --metronome  añade al render el clic del metrónomo (si no, nunca lo lleva)
  --analyze-masking  renderiza offline cada pista en solo (un ciclo entero de la
               canción, hasta 256 compases) y dice en qué bandas chocan
  --allow-midi abre los puertos MIDI virtuales «sampler entrada» y «sampler salida» (ALSA)
  --audio-device  con --edit y --allow-audio: suena por ese dispositivo (su nombre, el de
               --list-devices); manda sobre [dispositivos] audio de config.toml
  --midi-in, --midi-out  con --edit: conecta la entrada o la salida MIDI a ese puerto (su
               nombre, con o sin los números de ALSA del final); abren el MIDI sin
               --allow-midi y mandan sobre [dispositivos] de config.toml
  --window-devices  abre solo el panel de dispositivos y conecta con la ventana principal
               de ese script (F1 «Abrir el panel de dispositivos en su ventana» lo lanza
               con [dispositivos] terminal o [editor] terminal)
  --list-devices  lista los dispositivos de salida de audio (abre la tarjeta para
               preguntarle qué admite) y los puertos MIDI, y sale
  --new      con --edit: si el script no existe, empieza uno con la plantilla (se
               crea al guardar); sin --new, una ruta que no existe es un error
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
    let mut from = None;
    let mut min_gap_ms = None;
    let mut detector = None;
    let mut script = None;
    let mut edit = None;
    let mut metronome = false;
    let mut detect = false;
    let mut track = None;
    let mut masking = false;
    let mut split = false;
    let mut new = false;
    let mut window_editor = None;
    let mut audio_device = None;
    let mut midi_in = None;
    let mut midi_out = None;
    let mut list_devices = false;
    let mut window_devices = None;
    while let Some(arg) = args.next() {
        let mut value = || {
            args.next()
                .ok_or_else(|| format!("falta el valor de {arg}\n{USAGE}"))
        };
        match arg.as_str() {
            "--render" => render_to = Some(value()?),
            "--allow-audio" => allow_audio = true,
            "--allow-midi" => allow_midi = true,
            "--audio-device" => audio_device = Some(value()?),
            "--midi-in" => midi_in = Some(value()?),
            "--midi-out" => midi_out = Some(value()?),
            "--list-devices" => list_devices = true,
            "--window-devices" => window_devices = Some(value()?),
            "--metronome" => metronome = true,
            "--detect-bpm" => detect = true,
            "--analyze-masking" => masking = true,
            "--pattern" => {
                let text = value()?;
                let length = engine::notation::step_tokens(&text).len();
                let steps = engine::notation::parse_pattern_with(
                    &text,
                    length,
                    &[],
                    Some(&script::step_fx::resolve),
                )
                .map_err(|e| format!("--pattern: {e}"))?;
                pattern = Some(steps);
            }
            "--slices" => slices = Some(parse_number::<usize>("--slices", &value()?)?),
            "--bpm" => bpm = Some(parse_number::<f64>("--bpm", &value()?)?),
            "--bars" => bars = Some(parse_number::<u64>("--bars", &value()?)?),
            "--from" => from = Some(parse_number::<u64>("--from", &value()?)?),
            "--script" => script = Some(value()?),
            "--track" => track = Some(value()?),
            "--edit" => edit = Some(value()?),
            "--split" => split = true,
            "--new" => new = true,
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
    let device_flags = audio_device.is_some() || midi_in.is_some() || midi_out.is_some();
    if list_devices {
        if std::env::args().count() != 2 {
            return Err(format!("--list-devices va solo\n{USAGE}").into());
        }
        print!("{}", list_all_devices());
        return Ok(());
    }
    if let Some(target) = window_devices {
        if std::env::args().count() != 3 {
            return Err(format!(
                "--window-devices no se combina con otras opciones (los dispositivos son de la \
                 ventana principal)\n{USAGE}"
            )
            .into());
        }
        return run_devices_window(std::path::Path::new(&target));
    }
    if device_flags && edit.is_none() {
        return Err(
            format!("--audio-device, --midi-in y --midi-out van con --edit\n{USAGE}").into(),
        );
    }
    if audio_device.is_some() && !allow_audio {
        return Err(format!(
            "--audio-device va con --allow-audio (sin él no se abre la tarjeta)\n{USAGE}"
        )
        .into());
    }
    if let Some(target) = window_editor {
        if edit.is_some()
            || device_flags
            || split
            || new
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
    if new && edit.is_none() {
        return Err(format!("--new va con --edit\n{USAGE}").into());
    }
    if let Some(edit) = edit {
        if path.is_some() || script.is_some() || render_to.is_some() || pattern.is_some() {
            return Err(format!("--edit no se combina con otras opciones\n{USAGE}").into());
        }
        check_edit_target(std::path::Path::new(&edit), new)?;
        let devices = DeviceArgs {
            allow_audio,
            // Pedir un puerto MIDI es pedir el MIDI.
            allow_midi: allow_midi || midi_in.is_some() || midi_out.is_some(),
            audio: audio_device,
            midi_in,
            midi_out,
        };
        return run_editor(std::path::Path::new(&edit), devices, split);
    }
    if let Some(script) = script {
        if path.is_some() || pattern.is_some() || slices.is_some() || min_gap_ms.is_some() {
            return Err(format!(
                "con --script, el sample y los patrones van en el script\n{USAGE}"
            )
            .into());
        }
        if from.is_some() && (render_to.is_none() || track.is_some() || masking) {
            return Err(format!(
                "--from va con --script y --render (sin --track ni --analyze-masking)\n{USAGE}"
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
                None => render_script(&script, &out, bars, from.unwrap_or(0), metronome),
            },
            None if allow_audio => play_script(&script),
            None => Err(format!(
                "no se reproduce por la tarjeta de sonido sin --allow-audio; \
                 para escuchar, renderiza con --render <salida.wav>\n{USAGE}"
            )
            .into()),
        };
    }
    if from.is_some() {
        return Err(format!("--from va con --script\n{USAGE}").into());
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
        "{out}: {} a {} BPM, {} frames ({:.2} s)",
        script::bars_text(sequence.bars),
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
            "  también encaja {} BPM con {}",
            engine::bpm::round_bpm(bpm),
            script::bars_text(n as u64)
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
    println!("{out}: pista \"{track}\", {}", script::bars_text(bars));
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
    from: u64,
    metronome: bool,
) -> Result<(), BoxError> {
    let mut host = script::ScriptHost::new(RENDER_RATE);
    let (mut controller, mut engine) = engine::new_engine(RENDER_RATE);
    let song = host.load_file(&mut controller, std::path::Path::new(path))?;
    let bpm = song.bpm.unwrap_or(engine::DEFAULT_BPM);
    // Sin --bars: con línea de tiempo, la canción entera (hasta 256
    // compases) o lo que queda desde --from; si no, 4.
    let bars = match bars {
        Some(bars) => bars,
        None if song.timeline.is_empty() => DEFAULT_BARS,
        None => {
            let length = song.length_bars(1);
            if from >= length {
                return Err(format!(
                    "--from {from}: la línea de tiempo dura {} (del 0 al {})",
                    script::bars_text(length),
                    length - 1
                )
                .into());
            }
            length - from
        }
    };
    if from > engine::MAX_START_BAR {
        return Err(format!("--from {from}: como mucho {}", engine::MAX_START_BAR).into());
    }
    for warning in &song.warnings {
        eprintln!("aviso: {warning}");
    }
    for track in &song.tracks {
        match track.instrument.and_then(|i| i.synth) {
            Some(synth) => println!("pista \"{}\": synth \"{}\"", track.name, synth.kind),
            None => println!(
                "pista \"{}\": {} ({} cortes)",
                track.name,
                track.path.display(),
                track.slices.len()
            ),
        }
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
    controller.start_at(from)?;

    // Con la rampa de tempo y el compás de la canción, como suena.
    let frames = song.frames(RENDER_RATE, from + bars) - song.frames(RENDER_RATE, from);
    if frames > (MAX_RENDER_SECONDS as u64) * RENDER_RATE as u64 {
        return Err(format!("render demasiado largo (máximo {MAX_RENDER_SECONDS} s)").into());
    }
    let audio = engine::offline::render(&mut engine, frames as usize, 2);
    engine::offline::write_wav(out, &audio, 2, RENDER_RATE)?;
    let start = if from > 0 {
        format!(" desde el compás {from}")
    } else {
        String::new()
    };
    println!(
        "{out}: {}{start} a {bpm} BPM, {frames} frames ({:.2} s)",
        script::bars_text(bars),
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
/// Lo que abre `--edit`: tiene que existir, salvo un script con `--new`
/// (empieza con la plantilla y se crea al guardar, en una carpeta que ya
/// existe). Un proyecto nuevo se hace con la orden `guardar`.
fn check_edit_target(path: &std::path::Path, new: bool) -> Result<(), String> {
    if path.exists() {
        return Ok(());
    }
    let shown = path.display();
    let full = std::path::absolute(path).unwrap_or_else(|_| path.to_path_buf());
    if path
        .extension()
        .is_some_and(|e| e == script::project::EXTENSION)
    {
        return Err(format!(
            "{shown}: el proyecto no existe (buscado en {}); un proyecto se crea desde la \
             interfaz con la orden «guardar»",
            full.display()
        ));
    }
    if !new {
        return Err(format!(
            "{shown}: el script no existe (buscado en {}); para empezar uno nuevo con la \
             plantilla: tui --edit {shown} --new",
            full.display()
        ));
    }
    let parent = full.parent().unwrap_or(std::path::Path::new("/"));
    if !parent.is_dir() {
        return Err(format!(
            "{shown}: la carpeta {} no existe; créala antes (mkdir -p)",
            parent.display()
        ));
    }
    Ok(())
}

/// Los dispositivos que se piden por la línea de órdenes (mandan sobre
/// `[dispositivos]` de `config.toml`).
struct DeviceArgs {
    allow_audio: bool,
    allow_midi: bool,
    audio: Option<String>,
    midi_in: Option<String>,
    midi_out: Option<String>,
}

/// `--list-devices`: los de audio (con sus canales y frecuencias) y los
/// puertos MIDI, como texto.
fn list_all_devices() -> String {
    use engine::AudioBackend;
    let mut out = String::from("Audio de salida:\n");
    match engine::CpalBackend::default().list() {
        Ok(list) if list.is_empty() => out.push_str("  (ninguno)\n"),
        Ok(list) => {
            for d in list {
                let rates: Vec<String> = d.sample_rates.iter().map(u32::to_string).collect();
                out.push_str(&format!(
                    "  {}{}\n      {} canales · {} Hz · {}\n",
                    d.name,
                    if d.default { "  (predeterminado)" } else { "" },
                    d.channels,
                    rates.join(", "),
                    d.id
                ));
            }
        }
        Err(e) => out.push_str(&format!("  no se pudo listar: {e}\n")),
    }
    #[cfg(feature = "midi")]
    {
        use devices::MidiBackend;
        let mut midi = midi_io::Midir;
        for (title, list) in [
            ("MIDI de entrada", midi.inputs()),
            ("MIDI de salida", midi.outputs()),
        ] {
            out.push_str(&format!("{title}:\n"));
            match list {
                Ok(list) => {
                    let list: Vec<_> = list.iter().filter(|p| !p.starts_with("sampler:")).collect();
                    if list.is_empty() {
                        out.push_str("  (ninguno)\n");
                    }
                    for port in list {
                        out.push_str(&format!("  {}\n", devices::port_key(port)));
                    }
                }
                Err(e) => out.push_str(&format!("  no se pudo listar: {e}\n")),
            }
        }
    }
    #[cfg(not(feature = "midi"))]
    out.push_str("MIDI: compilado sin la feature midi\n");
    out
}

fn run_editor(path: &std::path::Path, args: DeviceArgs, split: bool) -> Result<(), BoxError> {
    let DeviceArgs {
        allow_audio,
        allow_midi,
        ..
    } = args;
    // Teclas, colores y los dispositivos de la última vez (si falta o está
    // roto: por defecto y un aviso).
    let config_path = config::dir().map(|dir| dir.join("config.toml"));
    let config = match &config_path {
        Some(path) => config::Config::load(path, theme::truecolor()),
        None => config::Config::default(),
    };
    let audio_device = args.audio.or(config.devices.audio.clone());
    let midi_in = args.midi_in.or(config.devices.midi_in.clone());
    let midi_out = args.midi_out.or(config.devices.midi_out.clone());
    let (controller, engine) = engine::new_engine(RENDER_RATE);
    // La salida vive en los dispositivos de la aplicación (se puede cambiar
    // en caliente); sin --allow-audio, la nula.
    let mut warnings = config.warnings.clone();
    let audio = if allow_audio {
        let backend = Box::new(engine::CpalBackend::default());
        let (output, warning) = engine::AudioOutput::open(backend, engine, audio_device.as_deref())
            .map_err(|e| format!("audio: {e}"))?;
        warnings.extend(warning);
        output
    } else {
        engine::AudioOutput::open_null(engine, RENDER_RATE)
    };
    let sample_rate = audio.sample_rate();
    let is_project = path
        .extension()
        .is_some_and(|e| e == script::project::EXTENSION);
    let mut app = if is_project {
        app::App::open_project(path, controller, sample_rate)?
    } else {
        app::App::open(path, controller, sample_rate)?
    };
    app.set_global_blocks(script::blocks::global_dir());
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
    let midi: Option<Box<dyn devices::MidiBackend>> =
        allow_midi.then(|| Box::new(midi_io::Midir) as Box<dyn devices::MidiBackend>);
    #[cfg(not(feature = "midi"))]
    let midi: Option<Box<dyn devices::MidiBackend>> = if allow_midi {
        return Err("--allow-midi: compilado sin la feature midi".into());
    } else {
        None
    };
    let mut devices = devices::Devices::new(Some(audio), allow_audio, midi);
    let changes = devices.start_midi(midi_in.as_deref(), midi_out.as_deref());
    // Los avisos de los dispositivos (uno guardado que ya no está...),
    // junto a los de la configuración.
    for change in &changes {
        if let devices::Change::Message(text) = change {
            warnings.push(text.clone());
        }
    }
    app.set_devices(devices, changes);
    app.set_devices_config(config_path);
    if !warnings.is_empty() {
        app.set_startup_warnings(&warnings);
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

/// `--window-devices`: solo el panel de dispositivos, conectado con la
/// ventana principal del mismo script (si aún no está, se reintenta). Nunca
/// abre la tarjeta ni el MIDI: se lo pide todo a la principal.
fn run_devices_window(path: &std::path::Path) -> Result<(), BoxError> {
    use ratatui::crossterm::event::{self, Event, KeyEventKind};
    // El socket va por el script (también si se abrió un proyecto).
    let script = if path
        .extension()
        .is_some_and(|e| e == script::project::EXTENSION)
    {
        script::Project::load(path)?.script
    } else {
        path.to_path_buf()
    };
    let socket = link::devices_socket_path(&link::runtime_dir()?, &script);
    let config = match config::dir() {
        Some(dir) => config::Config::load(&dir.join("config.toml"), theme::truecolor()),
        None => config::Config::default(),
    };
    theme::set(config.theme);
    let mut window = devices_window::DevicesWindow::new(&socket);
    let mut terminal = ratatui::init();
    let result = (|| -> Result<(), BoxError> {
        loop {
            window.tick();
            if window.leaving() {
                return Ok(());
            }
            terminal.draw(|frame| window.draw(frame))?;
            if !event::poll(Duration::from_millis(30))? {
                continue;
            }
            if let Event::Key(key) = event::read()?
                && key.kind != KeyEventKind::Release
                && window.handle(key.into()) == app::Flow::Quit
            {
                return Ok(());
            }
        }
    })();
    ratatui::restore();
    result
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
    // El ratón en el editor (`[editor] raton`): la terminal deja de
    // seleccionar sola (con Shift y arrastrar, sí).
    let mouse = app.mouse_enabled()
        && ratatui::crossterm::execute!(std::io::stdout(), event::EnableMouseCapture).is_ok();
    // Si la terminal sabe decir cuándo se suelta una tecla (protocolo de
    // teclado de kitty: kitty, foot, WezTerm, Ghostty...), se le pide: los
    // pads de un instrumento sueltan su nota al soltar la tecla. Si no
    // (tmux, la mayoría), la sueltan cuando dejan de llegar repeticiones.
    // Con el protocolo, además, las teclas llegan sin ambigüedad: Shift-
    // Ctrl-Espacio (el cue) no es Ctrl-Espacio (marcha), que sin él llegan
    // los dos como el mismo byte NUL. En tmux, con `extended-keys` y
    // `extended-keys-format csi-u` llegan igual de distintas (ver la guía).
    let releases = ratatui::crossterm::terminal::supports_keyboard_enhancement().unwrap_or(false)
        && ratatui::crossterm::execute!(
            std::io::stdout(),
            PushKeyboardEnhancementFlags(
                KeyboardEnhancementFlags::REPORT_EVENT_TYPES
                    | KeyboardEnhancementFlags::DISAMBIGUATE_ESCAPE_CODES
                    | KeyboardEnhancementFlags::REPORT_ALTERNATE_KEYS
            )
        )
        .is_ok();
    // Modos del terminal que se piden al entrar y se quitan al salir,
    // también con error o pánico (al soltar la guarda).
    let modes = TerminalModes {
        paste,
        releases,
        mouse,
    };
    app.set_key_releases(releases);
    let result = (|| -> Result<(), BoxError> {
        loop {
            app.poll_midi();
            app.tick();
            // El editor separado volvió a la principal (se lo pidió ella).
            if app.leaving() {
                return Ok(());
            }
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
                    Event::Mouse(m) => {
                        use app::MouseAction;
                        use event::{MouseButton, MouseEventKind};
                        let action = match m.kind {
                            MouseEventKind::Down(MouseButton::Left) => Some(MouseAction::Down),
                            MouseEventKind::Drag(MouseButton::Left) => Some(MouseAction::Drag),
                            MouseEventKind::Up(MouseButton::Left) => Some(MouseAction::Up),
                            // Shift y la rueda: hacia los lados.
                            MouseEventKind::ScrollUp
                                if m.modifiers.contains(event::KeyModifiers::SHIFT) =>
                            {
                                Some(MouseAction::ScrollLeft)
                            }
                            MouseEventKind::ScrollDown
                                if m.modifiers.contains(event::KeyModifiers::SHIFT) =>
                            {
                                Some(MouseAction::ScrollRight)
                            }
                            MouseEventKind::ScrollUp => Some(MouseAction::ScrollUp),
                            MouseEventKind::ScrollDown => Some(MouseAction::ScrollDown),
                            MouseEventKind::Down(MouseButton::Right) => {
                                Some(MouseAction::RightDown)
                            }
                            MouseEventKind::ScrollLeft => Some(MouseAction::ScrollLeft),
                            MouseEventKind::ScrollRight => Some(MouseAction::ScrollRight),
                            _ => None,
                        };
                        if let Some(action) = action {
                            app.mouse(action, m.column, m.row);
                        }
                    }
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
    mouse: bool,
}

impl Drop for TerminalModes {
    fn drop(&mut self) {
        use ratatui::crossterm::event::{
            DisableBracketedPaste, DisableMouseCapture, PopKeyboardEnhancementFlags,
        };
        if self.mouse {
            let _ = ratatui::crossterm::execute!(std::io::stdout(), DisableMouseCapture);
        }
        if self.releases {
            let _ = ratatui::crossterm::execute!(std::io::stdout(), PopKeyboardEnhancementFlags);
        }
        if self.paste {
            let _ = ratatui::crossterm::execute!(std::io::stdout(), DisableBracketedPaste);
        }
    }
}
