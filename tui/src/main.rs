use std::sync::Arc;
use std::time::Duration;

use engine::{DetectOptions, Detector, Pattern, STEPS, Slices};

const USAGE: &str = "\
uso: tui <archivo.wav>
     tui <archivo.wav> --render <salida.wav>
     tui <archivo.wav> --render <salida.wav> --pattern \"1 . 3 . ...\" \
[--slices N] [--bpm B] [--bars M] [--min-gap MS] [--detector flux|energy]

  --pattern  16 pasos separados por espacios: número de corte (desde 1) o \".\" (silencio)
  --slices   cortes a detectar (por defecto 8)
  --bpm      tempo (por defecto 120)
  --bars     compases a renderizar (por defecto 4)
  --min-gap  separación mínima entre cortes en ms (por defecto 40)
  --detector detección de transitorios: flux (flujo espectral, por defecto) o energy";
/// Frecuencia del motor cuando se renderiza sin dispositivo.
const RENDER_RATE: u32 = 48_000;
/// Límite del render offline.
const MAX_RENDER_SECONDS: usize = 600;
const DEFAULT_SLICES: usize = 8;
const DEFAULT_BARS: u64 = 4;

type BoxError = Box<dyn std::error::Error>;

/// Opciones del secuenciador para `--render`.
struct Sequence {
    pattern: [Option<usize>; STEPS],
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
    let mut pattern = None;
    let mut slices = None;
    let mut bpm = None;
    let mut bars = None;
    let mut min_gap_ms = None;
    let mut detector = None;
    while let Some(arg) = args.next() {
        let mut value = || {
            args.next()
                .ok_or_else(|| format!("falta el valor de {arg}\n{USAGE}"))
        };
        match arg.as_str() {
            "--render" => render_to = Some(value()?),
            "--pattern" => pattern = Some(parse_pattern(&value()?)?),
            "--slices" => slices = Some(parse_number::<usize>("--slices", &value()?)?),
            "--bpm" => bpm = Some(parse_number::<f64>("--bpm", &value()?)?),
            "--bars" => bars = Some(parse_number::<u64>("--bars", &value()?)?),
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
        (None, None) => play(&path),
    }
}

/// 16 pasos separados por espacios: número de corte desde 1, o "." para silencio.
fn parse_pattern(text: &str) -> Result<[Option<usize>; STEPS], String> {
    let steps = text
        .split_whitespace()
        .map(|token| match token {
            "." => Ok(None),
            _ => match token.parse::<usize>() {
                Ok(n) if n >= 1 => Ok(Some(n - 1)),
                _ => Err(format!(
                    "paso no válido en --pattern: \"{token}\" (usa 1, 2, ... o \".\")"
                )),
            },
        })
        .collect::<Result<Vec<_>, _>>()?;
    let count = steps.len();
    steps
        .try_into()
        .map_err(|_| format!("--pattern necesita {STEPS} pasos, tiene {count}"))
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

    let pattern = Pattern::new(sample, &slices, sequence.pattern).map_err(|e| match e {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_one_based_pattern_with_rests() {
        let steps = parse_pattern("1 . . 1  . . 3 .  1 . 5 .  . 3 . 6").unwrap();
        assert_eq!(steps[0], Some(0));
        assert_eq!(steps[1], None);
        assert_eq!(steps[6], Some(2));
        assert_eq!(steps[15], Some(5));
    }

    #[test]
    fn rejects_bad_patterns() {
        assert!(parse_pattern("1 . 3").is_err());
        assert!(parse_pattern("0 . . .  . . . .  . . . .  . . . .").is_err());
        assert!(parse_pattern("x . . .  . . . .  . . . .  . . . .").is_err());
    }
}
