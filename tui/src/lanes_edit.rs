//! Editar una lane desde su panel, sin interfaz: añadir, mover, borrar y
//! cambiar la curva de un punto, y afinarlo con las teclas. Cada edición da
//! los puntos nuevos tal como se escriben en el código (posición en
//! compases, valor en sus unidades o de 0 a 1 con `range`) y qué punto
//! queda elegido; quien la aplica los escribe en la tabla de `auto()` (ver
//! `script::points_text`).
//!
//! Las posiciones van por pasos enteros (la rejilla del secuenciador),
//! crecientes y sin repetir, dentro de lo que dura la lane; los valores,
//! dentro de la escala de la vista. Una lane se queda con al menos un
//! punto.

use engine::LaneCurve;

use crate::lanes_panel::LaneRow;

/// Lo que se le hace a una lane.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum LaneEdit {
    /// Un punto nuevo en `at` pasos con `value` (curva recta).
    Add {
        at: f64,
        value: f32,
    },
    /// El punto `index` a `at` pasos y `value`.
    Move {
        index: usize,
        at: f64,
        value: f32,
    },
    Delete(usize),
    /// La curva del punto `index`, la siguiente: recta, exp, suave, salto.
    Cycle(usize),
    /// El punto `index`, `steps` pasos y `levels` cuarentavos de la escala.
    Nudge {
        index: usize,
        steps: i32,
        levels: i32,
    },
}

/// Lo que da una edición: los puntos tal como se escriben y el elegido.
#[derive(Debug, Clone, PartialEq)]
pub struct Edited {
    pub points: Vec<(f64, f32, LaneCurve)>,
    pub selected: usize,
    /// Lo que se ha hecho (para el mensaje).
    pub what: &'static str,
}

/// Cuarentavos de la escala que mueve una flecha.
const NUDGE_LEVELS: f32 = 40.0;

/// La curva que sigue a `curve`.
pub fn next_curve(curve: LaneCurve) -> LaneCurve {
    match curve {
        LaneCurve::Linear => LaneCurve::Exp,
        LaneCurve::Exp => LaneCurve::Smooth,
        LaneCurve::Smooth => LaneCurve::Step,
        LaneCurve::Step => LaneCurve::Linear,
    }
}

/// Aplica `edit` a la lane de `row`: los puntos nuevos o por qué no.
pub fn apply(row: &LaneRow, edit: LaneEdit) -> Result<Edited, String> {
    // Los puntos de verdad (en pasos y en las unidades del parámetro).
    let mut points: Vec<(f64, f32, LaneCurve)> = row
        .lane
        .points()
        .iter()
        .map(|p| (p.at, p.value, p.curve))
        .collect();
    let length = row.lane.length();
    let (lo, hi, _) = row.scale;
    let clamp_value = |v: f32| v.clamp(lo.min(hi), hi.max(lo));
    let snap = |at: f64| at.round().clamp(0.0, length);
    let (selected, what) = match edit {
        LaneEdit::Add { at, value } => {
            let at = snap(at);
            if points.iter().any(|p| p.0 == at) {
                return Err("ya hay un punto en ese paso: arrástralo o elígelo".into());
            }
            if points.len() >= engine::MAX_LANE_POINTS {
                return Err(format!(
                    "una lane lleva como mucho {} puntos",
                    engine::MAX_LANE_POINTS
                ));
            }
            let index = points.partition_point(|p| p.0 < at);
            points.insert(index, (at, clamp_value(value), LaneCurve::Linear));
            (index, "punto añadido")
        }
        LaneEdit::Move { index, at, value } => {
            let at = moved(&points, index, at, length)?;
            let point = &mut points[index];
            point.0 = at;
            point.1 = clamp_value(value);
            (index, "punto movido")
        }
        LaneEdit::Delete(index) => {
            if index >= points.len() {
                return Err("ese punto no existe".into());
            }
            if points.len() == 1 {
                return Err(
                    "una lane se queda con un punto (para quitarla entera: pista:auto_clear)"
                        .into(),
                );
            }
            points.remove(index);
            (index.min(points.len() - 1), "punto borrado")
        }
        LaneEdit::Cycle(index) => {
            let point = points.get_mut(index).ok_or("ese punto no existe")?;
            point.2 = next_curve(point.2);
            (index, "curva cambiada")
        }
        LaneEdit::Nudge {
            index,
            steps,
            levels,
        } => {
            let &(at, value, _) = points.get(index).ok_or("ese punto no existe")?;
            let at = moved(&points, index, at + steps as f64, length)?;
            let level = row.level(value) + levels as f32 / NUDGE_LEVELS;
            let value = clamp_value(row.value_at(level));
            points[index].0 = at;
            points[index].1 = value;
            (index, "punto afinado")
        }
    };
    Ok(Edited {
        points: written_points(row, &points),
        selected,
        what,
    })
}

/// Los puntos de verdad (en pasos y en las unidades del parámetro) tal como
/// se escriben: en compases y, con `range`, de 0 a 1.
pub fn written_points(
    row: &LaneRow,
    points: &[(f64, f32, LaneCurve)],
) -> Vec<(f64, f32, LaneCurve)> {
    points
        .iter()
        .map(|&(at, value, curve)| (at / row.bar_steps as f64, written(row, value), curve))
        .collect()
}

