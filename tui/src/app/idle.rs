//! Cuándo no hace falta dibujar: con el transporte parado, sin entrada y
//! sin nada que suene, la pantalla no cambia y el bucle se salta los
//! cuadros (ver `run_terminal`). Lo que puede cambiarla sin una tecla
//! (una carga, un mensaje, el MIDI, los medidores...) entra en una huella;
//! lo que se mueve solo (la marcha, notas tenidas, grabaciones, avisos de
//! saturación) la anula, y entonces se dibuja al ritmo de siempre.

use std::hash::{Hash, Hasher};
use std::time::Instant;

use super::App;

impl App {
    /// La huella de lo que se ve en reposo, o `None` si algo se mueve solo
    /// (o es la ventana del editor, que sigue a la principal por el
    /// enlace). Dos huellas iguales: el cuadro saldría igual.
    pub fn idle_key(&self) -> Option<u64> {
        if self.editor_window() {
            return None;
        }
        let status = self.status?;
        if status.running
            || !self.held.is_empty()
            || self.recording.is_some()
            || self.touch.is_some()
        {
            return None;
        }
        let now = Instant::now();
        let holding = |until: &Option<Instant>| until.is_some_and(|u| now < u);
        if holding(&self.clipping_until)
            || holding(&self.limiting_until)
            || self.track_clip_until.iter().any(holding)
        {
            return None;
        }
        let mut h = std::collections::hash_map::DefaultHasher::new();
        // Los medidores como se dibujan (una cola que se apaga los mueve).
        let meters = self.meters.iter().chain(&self.group_meters);
        for m in meters.chain([&self.master_meter]) {
            ((m.clamp(0.0, 1.0) * 64.0) as u8).hash(&mut h);
        }
        (status.bar, status.step, status.bpm.to_bits(), status.hushed).hash(&mut h);
        (status.section_lost, status.heard.map(f64::to_bits)).hash(&mut h);
        self.message.hash(&mut h);
        self.error.as_ref().map(|e| &e.message).hash(&mut h);
        (self.loads, self.midi_events).hash(&mut h);
        Some(h.finish())
    }
}
