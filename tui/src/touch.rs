//! Grabar una lane moviendo su parámetro («touch»), sin interfaz: una toma
//! son los valores que se han tocado en cada posición de la lane (su fase,
//! la del cursor de reproducción); al acabar se simplifica (sin los puntos
//! que no cambian la curva) y sustituye a los puntos de la lane en el tramo
//! que se tocó.

use engine::LaneCurve;

/// Lo que se desvía como mucho la curva simplificada de la tocada, en
/// fracción de la altura de la escala.
pub const TOLERANCE: f32 = 0.01;

/// Si una toma pasa del final de la lane al principio (en `loop`), se
/// queda la última vuelta: un salto atrás de más de esta fracción de la
/// lane es una vuelta nueva.
const WRAP: f64 = 0.5;

/// Los puntos que dicen lo mismo que `samples` (posición y nivel de 0 a 1)
/// a menos de `tolerance` (Ramer-Douglas-Peucker): siempre el primero y el
/// último.
pub fn simplify(samples: &[(f64, f32)], tolerance: f32) -> Vec<(f64, f32)> {
    if samples.len() <= 2 {
        return samples.to_vec();
    }
    let mut keep = vec![false; samples.len()];
    keep[0] = true;
    keep[samples.len() - 1] = true;
    let mut stack = vec![(0, samples.len() - 1)];
    while let Some((a, b)) = stack.pop() {
        let (pa, pb) = (samples[a], samples[b]);
        let span = pb.0 - pa.0;
        let mut worst = (0.0f32, a);
        for (i, p) in samples.iter().enumerate().take(b).skip(a + 1) {
            let t = if span > 0.0 {
                ((p.0 - pa.0) / span) as f32
            } else {
                0.0
            };
            let line = pa.1 + t * (pb.1 - pa.1);
            let off = (p.1 - line).abs();
            if off > worst.0 {
                worst = (off, i);
            }
        }
        if worst.0 > tolerance {
            keep[worst.1] = true;
            stack.push((a, worst.1));
            stack.push((worst.1, b));
        }
    }
    samples
        .iter()
        .zip(keep)
        .filter_map(|(p, k)| k.then_some(*p))
        .collect()
}

/// La última vuelta de una toma, en orden de posición, sin repetir
/// posiciones (la última que se tocó en cada una).
pub fn last_pass(take: &[(f64, f32)], length: f64) -> Vec<(f64, f32)> {
    let start = take
        .windows(2)
        .rposition(|w| w[0].0 - w[1].0 > WRAP * length)
        .map_or(0, |i| i + 1);
    let mut pass: Vec<(f64, f32)> = Vec::new();
    for &(at, v) in &take[start..] {
        match pass.last_mut() {
            Some(last) if last.0 == at => last.1 = v,
            Some(last) if at < last.0 => {
                // Se volvió atrás dentro de la vuelta: lo de después se
                // vuelve a tocar.
                pass.retain(|p| p.0 < at);
                pass.push((at, v));
            }
            _ => pass.push((at, v)),
        }
    }
    pass
}

