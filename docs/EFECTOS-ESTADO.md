# Efectos: inventario antes de flanger, phaser y formant por corte

Estado del código el 2026-11-19 (commit `7560512`), antes de añadir
flanger, phaser y chorus. Lo que se decidió y se hizo después está en
«Flanger, phaser y formant por corte» de [ESTADO.md](ESTADO.md): los
tres efectos existen ya, el phaser también por corte y por paso, y
`pista:formant` sigue la regla del corte.

## Lo que hay

Búsqueda de `flanger`, `phaser`, `chorus`, `vibrato`, `tremolo` y
`formant` en `script/src/api.rs`, `engine/src`, la guía y `git log`:

- **flanger, phaser, chorus, tremolo**: no existen. `chorus` solo sale en
  tests como ejemplo de efecto que no existe (`script/tests/sends.rs`,
  `slice_fx.rs`, `registry.rs`, `tui/src/diagnose.rs`) y en una nota de la
  guía sobre el campo estéreo.
- **vibrato**: solo el LFO de altura de `sh101{ lfo_pitch = ... }` (y el
  `lfo_wave`/`lfo_rate`/`lfo_sync` del mismo synth). No es un efecto.
- **formant**: existe, en tiempo real (ver abajo).

### Efectos de pista (`pista:fx(nombre, {...})`, `engine::Fx`)

Hasta 4 por pista (`MAX_FX`), en orden; también en los grupos
(`group()`).

| Efecto (alias) | Parámetros (los automatizables en cursiva) |
| --- | --- |
| `filter` | `type` (`lowpass`/`highpass`), *`cutoff`* 20–20000 Hz, *`q`* 0.1–20 |
| `lowpass` (`lp`), `highpass` (`hp`) | *`cutoff`*, *`q`* |
| `bitcrush` (`bitcrusher`, `crush`) | *`bits`* 1–16, *`downsample`* 1–64 |
| `compressor` (`comp`) | *`threshold`* −60–0 dB, *`ratio`* 1–20, *`attack`* 0.1–200 ms, *`release`* 1–2000 ms, *`makeup`* 0–24 dB |
| `distortion` (`drive`) | *`drive`* 1–50, *`mix`* 0–1 |
| `gain` | *`level`* 0–4 |
| `eq` | *`freq1..4`*, *`gain1..4`*, *`q1..4`*, `type1..4` (`bell`/`lowshelf`/`highshelf`) |
| `dyneq` | *`freq`*, *`q`*, `type`, *`threshold`*, *`ratio`*, *`range`*, *`attack`*, *`release`*, `sidechain` |
| `duck` (y `pista:sidechain`) | `source`, `mode`, *`depth`*, *`attack`*, *`release`*, `curve`, *`threshold`*, *`ratio`*, *`knee`*, `filter`, `tap`, `band` |

Fuera de la cadena: los envíos a los buses `delay` y `reverb`
(`pista:send`, `bus()`), el volumen, el pan y el limitador del master.

### Efectos de corte y de paso

- **`cortes:fx(i, nombre, {...})`**: la misma cadena que la pista (los
  mismos efectos y parámetros, hasta 4 y 8 curvas), salvo `duck` y el
  `sidechain` del `dyneq` (error: escuchan otra pista).
- **Llaves de un paso** (`3{lp=800,q=4}{crush=6}`, `script/src/step_fx.rs`):
  cualquier efecto del registro con su parámetro principal (el primero
  numérico y automatizable), o sus claves; rampas del golpe `200>4000:4`.
- **Forma del corte** (`cortes:set(i, {...})`, y en las llaves las de
  `api::STEP_SHAPE_KEYS`): `attack`, `release`, `pan`, `length`, `play`,
  `crossfade`, `dur`, `fit`, `loop_start`, `loop_end`, `bend`,
  `bend_time`, `rev`, `formant`, `warp`, `smear`. `warp` y `smear` se
  precalculan al cargar con caché (`engine::deform`); los demás, en la voz.

## Cómo se define un efecto

Un efecto toca estos sitios (todos con un `match` sin comodín, así que el
compilador avisa de los que faltan):

1. **Motor, `engine/src/fx.rs`**: la variante de `Fx` (la configuración,
   `Copy`, va en el `Pattern` y entra con él al compás); `Fx::params` (los
   nombres automatizables, en orden de índice: los nuevos al final),
   `param_range` (rango y si es logarítmico), `param_value` y `validate`
   (los rangos, con el texto del error). La variante de `FxState` (el
   estado en el hilo de audio, **de tamaño fijo**: es un enum `Copy` que
   vive en `[FxState; MAX_FX]` de cada pista, de cada grupo y de cada voz
   con efectos), `update` (crea el estado al entrar el patrón y conserva la
   memoria si la ranura ya tenía uno del mismo tipo: sin clic),
   `set_param` (lo automatizado, ya suavizado), `reset` (silencio) y
   `process`.
