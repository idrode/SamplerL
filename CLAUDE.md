# Sampler de breaks con live coding (terminal)

Sampler + secuenciador para remixes con amen breaks, controlado por código en vivo. Proyecto en Rust. El desarrollo se hace en una laptop x86_64 con Gentoo.

## Idea central

- Panel izquierdo: editor de código. Panel derecho: vistas del estado. Panel inferior: efectos por pista.
- El código NO es la única fuente de verdad: el script produce un **modelo de estado** (pistas, patrones, cortes, efectos) y las vistas lo leen. Editor y paneles siempre coinciden.
- Layout objetivo:
  - Izquierda: editor de código.
  - Derecha arriba: sample (forma de onda, cortes, loop, slicer).
  - Derecha abajo: secuencia de pistas en vertical, estilo tracker (Renoise).
  - Inferior: efectos de cada pista.

## Stack

- Audio: `cpal`. Decodificación: `symphonia` (WAV, MP3, FLAC, OGG) y `hound`. Resampleo: `rubato`.
- Interfaz: solo terminal (TUI) con `ratatui` 0.30 + `ratatui-textarea` 0.9 (la continuación de `tui-textarea` dentro de la organización de ratatui; `tui-textarea` 0.7 obligaba a quedarse en ratatui 0.29). Decisión del usuario: nada de GUI por ahora, experiencia 100% terminal.
- Scripting: mini-notación para patrones (estilo Tidal) + Lua con `mlua` para la lógica. Recarga en caliente.
- DSP: `fundsp` para synths y efectos cuando toque.

## Reglas de arquitectura (no negociables)

1. El callback de audio NO reserva memoria, NO usa mutex ni bloquea, NO hace E/S.
2. El script y la UI nunca tocan el audio directamente: envían comandos por una cola sin bloqueos (lock-free) al hilo de audio.
3. Motor separado de la interfaz (crate de librería + frontend TUI). No se hace GUI, pero la separación mantiene el código limpio y testeable.
3b. Pensar la UI para terminal: rejilla de caracteres, bloques/braille para formas de onda, atajos de teclado y ratón opcional. Usar truecolor con degradación elegante.
4. Los cambios evaluados entran cuantizados al final del compás.
5. Si el código es inválido, el patrón anterior sigue sonando y el error se muestra en el editor.
6. Los samples se decodifican a buffer en memoria y se resamplean a la frecuencia del motor al cargarlos.

## Orden de construcción

Un paso cada vez; probar con tests (audio sintético, renderizado offline) antes de seguir.

1. ✅ Motor de audio: cargar un WAV y reproducirlo (por la tarjeta solo con `--allow-audio`; queda el aviso de underrun al arrancar, ver Estado).
2. ✅ Cortes: detección de transitorios (flujo espectral por defecto); cada corte se reproduce por número (`Controller::play_slice`). Asignar cortes a teclas de la interfaz: pendiente.
3. ✅ Secuenciador de 16 pasos con BPM, loop y cuantización al compás; varias pistas y lotes atómicos.
4. ✅ Efectos por paso (volumen, pitch, reverse, retrigger/stutter) y por pista (volumen, filtro paso bajo/alto, bitcrusher), con limitador suave al final.
5. ✅ Mini-notación + Lua (`bpm`, `sample`, `slice`, `slice_at`, `track`, `steps`, `gain`, `fx`, `every`, `chance`) con recarga al compás y recarga automática al guardar.
6. ✅ Interfaz de paneles en la terminal (`tui --edit`): editor, sample en braille, tracker vertical, efectos por pista y estado del motor.
7. En parte: el render offline a WAV ya existe (`--render`, también con `--script`). Falta guardar/cargar proyectos (hoy el proyecto es el `.lua` más sus WAV).

## Ideas para después

- Deformar y modificar el sample al importarlo (pitch, time-stretch, granular, etc.) y tocar melodías con él.
- Instrumentos propios (bajo y synths con osciladores, ADSR, filtro; evitar aliasing con PolyBLEP o wavetables).
- Salida MIDI hacia Surge XT (o similar) y captura del audio por PipeWire/JACK.
- Alojar plugins CLAP/LV2: solo si hiciera falta; es costoso.
- Time-stretch de calidad con una librería (p. ej. Rubber Band).

