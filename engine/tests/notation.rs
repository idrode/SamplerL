//! Mini-notación de patrones.

use std::sync::Arc;

use engine::notation::{check_label, parse_step, parse_steps};
use engine::{Pattern, STEPS, Sample, Slices, Step, new_engine, offline};

fn one(token: &str) -> Option<Step> {
    parse_step(token).unwrap_or_else(|e| panic!("\"{token}\": {e}"))
}

#[test]
fn parses_slices_rests_and_x() {
    assert_eq!(one("."), None);
    assert_eq!(one("1"), Some(Step::new(0)));
    assert_eq!(one("12"), Some(Step::new(11)));
    assert_eq!(one("x"), Some(Step::new(0)));
}

#[test]
fn parses_each_effect() {
    assert_eq!(one("3'"), Some(Step::new(2).reversed()));
    assert_eq!(one("3^7"), Some(Step::new(2).with_pitch(7.0)));
    assert_eq!(one("3^-5"), Some(Step::new(2).with_pitch(-5.0)));
    assert_eq!(one("3^+12"), Some(Step::new(2).with_pitch(12.0)));
    assert_eq!(one("3^0.5"), Some(Step::new(2).with_pitch(0.5)));
    assert_eq!(one("3@0.5"), Some(Step::new(2).with_gain(0.5)));
    assert_eq!(one("3@2"), Some(Step::new(2).with_gain(2.0)));
    assert_eq!(one("x'"), Some(Step::new(0).reversed()));
}

#[test]
fn effects_combine_in_any_order() {
    let expected = Some(Step::new(2).reversed().with_pitch(7.0).with_gain(0.5));
    for token in [
        "3'^7@0.5", "3^7'@0.5", "3@0.5^7'", "3^7@0.5'", "3'@0.5^7", "3@0.5'^7",
    ] {
        assert_eq!(one(token), expected, "{token}");
    }
    assert_eq!(
        one("4^-12'"),
        Some(Step::new(3).with_pitch(-12.0).reversed())
    );
}

#[test]
fn rejects_malformed_steps_with_a_message() {
    for bad in [
        "0",
        "3''",
        "3^",
        "3^-",
        "3@",
        "3@-1",
        "3^0^4^7^9^11",
        "3@1@2",
        "3^7^",
        "3x",
        "^7",
        "'",
        "a",
        "3^1.2.3",
        "-3",
        "3?",
        "3!!",
    ] {
        assert!(parse_step(bad).is_err(), "\"{bad}\" debería fallar");
    }
}

#[test]
fn pattern_needs_sixteen_steps_and_reports_the_bad_one() {
    let err = parse_steps("1 . 3").unwrap_err();
    assert_eq!(err.step, None);
    assert!(err.message.contains("16"), "{err}");

    let err = parse_steps("1 . . 1  . . 3 .  1 . 5^ .  . 3 . 6").unwrap_err();
    assert_eq!(err.step, Some(11));
    assert_eq!(err.token, "5^");
    assert!(err.to_string().starts_with("paso 11 (\"5^\")"), "{err}");

    // Espacios de más (para agrupar en tiempos) no cuentan.
    let steps = parse_steps("  1 . . 1   . . 3' .\t1 . 5 .\n. 3 . 6^7@0.5 ").unwrap();
    assert_eq!(steps[6], Some(Step::new(2).reversed()));
    assert_eq!(steps[15], Some(Step::new(5).with_pitch(7.0).with_gain(0.5)));
}

#[test]
fn notation_renders_the_same_as_built_steps() {
    const RATE: u32 = 48_000;
    let mut x = 0x1357_9bdfu32;
    let mut mono = vec![0.0f32; 12_000];
    for start in [0, 3000, 6000, 9000] {
        for s in &mut mono[start..start + 2000] {
            x ^= x << 13;
            x ^= x >> 17;
            x ^= x << 5;
            *s = 0.5 * ((x as f32 / u32::MAX as f32) * 2.0 - 1.0);
        }
    }
    let sample = Arc::new(Sample::from_interleaved(&mono, 1, RATE, RATE).unwrap());
    let slices = Slices::detect(&sample, 4);
    assert_eq!(slices.len(), 4);

    let render = |steps: [Option<Step>; STEPS]| {
        let (mut controller, mut engine) = new_engine(RATE);
        let pattern = Pattern::from_steps(sample.clone(), &slices, steps).unwrap();
        controller.set_pattern(pattern).unwrap();
        controller.start().unwrap();
        offline::render(&mut engine, engine::bar_frames(120.0, RATE) as usize, 2)
    };

    let from_text = parse_steps("1 . 2' .  3^7 . 4@0.5 .  x'^-5@1.5 . . 2  . 3 . .").unwrap();
    let mut built = [None; STEPS];
    built[0] = Some(Step::new(0));
    built[2] = Some(Step::new(1).reversed());
    built[4] = Some(Step::new(2).with_pitch(7.0));
    built[6] = Some(Step::new(3).with_gain(0.5));
    built[8] = Some(Step::new(0).reversed().with_pitch(-5.0).with_gain(1.5));
    built[11] = Some(Step::new(1));
    built[13] = Some(Step::new(2));
    assert_eq!(from_text, built);

    let audio = render(from_text);
    assert!(audio.iter().any(|&s| s.abs() > 0.3));
    assert_eq!(audio, render(built));
}

