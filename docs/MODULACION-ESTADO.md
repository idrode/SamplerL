# Modulación: inventario antes de la modulación por golpe

Estado del código el 2026-11-19 (commit `2d1b01d`), antes de añadir la
modulación por golpe. Lo que se decide y se hace después va en
«Modulación por golpe» de [ESTADO.md](ESTADO.md).

## Modulación global (la que hay)

Todo lo que se mueve hoy sigue el **reloj de la canción**, no el golpe:

- **`Curve`** (`engine/src/automation.rs`): `Ramp` (diente de sierra de
  `steps` pasos alineado con la canción), `Lfo` (seno entre `min` y `max`,
  en Hz o en pasos por ciclo; fase = segundos o pasos de la canción),
  `Hit` (rampa de un golpe: solo en los efectos de un paso, cuenta desde el
  paso del golpe, una vez), `Macro` (sigue una macro de 0 a 1) y `Lane`
  (apunta a la tabla de lanes del motor). `Curve::value_in(steps, seconds,
  origin, env, log)` es una función pura de la posición: no acumula nada.
- **Lanes** (`engine/src/lane.rs`): hasta 64 puntos `(at, valor, curva)`
  con curvas `Linear`, `Exp`, `Smooth` y `Step`, una duración en pasos,
  modos `Loop`/`Once`/`Hold`, desplazamiento y valor fijo. `Lane::at(t)`
  interpola en una posición dentro de la lane: **es reutilizable** con
  cualquier eje de tiempo (la validación de `Lane::new` también).
- **Dónde se evalúan**: cada `MOD_BLOCK` (32) frames, en `drive_block`
  (efectos de pista, grupo y voz), en la mezcla (volumen, pan, envíos), en
  los buses y en el sintetizador (`Synth::mix_into`, sus parámetros), con
  un polo de ~5 ms (`GAIN_SMOOTHING_SECONDS`) por encima. Las curvas «por
  golpe» (`Auto::PerHit`: pitch y formante de la pista, la forma del
  corte) se miran **una vez** al sonar el golpe y no se mueven dentro de él.
- **Mandos MIDI** (`engine/src/knob.rs`): mandan sobre la curva o el valor
  fijo según su peso (`KnobLive::apply`), en efectos, mezcla, buses y
  sintetizador; también la ranura de gesto del ratón.

## Lo que ya es «por golpe» o «por nota»

- **Envolventes ADSR de los sintetizadores** (`engine/src/synth/env.rs`):
  amplitud y filtro de cada voz, por muestra (segmentos exponenciales con
  estado `Stage`), disparadas en `Voice::play` y soltadas en
  `Voice::release`; `sh101` tiene una sola envolvente, `acid` y `sub` las
  suyas con accent. Son del sintetizador (no salen de él ni tienen hold ni
  curva configurable); la nueva AHDSR no las sustituye.
- **`bend`/`bend_time`** del corte (`engine.rs`, `Bend`): una curva de tono
  por golpe con forma cerrada (exponencial en el tiempo de la voz) que se
  lee al nacer el golpe; la voz lee el sample con su integral (`offset`),
  no con `frame · velocidad`.
- **Efectos de corte y de paso** (`3{lp=800}`, `engine/src/slice_fx.rs`):
  cada golpe copia su cadena (`VoiceFx`) con estado propio; las rampas de
  golpe (`200>4000:4`) cuentan desde su paso. Es el único sitio donde una
  curva arranca con el golpe, y solo es una rampa lineal por parámetro.
- **Expresión por nota (MPE)** en el sintetizador (`ExprMap`,
  `Synth::note_mod`/`set_expr`): cada voz lleva sus propios valores de
  bend, presión y slide y el bloque de la voz (`Block`) interpola por
  muestra su altura (`bend`), su volumen (`gain`) y sus parámetros
  (`from`/`to`, el corte en octavas). **Es el enganche para la modulación
  por golpe de los sintetizadores.**

## Voces, golpes y note-off

- **Golpe de corte** (`Engine::play_hit(t, hit, gate)`): crea una `Voice`
  (sample, rango, `Playback` con volumen, pan, velocidad de lectura, forma
  del corte, efectos de voz). `gate` son los frames del paso y sus `_`.
  Una pista de cortes es **monofónica**: el golpe siguiente pasa el
  anterior a `fading` (5 ms de fundido); el choke corta las de su grupo.
