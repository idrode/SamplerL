//! Funciones que generan o transforman patrones en mini-notación: reciben y
//! devuelven texto (de 1 a 64 pasos; 16 en un compás) que luego se pasa a
//! `steps`. No conocen los
//! cortes de ninguna pista: cada paso se trata como una palabra (`3`,
//! `kick^7`, `.`), y lo valida `steps` al usarse.

use engine::{MAX_STEPS, STEPS};

/// Pasos de un patrón (de 1 a `MAX_STEPS`; los cuenta `steps` con la
/// longitud de la pista), o un error con quién lo pidió.
pub fn tokens(text: &str, who: &str) -> Result<Vec<String>, String> {
    let tokens = engine::notation::step_texts(text);
    if !(1..=MAX_STEPS).contains(&tokens.len()) {
        return Err(format!(
            "{who}: el patrón necesita de 1 a {MAX_STEPS} pasos, tiene {}",
            tokens.len()
        ));
    }
    Ok(tokens)
}

/// Patrón agrupado por tiempos: `1 . 2 .  3 . 4 .  ...`.
pub fn format(tokens: &[String]) -> String {
    tokens
        .chunks(4)
        .map(|beat| beat.join(" "))
        .collect::<Vec<_>>()
        .join("  ")
}

/// Comprueba que `hit` es un único paso que suena (no `.` ni vacío).
fn check_hit(hit: &str, who: &str) -> Result<(), String> {
    if hit.is_empty() || hit.contains(char::is_whitespace) || hit == "." {
        return Err(format!(
            "{who}: el golpe tiene que ser un paso que suene (\"x\", \"3\", \"kick^7\"...), no \"{hit}\""
        ));
    }
    Ok(())
}

/// Ritmo euclídeo: `k` golpes repartidos lo más igual posible en `n`
/// pasos, empezando por un golpe (el paso `i` suena si `i·k mod n < k`).
/// Se repite hasta llenar `length` pasos (16 por defecto: un compás). `hit`
/// es el paso que suena.
pub fn euclid(k: i64, n: i64, hit: &str, length: Option<i64>) -> Result<String, String> {
    let who = format!("euclid({k}, {n})");
    let length = length.unwrap_or(STEPS as i64);
    if !(1..=MAX_STEPS as i64).contains(&length) {
        return Err(format!("{who}: los pasos van de 1 a {MAX_STEPS}"));
    }
    if !(1..=MAX_STEPS as i64).contains(&n) {
        return Err(format!("{who}: n va de 1 a {MAX_STEPS}"));
    }
    if !(0..=n).contains(&k) {
        return Err(format!("{who}: k va de 0 a n ({n})"));
    }
    check_hit(hit, &who)?;
    let tokens: Vec<String> = (0..length)
        .map(|i| {
            let i = i % n;
            if (i * k) % n < k {
                hit.to_string()
            } else {
                ".".to_string()
            }
        })
        .collect();
    Ok(format(&tokens))
}

/// Desplaza el patrón `n` pasos: con `n` positivo cada paso suena `n`
/// pasos más tarde (los del final pasan al principio); negativo, antes.
pub fn rotate(text: &str, n: i64) -> Result<String, String> {
    let tokens = tokens(text, &format!("rotate(patrón, {n})"))?;
    let len = tokens.len() as i64;
    let rotated: Vec<String> = (0..len)
        .map(|i| tokens[(i - n).rem_euclid(len) as usize].clone())
        .collect();
    Ok(format(&rotated))
}

/// El patrón al revés: el paso 16 pasa a ser el 1. No invierte el sonido
/// de cada golpe (para eso, el sufijo `'`).
pub fn reverse_pattern(text: &str) -> Result<String, String> {
    let mut tokens = tokens(text, "reverse_pattern")?;
    tokens.reverse();
    Ok(format(&tokens))
}

/// Generador pseudoaleatorio de `chance`, `mutate` y `shuffle` (xorshift64*): rápido y
/// reproducible con la misma semilla.
pub struct Rng(u64);

