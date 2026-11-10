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

    /// El mensaje para un cambio de cortes que no se pudo escribir.
    pub(super) fn not_written_message(&self, why: NotWritten) -> String {
        match why {
            NotWritten::Refused(why) => format!("no se cambia: {why}"),
            NotWritten::Generated(why) => format!(
                "no se puede guardar en el código: {why}; para editarlos desde la onda, \
                 escribe la llamada tal cual: slice(8) o slice({{ {{0, 120}}, {{120, 250}} }})"
            ),
        }
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
