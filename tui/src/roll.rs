//! El piano roll: la pista seleccionada en una cuadrícula, con el tono (o
//! el corte) en vertical y los pasos en horizontal, y las ediciones que se
//! escriben de vuelta en el texto Lua.
//!
//! Todo sale de lo que recibe el secuenciador (`TrackDef::bars`, el ciclo
//! de vueltas con `every`, `chance`, `fill` y lo generado ya resueltos), así
//! que lo que se ve es lo que sonará. Para escribir se usan las posiciones
//! de cada paso de la fase M (`TrackDef::spans`): una edición cambia solo
//! los caracteres de los tokens afectados, nada más del texto.
//!
//! Un paso se puede reescribir solo si es seguro: el texto es el cargado,
//! el paso tiene posición (no es de `euclid`, `rotate`, una concatenación),
//! su cadena no la comparte otra pista y, al releer su token, sale el mismo
//! paso que suena en esa vuelta (si no, es una variación de `every`,
//! `chance` o `fill`). Si no, [`Column::target`] dice por qué.
//!
//! Este módulo es puro: construye el modelo ([`build`]) y calcula las
//! ediciones ([`Edit`], [`apply`]); la aplicación las escribe en el editor
//! en un paso de deshacer y carga.

use std::collections::BTreeMap;

use engine::Step;
use script::patterns::{self, Key};
use script::{Song, TokenSpan, TrackDef};

/// Qué hay en vertical.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Axis {
    /// Notas MIDI (instrumentos, sintetizadores y pistas de cortes con
    /// `melody()`).
    #[default]
    Notes,
    /// Cortes (desde 0): una fila por corte.
    Slices,
}

/// Dónde está escrito un paso y cómo.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Target {
    pub span: TokenSpan,
    /// El token tal como está en el texto.
    pub token: String,
    /// Es de `melody()` (notas por su nombre o grados) y no de `steps()`.
    pub melody: bool,
}

/// Una columna del ciclo: un paso de una vuelta.
#[derive(Debug, Clone, PartialEq)]
pub struct Column {
    pub lap: usize,
    pub step: usize,
    /// El golpe que suena en esa vuelta (después de `every`, `chance`...).
    pub hit: Option<Step>,
    /// El paso es distinto del patrón base (lo cambia `every`, `chance` o
    /// `fill`): se dibuja con otro estilo.
    pub varies: bool,
    /// No está escrito tal cual en el texto (generado).
    pub generated: bool,
    /// Dónde escribir, o por qué no se puede.
    pub target: Result<Target, String>,
}

/// Cómo se dibuja una nota.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Style {
    /// Escrita, en la escala (o sin tonalidad).
    Scale,
    /// Escrita, fuera de la escala de `key()`.
    Outside,
    /// Una variación de `every`, `chance` o `fill`.
    Variation,
    /// Generada (`euclid`, `rotate`, `mutate`, `shuffle`, concatenaciones).
    Generated,
    /// Un golpe del texto que esta vuelta quita (`chance`, `every`).
    Dropped,
}

impl Style {
    pub fn code(self) -> char {
        match self {
            Style::Scale => 's',
            Style::Outside => 'o',
            Style::Variation => 'v',
            Style::Generated => 'g',
            Style::Dropped => 'x',
        }
    }

    pub fn from_code(c: char) -> Option<Style> {
        Some(match c {
            's' => Style::Scale,
            'o' => Style::Outside,
            'v' => Style::Variation,
            'g' => Style::Generated,
            'x' => Style::Dropped,
            _ => return None,
        })
    }
}

/// Una nota (o un golpe de un corte) de la cuadrícula.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Note {
    /// Columna del ciclo donde empieza.
    pub col: usize,
    /// Pasos que dura (1 + los `_`).
    pub len: usize,
    /// Nota MIDI (en `Axis::Notes`) o corte desde 0 (en `Axis::Slices`).
    pub pitch: i32,
    /// Intensidad por el volumen del paso (`@`): de 0 a 3.
    pub shade: u8,
    /// Marca de la cabeza: `' '` nada, una cifra el retrigger, `◂`
    /// reverse, `▴`/`▾` pitch de un corte, `!` accent, `/` slide.
    pub mark: char,
    pub style: Style,
}

/// La tonalidad que suena (con `transpose()`): tónica y qué clases de
/// nota son de la escala.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Scale {
    pub tonic: i32,
    pub mask: [bool; 12],
}

impl Scale {
    pub fn contains(&self, pitch: i32) -> bool {
        self.mask[pitch.rem_euclid(12) as usize]
    }
}

/// El piano roll de una pista.
#[derive(Debug, Clone, PartialEq)]
pub struct Roll {
    pub track: String,
    pub axis: Axis,
    /// Raíz de las notas (la del instrumento, o C4 en una pista de cortes).
    pub root: i32,
    /// Semitonos que suma `transpose()` a lo escrito en esta pista.
    pub transpose: i32,
    /// La tonalidad de `key()` tal como se escribe (para los grados).
    pub degrees: Option<Key>,
    /// La que suena (con `transpose()`), para marcar la escala.
    pub scale: Option<Scale>,
    /// Pasos de una vuelta.
    pub length: usize,
    pub columns: Vec<Column>,
    pub notes: Vec<Note>,
    /// Cortes de la pista y su etiqueta (en `Axis::Slices`).
    pub labels: Vec<Option<String>>,
    /// Por qué no se edita nada de la pista (además de lo de cada paso).
    pub locked: Option<String>,
}

impl Roll {
    /// Columnas del ciclo entero.
    pub fn steps(&self) -> usize {
        self.columns.len()
    }

    /// La nota que ocupa `(col, pitch)` (su cabeza o uno de sus `_`).
    pub fn note_at(&self, col: usize, pitch: i32) -> Option<&Note> {
        self.notes.iter().find(|n| {
            n.pitch == pitch && n.col <= col && col < n.col + n.len && n.style != Style::Dropped
        })
    }

    /// Notas que suenan (sin las quitadas): lo más grave y lo más agudo.
    pub fn range(&self) -> Option<(i32, i32)> {
        let mut pitches = self
            .notes
            .iter()
            .filter(|n| n.style != Style::Dropped)
            .map(|n| n.pitch);
        let first = pitches.next()?;
        Some(pitches.fold((first, first), |(lo, hi), p| (lo.min(p), hi.max(p))))
    }

    /// Dónde escribir el paso de la columna `col`.
    pub fn target(&self, col: usize) -> Result<&Target, String> {
        if let Some(why) = &self.locked {
            return Err(why.clone());
        }
        let column = self.columns.get(col).ok_or("ese paso no existe")?;
        column.target.as_ref().map_err(Clone::clone)
    }

