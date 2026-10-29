//! MIDI learn desde la interfaz:
//!
//! - «MIDI learn» (paleta) abre la lista de lo que se puede asignar
//!   (`script::learn::learnable`, filtrable); Enter lo elige y el siguiente
//!   CC que llega queda asignado (rango entero, curva logarítmica si el
//!   parámetro lo es). Un CC ya asignado pasa al parámetro nuevo.
//! - «Mandos MIDI» enseña las asignaciones (parámetro, CC, canal, rango y
//!   curva): ↑↓, Supr la borra, `c` cambia la curva, `l` le asigna otro
//!   mando, Enter deja escrita la orden `mando N mín máx curva` para el
//!   rango, Esc cierra.
//! - «Exportar los mandos MIDI a la configuración» los escribe en
//!   `[midi] mandos` de `config.toml`: son los de un proyecto sin los suyos.
//!
//! Los del proyecto se guardan en él (`mando = ...`). Cada cambio va al
//! motor como un lote (la tabla entra al compás) y los valores de los
//! mandos, ya (`ScriptHost::move_learned`).

use ratatui_textarea::{Input, Key};
use script::learn::{self, Learnable, Learned};

use super::{App, Flow};
use crate::model::{BlockPickerView, KnobsView};

/// La lista de parámetros para asignar, abierta.
pub(super) struct KnobPicker {
    query: String,
    selected: usize,
    items: Vec<Learnable>,
}

/// Esperando un CC para un parámetro: uno nuevo, o el mando `replace` de
/// la lista que cambia de CC.
#[derive(Debug, Clone)]
pub(super) struct Learning {
    target: Learnable,
    replace: Option<usize>,
}

impl App {
    /// Los mandos asignados, como van al motor.
    #[cfg(test)]
    pub fn learned(&self) -> &[Learned] {
        &self.learned
    }

    /// La línea de los mandos en el panel de dispositivos: cuántos hay, el
    /// último CC que llegó y si uno espera el suyo.
    pub(super) fn knobs_line(&self) -> String {
        let count = match self.learned.len() {
            0 => "sin mandos".to_string(),
            1 => "1 mando".to_string(),
            n => format!("{n} mandos"),
        };
        let last = self.last_cc.map_or(String::new(), |(channel, cc, value)| {
            format!(" · último CC {cc} (canal {channel}) = {value}")
        });
        let learning = match &self.learning {
            Some(l) => format!(
                " · aprendiendo {} · {}: mueve un mando",
                l.target.track, l.target.param
            ),
            None => String::new(),
        };
        format!("{count}{last}{learning}")
    }

    /// Los mandos de la configuración (los de un proyecto sin los suyos).
    pub(super) fn set_config_learned(&mut self, learned: Vec<Learned>) {
        if self.learned.is_empty() && !learned.is_empty() {
            self.learned = learned;
            self.host.set_learned(self.learned.clone());
        }
    }

    /// «MIDI learn»: la lista de lo que se puede asignar.
    pub fn open_midi_learn(&mut self) {
        let Some(song) = self.host.song() else {
            self.message = "MIDI learn: no hay nada cargado".into();
            return;
        };
        let items = learn::learnable(song);
        self.completion = None;
        self.knob_picker = Some(KnobPicker {
            query: String::new(),
            selected: 0,
            items,
        });
        self.message = if self.midi_in.is_none() {
            "MIDI learn: elige el parámetro (sin entrada MIDI: abre con --allow-midi para mover el mando)"
                .into()
        } else {
            "MIDI learn: escribe para filtrar, ↑↓ y Enter elige el parámetro, Esc cancela".into()
        };
    }

    /// Los parámetros que pasan el filtro (índices).
    fn knob_matches(&self) -> Vec<usize> {
        let Some(picker) = &self.knob_picker else {
            return Vec::new();
        };
        let words: Vec<String> = picker
            .query
            .split_whitespace()
            .map(script::blocks::fold)
            .collect();
        picker
            .items
            .iter()
            .enumerate()
            .filter(|(_, l)| {
                let name = script::blocks::fold(&format!("{} {}", l.track, l.param));
                words.iter().all(|w| name.contains(w.as_str()))
            })
            .map(|(i, _)| i)
            .collect()
    }

