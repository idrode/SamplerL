//! El callback de audio no reserva memoria (regla 1 de CLAUDE.md): un
//! asignador global cuenta las reservas que hace este hilo mientras corre
//! `Engine::process`, con retrasos (swing, humanize), retrigger, cambios de
//! patrón al compás y un patrón que se suelta a la cola de basura.

use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use engine::{Fx, Pattern, STEPS, Sample, Slices, Step, new_engine};

struct Counting;

static ALLOCATIONS: AtomicUsize = AtomicUsize::new(0);

thread_local! {
    /// Solo cuenta este hilo y solo mientras está activo (el arnés de tests
    /// reserva memoria en otros hilos).
    static COUNTING: Cell<bool> = const { Cell::new(false) };
}

unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        if COUNTING.with(Cell::get) {
            ALLOCATIONS.fetch_add(1, Ordering::Relaxed);
        }
        unsafe { System.alloc(layout) }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        if COUNTING.with(Cell::get) {
            ALLOCATIONS.fetch_add(1, Ordering::Relaxed);
        }
        unsafe { System.dealloc(ptr, layout) }
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, size: usize) -> *mut u8 {
        if COUNTING.with(Cell::get) {
            ALLOCATIONS.fetch_add(1, Ordering::Relaxed);
        }
        unsafe { System.realloc(ptr, layout, size) }
    }
}

#[global_allocator]
static GLOBAL: Counting = Counting;

const RATE: u32 = 48_000;

fn counted(f: impl FnOnce()) -> usize {
    let before = ALLOCATIONS.load(Ordering::Relaxed);
    COUNTING.with(|c| c.set(true));
    f();
    COUNTING.with(|c| c.set(false));
    ALLOCATIONS.load(Ordering::Relaxed) - before
}

