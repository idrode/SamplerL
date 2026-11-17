//! Ajustar un número del código con el ratón: con Ctrl (o Alt, `[editor]
//! ajuste_numeros`) y la rueda sobre un número, el número cambia; con Ctrl,
//! pulsar sobre él y arrastrar, también; y con el teclado, el del cursor
//! (Alt-A y Alt-X, con Shift de diez en diez; en vim, Ctrl-A y Ctrl-X en
//! Normal).
//!
//! **Un gesto** es una ráfaga: todo lo que cambia entre que empieza y que
//! acaba es un solo paso de deshacer. La terminal no dice cuándo se suelta
//! Ctrl ni cuándo se levantan los dedos, así que el gesto acaba tras una
//! pausa sin eventos (`ajuste_pausa_ms`) o con cualquier otro evento (una
//! tecla, un clic, el puntero en otro número o fuera del editor); Esc lo
//! cancela y deja el número como estaba. Al acabar se carga el texto: el
//! cambio entra en el compás siguiente, como cualquier carga.
//!
//! **La inercia** del touchpad (pasos que siguen llegando después de
//! levantar los dedos) y la deriva del puntero: un gesto que acaba por otro
//! evento deja una cola en la que la rueda con el modificador no hace nada
//! hasta una pausa de [`TAIL_GAP`]. Además, cada paso cambia como mucho lo
//! de un paso de la rueda y caben [`MAX_RATE`] por segundo (con una ráfaga
//! de [`BURST`]): un deslizamiento muy rápido no se dispara.
//!
//! Lo que cambia y cómo se escribe sale de [`crate::numbers`]; qué número
//! hay bajo el puntero y de qué parámetro es, del linter
//! ([`script::lint::number_at`]). Sin gesto ni modificador, nada de esto
//! cuesta: el movimiento del ratón sin Ctrl ni Alt no se mira.

use std::ops::Range;
use std::time::{Duration, Instant};

use ratatui_textarea::CursorMove;
use script::lint::{NumberAt, NumberLiteral};

use super::{App, EditGroup, Mode};
use crate::config::AdjustModifier;
use crate::numbers::{self, Stepper};

/// Tras un gesto cortado por otro evento, la rueda no hace nada hasta una
/// pausa así (la cola de la inercia del touchpad).
pub const TAIL_GAP: Duration = Duration::from_millis(150);
/// Pasos por segundo como mucho, y los que se pueden juntar de golpe.
pub const MAX_RATE: f64 = 60.0;
pub const BURST: f64 = 8.0;
/// Con `ajuste_aceleracion`, dos pasos más juntos que esto cuentan doble; y
/// más juntos que la mitad, cuádruple.
const ACCELERATE: Duration = Duration::from_millis(25);
/// Pasos de un ajuste grueso con el teclado (con Shift).
const COARSE: f64 = 10.0;
/// Celdas de arrastre por paso, en horizontal; en vertical, una fila son
/// dos pasos.
const DRAG_ROW: f64 = 2.0;
/// Lo menos que pasa entre dos valores en vivo al motor (unos 30 por
/// segundo): el mando los suaviza en 30 ms.
pub const LIVE_EVERY: Duration = Duration::from_millis(33);

/// Las teclas que acompañan al ratón.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Mods {
    pub ctrl: bool,
    pub alt: bool,
}

impl From<bool> for Mods {
    /// `true`: con Ctrl (como antes se decía «con Alt o Ctrl»).
    fn from(held: bool) -> Mods {
        Mods {
            ctrl: held,
            alt: false,
        }
    }
}