impl Rng {
    pub fn new(seed: u64) -> Self {
        // Mezcla de splitmix64: semillas parecidas (4 y 5) dan estados muy
        // distintos. El estado no puede ser 0.
        let mut z = seed.wrapping_add(0x9e37_79b9_7f4a_7c15);
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        Rng((z ^ (z >> 31)) | 1)
    }

    /// Número uniforme en [0, 1).
    pub fn next_f64(&mut self) -> f64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        let x = self.0.wrapping_mul(0x2545_f491_4f6c_dd1d);
        (x >> 11) as f64 / (1u64 << 53) as f64
    }
}

/// Cambia cada paso, con probabilidad `p`, por uno de los del propio
/// patrón elegido al azar (así se queda con su vocabulario de golpes y
/// silencios).
pub fn mutate(text: &str, p: f64, rng: &mut Rng) -> Result<String, String> {
    let tokens = tokens(text, &format!("mutate(patrón, {p})"))?;
    if !(0.0..=1.0).contains(&p) {
        return Err(format!("mutate(patrón, {p}): la probabilidad va de 0 a 1"));
    }
    let mutated: Vec<String> = (0..tokens.len())
        .map(|i| {
            if rng.next_f64() < p {
                let j = (rng.next_f64() * tokens.len() as f64) as usize;
                tokens[j.min(tokens.len() - 1)].clone()
            } else {
                tokens[i].clone()
            }
        })
        .collect();
    Ok(format(&mutated))
}

/// Los mismos pasos en otro orden al azar (Fisher-Yates).
pub fn shuffle(text: &str, rng: &mut Rng) -> Result<String, String> {
    let mut tokens = tokens(text, "shuffle")?;
    for i in (1..tokens.len()).rev() {
        let j = ((rng.next_f64() * (i + 1) as f64) as usize).min(i);
        tokens.swap(i, j);
    }
    Ok(format(&tokens))
}

/// Semitonos del `degree` (desde 1; negativos, por debajo de la tónica)
/// de una escala con esos `intervals`, sobre una tónica a `root` semitonos
/// de do.
pub fn scale_pitch(intervals: &[i32], root: i32, degree: i64) -> Result<i32, String> {
    if degree == 0 {
        return Err("el grado 0 no existe (1 es la tónica, -1 el grado de debajo)".into());
    }
    let len = intervals.len() as i64;
    let index = if degree > 0 { degree - 1 } else { degree };
    let octave = index.div_euclid(len);
    let semitones = root as i64 + intervals[index.rem_euclid(len) as usize] as i64 + 12 * octave;
    i32::try_from(semitones).map_err(|_| format!("el grado {degree} está demasiado lejos"))
}

/// Patrón de grados (`"1 . 3 .  5@0.5 ..."`) a pasos con pitch del `hit`:
/// `"1^0 . 1^3 .  1^7@0.5 ..."`.
pub fn melody(intervals: &[i32], root: i32, degrees: &str, hit: &str) -> Result<String, String> {
    let who = "melody";
    let tokens = tokens(degrees, who)?;
    check_hit(hit, who)?;
    if hit.contains(['^', '%']) {
        return Err(format!(
            "{who}: el golpe no puede llevar ^ ni % (el pitch lo pone la escala): \"{hit}\""
        ));
    }
    let steps: Vec<String> = tokens
        .iter()
        .enumerate()
        .map(|(i, token)| {
            if token == "." || token == engine::notation::HOLD {
                return Ok(token.clone());
            }
            let sign = usize::from(token.starts_with('-'));
            let digits = token[sign..].chars().take_while(char::is_ascii_digit).count();
            let (number, suffixes) = token.split_at(sign + digits);
            let degree: i64 = number
                .parse()
                .map_err(|_| format!("{who}: paso {} (\"{token}\"): debe empezar por un grado (1, 2, -1...) o ser \".\"", i + 1))?;
            if suffixes.contains(['^', '%']) {
                return Err(format!("{who}: paso {} (\"{token}\"): el pitch lo pone la escala, sin ^ ni %", i + 1));
            }
            let semitones = scale_pitch(intervals, root, degree)
                .map_err(|e| format!("{who}: paso {} (\"{token}\"): {e}", i + 1))?;
            if semitones.abs() > engine::MAX_SEMITONES as i32 {
                return Err(format!(
                    "{who}: paso {} (\"{token}\"): el grado {degree} da {semitones} semitonos (máximo ±36)",
                    i + 1
                ));
            }
            Ok(format!("{hit}^{semitones}{suffixes}"))
        })
        .collect::<Result<_, String>>()?;
    Ok(format(&steps))
}