    /// Las notas del paso de la columna `col` (cabeza en esa columna), de
    /// la primera del acorde a la última.
    pub fn pitches(&self, col: usize) -> Vec<i32> {
        match self.columns.get(col).and_then(|c| c.hit.as_ref()) {
            Some(step) => self.step_pitches(step),
            None => Vec::new(),
        }
    }

    fn step_pitches(&self, step: &Step) -> Vec<i32> {
        match self.axis {
            Axis::Slices => vec![step.slice as i32],
            Axis::Notes => {
                let base = self.root + step.semitones.round() as i32;
                std::iter::once(base)
                    .chain(step.chord.iter().flatten().map(|c| base + c.round() as i32))
                    .collect()
            }
        }
    }

    /// La columna donde empieza la nota que ocupa `col` en `pitch`.
    pub fn head(&self, col: usize, pitch: i32) -> Option<usize> {
        self.note_at(col, pitch).map(|n| n.col)
    }

    /// Lo que dice la línea de estado del paso del cursor.
    pub fn describe(&self, col: usize, pitch: i32) -> String {
        let Some(column) = self.columns.get(col) else {
            return String::new();
        };
        let place = format!(
            "{} · vuelta {} paso {}",
            self.pitch_name(pitch),
            column.lap + 1,
            column.step + 1
        );
        let head = self.head(col, pitch).unwrap_or(col);
        match self.target(head) {
            Ok(t) => format!("{place}: «{}»", t.token),
            Err(why) => format!("{place}: {why}"),
        }
    }

    /// El nombre de una fila: la nota (`C4`) o el corte con su etiqueta.
    pub fn pitch_name(&self, pitch: i32) -> String {
        match self.axis {
            Axis::Notes => engine::note_name(pitch),
            Axis::Slices => match self.labels.get(pitch.max(0) as usize).cloned().flatten() {
                Some(label) => format!("corte {} ({label})", pitch + 1),
                None => format!("corte {}", pitch + 1),
            },
        }
    }

    /// Lo más grave y lo más agudo que se puede escribir.
    pub fn bounds(&self) -> (i32, i32) {
        match self.axis {
            Axis::Slices => (0, self.labels.len().max(1) as i32 - 1),
            Axis::Notes => {
                let reach = engine::MAX_SEMITONES as i32;
                ((self.root - reach).max(0), (self.root + reach).min(127))
            }
        }
    }
}

/// Lo que hace falta para construir el piano roll de una pista.
pub struct Source<'a> {
    pub song: &'a Song,
    /// La definición que se enseña (la que suena, si es de la carga).
    pub def: &'a TrackDef,
    /// Las líneas del editor.
    pub lines: &'a [String],
    /// El texto del editor es el de la última carga buena.
    pub loaded: bool,
}

/// El piano roll de `src.def`.
pub fn build(src: &Source) -> Roll {
    let def = src.def;
    let melodic = def.melody.iter().any(|m| *m);
    let axis = if def.instrument.is_some() || melodic {
        Axis::Notes
    } else {
        Axis::Slices
    };
    let root = def.instrument.map_or(60, |i| i.root);
    // `transpose()` mueve las notas de los instrumentos y las de melody().
    let transpose = if def.instrument.is_some() || melodic {
        src.song.transpose
    } else {
        0
    };
    let degrees = src.song.key.map(|k| k.degrees());
    let scale = src.song.key.map(|k| {
        let sounding = k.transposed(src.song.transpose);
        let mut mask = [false; 12];
        for i in sounding.scale.intervals {
            mask[(sounding.tonic + i).rem_euclid(12) as usize] = true;
        }
        Scale {
            tonic: sounding.tonic,
            mask,
        }
    });
    let locked = if !src.loaded {
        Some(
            "el texto no es el que suena: carga antes (Ctrl-L) para editar desde el piano roll"
                .to_string(),
        )
    } else if def.frozen.is_some() {
        Some("la pista está congelada: descongélala (F1) para editarla".to_string())
    } else {
        None
    };
    // Cadenas de las otras pistas (de todas las secciones): no se tocan.
    let mut others: Vec<(TokenSpan, &str)> = Vec::new();
    for t in src
        .song
        .tracks
        .iter()
        .chain(src.song.sections.iter().flat_map(|s| s.tracks.iter()))
    {
        if t.name != def.name {
            for span in t.spans.iter().flatten().flatten() {
                others.push((*span, &t.name));
            }
        }
    }
    let length = def.length.max(1);
    let laps = def.bars.len().max(1);
    let mut roll = Roll {
        track: def.name.clone(),
        axis,
        root,
        transpose,
        degrees,
        scale,
        length,
        columns: Vec::with_capacity(length * laps),
        notes: Vec::new(),
        labels: def.labels.clone(),
        locked,
    };
    if roll.labels.is_empty() {
        roll.labels = vec![None; def.slices.len().max(1)];
    }
    let humanized = def.humanize.is_some();
    for lap in 0..laps {
        let bar = def.bars.get(lap).unwrap_or(&def.steps);
        let melody = def.melody.get(lap).copied().unwrap_or(false);
        for step in 0..length {
            let hit = bar.get(step).copied().flatten();
            let base = def.steps.get(step).copied().flatten();
            let varies = !same_step(hit.as_ref(), base.as_ref(), humanized, true);
            let span = def
                .spans
                .get(lap)
                .and_then(|s| s.get(step).copied().flatten());
            let target = match span {
                None => Err(GENERATED.to_string()),
                Some(span) => {
                    let token = token_at(src.lines, span);
                    match others.iter().find(|(s, _)| *s == span) {
                        Some((_, other)) => Err(format!(
                            "esa cadena la usa también la pista «{other}»: edítala en el código"
                        )),
                        None if !src.loaded => Ok(Target {
                            span,
                            token,
                            melody,
                        }),
                        None => literal(&roll, def, &token, melody, hit.as_ref(), humanized)
                            .map(|()| Target {
                                span,
                                token,
                                melody,
                            }),
                    }
                }
            };
            roll.columns.push(Column {
                lap,
                step,
                hit,
                varies,
                generated: span.is_none(),
                target,
            });
        }
    }
    // Las notas, con su duración dentro de su vuelta.
    let mut notes = Vec::new();
    for (col, column) in roll.columns.iter().enumerate() {
        let lap_end = (column.lap + 1) * length;
        match &column.hit {
            Some(step) => {
                let len = (1 + step.hold as usize).min(lap_end - col);
                let style = if column.generated {
                    Style::Generated
                } else if column.varies {
                    Style::Variation
                } else {
                    Style::Scale
                };
                for (i, pitch) in roll.step_pitches(step).into_iter().enumerate() {
                    let style = match (style, roll.axis, roll.scale) {
                        (Style::Scale, Axis::Notes, Some(scale)) if !scale.contains(pitch) => {
                            Style::Outside
                        }
                        (s, ..) => s,
                    };
                    notes.push(Note {
                        col,
                        len,
                        pitch,
                        shade: shade(step.gain),
                        mark: if i == 0 { mark(step, roll.axis) } else { ' ' },
                        style,
                    });
                }
            }
            None => {
                // Un golpe escrito que esta vuelta quita: se ve, con su ×.
                let base = def.steps.get(column.step).copied().flatten();
                if let (Some(base), false) = (base, column.generated) {
                    for pitch in roll.step_pitches(&base) {
                        notes.push(Note {
                            col,
                            len: 1,
                            pitch,
                            shade: shade(base.gain),
                            mark: '×',
                            style: Style::Dropped,
                        });
                    }
                }
            }
        }
    }
    roll.notes = notes;
    roll
}