- **Fin de un golpe**: en modo one-shot (por defecto) suena el corte
  entero (o su `dur`, `length`, bucle hasta `dur`) **sin note-off**; en
  modo gate (`pista:mode("gate")`), la puerta se cierra a los `gate`
  frames (`Voice::gate_at`) y empieza su cola; `release` del corte, choke,
  mute en vivo y el pánico hacen un fundido.
- **Instrumento de samples** (`instrument.rs`, `Note`): polifónico
  (`MAX_VOICES` por pista), note-off a los `gate` frames del paso, al
  soltar la tecla MIDI o al robar la voz.
- **Sintetizador** (`synth/mod.rs`): voces fijas por pista; `NoteEvent`
  lleva `gate` (frames hasta el release, o `HELD` hasta el note-off de la
  tecla), `accent`, `slide` y la tecla; legato (`tie`) no vuelve a
  disparar las envolventes; el note-off es `Voice::release`.
- **Polifonía**: 64 voces entre pistas por defecto (`[voces] limite`, hasta
  256), con robo de la más antigua.

## Velocidad y nota

- No hay un campo «velocidad»: el **volumen del golpe** hace de velocidad.
  El `@0.5` de un paso, la velocidad MIDI de entrada (`vel/127`) y la de
  salida (`volumen · 100`) son el mismo número. `!` (accent) es aparte y
  solo en los sintetizadores.
- La **nota** de un golpe de corte es su pitch en semitonos (sin pitch,
  el corte tal cual: do 4, 60); en un sintetizador o instrumento, la nota
  MIDI.

## Dónde viven volumen, pan, pitch y filtro de cada voz

- Corte: `Playback::gain` (del paso), `SliceShape::pan` (del corte),
  `Playback::rate` (pitch del paso más el de la pista), sin filtro propio
  (solo el de un efecto de corte o de paso, en `VoiceFx`).
- Sintetizador: `Voice::gain`, el pan de cada oscilador (`spread`),
  `Voice::pitch` con glide, el filtro de la voz con su envolvente.

## Prioridades de hoy

1. Efecto del paso → efecto del corte (sustituye el del mismo tipo) →
   efectos de la pista → volumen, pan y envíos de la pista → mezcla.
2. `~` (corte del primer filtro por paso) manda sobre la automatización
   de ese filtro.
3. Por golpe: la forma del paso manda sobre la del corte; el pitch del
   paso se suma al de la pista.
4. Mando MIDI sobre curva o valor fijo, según su peso; las curvas (lanes,
   macros, `ramp`, `lfo`) sobre el valor fijo.

## Qué existe y qué falta

| Pieza | Hoy | Decisión |
| --- | --- | --- |
| Envolvente de puntos | `Lane` (puntos, curvas, validación) por reloj | Reutilizar `Lane` (`Lane::new`, `Lane::at`) con el tiempo del golpe; bucle, sustain y release, nuevos |
| AHDSR | ADSR propio de cada sintetizador, incremental | Nueva, de forma cerrada (tiempo desde el golpe y desde el note-off), con hold y curvas |
| LFO por golpe | `Curve::Lfo` (fase de la canción) | Nuevo: fase desde el golpe; `sync` con la aritmética entera de `modfx::Sync` |
| Stepper, fader, operando | — | Nuevos |
| Velocidad y nota | el volumen del golpe; la nota | Velocidad = volumen del golpe (0 a 1), accent = 1; nota = la del golpe |
| Parámetros de los dispositivos con curvas | `Curve` en todas partes | Reutilizar `Curve` (ramp, lfo, macro, lane) por bloque |
| Enganche en el sintetizador | la expresión por nota (`Block`) | Reutilizar: altura, volumen y corte por voz |
| Enganche en un corte | `Voice` sin filtro ni pitch variable | Nuevo: volumen, pan, pitch (lectura acumulada) y un filtro por voz |
| Tabla en el motor por lote | lanes (`Box<Lanes>` al compás, basura) | Igual: una tabla de mod sets que entra al compás |
| Note-off de un corte | solo en modo gate | Definirlo: el final de su paso (y sus `_`) o de su `dur` |
