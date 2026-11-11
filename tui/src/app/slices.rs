//! Los cortes editados en la onda (ratón, teclas, partir, unir, etiquetar)
//! se escriben en el código: la llamada que corta el sample (`slice(8)`,
//! `slice_grid(16)` o `slice({ ... })`) pasa a la forma explícita,
//! `slice({ {inicio, fin}, ... })`, con el inicio y el final de cada corte
//! en ms del sample. Es un paso de deshacer y entra al cargar, como lo que
//! escriben el tracker, el piano roll y las lanes; se guarda con Ctrl-S. Si
//! la llamada no está escrita tal cual (sale de código), el cambio no se
//! puede guardar y se dice por qué.

use std::ops::Range;

use script::{Cut, TrackDef};

use super::App;

/// Lo que se quiere escribir: los cortes de la llamada (todos, en frames
/// del sample sin estirar) y sus etiquetas.
pub(super) struct Rewrite {
    pub ranges: Vec<Range<usize>>,
    pub labels: Vec<Option<String>>,
}

/// La llamada de la pista no se puede reescribir.
pub(super) enum NotWritten {
    /// No está escrita tal cual (sale de código): por qué.
    Generated(String),
    /// Ahora no (el texto cambió desde la última carga, la carga falló, o al
    /// releerlo no dice lo que se quería): por qué. No se cambia nada.
    Refused(String),
}

impl App {
    /// La llamada que corta el sample de `track` y sus cortes de partida
    /// (frames del sample sin estirar) y etiquetas.
    pub(super) fn slice_call(track: &TrackDef) -> Result<&script::SliceCall, NotWritten> {
        track.origin.call.as_ref().map_err(|why| {
            NotWritten::Generated(format!(
                "los cortes de {} salen de código ({why})",
                track.name
            ))
        })
    }

    /// Escribe en el código los cortes de la llamada de la pista
    /// seleccionada, en un paso de deshacer, y carga. Si al releerlo no da
    /// esos cortes (otra llamada los cambia después, `:move()`...), o cambia
    /// los de otra pista, lo deshace y dice por qué.
    pub(super) fn write_slices(&mut self, rewrite: Rewrite) -> Result<(), NotWritten> {
        let Some(track) = self.tracks().get(self.selected).cloned() else {
            return Err(NotWritten::Refused("no hay pistas".into()));
        };
        let call = Self::slice_call(&track)?.clone();
        // La posición es la del texto cargado: si ha cambiado, no vale.
        if self.loaded_text.as_deref() != Some(self.text().as_str()) || self.error.is_some() {
            return Err(NotWritten::Refused(match &self.error {
                Some(e) => format!(
                    "la última carga falló ({}): arréglalo y carga antes de editar cortes",
                    e.message
                ),
                None => "el texto ha cambiado desde la última carga: carga antes (Ctrl-S)".into(),
            }));
        }
        let lines: Vec<String> = self.editor.lines().to_vec();
        let indent: String = lines
            .get(call.span.start.0)
            .map(|l| l.chars().take_while(|c| c.is_whitespace()).collect())
            .unwrap_or_default();
        let written = call.rewritten(&rewrite.ranges, &rewrite.labels, &indent);
        let text = super::lanes::replace_span(&lines, call.span, &written);
        // Lo que tiene que quedar: las pistas de esta llamada con los cortes
        // nuevos y las demás como estaban.
        let before: Vec<(String, Vec<Range<usize>>, bool)> = self
            .tracks()
            .iter()
            .map(|t| {
                let ours = t
                    .origin
                    .call
                    .as_ref()
                    .is_ok_and(|c| c.span.start == call.span.start);
                (t.name.clone(), t.slices.iter().collect(), ours)
            })
            .collect();
        let zero = track.origin.zero_cross.map_or(0, |ms| {
            (ms / 1000.0 * track.sample.sample_rate() as f64).ceil() as usize
        });
        self.completion = None;
        self.marks.clear();
        self.slice_warning = None;
        self.replace_text(&text);
        self.load();
        // Solo se miran los cortes que cambian (los demás los puede cambiar
        // el código después, como antes).
        let changed: Vec<bool> = if rewrite.ranges.len() == call.ranges.len() {
            rewrite
                .ranges
                .iter()
                .zip(&call.ranges)
                .map(|(a, b)| a != b)
                .collect()
        } else {
            vec![true; rewrite.ranges.len()]
        };
        if let Some(problem) = self.check_rewrite(&before, &rewrite.ranges, &changed, zero.max(1)) {
            self.undo();
            self.load();
            return Err(NotWritten::Refused(problem));
        }
        Ok(())
    }