const GENERATED: &str = "ese paso no está escrito tal cual en el texto (sale de euclid, rotate, \
     mutate, shuffle, una cadena hecha a trozos...): cámbialo en el código";
const VARIATION: &str =
    "esta vuelta la cambia every, chance o fill: edita una vuelta escrita o el código";
const DROPPED: &str = "chance o every quitan este golpe en esta vuelta: edita el código";

/// El token del texto en `span` (vacío si la línea no está).
fn token_at(lines: &[String], span: TokenSpan) -> String {
    lines
        .get(span.row)
        .map(|l| {
            l.chars()
                .skip(span.start)
                .take(span.end.saturating_sub(span.start))
                .collect()
        })
        .unwrap_or_default()
}

/// Si el token `token` dice exactamente lo que suena en esa vuelta (`hit`):
/// entonces reescribirlo cambia lo que se ve. Si no, por qué.
fn literal(
    roll: &Roll,
    def: &TrackDef,
    token: &str,
    melody: bool,
    hit: Option<&Step>,
    humanized: bool,
) -> Result<(), String> {
    let parsed = parse_token(roll, def, token, melody)?;
    match (parsed, hit) {
        (None, None) => Ok(()),
        // Un `.` donde suena algo: lo pone `fill` o `every`.
        (None, Some(_)) if token == "." => Err(VARIATION.into()),
        (None, Some(_)) => Ok(()),
        (Some(_), None) => Err(DROPPED.into()),
        (Some(p), Some(h)) => {
            if same_step(Some(&p), Some(h), humanized, false) {
                Ok(())
            } else {
                Err(VARIATION.into())
            }
        }
    }
}

/// El paso que dice un token (con `transpose()`), como lo lee la carga.
pub fn parse_token(
    roll: &Roll,
    def: &TrackDef,
    token: &str,
    melody: bool,
) -> Result<Option<Step>, String> {
    if token == "." || token == engine::notation::HOLD {
        return Ok(None);
    }
    let text = if melody {
        patterns::note_step(token, Some(roll.root), roll.degrees)?
    } else {
        token.to_string()
    };
    let labels = if melody { &[][..] } else { &def.labels[..] };
    let step = engine::notation::parse_step_with(&text, labels, Some(&script::step_fx::resolve))?;
    Ok(step.map(|mut s| {
        s.semitones += roll.transpose as f32;
        s
    }))
}

/// Dos pasos suenan igual (sin contar el retraso del swing ni, con
/// humanize, el volumen; sin `hold`, tampoco lo que dura).
fn same_step(a: Option<&Step>, b: Option<&Step>, humanized: bool, hold: bool) -> bool {
    let (a, b) = match (a, b) {
        (None, None) => return true,
        (Some(a), Some(b)) => (a, b),
        _ => return false,
    };
    let close = |x: f32, y: f32| (x - y).abs() < 1e-3;
    a.slice == b.slice
        && close(a.semitones, b.semitones)
        && a.keep == b.keep
        && a.reverse == b.reverse
        && a.retrig == b.retrig
        && a.lock == b.lock
        && a.accent == b.accent
        && a.slide == b.slide
        && a.fx.is_some() == b.fx.is_some()
        && a.sub.is_some() == b.sub.is_some()
        && (!hold || a.hold == b.hold)
        && (humanized || close(a.gain, b.gain))
        && a.chord.iter().zip(&b.chord).all(|(x, y)| match (x, y) {
            (None, None) => true,
            (Some(x), Some(y)) => close(*x, *y),
            _ => false,
        })
}

/// Intensidad por el volumen, como la vista de ocupación.
fn shade(gain: f32) -> u8 {
    match gain {
        g if g <= 0.3 => 0,
        g if g <= 0.6 => 1,
        g if g <= 0.9 => 2,
        _ => 3,
    }
}

/// La marca de la cabeza de un golpe.
fn mark(step: &Step, axis: Axis) -> char {
    if step.retrig > 1 {
        return if step.retrig >= 10 {
            '+'
        } else {
            char::from_digit(step.retrig as u32, 10).unwrap_or('+')
        };
    }
    if step.reverse {
        return '◂';
    }
    if axis == Axis::Slices && step.semitones.abs() > 1e-3 {
        return if step.semitones > 0.0 { '▴' } else { '▾' };
    }
    if step.slide {
        return '/';
    }
    if step.accent {
        return '!';
    }
    ' '
}

// ---------------------------------------------------------------------
// Ediciones.

/// Una edición del piano roll.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Edit {
    /// Una nota nueva en `(col, pitch)`; si el paso ya tiene nota, se
    /// añade al acorde.
    Insert { col: usize, pitch: i32 },
    /// Quita la nota de `(col, pitch)` (si es un acorde, solo esa).
    Delete { col: usize, pitch: i32 },
    /// Sube o baja semitonos (en una pista de cortes, `semitones` ±1 es el
    /// corte de al lado y `octave` cambia el `^`). Sin selección, la nota
    /// del cursor; con ella, todas las notas que empiezan dentro.
    Transpose {
        col: usize,
        pitch: i32,
        semitones: i32,
        selection: Option<(usize, usize)>,
    },
    /// Alarga o acorta un paso (`_`).
    Resize { col: usize, pitch: i32, delta: i32 },
    /// Sube o baja el volumen (`@`).
    Gain { col: usize, pitch: i32, delta: f32 },
    /// Mueve en el tiempo la nota (o la selección) `delta` pasos y, a la
    /// vez, `semitones` (en una pista de cortes, cortes) arriba o abajo
    /// (lo que hace un arrastre del ratón).
    Move {
        col: usize,
        pitch: i32,
        delta: i32,
        semitones: i32,
        selection: Option<(usize, usize)>,
    },
    /// Copia la selección (o la nota) justo detrás.
    Duplicate {
        col: usize,
        pitch: i32,
        selection: Option<(usize, usize)>,
    },
}