/// Lee una nota al principio de `text`: letra de `a` a `g` (mayúscula o
/// minúscula), `#` o `b` opcional y octava opcional (`-1` a `9`; C4 = 60
/// es el do central). Devuelve la clase (semitonos sobre do, con la
/// alteración), la octava si la lleva y lo que sigue.
fn read_note(text: &str) -> Option<(i32, Option<i32>, &str)> {
    let mut chars = text.char_indices();
    let (_, letter) = chars.next()?;
    let class = match letter.to_ascii_lowercase() {
        'c' => 0,
        'd' => 2,
        'e' => 4,
        'f' => 5,
        'g' => 7,
        'a' => 9,
        'b' => 11,
        _ => return None,
    };
    let mut rest = &text[1..];
    let class = if let Some(after) = rest.strip_prefix('#') {
        rest = after;
        class + 1
    } else if let Some(after) = rest.strip_prefix('b') {
        rest = after;
        class - 1
    } else {
        class
    };
    let sign = usize::from(rest.starts_with('-'));
    let digits = rest[sign..]
        .chars()
        .take_while(char::is_ascii_digit)
        .count();
    if digits == 0 {
        return Some((class, None, rest));
    }
    let octave: i32 = rest[..sign + digits].parse().ok()?;
    if !(-1..=9).contains(&octave) {
        return None;
    }
    Some((class, Some(octave), &rest[sign + digits..]))
}

/// Nota MIDI de un nombre como `"C3"`, `"F#4"` o `"bb2"` (sin octava, la
/// 4: `"A"` es 69). C4 = 60.
pub fn note_number(name: &str) -> Option<i32> {
    match read_note(name)? {
        (class, octave, "") => Some(12 * (octave.unwrap_or(4) + 1) + class),
        _ => None,
    }
}

/// Nombre de una nota MIDI: `60` → `"C4"`, `61` → `"C#4"`.
pub fn note_name(midi: i32) -> String {
    const NAMES: [&str; 12] = [
        "C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "A#", "B",
    ];
    format!(
        "{}{}",
        NAMES[midi.rem_euclid(12) as usize],
        midi.div_euclid(12) - 1
    )
}

/// Patrón de notas (`"c e g e"`, `"c+e+g _ . a3@0.5"`) a pasos con pitch
/// respecto a la raíz `root` (nota MIDI): `"1^0 1^4 1^7 1^4"`. Una nota
/// sin octava va en la de la raíz; en un acorde (`+`), las siguientes sin
/// octava van justo por encima de la anterior (`g+b+d` es sol mayor). `.`
/// y `_` pasan tal cual, y cada nota admite los sufijos de los pasos menos
/// `^` (el tono lo da la nota).
pub fn notes(text: &str, root: i32) -> Result<String, String> {
    let who = "melody";
    let tokens = tokens(text, who)?;
    let steps: Vec<String> = tokens
        .iter()
        .enumerate()
        .map(|(i, token)| {
            note_step(token, Some(root))
                .map_err(|message| format!("{who}: paso {} (\"{token}\"): {message}", i + 1))
        })
        .collect::<Result<_, String>>()?;
    Ok(format(&steps))
}

/// Un paso de `melody` (una nota, un acorde, `.` o `_`) en mini-notación:
/// `e@0.5` → `1^4@0.5` con la raíz C4. Con `root`, también comprueba que
/// cada nota esté a menos de ±36 semitonos de ella; sin ella (el editor no
/// la sabe), los semitonos son sobre C4 y no se comprueban.
/// Las llaves de un paso en `melody`: solo las entienden los cortes.
pub const BRACES_IN_MELODY: &str = "las llaves ({...}) solo valen en steps(\"...\") de una pista de \
     cortes, no en melody() ni en un instrumento: usa los efectos de la pista (:fx)";