/// Lo que hay de ajustes de números.
#[derive(Debug, Default)]
pub(crate) struct Adjust {
    gesture: Option<Gesture>,
    /// El número bajo el puntero con el modificador (para resaltarlo).
    hover: Option<(usize, Range<usize>)>,
    /// La cola tras un gesto cortado: el último evento que se ignoró.
    tail: Option<Instant>,
    /// Los pasos que quedan por gastar y cuándo se miró.
    budget: f64,
    budget_at: Option<Instant>,
    /// Ya se ha visto llegar la rueda con el modificador (la terminal lo
    /// manda), y ya se avisó de que quizá no llega.
    seen: bool,
    hinted: bool,
    /// El destino del último gesto en vivo cuya carga aún puede no haber
    /// entrado, y el compás en que acabó.
    released: Option<(engine::KnobTarget, u64)>,
    /// Valores en vivo mandados al motor (para los tests y la medición).
    live_sent: u64,
}

/// Cómo se está cambiando.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Via {
    Wheel,
    /// Pulsado y arrastrando desde esa celda, con los pasos ya dados.
    Drag {
        column: u16,
        row: u16,
    },
    Keys,
}

/// Un gesto en curso.
#[derive(Debug)]
struct Gesture {
    row: usize,
    /// La columna donde empieza el número (no se mueve: crece a la derecha).
    start: usize,
    /// Los caracteres escritos ahora y lo más ancho que ha llegado a ser.
    len: usize,
    widest: usize,
    stepper: Stepper,
    /// El número como estaba escrito.
    original: String,
    /// El texto entero de antes (para deshacer de una vez).
    before: Vec<String>,
    /// Se ha escrito algo distinto del original (y está en el historial
    /// del editor).
    applied: bool,
    /// El cursor del editor al empezar.
    cursor: (usize, usize),
    last: Instant,
    via: Via,
    /// Pasos dados con el arrastre.
    dragged: f64,
    /// Qué es: `fx("filter") cutoff`, `:gain` o `número`.
    label: String,
    /// Suena mientras se ajusta (el parámetro del motor se sabe seguro), o
    /// por qué no.
    live: Result<Live, String>,
}

/// El valor en vivo de un gesto: su mando en el motor, cuándo se mandó el
/// último y el que espera turno.
#[derive(Debug)]
struct Live {
    target: script::live::LiveTarget,
    sent: Option<Instant>,
    pending: Option<f32>,
}

/// Lo que se enseña de un gesto en curso (la línea bajo el editor).
#[derive(Debug, Clone, PartialEq)]
pub struct AdjustInfo {
    pub label: String,
    /// El valor con su unidad.
    pub value: String,
    /// `20–20000 Hz · log`, o vacío sin registro.
    pub range: String,
    /// Dónde está en el rango (0 a 1), si lo hay.
    pub fraction: Option<f64>,
    /// Cuándo se oye.
    pub when: String,
}

impl App {
    /// El modificador de la configuración está pulsado en `mods`.
    fn adjust_held(&self, mods: Mods) -> bool {
        match self.editor_config.adjust.modifier {
            AdjustModifier::Ctrl => mods.ctrl,
            AdjustModifier::Alt => mods.alt && !mods.ctrl,
            AdjustModifier::Off => false,
        }
    }

    /// Hay un gesto en curso.
    pub(crate) fn adjusting(&self) -> bool {
        self.adjust.gesture.is_some()
    }

    /// El ratón puede ajustar en `column`, `row`: el editor, sin nada
    /// encima, con el ratón dentro del texto.
    fn adjust_area(&self, column: u16, row: u16) -> bool {
        self.mode == Mode::Edit
            && self.nothing_on_top()
            && self
                .editor_area
                .is_some_and(|a| a.contains(ratatui::layout::Position::new(column, row)))
    }

    /// La fila y la columna del texto bajo la celda, si caen dentro de una
    /// línea (no más allá de su final).
    fn adjust_text_at(&self, column: u16, row: u16) -> Option<(usize, usize)> {
        let area = self.editor_area?;
        let text_x = area.x + self.editor.line_number_width();
        if column < text_x {
            return None;
        }
        let (r, c) = self.text_at(column, row);
        let len = self.editor.lines().get(r)?.chars().count();
        (c < len).then_some((r, c))
    }