/// Lo que hay que escribir: cada token con su texto nuevo (ninguno se
/// repite), y dónde queda el cursor.
#[derive(Debug, Clone, PartialEq)]
pub struct Change {
    pub edits: Vec<(TokenSpan, String)>,
    pub cursor: (usize, i32),
    pub selection: Option<(usize, usize)>,
    pub message: String,
}

/// Los textos nuevos, por token: dos columnas que comparten el token
/// (vueltas del mismo texto) tienen que pedir lo mismo.
#[derive(Default)]
struct Plan {
    edits: BTreeMap<TokenSpan, (String, String)>,
}

impl Plan {
    fn set(&mut self, target: &Target, text: String) -> Result<(), String> {
        match self.edits.get(&target.span) {
            Some((_, old)) if *old != text => Err(
                "dos vueltas comparten ese texto y el cambio no es el mismo en las dos: cámbialo \
                 en el código"
                    .into(),
            ),
            _ => {
                self.edits
                    .insert(target.span, (target.token.clone(), text));
                Ok(())
            }
        }
    }

    /// Las ediciones que cambian algo.
    fn edits(self) -> Vec<(TokenSpan, String)> {
        self.edits
            .into_iter()
            .filter(|(_, (old, new))| old != new)
            .map(|(span, (_, new))| (span, new))
            .collect()
    }
}

/// Calcula una edición sobre el piano roll (sin tocar el texto).
pub fn apply(roll: &Roll, edit: Edit) -> Result<Change, String> {
    if let Some(why) = &roll.locked {
        return Err(why.clone());
    }
    let change = match edit {
        Edit::Insert { col, pitch } => insert(roll, col, pitch)?,
        Edit::Delete { col, pitch } => delete(roll, col, pitch)?,
        Edit::Transpose {
            col,
            pitch,
            semitones,
            selection,
        } => transpose(roll, col, pitch, semitones, selection)?,
        Edit::Resize { col, pitch, delta } => resize(roll, col, pitch, delta)?,
        Edit::Gain { col, pitch, delta } => gain(roll, col, pitch, delta)?,
        Edit::Move {
            col,
            pitch,
            delta,
            semitones,
            selection,
        } => move_notes(roll, col, pitch, delta, semitones, selection)?,
        Edit::Duplicate {
            col,
            pitch,
            selection,
        } => duplicate(roll, col, pitch, selection)?,
    };
    // Que cada token nuevo se entienda antes de tocar el texto.
    for (span, text) in &change.edits {
        let melody = roll
            .columns
            .iter()
            .find_map(|c| {
                c.target
                    .as_ref()
                    .ok()
                    .filter(|t| t.span == *span)
                    .map(|t| t.melody)
            })
            .unwrap_or(false);
        check_token(roll, text, melody)?;
    }
    if change.edits.is_empty() {
        return Err("no cambia nada".into());
    }
    Ok(change)
}

/// Que un token se lea (con las etiquetas de la pista).
fn check_token(roll: &Roll, token: &str, melody: bool) -> Result<(), String> {
    if token == "." || token == engine::notation::HOLD {
        return Ok(());
    }
    let text = if melody {
        patterns::note_step(token, Some(roll.root), roll.degrees)?
    } else {
        token.to_string()
    };
    let labels = if melody { &[][..] } else { &roll.labels[..] };
    engine::notation::parse_step_with(&text, labels, Some(&script::step_fx::resolve))
        .map(|_| ())
        .map_err(|e| format!("«{token}»: {e}"))
}

fn insert(roll: &Roll, col: usize, pitch: i32) -> Result<Change, String> {
    let target = roll.target(col)?;
    let column = &roll.columns[col];
    let (lo, hi) = roll.bounds();
    if !(lo..=hi).contains(&pitch) {
        return Err(out_of_reach(roll, pitch));
    }
    let mut plan = Plan::default();
    let message = match &column.hit {
        None if target.token == "." => {
            plan.set(target, new_token(roll, target.melody, pitch))?;
            format!("nota {} en el paso {}", roll.pitch_name(pitch), column.step + 1)
        }
        None if target.token == engine::notation::HOLD => {
            return Err(
                "ese paso alarga la nota de antes ([ la acorta): pon la nota en un silencio".into(),
            );
        }
        None => return Err(DROPPED.into()),
        Some(_) if roll.axis == Axis::Slices => {
            return Err(
                "en una pista de cortes cada paso es un golpe: + y - lo cambian de corte".into(),
            );
        }
        Some(_) => {
            let mut pitches = roll.pitches(col);
            if pitches.contains(&pitch) {
                return Err("ya hay esa nota en el paso".into());
            }
            if pitches.len() >= engine::MAX_CHORD {
                return Err(format!(
                    "un acorde tiene como mucho {} notas",
                    engine::MAX_CHORD
                ));
            }
            pitches.push(pitch);
            plan.set(target, with_pitches(roll, target, &pitches)?)?;
            format!("{} añadida al acorde", roll.pitch_name(pitch))
        }
    };
    Ok(Change {
        edits: plan.edits(),
        cursor: (col, pitch),
        selection: None,
        message,
    })
}

fn out_of_reach(roll: &Roll, pitch: i32) -> String {
    match roll.axis {
        Axis::Slices => format!("la pista no tiene el corte {}", pitch + 1),
        Axis::Notes => format!(
            "{} queda a más de ±{} semitonos de la raíz {}",
            engine::note_name(pitch),
            engine::MAX_SEMITONES,
            engine::note_name(roll.root)
        ),
    }
}

/// La nota de `(col, pitch)`: su cabeza y lo que dura.
fn note_under(roll: &Roll, col: usize, pitch: i32) -> Result<(usize, usize), String> {
    let note = roll
        .note_at(col, pitch)
        .ok_or("no hay ninguna nota ahí")?;
    Ok((note.col, note.len))
}