2. **Registro, `script/src/api.rs`**: un `Effect` en `EFFECTS` con sus
   `FxParam` (descripción, valor por defecto, `valid` con el rango en
   texto, `template` y `automate`: `Auto::Smooth` o `Auto::No(porqué)`) y
   el nombre en `effect_name`. De ahí salen el autocompletado, la ayuda, la
   guía (`cargo run -p script --example guia`), el linter, el ajuste de
   números con el ratón (`api::numeric`, que lee el rango de `valid`) y los
   errores de parámetros desconocidos.
3. **Lua, `script/src/lib.rs` `build_fx`**: de la tabla (o de las llaves
   del paso) al `Fx`, con los valores por defecto; las curvas (`ramp`,
   `lfo`, `lane`, `macro`) se apuntan con el nombre del parámetro y el
   valor fijo es su primer extremo; al final `Fx::validate`.
4. **Nombres en otros sitios**: `script/src/lanes.rs` `effect_names` (las
   lanes de `auto()`) y la etiqueta del panel de efectos en
   `tui/src/model.rs`.

**Automatización** (todo pasa por `engine::drive_block`): cada `MOD_BLOCK`
(32) frames, cada parámetro automatizable toma el destino de su curva
(`ramp`/`lfo`/`lane`/`macro`), o de un mando MIDI (`engine::knob`, que
manda según su peso) o de la ranura de gesto del ratón; lo suaviza con un
polo de ~5 ms y se lo da a `FxState::set_param`. Las lanes y las macros
son curvas más (`Curve::Lane`, `Curve::Macro`); MIDI learn apunta a
`(pista, ranura, parámetro)`. Un efecto nuevo con sus `params()` y su
`set_param` queda automatizable por todas las vías sin más.

**Silencio**: cuando una pista no tiene voz, ni notas, ni granos, sus
efectos se vacían (`rest_fx`, `reset`) y la pista no se procesa. La cola
de un filtro (unos ms) se corta ahí; un efecto con una cola larga
(retardo con realimentación) necesitaría decir que sigue sonando.

## Efectos por corte y por paso: dónde viven y cuánto cuestan

- **Dónde**: `engine/src/slice_fx.rs`. La cadena (`SliceFx`, hasta 4
  efectos y 8 curvas) va en el patrón, una por corte, y una tabla de
  cadenas de paso; cada golpe junta las dos (`SliceFx::with_step`: el paso
  sustituye el efecto del mismo tipo del corte, en su sitio, y los demás
  van detrás) y la copia en su voz (`VoiceFx`, con su `[FxState; MAX_FX]`),
  con 100 ms de cola tras acabar la voz.
- **Prioridad**: paso → corte → pista → mezcla. Los de la pista suenan
  sobre la suma de los golpes y no se quitan para un golpe.
- **Coste**: por voz, una cadena de `FxState` (el tamaño del enum por 4) y
  su CPU; 64 voces entre pistas por defecto (`[voces] limite`, hasta 256).
  Medido en la fase N: 4 efectos de paso en cada golpe de 8 pistas, 6,2 %
  de un núcleo (frente a 1,9 % sin nada).
- **Consecuencia para un efecto con memoria grande**: el estado tiene que
  caber en `FxState`, que se copia en cada voz. Una línea de retardo dentro
  del enum lo haría crecer para todos los efectos y todas las voces.

## Formant hoy

Ya existe, en tiempo real, con un solo nombre y una sola implementación:

- **Método**: LPC de orden 32 en f64 por grano, en el tiempo
  (`engine/src/shift.rs`), sobre el desplazador de granos de `%` (pitch sin
  cambiar la duración). Medido (fase K): una vocal a +7 queda a 1,3 dB de
  su envolvente original frente a 13,7 sin corrección. Coste: 4,8 % de un
  núcleo por voz (2,8 % sin formantes).
- **Dónde va**: `pista:formant(n)` (con `ramp`/`lfo`, por golpe) en los
  pasos con `%`; `formant` en `cortes:instrument` con `keep`; y por corte y
  por paso: `cortes:set(i, { formant = n })` y `3{formant=-3}` (con curva
  por golpe, `SHAPE_FORMANT`), que manda sobre el de la pista y vale también
  en golpes **sin** pitch (la voz pasa por los granos como un `%0`). Con
  `^`, error (los formantes se mueven con el tono).
- **Rango**: ±24 semitonos (`engine::MAX_FORMANT`).
- **Limitaciones**: el de la pista solo actúa en los pasos con `%` (en un
  paso sin pitch no hace nada, a diferencia del del corte); el material
  percusivo o con ruido suena artificial con granos; no existe la forma
  `cortes[3]:formant(3)` (sí `cortes:set(3, { formant = 3 })`).

## Lo que falta, según el encargo

- Flanger y phaser como efectos de pista (y de grupo), automatizables.
- Decidir con medidas si van por voz (corte y paso).
- Formant: unificar la regla de la pista con la del corte (también en
  golpes sin pitch) y documentar rangos recomendados y limitaciones. No
  hace falta otra implementación.
- Chorus, opcional.
