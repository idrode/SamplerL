//! Tap tempo, sin terminal: el tempo de los últimos golpes (media de los
//! intervalos), con los instantes que se le pasen (así se prueba sin
//! esperar).

use std::time::{Duration, Instant};

/// Intervalos que se promedian como mucho.
const TAPS: usize = 8;
/// Un golpe más tarde que esto empieza otra cuenta.
const RESET: Duration = Duration::from_secs(2);

#[derive(Debug, Clone, Default)]
pub struct TapTempo {
    taps: Vec<Instant>,
}

impl TapTempo {
    /// Un golpe en `now`. Devuelve el tempo (con un decimal) cuando hay al
    /// menos dos golpes seguidos.
    pub fn tap(&mut self, now: Instant) -> Option<f64> {
        if self
            .taps
            .last()
            .is_some_and(|last| now.saturating_duration_since(*last) > RESET)
        {
            self.taps.clear();
        }
        self.taps.push(now);
        if self.taps.len() > TAPS + 1 {
            self.taps.remove(0);
        }
        self.bpm()
    }

    /// Tempo de los golpes que hay (media de sus intervalos).
    pub fn bpm(&self) -> Option<f64> {
        let (first, last) = (self.taps.first()?, self.taps.last()?);
        let intervals = self.taps.len() - 1;
        if intervals == 0 {
            return None;
        }
        let seconds = last.duration_since(*first).as_secs_f64() / intervals as f64;
        (seconds > 0.0).then(|| (600.0 / seconds).round() / 10.0)
    }

    pub fn count(&self) -> usize {
        self.taps.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn averages_the_last_taps_and_resets_after_a_pause() {
        let t0 = Instant::now();
        let ms = |n: u64| t0 + Duration::from_millis(n);
        let mut tap = TapTempo::default();
        assert_eq!(tap.tap(ms(0)), None);
        // Golpes a 500 ms ± 20: 120 BPM de media.
        let jitter = [0, 520, 980, 1510, 2000, 2490, 3020, 3500];
        let mut bpm = None;
        for &t in &jitter[1..] {
            bpm = tap.tap(ms(t));
        }
        assert_eq!(bpm, Some(120.0));
        // Solo cuentan los últimos 8 intervalos.
        for k in 1..=8 {
            bpm = tap.tap(ms(3500 + 400 * k));
        }
        assert_eq!(bpm, Some(150.0));
        // Tras más de 2 s, otra cuenta.
        assert_eq!(tap.tap(ms(20_000)), None);
        assert_eq!(tap.count(), 1);
        assert_eq!(tap.tap(ms(20_350)), Some(171.4));
    }
}
