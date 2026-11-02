//! Las teclas del piano roll cuando tiene el cursor: cada orden con su
//! nombre en `[roll.teclas]` de `config.toml` y sus teclas por defecto. Las
//! flechas (mover el cursor), Shift y las flechas (seleccionar), RePág y
//! AvPág (otra octava a la vista) e Inicio y Fin no se cambian.

use ratatui_textarea::{Input, Key};

/// Una orden del piano roll.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RollOp {
    Insert,
    Delete,
    Up,
    Down,
    OctaveUp,
    OctaveDown,
    Longer,
    Shorter,
    Earlier,
    Later,
    Louder,
    Softer,
    Duplicate,
    Select,
    ToggleSnap,
    CycleSnap,
    ZoomIn,
    ZoomOut,
    Fit,
    Preview,
    NextNote,
    PreviousNote,
    Exit,
}

/// Las órdenes: nombre en `[roll.teclas]`, teclas por defecto y qué hacen.
pub const ROLL_KEYS: &[(&str, RollOp, &[&str], &str)] = &[
    (
        "insertar",
        RollOp::Insert,
        &["enter", "i"],
        "pone una nota en el cursor (sobre otra del mismo paso, la añade al acorde)",
    ),
    (
        "borrar",
        RollOp::Delete,
        &["delete", "backspace", "x"],
        "quita la nota del cursor (de un acorde, solo esa) y sus `_`",
    ),
    (
        "subir",
        RollOp::Up,
        &["+", "="],
        "un semitono arriba la nota del cursor o la selección (en cortes: el corte siguiente)",
    ),
    (
        "bajar",
        RollOp::Down,
        &["-"],
        "un semitono abajo (en cortes: el corte anterior)",
    ),
    (
        "octava_arriba",
        RollOp::OctaveUp,
        &[">"],
        "una octava arriba (en cortes: `^` +12)",
    ),
    (
        "octava_abajo",
        RollOp::OctaveDown,
        &["<"],
        "una octava abajo (en cortes: `^` -12)",
    ),
    ("alargar", RollOp::Longer, &["]"], "un paso más (`_`)"),
    ("acortar", RollOp::Shorter, &["["], "un paso menos"),
    (
        "antes",
        RollOp::Earlier,
        &[","],
        "mueve la nota o la selección antes, lo que diga la rejilla",
    ),
    (
        "despues",
        RollOp::Later,
        &["."],
        "mueve la nota o la selección después",
    ),
    ("mas_volumen", RollOp::Louder, &["V"], "`@` +0.1"),
    ("menos_volumen", RollOp::Softer, &["v"], "`@` -0.1"),
    (
        "duplicar",
        RollOp::Duplicate,
        &["d"],
        "copia la selección (o la nota) justo detrás, sobre silencios",
    ),
    (
        "seleccionar",
        RollOp::Select,
        &["s"],
        "empieza o quita una selección de pasos en el cursor (también Shift y ←→)",
    ),
    (
        "rejilla",
        RollOp::ToggleSnap,
        &["g"],
        "enciende o apaga la rejilla (apagada, todo va de paso en paso)",
    ),
    (
        "tamano_rejilla",
        RollOp::CycleSnap,
        &["G"],
        "la rejilla siguiente: 1/16, 1/8, 1/4, 1/2, 1/1",
    ),
    (
        "acercar",
        RollOp::ZoomIn,
        &["z"],
        "más detalle en el tiempo (hasta 1/32)",
    ),
    (
        "alejar",
        RollOp::ZoomOut,
        &["Z"],
        "menos detalle (hasta 1/4)",
    ),
    (
        "ajustar",
        RollOp::Fit,
        &["f"],
        "vuelve a ajustar la vista a la tesitura de la pista",
    ),
    (
        "escuchar",
        RollOp::Preview,
        &["space", "p"],
        "preescucha la nota del cursor (o la fila, en un hueco)",
    ),
    (
        "siguiente",
        RollOp::NextNote,
        &["tab"],
        "a la nota siguiente",
    ),
    (
        "anterior",
        RollOp::PreviousNote,
        &["shift-tab"],
        "a la nota anterior",
    ),
    (
        "salir",
        RollOp::Exit,
        &["esc"],
        "suelta las teclas (el piano roll se sigue viendo)",
    ),
];

/// Una tecla del piano roll: `enter`, `x`, `+`, `shift-tab`, `ctrl-d`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RollKey {
    key: Key,
    ctrl: bool,
    alt: bool,
    shift: bool,
}

impl RollKey {
    pub fn parse(text: &str) -> Result<RollKey, String> {
        let mut rest = text.trim();
        let (mut ctrl, mut alt, mut shift) = (false, false, false);
        loop {
            let lower = rest.to_lowercase();
            if rest.chars().count() > 1 && lower.starts_with("ctrl-") {
                ctrl = true;
                rest = &rest[5..];
            } else if rest.chars().count() > 1 && lower.starts_with("alt-") {
                alt = true;
                rest = &rest[4..];
            } else if rest.chars().count() > 1 && lower.starts_with("shift-") {
                shift = true;
                rest = &rest[6..];
            } else {
                break;
            }
        }
        let key = match rest.to_lowercase().as_str() {
            "enter" | "intro" => Key::Enter,
            "esc" => Key::Esc,
            "tab" => Key::Tab,
            "delete" | "supr" => Key::Delete,
            "backspace" | "retroceso" => Key::Backspace,
            "space" | "espacio" => Key::Char(' '),
            "home" | "inicio" => Key::Home,
            "end" | "fin" => Key::End,
            _ if rest.chars().count() == 1 => Key::Char(rest.chars().next().unwrap_or(' ')),
            f if f.starts_with('f')
                && f[1..].parse::<u8>().is_ok_and(|n| (1..=12).contains(&n)) =>
            {
                Key::F(f[1..].parse().unwrap_or(1))
            }
            _ => return Err(format!("«{text}»: tecla desconocida")),
        };
        Ok(RollKey {
            key,
            ctrl,
            alt,
            shift,
        })
    }

