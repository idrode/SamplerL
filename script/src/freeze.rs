//! Congelar una pista: su salida tras sus efectos (y antes de su volumen,
//! pan, envíos y mute, lo que oye un sidechain) renderizada a un WAV que la
//! sustituye ([`Song::render_frozen`]), y `frozen(pista, wav, huella)`, que
//! al cargar cambia la pista por una que toca ese WAV una vez por vuelta,
//! sin fundidos ([`apply_frozen`]). Volumen, pan, envíos, mute, solo,
//! curvas de mezcla y grupo siguen siendo los del script: lo que viene
//! detrás de la pista suena igual.

use std::sync::Arc;

use engine::{Curve, LfoRate, Meter, Sample, Slices, Step};

use super::{
    DEFAULT_SHAPE, FreezeRequest, FrozenDef, MAX_SONG_BARS, MAX_TRACKS, ScriptError, SliceFxDef,
    SliceOrigin, Song, TrackDef, lcm, song_batch,
};

/// Lo que da [`Song::render_frozen`].
#[derive(Debug, Clone)]
pub struct FreezeRender {
    /// Una vuelta de la salida de la pista (estéreo entrelazado), la
    /// segunda del render: ya lleva las colas de la anterior.
    pub audio: Vec<f32>,
    /// Compases de la vuelta.
    pub bars: u64,
    /// Huella de la pista (para `frozen()`).
    pub fingerprint: String,
    /// Lo que no se puede congelar del todo (un LFO en Hz, una macro...).
    pub warnings: Vec<String>,
}

impl Song {
    /// Renderiza la pista `name` para congelarla: dos vueltas de la
    /// canción con la salida de esa pista (tras sus efectos, antes de su
    /// volumen; ver `Engine::set_tap`), y se queda con la segunda. La vuelta
    /// es el ciclo de la canción (todas las pistas: las que la disparan por
    /// sidechain cuentan) y el de sus curvas.
    pub fn render_frozen(&self, sample_rate: u32, name: &str) -> Result<FreezeRender, ScriptError> {
        let error = |why: String| ScriptError::new(format!("congelar \"{name}\": {why}"));
        if !self.sections.is_empty() {
            return Err(error(
                "con secciones no se congela (cada sección suena distinta)".into(),
            ));
        }
        if self.tempo_ramp.is_some() {
            return Err(error(
                "con una rampa de tempo no se congela (cada vuelta dura distinto)".into(),
            ));
        }
        let Some(track) = self.tracks.iter().find(|t| t.name == name) else {
            return Err(error("no existe esa pista".into()));
        };
        if track.frozen.is_some() {
            return Err(error("ya está congelada".into()));
        }
        if track.choke.is_some() {
            return Err(error(
                "tiene choke (sus golpes cortan otras pistas, y eso no cabe en un WAV)".into(),
            ));
        }
        let mut warnings = Vec::new();
        let bar = self.bar_steps() as u64;
        let mut steps = self.length_bars(1) * bar;
        for curve in curves(track) {
            match period_steps(&curve) {
                Some(period) => steps = lcm(steps, period),
                None => {
                    let what = match curve {
                        Curve::Macro { .. } => "una macro (se congela con su valor de al cargar)",
                        _ => "un lfo en Hz (no se repite con los compases)",
                    };
                    let warning = format!("{name} lleva {what}: congelada no suena igual");
                    if !warnings.contains(&warning) {
                        warnings.push(warning);
                    }
                }
            }
        }
        let bars = steps.div_ceil(bar);
        if bars > MAX_SONG_BARS {
            return Err(error(format!(
                "tarda {bars} compases en repetirse (máximo {MAX_SONG_BARS})"
            )));
        }
        if track.midi.is_some() {
            warnings.push(format!("{name} congelada no manda notas MIDI"));
        }
        let frames = self.frames(sample_rate, bars) as usize;
        let audio = self
            .render_tap(sample_rate, 2 * bars, name)
            .expect("la pista existe")?;
        Ok(FreezeRender {
            audio: audio[2 * frames..].to_vec(),
            bars,
            fingerprint: fingerprint(track, self.bpm_at(0), self.meter),
            warnings,
        })
    }