    /// Un evento del ratón, antes que nadie: lo que es del gesto (la rueda
    /// con el modificador, arrastrar con él) se queda aquí (`true`); lo
    /// demás cierra el gesto en curso y sigue su camino (`false`).
    pub(super) fn adjust_mouse(
        &mut self,
        action: super::MouseAction,
        column: u16,
        row: u16,
        mods: Mods,
        now: Instant,
    ) -> bool {
        use super::MouseAction as M;
        let held = self.adjust_held(mods);
        let modifier = self.editor_config.adjust.modifier;
        // La rueda con el modificador: hacia arriba (o a la derecha), +1.
        let wheel = match action {
            M::ZoomIn if modifier == AdjustModifier::Ctrl => Some(1.0),
            M::ZoomOut if modifier == AdjustModifier::Ctrl => Some(-1.0),
            M::ScrollUp | M::ScrollRight if held => Some(1.0),
            M::ScrollDown | M::ScrollLeft if held => Some(-1.0),
            _ => None,
        };
        let dragging = self
            .adjust
            .gesture
            .as_ref()
            .is_some_and(|g| matches!(g.via, Via::Drag { .. }));
        if let Some(dir) = wheel {
            if !self.adjust_area(column, row) {
                self.adjust_end();
                return false;
            }
            self.adjust.seen = true;
            let dir = if self.editor_config.adjust.invert {
                -dir
            } else {
                dir
            };
            self.adjust_wheel(dir, column, row, now);
            return true;
        }
        match action {
            M::Drag if dragging => {
                self.adjust_drag(column, row, now);
                true
            }
            M::Up if dragging => {
                self.adjust_drag(column, row, now);
                self.adjust_end();
                true
            }
            M::Down if held && self.adjust_area(column, row) => {
                self.adjust_end();
                let Some(at) = self.adjust_text_at(column, row) else {
                    return false;
                };
                if self.adjust_start(at, Via::Drag { column, row }, now) {
                    self.drag_from = None;
                    true
                } else {
                    false
                }
            }
            _ => {
                self.adjust_interrupt();
                false
            }
        }
    }

    /// Un paso de la rueda con el modificador sobre `column`, `row`.
    fn adjust_wheel(&mut self, dir: f64, column: u16, row: u16, now: Instant) {
        let at = self.adjust_text_at(column, row);
        // La cola de un gesto cortado: nada hasta una pausa.
        if let Some(last) = self.adjust.tail {
            if now.saturating_duration_since(last) < TAIL_GAP {
                self.adjust.tail = Some(now);
                return;
            }
            self.adjust.tail = None;
        }
        if let Some(g) = &self.adjust.gesture {
            let on_it = g.via == Via::Wheel
                && at.is_some_and(|(r, c)| {
                    r == g.row && (g.start..=g.start + g.widest).contains(&c)
                });
            if !on_it {
                // El puntero se fue a otro sitio a mitad del gesto: se
                // acaba, y lo que siga de la ráfaga no cuenta.
                self.adjust_end();
                self.adjust.tail = Some(now);
                return;
            }
        } else {
            let Some(at) = at else { return };
            if !self.adjust_start(at, Via::Wheel, now) {
                return;
            }
        }
        let ticks = self.accelerated(dir, now);
        self.adjust_nudge(ticks, now);
    }

    /// Con `ajuste_aceleracion`, los pasos muy seguidos cuentan más.
    fn accelerated(&self, dir: f64, now: Instant) -> f64 {
        if !self.editor_config.adjust.acceleration {
            return dir;
        }
        let gap = self
            .adjust
            .gesture
            .as_ref()
            .map(|g| now.saturating_duration_since(g.last));
        match gap {
            Some(g) if g < ACCELERATE / 2 => dir * 4.0,
            Some(g) if g < ACCELERATE => dir * 2.0,
            _ => dir,
        }
    }