    /// Una tecla con la lista de parámetros abierta.
    pub(super) fn handle_knob_picker(&mut self, input: Input) -> Flow {
        let matches = self.knob_matches();
        let count = matches.len();
        let Some(picker) = &mut self.knob_picker else {
            return Flow::Continue;
        };
        match input.key {
            Key::Esc => {
                self.knob_picker = None;
                self.message.clear();
            }
            Key::Enter => {
                if let Some(&i) = matches.get(picker.selected) {
                    let target = picker.items[i].clone();
                    self.knob_picker = None;
                    self.message = format!(
                        "MIDI learn: mueve el mando para {} · {} (Esc cancela)",
                        target.track, target.param
                    );
                    self.learning = Some(Learning {
                        target,
                        replace: None,
                    });
                }
            }
            Key::Up if count > 0 => picker.selected = (picker.selected + count - 1) % count,
            Key::Down if count > 0 => picker.selected = (picker.selected + 1) % count,
            Key::Backspace => {
                picker.query.pop();
                picker.selected = 0;
            }
            Key::Char(c) if !input.ctrl && !input.alt => {
                picker.query.push(c);
                picker.selected = 0;
            }
            _ => {}
        }
        Flow::Continue
    }

    /// Esperando un mando: Esc cancela; lo demás, nada.
    pub(super) fn handle_learning(&mut self, input: Input) -> Flow {
        if input.key == Key::Esc {
            self.learning = None;
            self.message = "MIDI learn cancelado".into();
        }
        Flow::Continue
    }

    /// Un CC que llega: si se está aprendiendo, queda asignado; si no, mueve
    /// los mandos que lo escuchan. `true` si era para ellos.
    pub(super) fn midi_knob(&mut self, channel: u8, cc: u8, value: u8) -> bool {
        self.last_cc = Some((channel, cc, value));
        if let Some(learning) = self.learning.take() {
            let mut new = learning.target.assign(Some(channel), cc);
            match learning.replace {
                // Otro mando para la misma asignación: conserva su rango.
                Some(i) if i < self.learned.len() => {
                    new.min = self.learned[i].min;
                    new.max = self.learned[i].max;
                    new.curve = self.learned[i].curve;
                    self.learned.remove(i);
                    self.learned.insert(i.min(self.learned.len()), new.clone());
                }
                _ => self.learned.push(new.clone()),
            }
            // Un CC solo mueve una cosa: el que lo tenía, lo suelta.
            let mut kept = Vec::new();
            for l in std::mem::take(&mut self.learned) {
                let same = l.channel == new.channel && l.cc == new.cc;
                if !same || (l.track == new.track && l.param == new.param) {
                    kept.push(l);
                }
            }
            self.learned = kept;
            self.learned_changed(format!(
                "mando asignado: {} → {} ({}–{} {})",
                new.source(),
                new.target(),
                number(new.min),
                number(new.max),
                learn::curve_name(new.curve)
            ));
            return true;
        }
        match self
            .host
            .move_learned(&mut self.controller, channel, cc, value)
        {
            Ok(0) => false,
            Ok(_) => true,
            Err(e) => {
                self.message = format!("mando: {e}");
                true
            }
        }
    }

    /// Las asignaciones han cambiado: al motor (entran al compás) y al
    /// proyecto.
    fn learned_changed(&mut self, message: String) {
        self.host.set_learned(self.learned.clone());
        let warnings = match self.host.apply_learned(&mut self.controller) {
            Ok(w) => w,
            Err(e) => vec![e.to_string()],
        };
        let saved = self.save_project_file();
        self.message = if warnings.is_empty() {
            format!("{message}{saved}")
        } else {
            format!("{message}{saved} · ⚠ {}", warnings.join(" · "))
        };
    }

    /// «Mandos MIDI»: la lista de las asignaciones.
    pub fn open_knob_list(&mut self) {
        if self.learned.is_empty() {
            self.message = "no hay mandos MIDI: asígnalos con «MIDI learn»".into();
            return;
        }
        self.completion = None;
        self.knob_list = Some(0);
        self.message =
            "MANDOS: ↑↓ · Supr borrar · c curva · l otro mando · Enter rango · Esc".into();
    }

    /// Una tecla con la lista de mandos abierta.
    pub(super) fn handle_knob_list(&mut self, input: Input) -> Flow {
        let count = self.learned.len();
        let Some(selected) = self.knob_list.filter(|&i| i < count) else {
            self.knob_list = None;
            return Flow::Continue;
        };
        match input.key {
            Key::Esc | Key::Char('q') => {
                self.knob_list = None;
                self.message.clear();
            }
            Key::Up => self.knob_list = Some(selected.saturating_sub(1)),
            Key::Down => self.knob_list = Some((selected + 1).min(count - 1)),
            Key::Delete | Key::Backspace => {
                let gone = self.learned.remove(selected);
                self.knob_list =
                    (!self.learned.is_empty()).then(|| selected.min(self.learned.len() - 1));
                self.learned_changed(format!(
                    "mando borrado: {} ({})",
                    gone.target(),
                    gone.source()
                ));
            }
            Key::Char('c') => {
                let l = &mut self.learned[selected];
                l.curve = match l.curve {
                    engine::KnobCurve::Linear => engine::KnobCurve::Log,
                    engine::KnobCurve::Log => engine::KnobCurve::Square,
                    engine::KnobCurve::Square => engine::KnobCurve::Linear,
                };
                let text = format!("{}: curva {}", l.target(), learn::curve_name(l.curve));
                self.learned_changed(text);
            }
            Key::Char('l') => {
                let l = &self.learned[selected];
                let target = Learnable {
                    track: l.track.clone(),
                    param: l.param.clone(),
                    min: l.min,
                    max: l.max,
                    log: l.curve == engine::KnobCurve::Log,
                };
                self.knob_list = None;
                self.message = format!(
                    "MIDI learn: mueve el mando nuevo para {} (Esc cancela)",
                    l.target()
                );
                self.learning = Some(Learning {
                    target,
                    replace: Some(selected),
                });
            }
            Key::Enter => {
                let l = &self.learned[selected];
                self.knob_list = None;
                self.prompt = Some(format!(
                    "mando {} {} {} {}",
                    selected + 1,
                    number(l.min),
                    number(l.max),
                    learn::curve_name(l.curve)
                ));
                self.message = format!("{}: mínimo, máximo y curva (lin, log, cuad)", l.target());
            }
            _ => {}
        }
        Flow::Continue
    }