## Fuera de alcance realista

Alojar VST, mezclador completo con buses, automatización compleja. No intentarlo.

## Ejemplo de la API objetivo (Lua)

Toda esta API existe ya (pasos 4 y 5; ver Estado). Cada llamada solo construye el modelo de estado; el motor aplica los cambios al final del compás.

```lua
bpm(170)

local amen = sample("amen.wav"):slice(8)

track("drums", amen)
  :steps("1 . . 1  . . 3 .  1 . 5 .  . 3 . 6")

track("drums")
  :fx("filter", { type = "lowpass", cutoff = 3000 })
  :every(4, function(t) t:reverse() end)
  :chance(0.2, "retrig", 4)

track("kick", amen:slice_at(1))
  :steps("x . . . x . . . x . . . x . . .")
  :gain(0.9)
```

Hecho: `steps(...)` reproduce los cortes, y `fx`, `every` y `chance` funcionan (ver Estado, paso 5).

## Estado

- **Paso 1 hecho.** Workspace con `engine` (librería) y `tui` (binario mínimo: render offline; reproducir por la tarjeta exige `--allow-audio`, ver Notas; aún sin ratatui).
- Decisiones técnicas:
  - Samples en memoria como estéreo f32 entrelazado (mono se duplica, >2 canales se usan los dos primeros), resampleados al cargar con `rubato::Fft`. La longitud de salida se calcula con enteros porque rubato 5 puede dar un frame de más.
  - Carga de WAV con `hound`; `symphonia` todavía no se usa.
  - `new_engine()` devuelve `Controller` (control) y `Engine` (audio). Comandos por cola SPSC lock-free (`rtrb`). Los `Arc<Sample>` que el motor deja de usar vuelven por una segunda cola ("basura") y se liberan en el hilo de control: el callback nunca libera memoria.
  - Salida con `cpal` 0.18, dispositivo por defecto; el callback convierte formato con un bloque intermedio en la pila.
  - Test de render offline en `engine/tests/`: se llama a `Engine::process` por bloques sin dispositivo de audio.
- **Paso 2 hecho.** `Slices::detect(&sample, n)` (`engine/src/slice.rs`) busca los `n` transitorios más fuertes (máximos locales de una función de detección por encima de un umbral), con separación mínima, y afina cada uno al frame donde empieza el ataque. Los cortes son contiguos (hasta el siguiente transitorio o el final); lo anterior al primer transitorio queda fuera. Si hay menos golpes que `n`, devuelve menos cortes. Dos detectores (`DetectOptions::detector`, en el binario `--detector flux|energy`):
  - `Detector::Flux` (por defecto): flujo espectral por bandas con `rustfft`. FFT de 512 con Hann y salto de 256. Bandas de tercio de octava de al menos 3 bins, comprimidas con `ln(1 + 100·amplitud)`. La subida de cada banda se mide respecto al máximo de esa banda y sus vecinas dos saltos antes (estilo SuperFlux), para no tomar el ruido de una cola por golpes. La función es la media por banda de las subidas, así que un charles débil en agudos pesa como un bombo en graves. El afinado se hace sobre la diferencia entre muestras (realza agudos), en ventanas de 32 frames.
  - `Detector::Energy`: RMS por salto con compresión logarítmica y afinado sobre la amplitud. Falla con colas de bombo graves: la energía oscila y un charles débil encima no se distingue.
  - `Controller::play_slice(&sample, &slices, i)` resuelve el rango en el hilo de control y envía `Play { sample, start, end }`; el motor solo recorre frames. Errores: `SendError::{QueueFull, NoSuchSlice}`.
  - Tests en `engine/tests/slicing.rs` (golpes en posiciones conocidas con ambos detectores, charles más débil que la cola de un bombo) y `engine/tests/synth_break.rs` (los 8 golpes del break sintético a menos de 2 ms; error medido < 0,5 ms). El generador está en `engine/tests/support/synth_break.rs` y lo comparte el ejemplo.
  - Separación mínima entre transitorios: parámetro `DetectOptions::min_gap_seconds` (por defecto 40 ms, `Slices::detect_with`). Se compara entre posiciones ya afinadas, en frames.