    fn matches(&self, input: &Input) -> bool {
        // Un carácter ya dice si va con Shift (`V`, `>`): no se mira.
        let shift_ok = matches!(self.key, Key::Char(_)) || input.shift == self.shift;
        input.key == self.key && input.ctrl == self.ctrl && input.alt == self.alt && shift_ok
    }

    pub fn text(&self) -> String {
        let mut out = String::new();
        if self.shift {
            out.push_str("Shift-");
        }
        if self.ctrl {
            out.push_str("Ctrl-");
        }
        if self.alt {
            out.push_str("Alt-");
        }
        match self.key {
            Key::Enter => out.push_str("Enter"),
            Key::Esc => out.push_str("Esc"),
            Key::Tab => out.push_str("Tab"),
            Key::Delete => out.push_str("Supr"),
            Key::Backspace => out.push_str("Retroceso"),
            Key::Home => out.push_str("Inicio"),
            Key::End => out.push_str("Fin"),
            Key::Char(' ') => out.push_str("Espacio"),
            Key::Char(c) => out.push(c),
            Key::F(n) => out.push_str(&format!("F{n}")),
            _ => out.push('?'),
        }
        out
    }
}

/// Las teclas de cada orden.
#[derive(Debug, Clone, PartialEq)]
pub struct RollKeys {
    bindings: Vec<(RollKey, RollOp)>,
}

impl Default for RollKeys {
    fn default() -> Self {
        let mut bindings = Vec::new();
        for (_, op, keys, _) in ROLL_KEYS {
            for k in *keys {
                bindings.push((RollKey::parse(k).expect("las de siempre se leen"), *op));
            }
        }
        RollKeys { bindings }
    }
}

impl RollKeys {
    /// Cambia las teclas de la orden `name`; devuelve el error si no
    /// existe o una tecla no se lee.
    pub fn set(&mut self, name: &str, keys: &[String]) -> Result<(), String> {
        let Some((_, op, _, _)) = ROLL_KEYS.iter().find(|(n, ..)| *n == name) else {
            let names: Vec<&str> = ROLL_KEYS.iter().map(|(n, ..)| *n).collect();
            return Err(format!(
                "[roll.teclas] «{name}» no existe (hay: {})",
                names.join(", ")
            ));
        };
        let parsed = keys
            .iter()
            .map(|k| RollKey::parse(k))
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| format!("[roll.teclas] {name}: {e}"))?;
        self.bindings.retain(|(_, o)| o != op);
        // Una tecla nueva deja de hacer lo que hacía.
        self.bindings.retain(|(k, _)| !parsed.contains(k));
        self.bindings.extend(parsed.into_iter().map(|k| (k, *op)));
        Ok(())
    }

    pub fn lookup(&self, input: &Input) -> Option<RollOp> {
        self.bindings
            .iter()
            .find(|(k, _)| k.matches(input))
            .map(|(_, op)| *op)
    }

    /// La primera tecla de una orden, como se ve.
    pub fn describe(&self, op: RollOp) -> String {
        self.bindings
            .iter()
            .find(|(_, o)| *o == op)
            .map_or_else(|| "—".into(), |(k, _)| k.text())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn input(key: Key, shift: bool) -> Input {
        Input {
            key,
            ctrl: false,
            alt: false,
            shift,
        }
    }

    #[test]
    fn the_defaults_and_a_change() {
        let mut keys = RollKeys::default();
        assert_eq!(keys.lookup(&input(Key::Enter, false)), Some(RollOp::Insert));
        assert_eq!(
            keys.lookup(&input(Key::Char('V'), true)),
            Some(RollOp::Louder)
        );
        assert_eq!(
            keys.lookup(&input(Key::Char('v'), false)),
            Some(RollOp::Softer)
        );
        assert_eq!(
            keys.lookup(&input(Key::Tab, true)),
            Some(RollOp::PreviousNote)
        );
        assert_eq!(keys.lookup(&input(Key::Tab, false)), Some(RollOp::NextNote));
        keys.set("insertar", &["n".into()]).unwrap();
        assert_eq!(keys.lookup(&input(Key::Enter, false)), None);
        assert_eq!(
            keys.lookup(&input(Key::Char('n'), false)),
            Some(RollOp::Insert)
        );
        // `x` pasa a insertar: ya no borra.
        keys.set("insertar", &["x".into()]).unwrap();
        assert_eq!(
            keys.lookup(&input(Key::Char('x'), false)),
            Some(RollOp::Insert)
        );
        assert!(keys.set("volar", &["q".into()]).is_err());
        assert!(keys.set("borrar", &["ctrl-".into()]).is_err());
        assert_eq!(keys.describe(RollOp::Preview), "Espacio");
    }
}
