//! Preparar un sample al importarlo: recortar el silencio, normalizar y
//! fundidos, muestra a muestra.

use engine::{ImportOptions, Sample};

const RATE: u32 = 48_000;

/// 100 frames de silencio, 500 de señal (nunca por debajo de 0,05), 200
/// casi en silencio.
fn padded() -> Sample {
    let mut mono = vec![0.0f32; 100];
    mono.extend((0..500).map(|i| 0.05 + 0.2 * ((i as f32) * 0.1).sin().abs()));
    mono.extend(std::iter::repeat_n(1e-5, 200));
    Sample::from_interleaved(&mono, 1, RATE, RATE).unwrap()
}

fn left(s: &Sample) -> Vec<f32> {
    (0..s.frames()).map(|i| s.frame(i).0).collect()
}

#[test]
fn without_options_it_is_an_exact_copy() {
    let s = padded();
    assert!(ImportOptions::default().is_empty());
    assert_eq!(left(&s.processed(&ImportOptions::default())), left(&s));
}

#[test]
fn trim_cuts_the_silence_at_both_ends() {
    let s = padded();
    let trimmed = s.processed(&ImportOptions {
        trim_db: Some(-60.0),
        ..Default::default()
    });
    assert_eq!(trimmed.frames(), 500);
    assert_eq!(left(&trimmed), left(&s)[100..600]);
    // Con un umbral más bajo que el ruido, el ruido se queda.
    let loose = s.processed(&ImportOptions {
        trim_db: Some(-120.0),
        ..Default::default()
    });
    assert_eq!(loose.frames(), 700);
    // Todo silencio: no se quita nada.
    let silence = Sample::from_interleaved(&[0.0; 50], 1, RATE, RATE).unwrap();
    let kept = silence.processed(&ImportOptions {
        trim_db: Some(-60.0),
        ..Default::default()
    });
    assert_eq!(kept.frames(), 50);
}

#[test]
fn normalize_sets_the_peak() {
    let s = padded();
    let peak = |s: &Sample| left(s).iter().fold(0.0f32, |m, x| m.max(x.abs()));
    let full = s.processed(&ImportOptions {
        normalize_db: Some(0.0),
        ..Default::default()
    });
    assert!((peak(&full) - 1.0).abs() < 1e-6);
    let minus6 = s.processed(&ImportOptions {
        normalize_db: Some(-6.0),
        ..Default::default()
    });
    assert!((peak(&minus6) - 10f32.powf(-6.0 / 20.0)).abs() < 1e-6);
    // La forma no cambia: todo por la misma ganancia.
    let gain = peak(&full) / peak(&s);
    for (a, b) in left(&full).iter().zip(left(&s)) {
        assert!((a - b * gain).abs() < 1e-6);
    }
}

#[test]
fn fades_ramp_both_ends_linearly() {
    let flat = Sample::from_interleaved(&[0.5; 4800], 1, RATE, RATE).unwrap();
    let faded = flat.processed(&ImportOptions {
        fade_in_ms: 10.0,
        fade_out_ms: 20.0,
        ..Default::default()
    });
    let l = left(&faded);
    assert_eq!(l[0], 0.0);
    assert_eq!(l[240], 0.25);
    assert_eq!(l[480], 0.5);
    assert_eq!(l[2400], 0.5);
    assert_eq!(l[4799], 0.0);
    assert!((l[4799 - 480] - 0.25).abs() < 1e-6);
    // Orden: recortar, normalizar y fundir.
    let all = padded().processed(&ImportOptions {
        trim_db: Some(-60.0),
        normalize_db: Some(0.0),
        fade_in_ms: 1.0,
        fade_out_ms: 1.0,
    });
    assert_eq!(all.frames(), 500);
    assert_eq!(all.frame(0).0, 0.0);
    assert!(
        left(&all)[100..400]
            .iter()
            .any(|x| (x.abs() - 1.0).abs() < 1e-6)
    );
}

#[test]
fn ranges_are_checked() {
    for bad in [
        ImportOptions {
            trim_db: Some(6.0),
            ..Default::default()
        },
        ImportOptions {
            normalize_db: Some(-80.0),
            ..Default::default()
        },
        ImportOptions {
            fade_in_ms: -1.0,
            ..Default::default()
        },
    ] {
        assert!(bad.validate().is_err(), "{bad:?}");
    }
}