- **Fundidos**: cada voz tiene rampas de entrada y salida de 1,5 ms. Una voz interrumpida (nuevo disparo o `stop`) pasa a una segunda ranura y hace su fundido mientras entra la nueva. Tests en `engine/tests/fades.rs`.
- **Render offline**: `tui <archivo.wav> --render <salida.wav>` renderiza a 48 kHz sin abrir el dispositivo (WAV PCM 16 bits). Con `--pattern "1 . 3 . ..."` (16 pasos en mini-notación, ver paso 5) y opcionalmente `--slices N --bpm B --bars M --min-gap MS` renderiza exactamente M compases del secuenciador. Código en `engine::offline`.
- **Paso 3 hecho.** Secuenciador de 16 pasos (`engine/src/sequencer.rs`):
  - `Pattern::new(sample, &slices, [Option<usize>; 16])` resuelve los cortes a rangos en el hilo de control; el motor recibe un array fijo, sin reservas. Un paso `None` no dispara nada (lo que sonaba sigue).
  - `Controller::{set_pattern, set_bpm, start, stop}`. `set_pattern` y `set_bpm` quedan pendientes y entran en el paso 0 del siguiente compás (o al arrancar). `stop` es inmediato.
  - El paso k cae en `inicio_compás + round(k · frames_por_paso)`, y cada compás empieza donde acabó el anterior. No hay deriva aunque los frames por paso no sean enteros. El bloque de audio se parte en los límites de paso.
  - `new_engine(sample_rate)`: el motor necesita su frecuencia; `open_output` la ajusta a la del dispositivo.
  - `Engine::discard` suelta el `Arc` directamente si el motor guarda otra referencia al mismo sample (solo baja el contador). Así las voces que dispara el secuenciador no llenan la cola de basura.
  - Varias pistas (`MAX_TRACKS` = 8), cada una con su patrón, su voz y su ranura de fundido; suenan a la vez y se suman. `Controller::set_pattern` usa la pista 0; `set_track_pattern(t, Option<Pattern>)` cualquier pista (`None` la silencia). `play*` suena en la pista 0.
  - Lotes atómicos: `Controller::apply(Batch)` con tempo y patrones de varias pistas. Antes de enviar valida todo y comprueba que cabe en la cola (o no envía nada). El motor guarda los cambios aparte entre `BeginBatch` y `CommitBatch`, así que un lote leído a medias nunca entra a medias en un compás. Tests en `engine/tests/tracks.rs` y en `engine.rs` (lote leído a medias).
  - Tests en `engine/tests/sequencer.rs`: frame exacto de cada paso a 120 y 170 BPM, silencios, loop y cuantización.
- **Paso 4 hecho.** Efectos por paso (volumen, pitch, reverse, retrigger) y por pista (volumen, filtro, bitcrusher):
  - `Step { slice, gain, semitones, reverse }` (con `Step::new(i).with_gain(g).with_pitch(st).reversed()`) y `Pattern::from_steps(sample, &slices, [Option<Step>; 16])`. Se valida en el hilo de control: ganancia 0–4, pitch ±36 semitonos (`SendError::InvalidStep`).
  - La voz lee en posición fraccionaria (f64) con interpolación lineal: el pitch cambia la velocidad y la duración, sin time-stretch. Con pitch 0 la salida es idéntica bit a bit a la de antes. El reverse lee de `end-1` a `start`. Los fundidos van en tiempo de salida, así que son simétricos: el reverse es exactamente la inversión temporal del forward.
  - Dentro de una pista no hay polifonía: cada paso corta al anterior con fundido cruzado, como en un tracker.
  - Interpolación lineal: subiendo mucho el pitch habrá aliasing. Si molesta, cambiar a una interpolación mejor (cúbica o sinc corta).
  - Retrigger/stutter por paso (`Step::with_retrig(n)`, `3r4` en la notación, de 1 a 16): el paso se divide en n partes y el corte vuelve a empezar en cada una, con frame exacto (`round(j · duración_paso / n)`). Se programa en el motor como los pasos.
  - Efectos de pista (`engine/src/fx.rs`): `Pattern::with_fx(&[Fx])`, cadena de hasta 4 que entra con el patrón. `Fx::lowpass/highpass(corte)` (biquad RBJ, 12 dB/oct, `q` configurable) y `Fx::bitcrush(bits, downsample)`. El estado vive en arrays fijos de cada pista; un efecto del mismo tipo en la misma ranura conserva su memoria al cambiar de patrón, y en silencio se vacía.
  - Volumen de pista: `Pattern::with_gain(g)` (0–4), entra con el patrón. Se suaviza (~5 ms) si la pista suena; si está callada, entra directo.
  - Mezcla en trozos de hasta 256 frames con buffers estéreo fijos (pista y maestro) dentro de `Engine`: voces → efectos → volumen de pista → maestro → limitador → salida (en mono se promedia).
  - Limitador (`engine::soft_limit`): recortador suave sin memoria, lineal hasta 0,9 y curva tanh por encima; nunca pasa de ±1 y un NaN se convierte en silencio. No es un limitador con envolvente ni lookahead: si se nota la saturación, añadir uno de verdad.
  - Tests en `engine/tests/step_effects.rs`, `effects.rs` y `mix.rs` (8 pistas saturadas, suma ~±115, nunca pasan de ±1).