fn labels(names: &[&str]) -> Vec<Option<String>> {
    names
        .iter()
        .map(|n| (!n.is_empty()).then(|| n.to_string()))
        .collect()
}

#[test]
fn labels_stand_for_their_slices_with_the_same_suffixes() {
    use engine::notation::{parse_step_labeled, parse_steps_labeled};
    let l = labels(&["kick", "snare", "", "hat"]);
    let by_name = parse_steps_labeled(
        "kick . snare .  hat^7@0.5 . kick' .  snare'r2 . hatr4 .  x . 3 kick^-5r2'",
        &l,
    )
    .unwrap();
    let by_number = parse_steps("1 . 2 .  4^7@0.5 . 1' .  2'r2 . 4r4 .  1 . 3 1^-5r2'")
        .unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(by_name, by_number);
    let tokens = [
        "kick",
        "snare",
        "hat^7@0.5",
        "kick'",
        "snare'r2",
        "hatr4",
        "x",
        "kick^-5r2'",
    ];
    let numbers = ["1", "2", "4^7@0.5", "1'", "2'r2", "4r4", "1", "1^-5r2'"];
    for (t, n) in tokens.iter().zip(numbers) {
        assert_eq!(
            parse_step_labeled(t, &l).unwrap(),
            parse_step(n).unwrap(),
            "{t}"
        );
    }
}

#[test]
fn unknown_labels_name_the_step_and_the_known_ones() {
    use engine::notation::parse_steps_labeled;
    let l = labels(&["kick", "snare"]);
    let err = parse_steps_labeled("kick . snre .  . . . .  . . . .  . . . .", &l).unwrap_err();
    assert_eq!(err.step, Some(3));
    assert_eq!(
        err.to_string(),
        "paso 3 (\"snre\"): la etiqueta \"snre\" no existe (hay: kick, snare)"
    );
    let err = parse_steps("kick . . .  . . . .  . . . .  . . . .").unwrap_err();
    assert_eq!(
        err.to_string(),
        "paso 1 (\"kick\"): la etiqueta \"kick\" no existe (no hay cortes con etiqueta)"
    );
}

#[test]
fn label_names_cannot_clash_with_the_notation() {
    use engine::notation::check_label;
    for good in [
        "kick", "snare", "hat", "hat2", "open_hat", "_x", "bar", "r", "Kick",
    ] {
        assert!(check_label(good).is_ok(), "{good}: {:?}", check_label(good));
    }
    let cases = [
        ("", "vacía"),
        ("12", "es un número"),
        ("3kick", "empieza por un número"),
        ("x", "ya es el corte 1"),
        ("r4", "retrigger"),
        ("kickr2", "retrigger"),
        ("ki ck", "solo letras"),
        ("kick'", "solo letras"),
        ("bombo^", "solo letras"),
        ("caña", "solo letras"),
    ];
    for (bad, why) in cases {
        let err = check_label(bad).unwrap_err();
        assert!(err.contains(why), "{bad}: {err}");
    }
}

#[test]
fn several_pitches_are_a_chord() {
    let chord = one("1^0^4^7@0.5").unwrap();
    assert_eq!(chord.semitones, 0.0);
    assert_eq!(chord.chord, [Some(4.0), Some(7.0), None]);
    assert_eq!(chord.notes().collect::<Vec<_>>(), vec![0.0, 4.0, 7.0]);
    assert_eq!(chord.gain, 0.5);
    // Las notas del acorde se guardan sobre la primera: transponer el paso
    // las mueve todas.
    let up = Step {
        semitones: 12.0,
        ..one("1^-3^0^4^7").unwrap()
    };
    assert_eq!(up.notes().collect::<Vec<_>>(), vec![12.0, 15.0, 19.0, 22.0]);
    assert_eq!(
        Step::new(0).with_chord(&[4.0, 7.0]),
        one("1^4^7")
            .map(|s| Step {
                semitones: 0.0,
                chord: [Some(4.0), Some(7.0), None],
                ..s
            })
            .unwrap()
    );
    let err = parse_step("1^0^1^2^3^4").unwrap_err();
    assert!(err.contains("4 notas"), "{err}");
}

#[test]
fn hold_lengthens_the_previous_hit() {
    let steps = parse_steps("1 _ _ .  2 _ 3 _  _ _ _ _  . . . 4").unwrap();
    assert_eq!(steps[0].map(|s| s.hold), Some(2));
    assert_eq!(steps[1], None);
    assert_eq!(steps[4].map(|s| s.hold), Some(1));
    assert_eq!(steps[6].map(|s| s.hold), Some(5));
    assert_eq!(steps[15].map(|s| s.hold), Some(0));
    // `_` sin golpe antes (al principio, o tras un silencio).
    for bad in [
        "_ 1 . .  . . . .  . . . .  . . . .",
        "1 . _ .  . . . .  . . . .  . . . .",
    ] {
        let err = parse_steps(bad).unwrap_err();
        assert!(err.message.contains("no hay ninguno"), "{err}");
    }
    // `_` no puede ser una etiqueta.
    assert!(check_label("_").is_err());
    assert!(check_label("_kick").is_ok());
}