    /// Qué no cuadra tras reescribir la llamada, si algo: las pistas de la
    /// llamada (`before`, la tercera) tienen que tener los cortes de
    /// `ranges` donde `changed` (con su estiramiento, a `tolerance` frames
    /// con los cruces por cero), y las demás, los de antes.
    fn check_rewrite(
        &self,
        before: &[(String, Vec<Range<usize>>, bool)],
        ranges: &[Range<usize>],
        changed: &[bool],
        tolerance: usize,
    ) -> Option<String> {
        if let Some(e) = &self.error {
            return Some(format!("el código escrito no carga ({})", e.message));
        }
        let near = |a: usize, b: usize| a.abs_diff(b) <= tolerance;
        for (name, slices, ours) in before {
            let Some(track) = self.tracks().iter().find(|t| t.name == *name) else {
                return Some(format!("la pista {name} desaparece"));
            };
            if !ours {
                if track.slices.iter().collect::<Vec<_>>() != *slices {
                    return Some(format!(
                        "cambiaría los cortes de {name}, que salen de otra llamada"
                    ));
                }
                continue;
            }
            let ratio = track.stretch.map_or(1.0, |s| s.ratio);
            let frames = track.sample.frames();
            let scale = |f: usize| ((f as f64 * ratio).round() as usize).min(frames);
            for (k, got) in track.slices.iter().enumerate() {
                let want = track
                    .origin
                    .indices
                    .get(k)
                    .copied()
                    .flatten()
                    .filter(|&i| changed.get(i).copied().unwrap_or(false))
                    .and_then(|i| ranges.get(i));
                if let Some(want) = want
                    && !(near(got.start, scale(want.start)) && near(got.end, scale(want.end)))
                {
                    return Some(format!(
                        "el corte {} de {name} no queda donde se pone: el código lo cambia \
                         después de slice() (:move(), :add(), :remove()…)",
                        k + 1
                    ));
                }
            }
        }
        None
    }

    /// El mensaje para un cambio de cortes que no se pudo escribir. El
    /// motivo se queda además en el borde del panel del sample
    /// ([`App::slice_warning`]).
    pub(super) fn not_written_message(&mut self, why: NotWritten) -> String {
        let (warning, message) = match why {
            NotWritten::Refused(why) => (
                format!("el cambio de cortes no se fijó: {why}"),
                format!("no se cambia: {why}"),
            ),
            NotWritten::Generated(why) => (
                format!("los cortes movidos no se guardan: {why}"),
                format!(
                    "no se puede guardar en el código: {why}; para editarlos desde la onda, \
                     escribe la llamada tal cual: slice(8) o slice({{ {{0, 120}}, {{120, 250}} }})"
                ),
            ),
        };
        self.slice_warning = Some(warning);
        message
    }

    /// Las parejas (sample, corte) del proyecto que ya no usa ninguna
    /// llamada del detector o de la rejilla porque se han pasado a cortes
    /// explícitos: sus ajustes, ediciones y etiquetas no se guardan más.
    pub(super) fn dropped_from_project(&self) -> Vec<(String, Cut)> {
        if self.error.is_some() || self.host.song().is_none() {
            return Vec::new();
        }
        let Some(song) = self.host.song() else {
            return Vec::new();
        };
        // Las pistas de fuera, las de las secciones y las de la otra
        // variante.
        let songs = std::iter::once(song).chain(song.alternate.as_deref());
        let tracks: Vec<&TrackDef> = songs
            .flat_map(|s| {
                s.tracks
                    .iter()
                    .chain(s.sections.iter().flat_map(|x| &x.tracks))
            })
            .collect();
        let used = |sample: &str, cut: Cut| {
            tracks
                .iter()
                .any(|t| !t.origin.explicit && t.origin.sample == sample && t.origin.cut() == cut)
        };
        self.converted
            .iter()
            .filter(|(sample, cut)| !used(sample, *cut))
            .cloned()
            .collect()
    }
}