    /// `mando N mínimo máximo [curva]`: el rango (y la curva) del mando N
    /// de la lista.
    pub(super) fn knob_command(&mut self, args: &[&str]) {
        let usage = "uso: mando N mínimo máximo [lin|log|cuad]";
        let (Some(index), Some(min), Some(max)) = (
            args.first().and_then(|a| a.parse::<usize>().ok()),
            args.get(1).and_then(|a| a.parse::<f32>().ok()),
            args.get(2).and_then(|a| a.parse::<f32>().ok()),
        ) else {
            self.message = usage.into();
            return;
        };
        let Some(l) = index.checked_sub(1).and_then(|i| self.learned.get_mut(i)) else {
            self.message = format!("no hay mando {index} (hay {})", self.learned.len());
            return;
        };
        if let Some(name) = args.get(3) {
            match learn::curve_named(name) {
                Some(curve) => l.curve = curve,
                None => {
                    self.message = usage.into();
                    return;
                }
            }
        }
        l.min = min;
        l.max = max;
        let text = format!(
            "{}: {}–{} {}",
            l.target(),
            number(min),
            number(max),
            learn::curve_name(l.curve)
        );
        self.learned_changed(text);
    }

    /// «Exportar los mandos MIDI a la configuración».
    pub fn export_knobs(&mut self) {
        let Some(path) = self.devices_config.clone() else {
            self.message = "no hay config.toml (ni carpeta de configuración)".into();
            return;
        };
        self.message = match crate::config::save_learned(&path, &self.learned) {
            Ok(()) => format!(
                "{} mandos en [midi] mandos de {}: los usará un proyecto sin los suyos",
                self.learned.len(),
                path.display()
            ),
            Err(e) => format!("no se pudo escribir {}: {e}", path.display()),
        };
    }

    /// Lo que enseñan la lista de parámetros y la de mandos.
    pub(super) fn knob_picker_view(&self) -> Option<BlockPickerView> {
        let picker = self.knob_picker.as_ref()?;
        let matches = self.knob_matches();
        let items: Vec<(String, String, String)> = matches
            .iter()
            .map(|&i| {
                let l = &picker.items[i];
                let taken = self
                    .learned
                    .iter()
                    .find(|k| k.track == l.track && k.param == l.param)
                    .map_or(String::new(), |k| k.source());
                (format!("{} · {}", l.track, l.param), taken, String::new())
            })
            .collect();
        let description = matches
            .get(picker.selected)
            .map(|&i| {
                let l = &picker.items[i];
                format!(
                    "de {} a {}{}",
                    number(l.min),
                    number(l.max),
                    if l.log {
                        ", en escala logarítmica"
                    } else {
                        ""
                    }
                )
            })
            .unwrap_or_default();
        Some(BlockPickerView {
            title: "MIDI learn: elige el parámetro".into(),
            query: picker.query.clone(),
            items,
            selected: picker.selected.min(matches.len().saturating_sub(1)),
            description,
            preview: Vec::new(),
            confirm: String::new(),
        })
    }

    pub(super) fn knobs_view(&self) -> Option<KnobsView> {
        let selected = self.knob_list?;
        Some(KnobsView {
            rows: self
                .learned
                .iter()
                .map(|l| {
                    (
                        l.target(),
                        l.source(),
                        format!(
                            "{}–{} {}",
                            number(l.min),
                            number(l.max),
                            learn::curve_name(l.curve)
                        ),
                    )
                })
                .collect(),
            selected,
        })
    }
}

/// Un número sin ceros de más.
fn number(v: f32) -> String {
    let text = format!("{v}");
    text.strip_suffix(".0").unwrap_or(&text).to_string()
}