fn delete(roll: &Roll, col: usize, pitch: i32) -> Result<Change, String> {
    let (head, len) = note_under(roll, col, pitch)?;
    let target = roll.target(head)?;
    let mut plan = Plan::default();
    let pitches = roll.pitches(head);
    if pitches.len() > 1 {
        let rest: Vec<i32> = pitches.into_iter().filter(|p| *p != pitch).collect();
        plan.set(target, with_pitches(roll, target, &rest)?)?;
    } else {
        plan.set(target, ".".into())?;
        for k in 1..len {
            plan.set(roll.target(head + k)?, ".".into())?;
        }
    }
    Ok(Change {
        edits: plan.edits(),
        cursor: (head, pitch),
        selection: None,
        message: format!("{} quitada", roll.pitch_name(pitch)),
    })
}

/// Las cabezas de las notas de la selección (o la del cursor).
fn heads(
    roll: &Roll,
    col: usize,
    pitch: i32,
    selection: Option<(usize, usize)>,
) -> Result<Vec<usize>, String> {
    match selection {
        Some((a, b)) => {
            let mut out: Vec<usize> = (a..=b.min(roll.steps().saturating_sub(1)))
                .filter(|&c| roll.columns[c].hit.is_some())
                .collect();
            out.dedup();
            if out.is_empty() {
                return Err("no hay notas en la selección".into());
            }
            Ok(out)
        }
        None => Ok(vec![note_under(roll, col, pitch)?.0]),
    }
}

fn transpose(
    roll: &Roll,
    col: usize,
    pitch: i32,
    semitones: i32,
    selection: Option<(usize, usize)>,
) -> Result<Change, String> {
    let mut plan = Plan::default();
    let (lo, hi) = roll.bounds();
    let mut cursor_pitch = pitch;
    for head in heads(roll, col, pitch, selection)? {
        let target = roll.target(head)?;
        let step = roll.columns[head].hit.ok_or("no hay nota")?;
        let text = match roll.axis {
            // En una pista de cortes, ±1 es el corte de al lado y lo demás
            // (la octava) es el `^`.
            Axis::Slices if semitones.abs() == 1 => {
                let slice = step.slice as i32 + semitones;
                if !(lo..=hi).contains(&slice) {
                    return Err(out_of_reach(roll, slice));
                }
                if head == note_under(roll, col, pitch).map_or(usize::MAX, |n| n.0) {
                    cursor_pitch = slice;
                }
                with_slice(&target.token, &slice_text(roll, slice as usize))
            }
            Axis::Slices => {
                let semis = step.semitones.round() as i32 + semitones;
                if semis.abs() > engine::MAX_SEMITONES as i32 {
                    return Err(format!("el pitch se queda en ±{}", engine::MAX_SEMITONES));
                }
                let chord: Vec<i32> = std::iter::once(semis)
                    .chain(
                        step.chord
                            .iter()
                            .flatten()
                            .map(|c| semis + c.round() as i32),
                    )
                    .collect();
                with_step_pitches(&target.token, &chord)
            }
            Axis::Notes => {
                let old = roll.pitches(head);
                let new: Vec<i32> = if selection.is_some() {
                    old.iter().map(|p| p + semitones).collect()
                } else {
                    old.iter()
                        .map(|&p| if p == pitch { p + semitones } else { p })
                        .collect()
                };
                for &p in &new {
                    if !(lo..=hi).contains(&p) {
                        return Err(out_of_reach(roll, p));
                    }
                }
                let mut sorted = new.clone();
                sorted.sort_unstable();
                sorted.dedup();
                if sorted.len() != new.len() {
                    return Err("esa nota ya está en el acorde".into());
                }
                with_pitches(roll, target, &new)?
            }
        };
        plan.set(target, text)?;
    }
    if roll.axis == Axis::Notes || semitones.abs() != 1 {
        cursor_pitch = if roll.axis == Axis::Notes {
            pitch + semitones
        } else {
            pitch
        };
    }
    let what = match (roll.axis, semitones) {
        (Axis::Slices, 1 | -1) => "otro corte".to_string(),
        (_, s) => format!("{s:+} semitonos"),
    };
    Ok(Change {
        edits: plan.edits(),
        cursor: (col, cursor_pitch),
        selection,
        message: what,
    })
}

fn resize(roll: &Roll, col: usize, pitch: i32, delta: i32) -> Result<Change, String> {
    let (head, len) = note_under(roll, col, pitch)?;
    roll.target(head)?;
    let lap_end = (roll.columns[head].lap + 1) * roll.length;
    let mut plan = Plan::default();
    if delta == 0 {
        return Err("no cambia nada".into());
    }
    let new_len = if delta > 0 {
        let new_len = len + delta as usize;
        if head + new_len > lap_end {
            return Err("la nota llega al final de la vuelta".into());
        }
        for c in head + len..head + new_len {
            let target = roll.target(c)?;
            if target.token != "." || roll.columns[c].hit.is_some() {
                return Err("detrás no hay silencios: no se puede alargar tanto".into());
            }
            plan.set(target, engine::notation::HOLD.into())?;
        }
        new_len
    } else {
        if len <= 1 {
            return Err("ya dura un paso".into());
        }
        let new_len = (len as i64 + delta as i64).max(1) as usize;
        for c in head + new_len..head + len {
            plan.set(roll.target(c)?, ".".into())?;
        }
        new_len
    };
    Ok(Change {
        edits: plan.edits(),
        cursor: (col.min(head + new_len - 1), pitch),
        selection: None,
        message: format!(
            "dura {new_len} {}",
            if new_len == 1 { "paso" } else { "pasos" }
        ),
    })
}

fn gain(roll: &Roll, col: usize, pitch: i32, delta: f32) -> Result<Change, String> {
    let (head, _) = note_under(roll, col, pitch)?;
    let target = roll.target(head)?;
    let current = gain_of(&target.token);
    let new = ((current + delta) * 100.0).round() / 100.0;
    let new = new.clamp(0.0, engine::MAX_GAIN);
    if (new - current).abs() < 1e-6 {
        return Err(format!("el volumen ya está en {}", format_number(current)));
    }
    let mut plan = Plan::default();
    plan.set(target, with_gain(&target.token, new, target.melody))?;
    Ok(Change {
        edits: plan.edits(),
        cursor: (col, pitch),
        selection: None,
        message: format!("volumen {}", format_number(new)),
    })
}