    /// Arrastrando con el botón pulsado: a la derecha y hacia arriba sube.
    fn adjust_drag(&mut self, column: u16, row: u16, now: Instant) {
        let Some(g) = &self.adjust.gesture else {
            return;
        };
        let Via::Drag {
            column: c0,
            row: r0,
        } = g.via
        else {
            return;
        };
        let wanted = (column as f64 - c0 as f64) + (r0 as f64 - row as f64) * DRAG_ROW;
        let ticks = wanted - g.dragged;
        if ticks != 0.0 {
            if let Some(g) = self.adjust.gesture.as_mut() {
                g.dragged = wanted;
            }
            self.adjust_nudge(ticks, now);
        }
    }

    /// El ratón se ha movido (sin botón): con el modificador, resalta el
    /// número que hay debajo; un gesto de la rueda acaba si el puntero se
    /// va de su número. Devuelve si cambia lo que se ve.
    pub fn pointer_moved(&mut self, column: u16, row: u16, mods: Mods, now: Instant) -> bool {
        let held = self.adjust_held(mods);
        if !held && self.adjust.hover.is_none() && self.adjust.gesture.is_none() {
            return false;
        }
        let mut changed = false;
        if let Some(g) = &self.adjust.gesture
            && g.via == Via::Wheel
        {
            let at = self
                .adjust_area(column, row)
                .then(|| self.adjust_text_at(column, row))
                .flatten();
            let on_it =
                at.is_some_and(|(r, c)| r == g.row && (g.start..=g.start + g.widest).contains(&c));
            if !on_it {
                self.adjust_end();
                self.adjust.tail = Some(now);
                changed = true;
            }
        }
        let hover = if held && self.adjust_area(column, row) {
            self.adjust_text_at(column, row).and_then(|(r, c)| {
                match script::lint::number_at(&self.text(), r, c) {
                    NumberAt::Number(n) => Some((n.row, n.cols)),
                    _ => None,
                }
            })
        } else {
            None
        };
        if hover != self.adjust.hover {
            self.adjust.hover = hover;
            changed = true;
        }
        changed
    }

    /// Empieza un gesto sobre el número que hay en `at` (fila y columna
    /// del texto). Si no hay número, lo dice y no empieza.
    fn adjust_start(&mut self, at: (usize, usize), via: Via, now: Instant) -> bool {
        let literal = match script::lint::number_at(&self.text(), at.0, at.1) {
            NumberAt::Number(n) => n,
            NumberAt::InString => {
                self.message =
                    "los números dentro de una cadena (los patrones de steps) no se ajustan con el ratón"
                        .into();
                return false;
            }
            NumberAt::Unsupported(why) => {
                self.message = format!("este número no se ajusta: {why}");
                return false;
            }
            NumberAt::Nothing => return false,
        };
        let spec = literal.param.and_then(script::api::numeric);
        let Some(stepper) =
            Stepper::new(&literal.text, spec, self.editor_config.adjust.sensitivity)
        else {
            self.message = format!("«{}» no se ajusta", literal.text);
            return false;
        };
        let live = self.live_target(&literal, spec);
        let cursor = self.editor.cursor();
        self.editor.cancel_selection();
        self.completion = None;
        let len = literal.text.chars().count();
        self.adjust.gesture = Some(Gesture {
            row: literal.row,
            start: literal.cols.start,
            len,
            widest: len,
            stepper,
            original: literal.text.clone(),
            before: self.editor.lines().to_vec(),
            applied: false,
            cursor: (cursor.0, cursor.1),
            last: now,
            via,
            dragged: 0.0,
            label: label(&literal),
            live,
        });
        self.adjust.hover = Some((literal.row, literal.cols));
        true
    }