    /// Renderiza `bars` compases de la salida de la pista `name` tras sus
    /// efectos y antes de su volumen, pan, envíos y mute (lo que se congela,
    /// y lo que oye un sidechain), con las demás sonando dentro. `None` si
    /// no hay esa pista.
    pub fn render_tap(
        &self,
        sample_rate: u32,
        bars: u64,
        name: &str,
    ) -> Option<Result<Vec<f32>, ScriptError>> {
        let mut slots: [Option<String>; MAX_TRACKS] = Default::default();
        for (slot, track) in slots.iter_mut().zip(self.track_names()) {
            *slot = Some(track.to_string());
        }
        let tap = slots.iter().position(|s| s.as_deref() == Some(name))?;
        let render = || -> Result<Vec<f32>, ScriptError> {
            let (mut controller, mut engine) = engine::new_engine(sample_rate);
            let send = |e: engine::SendError| ScriptError::new(format!("motor: {e}"));
            controller
                .apply(song_batch(self, &slots, None)?)
                .map_err(send)?;
            controller.start().map_err(send)?;
            engine.set_tap(Some(tap));
            let frames = self.frames(sample_rate, bars) as usize;
            Ok(engine::offline::render(&mut engine, frames, 2))
        };
        Some(render())
    }

    /// Huella de la pista `name` como la escribe «Congelar»: si cambia, lo
    /// congelado ya no es lo que dice el script.
    pub fn freeze_fingerprint(&self, name: &str) -> Option<String> {
        let track = self.tracks.iter().find(|t| t.name == name)?;
        Some(fingerprint(track, self.bpm_at(0), self.meter))
    }
}

/// Las curvas que suenan dentro de la pista (las de la mezcla no: no van
/// en el WAV).
fn curves(track: &TrackDef) -> Vec<Curve> {
    let mut out: Vec<Curve> = track.automation.iter().map(|a| a.curve).collect();
    out.extend(track.pitch_curve);
    out.extend(track.formant_curve);
    for chain in &track.slice_fx {
        out.extend(chain.automation.iter().map(|a| a.curve));
    }
    for shape in &track.shapes {
        out.extend(shape.curves.iter().flatten());
    }
    if let Some(synth) = track.instrument.and_then(|i| i.synth) {
        out.extend(synth.curves.iter().flatten());
    }
    out
}

/// Pasos que tarda una curva en repetirse (`None`: no se repite con los
/// pasos).
fn period_steps(curve: &Curve) -> Option<u64> {
    let steps = match *curve {
        Curve::Ramp { steps, .. } => steps,
        Curve::Lfo {
            rate: LfoRate::Steps(steps),
            ..
        } => steps,
        // La rampa de un golpe vuelve a empezar con su golpe.
        Curve::Hit { .. } => return Some(1),
        Curve::Lfo { .. } | Curve::Macro { .. } => return None,
    };
    (steps.fract() == 0.0 && steps >= 1.0).then_some(steps as u64)
}

/// FNV-1a de todo lo que suena dentro de la pista (lo que va en el WAV),
/// con el tempo y el compás, en 16 cifras hexadecimales.
pub(crate) fn fingerprint(track: &TrackDef, bpm: f64, meter: Option<Meter>) -> String {
    let text = format!(
        "{:?}",
        (
            (
                &track.path,
                track.sample.frames(),
                track.sample.sample_rate()
            ),
            (&track.slices, track.length, &track.bars),
            (&track.fx, &track.automation, &track.sidechains),
            (
                &track.shapes,
                &track.slice_fx,
                track.instrument,
                track.granular
            ),
            (
                track.pitch,
                track.pitch_curve,
                track.formant,
                track.formant_curve
            ),
            (track.gate, track.stretch, bpm, meter),
        )
    );
    let hash = text.bytes().fold(0xcbf2_9ce4_8422_2325u64, |h, b| {
        (h ^ b as u64).wrapping_mul(0x100_0000_01b3)
    });
    format!("{hash:016x}")
}