pub fn note_step(token: &str, root: Option<i32>) -> Result<String, String> {
    if token == "." || token == engine::notation::HOLD {
        return Ok(token.to_string());
    }
    let base = root.unwrap_or(60);
    let root_octave = base.div_euclid(12) - 1;
    let mut rest = token;
    let mut pitches: Vec<i32> = Vec::new();
    loop {
        let (class, octave, after) = read_note(rest).ok_or_else(|| {
            "debe ser una nota (c, e, g#, bb3...), un acorde (c+e+g), \".\" o \"_\"".to_string()
        })?;
        let midi = match (octave, pitches.last()) {
            (Some(octave), _) => 12 * (octave + 1) + class,
            (None, None) => 12 * (root_octave + 1) + class,
            // La primera por encima de la anterior del acorde.
            (None, Some(&below)) => below + 1 + (class - below - 1).rem_euclid(12),
        };
        pitches.push(midi);
        match after.strip_prefix('+') {
            Some(next) => rest = next,
            None => {
                rest = after;
                break;
            }
        }
    }
    if pitches.len() > engine::MAX_CHORD {
        return Err(format!(
            "un acorde tiene como mucho {} notas",
            engine::MAX_CHORD
        ));
    }
    if rest.contains(['^', '%']) {
        return Err("el tono lo da la nota, sin ^ ni %".into());
    }
    if rest.contains('{') {
        return Err(BRACES_IN_MELODY.into());
    }
    engine::notation::parse_step(&format!("1{rest}"))?;
    let mut step = String::from("1");
    for midi in pitches {
        let semitones = midi - base;
        if root.is_some() && semitones.abs() > engine::MAX_SEMITONES as i32 {
            return Err(format!(
                "{} está a {semitones} semitonos de la raíz {} (máximo ±36)",
                note_name(midi),
                note_name(base)
            ));
        }
        step.push_str(&format!("^{semitones}"));
    }
    step.push_str(rest);
    Ok(step)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn euclidean_rhythms() {
        assert_eq!(
            euclid(3, 8, "x", None).unwrap(),
            "x . . x  . . x .  x . . x  . . x ."
        );
        assert_eq!(
            euclid(4, 16, "kick", None).unwrap(),
            "kick . . .  kick . . .  kick . . .  kick . . ."
        );
        assert_eq!(
            euclid(5, 8, "3^7", None).unwrap(),
            "3^7 . 3^7 .  3^7 3^7 . 3^7  3^7 . 3^7 .  3^7 3^7 . 3^7"
        );
        // n que no divide a 16: se repite y se corta.
        assert_eq!(
            euclid(2, 5, "x", None).unwrap(),
            "x . . x  . x . .  x . x .  . x . x"
        );
        assert_eq!(
            euclid(0, 4, "x", None).unwrap(),
            ". . . .  . . . .  . . . .  . . . ."
        );
        assert_eq!(euclid(16, 16, "x", None).unwrap().matches('x').count(), 16);
        assert!(
            euclid(3, 65, "x", None)
                .unwrap_err()
                .contains("n va de 1 a 64")
        );
        // Con otra longitud: 12 pasos (un 3 contra 4) o 32.
        assert_eq!(
            euclid(3, 12, "x", Some(12)).unwrap(),
            "x . . .  x . . .  x . . ."
        );
        assert_eq!(
            euclid(5, 32, "x", Some(32)).unwrap().matches('x').count(),
            5
        );
        assert!(
            euclid(3, 8, "x", Some(0))
                .unwrap_err()
                .contains("los pasos van de 1 a 64")
        );
        assert!(
            euclid(9, 8, "x", None)
                .unwrap_err()
                .contains("k va de 0 a n (8)")
        );
        assert!(euclid(3, 8, ".", None).unwrap_err().contains("el golpe"));
        assert!(euclid(3, 8, "1 2", None).unwrap_err().contains("el golpe"));
    }

    #[test]
    fn rotation_moves_steps_later_or_earlier() {
        let p = "1 2 3 4  5 6 7 8  9 10 11 12  13 14 15 16";
        assert_eq!(
            rotate(p, 1).unwrap(),
            "16 1 2 3  4 5 6 7  8 9 10 11  12 13 14 15"
        );
        assert_eq!(
            rotate(p, -1).unwrap(),
            "2 3 4 5  6 7 8 9  10 11 12 13  14 15 16 1"
        );
        assert_eq!(rotate(p, 16).unwrap(), p);
        assert_eq!(rotate(p, 35).unwrap(), rotate(p, 3).unwrap());
        // Cualquier longitud (la cuenta `steps` con la de la pista).
        assert_eq!(rotate("1 2 3", 1).unwrap(), "3 1 2");
        assert_eq!(
            rotate("", 1).unwrap_err(),
            "rotate(patrón, 1): el patrón necesita de 1 a 64 pasos, tiene 0"
        );
    }

    #[test]
    fn reversing_a_pattern_reverses_the_step_order() {
        let p = "1 2' 3 4  5 6 7 8  9 10 11 12  13 14 15 kick^7";
        assert_eq!(
            reverse_pattern(p).unwrap(),
            "kick^7 15 14 13  12 11 10 9  8 7 6 5  4 3 2' 1"
        );
        assert_eq!(reverse_pattern(&reverse_pattern(p).unwrap()).unwrap(), p);
        assert_eq!(reverse_pattern("1 2 3").unwrap(), "3 2 1");
        assert!(
            reverse_pattern(" ")
                .unwrap_err()
                .starts_with("reverse_pattern: ")
        );
    }

    #[test]
    fn neighbouring_seeds_give_different_numbers() {
        // (Antes, 42 y 43 daban lo mismo: solo cambiaba el bit que se fuerza a 1.)
        for seed in [0u64, 4, 42, 1 << 40] {
            assert_ne!(
                Rng::new(seed).next_f64(),
                Rng::new(seed + 1).next_f64(),
                "{seed}"
            );
        }
    }

    #[test]
    fn mutation_keeps_the_vocabulary_and_follows_the_seed() {
        let p = "1 . 2 .  3 . 4 .  5 6 2 .  7 . 8 .";
        let a = mutate(p, 0.5, &mut Rng::new(3)).unwrap();
        assert_eq!(
            a,
            mutate(p, 0.5, &mut Rng::new(3)).unwrap(),
            "misma semilla"
        );
        assert_ne!(a, mutate(p, 0.5, &mut Rng::new(4)).unwrap());
        assert_ne!(a, p);
        let original = tokens(p, "").unwrap();
        assert!(tokens(&a, "").unwrap().iter().all(|t| original.contains(t)));
        assert_eq!(mutate(p, 0.0, &mut Rng::new(3)).unwrap(), p);
        assert!(
            mutate(p, 1.5, &mut Rng::new(3))
                .unwrap_err()
                .contains("de 0 a 1")
        );
    }

    #[test]
    fn shuffling_permutes_the_steps_and_follows_the_seed() {
        let p = "1 2 3 4  5 6 7 8  9 10 11 12  13 14 15 16";
        let a = shuffle(p, &mut Rng::new(1)).unwrap();
        assert_eq!(a, shuffle(p, &mut Rng::new(1)).unwrap());
        assert_ne!(a, shuffle(p, &mut Rng::new(2)).unwrap());
        assert_ne!(a, p);
        let mut sorted = tokens(&a, "").unwrap();
        sorted.sort_by_key(|t| t.parse::<u32>().unwrap());
        assert_eq!(format(&sorted), p, "los mismos pasos");
        assert!(
            shuffle("", &mut Rng::new(1))
                .unwrap_err()
                .starts_with("shuffle: ")
        );
    }

    #[test]
    fn scale_degrees_become_semitones() {
        let minor = crate::api::scale("minor").unwrap().intervals;
        let major = crate::api::scale("major").unwrap().intervals;
        let got: Vec<i32> = [1, 2, 3, 4, 5, 6, 7, 8, 10, -1, -7, -8]
            .iter()
            .map(|&d| scale_pitch(minor, 0, d).unwrap())
            .collect();
        assert_eq!(got, [0, 2, 3, 5, 7, 8, 10, 12, 15, -2, -12, -14]);
        assert_eq!(scale_pitch(major, 2, 3).unwrap(), 6, "en re");
        assert!(scale_pitch(major, 0, 0).unwrap_err().contains("grado 0"));
        let penta = crate::api::scale("minor_pentatonic").unwrap().intervals;
        assert_eq!(scale_pitch(penta, 0, 6).unwrap(), 12);
    }

    #[test]
    fn melodies_from_degrees() {
        let minor = crate::api::scale("minor").unwrap().intervals;
        assert_eq!(
            melody(minor, 0, "1 3 5 3  . . . .  . . . .  . . . .", "1").unwrap(),
            "1^0 1^3 1^7 1^3  . . . .  . . . .  . . . ."
        );
        assert_eq!(
            melody(
                minor,
                3,
                "1@0.5 -1' 8r2 .  . . . .  . . . .  . . . .",
                "kick"
            )
            .unwrap(),
            "kick^3@0.5 kick^1' kick^15r2 .  . . . .  . . . .  . . . ."
        );
        let e = |d: &str, hit: &str| {
            melody(
                minor,
                0,
                &format!("{d} . . .  . . . .  . . . .  . . . ."),
                hit,
            )
            .unwrap_err()
        };
        assert!(e("x", "1").contains("paso 1 (\"x\"): debe empezar por un grado"));
        assert!(e("3^2", "1").contains("sin ^"));
        assert!(e("0", "1").contains("grado 0"));
        assert!(e("30", "1").contains("da 50 semitonos (máximo ±36)"));
        assert!(e("1", "1^5").contains("el golpe no puede llevar ^"));
        assert!(e("1", ".").contains("el golpe"));
    }

    #[test]
    fn tokens_need_one_to_sixty_four_steps() {
        assert_eq!(
            tokens("1 . 2 .  . . . .  . . . .  . . . .", "x")
                .unwrap()
                .len(),
            16
        );
        assert_eq!(tokens("1 .", "rotate").unwrap().len(), 2);
        assert_eq!(
            tokens(&". ".repeat(65), "rotate").unwrap_err(),
            "rotate: el patrón necesita de 1 a 64 pasos, tiene 65"
        );
    }

    #[test]
    fn note_names_and_numbers() {
        assert_eq!(note_number("C4"), Some(60));
        assert_eq!(note_number("c3"), Some(48));
        assert_eq!(note_number("A"), Some(69));
        assert_eq!(note_number("F#2"), Some(42));
        assert_eq!(note_number("bb2"), Some(46));
        assert_eq!(note_number("Cb4"), Some(59));
        assert_eq!(note_number("C-1"), Some(0));
        for bad in ["", "h", "C10", "C#x", "4"] {
            assert_eq!(note_number(bad), None, "{bad}");
        }
        assert_eq!(note_name(60), "C4");
        assert_eq!(note_name(61), "C#4");
        assert_eq!(note_name(47), "B2");
        assert_eq!(note_name(0), "C-1");
    }

    #[test]
    fn notes_become_steps_relative_to_the_root() {
        // Raíz C3: sin octava, la de la raíz.
        assert_eq!(
            notes("c e g e  . _ c4 b2  . . . .  . . . .", 48).unwrap(),
            "1^0 1^4 1^7 1^4  . _ 1^12 1^-1  . . . .  . . . ."
        );
        // Raíz A2 (45): "c" es C2, por debajo.
        assert_eq!(notes("c a", 45).unwrap(), "1^-9 1^0");
        // Acordes: las notas sin octava, por encima de la anterior.
        assert_eq!(
            notes("c+e+g g+b+d a3+c", 48).unwrap(),
            "1^0^4^7 1^7^11^14 1^9^12"
        );
        // Sufijos (menos ^) tal cual.
        assert_eq!(notes("c@0.5r2 e'~800", 60).unwrap(), "1^0@0.5r2 1^4'~800");
        for (bad, why) in [
            ("h", "debe ser una nota"),
            ("c^7", "sin ^"),
            ("c+e+g+b+d", "como mucho 4"),
            ("c7", "48 semitonos"),
            ("c!", "inesperado"),
        ] {
            let err = notes(bad, 48).unwrap_err();
            assert!(err.contains(why) && err.contains("paso 1"), "{bad}: {err}");
        }
    }
}