    /// Mueve el número del gesto `ticks` pasos (con el límite de ritmo) y
    /// lo escribe.
    fn adjust_nudge(&mut self, ticks: f64, now: Instant) {
        let keys = self
            .adjust
            .gesture
            .as_ref()
            .is_some_and(|g| g.via == Via::Keys);
        if keys {
            if let Some(g) = self.adjust.gesture.as_mut() {
                g.last = now;
                let text = g.stepper.nudge(ticks);
                self.adjust_write(&text);
            }
            return;
        }
        // El límite de ritmo: un cubo que se llena a MAX_RATE por segundo.
        let since = self
            .adjust
            .budget_at
            .map_or(Duration::from_secs(1), |t| now.saturating_duration_since(t));
        self.adjust.budget = (self.adjust.budget + since.as_secs_f64() * MAX_RATE).min(BURST);
        self.adjust.budget_at = Some(now);
        let cost = ticks.abs().min(BURST);
        if self.adjust.budget < cost.min(1.0) {
            return;
        }
        let ticks = ticks.signum() * cost.min(self.adjust.budget.max(1.0));
        self.adjust.budget = (self.adjust.budget - ticks.abs()).max(0.0);
        let Some(g) = self.adjust.gesture.as_mut() else {
            return;
        };
        g.last = now;
        let text = g.stepper.nudge(ticks);
        self.adjust_write(&text);
        self.adjust_live(now, false);
    }

    /// El destino en el motor del número, si se sabe seguro: con el texto
    /// cargado (las posiciones son las suyas), en esta ventana (la del
    /// editor aparte no tiene el motor) y lo que diga el script
    /// ([`script::live::resolve`]).
    fn live_target(
        &self,
        literal: &NumberLiteral,
        spec: Option<script::api::Numeric>,
    ) -> Result<Live, String> {
        if self.editor_window() {
            return Err("en la ventana del editor, al acabar".into());
        }
        if self.loaded_text.as_deref() != Some(self.text().as_str()) {
            return Err("el texto no es el cargado".into());
        }
        let value: f64 = literal.text.parse().map_err(|_| "no es un número")?;
        let range = spec.map(|s| (s.min, s.max, s.log));
        let target = self
            .host
            .live_target(&literal.place, value, range)
            .map_err(str::to_string)?;
        // El motor tiene un solo gesto: otro parámetro espera a que entre
        // la carga del anterior (si no, el anterior volvería a lo de antes
        // hasta el compás).
        if let Some((before, bar)) = self.adjust.released
            && before != target.map.target
            && self.status.is_none_or(|s| !s.running || s.bar <= bar)
        {
            return Err("la carga del anterior aún no ha entrado".into());
        }
        Ok(Live {
            target,
            sent: None,
            pending: None,
        })
    }

    /// Manda al motor el valor del gesto (como mucho cada [`LIVE_EVERY`];
    /// el que llega antes espera y sale en el `tick`). `force`: ya.
    fn adjust_live(&mut self, now: Instant, force: bool) {
        let Some(g) = self.adjust.gesture.as_mut() else {
            return;
        };
        let value = g.stepper.value();
        let Ok(live) = g.live.as_mut() else {
            return;
        };
        let t = live.target.position(value);
        let due = force
            || live
                .sent
                .is_none_or(|s| now.saturating_duration_since(s) >= LIVE_EVERY);
        if !due {
            live.pending = Some(t);
            return;
        }
        live.pending = None;
        live.sent = Some(now);
        self.adjust.live_sent += 1;
        let map = live.target.map;
        if self.controller.gesture(Some(map), t).is_err()
            && let Some(g) = self.adjust.gesture.as_mut()
        {
            g.live = Err("el motor no lo recibe".into());
        }
    }

    /// Quita el valor en vivo del motor ya (cancelado, sin cambios o la
    /// carga falló): vuelve a mandar lo cargado.
    fn live_off(&mut self) {
        let _ = self.controller.gesture(None, 0.0);
    }

