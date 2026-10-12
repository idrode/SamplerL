//! Mini-notación de patrones.

use std::sync::Arc;

use engine::notation::{parse_step, parse_steps};
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
        "0", "3''", "3^", "3^-", "3@", "3@-1", "3^7^2", "3@1@2", "3x", "^7", "'", "a", "3^1.2.3",
        "-3", "3!",
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