- **Paso 5 (mini-notación hecha)** en `engine::notation` (`parse_steps`, `parse_step`), usada por `--pattern`, `PATTERN` del script y Lua. 16 pasos separados por espacios (los espacios de más no cuentan):
  - `.` silencio; `3` corte 3 (desde 1); `x` = `1`.
  - Sufijos combinables, en cualquier orden, cada uno una vez: `'` reverse, `^7` / `^-5` / `^0.5` pitch en semitonos, `@0.5` volumen, `r4` retrigger. Ejemplo: `3'^7@0.5r2`.
  - Errores con número de paso y token: `paso 11 ("5^"): falta el número después de ^`. Los rangos (ganancia 0–4, pitch ±36, retrigger 1–16) los valida `Pattern::from_steps`.
  - Tests en `engine/tests/notation.rs` (incluido que un patrón en notación renderiza igual que el construido con `Step`).
- **Paso 5 (Lua hecho)** en el crate `script` (`mlua` 0.12 con Lua 5.4 `vendored`; compila con el gcc del sistema). Depende de `engine`; `tui` depende de los dos.
  - `ScriptHost::load(controller, fuente, nombre, carpeta)` / `load_file(controller, ruta)`: evalúa el script en un Lua nuevo, construye el modelo `Song { bpm, tracks: Vec<TrackDef> }` y lo envía con `Controller::apply` como un lote: todo entra junto en el siguiente compás. `ScriptHost::song()` es el último modelo válido (lo leerán las vistas).
  - Si algo falla, no se envía nada y sigue sonando lo anterior. Esto incluye sintaxis, errores de ejecución, notación, corte inexistente, archivo que no existe, pista no definida, BPM fuera de rango, más de 8 pistas o un script que tarda más de 2 s (`set_time_limit`; se corta con un hook cada 10000 instrucciones).
  - Errores con archivo y línea: `song.lua:3: steps: paso 11 ("5^"): falta el número después de ^`.
  - API: `bpm(n)`, `sample(ruta)` (relativa a la carpeta del script), `:slice(n)` (detector por defecto; `:count()` da los cortes), `:slice_at(i)` (solo el corte i, desde 1; en el patrón es `1`/`x`), `track(nombre, sliced)` define o redefine una pista, `track(nombre)` la recupera. Métodos de pista, encadenables: `:steps("...")` en mini-notación, `:gain(g)`, `:fx(nombre, {params})`, `:every(n, fn)` y `:chance(p, efecto[, valor])`.
  - `:fx`: `"filter"` (`type` = `"lowpass"`/`"highpass"`, `cutoff`, `q`), `"lowpass"`/`"highpass"` (`cutoff`, `q`), `"bitcrush"` (`bits` = 8, `downsample` = 1). Una clave desconocida da error (para detectar erratas).
  - `:every(n, fn)`: como en Tidal, en los compases 0, n, 2n… de la canción se llama `fn(compás)`. El compás admite `steps`, `reverse` (invierte el sentido de cada golpe), `pitch`, `retrig` y `gain`. Llamar a `track`, `bpm` o `sample` dentro da error.
  - `:chance(p, efecto[, valor])`: cada golpe sufre el efecto con probabilidad p. Efectos: `retrig`/`stutter` n, `reverse`, `pitch` st, `gain` g, `mute`. `ScriptHost::set_seed(Some(s))` hace el azar reproducible (los tests usan una semilla fija); sin semilla cambia en cada carga.
  - `every` y `chance` se expanden al evaluar a un ciclo de compases (`TrackDef::bars`): longitud = mcm de los `every`, con 16 compases si hay `chance`; máximo 64. El motor (`Pattern::from_bars`) toca `bars[compás_global % longitud]`: es determinista y queda alineado con el compás de la canción aunque se recargue. El azar de `chance` se repite cada 16 compases.
  - Los patrones que el motor suelta vuelven enteros por la cola de basura, porque llevan memoria propia (sus compases).
  - Solo se cargan las librerías `math`, `string`, `table` y `utf8` (sin `io` ni `os`).
  - Los samples se cachean por ruta y fecha de modificación: recargar no vuelve a decodificar y conserva el mismo `Arc`. Cada nombre de pista conserva su pista del motor entre recargas; las pistas que desaparecen del script se silencian en el compás siguiente.
  - Render: `cargo run -p tui -- --script scripts/demo.lua --render renders/demo-lua.wav --bars 8` (necesita `renders/synth_break.wav`, que genera `scripts/render-demo.sh`). `scripts/demo.lua` usa toda la API.
  - Recarga automática al guardar (`script/src/watch.rs`): `ScriptWatcher::poll(host, controller)` lee el archivo y, si su contenido cambió, lo carga con `ScriptHost::load`. Mismas garantías: entra entero en el compás siguiente y, si falla, sigue sonando lo anterior. Compara contenido, no fecha, así que un guardado a medias da un error y el siguiente sondeo recoge la versión completa. Los errores de lectura (archivo borrado un momento) se informan una vez. Un script que falló no se reintenta hasta que cambie. Sin dependencias nuevas (sondeo, no `notify`).
  - `tui --script canción.lua --allow-audio` reproduce y recarga al guardar (sondeo cada 200 ms). Claude Code no lo ha ejecutado nunca: abre la tarjeta.
  - Tests en `script/tests/lua.rs`: API con dos pistas, `track(nombre)`, recarga cuantizada (patrón y tempo), 11 tipos de error con el audio idéntico al de no haber recargado, caché de samples y pistas estables. Además, en `script/tests/api.rs`: el ejemplo de CLAUDE.md tal cual, `slice_at`, `every`, `chance` con semilla, `fx` igual que en el motor y 16 errores de la API con línea y audio intacto. Y en `script/tests/watch.rs`: recarga al guardar cuantizada, guardado a medias, error de ejecución, archivo borrado y sin reintento hasta que cambie.