    /// Escribe `text` en el sitio del número (deshaciendo lo que escribió
    /// el gesto antes: el historial del editor no crece con cada paso).
    fn adjust_write(&mut self, text: &str) {
        let Some(g) = self.adjust.gesture.as_mut() else {
            return;
        };
        if g.applied {
            let mut tries = 0;
            while self.editor.lines() != g.before.as_slice() && tries < 8 && self.editor.undo() {
                tries += 1;
            }
            g.applied = false;
        }
        let original = g.original.chars().count();
        let clamp = |n: usize| n.min(u16::MAX as usize) as u16;
        if text != g.original {
            self.editor
                .move_cursor(CursorMove::Jump(clamp(g.row), clamp(g.start)));
            self.editor.delete_str(original);
            self.editor.insert_str(text);
            g.applied = true;
        }
        g.len = text.chars().count();
        g.widest = g.widest.max(g.len);
        // El cursor, donde estaba (más a la derecha si el número creció
        // delante de él en su línea).
        let (row, mut col) = g.cursor;
        if row == g.row && col > g.start {
            col = (col + g.len).saturating_sub(original);
        }
        self.editor
            .move_cursor(CursorMove::Jump(clamp(row), clamp(col)));
        self.adjust.hover = Some((g.row, g.start..g.start + g.len));
    }

    /// Otro evento corta el gesto: acaba, y la cola de la ráfaga que siga
    /// llegando no empieza otro.
    fn adjust_interrupt(&mut self) {
        if let Some(g) = &self.adjust.gesture
            && g.via == Via::Wheel
        {
            self.adjust.tail = Some(g.last);
        }
        self.adjust_end();
    }

    /// Acaba el gesto en curso: si cambió algo, es un paso de deshacer y se
    /// carga (entra en el compás siguiente).
    pub(crate) fn adjust_end(&mut self) {
        let Some(g) = self.adjust.gesture.take() else {
            return;
        };
        self.adjust.hover = None;
        let live = g.live.is_ok();
        let after = self.editor.lines().to_vec();
        if !g.applied || after == g.before {
            if live {
                self.live_off();
            }
            return;
        }
        self.undo_groups.push(EditGroup {
            before: g.before,
            after,
        });
        self.redo_groups.clear();
        let value = numbers::shown(&g.stepper.text(), g.stepper.spec());
        if let Ok(live) = &g.live {
            // El último valor, ya; y la carga lo retira al compás, con el
            // texto que lo dice.
            let t = live.target.position(g.stepper.value());
            let _ = self.controller.gesture(Some(live.target.map), t);
            self.host.release_gesture();
            let bar = self.status.map_or(0, |s| s.bar);
            self.adjust.released = Some((live.target.map.target, bar));
        }
        let loads = self.loads;
        self.load();
        if live && (self.error.is_some() || self.loads == loads) {
            // La carga no llegó al motor: sigue sonando lo de antes, también
            // en este parámetro.
            self.live_off();
        }
        if self.error.is_none() {
            let when = if live {
                "suena ya; el texto entra en el compás siguiente"
            } else {
                "entra en el compás siguiente"
            };
            self.message = format!("{} = {value}: {when} (Ctrl-Z lo deshace)", g.label);
        }
    }

    /// Esc durante un gesto: el número vuelve a como estaba.
    fn adjust_cancel(&mut self) {
        let Some(g) = self.adjust.gesture.take() else {
            return;
        };
        self.adjust.hover = None;
        if g.live.is_ok() {
            self.live_off();
        }
        if g.applied {
            let mut tries = 0;
            while self.editor.lines() != g.before.as_slice() && tries < 8 && self.editor.undo() {
                tries += 1;
            }
            let clamp = |n: usize| n.min(u16::MAX as usize) as u16;
            self.editor
                .move_cursor(CursorMove::Jump(clamp(g.cursor.0), clamp(g.cursor.1)));
        }
        // Lo que siga llegando de la ráfaga (la inercia) no empieza otro.
        self.adjust.tail = Some(g.last);
        self.message = format!("{}: cancelado, vuelve a {}", g.label, g.original);
    }

