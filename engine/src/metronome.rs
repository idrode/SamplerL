//! Metrónomo: un clic corto en cada pulso (pasos 0, 4, 8 y 12) con acento
//! en el primero del compás. Se genera en el hilo de audio sin reservas:
//! un seno con una caída exponencial, que empieza al frame exacto del paso.

/// Duración del clic.
const CLICK_SECONDS: f32 = 0.03;
/// Frecuencia y nivel del clic con acento (primer pulso) y del normal.
pub const ACCENT_HZ: f32 = 1600.0;
pub const ACCENT_GAIN: f32 = 0.5;
pub const BEAT_HZ: f32 = 1000.0;
pub const BEAT_GAIN: f32 = 0.3;

#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct Metronome {
    pub enabled: bool,
    /// Frames que le quedan al clic que suena.
    left: u32,
    total: u32,
    phase: f32,
    step: f32,
    gain: f32,
}

impl Metronome {
    /// Empieza un clic si toca pulso en este paso (y está encendido).
    pub fn on_step(&mut self, step: usize, sample_rate: u32) {
        if !self.enabled || !step.is_multiple_of(4) {
            return;
        }
        let (hz, gain) = if step == 0 {
            (ACCENT_HZ, ACCENT_GAIN)
        } else {
            (BEAT_HZ, BEAT_GAIN)
        };
        self.total = (CLICK_SECONDS * sample_rate as f32) as u32;
        self.left = self.total;
        self.phase = 0.0;
        self.step = std::f32::consts::TAU * hz / sample_rate as f32;
        self.gain = gain;
    }

    /// Suma el clic (si suena) a un buffer estéreo entrelazado.
    pub fn mix_into(&mut self, out: &mut [f32]) {
        for frame in out.as_chunks_mut::<2>().0 {
            if self.left == 0 {
                return;
            }
            // Caída exponencial: de 1 a ~0,002 en la duración del clic.
            let t = 1.0 - self.left as f32 / self.total as f32;
            let s = self.gain * (-6.0 * t).exp() * self.phase.sin();
            frame[0] += s;
            frame[1] += s;
            self.phase += self.step;
            if self.phase > std::f32::consts::TAU {
                self.phase -= std::f32::consts::TAU;
            }
            self.left -= 1;
        }
    }

    pub fn stop(&mut self) {
        self.left = 0;
    }
}
