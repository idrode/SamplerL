//! Los dispositivos desde la aplicación: lo que hace `crate::devices` se
//! aplica aquí (la cola nueva de la entrada MIDI, el destino nuevo de las
//! notas y los avisos en la línea de estado).

// El panel (parte 3) usa lo que aún no se llama.
#![allow(dead_code)]

use super::App;
use crate::devices::{Change, Devices, DevicesView, Kind};

impl App {
    /// Los dispositivos de esta ventana principal, con lo que haya que
    /// aplicar al abrirlos (las colas del MIDI y los avisos).
    pub fn set_devices(&mut self, devices: Devices, changes: Vec<Change>) {
        self.devices = Some(devices);
        self.apply_device_changes(changes);
    }

    fn apply_device_changes(&mut self, changes: Vec<Change>) {
        for change in changes {
            match change {
                Change::Message(text) => self.message = text,
                Change::Input(queue) => self.midi_in = Some(queue),
                Change::Output(sink) => self.midi_out = Some(sink),
            }
        }
    }

    /// Elige el dispositivo `name` de la lista `kind`.
    pub fn choose_device(&mut self, kind: Kind, name: &str) {
        let Some(devices) = &mut self.devices else {
            self.message = "esta ventana no tiene dispositivos".into();
            return;
        };
        let changes = devices.choose(kind, name);
        self.apply_device_changes(changes);
    }

    /// Vuelve a leer las listas de dispositivos.
    pub fn refresh_devices(&mut self) {
        let Some(devices) = &mut self.devices else {
            self.message = "esta ventana no tiene dispositivos".into();
            return;
        };
        let changes = devices.refresh();
        self.apply_device_changes(changes);
    }

    /// En cada vuelta: termina un cambio de audio y se recupera si un
    /// dispositivo se fue.
    pub(super) fn poll_devices(&mut self) {
        let failed = self.midi_out.as_ref().is_some_and(|s| s.failed());
        let Some(devices) = &mut self.devices else {
            return;
        };
        let changes = devices.poll(failed);
        self.apply_device_changes(changes);
    }

    /// Lo que se ve en el panel de dispositivos.
    pub fn devices_view(&self) -> Option<DevicesView> {
        self.devices.as_ref().map(Devices::view)
    }

    /// Lo elegido ahora (para guardarlo).
    pub fn device_choice(&self) -> Option<crate::devices::Choice> {
        self.devices.as_ref().map(|d| d.choice().clone())
    }
}