    /// Una tecla: Esc cancela el gesto; cualquier otra lo acaba (y sigue su
    /// camino), salvo las de ajustar con el teclado sobre el mismo número.
    pub(super) fn adjust_key(&mut self, input: &ratatui_textarea::Input) -> bool {
        let Some(g) = &self.adjust.gesture else {
            return false;
        };
        if input.key == ratatui_textarea::Key::Esc {
            self.adjust_cancel();
            return true;
        }
        let adjusting_keys = g.via == Via::Keys
            && matches!(
                self.keys.lookup(input),
                Some(crate::config::Bound::Action(
                    crate::palette::Action::NumberUp
                        | crate::palette::Action::NumberDown
                        | crate::palette::Action::NumberUpCoarse
                        | crate::palette::Action::NumberDownCoarse
                ))
            );
        if !adjusting_keys && !self.vim_number_key(input) {
            self.adjust_interrupt();
        }
        false
    }

    /// Ctrl-A o Ctrl-X en Normal de vim.
    pub(super) fn vim_number_key(&self, input: &ratatui_textarea::Input) -> bool {
        self.vim
            .as_ref()
            .is_some_and(|v| v.mode == super::vim::VimMode::Normal)
            && input.ctrl
            && !input.alt
            && matches!(input.key, ratatui_textarea::Key::Char('a' | 'x'))
    }

    /// Con el teclado: el número del cursor (o el primero que hay a su
    /// derecha en la línea, como en vim) sube o baja `ticks` pasos.
    pub(super) fn adjust_by_keys(&mut self, ticks: f64, coarse: bool) {
        let now = Instant::now();
        let ticks = if coarse { ticks * COARSE } else { ticks };
        let continuing = self
            .adjust
            .gesture
            .as_ref()
            .is_some_and(|g| g.via == Via::Keys);
        if !continuing {
            self.adjust_end();
            let cursor = self.editor.cursor();
            let line: Vec<char> = self
                .editor
                .lines()
                .get(cursor.0)
                .map_or(Vec::new(), |l| l.chars().collect());
            // Desde el cursor hacia la derecha, el primer número.
            let text = self.text();
            let found =
                (cursor.1..line.len().max(cursor.1 + 1)).find_map(
                    |c| match script::lint::number_at(&text, cursor.0, c) {
                        NumberAt::Number(n) => Some((n.row, n.cols.start)),
                        _ => None,
                    },
                );
            let Some(at) = found else {
                self.message = "no hay ningún número en el cursor ni a su derecha".into();
                return;
            };
            if !self.adjust_start(at, Via::Keys, now) {
                return;
            }
        }
        // Las teclas no pasan por el límite de ritmo (la repetición del
        // teclado ya lo es).
        self.adjust_nudge(ticks, now);
    }

    /// Acaba el gesto tras la pausa (en cada vuelta del bucle).
    pub(super) fn adjust_tick(&mut self, now: Instant) {
        // El valor en vivo que esperaba turno.
        if self.adjust.gesture.as_ref().is_some_and(|g| {
            g.live.as_ref().is_ok_and(|l| {
                l.pending.is_some()
                    && l.sent
                        .is_none_or(|s| now.saturating_duration_since(s) >= LIVE_EVERY)
            })
        }) {
            self.adjust_live(now, true);
        }
        let pause = Duration::from_millis(self.editor_config.adjust.pause_ms as u64);
        if self
            .adjust
            .gesture
            .as_ref()
            .is_some_and(|g| now.saturating_duration_since(g.last) >= pause)
        {
            self.adjust_end();
        }
    }