#[test]
fn percent_is_a_pitch_that_keeps_the_duration() {
    let step = parse_step("3%7").unwrap().unwrap();
    assert_eq!((step.slice, step.semitones, step.keep), (2, 7.0, true));
    let step = parse_step("1'%-12@0.5").unwrap().unwrap();
    assert!(step.keep && step.reverse && step.semitones == -12.0 && step.gain == 0.5);
    // ^ sigue cambiando la duración.
    assert!(!parse_step("1^7").unwrap().unwrap().keep);
    // Detrás de %, las notas del acorde.
    let chord = parse_step("1%0^4^7").unwrap().unwrap();
    assert!(chord.keep);
    assert_eq!(chord.notes().collect::<Vec<_>>(), [0.0, 4.0, 7.0]);
    for (bad, message) in [
        ("1^4%7", "en vez del primer ^"),
        ("1%", "falta el número después de %"),
        ("1%7%3", "en vez del primer ^"),
    ] {
        let err = parse_step(bad).unwrap_err();
        assert!(err.contains(message), "{bad}: {err}");
    }
}

#[test]
fn step_effects_are_read_and_handed_to_the_resolver() {
    use engine::notation::{FxValue, WrittenFx, parse_pattern_with, step_tokens};
    // Las llaves no se parten por los espacios de dentro.
    assert_eq!(
        step_tokens("1{lp = 800, q=4} .  x"),
        vec![(0, 16), (17, 18), (20, 21)]
    );
    let seen = std::cell::RefCell::new(Vec::new());
    let resolve = |written: &[WrittenFx]| {
        seen.borrow_mut().push(written.to_vec());
        Ok(engine::SliceFx::new(&[engine::Fx::gain(0.5)], &[]).unwrap())
    };
    let steps = parse_pattern_with(
        "1{lp=200>4000:4, q=2}{crush} . 2'{eq,gain2=-3,type2=lowshelf}^7 .",
        4,
        &[],
        Some(&resolve),
    )
    .unwrap();
    assert!(steps[0].unwrap().fx.is_some() && steps[1].is_none());
    let s = steps[2].unwrap();
    assert!(s.reverse && s.semitones == 7.0 && s.fx.is_some());
    let seen = seen.into_inner();
    assert_eq!(
        seen[0],
        vec![
            WrittenFx {
                name: "lp".into(),
                main: Some(FxValue::Ramp {
                    from: 200.0,
                    to: 4000.0,
                    steps: Some(4.0)
                }),
                params: vec![("q".into(), FxValue::Number(2.0))],
            },
            WrittenFx {
                name: "crush".into(),
                main: None,
                params: vec![],
            },
        ]
    );
    assert_eq!(
        seen[1][0].params,
        vec![
            ("gain2".into(), FxValue::Number(-3.0)),
            ("type2".into(), FxValue::Word("lowshelf".into()))
        ]
    );
    // El error del que resuelve sale con el paso y el token.
    let fail = |_: &[WrittenFx]| Err("no me gusta".to_string());
    let error = parse_pattern_with(". 1{zz=1}", 2, &[], Some(&fail)).unwrap_err();
    assert_eq!(error.to_string(), "paso 2 (\"1{zz=1}\"): no me gusta");
}

/// Accent (`!`) y slide (`/`): sufijos como los demás, en cualquier orden
/// y una vez cada uno; con etiquetas y con los acordes también.
#[test]
fn accent_and_slide_suffixes() {
    let steps = engine::notation::parse_pattern_labeled(
        "1! 1/ 1^3!/ kick/'@0.5 . 1^0^4!",
        6,
        &[Some("kick".into())],
    )
    .unwrap();
    let s = |i: usize| steps[i].unwrap();
    assert!(s(0).accent && !s(0).slide);
    assert!(s(1).slide && !s(1).accent);
    assert!(s(2).accent && s(2).slide && s(2).semitones == 3.0);
    assert!(s(3).slide && s(3).reverse && s(3).gain == 0.5);
    assert!(steps[4].is_none());
    assert!(s(5).accent && s(5).chord[0] == Some(4.0));
    assert_eq!(Step::new(0).accented().slid(), {
        let mut step = Step::new(0);
        step.accent = true;
        step.slide = true;
        step
    });
    for (token, message) in [
        ("1!!", "accent (!) repetido"),
        ("1//", "slide (/) repetido"),
    ] {
        assert_eq!(parse_step(token).unwrap_err(), message);
    }
    // Sin ellos, todo como siempre.
    assert!(!parse_step("3^7@0.5").unwrap().unwrap().accent);
}
