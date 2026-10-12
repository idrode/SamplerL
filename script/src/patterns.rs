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