/// Dónde queda el punto `index` llevado a `at`: en un paso entero, entre
/// sus vecinos (sin pisarlos) y dentro de la lane.
fn moved(
    points: &[(f64, f32, LaneCurve)],
    index: usize,
    at: f64,
    length: f64,
) -> Result<f64, String> {
    if index >= points.len() {
        return Err("ese punto no existe".into());
    }
    let low = if index == 0 {
        0.0
    } else {
        points[index - 1].0 + 1.0
    };
    let high = points.get(index + 1).map_or(length, |p| p.0 - 1.0);
    if low > high {
        return Ok(points[index].0);
    }
    Ok(at.round().clamp(low, high))
}

/// El valor como se escribe: en sus unidades o, con `range`, de 0 a 1.
fn written(row: &LaneRow, value: f32) -> f32 {
    let Some((lo, hi)) = row.range else {
        return round(value);
    };
    let t = if row.range_log && lo > 0.0 && hi > 0.0 && value > 0.0 {
        (value.ln() - lo.ln()) / (hi.ln() - lo.ln())
    } else {
        (value - lo) / (hi - lo)
    };
    round(t.clamp(0.0, 1.0))
}

/// Redondeado a lo que se lee: 4 cifras significativas.
fn round(value: f32) -> f32 {
    if value == 0.0 {
        return 0.0;
    }
    let digits = 3 - value.abs().log10().floor() as i32;
    let scale = 10f32.powi(digits.max(0));
    (value * scale).round() / scale
}

#[cfg(test)]
mod tests {
    use super::*;
    use engine::{Lane, LanePoint};

    fn row(points: &[(f64, f32, LaneCurve)], range: Option<(f32, f32)>) -> LaneRow {
        let lane = Lane::new(
            &points
                .iter()
                .map(|&(at, value, curve)| LanePoint { at, value, curve })
                .collect::<Vec<_>>(),
            32.0,
            false,
        )
        .unwrap();
        LaneRow {
            param: "gain".into(),
            index: 0,
            lane,
            bar_steps: 16,
            live: None,
            scale: (0.0, 4.0, false),
            range,
            range_log: false,
            editable: Err(String::new()),
            selected: None,
        }
    }

    fn two() -> LaneRow {
        row(
            &[(0.0, 1.0, LaneCurve::Linear), (16.0, 2.0, LaneCurve::Exp)],
            None,
        )
    }

    #[test]
    fn add_goes_to_its_step_in_order() {
        let e = apply(
            &two(),
            LaneEdit::Add {
                at: 8.4,
                value: 3.0,
            },
        )
        .unwrap();
        assert_eq!(
            e.points,
            [
                (0.0, 1.0, LaneCurve::Linear),
                (0.5, 3.0, LaneCurve::Linear),
                (1.0, 2.0, LaneCurve::Exp)
            ]
        );
        assert_eq!(e.selected, 1);
        assert!(
            apply(
                &two(),
                LaneEdit::Add {
                    at: 16.2,
                    value: 1.0
                }
            )
            .is_err()
        );
        // Fuera de la escala, al borde; fuera de la lane, a su final.
        let e = apply(
            &two(),
            LaneEdit::Add {
                at: 99.0,
                value: 9.0,
            },
        )
        .unwrap();
        assert_eq!(e.points[2], (2.0, 4.0, LaneCurve::Linear));
    }

    #[test]
    fn move_stays_between_its_neighbours() {
        let three = row(
            &[
                (0.0, 1.0, LaneCurve::Linear),
                (8.0, 1.0, LaneCurve::Linear),
                (16.0, 2.0, LaneCurve::Linear),
            ],
            None,
        );
        let e = apply(
            &three,
            LaneEdit::Move {
                index: 1,
                at: 30.0,
                value: 0.5,
            },
        )
        .unwrap();
        assert_eq!(e.points[1], (15.0 / 16.0, 0.5, LaneCurve::Linear));
        let e = apply(
            &three,
            LaneEdit::Move {
                index: 1,
                at: -5.0,
                value: 0.5,
            },
        )
        .unwrap();
        assert_eq!(e.points[1].0, 1.0 / 16.0);
    }

    #[test]
    fn delete_cycle_and_nudge() {
        let e = apply(&two(), LaneEdit::Delete(0)).unwrap();
        assert_eq!(e.points, [(1.0, 2.0, LaneCurve::Exp)]);
        let one = row(&[(0.0, 1.0, LaneCurve::Linear)], None);
        assert!(
            apply(&one, LaneEdit::Delete(0))
                .unwrap_err()
                .contains("auto_clear")
        );
        let e = apply(&two(), LaneEdit::Cycle(1)).unwrap();
        assert_eq!(e.points[1].2, LaneCurve::Smooth);
        assert_eq!(next_curve(LaneCurve::Step), LaneCurve::Linear);
        // Un paso a la derecha y un cuarentavo de la escala (0.1) arriba.
        let e = apply(
            &two(),
            LaneEdit::Nudge {
                index: 0,
                steps: 1,
                levels: 1,
            },
        )
        .unwrap();
        assert_eq!(e.points[0], (1.0 / 16.0, 1.1, LaneCurve::Linear));
    }

    #[test]
    fn with_range_the_values_are_written_from_0_to_1() {
        let mut r = row(&[(0.0, 1.0, LaneCurve::Linear)], Some((0.0, 4.0)));
        r.scale = (0.0, 4.0, false);
        let e = apply(
            &r,
            LaneEdit::Add {
                at: 16.0,
                value: 3.0,
            },
        )
        .unwrap();
        assert_eq!(
            e.points,
            [
                (0.0, 0.25, LaneCurve::Linear),
                (1.0, 0.75, LaneCurve::Linear)
            ]
        );
    }
}