/// Los puntos de la lane (`points`, en pasos y con su valor) con la toma
/// (`take`, posición en pasos y valor) en su tramo: lo de antes y lo de
/// después se queda; lo de dentro, la toma simplificada (con `level`, la
/// altura de 0 a 1 de un valor, y `value`, al revés), en rectas. Como mucho
/// `max` puntos: si no caben, se simplifica más.
pub fn merge(
    points: &[(f64, f32, LaneCurve)],
    take: &[(f64, f32)],
    length: f64,
    level: impl Fn(f32) -> f32,
    value: impl Fn(f32) -> f32,
    max: usize,
) -> Vec<(f64, f32, LaneCurve)> {
    let pass = last_pass(take, length);
    let (Some(first), Some(last)) = (pass.first(), pass.last()) else {
        return points.to_vec();
    };
    let (from, to) = (first.0, last.0);
    let outside: Vec<(f64, f32, LaneCurve)> = points
        .iter()
        .copied()
        .filter(|p| p.0 < from || p.0 > to)
        .collect();
    let levels: Vec<(f64, f32)> = pass.iter().map(|&(at, v)| (at, level(v))).collect();
    let mut tolerance = TOLERANCE;
    loop {
        let simple = simplify(&levels, tolerance);
        if simple.len() + outside.len() <= max || tolerance > 1.0 {
            let mut out = outside.clone();
            out.extend(
                simple
                    .iter()
                    .map(|&(at, l)| (at, value(l), LaneCurve::Linear)),
            );
            out.sort_by(|a, b| a.0.total_cmp(&b.0));
            out.dedup_by(|a, b| a.0 == b.0);
            out.truncate(max);
            return out;
        }
        tolerance *= 2.0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_straight_line_is_two_points_and_a_corner_three() {
        let line: Vec<(f64, f32)> = (0..=16).map(|i| (i as f64, i as f32 / 16.0)).collect();
        assert_eq!(simplify(&line, 0.01), [(0.0, 0.0), (16.0, 1.0)]);
        let corner: Vec<(f64, f32)> = (0..=16)
            .map(|i| (i as f64, if i <= 8 { i as f32 / 8.0 } else { 1.0 }))
            .collect();
        assert_eq!(
            simplify(&corner, 0.01),
            [(0.0, 0.0), (8.0, 1.0), (16.0, 1.0)]
        );
        // Lo que se desvía menos que la tolerancia, fuera.
        let wobble = [(0.0, 0.0), (1.0, 0.005), (2.0, 0.0)];
        assert_eq!(simplify(&wobble, 0.01), [(0.0, 0.0), (2.0, 0.0)]);
    }

    #[test]
    fn only_the_last_pass_and_the_last_touch_at_each_place() {
        // Del paso 28 al 31, vuelta, del 0 al 2: la última vuelta.
        let take = [(28.0, 0.1), (31.0, 0.2), (0.0, 0.5), (1.0, 0.6), (2.0, 0.7)];
        assert_eq!(last_pass(&take, 32.0), [(0.0, 0.5), (1.0, 0.6), (2.0, 0.7)]);
        // Volver atrás un poco: lo de después se toca otra vez.
        let take = [(4.0, 0.1), (5.0, 0.2), (6.0, 0.3), (5.0, 0.9), (5.0, 0.8)];
        assert_eq!(last_pass(&take, 32.0), [(4.0, 0.1), (5.0, 0.8)]);
    }

    #[test]
    fn the_take_replaces_the_points_in_its_stretch() {
        let points = [
            (0.0, 0.0, LaneCurve::Step),
            (8.0, 0.5, LaneCurve::Linear),
            (24.0, 1.0, LaneCurve::Exp),
        ];
        // Una rampa tocada del paso 4 al 12.
        let take: Vec<(f64, f32)> = (4..=12).map(|i| (i as f64, (i - 4) as f32 / 8.0)).collect();
        let merged = merge(&points, &take, 32.0, |v| v, |l| l, 64);
        assert_eq!(
            merged,
            [
                (0.0, 0.0, LaneCurve::Step),
                (4.0, 0.0, LaneCurve::Linear),
                (12.0, 1.0, LaneCurve::Linear),
                (24.0, 1.0, LaneCurve::Exp),
            ]
        );
        // Sin toma, nada cambia.
        assert_eq!(merge(&points, &[], 32.0, |v| v, |l| l, 64), points);
    }

    #[test]
    fn a_busy_take_fits_in_the_points_there_are() {
        let take: Vec<(f64, f32)> = (0..400)
            .map(|i| (i as f64 * 0.08, ((i * 37) % 100) as f32 / 100.0))
            .collect();
        let merged = merge(&[], &take, 32.0, |v| v, |l| l, 64);
        assert!(merged.len() <= 64, "{}", merged.len());
        assert!(merged.windows(2).all(|w| w[0].0 < w[1].0));
    }
}
