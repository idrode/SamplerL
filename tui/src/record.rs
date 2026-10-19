//! Grabar lo tocado, sin terminal: los cortes que se disparan en vivo
//! (pads o MIDI) se cuantizan al paso más cercano de la rejilla de la
//! canción durante N compases y salen como un patrón en mini-notación. Las
//! notas de un instrumento salen con su tono (`1^4`) y lo que duran hasta
//! su note off, con `_`.

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
    /// Lo grabado en cada paso. Si caen dos golpes en el mismo paso, se
    /// queda el último.
    hits: Vec<Option<Hit>>,
}

/// Un golpe grabado.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Hit {
    /// Corte (desde 0).
    slice: usize,
    gain: f32,
    /// Nota de un instrumento (o de una nube con un solo corte): semitonos
    /// sobre la raíz.
    semitones: Option<f32>,
    /// Un corte de una nube de granos: suena mientras se mantiene la tecla,
    /// como una nota.
    held: bool,
    /// Pasos que sigue sonando después del suyo (hasta el note off).
    hold: usize,
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
        self.put(position, slice, gain, None)
    }

    /// Apunta un corte de una nube de granos tocado en `position`: dura
    /// hasta que se suelte ([`Recording::release`]).
    pub fn hit_held(&mut self, position: f64, slice: usize, gain: f32) -> Option<usize> {
        let step = self.put(position, slice, gain, None)?;
        if let Some(Some(hit)) = self.hits.get_mut(step) {
            hit.held = true;
        }
        Some(step)
    }

    /// Apunta una nota de un instrumento (`semitones` sobre la raíz) tocada
    /// en `position`. Devuelve el paso, para [`Recording::release`].
    pub fn note(&mut self, position: f64, semitones: f32, gain: f32) -> Option<usize> {
        self.put(position, 0, gain, Some(semitones))
    }

    /// La nota apuntada en el paso `step` se soltó en `position`: dura hasta
    /// el paso más cercano (al menos el suyo).
    pub fn release(&mut self, step: usize, position: f64) {
        let end = (position - self.start as f64).round().max(0.0) as usize;
        if let Some(Some(hit)) = self.hits.get_mut(step)
            && (hit.semitones.is_some() || hit.held)
        {
            hit.hold = end.saturating_sub(step + 1).min(self.steps - step - 1);
        }
    }

    fn put(
        &mut self,
        position: f64,
        slice: usize,
        gain: f32,
        semitones: Option<f32>,
    ) -> Option<usize> {
        let step = self.step_at(position)?;
        self.hits[step] = Some(Hit {
            slice,
            gain,
            semitones,
            held: false,
            hold: 0,
        });
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
        // Pasos que alarga la nota de antes (hasta el siguiente golpe).
        let mut held = 0;
        let tokens: Vec<String> = self
            .hits
            .iter()
            .map(|hit| match hit {
                None if held > 0 => {
                    held -= 1;
                    "_".to_string()
                }
                None => ".".to_string(),
                Some(hit) => {
                    held = hit.hold;
                    let mut token = match hit.semitones {
                        Some(st) => format!("1^{}", (st * 100.0).round() / 100.0),
                        None => labels
                            .get(hit.slice)
                            .cloned()
                            .flatten()
                            .unwrap_or_else(|| (hit.slice + 1).to_string()),
                    };
                    let gain = (hit.gain * 100.0).round() / 100.0;
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

    #[test]
    fn instrument_notes_keep_their_pitch_and_length() {
        let mut rec = Recording::new("lead", 16, 1, 16);
        let c = rec.note(16.1, 0.0, 1.0).unwrap();
        rec.release(c, 19.8);
        let e = rec.note(20.4, 4.0, 0.5).unwrap();
        // Soltada antes de su paso siguiente: solo su paso.
        rec.release(e, 20.9);
        let g = rec.note(24.0, 7.0, 1.0).unwrap();
        // Otra nota antes de soltarla corta los `_`.
        rec.release(g, 31.0);
        rec.note(26.0, -5.0, 1.0);
        assert_eq!(
            rec.notation(&[]),
            "1^0 _ _ _  1^4@0.5 . . .  1^7 _ 1^-5 .  . . . ."
        );
        // Sin soltar: un paso. Soltada después del final: hasta el final.
        let mut rec = Recording::new("lead", 0, 1, 16);
        rec.note(0.0, 0.0, 1.0);
        let last = rec.note(14.0, 2.0, 1.0).unwrap();
        rec.release(last, 40.0);
        assert_eq!(rec.notation(&[]), "1^0 . . .  . . . .  . . . .  . . 1^2 _");
    }

    #[test]
    fn grain_slices_last_while_the_key_is_held_and_cuts_are_not_held() {
        let mut rec = Recording::new("nube", 0, 1, 16);
        let kick = rec.hit_held(0.2, 0, 1.0).unwrap();
        rec.release(kick, 3.1);
        let snare = rec.hit_held(8.0, 2, 0.5).unwrap();
        rec.release(snare, 9.4);
        // Un corte normal no se alarga aunque se llame a release.
        let plain = rec.hit(12.0, 1, 1.0).unwrap();
        rec.release(plain, 15.0);
        let labels = vec![Some("kick".into()), None, Some("hat".into())];
        assert_eq!(
            rec.notation(&labels),
            "kick _ _ .  . . . .  hat@0.5 . . .  2 . . ."
        );
    }
}