fn move_notes(
    roll: &Roll,
    col: usize,
    pitch: i32,
    delta: i32,
    semitones: i32,
    selection: Option<(usize, usize)>,
) -> Result<Change, String> {
    if delta == 0 && semitones == 0 {
        return Err("no cambia nada".into());
    }
    let heads = heads(roll, col, pitch, selection)?;
    // Lo que queda en cada columna tocada.
    let mut after: BTreeMap<usize, String> = BTreeMap::new();
    let mut sources: Vec<(usize, usize, String)> = Vec::new();
    for &head in &heads {
        let len = roll
            .notes
            .iter()
            .filter(|n| n.col == head && n.style != Style::Dropped)
            .map(|n| n.len)
            .max()
            .unwrap_or(1);
        let target = roll.target(head)?;
        let token = if semitones == 0 {
            target.token.clone()
        } else {
            shifted(roll, head, target, semitones)?
        };
        for k in 0..len {
            roll.target(head + k)?;
            after.insert(head + k, ".".into());
        }
        sources.push((head, len, token));
    }
    let in_source = |c: usize| sources.iter().any(|(h, l, _)| (*h..h + l).contains(&c));
    let mut placed: Vec<usize> = Vec::new();
    for (head, len, token) in &sources {
        let to = *head as i64 + delta as i64;
        if to < 0 || to as usize + len > roll.steps() {
            return Err("se sale del ciclo".into());
        }
        let to = to as usize;
        let lap = roll.columns[*head].lap;
        if roll.columns[to].lap != lap || roll.columns[to + len - 1].lap != lap {
            return Err("una nota no cruza el final de la vuelta (cada vuelta es su texto)".into());
        }
        for k in 0..*len {
            let c = to + k;
            let target = roll.target(c)?;
            if !in_source(c) && (target.token != "." || roll.columns[c].hit.is_some()) {
                return Err("ahí ya hay una nota: muévela antes o bórrala".into());
            }
            if placed.contains(&c) {
                return Err("dos notas caen en el mismo paso".into());
            }
            placed.push(c);
            after.insert(
                c,
                if k == 0 {
                    token.clone()
                } else {
                    engine::notation::HOLD.into()
                },
            );
        }
    }
    let mut plan = Plan::default();
    for (c, text) in after {
        plan.set(roll.target(c)?, text)?;
    }
    let shift = |c: usize| (c as i64 + delta as i64).max(0) as usize;
    let message = match (delta, semitones) {
        (0, s) => format!("{s:+} {}", unit(roll)),
        (d, 0) => format!("{d:+} pasos"),
        (d, s) => format!("{d:+} pasos y {s:+} {}", unit(roll)),
    };
    Ok(Change {
        edits: plan.edits(),
        cursor: (shift(col), pitch + semitones),
        selection: selection.map(|(a, b)| (shift(a), shift(b))),
        message,
    })
}

/// Semitonos o cortes.
fn unit(roll: &Roll) -> &'static str {
    match roll.axis {
        Axis::Notes => "semitonos",
        Axis::Slices => "cortes",
    }
}

/// El token de la cabeza `head` con todas sus notas `semitones` arriba (en
/// una pista de cortes, otro corte).
fn shifted(roll: &Roll, head: usize, target: &Target, semitones: i32) -> Result<String, String> {
    let (lo, hi) = roll.bounds();
    let new: Vec<i32> = roll.pitches(head).iter().map(|p| p + semitones).collect();
    if let Some(&p) = new.iter().find(|p| !(lo..=hi).contains(*p)) {
        return Err(out_of_reach(roll, p));
    }
    match roll.axis {
        Axis::Notes => with_pitches(roll, target, &new),
        Axis::Slices => Ok(with_slice(&target.token, &slice_text(roll, new[0] as usize))),
    }
}

fn duplicate(
    roll: &Roll,
    col: usize,
    pitch: i32,
    selection: Option<(usize, usize)>,
) -> Result<Change, String> {
    let (a, b) = match selection {
        Some(range) => range,
        None => {
            let (head, len) = note_under(roll, col, pitch)?;
            (head, head + len - 1)
        }
    };
    let n = b - a + 1;
    if b + n > roll.steps() {
        return Err("detrás no cabe: se saldría del ciclo".into());
    }
    let mut plan = Plan::default();
    for k in 0..n {
        let (from, to) = (a + k, a + n + k);
        let source = roll.target(from)?;
        let dest = roll.target(to)?;
        if source.melody != dest.melody {
            return Err("una vuelta es de melody() y la otra de steps(): cópialo en el código".into());
        }
        if dest.token != "." || roll.columns[to].hit.is_some() {
            return Err(format!(
                "el paso {} ya tiene algo: duplicar solo escribe en silencios",
                roll.columns[to].step + 1
            ));
        }
        // Un `_` al principio (de una nota de antes) o al empezar una
        // vuelta no tiene nota a la que alargar.
        let text = if source.token == engine::notation::HOLD && (k == 0 || roll.columns[to].step == 0)
        {
            ".".to_string()
        } else {
            source.token.clone()
        };
        plan.set(dest, text)?;
    }
    Ok(Change {
        edits: plan.edits(),
        cursor: (col + n, pitch),
        selection: selection.map(|_| (a + n, b + n)),
        message: format!("{n} {} duplicados detrás", if n == 1 { "paso" } else { "pasos" }),
    })
}

// ---------------------------------------------------------------------
// Tokens: cambiar una parte sin tocar las demás.

/// El token de una nota nueva.
fn new_token(roll: &Roll, melody: bool, pitch: i32) -> String {
    match (roll.axis, melody) {
        (Axis::Slices, _) => slice_text(roll, pitch.max(0) as usize),
        (Axis::Notes, true) => note_text(pitch - roll.transpose, NoteStyle::default(), true),
        (Axis::Notes, false) => format!("1^{}", pitch - roll.transpose - roll.root),
    }
}

/// Cómo se escribe un corte: su etiqueta si la tiene, o su número.
fn slice_text(roll: &Roll, slice: usize) -> String {
    roll.labels
        .get(slice)
        .cloned()
        .flatten()
        .unwrap_or_else(|| (slice + 1).to_string())
}

