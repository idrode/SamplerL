//! Banco del consumo de la interfaz: el bucle de `run_terminal` sin
//! terminal (motor offline al ritmo de los cuadros, `tick` y dibujo con el
//! backend de crossterm sobre un sumidero que cuenta bytes). Reparte el
//! tiempo de cada cuadro entre la lógica (`tick`), los widgets
//! (`ui::draw`) y el diff y la escritura (lo demás de `Terminal::draw`).
//!
//! `cargo test --release -p tui consumo -- --ignored --nocapture`
//! (`CONSUMO_CUADROS=600` para más cuadros; `CONSUMO_DIFF=1` cuenta las
//! celdas que cambian de un cuadro a otro en cada panel). Ver «Consumo de la interfaz»
//! en docs/ESTADO.md; la medición del proceso entero, con hilos, en
//! `scripts/medir-consumo.py`.

use std::io::Write;
use std::path::Path;
use std::time::{Duration, Instant};

use engine::new_engine;
use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;

use super::RATE;
use crate::app::App;
use crate::ui;

/// Cuenta lo escrito sin guardarlo.
#[derive(Default, Clone)]
struct Sink {
    bytes: std::rc::Rc<std::cell::Cell<usize>>,
}

impl Write for Sink {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.bytes.set(self.bytes.get() + buf.len());
        Ok(buf.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

struct Run {
    tick: Duration,
    widgets: Duration,
    flush: Duration,
    bytes: usize,
    frames: usize,
    /// Celdas cambiadas por panel (con `CONSUMO_DIFF`).
    changed: std::collections::BTreeMap<String, usize>,
    /// Cuadros iguales al anterior (con `CONSUMO_DIFF`).
    same: usize,
}

fn run(song: &Path, width: u16, height: u16, frames: usize, playing: bool) -> Run {
    let (controller, mut engine) = new_engine(RATE);
    let mut app = App::open(song, controller, RATE).unwrap();
    if !playing {
        app.toggle_transport();
    }
    let sink = Sink::default();
    let backend = CrosstermBackend::new(sink.clone());
    let mut terminal = Terminal::with_options(
        backend,
        ratatui::TerminalOptions {
            viewport: ratatui::Viewport::Fixed(ratatui::layout::Rect::new(0, 0, width, height)),
        },
    )
    .unwrap();
    let per_frame = RATE as usize / crate::config::DEFAULT_REFRESH.max(1) as usize;
    let mut block = vec![0.0f32; 512 * 2];
    let mut out = Run {
        tick: Duration::ZERO,
        widgets: Duration::ZERO,
        flush: Duration::ZERO,
        bytes: 0,
        frames: 0,
        changed: Default::default(),
        same: 0,
    };
    // Unos cuadros para que todo se asiente (cargas, el master que se pide).
    let warm = 60;
    let diff = std::env::var_os("CONSUMO_DIFF").is_some();
    let mut prev: Option<ratatui::buffer::Buffer> = None;
    for i in 0..warm + frames {
        let mut left = per_frame;
        while left > 0 {
            let n = left.min(512);
            engine.process(&mut block[..n * 2], 2);
            left -= n;
        }
        let t0 = Instant::now();
        app.tick();
        let t1 = Instant::now();
        let mut widgets = Duration::ZERO;
        let before = sink.bytes.get();
        let completed = terminal
            .draw(|frame| {
                let w = Instant::now();
                ui::draw(frame, &mut app);
                widgets = w.elapsed();
            })
            .unwrap();
        let t2 = Instant::now();
        // Con `CONSUMO_DIFF`, las celdas que cambian de un cuadro a otro,
        // por panel (lo que no es de un panel, por fila).
        if diff {
            let drawn = completed.buffer.clone();
            if let Some(prev) = prev.as_ref().filter(|_| i >= warm) {
                if drawn == *prev {
                    out.same += 1;
                }
                for y in 0..height {
                    for x in 0..width {
                        if drawn[(x, y)] != prev[(x, y)] {
                            let at = ratatui::layout::Position::new(x, y);
                            let label = app
                                .panels
                                .focus
                                .iter()
                                .find(|(r, _)| r.contains(at))
                                .map_or(format!("fila {y}"), |(_, p)| format!("{p:?}"));
                            *out.changed.entry(label).or_default() += 1;
                        }
                    }
                }
            }
            prev = Some(drawn);
        }
        if i >= warm {
            out.tick += t1 - t0;
            out.widgets += widgets;
            out.flush += (t2 - t1).saturating_sub(widgets);
            out.bytes += sink.bytes.get() - before;
            out.frames += 1;
        }
    }
    out
}

#[test]
#[ignore = "banco: cargo test --release -p tui consumo -- --ignored --nocapture"]
fn consumo() {
    let root = Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/.."));
    let wav = root.join("renders/synth_break.wav");
    if !wav.exists() {
        let (audio, _) = super::synth::synth_break();
        std::fs::create_dir_all(root.join("renders")).unwrap();
        engine::offline::write_wav(&wav, &audio, 1, super::synth::RATE).unwrap();
    }
    let frames: usize = std::env::var("CONSUMO_CUADROS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(300);
    println!(
        "{:<8} {:<8} {:>7} {:>9} {:>9} {:>9} {:>9} {:>8}",
        "canción", "estado", "tamaño", "tick µs", "widgets", "diff+esc", "total", "B/cuadro"
    );
    let diff = std::env::var_os("CONSUMO_DIFF").is_some();
    for (name, file) in [
        ("cancion", "cancion.lua"),
        ("visual", "visual.lua"),
        ("larga", "larga.lua"),
    ] {
        let song = root.join("scripts/consumo").join(file);
        for (width, height) in [(80, 24), (120, 40), (200, 50), (240, 70)] {
            for playing in [false, true] {
                if !playing && name != "cancion" {
                    continue;
                }
                let r = run(&song, width, height, frames, playing);
                let us = |d: Duration| d.as_secs_f64() * 1e6 / r.frames as f64;
                println!(
                    "{:<8} {:<8} {:>7} {:>9.0} {:>9.0} {:>9.0} {:>9.0} {:>8}",
                    name,
                    if playing { "sonando" } else { "parado" },
                    format!("{width}x{height}"),
                    us(r.tick),
                    us(r.widgets),
                    us(r.flush),
                    us(r.tick + r.widgets + r.flush),
                    r.bytes / r.frames,
                );
                if diff {
                    println!("    cuadros iguales al anterior: {} de {}", r.same, r.frames);
                }
                let mut changed: Vec<_> = r.changed.iter().collect();
                changed.sort_by_key(|e| std::cmp::Reverse(*e.1));
                for (panel, n) in changed {
                    println!(
                        "    {panel:<14} {:>7.1} celdas por cuadro",
                        *n as f64 / r.frames as f64
                    );
                }
            }
        }
    }
}