/// Qué se mueve de un corte (con el ratón o con `,` `.` `<` `>`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Edge {
    /// Su inicio, sin tocar a los demás (queda un hueco o un solape).
    #[default]
    Start,
    /// Su final, sin tocar a los demás.
    End,
    /// El corte entero (inicio y final a la vez).
    Whole,
    /// La marca de su inicio con los cortes que empiezan o acaban justo
    /// ahí (como antes: el anterior acaba donde empieza este).
    Shared,
}

impl Edge {
    /// El siguiente con la tecla `b`.
    pub fn next(self) -> Edge {
        match self {
            Edge::Start => Edge::End,
            Edge::End => Edge::Whole,
            Edge::Whole => Edge::Shared,
            Edge::Shared => Edge::Start,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Edge::Start => "el inicio",
            Edge::End => "el final",
            Edge::Whole => "el corte entero",
            Edge::Shared => "la marca compartida (los vecinos la siguen)",
        }
    }

    /// El frame de este borde del corte `r` (el inicio, salvo el final).
    pub fn of(self, r: &Range<usize>) -> usize {
        match self {
            Edge::End => r.end,
            _ => r.start,
        }
    }
}

/// Adónde va el borde: tantos ms desde donde está (las teclas) o a esos
/// ms del sample original (el ratón, al soltar).
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Target {
    Delta(f64),
    At(f64),
}

/// Dónde se quedó un movimiento que no llegó adonde se pedía.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Limit {
    /// El principio del sample.
    SampleStart,
    /// El final del sample.
    SampleEnd,
    /// Su propio otro borde (no puede durar menos del mínimo).
    Own,
    /// El corte `j` (desde 0) con el que comparte la marca.
    Neighbour(usize),
}