/// El token con estas notas (las que suenan), escrito como estaba: con
/// nombres o grados en `melody()` y con `^` en `steps()`.
fn with_pitches(roll: &Roll, target: &Target, pitches: &[i32]) -> Result<String, String> {
    let written: Vec<i32> = pitches.iter().map(|p| p - roll.transpose).collect();
    if target.melody {
        let (head, rest) = split_melody(&target.token);
        let style = NoteStyle::of(head);
        // Cada nota como estaba: grado si era un grado (y la nueva sigue en
        // la escala); las que se añaden, grados si todas lo eran.
        let parts: Vec<bool> = head.split('+').map(head_is_degrees).collect();
        let all_degrees = !parts.is_empty() && parts.iter().all(|d| *d);
        let text: Vec<String> = written
            .iter()
            .enumerate()
            .map(|(i, &p)| {
                let degree = parts.get(i).copied().unwrap_or(all_degrees);
                if degree
                    && let Some(d) = roll.degrees.and_then(|k| degree_of(k, roll.root, p))
                {
                    return d.to_string();
                }
                // Sin octava solo la primera nota que cae en la de la raíz
                // y si antes tampoco la llevaba.
                let octave = i > 0 || style.octave || p.div_euclid(12) != roll.root.div_euclid(12);
                note_text(p, style, octave)
            })
            .collect();
        Ok(format!("{}{rest}", text.join("+")))
    } else {
        let semis: Vec<i32> = written.iter().map(|p| p - roll.root).collect();
        Ok(with_step_pitches(&target.token, &semis))
    }
}

/// Cómo se escribían las notas de un token: mayúsculas, bemoles y octava.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct NoteStyle {
    upper: bool,
    flats: bool,
    octave: bool,
}

impl Default for NoteStyle {
    fn default() -> Self {
        NoteStyle {
            upper: false,
            flats: false,
            octave: true,
        }
    }
}

impl NoteStyle {
    fn of(head: &str) -> NoteStyle {
        let first = head.chars().find(|c| c.is_ascii_alphabetic());
        NoteStyle {
            upper: first.is_some_and(|c| c.is_ascii_uppercase()),
            flats: head
                .char_indices()
                .any(|(i, c)| c == 'b' && i > 0 && head[..i].ends_with(|p: char| p.is_ascii_alphabetic())),
            octave: head.chars().any(|c| c.is_ascii_digit()) || head.is_empty(),
        }
    }
}

/// `61` → `c#4` (o `db4` con bemoles, `C#4` en mayúsculas).
fn note_text(midi: i32, style: NoteStyle, octave: bool) -> String {
    const SHARPS: [&str; 12] = [
        "c", "c#", "d", "d#", "e", "f", "f#", "g", "g#", "a", "a#", "b",
    ];
    const FLATS: [&str; 12] = [
        "c", "db", "d", "eb", "e", "f", "gb", "g", "ab", "a", "bb", "b",
    ];
    let names = if style.flats { FLATS } else { SHARPS };
    let mut name = names[midi.rem_euclid(12) as usize].to_string();
    if style.upper {
        let mut chars = name.chars();
        name = chars
            .next()
            .map(|c| c.to_ascii_uppercase().to_string() + chars.as_str())
            .unwrap_or_default();
    }
    if octave {
        name.push_str(&(midi.div_euclid(12) - 1).to_string());
    }
    name
}

/// El grado de `key` que da la nota `midi` (escrita) desde `root`, si es
/// de la escala.
fn degree_of(key: Key, root: i32, midi: i32) -> Option<i64> {
    let target = midi - root;
    (1..=36)
        .flat_map(|d| [d, -d])
        .find(|&d| key.degree(root, d).ok() == Some(target))
}

/// Si las notas de un token de `melody()` son grados (`1`, `-2`, `1+3+5`).
fn head_is_degrees(head: &str) -> bool {
    head.split('+')
        .all(|part| !part.is_empty() && part.trim_start_matches('-').chars().all(|c| c.is_ascii_digit()))
}

/// Separa un token de `melody()` en sus notas (`c+e+g`, `bb3`, `1+3`) y
/// los sufijos (`@0.5r2`).
fn split_melody(token: &str) -> (&str, &str) {
    let bytes = token.as_bytes();
    let mut i = 0;
    loop {
        let start = i;
        // Un grado.
        if i < bytes.len() && bytes[i] == b'-' {
            i += 1;
        }
        let digits = bytes[i..].iter().take_while(|b| b.is_ascii_digit()).count();
        if digits > 0 {
            i += digits;
        } else {
            i = start;
            // Una nota: letra, alteración y octava.
            if i < bytes.len() && matches!(bytes[i].to_ascii_lowercase(), b'a'..=b'g') {
                i += 1;
                if i < bytes.len() && (bytes[i] == b'#' || bytes[i] == b'b') {
                    i += 1;
                }
                let sign = usize::from(i < bytes.len() && bytes[i] == b'-');
                let digits = bytes[i + sign..]
                    .iter()
                    .take_while(|b| b.is_ascii_digit())
                    .count();
                if digits > 0 {
                    i += sign + digits;
                }
            } else {
                return (&token[..start], &token[start..]);
            }
        }
        if i < bytes.len() && bytes[i] == b'+' {
            i += 1;
        } else {
            return (&token[..i], &token[i..]);
        }
    }
}

/// Un sufijo de un token: dónde empieza, dónde acaba y qué es.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Part {
    Pitch,
    Gain,
    Other,
}

/// La cabeza (el corte: número, etiqueta o `x`) de un token de `steps()`
/// y sus sufijos, con su sitio.
fn split_step(token: &str) -> (usize, Vec<(usize, usize, Part)>) {
    let head = match engine::notation::split_label(token) {
        Some((name, _)) => name.len(),
        None => token.len() - token.trim_start_matches(|c: char| c.is_ascii_digit()).len(),
    };
    (head, suffixes(token, head))
}

/// Los sufijos de `token` desde `from`.
fn suffixes(token: &str, from: usize) -> Vec<(usize, usize, Part)> {
    let bytes = token.as_bytes();
    let mut out = Vec::new();
    let mut i = from;
    let number = |mut j: usize, signed: bool| {
        if signed && j < bytes.len() && (bytes[j] == b'-' || bytes[j] == b'+') {
            j += 1;
        }
        while j < bytes.len() && (bytes[j].is_ascii_digit() || bytes[j] == b'.') {
            j += 1;
        }
        j
    };
    while i < bytes.len() {
        let start = i;
        let (end, part) = match bytes[i] {
            b'{' => (
                token[i..].find('}').map_or(bytes.len(), |e| i + e + 1),
                Part::Other,
            ),
            b'^' | b'%' => (number(i + 1, true), Part::Pitch),
            b'@' => (number(i + 1, false), Part::Gain),
            b'~' => (number(i + 1, false), Part::Other),
            b'r' => (number(i + 1, false), Part::Other),
            _ => (
                i + token[i..].chars().next().map_or(1, char::len_utf8),
                Part::Other,
            ),
        };
        out.push((start, end.max(start + 1), part));
        i = end.max(start + 1);
    }
    out
}