- **Paso 6 hecho.** Interfaz de terminal: `tui --edit canción.lua` (`tui/src/{app,model,ui}.rs`).
  - Paneles: editor a la izquierda; a la derecha, estado del motor (compacto), sample (forma de onda en braille con los cortes numerados y el que suena resaltado) y tracker vertical (16 filas, una columna por pista, celdas en mini-notación, fila actual resaltada y desplazamiento si no cabe); abajo, efectos por pista (volumen, nivel y cadena de efectos).
  - Cada panel es un `Widget` que solo lee el `ViewModel` (`tui/src/model.rs`). `App::view()` lo construye con el `Song` del script, la instantánea del motor y el estado del editor. La forma de onda va como resumen mínimo/máximo (2048 tramos) calculado al cargar, no al dibujar.
  - Estado del motor para la interfaz (`engine/src/status.rs`): `Controller::status()` da marcha, tempo, compás, último paso, último golpe y pico por pista. El hilo de audio solo escribe atómicos sueltos; los picos se acumulan con `fetch_max` y se vacían al leerlos.
  - Sin `--allow-audio`, el motor avanza con `open_null_output` (un hilo que llama a `process` a ritmo real y tira el audio): se ve todo sin abrir la tarjeta. Con `--allow-audio` usa la tarjeta (no lo ha ejecutado nunca Claude Code).
  - Teclas: Ctrl-S guardar y cargar, Ctrl-L cargar sin guardar, Ctrl-T marcha/paro, Ctrl-G ir a la línea del error, Ctrl-O siguiente pista, Ctrl-Q salir (dos veces si hay cambios). Elegidas entre las Ctrl+letra que no usa `ratatui-textarea`. Si el archivo no existe, se abre una plantilla.
  - Errores de Lua en el editor: borde rojo con `✗ error en la línea N · Ctrl-G` y el mensaje completo al pie, ajustado al ancho. Sigue sonando lo anterior.
  - Tests con el `TestBackend` de ratatui en `tui/src/tests.rs` (paneles con `ViewModel` hechos a mano y la aplicación entera con el motor movido por render offline). `cargo test -p tui pantalla -- --ignored --nocapture` imprime una pantalla para verla.
  - Con 40 líneas de alto, el tracker no muestra las 16 filas a la vez (se desplaza siguiendo la que suena).
  - Pendiente: recargar si el `.lua` cambia fuera del editor, resaltar la línea del error dentro del texto y asignar cortes a teclas.
