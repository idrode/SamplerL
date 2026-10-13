//! Grabar lo tocado, sin terminal: los cortes que se disparan en vivo
//! (pads o MIDI) se cuantizan al paso más cercano de la rejilla de la
//! canción durante N compases y salen como un patrón en mini-notación.

use script::patterns;

/// Una grabación en curso.
#[derive(Debug, Clone, PartialEq)]
pub struct Recording {
    /// La pista que se graba (la seleccionada al empezar).
    pub track: String,
    /// Semicorchea de la canción en que empieza (el principio de un
    /// compás) y cuántos pasos dura.
    start: u64,
    pub steps: usize,
    /// Pasos de un compás (para decir por qué compás va).
    bar_steps: usize,
    /// Lo grabado en cada paso: corte (desde 0) y volumen. Si caen dos
    /// golpes en el mismo paso, se queda el último.
    hits: Vec<Option<(usize, f32)>>,
}

impl Recording {
    /// Una grabación de `bars` compases de `bar_steps` pasos que empieza en
    /// la semicorchea `start`.
    pub fn new(track: &str, start: u64, bars: usize, bar_steps: usize) -> Recording {
        Recording {
            track: track.to_string(),
            start,
            steps: bars * bar_steps,
            bar_steps,
            hits: vec![None; bars * bar_steps],
        }
    }

    /// El paso (desde 0) en que cae un golpe tocado en la posición
    /// `position` (pasos de la canción con decimales): el más cercano. Un
    /// golpe un poco antes de empezar (menos de medio paso) cuenta como el
    /// primero.
    pub fn step_at(&self, position: f64) -> Option<usize> {
        let k = (position - self.start as f64).round();
        (k >= 0.0 && (k as usize) < self.steps).then_some(k as usize)
    }

    /// Apunta un golpe del corte `slice` con volumen `gain` tocado en
    /// `position`. Devuelve el paso, si cae dentro.
    pub fn hit(&mut self, position: f64, slice: usize, gain: f32) -> Option<usize> {
        let step = self.step_at(position)?;
        self.hits[step] = Some((slice, gain));
        Some(step)
    }

    /// Ya ha pasado el último paso (hasta medio paso después, un golpe
    /// todavía caería en él).
    pub fn done(&self, position: f64) -> bool {
        position >= self.start as f64 + self.steps as f64 - 0.5
    }

    /// Todavía no ha empezado (es la cuenta atrás hasta el compás).
    pub fn waiting(&self, position: f64) -> bool {
        position < self.start as f64 - 0.5
    }

    /// Compás de la grabación (desde 1) en `position`.
    pub fn bar(&self, position: f64) -> usize {
        let into = (position - self.start as f64).max(0.0) as usize;
        (into / self.bar_steps + 1).min(self.steps / self.bar_steps)
    }

    /// Lo grabado en mini-notación: cada golpe con la etiqueta de su corte
    /// (o su número) y `@volumen` si no es 1; agrupado por tiempos.
    pub fn notation(&self, labels: &[Option<String>]) -> String {
        let tokens: Vec<String> = self
            .hits
            .iter()
            .map(|hit| match hit {
                None => ".".to_string(),
                Some((slice, gain)) => {
                    let mut token = labels
                        .get(*slice)
                        .cloned()
                        .flatten()
                        .unwrap_or_else(|| (slice + 1).to_string());
                    let gain = (gain * 100.0).round() / 100.0;
                    if gain < 1.0 {
                        token.push_str(&format!("@{gain}"));
                    }
                    token
                }
            })
            .collect();
        patterns::format(&tokens)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hits_are_quantized_to_the_nearest_step() {
        // Empieza en el compás 2 (semicorchea 32) y dura un compás.
        let mut rec = Recording::new("drums", 32, 1, 16);
        assert!(rec.waiting(31.0));
        assert!(!rec.waiting(31.6));
        assert_eq!(rec.hit(31.7, 0, 1.0), Some(0), "un poco antes: el primero");
        assert_eq!(rec.hit(36.4, 1, 0.5), Some(4));
        assert_eq!(rec.hit(37.6, 2, 0.757), Some(6));
        assert_eq!(rec.hit(47.4, 0, 1.0), Some(15));
        assert_eq!(rec.hit(47.6, 0, 1.0), None, "fuera");
        assert_eq!(rec.hit(20.0, 0, 1.0), None);
        // El mismo paso dos veces: el último.
        rec.hit(38.1, 3, 1.0);
        assert!(!rec.done(47.4));
        assert!(rec.done(47.5));
        assert_eq!(rec.notation(&[]), "1 . . .  2@0.5 . 4 .  . . . .  . . . 1");
        let labels = vec![Some("kick".into()), None, Some("hat".into()), None];
        assert_eq!(
            rec.notation(&labels),
            "kick . . .  2@0.5 . 4 .  . . . .  . . . kick"
        );
    }

    #[test]
    fn several_bars_and_other_meters() {
        let mut rec = Recording::new("d", 12, 2, 12);
        assert_eq!(rec.steps, 24);
        assert_eq!(rec.hit(12.0, 0, 0.756), Some(0));
        assert_eq!(rec.hit(30.0, 1, 1.0), Some(18));
        assert_eq!(rec.bar(20.0), 1);
        assert_eq!(rec.bar(25.0), 2);
        assert_eq!(
            rec.notation(&[]),
            "1@0.76 . . .  . . . .  . . . .  . . . .  . . 2 .  . . . ."
        );
    }
}