/// El token de `steps()` con otros semitonos (`^a^b`, el primero con `%`
/// si lo llevaba) donde estaba el primero, o tras el corte.
fn with_step_pitches(token: &str, semis: &[i32]) -> String {
    let (head, parts) = split_step(token);
    let keep = parts
        .iter()
        .find(|(_, _, p)| *p == Part::Pitch)
        .is_some_and(|(s, _, _)| token[*s..].starts_with('%'));
    let pitch: String = semis
        .iter()
        .enumerate()
        .map(|(i, s)| format!("{}{s}", if i == 0 && keep { '%' } else { '^' }))
        .collect();
    replace_parts(token, head, &parts, Part::Pitch, &pitch)
}

/// El token con otro volumen (`@`); con 1, sin él.
fn with_gain(token: &str, gain: f32, melody: bool) -> String {
    let text = if (gain - 1.0).abs() < 1e-6 {
        String::new()
    } else {
        format!("@{}", format_number(gain))
    };
    let (head, parts) = if melody {
        let (notes, _) = split_melody(token);
        (notes.len(), suffixes(token, notes.len()))
    } else {
        split_step(token)
    };
    replace_parts(token, head, &parts, Part::Gain, &text)
}

/// Volumen escrito en un token (1 sin `@`).
fn gain_of(token: &str) -> f32 {
    token
        .find('@')
        .and_then(|at| {
            let rest = &token[at + 1..];
            let end = rest
                .find(|c: char| !(c.is_ascii_digit() || c == '.'))
                .unwrap_or(rest.len());
            rest[..end].parse().ok()
        })
        .unwrap_or(1.0)
}

/// Quita los sufijos de tipo `kind` y pone `text` donde estaba el primero
/// (o tras la cabeza).
fn replace_parts(
    token: &str,
    head: usize,
    parts: &[(usize, usize, Part)],
    kind: Part,
    text: &str,
) -> String {
    let mut out = token[..head].to_string();
    let mut placed = false;
    for &(s, e, p) in parts {
        if p == kind {
            if !placed {
                out.push_str(text);
                placed = true;
            }
        } else {
            out.push_str(&token[s..e]);
        }
    }
    if !placed {
        // Detrás del corte y de sus tonos: `3^7` → `3^7@0.5`.
        let at = parts
            .iter()
            .take_while(|(_, _, p)| *p == Part::Pitch)
            .map(|(_, e, _)| *e)
            .last()
            .unwrap_or(head);
        return format!("{}{text}{}", &token[..at], &token[at..]);
    }
    out
}

/// Otro corte en un token de `steps()` (los sufijos no cambian).
fn with_slice(token: &str, slice: &str) -> String {
    let (head, _) = split_step(token);
    format!("{slice}{}", &token[head..])
}

/// `0.5`, `1.25`, `2`: sin ceros de más.
fn format_number(value: f32) -> String {
    let text = format!("{value:.2}");
    text.trim_end_matches('0').trim_end_matches('.').to_string()
}

/// Cómo se escribe una nota nueva en `melody()` (para los tests).
#[cfg(test)]
pub fn tests_note_text(midi: i32) -> String {
    note_text(midi, NoteStyle::default(), true)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn melody_tokens_split_into_notes_and_suffixes() {
        assert_eq!(split_melody("c"), ("c", ""));
        assert_eq!(split_melody("bb3@0.5"), ("bb3", "@0.5"));
        assert_eq!(split_melody("c+e+g!"), ("c+e+g", "!"));
        assert_eq!(split_melody("gr2"), ("g", "r2"));
        assert_eq!(split_melody("1+3+5{sub=0}"), ("1+3+5", "{sub=0}"));
        assert_eq!(split_melody("-2/"), ("-2", "/"));
        assert_eq!(split_melody("eb/"), ("eb", "/"));
    }

    #[test]
    fn step_pitches_replace_only_the_pitch() {
        assert_eq!(with_step_pitches("1", &[7]), "1^7");
        assert_eq!(with_step_pitches("1^3@0.5", &[5]), "1^5@0.5");
        assert_eq!(with_step_pitches("3'^7@0.5r2", &[0, 4]), "3'^0^4@0.5r2");
        assert_eq!(with_step_pitches("1%0^4^7", &[2, 6, 9]), "1%2^6^9");
        assert_eq!(with_step_pitches("kick{lp=800}^-5", &[-3]), "kick{lp=800}^-3");
    }

    #[test]
    fn gains_are_rewritten_in_place() {
        assert_eq!(with_gain("3", 0.5, false), "3@0.5");
        assert_eq!(with_gain("3^7r2", 0.5, false), "3^7@0.5r2");
        assert_eq!(with_gain("3@0.5r2", 0.7, false), "3@0.7r2");
        assert_eq!(with_gain("3@0.5r2", 1.0, false), "3r2");
        assert_eq!(with_gain("c4@0.5", 0.25, true), "c4@0.25");
        assert_eq!(with_gain("eb", 0.5, true), "eb@0.5");
        assert_eq!(with_gain("1+3r2", 0.5, true), "1+3@0.5r2");
        assert_eq!(with_gain("kick", 0.5, false), "kick@0.5");
        assert_eq!(gain_of("3@0.25r2"), 0.25);
        assert_eq!(gain_of("3"), 1.0);
    }

    #[test]
    fn slices_change_their_head_only() {
        assert_eq!(with_slice("3'^7", "4"), "4'^7");
        assert_eq!(with_slice("kickr4", "snare"), "snarer4");
        assert_eq!(with_slice("12@0.5", "2"), "2@0.5");
    }

    #[test]
    fn notes_are_written_in_the_style_of_the_token() {
        assert_eq!(note_text(61, NoteStyle::default(), true), "c#4");
        let flats = NoteStyle::of("Eb3");
        assert!(flats.flats && flats.upper && flats.octave);
        assert_eq!(note_text(61, flats, true), "Db4");
        assert!(!NoteStyle::of("b").flats);
        assert!(!NoteStyle::of("e").octave);
        assert!(head_is_degrees("1+3+5") && head_is_degrees("-2") && !head_is_degrees("c+e"));
    }

    #[test]
    fn degrees_round_trip() {
        let key = Key {
            tonic: 9,
            intervals: &[0, 2, 3, 5, 7, 8, 10],
        };
        // La menor desde C4: el grado 1 es A3 (57).
        assert_eq!(degree_of(key, 60, 57), Some(1));
        assert_eq!(degree_of(key, 60, 60), Some(3));
        assert_eq!(degree_of(key, 60, 61), None);
        assert_eq!(degree_of(key, 60, 55), Some(-1));
    }
}