/// `ranges` con `edge` del corte `i` llevado a `to` (en frames; con
/// `Whole`, el inicio nuevo), sin salirse de `0..total` ni dejar ningún
/// corte afectado con menos de `gap` frames. Devuelve los cortes y el tope,
/// si se quedó antes.
pub fn moved(
    ranges: &[Range<usize>],
    i: usize,
    edge: Edge,
    to: i64,
    total: usize,
    gap: usize,
) -> (Vec<Range<usize>>, Option<Limit>) {
    let mut out = ranges.to_vec();
    let Some(r) = ranges.get(i).cloned() else {
        return (out, None);
    };
    let clamp = |to: i64, low: (i64, Limit), high: (i64, Limit)| -> (usize, Option<Limit>) {
        if to < low.0 {
            (low.0.max(0) as usize, Some(low.1))
        } else if to > high.0 {
            (high.0.max(low.0).max(0) as usize, Some(high.1))
        } else {
            (to as usize, None)
        }
    };
    let (gap_i, total_i) = (gap as i64, total as i64);
    match edge {
        Edge::Start => {
            let (at, limit) = clamp(
                to,
                (0, Limit::SampleStart),
                (r.end as i64 - gap_i, Limit::Own),
            );
            out[i].start = at;
            (out, limit)
        }
        Edge::End => {
            let (at, limit) = clamp(
                to,
                (r.start as i64 + gap_i, Limit::Own),
                (total_i, Limit::SampleEnd),
            );
            out[i].end = at;
            (out, limit)
        }
        Edge::Whole => {
            let len = (r.end - r.start) as i64;
            let (at, limit) = clamp(
                to,
                (0, Limit::SampleStart),
                (total_i - len, Limit::SampleEnd),
            );
            out[i] = at..at + len as usize;
            (out, limit)
        }
        Edge::Shared => {
            let f = r.start;
            // Los que empiezan ahí (como este) y los que acaban ahí.
            let starts: Vec<usize> = (0..ranges.len())
                .filter(|&j| ranges[j].start == f)
                .collect();
            let ends: Vec<usize> = (0..ranges.len()).filter(|&j| ranges[j].end == f).collect();
            let mut low = (0i64, Limit::SampleStart);
            for &j in &ends {
                let bound = ranges[j].start as i64 + gap_i;
                if bound > low.0 {
                    low = (bound, Limit::Neighbour(j));
                }
            }
            let mut high = (total_i, Limit::SampleEnd);
            for &j in &starts {
                let bound = ranges[j].end as i64 - gap_i;
                if bound < high.0 {
                    high = (
                        bound,
                        if j == i {
                            Limit::Own
                        } else {
                            Limit::Neighbour(j)
                        },
                    );
                }
            }
            let (at, limit) = clamp(to, low, high);
            for &j in &starts {
                out[j].start = at;
            }
            for &j in &ends {
                out[j].end = at;
            }
            (out, limit)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const GAP: usize = 10;

    #[test]
    fn one_edge_moves_without_its_neighbours() {
        let r = vec![0..100, 100..200, 200..300];
        let (out, limit) = moved(&r, 0, Edge::End, 60, 300, GAP);
        assert_eq!(out, [0..60, 100..200, 200..300]);
        assert_eq!(limit, None);
        // Más allá del vecino: se solapa con él.
        let (out, _) = moved(&r, 0, Edge::End, 150, 300, GAP);
        assert_eq!(out, [0..150, 100..200, 200..300]);
        let (out, _) = moved(&r, 1, Edge::Start, 130, 300, GAP);
        assert_eq!(out, [0..100, 130..200, 200..300]);
        let (out, limit) = moved(&r, 1, Edge::Start, 195, 300, GAP);
        assert_eq!(out[1], 190..200);
        assert_eq!(limit, Some(Limit::Own));
        let (out, limit) = moved(&r, 2, Edge::End, 999, 300, GAP);
        assert_eq!(out[2], 200..300);
        assert_eq!(limit, Some(Limit::SampleEnd));
        let (_, limit) = moved(&r, 0, Edge::Start, -5, 300, GAP);
        assert_eq!(limit, Some(Limit::SampleStart));
    }

    #[test]
    fn the_whole_slice_moves_and_keeps_its_length() {
        let r = vec![0..100, 100..200];
        let (out, limit) = moved(&r, 1, Edge::Whole, 40, 300, GAP);
        assert_eq!(out, [0..100, 40..140]);
        assert_eq!(limit, None);
        let (out, limit) = moved(&r, 1, Edge::Whole, 250, 300, GAP);
        assert_eq!(out[1], 200..300);
        assert_eq!(limit, Some(Limit::SampleEnd));
    }

    #[test]
    fn the_shared_mark_takes_its_neighbours_along() {
        let r = vec![0..100, 100..200, 200..300];
        let (out, limit) = moved(&r, 1, Edge::Shared, 140, 300, GAP);
        assert_eq!(out, [0..140, 140..200, 200..300]);
        assert_eq!(limit, None);
        let (out, limit) = moved(&r, 1, Edge::Shared, 5, 300, GAP);
        assert_eq!(out, [0..10, 10..200, 200..300]);
        assert_eq!(limit, Some(Limit::Neighbour(0)));
        let (out, limit) = moved(&r, 1, Edge::Shared, 250, 300, GAP);
        assert_eq!(out, [0..190, 190..200, 200..300]);
        assert_eq!(limit, Some(Limit::Own));
        // Sin vecino en la marca, como el inicio solo.
        let r = vec![0..80, 100..200];
        let (out, _) = moved(&r, 1, Edge::Shared, 50, 300, GAP);
        assert_eq!(out, [0..80, 50..200]);
    }
}