    /// La rueda sin el modificador sobre un número, la primera vez en la
    /// sesión que nunca ha llegado con él: por si la terminal se lo queda.
    pub(super) fn adjust_hint(&mut self, column: u16, row: u16) {
        if self.adjust.seen
            || self.adjust.hinted
            || self.editor_config.adjust.modifier == AdjustModifier::Off
        {
            return;
        }
        let Some((r, c)) = self.adjust_text_at(column, row) else {
            return;
        };
        if let NumberAt::Number(_) = script::lint::number_at(&self.text(), r, c) {
            self.adjust.hinted = true;
            let (key, other) = match self.editor_config.adjust.modifier {
                AdjustModifier::Alt => ("Alt", "\"ctrl\""),
                _ => ("Ctrl", "\"alt\""),
            };
            self.message = format!(
                "para cambiar un número: {key} y la rueda encima (si la terminal no lo deja llegar, \
                 [editor] ajuste_numeros = {other}; en tmux, ver la guía)"
            );
        }
    }

    /// El número que se resalta: el del gesto o el que hay bajo el puntero
    /// con el modificador.
    pub fn adjust_mark(&self) -> Option<(usize, Range<usize>)> {
        self.adjust.hover.clone()
    }

    /// Lo que se enseña del gesto en curso.
    pub fn adjust_info(&self) -> Option<AdjustInfo> {
        let g = self.adjust.gesture.as_ref()?;
        let spec = g.stepper.spec();
        let range = spec.map_or(String::new(), |s| {
            let unit = if s.unit.is_empty() {
                String::new()
            } else {
                format!(" {}", s.unit)
            };
            let scale = if s.log { " · log" } else { "" };
            let int = if s.integer { " · entero" } else { "" };
            format!(
                "{}–{}{unit}{scale}{int}",
                script::number_text(s.min),
                script::number_text(s.max)
            )
        });
        Some(AdjustInfo {
            label: g.label.clone(),
            value: numbers::shown(&g.stepper.text(), spec),
            range,
            fraction: g.stepper.fraction(),
            when: match &g.live {
                Ok(_) => "suena ya".into(),
                Err(why) => format!("se oirá en el compás siguiente ({why})"),
            },
        })
    }

    /// La huella del ajuste para el dibujo en reposo.
    pub(crate) fn adjust_idle_key(&self) -> Option<(usize, usize, usize)> {
        self.adjust
            .hover
            .as_ref()
            .map(|(r, cols)| (*r, cols.start, cols.end))
    }
}

/// Qué es el número, para la línea del gesto.
fn label(n: &NumberLiteral) -> String {
    let p = &n.place;
    let call = p.call.as_deref().unwrap_or("");
    match (&p.key, n.param) {
        (Some(key), Some(_)) => match &p.first {
            Some(first) if !first.starts_with("capa:") => format!("{call}(\"{first}\") {key}"),
            Some(layer) => format!("{call} {} {key}", layer.trim_start_matches("capa:")),
            None => format!("{call} {key}"),
        },
        (None, Some(param)) => format!(":{}", param.name),
        (Some(key), None) => format!("{key} (sin rango)"),
        (None, None) if call.is_empty() => "número (sin rango)".into(),
        (None, None) => format!("número de {call}() (sin rango)"),
    }
}

#[cfg(test)]
impl App {
    /// Como en cada vuelta del bucle, en el instante `now`.
    pub fn adjust_tick_for_test(&mut self, now: Instant) {
        self.adjust_tick(now);
    }

    /// Los valores en vivo mandados al motor.
    pub fn live_sent_for_test(&self) -> u64 {
        self.adjust.live_sent
    }

    /// Las cargas hechas.
    pub fn loads_for_test(&self) -> u64 {
        self.loads
    }
}

/// El ritmo como mucho y la ráfaga (para los tests).
#[cfg(test)]
pub fn adjust_limits() -> (f64, f64) {
    (MAX_RATE, BURST)
}
