//! El callback de audio no reserva memoria (regla 1 de CLAUDE.md): un
//! asignador global cuenta las reservas que hace este hilo mientras corre
//! `Engine::process`, con retrasos (swing, humanize), retrigger, pitch,
//! instrumentos (polifonía, ADSR, bucles), el EQ (suavizado y automatizado),
//! el EQ dinámico con sidechain, efectos por corte (con sus colas), bucles de corte con fundido, nubes de granos en vivo, sintetizadores (mono con accent, slide y el sub de cada nota, polifónicos con unison y sub abierto, un subharm de 8 voces con sus seis capas, un sh101 con legato, curvas y notas en vivo), cambios de
//! patrón al compás, un patrón que se suelta a la cola de basura y el paso
//! de cada pista y el retardo de salida que se publican para la interfaz.

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
            // Pitch arriba y abajo: la interpolación sinc (su tabla se
            // calcula al crear el motor).
            let pitch = [0.0, 7.0, 24.0, -12.0, 0.0, 12.5][k % 6];
            *slot = Some(
                Step::new(k % 2)
                    .with_retrig(if k % 4 == 3 { 3 } else { 1 })
                    .with_delay(delay.min(0.9))
                    .with_pitch(pitch),
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
        .unwrap()
        .with_tag(engine::MAX_TAG)
        .unwrap();
    let second = Pattern::from_bars(sample.clone(), &slices, &bars[..2]).unwrap();

    let (mut controller, mut engine) = new_engine(RATE);
    controller.set_bpm(170.0).unwrap();
    controller.set_pattern(first).unwrap();
    controller.start().unwrap();
    let mut out = vec![0.0f32; 512 * 2];

    let n = counted(|| {
        for _ in 0..400 {
            // Como el callback de la salida: el retardo y el bloque. El
            // paso de cada pista (su cursor) se publica en cada paso.
            engine.set_output_delay(1024);
            engine.process(&mut out, 2);
        }
    });
    assert_eq!(n, 0, "reservas en process");
    assert!(controller.status().tracks[0].cursor.is_some());

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
        .with_fx(&[
            Fx::lowpass(3000.0),
            Fx::distortion(4.0, 0.5),
            Fx::eq(&[
                engine::EqBand::bell(300.0, -3.0, 2.0),
                engine::EqBand {
                    kind: engine::EqKind::HighShelf,
                    freq_hz: 6000.0,
                    gain_db: 4.0,
                    q: 0.707,
                },
            ]),
        ])
        .unwrap()
        .with_automation(&[
            // La ganancia de la primera banda del EQ, con un LFO.
            engine::Automation {
                fx: 2,
                param: 1,
                curve: engine::Curve::Lfo {
                    min: -6.0,
                    max: 0.0,
                    rate: engine::LfoRate::Steps(8.0),
                },
            },
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
        .unwrap()
        .with_midi(Some((10, 36)))
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

    // Otro EQ en la misma ranura con otras ganancias: suaviza hacia ellas.
    let reshaped = Pattern::from_bars(sample.clone(), &slices, &bars[..1])
        .unwrap()
        .with_fx(&[Fx::eq(&[
            engine::EqBand::bell(250.0, 6.0, 1.0),
            engine::EqBand::flat(1),
            engine::EqBand::bell(3000.0, -9.0, 4.0),
        ])])
        .unwrap();
    let first_eq = Pattern::from_bars(sample.clone(), &slices, &bars[..1])
        .unwrap()
        .with_fx(&[Fx::eq(&[engine::EqBand::bell(250.0, -6.0, 1.0)])])
        .unwrap();
    controller.set_track_pattern(6, Some(first_eq)).unwrap();
    let n = counted(|| {
        for _ in 0..200 {
            engine.process(&mut out, 2);
        }
    });
    assert_eq!(n, 0, "reservas con el EQ");
    controller.set_track_pattern(6, Some(reshaped)).unwrap();
    let n = counted(|| {
        for _ in 0..200 {
            engine.process(&mut out, 2);
        }
    });
    assert_eq!(n, 0, "reservas al suavizar el EQ");

    // EQ dinámico: uno que se escucha a sí mismo y otro que escucha a la
    // pista 6 por sidechain (con el orden de mezcla cambiado y la salida de
    // la 6 guardada para él), con la 6 sonando y luego callada.
    let ducked = Pattern::from_bars(sample.clone(), &slices, &bars[..1])
        .unwrap()
        .with_fx(&[
            Fx::DynEq(engine::DynEqParams {
                threshold_db: -40.0,
                ..engine::DynEqParams::new(200.0)
            }),
            Fx::DynEq(
                engine::DynEqParams {
                    threshold_db: -40.0,
                    range_db: 12.0,
                    ..engine::DynEqParams::new(100.0)
                }
                .with_sidechain(6),
            ),
        ])
        .unwrap();
    controller.set_track_pattern(3, Some(ducked)).unwrap();
    let n = counted(|| {
        for _ in 0..200 {
            engine.process(&mut out, 2);
        }
    });
    assert_eq!(n, 0, "reservas con el EQ dinámico y el sidechain");

    // Modo gate, forma por corte (envolvente, pan, parte, bucle e ida y
    // vuelta) y choke entre dos pistas.
    let shaped = |steps: [Option<Step>; STEPS]| {
        Pattern::from_steps(sample.clone(), &slices, steps)
            .unwrap()
            .with_gate(true)
            .with_choke(Some(2))
            .unwrap()
            .with_shapes(&[
                // Con pan, una parte del corte, bucle e ida y vuelta.
                engine::SliceShape {
                    attack_ms: 5.0,
                    release_ms: 30.0,
                    pan: -0.5,
                    length: 1.0,
                    play: engine::PlayMode::Loop,
                    crossfade_ms: 0.0,
                    curves: [None; engine::SHAPE_PARAMS],
                },
                engine::SliceShape {
                    pan: 0.7,
                    length: 0.5,
                    play: engine::PlayMode::PingPong,
                    ..engine::SliceShape::DEFAULT
                },
            ])
            .unwrap()
    };
    let mut odd = [None; STEPS];
    let mut even = [None; STEPS];
    for k in 0..STEPS {
        if k % 2 == 0 {
            even[k] = Some(Step::new(0).with_retrig(2));
        } else {
            odd[k] = Some(Step::new(1));
        }
    }
    controller.set_track_pattern(6, Some(shaped(even))).unwrap();
    controller.set_track_pattern(7, Some(shaped(odd))).unwrap();
    let n = counted(|| {
        for _ in 0..400 {
            engine.process(&mut out, 2);
        }
    });
    assert_eq!(n, 0, "reservas con gate, envolvente y choke");
    controller
        .play_slice_shaped(7, &sample, &slices, 0, engine::SliceShape::DEFAULT)
        .unwrap();
    // Una nota MIDI: un pad con su velocidad.
    controller
        .play_slice_with_gain(7, &sample, &slices, 1, engine::SliceShape::DEFAULT, 0.4)
        .unwrap();
    let n = counted(|| {
        for _ in 0..20 {
            engine.process(&mut out, 2);
        }
    });
    assert_eq!(n, 0, "reservas con un pad que hace choke");

    // Instrumento: polifonía con robo de voces (más notas que voces, un
    // acorde), ADSR, bucle sostenido con fundido, notas alargadas con `_` y
    // salida MIDI de sus notas.
    let tone: Vec<f32> = (0..24_000).map(|i| (i as f32 * 0.05).sin() * 0.3).collect();
    let tone = Arc::new(Sample::from_interleaved(&tone, 1, RATE, RATE).unwrap());
    let whole = Slices::grid(&tone, 1);
    let melody: Vec<Option<Step>> = (0..STEPS)
        .map(|k| {
            let step = Step::new(0)
                .with_pitch(k as f32 - 5.0)
                .with_hold((k % 3) as u8);
            Some(if k % 4 == 0 {
                step.with_chord(&[k as f32 - 1.0, k as f32 + 2.0])
            } else {
                step.with_retrig(if k % 5 == 0 { 2 } else { 1 })
            })
        })
        .collect();
    let mut instrument = engine::Instrument::new(3);
    instrument.adsr = engine::Adsr {
        attack_ms: 4.0,
        decay_ms: 30.0,
        sustain: 0.6,
        release_ms: 400.0,
    };
    instrument.sustain_loop = Some((4000, 9000));
    instrument.crossfade = 480;
    let synth = Pattern::from_steps(tone.clone(), &whole, melody)
        .unwrap()
        .with_instrument(Some(instrument))
        .unwrap()
        .with_choke(Some(2))
        .unwrap()
        // Sus notas, también por MIDI (con robos y acordes).
        .with_midi(Some((2, 48)))
        .unwrap();
    controller.set_track_pattern(6, Some(synth)).unwrap();
    let n = counted(|| {
        for _ in 0..400 {
            engine.process(&mut out, 2);
        }
    });
    assert_eq!(n, 0, "reservas con un instrumento");
    // Notas en vivo en el instrumento: más que voces (roba), la misma tecla
    // dos veces y sus note off.
    for key in 0..6u8 {
        controller
            .note_on(6, &tone, 0..24_000, instrument, key as f32, 0.8, key % 4)
            .unwrap();
    }
    let n = counted(|| {
        for _ in 0..50 {
            engine.process(&mut out, 2);
        }
    });
    assert_eq!(n, 0, "reservas con notas en vivo");
    for key in 0..4u8 {
        controller.note_off(6, key).unwrap();
    }
    let n = counted(|| {
        for _ in 0..100 {
            engine.process(&mut out, 2);
        }
    });
    assert_eq!(n, 0, "reservas al soltar notas en vivo");

    // Pitch que conserva la duración: pasos con `%` (con reverse, bucle y
    // retrigger) y un instrumento con `keep` (acordes, bucle, notas en
    // vivo).
    let kept: Vec<Option<Step>> = (0..STEPS)
        .map(|k| {
            let step = Step::new(k % 2).with_kept_pitch(k as f32 - 8.0);
            Some(match k % 3 {
                0 => step.reversed(),
                1 => step.with_retrig(3),
                _ => step,
            })
        })
        .collect();
    let looped = vec![
        engine::SliceShape {
            play: engine::PlayMode::Loop,
            ..engine::SliceShape::DEFAULT
        };
        slices.len()
    ];
    let shifted = Pattern::from_steps(sample.clone(), &slices, kept)
        .unwrap()
        .with_shapes(&looped)
        .unwrap();
    // Con los formantes en su sitio (corrección por LPC en cada grano).
    let shifted = shifted.with_formant(Some(0.0)).unwrap();
    controller.set_track_pattern(5, Some(shifted)).unwrap();
    let mut kept_instrument = instrument;
    kept_instrument.keep = true;
    kept_instrument.formant = Some(-3.0);
    let melody_kept: Vec<Option<Step>> = (0..STEPS)
        .map(|k| {
            Some(
                Step::new(0)
                    .with_pitch(k as f32 - 5.0)
                    .with_chord(&[k as f32]),
            )
        })
        .collect();
    let synth_kept = Pattern::from_steps(tone.clone(), &whole, melody_kept)
        .unwrap()
        .with_instrument(Some(kept_instrument))
        .unwrap();
    controller.set_track_pattern(4, Some(synth_kept)).unwrap();
    let n = counted(|| {
        for _ in 0..400 {
            engine.process(&mut out, 2);
        }
    });
    assert_eq!(n, 0, "reservas con pitch que conserva la duración");
    for key in 0..4u8 {
        controller
            .note_on(
                4,
                &tone,
                0..24_000,
                kept_instrument,
                key as f32 * 3.0,
                0.8,
                key,
            )
            .unwrap();
    }
    let n = counted(|| {
        for _ in 0..100 {
            engine.process(&mut out, 2);
        }
    });
    assert_eq!(n, 0, "reservas con notas en vivo con keep");

    // Granos: nubes con curvas, reverse, retrigger, `_`, choke con otra
    // pista de granos, y cambiar de patrón con granos sonando.
    let cloud = |seed: u64, choke: u8| {
        let steps: Vec<Option<Step>> = (0..STEPS)
            .map(|k| match k % 4 {
                0 => Some(Step::new(k % 2).with_hold(1)),
                1 => None,
                2 => Some(Step::new(0).reversed().with_retrig(3).with_pitch(5.0)),
                _ => Some(Step::new(1).with_gain(0.5)),
            })
            .collect();
        let granular = engine::Granular {
            size_ms: engine::Knob::Moving(engine::Curve::Lfo {
                min: 10.0,
                max: 120.0,
                rate: engine::LfoRate::Hz(3.0),
            }),
            density: engine::Knob::Moving(engine::Curve::Ramp {
                from: 20.0,
                to: 200.0,
                steps: 8.0,
            }),
            jitter_ms: engine::Knob::Fixed(30.0),
            spread: engine::Knob::Fixed(5.0),
            ..engine::Granular::new(seed)
        };
        Pattern::from_steps(sample.clone(), &slices, steps)
            .unwrap()
            .with_granular(Some(granular))
            .unwrap()
            .with_choke(Some(choke))
            .unwrap()
    };
    controller.set_track_pattern(3, Some(cloud(1, 9))).unwrap();
    controller.set_track_pattern(2, Some(cloud(2, 9))).unwrap();
    let n = counted(|| {
        for _ in 0..400 {
            engine.process(&mut out, 2);
        }
    });
    assert_eq!(n, 0, "reservas con granos");
    // Sin granos en el patrón nuevo: los que suenan acaban.
    controller
        .set_track_pattern(
            3,
            Some(Pattern::from_steps(sample.clone(), &slices, vec![None; STEPS]).unwrap()),
        )
        .unwrap();
    let n = counted(|| {
        for _ in 0..200 {
            engine.process(&mut out, 2);
        }
    });
    assert_eq!(n, 0, "reservas al quitar los granos");

    // Secciones: un arreglo de tres secciones (con pistas distintas en
    // cada una) que cambia de sección cada compás, un salto, repetir y
    // volver, otro arreglo encima (recargar) y quitarlo.
    let arranged = |seed: u64| {
        let mut arrangement = engine::Arrangement::new();
        for s in 0..3u64 {
            let patterns: Vec<(usize, Pattern)> = (0..3)
                .filter(|t| !(*t as u64 + s).is_multiple_of(3))
                .map(|t| {
                    let steps: Vec<Option<Step>> = (0..STEPS)
                        .map(|k| (k % (t + 2) == 0).then(|| Step::new(k % 2)))
                        .collect();
                    (
                        t,
                        Pattern::from_steps(sample.clone(), &slices, steps).unwrap(),
                    )
                })
                .collect();
            arrangement.add_section(seed + s, patterns).unwrap();
        }
        for (s, bars) in [(0, 1), (1, 2), (2, 1), (1, 1)] {
            arrangement.push(s, bars).unwrap();
        }
        arrangement
    };
    let mut batch = engine::Batch::new();
    batch.set_arrangement(Some(arranged(10)));
    controller.apply(batch).unwrap();
    let n = counted(|| {
        for _ in 0..400 {
            engine.process(&mut out, 2);
        }
    });
    assert_eq!(n, 0, "reservas con secciones");
    controller.jump_to(2).unwrap();
    controller.loop_section(true).unwrap();
    let n = counted(|| {
        for _ in 0..300 {
            engine.process(&mut out, 2);
        }
    });
    assert_eq!(n, 0, "reservas al saltar y repetir una sección");
    controller.loop_section(false).unwrap();
    let mut batch = engine::Batch::new();
    batch.set_arrangement(Some(arranged(11)));
    controller.apply(batch).unwrap();
    let n = counted(|| {
        for _ in 0..300 {
            engine.process(&mut out, 2);
        }
    });
    assert_eq!(n, 0, "reservas al recargar el arreglo");
    let mut batch = engine::Batch::new();
    batch.set_arrangement(None);
    for t in 0..3 {
        batch.set_pattern(t, None);
    }
    controller.apply(batch).unwrap();
    let n = counted(|| {
        for _ in 0..200 {
            engine.process(&mut out, 2);
        }
    });
    assert_eq!(n, 0, "reservas al quitar el arreglo");
    controller.collect_garbage();

    // Efectos por corte: dos pistas con cadenas distintas en cada corte
    // (filtro, EQ, EQ dinámico, bitcrusher, distorsión, compresor, gain),
    // curvas en sus parámetros, retrigger, reverse, pitch con y sin `%`,
    // gate y choke entre ellas; pads con sus efectos, otro patrón con las
    // colas sonando y quitarlos.
    let ramp = engine::Curve::Ramp {
        from: 300.0,
        to: 6000.0,
        steps: 16.0,
    };
    let lfo = engine::Curve::Lfo {
        min: 0.2,
        max: 1.0,
        rate: engine::LfoRate::Steps(8.0),
    };
    let chains = |seed: usize| -> Vec<Option<engine::SliceFx>> {
        let mut filter = Fx::lowpass(1200.0);
        if let Fx::Filter { q, .. } = &mut filter {
            *q = 12.0;
        }
        vec![
            Some(
                engine::SliceFx::new(
                    &[filter, Fx::bitcrush(6, 1 + seed as u32), Fx::gain(0.8)],
                    &[
                        engine::Automation {
                            fx: 0,
                            param: 0,
                            curve: ramp,
                        },
                        engine::Automation {
                            fx: 2,
                            param: 0,
                            curve: lfo,
                        },
                    ],
                )
                .unwrap(),
            ),
            Some(
                engine::SliceFx::new(
                    &[
                        Fx::eq(&[engine::EqBand::bell(800.0, 6.0, 2.0)]),
                        Fx::DynEq(engine::DynEqParams::new(200.0)),
                        Fx::distortion(8.0, 0.5),
                        Fx::compressor(-20.0, 4.0),
                    ],
                    &[],
                )
                .unwrap(),
            ),
        ]
    };
    let sliced = |t: usize, seed: usize| {
        let steps: Vec<Option<Step>> = (0..STEPS)
            .map(|k| {
                let step = Step::new((k + t) % 2).with_retrig(if k % 5 == 2 { 3 } else { 1 });
                Some(match (k + seed) % 4 {
                    0 => step.reversed(),
                    1 => step.with_pitch(7.0),
                    2 => step.with_kept_pitch(-5.0),
                    _ => step,
                })
            })
            .collect();
        Pattern::from_steps(sample.clone(), &slices, steps)
            .unwrap()
            .with_fx(&[Fx::highpass(60.0)])
            .unwrap()
            .with_slice_fx(&chains(seed))
            .unwrap()
            .with_gate(t == 0)
            .with_choke(Some(3))
            .unwrap()
    };
    let mut batch = engine::Batch::new();
    for t in 0..2 {
        batch.set_pattern(t, Some(sliced(t, 0)));
    }
    controller.apply(batch).unwrap();
    let n = counted(|| {
        for _ in 0..400 {
            engine.process(&mut out, 2);
        }
    });
    assert_eq!(n, 0, "reservas con efectos por corte");
    for i in 0..2 {
        controller
            .play_slice_with_gain(1, &sample, &slices, i, engine::SliceShape::DEFAULT, 1.0)
            .unwrap();
    }
    let mut batch = engine::Batch::new();
    for t in 0..2 {
        batch.set_pattern(t, Some(sliced(t, 1)));
    }
    controller.apply(batch).unwrap();
    let n = counted(|| {
        for _ in 0..300 {
            engine.process(&mut out, 2);
        }
    });
    assert_eq!(
        n, 0,
        "reservas con pads y otro patrón con efectos por corte"
    );
    let mut batch = engine::Batch::new();
    for t in 0..2 {
        batch.set_pattern(t, None);
    }
    controller.apply(batch).unwrap();
    let n = counted(|| {
        for _ in 0..200 {
            engine.process(&mut out, 2);
        }
    });
    assert_eq!(n, 0, "reservas al quitar los efectos por corte");
    controller.collect_garbage();

    // Bucles de corte con fundido en el salto (con lo de antes del inicio
    // y con lo de después del final), con pitch, al revés, con `%` y desde
    // un pad.
    let looped = |seed: usize| {
        let shapes: Vec<engine::SliceShape> = (0..2)
            .map(|i| engine::SliceShape {
                play: engine::PlayMode::Loop,
                crossfade_ms: [5.0, 40.0][(i + seed) % 2],
                ..engine::SliceShape::DEFAULT
            })
            .collect();
        let steps: Vec<Option<Step>> = (0..STEPS)
            .map(|k| {
                (k % 4 == 0).then(|| {
                    let step = Step::new((k / 4 + seed) % 2);
                    match k / 4 {
                        0 => step,
                        1 => step.reversed(),
                        2 => step.with_pitch(7.0),
                        _ => step.with_kept_pitch(-5.0),
                    }
                })
            })
            .collect();
        Pattern::from_steps(sample.clone(), &slices, steps)
            .unwrap()
            .with_shapes(&shapes)
            .unwrap()
    };
    controller.set_track_pattern(0, Some(looped(0))).unwrap();
    let n = counted(|| {
        for _ in 0..400 {
            engine.process(&mut out, 2);
        }
    });
    assert_eq!(n, 0, "reservas con bucles fundidos");
    let shape = engine::SliceShape {
        play: engine::PlayMode::Loop,
        crossfade_ms: 20.0,
        ..engine::SliceShape::DEFAULT
    };
    controller
        .play_slice_with_gain(0, &sample, &slices, 1, shape, 1.0)
        .unwrap();
    controller.set_track_pattern(0, Some(looped(1))).unwrap();
    let n = counted(|| {
        for _ in 0..300 {
            engine.process(&mut out, 2);
        }
    });
    assert_eq!(n, 0, "reservas con un pad en bucle fundido y otro patrón");
    controller.set_track_pattern(0, None).unwrap();
    let n = counted(|| {
        for _ in 0..200 {
            engine.process(&mut out, 2);
        }
    });
    assert_eq!(n, 0, "reservas al quitar los bucles");
    controller.collect_garbage();

    // Nubes de granos en vivo: pads y MIDI abren y sueltan nubes (con
    // curvas, jitter y spread) en una pista con granos y en una sin ellos,
    // una tecla sustituye a otra, un patrón nuevo entra con la tecla
    // mantenida, el paro las corta y una emisión del patrón las sustituye.
    let cloud = |seed: u64, density: f32| {
        let params = engine::Granular {
            size_ms: engine::Knob::Fixed(40.0),
            density: engine::Knob::Moving(engine::Curve::Lfo {
                min: density / 2.0,
                max: density,
                rate: engine::LfoRate::Steps(8.0),
            }),
            jitter_ms: engine::Knob::Fixed(80.0),
            spread: engine::Knob::Fixed(3.0),
            ..engine::Granular::new(seed)
        };
        let steps: Vec<Option<Step>> = (0..STEPS)
            .map(|k| (k % 8 == 4).then(|| Step::new(k % 2)))
            .collect();
        Pattern::from_steps(sample.clone(), &slices, steps)
            .unwrap()
            .with_granular(Some(params))
            .unwrap()
    };
    controller
        .set_track_pattern(1, Some(cloud(1, 40.0)))
        .unwrap();
    controller.start().unwrap();
    let n = counted(|| {
        for _ in 0..50 {
            engine.process(&mut out, 2);
        }
    });
    assert_eq!(n, 0, "reservas al montar la nube");
    for (i, (key, semitones)) in [(130u8, 0.0f32), (131, 7.0), (132, -12.0)]
        .into_iter()
        .enumerate()
    {
        controller
            .grain_on(1, &sample, slices.get(i % 2).unwrap(), semitones, 0.8, key)
            .unwrap();
        let n = counted(|| {
            for _ in 0..60 {
                engine.process(&mut out, 2);
            }
        });
        assert_eq!(n, 0, "reservas con la tecla {key} de una nube en vivo");
    }
    controller.grain_off(1, 130).unwrap();
    controller.grain_off(1, 132).unwrap();
    // Otro patrón (más denso) con la tecla mantenida, y un pad en otra
    // pista sin granos.
    let mut batch = engine::Batch::new();
    batch.set_pattern(1, Some(cloud(2, 80.0)));
    controller.apply(batch).unwrap();
    controller
        .grain_on(0, &sample, 0..sample.frames(), 3.0, 1.0, 140)
        .unwrap();
    let n = counted(|| {
        for _ in 0..400 {
            engine.process(&mut out, 2);
        }
    });
    assert_eq!(n, 0, "reservas con otro patrón y una tecla mantenida");
    controller.grain_off(1, 131).unwrap();
    controller.grain_off(0, 140).unwrap();
    controller
        .grain_on(1, &sample, 0..sample.frames(), 0.0, 1.0, 133)
        .unwrap();
    controller.stop().unwrap();
    let n = counted(|| {
        for _ in 0..100 {
            engine.process(&mut out, 2);
        }
    });
    assert_eq!(n, 0, "reservas al parar con una nube en vivo");
    controller
        .grain_on(1, &sample, 0..sample.frames(), 0.0, 1.0, 134)
        .unwrap();
    controller.set_track_pattern(1, None).unwrap();
    controller.start().unwrap();
    let n = counted(|| {
        for _ in 0..300 {
            engine.process(&mut out, 2);
        }
    });
    assert_eq!(n, 0, "reservas al quitar el patrón con una nube en vivo");
    controller.set_track_pattern(0, None).unwrap();
    // Una tecla con el patrón con granos todavía esperando al compás (sus
    // parámetros de reserva) y entrando después.
    controller
        .set_track_pattern(2, Some(cloud(3, 40.0)))
        .unwrap();
    controller
        .grain_on(2, &sample, 0..sample.frames(), 0.0, 1.0, 135)
        .unwrap();
    let n = counted(|| {
        for _ in 0..500 {
            engine.process(&mut out, 2);
        }
    });
    assert_eq!(n, 0, "reservas con una nube en vivo y el patrón pendiente");
    controller.grain_off(2, 135).unwrap();
    controller.set_track_pattern(2, None).unwrap();
    let n = counted(|| {
        for _ in 0..300 {
            engine.process(&mut out, 2);
        }
    });
    assert_eq!(n, 0, "reservas al quitar el patrón de una nube en vivo");
    controller.collect_garbage();

    // Todo automatizado: los parámetros nuevos del compresor, el
    // bitcrusher y el EQ dinámico, el volumen, el pan y los envíos de la
    // pista, y los parámetros de los buses.
    let ramp = |from: f32, to: f32| engine::Curve::Ramp {
        from,
        to,
        steps: 8.0,
    };
    let auto = |fx: usize, param: usize, curve| engine::Automation { fx, param, curve };
    let mut mix: engine::MixCurves = [None; engine::MIX_PARAMS];
    mix[engine::MIX_GAIN] = Some(ramp(0.2, 1.5));
    mix[engine::MIX_PAN] = Some(engine::Curve::Lfo {
        min: -1.0,
        max: 1.0,
        rate: engine::LfoRate::Hz(3.0),
    });
    mix[engine::MIX_SEND] = Some(ramp(0.0, 1.0));
    mix[engine::MIX_SEND + 1] = Some(ramp(1.0, 0.0));
    let everything = Pattern::from_bars(sample.clone(), &slices, &bars[..1])
        .unwrap()
        .with_fx(&[
            Fx::compressor(-30.0, 4.0),
            Fx::bitcrush(8, 1),
            Fx::DynEq(engine::DynEqParams::new(200.0)),
        ])
        .unwrap()
        .with_automation(&[
            auto(0, 2, ramp(1.0, 20.0)),
            auto(0, 3, ramp(0.1, 200.0)),
            auto(0, 4, ramp(1.0, 2000.0)),
            auto(1, 0, ramp(16.0, 1.0)),
            auto(1, 1, ramp(1.0, 64.0)),
            auto(2, 4, ramp(1.0, 20.0)),
            auto(2, 5, ramp(0.1, 200.0)),
            auto(2, 6, ramp(1.0, 2000.0)),
        ])
        .unwrap()
        .with_pan(0.3)
        .unwrap()
        .with_mix_automation(mix)
        .unwrap();
    controller.set_track_pattern(4, Some(everything)).unwrap();
    let buses = engine::Buses {
        curves: [
            Some(ramp(0.0, 0.95)),
            Some(ramp(1.0, 0.0)),
            Some(ramp(0.0, 1.0)),
            Some(ramp(1.0, 0.0)),
            Some(ramp(0.2, 1.0)),
        ],
        ..engine::Buses::default()
    };
    controller.set_buses(buses).unwrap();
    let n = counted(|| {
        for _ in 0..400 {
            engine.process(&mut out, 2);
        }
    });
    assert_eq!(
        n, 0,
        "reservas con todo automatizado (efectos, mezcla y buses)"
    );
    // Sin las curvas de los buses: vuelven suavizados a sus valores fijos.
    controller.set_buses(engine::Buses::default()).unwrap();
    controller.set_track_pattern(4, None).unwrap();
    let n = counted(|| {
        for _ in 0..200 {
            engine.process(&mut out, 2);
        }
    });
    assert_eq!(n, 0, "reservas al quitar las curvas de los buses");
    controller.collect_garbage();

    // Curvas por golpe: el pitch y el formante de la pista y la forma de
    // los cortes (con `length` y un bucle con fundido), con retrigger.
    let mut curves = [None; engine::SHAPE_PARAMS];
    for (i, curve) in curves.iter_mut().enumerate() {
        let (lo, hi) = [
            (0.0, 50.0),
            (0.0, 80.0),
            (-1.0, 1.0),
            (0.05, 1.0),
            (0.0, 20.0),
        ][i];
        *curve = Some(ramp(lo, hi));
    }
    let shaped = engine::SliceShape {
        play: engine::PlayMode::Loop,
        curves,
        ..engine::SliceShape::DEFAULT
    };
    let per_hit = Pattern::from_bars(sample.clone(), &slices, &bars[..1])
        .unwrap()
        .with_shapes(&[shaped, shaped, shaped, shaped])
        .unwrap()
        .with_gate(true)
        .with_formant(Some(0.0))
        .unwrap()
        .with_formant_curve(Some(ramp(-6.0, 6.0)))
        .unwrap()
        .with_pitch_curve(Some(engine::Curve::Lfo {
            min: -12.0,
            max: 12.0,
            rate: engine::LfoRate::Steps(8.0),
        }))
        .unwrap();
    controller.set_track_pattern(3, Some(per_hit)).unwrap();
    let n = counted(|| {
        for _ in 0..400 {
            engine.process(&mut out, 2);
        }
    });
    assert_eq!(n, 0, "reservas con curvas por golpe");
    controller.set_track_pattern(3, None).unwrap();
    let n = counted(|| {
        for _ in 0..100 {
            engine.process(&mut out, 2);
        }
    });
    assert_eq!(n, 0, "reservas al quitar las curvas por golpe");
    controller.collect_garbage();

    // Efectos por paso: con una rampa de golpe, retrigger, reverse y
    // pitch, juntos con los de su corte (uno sustituido y otro detrás).
    let step_chain = engine::SliceFx::new(
        &[Fx::lowpass(300.0), Fx::bitcrush(6, 2)],
        &[engine::Automation {
            fx: 0,
            param: 0,
            curve: engine::Curve::Hit {
                from: 300.0,
                to: 9000.0,
                steps: 2.0,
            },
        }],
    )
    .unwrap();
    let slice_chain =
        engine::SliceFx::new(&[Fx::lowpass(2000.0), Fx::distortion(3.0, 0.5)], &[]).unwrap();
    let stepped: Vec<Option<Step>> = (0..STEPS)
        .map(|i| {
            let step = Step::new(i % 2).with_fx((i % 3 == 0).then_some(step_chain));
            Some(match i % 4 {
                0 => step.with_retrig(3),
                1 => step.reversed(),
                2 => step.with_pitch(5.0),
                _ => step,
            })
        })
        .collect();
    let with_steps = Pattern::from_steps(sample.clone(), &slices, stepped)
        .unwrap()
        .with_slice_fx(&[Some(slice_chain), None])
        .unwrap();
    controller.set_track_pattern(2, Some(with_steps)).unwrap();
    let n = counted(|| {
        for _ in 0..400 {
            engine.process(&mut out, 2);
        }
    });
    assert_eq!(n, 0, "reservas con efectos por paso");
    controller.set_track_pattern(2, None).unwrap();
    let n = counted(|| {
        for _ in 0..100 {
            engine.process(&mut out, 2);
        }
    });
    assert_eq!(
        n, 0,
        "reservas al quitar los efectos por paso con sus colas"
    );
    controller.collect_garbage();

    // Sintetizadores: un acid mono con accents, slides y una rampa del
    // corte, y uno polifónico con unison de 7, acordes y un LFO; notas en
    // vivo y sus note off; choke entre los dos, y quitarlos.
    let silence = Arc::new(Sample::silence(RATE));
    let one = Slices::grid(&silence, 1);
    let acid_steps: Vec<Option<Step>> = (0..STEPS)
        .map(|k| {
            let step = Step::new(0).with_pitch([0.0, 12.0, 3.0, -5.0][k % 4]);
            let mut step = if k % 3 == 0 { step.accented() } else { step };
            // El sub de algunas notas: nivel, divisor, forma y pan.
            if k % 4 == 2 {
                step.sub = Some(engine::synth::SubNote {
                    level: Some(0.7),
                    div: Some(3),
                    wave: Some(engine::synth::Wave::Sine),
                    pan: Some(-0.5),
                    drive: Some(0.4),
                });
            }
            Some(if k % 4 == 1 { step.slid() } else { step })
        })
        .collect();
    let mut curves = [None; engine::synth::PARAM_COUNT];
    curves[engine::synth::CUTOFF] = Some(engine::Curve::Ramp {
        from: 200.0,
        to: 4000.0,
        steps: 16.0,
    });
    let acid = Pattern::from_steps(silence.clone(), &one, acid_steps)
        .unwrap()
        .with_synth(Some(engine::synth::acid::patch(36.0)), curves)
        .unwrap()
        .with_choke(Some(3))
        .unwrap()
        .with_fx(&[Fx::lowpass(5000.0)])
        .unwrap();
    let mut poly = engine::synth::Patch::new()
        .with(engine::synth::DETUNE, 20.0)
        .with(engine::synth::SPREAD, 0.7)
        .with(engine::synth::NOISE, 0.1)
        .with(engine::synth::SUB, 0.3)
        .with(engine::synth::SUB_PAN, 0.6)
        .with(engine::synth::SUB_DRIVE, 0.3);
    poly.sub_div = 3;
    poly.sub_wave = engine::synth::Wave::Triangle;
    poly.unison = 7;
    poly.voices = 8;
    poly.filter = engine::synth::FilterMode::Ladder;
    let mut lfo = [None; engine::synth::PARAM_COUNT];
    lfo[engine::synth::RESONANCE] = Some(engine::Curve::Lfo {
        min: 0.1,
        max: 0.9,
        rate: engine::LfoRate::Hz(3.0),
    });
    let chords: Vec<Option<Step>> = (0..STEPS)
        .map(|k| (k % 2 == 0).then(|| Step::new(0).with_chord(&[3.0, 7.0, 10.0]).with_hold(1)))
        .collect();
    let pads = Pattern::from_steps(silence.clone(), &one, chords)
        .unwrap()
        .with_synth(Some(poly), lfo)
        .unwrap()
        .with_choke(Some(3))
        .unwrap()
        .with_midi(Some((2, 48)))
        .unwrap();
    // Un sub: notas pegadas y ligadas, saturación antes del filtro y una
    // rampa de `tone`.
    let mut sub = engine::synth::sub::patch(24.0);
    engine::synth::sub::param("drive")
        .unwrap()
        .set(&mut sub, 0.5);
    let mut tone = [None; engine::synth::PARAM_COUNT];
    tone[engine::synth::CUTOFF] = Some(engine::Curve::Ramp {
        from: 80.0,
        to: 1500.0,
        steps: 8.0,
    });
    let sub_steps: Vec<Option<Step>> = (0..STEPS)
        .map(|k| {
            let step = Step::new(0).with_pitch([0.0, 7.0, 12.0, 5.0][k % 4]);
            Some(if k % 4 == 2 { step.slid() } else { step })
        })
        .collect();
    let sub = Pattern::from_steps(silence.clone(), &one, sub_steps)
        .unwrap()
        .with_synth(Some(sub), tone)
        .unwrap();
    controller.set_track_pattern(4, Some(sub)).unwrap();
    // Un subharm a plena carga: 8 voces, las seis capas sonando, abiertas
    // y con divisores distintos, con acordes y notas ligadas.
    let mut full = engine::synth::subharm::patch(36.0);
    engine::synth::subharm::param("voices")
        .unwrap()
        .set(&mut full, 8.0);
    full.layers = Some(std::array::from_fn(|i| engine::synth::Layer {
        div: [1, 2, 3, 5, 7, 16][i],
        rdiv: [4, 3, 5, 7, 2, 16][i],
        pan: [-0.8, 0.8, -0.4, 0.4, -1.0, 1.0][i],
        level: 1.0,
    }));
    let harm_steps: Vec<Option<Step>> = (0..STEPS)
        .map(|k| {
            Some(
                Step::new(0)
                    .with_chord(&[0.0, 7.0, 12.0, 19.0])
                    .with_hold(if k % 2 == 0 { 1 } else { 0 }),
            )
        })
        .collect();
    let harm = Pattern::from_steps(silence.clone(), &one, harm_steps)
        .unwrap()
        .with_synth(Some(full), [None; engine::synth::PARAM_COUNT])
        .unwrap();
    controller.set_track_pattern(3, Some(harm)).unwrap();
    controller.set_track_pattern(5, Some(acid)).unwrap();
    controller.set_track_pattern(6, Some(pads)).unwrap();
    let n = counted(|| {
        for _ in 0..400 {
            engine.process(&mut out, 2);
        }
    });
    assert_eq!(n, 0, "reservas con sintetizadores");
    let status = controller.status();
    assert_eq!(status.tracks[3].voices, 8, "el subharm, con sus 8 voces");
    // La pista 4 aún suelta lo que tocaba antes: al menos la voz del sub.
    assert!(status.tracks[4].voices >= 1, "el sub suena");
    let voices = (status.tracks[5].voices, status.tracks[6].voices);
    assert!(voices.0 == 1 && voices.1 >= 4, "voces: {voices:?}");
    for key in 0..10 {
        controller
            .synth_note_on(7, poly, key as f32, 0.8, key)
            .unwrap();
    }
    let n = counted(|| {
        for _ in 0..100 {
            engine.process(&mut out, 2);
        }
    });
    assert_eq!(n, 0, "reservas con notas en vivo del sintetizador");
    for key in 0..10 {
        controller.note_off(7, key).unwrap();
    }
    controller.set_track_pattern(3, None).unwrap();
    controller.set_track_pattern(4, None).unwrap();
    controller.set_track_pattern(5, None).unwrap();
    controller.set_track_pattern(6, None).unwrap();
    let n = counted(|| {
        for _ in 0..400 {
            engine.process(&mut out, 2);
        }
    });
    assert_eq!(n, 0, "reservas al soltar y quitar los sintetizadores");
    controller.collect_garbage();
    let _ = controller.midi_events().count();

    // Un sh101, con el filtro a la frecuencia del motor y al doble: notas
    // ligadas y separadas, el sub en pulso al 25 %, ruido y pulso; luego
    // teclas en vivo que se ligan y vuelven a la anterior.
    for quality in [1.0, 2.0] {
        let mut sh = engine::synth::sh101::patch(36.0)
            .with(engine::synth::SUB, 0.6)
            .with(engine::synth::RESONANCE, 0.9);
        for (name, value) in [
            ("pulse", 0.5),
            ("noise", 0.2),
            ("glide", 60.0),
            ("quality", quality),
        ] {
            engine::synth::sh101::param(name)
                .unwrap()
                .set(&mut sh, value);
        }
        sh.sub_div = 4;
        sh.sub_wave = engine::synth::Wave::Pulse25;
        sh.analog.as_mut().unwrap().glide = engine::synth::GlideMode::Auto;
        let sh_steps: Vec<Option<Step>> = (0..STEPS)
            .map(|k| {
                let step = Step::new(0).with_pitch([0.0, 12.0, 7.0, 3.0][k % 4]);
                Some(if k % 4 == 1 { step.slid() } else { step })
            })
            .collect();
        let mut sweep = [None; engine::synth::PARAM_COUNT];
        sweep[engine::synth::CUTOFF] = Some(engine::Curve::Ramp {
            from: 100.0,
            to: 18_000.0,
            steps: 16.0,
        });
        let pattern = Pattern::from_steps(silence.clone(), &one, sh_steps)
            .unwrap()
            .with_synth(Some(sh), sweep)
            .unwrap();
        controller.set_track_pattern(3, Some(pattern)).unwrap();
        let n = counted(|| {
            for _ in 0..400 {
                engine.process(&mut out, 2);
            }
        });
        assert_eq!(n, 0, "reservas con un sh101 (quality {quality})");
        // (La pista 3 aún suelta las voces del subharm.)
        assert!(controller.status().tracks[3].voices >= 1, "el sh101 suena");
        controller.set_track_pattern(3, None).unwrap();
        for key in 0..20 {
            controller
                .synth_note_on(7, sh, key as f32, 0.8, key)
                .unwrap();
        }
        let n = counted(|| {
            for _ in 0..100 {
                engine.process(&mut out, 2);
            }
        });
        for key in (0..20).rev() {
            controller.note_off(7, key).unwrap();
        }
        let n = n + counted(|| {
            for _ in 0..200 {
                engine.process(&mut out, 2);
            }
        });
        assert_eq!(
            n, 0,
            "reservas con notas en vivo del sh101 (quality {quality})"
        );
        controller.collect_garbage();
    }

    // Rampa de tempo y cambio de compás.
    controller
        .set_tempo_ramp(engine::TempoRamp {
            from: 150.0,
            to: 180.0,
            bars: 2,
        })
        .unwrap();
    controller
        .set_meter(engine::Meter::new(7, 8).unwrap())
        .unwrap();
    let n = counted(|| {
        for _ in 0..400 {
            engine.process(&mut out, 2);
        }
    });
    assert_eq!(n, 0, "reservas con rampa de tempo y compás de 7/8");

    // La posición se publica en cada bloque (un atómico): sin reservas.
    assert!(controller.position().is_some());

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