#[test]
fn the_audio_thread_never_allocates() {
    let mut mono = vec![0.0f32; 6000];
    mono[..800].fill(0.5);
    mono[3000..3800].fill(0.25);
    let sample = Arc::new(Sample::from_interleaved(&mono, 1, RATE, RATE).unwrap());
    let slices = Slices::detect(&sample, 2);

    // Swing en los contratiempos, retrigger retrasado y un ciclo de compases
    // con retrasos distintos (como humanize).
    let bar = |seed: f32| {
        let mut steps = [None; STEPS];
        for (k, slot) in steps.iter_mut().enumerate() {
            let delay = if k % 2 == 1 { 0.32 } else { 0.0 } + (seed * k as f32) % 0.3;
            *slot = Some(
                Step::new(k % 2)
                    .with_retrig(if k % 4 == 3 { 3 } else { 1 })
                    .with_delay(delay.min(0.9)),
            );
        }
        steps
    };
    let bars: Vec<_> = (0..4).map(|b| bar(0.07 * b as f32)).collect();
    let first = Pattern::from_bars(sample.clone(), &slices, &bars)
        .unwrap()
        .with_fx(&[
            Fx::lowpass(3000.0),
            Fx::bitcrush(8, 2),
            Fx::compressor(-20.0, 6.0),
            Fx::distortion(6.0, 0.5),
        ])
        .unwrap();
    let second = Pattern::from_bars(sample.clone(), &slices, &bars[..2]).unwrap();

    let (mut controller, mut engine) = new_engine(RATE);
    controller.set_bpm(170.0).unwrap();
    controller.set_pattern(first).unwrap();
    controller.start().unwrap();
    let mut out = vec![0.0f32; 512 * 2];

    let n = counted(|| {
        for _ in 0..400 {
            engine.process(&mut out, 2);
        }
    });
    assert_eq!(n, 0, "reservas en process");

    // Un patrón nuevo entra al compás y el viejo se va a la cola de basura.
    controller.set_track_pattern(1, Some(second)).unwrap();
    let n = counted(|| {
        for _ in 0..400 {
            engine.process(&mut out, 2);
        }
    });
    assert_eq!(n, 0, "reservas al cambiar de patrón");
    // Mute y solo en vivo, un patrón con mute del script y el metrónomo.
    controller.set_metronome(true).unwrap();
    controller.set_mute(0, true).unwrap();
    controller.set_solo(1, true).unwrap();
    let muted = Pattern::from_bars(sample.clone(), &slices, &bars[..1])
        .unwrap()
        .with_mute(true)
        .with_solo(true);
    controller.set_track_pattern(2, Some(muted)).unwrap();
    let n = counted(|| {
        for _ in 0..400 {
            engine.process(&mut out, 2);
        }
    });
    assert_eq!(n, 0, "reservas con mute y solo");
    controller.set_mute(0, false).unwrap();
    controller.set_solo(1, false).unwrap();

    // Preescucha: varias seguidas (con fundido cruzado) y parada.
    for i in [0, 1, 0] {
        controller.preview_slice(&sample, &slices, i).unwrap();
        let n = counted(|| {
            for _ in 0..20 {
                engine.process(&mut out, 2);
            }
        });
        assert_eq!(n, 0, "reservas con preescucha");
    }
    controller.stop_preview().unwrap();
    let n = counted(|| {
        for _ in 0..20 {
            engine.process(&mut out, 2);
        }
    });
    assert_eq!(n, 0, "reservas al parar la preescucha");

    // Longitudes variables: 12 y 32 pasos con ciclo, en polirritmia.
    let laps = |length: usize| -> Vec<Vec<Option<Step>>> {
        (0..3)
            .map(|lap| {
                (0..length)
                    .map(|k| ((k + lap) % 3 == 0).then(|| Step::new(k % 2).with_delay(0.2)))
                    .collect()
            })
            .collect()
    };
    controller
        .set_track_pattern(
            3,
            Some(Pattern::from_bars(sample.clone(), &slices, &laps(12)).unwrap()),
        )
        .unwrap();
    controller
        .set_track_pattern(
            4,
            Some(Pattern::from_bars(sample.clone(), &slices, &laps(32)).unwrap()),
        )
        .unwrap();
    let n = counted(|| {
        for _ in 0..400 {
            engine.process(&mut out, 2);
        }
    });
    assert_eq!(n, 0, "reservas con longitudes variables");

    // Envíos al delay y a la reverb, con cambio de parámetros al compás.
    // Con automatización (rampa y LFO) y cortes por paso (`~`).
    let locked: Vec<Option<Step>> = (0..STEPS)
        .map(|k| Some(Step::new(k % 2).with_lock(200.0 + 500.0 * k as f32)))
        .collect();
    let sending = Pattern::from_steps(sample.clone(), &slices, locked)
        .unwrap()
        .with_fx(&[Fx::lowpass(3000.0), Fx::distortion(4.0, 0.5)])
        .unwrap()
        .with_automation(&[
            engine::Automation {
                fx: 0,
                param: 1,
                curve: engine::Curve::Ramp {
                    from: 0.5,
                    to: 4.0,
                    steps: 32.0,
                },
            },
            engine::Automation {
                fx: 1,
                param: 1,
                curve: engine::Curve::Lfo {
                    min: 0.0,
                    max: 1.0,
                    rate: engine::LfoRate::Hz(2.0),
                },
            },
        ])
        .unwrap()
        .with_sends([0.8, 0.6])
        .unwrap();
    controller.set_track_pattern(5, Some(sending)).unwrap();
    controller
        .set_buses(engine::Buses {
            delay: engine::DelayParams {
                division: engine::Division::Quarter,
                feedback: 0.9,
                level: 1.0,
            },
            ..engine::Buses::default()
        })
        .unwrap();
    let n = counted(|| {
        for _ in 0..400 {
            engine.process(&mut out, 2);
        }
    });
    assert_eq!(
        n, 0,
        "reservas con envíos, automatización y cortes por paso"
    );

    controller.stop().unwrap();
    controller.start().unwrap();
    let n = counted(|| {
        for _ in 0..100 {
            engine.process(&mut out, 2);
        }
    });
    assert_eq!(n, 0, "reservas al parar y arrancar");
    controller.collect_garbage();
}

#[test]
fn the_counter_sees_allocations() {
    let n = counted(|| {
        let v = std::hint::black_box(vec![1u8; 64]);
        drop(v);
    });
    assert!(n >= 2, "{n}");
}