- **Audio de prueba repetible**: `scripts/render-demo.sh [entrada.wav]` renderiza `renders/demo.wav` sin usar la salida de audio. Sin argumento genera antes `renders/synth_break.wav` con `cargo run -p engine --example synth_break` (break sintético determinista: 8 golpes en corcheas a 140 BPM, 44,1 kHz). Variables: `PATTERN`, `SLICES`, `BPM`, `BARS`, `MIN_GAP` (ms), `DETECTOR` (`flux`|`energy`). `renders/` está en `.gitignore`. Para escucharlo: `scp <laptop>:~/sampler/renders/demo.wav .`
- Pendiente conocido: cpal avisa de un underrun al arrancar el stream (una vez, ~200 ms). PipeWire estaba inactivo y cpal iba directo a ALSA.

## Entorno de desarrollo

- Máquina de desarrollo: laptop vieja con Gentoo, x86_64, i7, 12 GB de RAM, GTX 950 (no se usa: todo es terminal, sin drivers propietarios).
- Se trabaja sobre todo por SSH a través de Tailscale. Claude Code corre dentro de una sesión de `tmux` para sobrevivir a cortes de conexión.
- La laptop no debe suspenderse al cerrar la tapa (`HandleLidSwitch=ignore`).
- El audio sale por los altavoces de la laptop, no por la sesión SSH. Para escuchar de forma remota: render offline a WAV y descarga con `scp`. Stream por red solo para escuchar, no para tocar.
- PipeWire como servicio de usuario, usuario en el grupo `audio`, `linger` activado para que el motor abra audio sin sesión gráfica.
- Compilación: usar `mold` y/o `sccache`, compilación incremental.

## Portabilidad (también se usará en Asahi Linux, ARM64)

- Código portable: nada de intrínsecos ni asm específicos de x86. Si se usa SIMD, con crates portables o detección en tiempo de ejecución.
- No fijar `target-cpu=native` en `.cargo/config.toml` ni en el repo (rompe al compilar en otra arquitectura).
- Preferir crates en Rust puro (`symphonia`, `rubato`, `fundsp`); `mlua` con la feature vendored de Lua para no depender del sistema.
- Dependencias de C/C++ (p. ej. Rubber Band) detrás de una feature opcional.
- No asumir un tamaño de página de 4 KiB (Asahi usa 16 KiB).
- Probar el build en ambas arquitecturas de vez en cuando.

## Notas

- **Salida de audio real: nunca usarla desde Claude Code.** `tui` sin `--render` se niega a reproducir por la tarjeta salvo con `--allow-audio`. Claude Code no debe pasar nunca `--allow-audio` ni abrir el dispositivo de audio por otra vía: se verifica solo con tests y render offline. `--allow-audio` es solo para que el usuario lo use a mano.

- REA (morluto/rea) no se usa en este proyecto: es para ingeniería inversa, no para escribir un sampler.
- Claude Code no puede oír el resultado: puede renderizar offline y analizar picos, longitud y alineación de cortes; el criterio de "suena bien" es del usuario.
- Respeta los derechos de los samples usados en tracks publicados.