/// `frozen()`: la pista de `request` pasa a tocar su WAV. Error con la
/// línea si no hay esa pista o tiene choke; aviso si la huella no
/// coincide (ha cambiado) o el WAV no dura compases enteros a este tempo.
pub(crate) fn apply_frozen(
    tracks: &mut [TrackDef],
    request: &FreezeRequest,
    bpm: f64,
    meter: Option<Meter>,
    sample_rate: u32,
    warnings: &mut Vec<String>,
) -> Result<(), ScriptError> {
    let who = format!(
        "{}frozen(\"{}\", \"{}\")",
        request.location, request.track, request.wav
    );
    let Some(track) = tracks.iter_mut().find(|t| t.name == request.track) else {
        return Err(ScriptError::new(format!(
            "{who}: no hay ninguna pista \"{}\"",
            request.track
        )));
    };
    if track.choke.is_some() {
        return Err(ScriptError::new(format!(
            "{who}: la pista tiene choke y congelada no podría cortar a las otras"
        )));
    }
    let mut stale = request
        .fingerprint
        .as_ref()
        .is_some_and(|f| *f != fingerprint(track, bpm, meter));
    let steps = meter.map_or(engine::STEPS, |m| m.steps);
    let bar = engine::bar_frames_steps(steps, bpm, sample_rate) as usize;
    let frames = request.sample.frames();
    let bars = frames.div_ceil(bar).max(1) as u64;
    if !frames.is_multiple_of(bar) {
        stale = true;
    }
    if stale {
        warnings.push(format!(
            "la pista {} ha cambiado desde que se congeló (o el tempo): suena lo congelado; \
             «Descongelar» y «Congelar» otra vez para ponerla al día",
            request.track
        ));
    }
    if track.midi.is_some() {
        warnings.push(format!("{} congelada no manda notas MIDI", request.track));
    }
    *track = frozen_track(track, request, bars, steps, stale);
    Ok(())
}

/// La pista que toca el WAV: un golpe al empezar cada vuelta de `bars`
/// compases (sin fundidos), sin efectos ni nada de lo que va dentro; con
/// el volumen, el pan, los envíos, el mute, el solo y las curvas de mezcla
/// de la original.
fn frozen_track(
    original: &TrackDef,
    request: &FreezeRequest,
    bars: u64,
    steps: usize,
    stale: bool,
) -> TrackDef {
    let sample: Arc<Sample> = request.sample.clone();
    let mut first = vec![None; steps];
    first[0] = Some(Step::new(0));
    let rest = vec![None; steps];
    let cycle: Vec<_> = (0..bars)
        .map(|b| if b == 0 { first.clone() } else { rest.clone() })
        .collect();
    TrackDef {
        slices: Slices::grid(&sample, 1),
        sample,
        path: request.path.clone(),
        length: steps,
        steps: first,
        bars: cycle,
        fx: Vec::new(),
        automation: Vec::new(),
        sidechains: Vec::new(),
        labels: vec![None],
        adjusted: vec![false],
        swing: 50.0,
        humanize: None,
        pitch: 0.0,
        pitch_curve: None,
        formant_curve: None,
        shapes: vec![DEFAULT_SHAPE],
        gate: false,
        choke: None,
        midi: None,
        origin: SliceOrigin {
            sample: request.wav.clone(),
            count: 1,
            grid: true,
            indices: vec![None],
            zero_cross: None,
        },
        stretch: None,
        instrument: None,
        granular: None,
        formant: None,
        slice_fx: vec![SliceFxDef::default()],
        spans: Vec::new(),
        frozen: Some(FrozenDef {
            wav: request.wav.clone(),
            bars,
            stale,
        }),
        ..original.clone()
    }
}
