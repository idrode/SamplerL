//! Funciones que generan o transforman patrones en mini-notación: reciben y
//! devuelven texto de 16 pasos que luego se pasa a `steps`. No conocen los
//! cortes de ninguna pista: cada paso se trata como una palabra (`3`,
//! `kick^7`, `.`), y lo valida `steps` al usarse.

use engine::STEPS;

/// Pasos de un patrón (exactamente 16), o un error con quién lo pidió.
pub fn tokens(text: &str, who: &str) -> Result<Vec<String>, String> {
    let tokens: Vec<String> = text.split_whitespace().map(String::from).collect();
    if tokens.len() != STEPS {
        return Err(format!(
            "{who}: el patrón necesita {STEPS} pasos, tiene {}",
            tokens.len()
        ));
    }
    Ok(tokens)
}

/// Patrón de 16 pasos agrupado por tiempos: `1 . 2 .  3 . 4 .  ...`.
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
/// Se repite hasta llenar los 16 pasos. `hit` es el paso que suena.
pub fn euclid(k: i64, n: i64, hit: &str) -> Result<String, String> {
    let who = format!("euclid({k}, {n})");
    if !(1..=STEPS as i64).contains(&n) {
        return Err(format!("{who}: n va de 1 a {STEPS}"));
    }
    if !(0..=n).contains(&k) {
        return Err(format!("{who}: k va de 0 a n ({n})"));
    }
    check_hit(hit, &who)?;
    let tokens: Vec<String> = (0..STEPS as i64)
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
        // El estado no puede ser 0.
        Rng(seed ^ 0x9e37_79b9_7f4a_7c15 | 1)
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

/// Cambia cada paso, con probabilidad `p`, por uno de los 16 del propio
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

/// Los mismos 16 pasos en otro orden al azar (Fisher-Yates).
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
    if hit.contains('^') {
        return Err(format!(
            "{who}: el golpe no puede llevar ^ (el pitch lo pone la escala): \"{hit}\""
        ));
    }
    let steps: Vec<String> = tokens
        .iter()
        .enumerate()
        .map(|(i, token)| {
            if token == "." {
                return Ok(".".to_string());
            }
            let sign = usize::from(token.starts_with('-'));
            let digits = token[sign..].chars().take_while(char::is_ascii_digit).count();
            let (number, suffixes) = token.split_at(sign + digits);
            let degree: i64 = number
                .parse()
                .map_err(|_| format!("{who}: paso {} (\"{token}\"): debe empezar por un grado (1, 2, -1...) o ser \".\"", i + 1))?;
            if suffixes.contains('^') {
                return Err(format!("{who}: paso {} (\"{token}\"): el pitch lo pone la escala, sin ^", i + 1));
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn euclidean_rhythms() {
        assert_eq!(
            euclid(3, 8, "x").unwrap(),
            "x . . x  . . x .  x . . x  . . x ."
        );
        assert_eq!(
            euclid(4, 16, "kick").unwrap(),
            "kick . . .  kick . . .  kick . . .  kick . . ."
        );
        assert_eq!(
            euclid(5, 8, "3^7").unwrap(),
            "3^7 . 3^7 .  3^7 3^7 . 3^7  3^7 . 3^7 .  3^7 3^7 . 3^7"
        );
        // n que no divide a 16: se repite y se corta.
        assert_eq!(
            euclid(2, 5, "x").unwrap(),
            "x . . x  . x . .  x . x .  . x . x"
        );
        assert_eq!(
            euclid(0, 4, "x").unwrap(),
            ". . . .  . . . .  . . . .  . . . ."
        );
        assert_eq!(euclid(16, 16, "x").unwrap().matches('x').count(), 16);
        assert!(euclid(3, 17, "x").unwrap_err().contains("n va de 1 a 16"));
        assert!(euclid(9, 8, "x").unwrap_err().contains("k va de 0 a n (8)"));
        assert!(euclid(3, 8, ".").unwrap_err().contains("el golpe"));
        assert!(euclid(3, 8, "1 2").unwrap_err().contains("el golpe"));
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
        assert_eq!(
            rotate("1 .", 1).unwrap_err(),
            "rotate(patrón, 1): el patrón necesita 16 pasos, tiene 2"
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
        assert!(
            reverse_pattern("1")
                .unwrap_err()
                .starts_with("reverse_pattern: ")
        );
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
            shuffle("1 2", &mut Rng::new(1))
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
    fn tokens_need_sixteen_steps() {
        assert_eq!(
            tokens("1 . 2 .  . . . .  . . . .  . . . .", "x")
                .unwrap()
                .len(),
            16
        );
        assert_eq!(
            tokens("1 .", "rotate").unwrap_err(),
            "rotate: el patrón necesita 16 pasos, tiene 2"
        );
    }
}
