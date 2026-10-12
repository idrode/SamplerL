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
