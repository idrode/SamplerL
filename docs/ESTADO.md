# Estado

Lo hecho, con detalle: dónde está en el código, cómo funciona, sus tests y las
decisiones de diseño de cada fase. Las reglas del proyecto están en
[CLAUDE.md](../CLAUDE.md), el plan (lo que falta y el orden) en
[ROADMAP.md](ROADMAP.md) y el uso en [GUIA.md](GUIA.md) (referencia) y [PRIMERA-CANCION.md](PRIMERA-CANCION.md) (tutorial).

Al terminar algo, se apunta aquí (detalle y decisiones) y se marca en el
ROADMAP.

## Pasos 1-7 y piezas comunes

- **Paso 1 hecho.** Workspace con `engine` (librería) y `tui` (binario mínimo: render offline; reproducir por la tarjeta exige `--allow-audio`, ver las Notas de `CLAUDE.md`; aún sin ratatui).
- Decisiones técnicas:
  - Samples en memoria como estéreo f32 entrelazado (mono se duplica, >2 canales se usan los dos primeros), resampleados al cargar con `rubato::Fft`. La longitud de salida se calcula con enteros porque rubato 5 puede dar un frame de más.
  - Carga de WAV con `hound`; FLAC, MP3, OGG Vorbis y AIFF con `symphonia` (ver M4 en «Arreglos de la segunda auditoría»).
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
  - La voz lee en posición fraccionaria (f64), interpolada (sinc desde la fase K; antes lineal): el pitch cambia la velocidad y la duración, sin time-stretch. Con pitch 0 la salida es idéntica bit a bit a la de antes. El reverse lee de `end-1` a `start`. Los fundidos van en tiempo de salida, así que son simétricos: el reverse es exactamente la inversión temporal del forward.
  - Dentro de una pista no hay polifonía: cada paso corta al anterior con fundido cruzado, como en un tracker.
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
  - Si algo falla, no se envía nada y sigue sonando lo anterior. Esto incluye sintaxis, errores de ejecución, notación, corte inexistente, archivo que no existe, pista no definida, BPM fuera de rango, más de 8 pistas, un script que tarda más de 2 s o usa más de 256 MB (`set_time_limit` y `set_memory_limit`, `[script]` en la configuración; ver «Límites del script») o un fallo del programa (un pánico: «error interno», ver N5 en «Arreglos de la segunda auditoría»).
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
  - Paneles: editor a la izquierda; a la derecha, estado del motor (compacto), sample (forma de onda en barras: una por columna, del centro al pico de su trozo, con bloques de octavos de celda en los bordes y una raya en el silencio, `ui::wave_bar`; antes era braille; con los cortes numerados y marcados de arriba abajo, y el que suena resaltado) y tracker vertical (16 filas, una columna por pista, celdas en mini-notación, fila actual resaltada y desplazamiento si no cabe); abajo, efectos por pista (volumen, nivel y cadena de efectos).
  - Cada panel es un `Widget` que solo lee el `ViewModel` (`tui/src/model.rs`). `App::view()` lo construye con el `Song` del script, la instantánea del motor y el estado del editor. La forma de onda va como resumen mínimo/máximo (2048 tramos) calculado al cargar, no al dibujar.
  - Estado del motor para la interfaz (`engine/src/status.rs`): `Controller::status()` da marcha, tempo, compás, último paso, último golpe y pico por pista. El hilo de audio solo escribe atómicos sueltos; los picos se acumulan con `fetch_max` y se vacían al leerlos.
  - Sin `--allow-audio`, el motor avanza con `open_null_output` (un hilo que llama a `process` a ritmo real y tira el audio): se ve todo sin abrir la tarjeta. Con `--allow-audio` usa la tarjeta (no lo ha ejecutado nunca Claude Code).
  - Teclas: Ctrl-S guardar y cargar, Ctrl-L cargar sin guardar, Ctrl-T marcha/paro, Ctrl-G ir a la línea del error, Ctrl-O siguiente pista, Esc modo tocar, F2 línea de órdenes, F3 recargar del disco, F4 autocompletado, Ctrl-Q salir (dos veces si hay cambios). Las Ctrl+letra son las que no usa `ratatui-textarea` (Ctrl-Z, la última libre, es deshacer); las acciones nuevas van en la paleta de comandos (F1), no en atajos. Si el archivo no existe, se abre una plantilla (desde «Mejoras sueltas», solo con `--new`).
  - Errores de Lua en el editor: borde rojo con `✗ error en la línea N · Ctrl-G`, el mensaje completo al pie, ajustado al ancho, y la línea del error con fondo rojo oscuro (color 52 de la paleta de 256) si está a la vista (la fila sale de `TextArea::scroll_offset`). Sigue sonando lo anterior.
  - Modo tocar (Esc): 1-9 y 0 disparan los cortes 1-10 de la pista seleccionada y q-i los cortes 11-18 (`app::PADS`); el texto no se modifica. El disparo es inmediato (tocar no es un cambio de código, no se cuantiza) y suena en la pista seleccionada con su volumen y sus efectos, cortando su voz como un paso (`Controller::play_slice_on`).
  - Cambios externos del `.lua`: cada 500 ms se compara el disco con lo último que guardó o cargó el editor. Si cambió fuera, se avisa (⚠ en el editor) sin tocar nada. F3 recarga y carga (entra en el compás siguiente); si había texto sin guardar, antes se escribe en `<nombre>.sin-guardar.lua`. Ctrl-S con un cambio externo pendiente pide otro Ctrl-S para sobrescribir.
  - Línea de órdenes (F2; Enter ejecuta, Esc cancela): `exportar [archivo.wav] [compases]`, `guardar [proyecto.sampler]`, `abrir proyecto.sampler` (o `abrir!` para descartar cambios), `muestras [carpeta]`. Las rutas son relativas a la carpeta del proyecto o, si no hay, a la del script.
  - Tests con el `TestBackend` de ratatui en `tui/src/tests.rs` (paneles con `ViewModel` hechos a mano y la aplicación entera con el motor movido por render offline). `cargo test -p tui pantalla -- --ignored --nocapture` imprime una pantalla para verla.
  - Con 40 líneas de alto, el tracker no muestra las 16 filas a la vez (se desplaza siguiendo la que suena).
- **Paso 7 hecho.** Proyectos y exportación:
  - Proyecto `.sampler` (`script/src/project.rs`): texto con `script = …`, `samples = …` (carpeta donde `sample()` busca los WAV; si no hay, la del script) y `bpm = …`. Las rutas se guardan relativas al proyecto. El `bpm` del proyecto es el tempo por defecto: si el script llama a `bpm()`, manda el script (`ScriptHost::set_sample_dir`, `set_default_bpm`). Errores con archivo y línea (clave desconocida, bpm no válido, falta `script`).
  - `tui --edit proyecto.sampler` abre un proyecto; `guardar` guarda el script y el proyecto (sin nombre: el abierto, o `<script>.sampler`); `abrir` carga otro (se niega con cambios sin guardar salvo `abrir!`).
  - `exportar`: `Song::render(frecuencia, compases)` renderiza en un motor aparte (no toca lo que suena) lo último cargado bien, y se guarda en WAV de 16 bits. Por defecto, `<script>-mezcla.wav` y la canción entera: `Song::length_bars(4)` = mcm de los ciclos de las pistas, al menos 4 compases. Es idéntico, muestra a muestra, a lo que suena en el motor en vivo (test).
  - Tests en `script/tests/project.rs` y `tui/src/tests.rs` (órdenes, exportar, proyectos, carpeta de samples, cambios externos, modo tocar, línea de error).
- **Registro de la API de Lua** (`script/src/api.rs`): fuente única de nombres, firmas, resúmenes, parámetros y plantillas de inserción (`${...}` marca cada parámetro) de las funciones y métodos (`FUNCTIONS`, con su `Receiver`: global, sample, cortes, pista o compás), de los efectos de `:fx` con sus parámetros y valores válidos (`EFFECTS`), de los de `:chance` (`CHANCE_EFFECTS`) y de los snippets (`SNIPPETS`: nombre, resumen, plantilla de varias líneas y lo que necesita tener antes, `requires`). Lo usan el autocompletado del editor, la validación de parámetros de `:fx` y los mensajes de error, y la sección 4 de `docs/GUIA.md`: `api::guide_markdown()` la genera entre las marcas `api:inicio`/`api:fin` y `cargo run -p script --example guia` la reescribe; un test falla si la guía no coincide. `script/tests/registry.rs` comprueba que cada función registrada existe en Lua (también los métodos del compás de `every`), que no hay funciones globales sin registrar y que plantillas, efectos, parámetros, valores y snippets (tras su `requires`) se aceptan. La guía muestra cada snippet con su `requires` como bloque de Lua, así que `guide.rs` también los ejecuta.
- **Autocompletado en el editor** (`tui/src/complete.rs` la lógica, sin terminal; `App` el estado y las teclas; `ui::CompletionPopup` el dibujo):
  - `complete::context(líneas, fila, columna, wavs)` recorre el texto hasta el cursor (cadenas, comentarios, llamadas y tablas abiertas) y da el prefijo a sustituir y las opciones del contexto; `Context::matches` filtra por prefijo sin distinguir mayúsculas. Tras `:` deduce el tipo del receptor (`sample(...)`, `track(...)`, lo que devuelve cada método según el registro, variables `local` y el parámetro de la función de `every`, también en cadenas que siguen en otra línea); si no lo sabe, ofrece todos los métodos. Dentro de `fx("...")` los efectos, en su tabla los parámetros y en `type = "..."` sus valores; en `sample("...")` los WAV de la carpeta de muestras (se lee solo en ese contexto); en `track("...")` las pistas del texto; en `chance(p, "...")` sus efectos; al inicio de línea globales, variables locales y palabras clave. Dentro de `steps("...")`, otras cadenas, comentarios, números o tras `.`: nada.
  - Se abre solo al escribir una letra, `:` o comillas si hay opciones; con prefijo vacío solo justo tras `:` o comillas. Se cierra si no queda ninguna, o si la única ya está escrita entera y no tiene plantilla (`end`; `fx` sigue abierto para insertar su plantilla). F4 lo abre a mano (sin prefijo). Los atajos y el cambio a modo tocar lo cierran.
  - Con el popup abierto: ↑/↓ eligen, Tab acepta, Esc solo cierra (no entra en modo tocar). Enter acepta solo si se ha movido la selección con las flechas (`Completion::navigated`, que vuelve a `false` cuando la lista se recalcula al escribir); si no, cierra el popup y llega al editor como salto de línea. En modo tocar nunca se abre.
  - Aceptar sustituye el prefijo por la plantilla del registro (`editing::insert_template`, también para los snippets, que pueden tener varias líneas: las nuevas llevan la sangría de la actual), selecciona el primer campo (escribir lo sustituye) y guarda los demás en `editing::Fields` (texto entre campos, que puede tener saltos de línea, + texto del campo, y uno vacío al final). Tab salta al siguiente: justo tras el cursor si coincide el texto intermedio, o la primera aparición más adelante en el texto; tras el último, el cursor queda al final de la plantilla. Aceptar se deshace de una vez (`App::grouped`). Aceptar un efecto (opción de tipo «efecto») regenera la tabla de parámetros que lo sigue en la línea (`editing::replace_fx_table`: borra lo que quede del nombre anterior tras el cursor, sustituye `{ ... }` por `api::Effect::table_with(tabla vieja)`: los parámetros de los dos efectos conservan su valor escrito (leído con `api::table_items`), los de la plantilla del nuevo que falten salen por defecto, en el orden de sus parámetros, marca su primer parámetro y deja el final tras el `)` de `fx(...)`), todo en un paso de deshacer; si no hay tabla detrás, solo cambia el nombre y se conservan los campos pendientes. Un efecto escrito entero no cierra el popup, para poder aceptarlo y regenerar. Otros nombres sin plantilla conservan los campos pendientes. Con un parámetro marcado, las flechas cancelan la selección en vez de extenderla.
  - El popup va debajo del cursor (alineado con el prefijo) o encima si no cabe, dentro de la pantalla y dibujado lo último, encima de los paneles. Muestra hasta 8 opciones con su tipo, y la firma y la descripción (con los parámetros) de la elegida. `ViewModel::completion` lo describe.
  - Tests: unitarios de contexto en `complete.rs` y con `TestBackend` en `tui/src/tests.rs` (filtrado, contexto, flechas, aceptar con plantilla y marcas, nombre dentro de plantilla, Esc sin modo tocar, F4, posición del popup). `cargo test -p tui pantalla_autocompletado -- --ignored --nocapture` imprime la pantalla con el popup.

## Fase A: editor ✅

El autocompletado contextual está en «Pasos 1-7 y piezas comunes». `cargo test -p tui pantalla_paleta -- --ignored --nocapture` imprime la paleta abierta con un aviso de `steps`.

- Paleta de comandos (`tui/src/palette.rs`, F1 en los dos modos): `ACTIONS` registra cada acción (nombre, descripción, atajo); `filter` busca cada palabra como trozo del nombre sin mayúsculas ni tildes; la última usada sale la primera (repetir = F1 Enter). Con la paleta abierta las teclas solo filtran (no disparan pads), ↑/↓ eligen, Enter ejecuta (`App::run_item`/`run_action`), Esc cierra sin cambiar de modo. La tabla de la sección «Paleta de comandos» de `docs/GUIA.md` se genera desde `ACTIONS` (marcas `paleta:inicio`/`paleta:fin`); un test la comprueba y `ACTUALIZAR_GUIA=1 cargo test -p tui the_guide_lists_the_palette` la reescribe.
- Deshacer y rehacer: Ctrl-Z o Ctrl-U, Ctrl-R y la paleta, todos por `App::undo`/`redo`. Las acciones compuestas (operaciones de línea, aceptar una plantilla) se apuntan con `App::grouped` como texto antes/después y se deshacen o rehacen enteras si el texto coincide con su resultado; si no, un paso del historial del editor.
- Snippets `amen`, `fill` (con `:fill`), `ghost`, `intro` y, de la fase C, `euclid`, `melodia` (escala con `:melody`) y `groove` (swing + humanize) (del registro `api::SNIPPETS`): desde el autocompletado al inicio de línea o desde la paleta («Snippet: …», `App::insert_snippet`); Tab salta entre campos como en las plantillas. Tests en `tui/src/tests.rs` (rellenar `ghost` campo a campo y cargarlo, `amen` como canción entera, sangría y deshacer desde la paleta).
- Diagnóstico en vivo de `steps("...")` (`tui/src/diagnose.rs`): `App::view` revisa el texto del editor en cada dibujo (sin cargar) y da `ViewModel::diagnostics`. Mira cada `:steps(` fuera de cadenas y comentarios (también `c:steps` de `every`; una cadena sin cerrar, hasta el final de la línea): primero cada paso con `notation::parse_step` (así el error de un paso se ve aunque falten pasos), luego el número de pasos (`parse_steps`) y luego los rangos de ganancia, pitch y retrigger. Los cortes inexistentes no (dependen de la pista; los da la carga). El paso erróneo se pinta en rojo claro subrayado (`ui::DIAGNOSTIC`; la línea del cursor ya va subrayada entera, así que lo que lo distingue es el color); con un patrón vacío o con pasos de más o de menos, todo el patrón. El primer aviso (`⚠ línea N: …`, con `+k más`) va al pie del editor, ajustado al ancho.
- Ayuda contextual (`complete::help_at`, en la última línea del editor en el modo editor, `ViewModel::help`): con el mismo recorrido que el autocompletado. Sobre una palabra (contando hasta su final) que es una función global o un método (con el tipo del receptor deducido), su firma y resumen; si no, la llamada de la API más interior que contiene el cursor (`Frame::Paren` guarda el `:` de los métodos) con el parámetro del argumento actual, más el efecto elegido en `fx("...")`/`chance(p, "...")`; dentro de la tabla de `fx`, el efecto y sus parámetros, o el rango y valor por defecto de la clave tras `=`. Todo sale del registro de la API.
- Pegar desde el terminal (`App::paste`, `tui/src/main.rs`): al entrar se pide bracketed paste (`EnableBracketedPaste`) y al salir se quita; lo hace la guarda `TerminalModes` al soltarse, así que también con error o pánico (junto con las banderas del teclado de kitty). `Event::Paste` llega a `App::paste`: normaliza `\r\n` y `\r` a `\n` (`editing::normalize_newlines`), cierra el autocompletado sin aceptar, olvida los campos de plantilla y hace un solo `insert_str` dentro de `App::grouped` (un Ctrl-Z lo deshace entero). No pasa por `handle`, así que no hay atajos, autosangría ni snippets. En la línea de órdenes entra la primera línea; en el modo tocar o con un panel encima, nada. Sin bracketed paste el terminal manda lo pegado como teclas: el bucle lee de una vez todo lo que ya ha llegado (hasta `BURST_MAX` eventos) y `App::handle_keys` lo toma como pegado si es una ráfaga de dos o más teclas de texto con algún Enter (`\r`), Ctrl-J (`\n`) o Tab, en el modo editor (`editing::burst_text`); si no, las atiende una a una (los pads del modo tocar no cambian). Tests en `tui/src/tests.rs` (tres líneas con sangría y un comentario `--`, con el autocompletado abierto tras moverse, ráfaga con `\r` y Ctrl-J, ráfaga en el modo tocar).
- Comentar/descomentar, duplicar, subir y bajar líneas (`tui/src/editing.rs`, desde la paleta): actúan sobre la línea del cursor o las de la selección (una selección que acaba en la columna 0 no incluye esa línea). Comentar pone `-- ` a la sangría mínima; si todas las líneas con texto ya lo están, lo quita. Se hacen con `insert_str`/`delete_str` del editor para que el historial las recoja.

## Fase B: etiquetas de corte ✅

- Notación (`engine::notation`): `parse_steps_labeled(texto, etiquetas)` / `parse_step_labeled` aceptan un nombre en lugar del número, con los mismos sufijos (`kick^7@0.5`, `snare'r2`); `parse_steps`/`parse_step` son lo mismo sin etiquetas. `split_label` separa la palabra inicial (letras, números, `_`) y, si acaba en `r` + dígitos, eso es el retrigger (`kickr4` = `kick` + `r4`; `xr4` sigue siendo `1r4`). `check_label` exige letras ASCII, números y `_`, empezar por letra o `_`, y prohíbe números, `x`, `r` + dígitos y finales en `r` + dígitos. Etiqueta desconocida: `paso 3 ("snre"): la etiqueta "snre" no existe (hay: kick, snare)`.
- Lua: `cortes:name(i, "kick")` (encadenable; `nil` quita). Las etiquetas viven en un `Rc<RefCell<Vec<Option<String>>>>` compartido por la variable y sus pistas (un `name` posterior a `track()` vale para los `steps` siguientes); `slice_at` hace una copia con la etiqueta de su corte. `TrackDef` lleva `labels` (una por corte) y `origin: SliceOrigin` (sample como se escribió, `n` de `slice(n)` y qué corte del original es cada uno), que usa la interfaz para etiquetar. Errores con línea: nombre no válido, corte inexistente, etiqueta repetida en otro corte.
- Proyecto: líneas `etiqueta = <sample> <n> <corte> <nombre>` (`SliceLabel`; el sample puede tener espacios, se lee de derecha a izquierda). Se validan al cargar (nombre, corte dentro de `n`, sin repetir corte ni nombre en el mismo sample y `n`). `ScriptHost::set_labels` las aplica al hacer `sample(nombre):slice(n)` con el mismo nombre y `n`; `:name()` manda. Si ese sample tiene etiquetas en el proyecto pero ninguna para esa `n`, no se aplican y se avisa en `Song::warnings` (agrupadas por su `n`, sin repetir: `la etiqueta «kick» de amen.wav es de slice(8) y este script usa slice(6): no se aplica`); la interfaz los añade al mensaje de «cargado» y `--script` los imprime por stderr. Si además se usan en `steps`, el error de etiqueta desconocida acaba con `; la etiqueta «kick» del proyecto es de slice(8) y este script usa slice(6)` (las pistas y los compases de `every` guardan esas etiquetas en `stale`, también con `slice_at`). `ScriptHost::set_label` pone, cambia o quita una con las mismas comprobaciones.
- Interfaz: `TrackView::labels`; el tracker escribe la etiqueta en lugar del número (`model::step_text(paso, etiquetas)`, recortado al ancho de columna) y la vista del sample `N nombre` en cada marca (la siguiente marca puede taparlo). `App::known_labels` junta las de lo último cargado, las del proyecto y las de `:name(i, "…")` en el texto (`complete::text_labels`); las usan el autocompletado dentro de `steps("...")` (para el paso que se escribe, si empieza por letra; prefijo = ese paso) y el diagnóstico en vivo (`diagnose::steps_diagnostics(líneas, etiquetas)`, con `parse_step(s)_labeled`). Al ser de cualquier pista, una etiqueta de otra pista no da aviso en vivo; lo da la carga.
- Etiquetar desde la interfaz: `App::selected_slice` (el último corte tocado con los pads o el elegido con «Corte siguiente/anterior» de la paleta; vuelve a 0 al cambiar de pista; `ViewModel::selected_slice`, resaltado con fondo amarillo y en el título de la vista del sample). «Etiquetar el corte seleccionado» abre la línea de órdenes con `etiquetar N `; la orden `etiquetar corte [nombre]` traduce el corte de la pista a su corte del `slice(n)` original (`SliceOrigin::indices`, así que en una pista de `slice_at` vale para el sample entero), llama a `ScriptHost::set_label`, guarda el proyecto abierto conservando su `bpm` (`App::project_with`) y recarga. Avisa si el script nombra ese corte con `:name()` (manda el script) o si la recarga falla. Tests en `tui/src/tests.rs` (etiquetar el corte tocado, proyecto guardado y reabierto, `slice_at`, errores, selección y prioridad del script).
- Tests en `script/tests/labels.rs`: `"kick . snare ."` renderiza idéntico, muestra a muestra, a `"1 . 2 ."` (también con sufijos, `every`, `chance` con semilla, `slice_at`, etiquetas del proyecto y en el motor en vivo), errores, proyecto guardado/cargado. Notación en `engine/tests/notation.rs`.

## Fase C: generadores y transformaciones ✅

Funciones de patrones en `script/src/patterns.rs` (puras, sobre texto: cada paso es una palabra que valida luego `steps`; de 1 a 64 pasos, que `steps` cuenta con la longitud de la pista, y devuelven el patrón agrupado por tiempos con `patterns::format`), registradas como globales de Lua (valen dentro de `every`) y en el registro de la API. Tests de render en `script/tests/patterns.rs` (cada función suena idéntica al patrón que escribe; `value()` saca el texto devuelto con un `error()`).

- Azar (`patterns::Rng`, el xorshift de `chance`): con semilla propia (`mutate(…, semilla)`, `shuffle(…, semilla)`), un generador nuevo con ella: mismo resultado siempre. Sin ella, un generador por carga sembrado con la semilla de la carga (la de `ScriptHost::set_seed` o la hora, como `chance`) mezclada con `PATTERN_SEED`, aparte del de `chance` para no cambiar sus resultados; dentro de `every` cada compás saca números nuevos y el ciclo es determinista con la semilla.
- `mutate(patrón, p [, semilla])`: cada paso, con probabilidad `p`, se cambia por uno de los 16 del propio patrón al azar (conserva su vocabulario).
- `shuffle(patrón [, semilla])`: permutación de los 16 pasos (Fisher-Yates con el `Rng`).
- Escalas: `scale(nombre [, tónica])` devuelve un objeto escala (receptor `Receiver::Scale`, «escala») con `:pitch(grado)` (semitonos) y `:melody(grados [, golpe])` (patrón de grados → pasos con `^`: `"1 3 5 3"` → `"1^0 1^3 1^7 1^3"`; cada grado admite los demás sufijos, no `^`; golpe por defecto `"1"`, sin `^`). Grados desde 1 hacia arriba por octavas y negativos por debajo (-1 = último grado una octava abajo); el 0 no existe; más de ±36 semitonos da error con el paso. La tónica (de C a B con `#`/`b`, letra en minúscula vale) suma sus semitonos sobre do: el tono original del corte hace de do. Las escalas (`api::SCALES`, con sus intervalos) y las tónicas (`api::NOTES`) están en el registro; el autocompletado las ofrece dentro de `scale("…", "…")` y la guía genera la tabla de escalas. Lógica en `patterns::scale_pitch`/`melody`.
- Microtiming en el motor: `Step::delay` (fracción de paso, de 0 a `MAX_DELAY` = 0,9; validado en `Pattern::from_steps`) llega al `Trigger`; al disparar el paso, el motor lo pasa a frames con la duración real del paso (`round(retraso · frames_del_paso)`). Un golpe retrasado se programa como un `Retrig` con `done = 0` que empieza en `paso + retraso`: él y sus subdisparos caen al frame exacto con el mismo mecanismo que el retrigger, sin reservas. Un paso vacío ya no corta los subdisparos pendientes (los de un golpe retrasado pueden pasar al paso siguiente); un paso con golpe sí, como siempre. Al entrar un patrón nuevo se descartan los pendientes (podrían ser de otro sample). Tests en `engine/tests/microtiming.rs` (frames exactos a 120 y 170 BPM, retrigger retrasado) y `engine/tests/no_alloc.rs` (asignador global que cuenta las reservas del hilo mientras corre `process`: cero con retrasos, retrigger, efectos, cambio de patrón, paro y arranque; y un control de que el contador las ve).
- `pista:swing(porcentaje)`: estilo MPC, de 50 (recto) a 75; retrasa los contratiempos de semicorchea (pasos impares desde 0) `2·p/100 − 1` pasos (66 → 0,32 pasos = 1920 frames a 120 BPM). Se aplica al expandir, después de `every`/`chance`, sumado al retraso que haya (tope `MAX_DELAY`). `TrackDef::swing`; el panel de efectos lo muestra junto al nombre de la pista.
- `pista:humanize(ms [, volumen [, semilla]])`: a cada golpe de cada compás, un retraso al azar de 0 a `ms` (0-100, pasado a pasos con el tempo de la canción o `DEFAULT_BPM`; sumado al swing, tope `MAX_DELAY`; nunca adelanta) y su volumen por un factor de `1 − volumen` a 1 (nunca sube). Fuerza un ciclo de 16 compases como `chance`. Semilla propia o, sin ella, la de la carga mezclada con `HUMANIZE_SEED` y un FNV del nombre de la pista (cada pista, su azar). `TrackDef::humanize`. El test comprueba que cada golpe suena en `paso + round(retraso · 6000)` exacto, leyendo los retrasos del modelo.
- `Rng::new` mezcla la semilla con splitmix64 (antes `semilla ^ c | 1` hacía que semillas vecinas como 42 y 43 dieran el mismo azar).
- `pista:fill(efecto, n [, pasos [, valor]])`: `Transform::Fill`, en orden con `every`/`chance` y con su `n` en el ciclo. En los compases con `(b + 1) % n == 0` (el último de cada grupo de `n`), los últimos `pasos` (4 por defecto) sufren el efecto (`apply_fill`): `stutter` (valor = retrigger, 4) y `roll` (retrigger 2, 4, 8, 16) los rellenan con el golpe del principio del relleno o el último antes (o el primero del compás); `reverse` invierte el sentido; `pitch` suma semitonos (12 por defecto, con tope ±36); `mute` los quita. Efectos en `api::FILL_EFFECTS` (autocompletado dentro de `fill("…")`, tabla en la guía).
- `reverse_pattern(patrón)`: invierte el orden de los pasos (no el sonido de cada golpe, que es el sufijo `'`).
- `rotate(patrón, n)`: con `n` positivo cada paso suena `n` pasos más tarde (`resultado[i] = patrón[(i - n) mod 16]`); negativo, antes.
- `euclid(k, n [, golpe])`: el paso `i` suena si `i·k mod n < k` (empieza por golpe), repetido hasta 16 y cortado si `n` no divide a 16; `golpe` por defecto `x`, cualquier paso que suene.

## Fase D: mezcla y cortes ✅

En el orden en que se hizo, de abajo arriba:

- Ajuste manual de cortes: `Slices::with_start(i, frame, margen)` mueve el inicio del corte `i` (el anterior acaba ahí) si queda a más de `margen` de sus vecinos, o devuelve los límites. Posiciones absolutas en ms: proyecto `corte = <sample> <n> <corte> <ms>` (`SlicePosition`, validadas al cargar el proyecto) y `ScriptHost::set_positions/set_position`, aplicadas al hacer `sample(nombre):slice(n)` después del detector (mandan sobre él); si no caben, aviso en `Song::warnings` y no se aplican. Lua `cortes:move(i, ms)` (cambia la variable, con error de línea si no cabe). Margen de 1 ms. `TrackDef::adjusted` (también con `slice_at`). Interfaz: `,`/`.` (±5 ms) y `<`/`>` (±1 ms) en el modo tocar y tres acciones de paleta (`App::adjust_slice`): pone la posición, recarga, y si la carga avisa de que no cabe la deja como estaba («no se mueve: …»); guarda el proyecto (`App::save_project_file`, también usado por etiquetar) y preescucha. La vista del sample marca `N*` y da la posición en ms del seleccionado. Tests en `engine/tests/slicing.rs`, `script/tests/mixer.rs` (frames exactos, proyecto = `move`, avisos, errores, proyecto guardado) y `tui/src/tests.rs` (teclas, tope, reabrir, quitar).
- Preescucha: voz aparte en el motor (`Engine::preview` y `preview_fading`, `Command::Preview/StopPreview`, `Controller::preview_slice/stop_preview`), sumada al master sin volumen, efectos, mute ni solo de ninguna pista, también con el secuenciador parado; una nueva hace fundido cruzado con la anterior y sus samples se sueltan con `discard` (que ahora también mira la preescucha en `holds`). En la interfaz: ←/→ en el modo tocar recorren los cortes de la pista seleccionada y los preescuchan (`App::browse_slice`), espacio repite, y «Preescuchar el corte seleccionado» en la paleta. Tests en `engine/tests/preview.rs` (igual que el corte en una pista; con una pista sonando, la suma exacta; con la pista en mute, solo la preescucha; fundidos), `no_alloc.rs` y `tui/src/tests.rs`.
- Detección de BPM (`engine/src/bpm.rs`, `detect_bpm(&Sample) -> Option<BpmGuess>`): supone un loop de compases enteros de 4/4. Para 1, 2, 4 y 8 compases, tempo = 240 · compases / duración, dentro de 60-200; puntuación = 1 − 2 · error medio de los transitorios (`Slices::detect(…, 32)`) respecto a la rejilla de semicorcheas desde el inicio. Gana la mejor; con empate (diferencia < 0,05), la más cercana a 130 BPM, y la otra va en `alternative`. `BpmGuess::rounded` (entero si está a menos de 0,05). `tui <wav> --detect-bpm`, `sample:detect_bpm()` en Lua (tempo y compases, o nil), «Detectar el BPM del sample» en la paleta (deja escrita la orden `bpm N`, que cambia o añade la línea `bpm()` del script sin guardar, en un paso de deshacer). Tests en `engine/tests/bpm.rs` (break sintético a 140, uno de 2 compases a 172 en semicorcheas y uno en corcheas a 96 con alternativa 192), `script/tests/mixer.rs` y `tui/src/tests.rs`.
- Metrónomo (`engine/src/metronome.rs`): en `Engine::step`, en los pasos 0, 4, 8 y 12 empieza un clic (seno de 30 ms con caída exponencial; acento en el paso 0: 1600 Hz a 0,5, si no 1000 Hz a 0,3) al frame exacto del paso; se suma al master antes del limitador, sin reservas. `Controller::set_metronome` (inmediato). En Lua `metronome([sí])` (`Song::metronome`, la interfaz lo aplica al cargar); paleta «Metrónomo»; `♩ clic` en el panel del motor. Nunca entra en un render salvo que se pida: `Song::render` sin clic, `Song::render_with_metronome(…, true)`, `exportar … metronomo` (o `metrónomo`/`clic`, en cualquier posición) y `--metronome` en `--render`. Tests en `engine/tests/metronome.rs` (frames, frecuencia y nivel de cada clic), `script/tests/mixer.rs`, `tui/src/tests.rs` y `no_alloc.rs`.
- Medidores: el motor mide el master antes del limitador (`SharedStatus::add_master`: pico con `fetch_max` y frames por encima de `LIMIT_THRESHOLD` con `fetch_add`, atómicos) y `Status::master` (`MasterStatus { peak, limited_frames }`, se vacía al leer). La interfaz (`App::tick`) lleva el medidor del master con caída (puede pasar de 1) y mantiene 1 s (`WARNING_HOLD`) los avisos de limitador, saturación del master (pico > 1) y de cada pista (su pico > 1). `ViewModel::master` (`MasterView`, con `db()`): línea `master` en el panel del motor con `LIMITADOR`/`SATURA`; `SAT` en el panel de efectos. «Reiniciar medidores» en la paleta. Tests en `engine/tests/status.rs` y `tui/src/tests.rs` (suma de 1,0: limitador sin saturar; volumen 4: satura; una pista: -6 dB).
- Mute y solo. En el motor, por pista: `Pattern::with_mute/with_solo` (del script, `pista:mute([sí])`/`:solo([sí])`, entran al compás) y `Controller::set_mute/set_solo` (en vivo, inmediatos, `Command::SetMute/SetSolo`). Se oye si no hay mute (de ninguno de los dos) y, si alguna pista está en solo, está en solo; un mute gana. La ganancia `Track::audible` se suaviza como el volumen (~5 ms, sin clics) y las voces siguen avanzando; si la voz empieza desde el silencio (`Track::fresh`), entra de golpe (si no, el primer golpe de una pista que no debe sonar se colaba). En la interfaz, `App::muted/soloed` por nombre (sobreviven a recargas), `App::sync_mix` envía solo los cambios por pista del motor (`sent_mix`; la cola es de 64). Paleta («Mute de la pista», «Solo de la pista», «Quitar mute y solo») y `m`/`s` en el modo tocar. `TrackView::mute/solo` (script o en vivo) y `ViewModel::audible(i)`: el tracker pone `M`/`S` tras el nombre y atenúa las columnas que no se oyen; el panel de efectos, `MUTE`/`SOLO`/«(solo de otra)». Tests en `engine/tests/mute_solo.rs`, `no_alloc.rs` (con mute/solo) y `tui/src/tests.rs`.

## Fase E: estructura y versiones ✅

Hechos la grabación de lo tocado, las secciones con línea de tiempo, el historial de versiones y el A/B de variantes.

- **Secciones y línea de tiempo** (`engine/src/arrangement.rs`): `Arrangement { sections: Vec<Section { id, patterns: [Option<Pattern>; 8] }>, timeline: Vec<(sección, compases)> }` (`add_section`, `push`; hasta `MAX_SECTIONS` = 64 secciones y `MAX_ENTRIES` = 256 entradas; `SendError::InvalidArrangement`) y `Batch::set_arrangement(Some | None)` (`Command::SetArrangement`, con su lote: `staged_arrangement` → `pending_arrangement`). El motor guarda el que suena en `Playlist { arrangement, place: Place { entry, bar }, current, origin_step, origin_frame, looping, jump, restart }`. Al empezar cada compás (`Engine::advance_playlist`, antes de las pistas): si hay arreglo pendiente, entra entero: dónde seguir es `arrangement::resume` (lo que el anterior iba a tocar en este compás, en el nuevo: la misma entrada si es la misma sección por su `id`, si no la primera entrada de esa sección, con el mismo compás; si no cabe, la entrada siguiente; si la sección no está, el principio y `SharedStatus::set_lost`), los patrones de su sección pasan a las pistas (`put_pattern`, lo que antes hacía el cambio de patrón: parámetros suavizados, subdisparos, volumen y envíos si está callada, estado de los efectos) y los que había van a la basura con el arreglo anterior (`Garbage::Arrangement`). Si no, `Playlist::next` (el compás siguiente de la entrada, la entrada siguiente, la misma si se repite, la del salto, o la primera al acabar) y, si cambia la sección, sus patrones se intercambian con los que hay en las pistas (movimientos: sin reservas ni liberaciones). Al empezar una entrada, `origin_step` y `origin_frame` son ese compás: las pistas leen `hit(global − origin_step)` y las curvas de los efectos y de los granos (`Clock`) cuentan desde ahí, así que `every`, `fill`, `chance`, `humanize`, `:length()`, `ramp` y `lfo` cuentan desde el inicio de la sección. Sin arreglo, el origen es 0: lo de siempre bit a bit. `Command::Start` vuelve al principio (`restart`). `holds` mira también los patrones de los arreglos. `Status::section` (`SectionStatus { entry, bar, looping }`, de `set_section` en cada compás), `Status::section_lost` (se vacía al leerlo) y `Controller::section()` (sin vaciar nada). En Lua (`script/src/lib.rs`): `section(nombre, función)` guarda la función (`SectionBuild`); al acabar el script, `run_sections` la llama sobre una copia de las pistas de fuera (`TrackBuild: Clone`; `track()` dentro busca y crea en esa copia, y una pista nueva cuenta en `Builder::names`, como mucho 8 entre todas las secciones); `arrange({ {sección, compases}, ... })` guarda la lista y dónde se llamó (`location`, la mitad de `fail`). `bpm`, `bpm_ramp`, `meter`, `metronome`, `bus`, `section` y `arrange` dentro de una sección dan error con la línea (`GLOBAL_ONLY`). `eval` estira (`apply_stretch`, ahora con la lista de lo estirado compartida entre secciones) y expande cada sección; `Song::sections` (`SectionDef { name, tracks }`) y `Song::timeline`; `Song::tracks` son las de la primera sección de la línea de tiempo (sin `arrange`, las de la primera sección, con aviso si hay más; sin secciones, las de siempre). `song_batch`: con línea de tiempo, un `Arrangement` con el patrón de cada pista de cada sección en su ranura (`track_pattern`) y el id de cada sección (`section_id`, FNV-1a del nombre); sin ella, los patrones sueltos y `set_arrangement(None)`. Las ranuras salen de `Song::track_names` (todas las secciones). `length_bars` con línea de tiempo es su total hasta `MAX_SONG_BARS` (aviso si se pasa); `section_at`, `section_after`, `section_tracks`, `timeline_bars`. `ScriptHost::load` avisa si la sección del compás siguiente ya no está en la versión nueva. `--render` sin `--bars` graba la línea de tiempo entera (y ahora calcula los frames con `Song::frames`: con la rampa de tempo y el compás) y la imprime. La interfaz enseña las pistas de la sección que suena (`App::tracks`) y el tracker cuenta sus vueltas desde el inicio de la sección (`section_global`). Tests en `engine/tests/arrangement.rs` (cada sección en su compás y al frame, vueltas desde el inicio de la sección, sin arreglo bit a bit, recargar conserva el compás, sección que desaparece, saltos y repetición al compás siguiente, arrancar de nuevo, validación), `no_alloc.rs` (cambios de sección, salto, repetir, recargar y quitar el arreglo) y `script/tests/sections.rs` (secciones en su compás, una sola sección igual que sin ella, `every` desde el inicio de la sección, el render offline igual muestra a muestra que lo que suena en vivo, recargar a mitad de canción conserva la posición, volver al principio con aviso, avisos, errores con archivo y línea).
- **Control en vivo de las secciones**: `Controller::jump_to(entrada)` y `loop_section(sí)` (`Command::JumpTo`, `Command::LoopSection`): el salto queda en `Playlist::jump` y entra al empezar el compás siguiente (y la línea de tiempo sigue desde ahí); repetir hace que `Playlist::next` vuelva a la misma entrada al acabarla (y el estado lo publica ya). En la interfaz (`App::step_section`, `jump_to_section`, `toggle_section_loop`, `set_section_loop`): `]` y `[` en el modo tocar, `l` para repetir o volver, las acciones de la paleta «Saltar a la sección siguiente/anterior», «Saltar a una sección» (abre `seccion `), «Repetir la sección» y «Volver a la línea de tiempo» (con nombres en `[teclas]`: `seccion_siguiente`, `seccion_anterior`, `repetir_seccion`) y la orden `seccion NOMBRE` (su primera entrada). Sin `arrange()`, lo dicen. `ViewModel::timeline` (`TimelineView { entries, current, bar, looping }`, de la canción cargada y la posición del estado) y `ui::TimelineBar`: una fila encima del panel del sample (se la quita a él; sin línea de tiempo no existe) con cada entrada `nombre compases` separadas por `│` y la que suena resaltada como `▶ drop 3/8` (`⟳` si se repite); si no caben, empieza con `… ` lo bastante adelante para que se vea la que suena. Tests en `engine/tests/arrangement.rs` y `tui/src/tests/sections.rs` (saltos, repetir y volver desde el modo tocar con la posición del motor, paleta, orden con una sección que no está, sin línea de tiempo, la fila con su resalte y `⟳`, y una línea de tiempo larga en 40 columnas).
- **Historial de versiones** (`tui/src/history.rs`): `History::for_script(raíz, script)` (carpeta `<nombre>-<id>` con el id sacado de la ruta canónica, así dos `song.lua` no se mezclan; raíz `history::root()`: `$XDG_STATE_HOME/sampler/historial` o `~/.local/state/sampler/historial`, que `main` pasa con `App::set_history_root`). `record(texto, ahora)`: si el hash (`history::hash`, FNV-1a de 64 bits) es el de la última versión, nada; si no, escribe `<segundos>-<nanos>-<hash>.lua` (a un `.part` y renombrado: nunca queda a medias) y `prune` borra las más antiguas mientras pasen de `MAX_VERSIONS` = 200 o de `MAX_BYTES` = 20 MB (nunca la que acaba de escribir). `list` (de la más nueva a la más antigua, por el nombre; lo que no tiene ese nombre se ignora) y `read`; todo devuelve `io::Result`. `App::load` guarda una versión al cargar bien; si falla, `⚠ historial: …` en el mensaje. «Versiones» (paleta, `App::open_versions`) lee todas y abre un selector (`VersionPicker`, dibujado con el popup de los bloques: fecha UTC con `history::date`, líneas, `history::ago`, vista previa y si es igual que el editor); escribir filtra por fecha o texto; Enter (`restore_version`) sustituye el texto del editor dentro de `App::grouped` (un Ctrl-Z) sin escribir el archivo. Tests en `tui/src/history.rs` (sin duplicar, volver a un texto anterior sí cuenta, límites por número y por tamaño, disco roto = error, carpeta por script, fechas) y `tui/src/tests/history.rs` (cargas buenas y con error, sin duplicar, lista, filtro, restaurar sin tocar el archivo y Ctrl-Z, disco roto, sin historial).
- **Variantes A/B**: `variant(nombre, "A"|"B", función)` guarda la variante (`VariantBuild`, con dónde se llamó). Al acabar el script, `run_bodies` (antes `run_sections`) construye una versión sin variantes o dos (A y B): en cada una, las variantes de pista llaman a su función con la pista (`TrackHandle`) sobre las pistas de fuera, y cada sección es una copia de ellas que cambian su función y la de su variante (`BuiltVersion { letter, base, sections }`). Un nombre que no es ni sección ni pista de fuera da error con la línea de `variant()`; `variant` va fuera de las secciones (`GLOBAL_ONLY`); letra distinta de A y B o repetida, error. `eval` expande cada versión con el azar desde la misma semilla (lo que no cambia sale igual; sin variantes, exactamente como antes) y devuelve la A con la B en `Song::alternate` (`Song::variant`: `'A'`, `'B'` o `None`); las pistas de las dos, como mucho 8 entre ambas. `ScriptHost` guarda la que suena (`song`) y la otra (`other`), con ranuras para la unión de las dos (alternar no mueve ninguna); al recargar sigue con la letra que sonaba. `set_variant`/`toggle_variant` mandan la otra versión con `song_batch` como un lote: entra entera al empezar el compás siguiente y, con arreglo, el motor sigue por la misma sección y el mismo compás (sus ids no cambian). En la interfaz: `v` en el modo tocar y «Alternar variante A/B» en la paleta (`[teclas] variante`), `App::toggle_variant`, `ViewModel::variant` y `variante A`/`variante B` resaltado en la primera línea del panel del motor; las pistas de la vista siguen a la variante. Tests en `script/tests/variants.rs` (la A bit a bit igual que sin variantes, la B igual que escribir el cambio, alternar entra en el compás exacto, con secciones sigue por su compás y la intro sin variante no cambia, recargar conserva la B, sin variantes, errores con línea) y `tui/src/tests/variants.rs`.

- **Grabar lo tocado.** El motor publica en un atómico, al acabar cada bloque, la posición de la canción en pasos con decimales (`SharedStatus::set_position`, `Controller::position()`; `None` parado; no vacía los picos como `status()`). `tui/src/record.rs` (sin terminal): `Recording::new(pista, semicorchea de inicio, compases, pasos por compás)`, `hit(posición, corte, volumen)` (el paso más cercano: `round(posición − inicio)`; hasta medio paso antes del inicio cuenta como el primero; dos en el mismo paso, el último), `done`/`waiting`/`bar` y `notation(etiquetas)` (etiqueta o número del corte, `@volumen` a dos decimales si es menor que 1, agrupado por tiempos con `patterns::format`). La orden `grabar [N]` (de 1 a 64/pasos del compás; necesita el secuenciador en marcha) empieza al principio del compás siguiente (`Status::global − Status::step + bar_steps`) con la pista seleccionada; «Grabar lo tocado» (paleta) deja escrito `grabar 1` y, si ya se graba, para antes. `play_pad_with_gain` apunta cada golpe de esa pista con `Controller::position()` (la ruta de los pads y de las notas MIDI, con la velocidad como volumen). `App::tick` (`watch_recording`) acaba la grabación al pasar su último paso (o la cancela si se para el secuenciador) y `finish_recording` escribe en el cursor `steps("...")` (o `length(N):steps("...")` si no son 16 pasos) en un paso de deshacer. `● REC drums 1/2` (o `: al compás`) en el panel del motor (`ViewModel::recording`). Tests en `engine/tests/status.rs` (posición), `no_alloc.rs`, `record.rs` y `tui/src/tests/record.rs` (pads y notas MIDI simuladas en sus pasos con su volumen, un golpe antes del compás, dos compases con `length(32)`, parar antes, cancelar al parar, sin marcha).

### Decisiones

- **El motor lleva la lista de reproducción** (y no el anfitrión mandando
  un lote en cada cambio). Se midió lo que tarda el anfitrión en cargar y
  preparar un lote: 1-5 ms el ejemplo de varias pistas, 38 ms la primera
  vez con un `stretch` (más con WAV largos o en debug), con el bucle de la
  interfaz despertando cada 30 ms; un compás dura 1,4 s a 170 BPM. Casi
  siempre llegaría a tiempo, pero un tirón retrasaría una sección un
  compás, y el render offline tendría que imitar ese bucle. Con el arreglo
  en el motor, cada cambio es al frame del compás, en vivo y offline igual
  (el mismo motor con el mismo lote), y el motor sabe dónde empezó cada
  sección. Guardarlo cuesta unos KB por pista y sección (los patrones ya
  resueltos).
- **Todo cuenta desde el inicio de la sección**: las vueltas de los
  patrones (`every`, `fill`, `chance`, `humanize`, `:length()`) y las
  curvas de `ramp` y `lfo` (también en los granos). Así un `fill` cada 4
  cae al final de cada 4 compases del drop aunque la intro dure 3, y una
  sección que se repite suena igual cada vez. El tempo, su rampa y el
  compás son de la canción (`bpm_ramp` cuenta desde que entra, como
  siempre): una sección no puede cambiarlos (error con la línea).
- **Sintaxis**: la propuesta. `section(nombre, función)` trabaja sobre una
  copia de las pistas de fuera, porque casi siempre una sección es «lo
  mismo con algo cambiado» (un mute, un filtro, otro patrón); las pistas
  nuevas de una sección solo suenan en ella. La función se llama al acabar
  el script, así puede usar pistas definidas después. `arrange()` con
  `{nombre, compases}`. Sin `arrange`, la primera sección (una sola
  sección es lo de siempre, bit a bit).
- **Recargar**: el motor sigue, en el arreglo nuevo, lo que iba a tocar el
  compás siguiente en el anterior, por el nombre de la sección (su id): la
  misma entrada si sigue siendo esa sección, si no la primera entrada con
  ella, con el mismo compás y el mismo origen (las vueltas siguen por donde
  iban). Si la sección no está, el principio; el anfitrión lo avisa al
  cargar (prevé el compás siguiente) y el estado lo confirma.
- **Al acabar la línea de tiempo vuelve a empezar** (para tocar en vivo);
  exportar graba la línea de tiempo una vez.
- **Historial**: en el estado del usuario y no junto al proyecto, para no
  ensuciar el repositorio (y para que haya historial aunque no haya
  proyecto). Se guarda al cargar bien, que es cuando el texto «funciona»,
  y solo si cambió respecto a la última versión. La fecha se enseña en
  UTC (sin depender de una base de datos de zonas horarias) junto a
  «hace N min». Restaurar es editar: el texto entra como un cambio que se
  deshace, y el archivo solo cambia al guardar.
- **Variantes en el anfitrión, no en el motor**: al cargar se preparan
  las dos versiones de la canción, y alternar es mandar la otra como un
  lote más (al compás siguiente, entero, sin volver a evaluar Lua). El
  motor no necesita nada nuevo: con secciones, el arreglo de la B tiene
  las mismas secciones y su recolocación por nombre deja la canción en la
  misma sección y compás. La sintaxis es la propuesta, con el nombre de una
  sección o de una pista de fuera (que la función recibe): así sirve con y
  sin secciones. Basta con definir la B; la A es lo escrito. Ambas
  versiones tiran el azar (`chance`, `mutate`...) desde la misma semilla
  para que alternar no cambie lo que no toca la variante.
- **En vivo**: saltar va a una entrada de la línea de tiempo (por nombre,
  la primera de esa sección) y sigue desde ahí; repetir es un estado
  aparte (sobrevive a los saltos y a recargar) que se quita con «volver a
  la línea de tiempo». Las teclas del modo tocar (`[`, `]`, `l`) no
  chocan con los pads ni con el ajuste de cortes.
- **Grabar lo tocado**: la grabación empieza en el compás siguiente y
  cuantiza cada golpe al paso más cercano de la rejilla de la canción
  (con la posición en pasos que publica el motor); graba la pista
  seleccionada al empezar, por la misma ruta que los pads (así también
  las notas MIDI); escribe texto en el cursor, sin cargar nada.

## Fase F: biblioteca de bloques ✅

Hecha antes que la E. `cargo test -p tui pantalla_bloques -- --ignored --nocapture` imprime el selector.

- `script::blocks` (sin interfaz): `Library::open(global, proyecto)` lee los `.lua` de la carpeta global (`blocks::global_dir()`: `$XDG_CONFIG_HOME/sampler/blocks` o `~/.config/sampler/blocks`; se crea al leer si falta) y de `blocks/` del proyecto (`blocks::project_dir(base)`, la del proyecto o la del script; se crea al guardar el primer bloque). Un bloque del proyecto tapa a uno global del mismo nombre. Cabecera `-- nombre:`/`-- descripción:`/`-- etiquetas:` (también sin tilde y en inglés; las líneas `-- clave: valor` del principio con clave conocida, y una línea en blanco tras ellas), el resto es el cuerpo, con campos `${...}`; sin `nombre`, el del archivo. Nombres como identificadores (letras, números, `_`; `check_name`) para poder completarlos. Archivos ilegibles, con nombre no válido, vacíos o repetidos en la misma carpeta van a `Library::warnings` y no impiden los demás. `save` (con `overwrite`; `SaveError::Exists`), `update` (cambia el cuerpo, conserva cabecera y archivo), `delete`, `filter` (cada palabra en nombre, etiquetas o descripción, sin tildes; `#` delante vale). Los bloques nunca se cargan con el script (test). `api::expand_template` deja como texto un `${` sin cerrar (antes hacía panic).
- `App`: `set_global_blocks` (la llama `main` con `blocks::global_dir()`; los tests ponen una carpeta suya, nunca la real) y `reload_blocks` (al abrir y al cambiar de proyecto); la biblioteca se relee en `tick` cada 500 ms y al abrir el selector.
- Guardar: «Guardar la selección como bloque» abre la línea de órdenes con `bloque `; `bloque nombre [etiquetas] [-- descripción]` guarda la selección (`editing::selected_text`, sin la sangría común con `editing::dedent`) en la global, `bloque-proyecto` en la del proyecto y `!` sustituye.
- Selector (`App::block_picker`, `BlockMode::{Insert, Update, Delete}`; `ViewModel::block_picker`, `ui::BlockPickerPopup`, alto según la vista previa): filtra escribiendo, ↑/↓, Esc cierra sin cambiar de modo. Insertar = `insert_template(0, cuerpo)` como un snippet (campos con Tab, un paso de deshacer). Actualizar (con la selección capturada al abrirlo) y borrar piden un segundo Enter sobre el mismo bloque (`armed`; moverse o filtrar lo quita).
- Ranuras 1-9 (`App::slots`, `SlotOp`): las dos acciones de la paleta esperan el número (`slot_prompt`; otra tecla cancela) y `ui::SlotsPopup` enseña el contenido. Guardar toma la selección o la línea entera con su `\n` (`editing::selection_or_line`); pegar (`editing::paste`) pone las líneas enteras al principio de la línea del cursor, como `P` de vim, en un paso de deshacer. Solo en memoria.
- Autocompletado: `complete::context` recibe los bloques y los ofrece al inicio de línea tras los snippets (tipo «bloque», firma con carpeta y etiquetas; inserta el cuerpo).
- El motor no se ha tocado (no hacía falta ampliar `no_alloc.rs`).
- Tests en `script/tests/blocks.rs` (formato, carpetas, guardar/actualizar/borrar, prioridad del proyecto, archivos rotos, búsqueda, nunca se cargan) y `tui/src/tests/blocks.rs` con `TestBackend` (guardar, selector con vista previa y filtro, actualizar y borrar con confirmación, ranuras, autocompletado, y un bloque con Lua roto: error en el editor y audio idéntico, muestra a muestra, al de no haberlo cargado).

## Fase G: lo que más cambia el sonido ✅

Huellas del audio de antes de la fase en `script/tests/golden.rs` (hash FNV de los bits de tres canciones que usan casi toda la API, con semilla fija; solo se comparan en x86_64 porque libm puede cambiar el último bit en otras arquitecturas): sin usar lo nuevo, la salida es idéntica bit a bit.

- **Longitud variable** (`pista:length(n)`, de 1 a `MAX_STEPS` = 64). El reloj no cambia (semicorcheas, compases de 16 pasos, cambios al compás). `Pattern` guarda `length` y las vueltas del ciclo aplanadas en un `Vec<Option<Trigger>>` (construido en el hilo de control); `Pattern::hit(global)` toca la semicorchea global `g = compás·16 + paso` como el paso `g % length` de la vuelta `g / length` (alineado con la canción aunque se recargue; con 16, vuelta = compás y todo es como antes). `Pattern::new/from_steps/from_bars` aceptan cualquier `AsRef<[...]>` (arrays de 16 o `Vec`); `SendError::InvalidLength`. Notación: `parse_pattern_labeled(texto, longitud, etiquetas)` (`parse_steps*` siguen siendo de 16). Script: `Bar = Vec<Option<Step>>`, `TrackDef::length`, `TrackDef::lap(compás, paso)` y `lap_steps(vuelta)`; `:length(n)` va antes de `:steps()` (después, con otra longitud, da error con línea) y `steps`/`c:steps` piden esa longitud; `every`, `fill` (con más pasos que la vuelta, la vuelta entera), `chance` y `humanize` cuentan vueltas; el swing va en los contratiempos de la canción (`(vuelta·longitud + i)` impar; con longitud impar el ciclo se dobla). `Song::length_bars` = mcm de `longitud·ciclo` de cada pista y 16, en compases, con tope `MAX_SONG_BARS` = 256. Funciones de patrones: de 1 a 64 pasos; `euclid(k, n [, golpe [, pasos]])`. Snippet `polirritmo`. Interfaz: `TrackView::{length, steps (Vec), position}`; el tracker tiene tantas filas como la pista más larga, resalta el paso de cada pista (la fila entera si todas coinciden) y se desplaza siguiendo la pista seleccionada; el panel del motor dice `N pasos`. Diagnóstico en vivo (`diagnose::track_lengths`): longitud de `:length(n)` literal en la misma cadena de `track(` a `track(` (fuera de cadenas y comentarios) o en la que define esa pista; con un `n` que no es un literal no cuenta los pasos. `--pattern` admite de 1 a 64 pasos. Tests en `engine/tests/length.rs` (frames, polirritmia sumada, 32 pasos, ciclo por vueltas, rangos), `no_alloc.rs` (12 y 32 pasos con ciclo), `script/tests/length.rs` (cada caso idéntico muestra a muestra a su patrón desenrollado: 3 contra 4, `every`, `fill`, swing con longitud impar; longitud de la canción; errores) y `tui/src/tests/length.rs`.
- **Time-stretch** (`sample:stretch([bpm_origen])`, `engine/src/stretch.rs`): WSOLA en Rust puro, sin dependencias, en el hilo de control al cargar. `Sample::stretched(ratio)` / `stretched_with(ratio, límites)` (ratio = duración nueva / original, de `MIN_STRETCH` 0,25 a `MAX_STRETCH` 4; con 1, copia exacta): el audio se parte en tramos de golpe a golpe (los transitorios de `Slices::detect(…, 128)` más los límites dados, que el script pone en los inicios de los cortes) y cada tramo `a..b` pasa a `round(a·r)..round(b·r)`, así cada golpe empieza justo en su sitio escalado (como ReCycle) y `Slices::scaled(ratio, frames)` da los mismos límites. Dentro del tramo, ventanas de Hann de 32 ms al 50 %, cada una desplazada hasta ±8 ms (correlación normalizada con la continuación natural de la anterior, búsqueda gruesa cada 4 muestras y fina a 1); la primera, en el golpe y sin buscar; no se lee fuera del tramo; se normaliza por la suma de ventanas. En Lua, `SampleHandle::stretch` guarda el tempo de origen (sin número, `detect_bpm` redondeado; si no lo ve, error con línea) y los cortes se detectan y ajustan (`move`, proyecto) en el original; `ScriptHost::apply_stretch`, tras la evaluación y antes de expandir, estira al tempo de la canción (`bpm()`, el del proyecto o 120; fuera de ×0,25-×4, error), escala los cortes y pone `TrackDef::stretch` (`Stretch { source_bpm, ratio }`). Las pistas del mismo sample comparten el resultado (límites = los cortes de todas) y `ScriptHost::stretched` lo guarda para la carga siguiente (mismo `Arc` si no cambian sample, proporción ni cortes). Interfaz: `TrackView::stretch`, la vista del sample dice `140→170 BPM` y da las posiciones en ms del original, como `App::adjust_slice`. Rubber Band no se usa (ver decisiones en el ROADMAP). Tests en `engine/tests/stretch.rs` (seno a 440 Hz a 6 proporciones: duración exacta, frecuencia a menos del 0,2 % y nivel; copia exacta con 1; determinista; break de 140 a 170: tempo detectado 170 y cada golpe a menos de 2 ms de su sitio), `script/tests/stretch.rs` (cortes en la rejilla de 170, automático, sin cambios a su propio tempo, `Arc` compartido y reutilizado, `move` en ms del original, errores) y `tui/src/tests/stretch.rs`.
- **Delay y reverb como envíos** (`engine/src/bus.rs`): dos buses compartidos, `Rack` en el motor. Delay estéreo con realimentación (`Division`: `1/4`, `3/16`, `1/8`, `1/16`; retardo `bus::delay_frames` = `round(pasos · frames_por_paso)`, el mismo redondeo que los pasos, al tempo que suena; `feedback` 0-0,95, `level` 0-1); reverb Freeverb (8 peines con amortiguación y 4 pasatodo por canal, retardos escalados desde 44,1 kHz, el derecho +23; `size`, `damp`, `level`). Buffers reservados en `new_engine` (el delay, para la negra a 20 BPM) y otra vez en `set_sample_rate` (hilo de control). `Buses` entra por `Batch::set_buses`/`Controller::set_buses` (`Command::SetBuses`, guardado aparte en un lote) al empezar el compás, como el tempo. Envíos por pista `Pattern::with_sends([delay, reverb])` (0-1, `SendError::InvalidBus`), post-fader (después de volumen, mute y solo), suavizados como el volumen (si la pista estaba callada, entran directos); `send_buf` fijos por trozo. Cada bus se duerme cuando no recibe nada y su cola se ha apagado (delay: `retardo · (1 + repeticiones hasta -120 dB)`; reverb: 1 s sin entrada y pico < 1e-6) y entonces se vacía (para no repetir audio viejo al despertar): un bus dormido no se procesa ni se suma, así que sin envíos la salida es idéntica bit a bit (huellas). Valores pequeños a cero (`flush`) para evitar subnormales. Lua: `bus(nombre, { ... })` (global, claves del registro `api::BUSES`, error con línea en claves, rangos y `time`; sin `bus()`, `Buses::default()`; `Song::buses` siempre va en el lote) y `pista:send(bus, nivel)` (`TrackDef::sends`). Interfaz: `→ d0.30 r0.15` en el panel de efectos (`TrackView::sends`); autocompletado de buses en `bus("...")` y `send("...")`, de sus parámetros y de los valores de `time` en la tabla (`Scan::fx_effect` mira `fx` y `bus`), y aceptar otro bus rehace la tabla como con los efectos; ayuda `bus("delay", { ... })`. Tests en `engine/tests/sends.rs` (eco al frame exacto en 4 divisiones y tempos, segundo eco con feedback, cambio al compás, 8 pistas a tope con feedback 0,95 y reverb máxima dentro de ±1 también parado, reposo con silencio exacto, cola de reverb determinista y que decae, rangos), `no_alloc.rs` (envíos y cambio de buses), `script/tests/sends.rs` (igual que el motor, por defecto, errores) y `tui/src/tests/sends.rs`. El render de `exportar` corta la cola al final (dura los compases pedidos).
- **Compresor y distorsión** como efectos de pista (`Fx::Compressor { threshold_db, ratio, attack_ms, release_ms, makeup_db }`, `Fx::Distortion { drive, mix }`, en la cadena de hasta 4 de `:fx`). Compresor estéreo enlazado, feedforward, por picos: reducción en dB = `(nivel_dB − umbral) · (1 − 1/ratio)` por encima del umbral, envolvente de un polo en dB con ataque y release (`exp(−1/(t·fs))`), ganancia `10^((makeup − reducción)/20)`; con la señal bajo el umbral y sin makeup no toca nada (bit a bit); otro compresor en la misma ranura conserva su envolvente. Distorsión: `x + mix·(tanh(drive·x) − x)`, sin estado (nunca pasa de 1). Rangos: umbral −60-0 dB, ratio 1-20, ataque 0,1-200 ms, release 1-2000 ms, makeup 0-24 dB; drive 1-50, mix 0-1. Lua `:fx("compressor"/"comp", { threshold, ratio, attack, release, makeup })` y `:fx("distortion"/"drive", { drive, mix })` (registro `api::EFFECTS`, por defecto −18 dB, 4:1, 5 ms, 100 ms, 0 dB; drive 4, mix 1). Panel: `comp -18 dB 4:1 +6`, `drive ×8 mix 0.50`. Tests en `engine/tests/dynamics.rs` (nivel estable exacto con y sin makeup, sin cambios bajo el umbral, 63 % de la reducción a los 10 ms con 10 ms de ataque, fórmula de la distorsión, rangos), `no_alloc.rs` (cadena de 4 con los dos) y `script/tests/dynamics.rs`.
- **Automatización** (`engine/src/automation.rs`): `Curve::Ramp { from, to, steps }` (diente de sierra que se repite cada `steps` pasos) y `Curve::Lfo { min, max, rate }` (seno que empieza en `min`; `LfoRate::Hz` o `Steps`), evaluadas con la posición de la canción (`Transport::position_steps`: `compás·16 + (frame − inicio del compás)/frames_por_paso`, y los segundos desde que arrancó): deterministas y alineadas aunque se recargue (test: recargar el mismo script a mitad da el mismo audio). Frecuencias en escala logarítmica (`automation::mix`, exacta en los extremos). `Automation { fx, param, curve }` en `Pattern::with_automation` (hasta `MAX_AUTOMATIONS` = 8; después de `with_fx`, validada contra la cadena y el rango del parámetro) entra con el patrón al compás. Parámetros automatizables (`Fx::params`/`param_index`/`param_range`/`param_value`): filtro `cutoff`, `q`; compresor `threshold`, `makeup`; distorsión `drive`, `mix` (el bitcrusher, ninguno). Parameter locks: sufijo `~Hz` en la notación (`Step::lock`, `Trigger::lock`; 20-20000 Hz), el corte del primer filtro de la pista desde ese golpe hasta el siguiente (`Track::lock`, lo pone `play_hit`, también en retrigger). En la mezcla, si la pista tiene curvas, lock o parámetros aún moviéndose (`Track::mods`), `modulate` procesa la cadena en bloques de 32 frames: destino = lock, curva o valor fijo; suavizado de un polo (~5 ms, en log para frecuencias); `FxState::set_param` solo recalcula los coeficientes del biquad si cambia el valor; si la voz empieza desde el silencio, el destino entra ya (test: `~500` con corte fijo 8000 suena bit a bit como un filtro fijo de 500); un parámetro sin nada que lo mueva y ya en su sitio vuelve al valor fijo y deja de procesarse aparte. Si no, el camino de siempre (huellas intactas). Un efecto de otro tipo en la ranura al cambiar de patrón olvida su suavizado; en silencio todo vuelve al fijo. Lua: `ramp(desde, hasta, "4 compases")` y `lfo(frecuencia, mín, máx)` (número = Hz de 0,01 a 50, o `"1 compás"`/`"8 pasos"`) devuelven un `CurveHandle` que va en lugar de un número en la tabla de `:fx` (`parse_fx` devuelve también las curvas; el valor fijo es su primer extremo; `TrackDef::automation`); duraciones con `parse_duration` (compases o pasos, hasta 64 compases); errores con línea (parámetro no automatizable, fuera de rango, duración, frecuencia, `~` fuera de rango o sin filtro en la pista). Registro: `ramp`, `lfo`, `api::automatable(efecto, parámetro)` (lo pregunta al motor), snippet `barrido`. Interfaz: tras `cutoff =` (y los demás automatizables) el autocompletado ofrece `ramp` y `lfo`; el panel de efectos marca `∿cutoff`; el tracker enseña `~800`; el diagnóstico en vivo comprueba el rango de `~`. Tests en `engine/tests/automation.rs` (valores de las curvas, lock = filtro fijo bit a bit, vuelta al fijo, rampa que abre el filtro sin clics, locks alternos con golpes solapados sin clics, determinismo, validación), `no_alloc.rs` (rampa, LFO y locks), `script/tests/automation.rs` (igual que el motor, `~`, recarga alineada, errores) y `tui/src/tests/automation.rs`.

### Decisiones

- **Longitud**: los pasos siguen siendo semicorcheas y el reloj, los
  compases de 16 (los cambios entran al empezar un compás). Una pista de
  `n` pasos toca la semicorchea global `g` como su paso `g % n`, alineada
  con la canción desde que arranca (no desde que entra el patrón): recargar
  no la desfasa. En esas pistas `every`, `fill`, `chance` y `humanize`
  cuentan vueltas del patrón; el swing sigue los contratiempos de la
  canción. `:length()` va antes de `:steps()`. Con 16 pasos, idéntico bit
  a bit (huellas en `script/tests/golden.rs`).
- **Time-stretch**: WSOLA en Rust puro, sin dependencias, al cargar, por
  tramos de golpe a golpe (los transitorios y los inicios de los cortes):
  cada golpe empieza exactamente en su sitio escalado y dentro de cada
  tramo se copian ventanas de 32 ms desplazadas hasta ±8 ms para que
  encajen, así que el tono no cambia. Para breaks (×0,7-×1,4) suena limpio
  y no añade dependencias de C ni problemas en ARM64; Rubber Band
  (vocoder de fase con preservación de transitorios) daría más calidad
  con proporciones grandes o material tonal, pero obligaría a una
  dependencia de C++: queda como idea, tras una feature opcional.
- **Envíos**: dos buses globales (delay y reverb Freeverb), post-fader, con
  buffers reservados al crear el motor; los parámetros entran en el lote
  al compás. Un bus sin entrada y con la cola apagada se duerme (no suma
  nada: sin envíos, la salida es idéntica bit a bit) y se vacía.
- **Compresor y distorsión**: efectos de la cadena de cada pista
  (compresor por picos, estéreo enlazado; distorsión `tanh`).
- **Automatización**: curvas en el patrón (entran al compás) que dependen
  solo de la posición de la canción (deterministas, alineadas al
  recargar); `ramp` es un diente de sierra que se repite. Los parameter
  locks (`~`) son del corte del primer filtro y duran hasta el siguiente
  golpe. Todo se suaviza ~5 ms y el filtro recalcula sus coeficientes cada
  32 frames.

Sitios que asumían 16 pasos: todos adaptados salvo estos, que quedan así a
propósito o con un límite. El diagnóstico en vivo solo conoce la longitud
si `:length(n)` lleva un número escrito tal cual y está en la misma cadena
de `track(...)` (o en la que define esa pista): con una variable no cuenta
los pasos (los revisa uno a uno) y un `local t = track(...)` seguido de
`t:steps(...)` después de otro `track(` se mide con la longitud de ese
otro. El tracker tiene 16 filas a la vista y se desplaza con las pistas
más largas. Exportar la canción entera tiene un tope de 256 compases (con
longitudes que no encajan, como 61 y 63 pasos, el ciclo común sería
enorme).

## Fase H: cortes y breaks ✅

- **Gate/one-shot, envolvente por corte y choke.** `SliceShape { attack_ms, release_ms }` (0 = el fundido mínimo de 1,5 ms; hasta 2000 y 5000 ms) va en `Playback` y se pone por corte con `Pattern::with_shapes` (cada `Trigger` sabe su `slice`). La voz tiene rampas de entrada (`fade_in`) y salida (`fade_out`) separadas, y `release: Option<(quedan, total)>`: interrumpir sigue usando el fundido mínimo; con los valores por defecto la envolvente es la de antes bit a bit (`min(a/f, b/f) = min(a, b)/f`). Modo gate (`Pattern::with_gate`): `play_hit` recibe la duración del paso (o de su parte con retrigger) y la voz abre su cola ahí (`Voice::gate_at`). Choke (`Pattern::with_choke`, 1-16): tras disparar un golpe (secuenciador o pad, `Engine::choke`), las demás pistas con el mismo grupo se callan con el fundido mínimo. Pads con forma: `Controller::play_slice_shaped`. Lua: `cortes:set([i,] { attack, release })` (claves del registro `api::SLICE_PARAMS`; las formas se comparten entre la variable y sus pistas como las etiquetas, `slice_at` copia la suya; `TrackDef::shapes`), `pista:mode("gate"|"oneshot")` (`TrackDef::gate`), `pista:choke(n)` (`TrackDef::choke`; 0 lo quita). Interfaz: autocompletado y ayuda en la tabla de `set` (`api::SLICE_TABLE`), `· gate · choke N` en el panel de efectos, pads con la forma de su corte. Tests en `engine/tests/choke_gate.rs` (gate al final del paso y con retrigger, ataque y cola al frame, cola al final en one-shot, choke por grupo y desde un pad, rangos), `no_alloc.rs`, `script/tests/shapes.rs` y `tui/src/tests/shapes.rs`.
- **Rejilla, añadir y quitar cortes.** `Slices::grid(sample, n)` (corte `k` en `round(k·frames/n)`), `Slices::split(frame, margen)` (parte el corte que lo contiene, o añade uno antes del primero; devuelve el índice del nuevo; a más de 1 ms de los límites) y `Slices::without(i)` (quita la marca del corte `i ≥ 1`: el anterior se alarga). La forma de cortar es la clave de etiquetas, ajustes y ediciones: `project::Cut { count, grid }` (`slice(8)` o `slice_grid(16)`; en el proyecto `8` o `rejilla16`; `Cut::call()` para los mensajes), `SliceLabel::slices`/`SlicePosition::slices` son `Cut`, `SliceOrigin` lleva `grid` y `cut()`, y `set_label`/`set_position` reciben un `Cut`. Ediciones del proyecto (`SliceEdit { sample, cut, kind: EditKind::Split(ms) | Join(corte) }`, líneas `partir = amen.wav 8 180.25` y `unir = amen.wav 8 7`, se guardan antes que los `corte`): `cut_sample` (común a `slice` y `slice_grid`) las aplica en orden justo después de detectar (las que no caben, aviso en `Song::warnings`), luego los ajustes y luego las etiquetas, que van con la numeración final; al cargar el proyecto, una etiqueta o ajuste de un corte que no existe (contando los `partir`) da error con línea. `ScriptHost::split_slice`/`join_slice` añaden la edición y corren las etiquetas y ajustes siguientes (`renumber`; los del corte quitado se van). Lua: `sample:slice_grid(n)`, `cortes:add(ms)` y `cortes:remove(i)` (como `move`: cambian la variable y las pistas definidas antes no lo ven; copian etiquetas y formas; `SliceOrigin::indices` es `Vec<Option<usize>>` y los cortes añadidos en el script son `None`, que la interfaz no deja etiquetar ni mover). Interfaz: «Partir el corte seleccionado» (en su mitad, al microsegundo, en ms del original) y «Unir el corte seleccionado con el anterior» (`App::edit_slices`: si la carga no lo acepta, deja etiquetas, ajustes y ediciones como estaban), guardan el proyecto. Tests en `engine/tests/slicing.rs`, `script/tests/grid.rs` (rejilla igual que el motor, `add`/`remove`, ediciones antes de ajustes y etiquetas, claves separadas, guardar y leer, errores) y `tui/src/tests/grid.rs`.
- **Pan, parte del corte, bucle e ida y vuelta.** `SliceShape` suma `pan` (-1 a 1, ley de balance: el lado contrario por `1 − |pan|`, el propio por 1; al centro, multiplicar por 1 no cambia ni un bit), `length` (0,01 a 1: la parte del corte que suena desde su inicio; `SliceShape::range`, aplicada al resolver en `with_shapes` y en los pads) y `play: PlayMode::{Once, Loop, PingPong}`. En bucle la voz no acaba sola (`length = usize::MAX`, sin rampa de final): suena hasta el siguiente golpe de la pista, la puerta del modo gate o el choke; `Voice::read` lleva el desplazamiento al tramo (`rem_euclid`; ida y vuelta sin repetir los extremos, periodo `2·(n−1)`). El salto del bucle lleva un fundido cruzado (ver «Fundido en el salto de los bucles de corte» en «Mejoras sueltas»). Lua: claves `pan`, `length` y `play` (`"normal"`, `"loop"`, `"pingpong"`) en `cortes:set`; autocompletado de los valores de `play`. Tests en `engine/tests/slice_shape.rs` (pan exacto por lado, `length` = un corte que acabara ahí, bucle periódico y cortado por la puerta, ida y vuelta simétrica y periódica, rangos), `no_alloc.rs`, `script/tests/shapes.rs` y `tui/src/tests/shapes.rs`.
- **Remuestreo.** `Song::render_track(frecuencia, compases, pista)` renderiza en un motor aparte una copia de la canción con esa pista en solo (y sin mute): sus efectos, volumen, envíos y la cola de los buses, y el limitador; `Song::track_length_bars(pista)` = lo que tarda en repetirse (`mcm(16, ciclo·longitud)` en compases). `tui --script canción.lua --render salida.wav --track pista [--bars M]`. Interfaz: orden `remuestrear [pista|master] [compases] [archivo.wav]` (por defecto la pista seleccionada y `<pista>-remuestreo.wav` en la carpeta de samples; no sobrescribe salvo `remuestrear!`; WAV de 16 bits a la frecuencia del motor) y «Remuestrear la pista seleccionada» en la paleta, que la deja escrita. Tests en `script/tests/resample.rs` (igual que la canción con solo esa pista, también en mute; duración) y `tui/src/tests/resample.rs` (WAV escrito, no sobrescribe, master, y se vuelve a usar con `sample()`).
- **Preparar al importar.** `engine::ImportOptions { trim_db, normalize_db, fade_in_ms, fade_out_ms }` y `Sample::processed(&opciones)` (hilo de control): recorta los extremos por debajo del umbral (si todo es silencio, nada), normaliza el pico y aplica fundidos lineales, en ese orden; sin opciones, copia exacta. Lua: `sample(ruta, { trim = true | dB, normalize = true | dB, fade_in = ms, fade_out = ms })` (`true`: -60 dB y 0 dB; claves del registro `api::SAMPLE_PARAMS`, errores con línea), con caché de los preparados entre cargas (`Imports`: mismo `Arc` si no cambian el WAV ni las opciones). Los cortes, los ajustes en ms y `stretch` van sobre el sample preparado. Autocompletado y ayuda en la tabla (`api::SAMPLE_TABLE`). Tests en `engine/tests/import.rs` (recorte exacto, umbral, todo silencio, pico normalizado y misma forma, fundidos al frame, orden, rangos), `script/tests/import.rs` y `tui/src/tests/import.rs`.

- **Efectos por corte** (`engine/src/slice_fx.rs`, añadido después de la fase K): `SliceFx { fx: [Option<Fx>; 4], automation: [Option<Automation>; 8] }` (`SliceFx::new` valida cada efecto, sus curvas y que ningún EQ dinámico tenga sidechain) y `Pattern::with_slice_fx(&[Option<SliceFx>])` (una por corte; las vacías cuentan como ninguna; si no hay ninguna, `Vec` vacío; error con un instrumento, en los dos órdenes). `play_hit` copia la cadena del corte del golpe en su voz (`Voice::fx: Option<VoiceFx>`: la cadena, un `FxState` por efecto, los valores suavizados de sus parámetros y la cola que le queda; tamaño fijo, nada de memoria dinámica) y `Voice::mix_with_fx` la suma a la pista pasando por `Engine::voice_buf` (un buffer de trozo reservado al crear el motor) y por sus efectos; sin cadena, `mix_into` directo como siempre. Orden: voz (volumen del paso, envolvente y pan) → efectos del corte → efectos de la pista → volumen de la pista y mute/solo → mezcla (y envíos, que son de la pista). Cada golpe empieza sus efectos sin memoria (también cada subdisparo del retrigger); la voz cortada sigue con los suyos mientras hace su fundido, y al acabar una voz sus efectos suenan aún 100 ms (`TAIL_SECONDS`: la resonancia de un filtro) antes de soltarla (`Voice::ringing`; `fade_out` y la ranura de fundido la esperan). Las curvas (`ramp`/`lfo`) se evalúan con la posición de la canción cada 32 frames y se suavizan como las de la pista: `drive_block`, la parte de `modulate` que ahora comparten la pista y la voz (con la misma aritmética: las huellas no cambian); el primer bloque de cada voz entra ya en su valor. Pads: `Command::Play` lleva el corte (`slice`) y, si el sample es el del patrón de la pista (`Arc::ptr_eq`), suena con su cadena. Granos: la nube no usa voces, así que no pasa por ellos. Efecto nuevo `Fx::Gain { level }` (0 a 4, lineal, automatizable; también en la pista: así se puede mover su volumen con `ramp`/`lfo`). Lua: `cortes:fx(cuál, nombre, { ... })` (`cuál`: número desde 1, etiqueta o lista; los efectos y la tabla de `parse_fx`, con curvas por `fx_automations`, compartido con `pista:fx`), hasta 4 por corte; las cadenas (`SliceFxDef { fx, automation }`) se comparten entre la variable y sus pistas como las formas (`TrackDef::slice_fx`; `slice_at` copia la suya; `add`/`remove` las mueven con su corte). Errores con línea: etiqueta o corte que no existe, lista vacía o de otro tipo, efecto o parámetro desconocido, rangos y curvas fuera de rango, `sidechain`, más de 4; `instrumento:fx()` da error. Aviso en `Song::warnings` (`grains_skip_slice_fx`) si la pista tiene granos. Registro: función `fx` de `Receiver::Slices` y efecto `gain`. Interfaz: el panel de efectos dice cada corte con su cadena antes de las de la pista (`TrackView::slice_fx`, `SliceFxView`: `◇ 2 snare › crush 6b ÷1 · LP 800 Hz q0.71 ∿cutoff`); en `cortes:fx(` el autocompletado ofrece etiquetas (también dentro de la lista), luego efectos y luego sus parámetros (`Scan::slices_fx`, `fx_effect` con el tercer argumento), y la ayuda la firma y el parámetro; el diagnóstico en vivo (`diagnose::check_fx`) subraya una etiqueta que no conoce nadie (las de las pistas, del proyecto y del texto) y un efecto que no existe (también en `pista:fx`). Tests en `engine/tests/slice_fx.rs` (solo cambia el corte elegido, con bitcrusher y con filtro resonante más distorsión, y los demás golpes bit a bit; sin cadenas, con cadenas vacías o en un corte que no suena, bit a bit; dos cortes con efectos distintos y dos pistas a la vez, cada uno con los suyos; cada golpe empieza sin memoria; rampa en `level`; reverse, `^`, `%`, retrigger, gate y choke; pads; granos los ignoran e instrumento los rechaza; 8 pistas saturadas ≤ ±1; la cola suena y se calla; validación), `no_alloc.rs` (dos pistas con cadenas de todos los efectos y curvas, retrigger, reverse, pitch con y sin `%`, gate, choke, pads y cambiar y quitar patrones con colas sonando), `script/tests/slice_fx.rs` (solo cambian los cortes elegidos por número, etiqueta y lista, antes o después de la pista; la cadena y sus curvas llegan a la pista y `slice_at` la copia; `add`/`remove`; aviso con granos y error en instrumento; 8 pistas ≤ ±1; errores con línea sin dejar de sonar lo anterior; el cambio entra al compás), `script/tests/registry.rs`, `tui/src/complete.rs`, `tui/src/diagnose.rs` y `tui/src/tests/slice_fx.rs`.

### Decisiones

- **Efectos por corte en la voz, no en una pista oculta.** Separar cada
  corte con efectos en una pista interna gastaría pistas del motor (son 8)
  y rompería lo que es de la pista (choke entre sus golpes, el golpe que
  corta al anterior, gate, retrigger). Copiar la cadena en la voz mantiene
  todo eso y es barato: ~2 KB más por voz (dos por pista) y, en el hilo
  de audio, solo un buffer de trozo y una suma por voz con efectos. Estado
  por voz y no por corte: dos golpes del mismo corte que se solapan (uno
  haciendo su fundido) no se pisan la memoria. Sin efectos de corte, ni
  siquiera se pasa por el buffer aparte: `golden.rs` no cambia.
- **Orden voz → corte → pista → volumen → mezcla.** El efecto del corte ve
  el golpe ya con su volumen, envolvente y pan (un `@0.5` satura menos), y
  la pista sigue tratando la suma como siempre. `~` (el corte por paso)
  sigue siendo del primer filtro de la pista: así un paso no cambia de
  significado según su corte tenga efectos.
- **La sintaxis va en los cortes** (`cortes:fx(cuál, ...)`), como
  `:set` y `:name`: la forma de un corte es de la variable y vale en todas
  sus pistas y en sus pads. `cuál` admite número, etiqueta o lista para
  escribir `{ "kick", 7 }` de una vez. La etiqueta tiene que existir ya al
  llamar a `fx` (si no, error con la línea): resolverla más tarde
  escondería las erratas.
- **Combinaciones**: reverse, pitch (`^` y `%`), retrigger, gate, choke y
  pads funcionan (los efectos van tras la voz, lea como lea el sample).
  Con granos se ignoran con aviso (la nube no tiene voces y no se quiere
  romper otra pista que comparta la variable); en un instrumento, error
  (es un solo corte: lo mismo con `pista:fx`). El EQ dinámico de un corte
  no admite sidechain: el corte suena antes de que se mezclen las demás
  pistas y escucharlas por voz obligaría a reordenar la mezcla.
- **Envíos por corte, fuera.** No son baratos de hacer bien: el envío de la
  pista sale después de sus efectos y su volumen, y la voz de un corte ya
  no se puede separar de la suma que procesan los efectos de la pista;
  hacerlo exigiría pasar cada voz por los efectos de la pista por separado
  (el doble de estado y de CPU por voz). Un envío antes de los efectos de
  la pista sí sería barato, pero sonaría distinto que el de la pista y
  confundiría. Para un corte con reverb: `track("caja", amen:slice_at(3))`
  con su `send`, que copia la forma y los efectos del corte.
- **Volumen y pan por corte**: el pan ya era de la forma (`cortes:set(i,
  { pan = ... })`); el volumen es el efecto nuevo `gain`, que por ser un
  efecto admite `ramp()`/`lfo()`.

- **Por pista o por corte**: el modo (gate/one-shot) y el grupo de choke
  son de la pista, como en un pad de una caja de ritmos; la forma
  (envolvente, pan, parte, bucle) es de cada corte y se comparte entre la
  variable y sus pistas, como las etiquetas. Con los valores por defecto,
  todo suena igual que antes bit a bit (huellas).
- **Gate**: la voz se cierra al acabar su paso (o su parte del
  retrigger) y hace la cola del corte. **Choke**: el corte callado hace el
  fundido mínimo; vale también para los pads.
- **Bucles**: suenan hasta el siguiente golpe de la pista, la puerta o el
  choke; el salto lleva un fundido cruzado de 10 ms por defecto (ver
  «Mejoras sueltas»). `length` es una fracción del corte, no ms.
- **Cortes a mano**: en el proyecto, `partir` va por posición (ms) y
  `unir` por número, y se aplican en orden justo después de detectar; los
  ajustes y las etiquetas van con la numeración que queda, y la interfaz
  los corre al partir o unir. Las claves del proyecto distinguen
  `slice(8)` de `slice_grid(8)` (`rejilla8`). En Lua, `add`/`remove`
  cambian la variable como `move`, y esos cortes no se pueden etiquetar ni
  mover desde la interfaz.
- **Remuestreo**: render offline de la pista en solo (con sus efectos,
  envíos y cola de los buses) o del master, a WAV de 16 bits en la
  carpeta de samples; no sobrescribe sin `!`. Dura, por defecto, lo que
  la pista tarda en repetirse; la cola que pase del final se corta (para
  que vuelva a encajar como loop).
- **Importar**: se prepara al cargar (recorte, normalizado y fundidos, en
  ese orden) sin tocar el archivo; los ms de los ajustes son del sample
  preparado.

## Fase I: comodidad ✅

- **Explorador de samples** (`tui/src/browser.rs` la lista y el filtro, sin terminal; `App::browser` el estado): «Explorador de samples» en la paleta lista los WAV de la carpeta de muestras y sus subcarpetas (rutas relativas con `/`, hasta 4 niveles y 2000 archivos, sin carpetas ocultas); escribir filtra (cada palabra, sin mayúsculas ni tildes), ↑/↓ eligen y preescuchan el archivo entero por la voz de preescucha (`preview_slice` con `Slices::grid(sample, 1)`; los WAV leídos se guardan mientras está abierto), Tab repite, Enter inserta `sample("ruta")` en el cursor (un paso de deshacer) y Esc cierra; los dos paran la preescucha. `ui::BrowserPopup`, `ViewModel::browser`. Tests en `browser.rs` y `tui/src/tests/browser.rs` (sin tocar las pistas).
- **Tap tempo, rampas de BPM y compases.** `tui/src/tap.rs` (`TapTempo::tap(instante)`: media de hasta 8 intervalos, otra cuenta tras 2 s; el instante se pasa, así se prueba sin esperar); «Tap tempo» en la paleta (`App::start_tap`/`tap_at`/`handle_tap`: espacio golpea, Enter escribe `bpm(N)` con `set_bpm_line`, otra tecla cancela). Motor: `TempoRamp { from, to, bars }` (`bpm_at(k)`: el compás `k` desde que entra, interpolado; después, `to`) y `Meter { steps, beat }` (`Meter::new(n, d)`, d = 4, 8 o 16: `n·16/d` pasos y un pulso cada `16/d`), por `Batch::set_tempo_ramp`/`set_meter` y `Controller::set_tempo_ramp`/`set_meter` (`Command::SetTempoRamp/SetMeter`, guardados aparte en el lote). El transporte los aplica al empezar el compás: cada compás a su tempo (cuantizado; los pasos siguen al frame exacto), la misma rampa reenviada sigue por donde iba (recargar no la reinicia), un `bpm` fijo la para y `start` la vuelve a empezar. El transporte cuenta `steps_before` (semicorcheas de los compases anteriores): `advance` devuelve también la semicorchea global, que leen las pistas, y `position_steps` la usa; `Status::{global, bar_steps}`; `TrackDef::lap(global)`. Metrónomo con un pulso cada `beat` pasos. El delay desliza su retardo (~50 ms, lectura interpolada; con retardo entero, exacta) cuando cambia el tempo o la división: sin clics. `bar_frames_steps`. Script: `bpm_ramp(desde, hasta, "N compases")` (manda sobre `bpm()`; `Song::bpm` = `desde`), `meter(n, d)`; `Song::{tempo_ramp, meter, bar_steps, bpm_at, frames}` (el render suma compás a compás), el lote envía siempre el compás (4/4 sin `meter()`). Tests en `engine/tests/tempo.rs` (frames de una rampa, recarga, `bpm` que la para, 3/4 y 7/8 con metrónomo, delay sin saltos), `no_alloc.rs`, `script/tests/tempo.rs`, `tap.rs` y `tui/src/tests/tempo.rs`.
- **Teclas y colores configurables** (`tui/src/config.rs`, `tui/src/theme.rs`; dependencia nueva `toml` en Rust puro, solo lectura con `toml::Table`, sin derive). `~/.config/sampler/config.toml` (`config::dir()`, la carpeta de los bloques globales): `[teclas]` con las acciones de `config::BINDABLE` (nombre, `Bound::{Action, Palette}` y teclas por defecto: los atajos de siempre más algunas acciones de la paleta sin tecla), cada una con una tecla o una lista (`KeyCombo::parse`: `ctrl-`/`alt-` + letra o número, `f1`-`f12`, `esc`; una letra sola no vale); las que aparecen sustituyen a las suyas solo si alguna vale; choques entre dos de la configuración, la segunda fuera con aviso; una tecla por defecto que coge otra acción, aviso. F1 abre siempre la paleta (`Keymap::lookup`, antes que nada) y no se puede dar a otra cosa: la paleta nunca queda inaccesible. `App::handle` pasa por el mapa (`App::set_config`; `handle_plain` para lo que no es atajo) y la paleta (`palette::entries_with`) y la línea de ayuda (`Keymap::help`, `ViewModel::key_help`) enseñan las teclas que hay. `[colores]` con los nombres de `theme::NAMES` (nombre de color, `#rrggbb` o número de 256): tema por hilo (`theme::set/get`; los widgets usan `accent()`, `dim()`... en vez de constantes); sin truecolor (`COLORTERM`), `theme::downgrade` pasa los RGB al más cercano del cubo de 256 o de la escala de grises. Si el archivo falta, está roto o tiene claves o valores que no valen: por defecto y avisos en el mensaje, nunca un fallo. Tablas de la guía generadas (`config:inicio`/`config:fin`, `ACTUALIZAR_GUIA=1 cargo test -p tui the_guide_lists_the_config`). Tests en `config.rs`, `theme.rs` y `tui/src/tests/config.rs`.
- **Entrada MIDI** (solo con `--allow-midi`; feature `midi`, por defecto, que trae `midir` 0.11, que usa la misma ALSA que cpal: `cargo build --no-default-features` compila sin MIDI y `--allow-midi` lo dice). `tui/src/midi.rs` (sin dispositivos): `parse(bytes) -> MidiEvent` (notas, note off o nota con velocidad 0, reloj, Start/Continue/Stop), `MidiConfig { base_note: 36, channel, clock }` (`[midi] nota_base`, `canal`, `reloj` en `config.toml`), `slice_for(canal, nota)`, y `ClockTempo` (24 PPQN; pendiente por mínimos cuadrados de los instantes de los últimos 96 pulsos, cuatro negras; redondeo a 0,1 y solo si cambia más de 0,5 BPM; un hueco de más de 0,5 s empieza otra cuenta; nada antes de una negra). `tui/src/midi_io.rs` (feature `midi`): puerto virtual de entrada «sampler entrada» con `Ignore::None` (para recibir el reloj); su callback mete `TimedEvent` en una cola `rtrb` (se pierde si está llena, nunca bloquea) y la aplicación la vacía en su bucle (`App::poll_midi`, cada 2 ms con MIDI). `App::midi_event`: una nota dispara el corte de la pista seleccionada por `play_pad_with_gain` (la ruta de los pads; velocidad/127 como volumen, `Controller::play_slice_with_gain`); con `reloj`, el tempo por `set_bpm` (al compás) y Start/Continue/Stop la marcha. Tests en `midi.rs` (mensajes, notas, reloj a 120 con ±2 ms de jitter que da un solo tempo a menos de 0,5 BPM, cambio a 140, hueco) y `tui/src/tests/midi_in.rs` (notas por la cola como pads, velocidad, canal y base de la configuración, reloj ignorado sin `reloj` y que pone 150 con él, Stop). `no_alloc.rs` con un golpe con volumen.
- **Salida MIDI.** `Pattern::with_midi(Some((canal, nota_base)))` (`SendError` si el canal no es 1-16 o la nota pasa de 127): en `play_hit`, si la pista se oye (sin mute ni solo de otra), el motor apaga la nota anterior de la pista y empuja un note on (`MidiNote { frame, channel, note: base + corte, velocity: volumen·100 }`) con el frame exacto a una cola `rtrb` de 512 (si se llena, se pierde; nunca bloquea ni reserva) y programa su note off para el final del paso (o de su parte del retrigger; `Track::midi_off`, `Engine::midi_due` tras cada trozo, con el frame en que tocaba); `Stop` las apaga. `Controller::midi_events()` las vacía; `MidiNote::bytes()`. Los pads no mandan notas. Lua `pista:midi(canal [, nota_base = 36])` (`TrackDef::midi`). Interfaz: `midi::MidiSink` (el puerto virtual «sampler salida» de `midi_io::open_output` con `--allow-midi`, o `Recorder` en los tests); `App::tick` vacía la cola al destino (sin destino, la tira). `· MIDI 2/48` en el panel de efectos. Guía: cómo conectarlo a Surge XT (`aconnect`) y grabar su audio por PipeWire (`pw-record --target`). Tests en `engine/tests/midi_out.rs` (notas y note off al frame, retrigger, velocidad, mute, `Stop`), `no_alloc.rs`, `script/tests/midi.rs` y `tui/src/tests/midi_out.rs`.
- **Ranuras que persisten.** `App::set_slots_dir` (`main` pasa `config::dir()/ranuras`; los tests, una carpeta suya): lee `1.txt` a `9.txt` al abrir (un archivo ilegible se avisa y esa ranura queda vacía) y cada «Guardar en una ranura» escribe su archivo (`App::save_slot`; crea la carpeta; si falla, se avisa y la ranura sigue en memoria). Test en `tui/src/tests/blocks.rs`.

### Decisiones

- **Explorador**: un popup como el de los bloques; preescucha por la voz
  aparte (la de los cortes), así nunca toca las pistas; Enter escribe
  `sample("ruta")` en el cursor.
- **Tempo**: el tap tempo propone y escribe `bpm(N)` (no cambia el tempo a
  escondidas). La rampa da a cada compás su tempo (cuantizado al compás,
  los pasos siguen al frame); recargar el mismo script no la reinicia. El
  compás (`meter`) cambia los pasos por compás; el transporte cuenta
  semicorcheas acumuladas para que las pistas sigan alineadas. Para que
  el delay no haga clic, su retardo se desliza ~50 ms cuando cambia el
  tempo.
- **Configuración**: TOML (crate `toml`, Rust puro, solo lectura). F1 abre
  siempre la paleta y no se puede dar a otra cosa: la paleta nunca queda
  fuera de alcance. Las teclas sin Ctrl/Alt (salvo F y Esc) no valen
  (escribirían texto). Colores con degradación a 256 sin truecolor. Si
  falta o está roto, por defecto y un aviso.
- **MIDI**: `midir` (ALSA, la misma biblioteca que ya usa cpal) tras la
  feature `midi`, activada por defecto (`--no-default-features` compila sin
  ella); nada se abre sin `--allow-midi`. Puertos virtuales («sampler
  entrada», «sampler salida») en vez de buscar dispositivos: se conectan
  con `aconnect`. La entrada pasa por una cola sin bloqueos a la
  aplicación, que la atiende cada 2 ms con la misma ruta que los pads (así
  usa la pista seleccionada, su forma y su choke). El reloj se estima por
  mínimos cuadrados sobre cuatro negras, con histéresis de 0,5 BPM, y
  entra al compás. La salida la genera el motor con el frame exacto de
  cada golpe (cola sin bloqueos) y la interfaz la manda; el jitter es el
  del bloque de audio y del bucle (unos ms). Nada de esto se ha probado
  con dispositivos reales: la lógica, con mensajes y destinos simulados.
- **Ranuras**: texto tal cual en `~/.config/sampler/ranuras/N.txt`.

## Fase K: melodías con samples ✅

Hechos los ocho puntos: interpolación del pitch, instrumento, pitch sin cambiar la duración, granular y congelar, tocarlo en vivo, salida MIDI de sus notas, diagnóstico en vivo de `melody` y formantes.

- **Interpolación del pitch** (`engine/src/interp.rs`, `interp::read`, la usa `Voice::read`): sinc con ventana de Kaiser (8 cruces por cero, β = 8) en una tabla de 512 entradas por cruce (`LazyLock` de un array fijo, sin memoria dinámica; `new_engine` la calcula con `interp::prepare` para que el audio nunca la inicialice), interpolada entre entradas, normalizada por la suma de pesos. Al subir (`r` > 1) el corte baja a `0,9 / r` y el núcleo se ensancha a `8 / corte` frames por lado: filtra lo que se plegaría. Con `r` ≤ 1 el corte es 1 (pasa por las muestras) y una posición entera devuelve la muestra: pitch 0, bit a bit. Los vecinos fuera del rango de la voz repiten su borde. Tests en `engine/tests/interpolation.rs` (seno agudo a +12 y +24 que pasaría del Nyquist: lineal 0 dB, ahora < -75 dB; sin pasarlo, limpio y con su nivel; +7 y -12: lineal > -40 dB, ahora < -85 dB; pitch 0 bit a bit), `step_effects.rs` (el borde de un bloque se mide a media altura) y `no_alloc.rs` (pasos con pitch). Huellas nuevas en `golden.rs` para las canciones con pitch, y sus versiones sin pitch con las huellas de antes.
- **Instrumento** (`engine/src/instrument.rs`): `Instrument { voices (1-16, MAX_VOICES), adsr: Adsr { attack_ms, decay_ms, sustain, release_ms }, sustain_loop: Option<(inicio, fin)> en frames, crossfade (frames) }` con `validate(frames)` y `Pattern::with_instrument` (`SendError::InvalidInstrument`). Cada pista del motor tiene `notes: [Option<Note>; 16]` y `tails` (robadas), fijos. `play_hit` en una pista con instrumento llama a `play_notes`: una nota por cada nota del acorde; si las voces (`Note::is_voice`: activa y no cortada) llegan al límite, roba la de menor `born` (orden de llegada), la corta (`kill`, fundido mínimo) y la pasa a `tails` (`park_tail`; si están llenas, sale la más antigua). Las notas no se cortan entre sí. `Note`: posición `rendered · rate` (con bucle, `inicio + (pos − inicio) mod largo`), lectura por `interp::read`, fundido cruzado lineal en `[fin − crossfade, fin)` con `pos − largo` (lo de antes del inicio), sin bucle si va al revés; ADSR como función pura del frame (`adsr(n)`, tramos de al menos 1,5 ms, release desde el nivel de `gate`), fundido mínimo al final del sample sin bucle; `is_active` hasta `gate + release` (o el final del sample). La duración (`gate`) es la del paso más `Step::hold` pasos (`Retrig::gate`: con retrigger, cada parte su trozo y la última, además, el `hold`); vale también para el modo gate y las notas MIDI de salida. Choke y `Stop` cortan las notas; la mezcla las suma en el buffer de la pista (después, efectos, volumen, envíos) y suelta las acabadas con `discard`; `holds` mira notas y colas. `TrackStatus::voices` (máximo a la vez desde la lectura anterior, `SharedStatus::add_voices` con `fetch_max`). Notación: `_` (`notation::HOLD`) suma `hold` al golpe anterior (error si no hay; `_` ya no vale como etiqueta) y varios `^` son un acorde (`Step::chord`: hasta `MAX_CHORD − 1` = 3 intervalos sobre `semitones`; `Step::notes()`, `with_chord`, `with_hold`; `Trigger::chord` con sus velocidades; el rango ±36 se valida por nota). Lua: `sample:instrument({ root, loop, voices, adsr, crossfade })` (`InstrumentHandle`, receptor `Receiver::Instrument`; claves de `api::INSTRUMENT_PARAMS`, tabla `api::INSTRUMENT_TABLE` para el autocompletado y la ayuda; `root` nota o MIDI, `loop` `{inicio, fin}` en ms, `adsr` `{a, d, s, r}`, `crossfade` 10 ms por defecto, acortado con aviso en `Song::warnings` si no cabe antes del inicio o en el bucle; error con `stretch`). `track()` acepta cortes o instrumento (`track_source`): un instrumento es una pista de un corte (`Slices::grid(1)`, `SliceOrigin` con `indices = [None]`) con `TrackDef::instrument` (`InstrumentDef { root, loop_ms, engine }`). `pista:melody(notas)` y `c:melody()` en `every` (`patterns::notes(texto, raíz)`: notas `c`…`b` con `#`/`b` y octava opcional (sin ella, la de la raíz), acordes con `+` (las siguientes sin octava, justo por encima), `.` y `_`, sufijos salvo `^`; `patterns::note_number`/`note_name`, C4 = 60); en una pista de cortes, raíz C4 sobre el corte 1. Errores con línea: acorde en una pista de cortes, `midi()` en un instrumento, notas fuera de ±36, opciones. Snippet `instrumento`. Interfaz: `TrackView::instrument` (`InstrumentView { root, voices, sounding }`), el tracker escribe las notas (`TrackView::cell_text`, `model::note_text`: `C3+E3+G3`) y `│` en las filas alargadas (`TrackView::held`), el panel de efectos `· ♪ C3 3/4`, autocompletado y ayuda en la tabla de `instrument`. Tests en `engine/tests/instrument.rs` (frecuencia de cada nota, ADSR al frame, solapes = suma, acorde, voces que nunca pasan del límite con 1-16, robo de la más antigua sin clic, bucle sostenido sin clic con fundido y con clic sin él, fin del sample, paro, rangos), `engine/tests/notation.rs` (acordes, `_`), `no_alloc.rs` (instrumento con acordes, robo, bucle y choke), `script/tests/instrument.rs` (frecuencia por nombre de nota con tres raíces, igual muestra a muestra que el motor, bucle sin clic, robo con acordes, `melody` en cortes y en `every`, aviso del fundido, 20 errores con línea), `script/tests/registry.rs` y `tui/src/tests/instrument.rs`.

- **Instrumento en vivo** (punto 5): `Controller::note_on(pista, sample, rango, instrumento, semitonos, volumen, tecla)` y `note_off(pista, tecla)` (`Command::NoteOn/NoteOff`; llevan el sample y el instrumento, como `Play`, así que suenan también con el secuenciador parado). En el motor, `Engine::open_note` (común con las notas del secuenciador: busca voz, roba la más antigua) con `gate = instrument::HELD` (`u64::MAX`: suena hasta el note off) y `Note::key`; `note_off` llama a `Note::release_now` (el release empieza en ese frame desde el nivel en que esté) en las notas de esa tecla; la misma tecla otra vez suelta la anterior. Choke y paro, como siempre. En la interfaz (`App::live_note_on/live_note_off`, `HeldNote`): en una pista con instrumento, el pad `i` (`app::PADS`) toca `i` semitonos sobre la raíz con la tecla `PAD_KEY + i` (128 + i; las de 0 a 127 son las notas MIDI), y una nota MIDI suena a su altura (`nota − raíz`, filtrada por `[midi] canal`, sin `nota_base`) y su note off la suelta, aunque se haya cambiado de pista. Sin avisos de soltar tecla, una pulsación de un pad que ya suena es su repetición de teclado (no vuelve a empezar la nota) y `App::release_expired_pads(ahora)` (en `tick`) la suelta si no llega ninguna en `PAD_FIRST_HOLD` (0,7 s) tras la pulsación o `PAD_REPEAT_HOLD` (0,15 s) tras la última repetición; con ellos (`App::set_key_releases`: `main` pide `REPORT_EVENT_TYPES` del protocolo de teclado de kitty si `supports_keyboard_enhancement`, y pasa los `Release` a `App::key_release`), al soltar. Salir del modo tocar o cambiar de pista suelta los pads. Grabar: `Recording::note(posición, semitonos, volumen)` y `release(paso, posición)` escriben `1^st` y `_` hasta el note off. En pistas de cortes, nada cambia. Tests en `engine/tests/instrument.rs` (sostenida hasta el note off, release al frame, teclas independientes, misma tecla, robo, rangos), `no_alloc.rs` (notas en vivo y sus note off), `tui/src/record.rs` y `tui/src/tests/instrument.rs` (pads con repeticiones simuladas y por tiempo, con avisos de soltar, MIDI con note on y off a su altura y velocidad, grabación con tono y duración).

- **Salida MIDI de las notas** (punto 6): en una pista con instrumento, `Pattern::with_midi(Some((canal, raíz)))` (el script pone la raíz: `pista:midi(canal)`, y `nota_base` da error con línea). `Engine::play_notes` manda, por cada nota del golpe (también las del acorde), un note on con el frame exacto del golpe (nota = raíz + `round(12·log2(velocidad))`, velocidad = volumen·100) y guarda su note off en `Track::midi_notes` (`MidiHeld { canal, nota, at, born }`, hasta `MIDI_HELD` = 32 por pista; si se llena, se apaga la más antigua) para el final de su paso y sus `_` (`at = golpe + gate`). `midi_due`, tras cada trozo, los manda en orden de frame. Una nota robada (`open_note`, por su `born`), cortada por choke o por `Stop` (`kill_notes`) apaga su nota MIDI en ese frame; la misma nota aún encendida se apaga antes de volver a sonar. En mute o fuera del solo no sale nada; las notas en vivo (pads, MIDI de entrada) no salen, como los pads. El panel de efectos dice `· MIDI 2`. Tests en `engine/tests/midi_out.rs` (altura, acorde, `_`, retrigger, robo, mute, paro, todo al frame), `no_alloc.rs`, `script/tests/instrument.rs` y `tui/src/tests/midi_out.rs`.

- **Diagnóstico en vivo de `melody("...")`** (punto 7): `diagnose::steps_diagnostics` recorre `:steps(` y `:melody(` (también `c:melody` en `every`) igual. Para `melody`, `check_melody` pasa cada nota a un paso con `patterns::note_step(token, None)` (la conversión de `notes`, separada; sin raíz, no comprueba los ±36 semitonos) y subraya la que falla; luego `check_pattern` (común con `steps`, sobre los pasos ya convertidos pero con las columnas y el texto de las notas) mira cuántos pasos hay con la longitud de la pista, los `_` sin golpe (subraya ese `_`) y los rangos de volumen, retrigger y `~` (el pitch no: depende de la raíz). Los avisos salen ordenados por línea y columna. Tests en `tui/src/diagnose.rs` (nota errónea subrayada, acorde de más, `^`, sufijos, rangos, `_`, número de pasos, `:length`, `every`) y `tui/src/tests/instrument.rs` (aviso al pie mientras se escribe).

- **Pitch sin cambiar la duración** (punto 3, `engine/src/shift.rs`): `Shift` (en `Voice` y en `Note`, `Option<Shift>`, solo con `keep` y un pitch distinto de 0) hace que la voz avance por el sample a velocidad 1 (su `length` se calcula con velocidad 1) y lo lea en granos de Hann de 30 ms solapados a la mitad, cada uno a la velocidad del pitch. `Shift::grains(n, fit, mono)` da los dos granos del frame de salida `n` (desplazamiento en el eje de la voz y peso; suman 1) y, al empezar cada medio grano, ancla el siguiente en `k·hop` desplazado hasta ±6 ms al mejor encaje con lo que seguiría leyendo el anterior (correlación normalizada sobre la versión mono; candidatos cada 4 frames, 90 puntos de referencia guardados en la pila, y afinado a 1). El primer grano no tiene fundido de entrada; sin bucle (`fit`), ningún grano se ancla tan tarde que lea más allá del final del corte, y lo que pase del final no se lee. Las voces pasan el desplazamiento a posición del sample con su bucle, ping-pong o sentido (`Voice::position`, `Note::position`) y leen con `interp::read` a la velocidad del pitch (el filtro antialias de siempre). Notación: `%n` (`Step::keep`, `Step::with_kept_pitch`; va en vez del primer `^` y detrás pueden ir las notas del acorde; `^` y luego `%`, error); `Playback::keep` y `NoteOn::keep`. Instrumento: `Instrument::keep` (`keep = true` en `sample:instrument{}`, del registro `api::INSTRUMENT_PARAMS`) para todas sus notas, también las en vivo. Con pitch 0 no se crea el `Shift`: `%0` es bit a bit lo de siempre. `scale:melody` y `melody` no aceptan `%` (el tono lo ponen ellas). Aviso en `Song::warnings` (`far_kept_pitch`, `KEEP_PITCH_LIMIT` = 12) si una pista transpone así más de ±12. El tracker escribe `%` (`model::step_text`). Tests en `engine/tests/pitch_keep.rs` (seno a 440 Hz con +7, −12, +12 y −5: duración < 1 ms de error y frecuencia < 0,5 %; `%0` idéntico con cortes, reverse, retrigger y volumen; reverse; instrumento con y sin `keep`; cada golpe de un break a +7 y −12 con su ataque a menos de 1 ms de su paso y determinista; la medida, ignorada), `engine/tests/notation.rs`, `no_alloc.rs` (pasos con `%` en bucle, reverse y retrigger; instrumento con `keep`, acordes y notas en vivo), `script/tests/pitch_keep.rs` (duración y frecuencia desde Lua, `keep`, `%0`, `every` conserva el `%`, avisos, errores con línea), `tui/src/diagnose.rs` y `tui/src/model.rs`.

- **Granular y congelar** (punto 4, `engine/src/grain.rs`): `Granular { size_ms, density, position, speed, jitter_ms, pitch, spread: Knob, seed }` (`Knob::Fixed` o `Knob::Moving(Curve)`; rangos y valores por defecto en `GRAIN_PARAMS`; `validate` mira también los extremos de las curvas con `automation::check_curve`, y que `density × size` no pase de `MAX_GRAINS` = 32) y `Pattern::with_granular` (`SendError::InvalidGranular`; no con instrumento). Cada pista del motor tiene una `Cloud`: `grains: [Option<Grain>; 32]` (cada grano con su `Arc<Sample>`, que suelta `discard` al acabar) y la `Emission` abierta. `play_hit` en una pista con granos abre la emisión sobre el rango del corte durante `gate` (el paso y sus `_`, o su parte del retrigger) con el pitch, el sentido y el volumen del paso, y cierra la anterior. `Cloud::mix_into` (por frame): si toca, nace un grano con los parámetros evaluados en ese frame de la canción (`Position`: frame, pasos y segundos); su azar es `Random` (splitmix64) con la semilla y el frame de la canción; empieza en `pos · largo + scan ± jitter` (envuelto en el corte, en un frame entero y movido para que el grano quepa si cabe), con velocidad `2^((pitch_paso + pitch ± spread)/12)` (negativa con `'`) y volumen `1/√max(1, densidad·tamaño/2)`; el siguiente nace `rate/density` frames después; `scan` avanza `speed` por frame. Cada grano suma su ventana de Hann y lee con `interp::read` (en frame entero y sin pitch, sin interpolar). Paro y choke (`kill_notes`) cortan la emisión y funden los granos en 1,5 ms; si el patrón nuevo no tiene granos, no nacen más y los que suenan acaban. `TrackStatus::grains`: granos nacidos desde la lectura anterior. Lua: `pista:grains({ ... })` y `pista:freeze([pos [, { ... }]])` (`set_granular`; `freeze` pone `speed = 0`, y `speed` en su tabla es un error), números o `ramp()`/`lfo()` (`knob`), `seed` entero (sin ella, FNV-1a del nombre de la pista); errores con línea (rangos, clave desconocida, tipo, semilla, solape, instrumento). Registro: funciones `grains` y `freeze`, `api::GRAIN_PARAMS` y `api::GRAINS_TABLE` (autocompletado y ayuda de la tabla, tabla en la guía). Interfaz: `TrackView::grains` (`model::grains_text`: `⁘ 80 ms · 20/s`, `❄ 0.4 · 120 ms · 25/s`, `∿` si se mueve) en el título de la pista del panel de efectos. Tests en `engine/tests/granular.rs` (granos por segundo según el motor y en el audio con cuatro densidades, solo durante el paso y sus `_`, congelar lee su sitio y el pitch lo mueve, `speed` recorre el corte, misma semilla = misma salida y otra = otra, 8 nubes al máximo nunca pasan de ±1 con el limitador, rampa de densidad y LFO de pitch, paro, rangos; el coste, ignorado), `no_alloc.rs` (dos nubes con curvas, reverse, retrigger, `_` y choke entre ellas, y quitar los granos con granos sonando), `script/tests/granular.rs`, `script/tests/registry.rs` y `tui/src/tests/grains.rs`.

- **Nube en vivo** (`Cloud::open_live`/`release_live`, `Controller::grain_on`/`grain_off`, `Command::GrainOn`/`GrainOff`; añadido después del punto 8): una tecla abre una `Emission` con `live: Some(Live { key, serial, elapsed })` que dura hasta `HELD` (`u64::MAX`) frames, es decir, hasta que se suelta su tecla (`release_live(key)`: si la nube de la pista es de esa tecla, no nacen más granos y los que suenan acaban su ventana de Hann, sin clic; si es de otra, nada). Una nube por pista: la última tecla manda (el golpe de un paso también sustituye a la de una tecla). Los parámetros son los de la pista, mirados en cada grano como siempre: los del patrón que suena (entran al compás con el patrón nuevo, con la tecla apretada) o, si ese aún no tiene granos o no ha entrado (antes de la primera marcha), los del que espera (`Cloud::standby`, que `put_pattern` borra cuando entra otro); si la pista no tiene granos de ningún tipo, la tecla toca el corte tal cual (una `Voice`). El azar sale de la semilla y de un reloj propio de la nube, `(serial << 40) ^ elapsed` (cuántas nubes en vivo lleva la pista y cuántos frames lleva esta), no de la posición de la canción: tocar lo mismo en el mismo orden suena igual con el secuenciador parado o en marcha y esté donde esté, y cada pulsación suena distinta de la anterior. Sin reservas (`Emission` sin memoria dinámica; el `Arc` del sample vuelve por `discard`; un rango vacío no abre nada); `is_playing` cuenta la nube (la interfaz ya no cree que está parado mientras suena). Pitch: `Controller::grain_on` suma `semitones` (±36) al de los granos. Interfaz (`App::live_grain_on`, `HeldNote::grain`): los pads y las notas MIDI de una pista con `granular` (`selected_granular`) usan la misma ruta de las notas de un instrumento (`held`, repeticiones del teclado, `release_expired_pads`, `key_release`, `release_pads` al salir del modo tocar o cambiar de pista, `live_note_off` del MIDI), con `grain_on`/`grain_off`. La tecla elige **el corte** si la pista tiene varios (el pad `n` es el corte `n+1`, con la forma del corte; la nota MIDI, como en las pistas de cortes: nota − nota base) y **el pitch** si tiene uno solo (el pad `n` son `n` semitonos, la nota MIDI, nota − nota base, también negativos); la velocidad MIDI es el volumen. Grabar lo tocado (`Recording::hit_held`) apunta el corte con `_` mientras se mantuvo (o `1^n` con su duración, con un corte), así que el resultado se pega en la pista tal cual. Tests en `engine/tests/granular_live.rs` (granos por segundo mientras se mantiene y ninguno al soltar, con el secuenciador parado y en marcha; otra tecla sustituye y soltar la primera no corta; el corte y el pitch elegidos y sumados al de la nube; azar igual esté donde esté la canción y con otra semilla distinto; dos pulsaciones distintas pero repetibles; soltar sin clic; el patrón nuevo entra al compás con la tecla apretada; antes de la primera marcha; sin granos toca el corte; paro; 8 nubes ≤ ±1; rangos), `no_alloc.rs` (teclas, sustitución, patrón nuevo, pendiente, paro y quitar el patrón con una nube en vivo), `tui/src/tests/grains_live.rs` (pads por corte y por pitch, con y sin aviso de soltar, MIDI con velocidad, la pista sin granos, recargar con la tecla apretada y grabar) y `tui/src/record.rs`.

- **Formantes** (punto 8, `engine/src/shift.rs`): `Shift::new(rate, sample_rate, formant)` con `formant: Option<f32>` en semitonos; si los formantes irían adonde los lleva el pitch, no hay corrección. `Shift::needed`: hace falta desplazador con pitch o con formantes movidos (así `%0` con `formant(5)` cambia el timbre sin cambiar la nota, y `%0` con `formant(0)` es bit a bit lo de siempre). Al empezar cada grano, `Shift::correction` analiza con LPC (`lpc`: orden 32, autocorrelación con ventana de Hann y Levinson-Durbin en f64, ruido blanco a −90 dB) lo que lee el grano y el mismo tramo del original leído a la velocidad de los formantes (`2^(n/12)`; los dos sobre el mismo trozo de original, que dura al menos un grano: al bajar, la ventana de lo leído se alarga), y guarda `Correction { whiten, color, gain }` (coeficientes de reflexión y `√(error buscado / error leído)`). `Shift::correct` pasa la suma de los dos granos por `Lattice` (FIR en celosía con `whiten`, ganancia, IIR en celosía con `color`; los dos canales a la vez) con los coeficientes del grano anterior y del último mezclados con el fundido de los granos (`mix`, rehechos cada `MIX_EVERY` = 8 frames). La búsqueda del encaje y el análisis leen la versión mono con interpolación lineal (`shift::mono_at`). `Voice::read` y `Note::read` ya no copian el desplazador (`voice_position` y `Axis` leen el sample mientras se toca); las pistas del motor van en el heap (`Engine::tracks: Box<[Track; 8]>`, creadas en `new_engine`), porque con los filtros cada voz ocupa 1,6 KB y el motor entero no cabía en la pila de los tests. `Pattern::with_formant` (para los pasos con `%`; `Playback::formant`, `NoteOn::formant`), `Instrument::formant` (necesita `keep`; `validate`), `shift::check_formant` y `MAX_FORMANT` = 24. Lua: `pista:formant(n)` (en un instrumento, error con la línea: va en su tabla) y `formant = n` en `sample:instrument{}` (con `keep = true`); `TrackDef::formant`; aviso si la pista no tiene pasos con `%` (`idle_formant`). Registro: función `formant` y parámetro `formant` de `INSTRUMENT_PARAMS`. Interfaz: `TrackView::formant`, `· formant 0` / `· formant -3` en el panel de efectos. Tests en `engine/tests/formant.rs` (una vocal sintética, pulsos a 150 Hz por tres resonadores, comparada armónico a armónico con su envolvente analítica: a +7 con `formant(0)` a menos de 2 dB de la original y a más de 8 de la movida, y sin corrección al revés; a −12 también; `%0` con `formant(5)` sigue la envolvente desplazada; el cuerpo de una caja a 900 Hz se queda a ±15 % con la corrección a +7 y −7 y se mueve ×r sin ella; sin `formant`, con `^` y con `%0`, bit a bit lo de siempre; instrumento con y sin `formant` y sin `keep`; la medida, ignorada), `no_alloc.rs` (pasos con `%` y `formant(0)`, instrumento con `keep`, `formant = -3`, acordes y notas en vivo), `script/tests/formant.rs` (el script suena como el motor, instrumento, aviso, errores con línea), `script/tests/registry.rs`, `tui/src/tests/formant.rs` y `tui/src/complete.rs`.

### Decisiones

- **Nube en vivo: qué elige la tecla.** La API ya tenía dos formas de decir
  qué granos nacen: el corte (el número de un paso) y el pitch (`^`, que se
  suma al de la nube). Se usa el corte cuando la pista tiene varios, porque
  es lo mismo que un paso y lo mismo que los pads de siempre; y el pitch
  cuando tiene uno solo, que es lo normal en una textura (`slice(1)`) y
  donde, si no, solo habría una tecla útil, igual que las de un
  instrumento (la primera, tal cual; cada una, un semitono). La posición
  dentro del corte (`pos`) no se elige con la tecla: es un parámetro con
  `ramp`/`lfo` y se toca desde el script, que entra al compás. Se descartó
  una posición por tecla: con un corte, dejaba sin pitch las texturas, y
  con varios, mezclaba dos significados de la misma tecla.
- **Nube en vivo: azar propio.** Con el reloj de la canción, una nube en
  vivo con el secuenciador parado repetiría el mismo azar en cada trozo
  (el frame no avanza) y dependería de dónde esté la canción al pulsar.
  Con el suyo (qué pulsación es y cuánto lleva), es determinista en lo
  tocado y no en el tiempo del reloj.
- **Formantes: LPC por grano, en el tiempo.** Primero se midió qué
  falla sin corrección (`cargo test -p engine --release --test formant --
  --ignored --nocapture`): una vocal subida +7 con `%` sigue la
  envolvente movida ×1,5 (a 0,5 dB) y queda a 13,7 dB de la original; a
  +12, 16,5 dB; a −12, 18,9 dB; el cuerpo de una caja a 875 Hz pasa a
  1375 Hz (+7) o a 575 Hz (−7). Las opciones: PSOLA (pide detectar el
  pitch y falla con ruido y con material polifónico: una caja), envolvente
  cepstral (FFT por voz, más coste y más código) o LPC en el tiempo. Se
  eligió LPC: Rust puro, sin FFT ni memoria dinámica, y encaja en los
  granos que ya había (cada uno se analiza al empezar). Resultado, error
  de los armónicos frente a la envolvente original (y frente a la movida):

  | Semitonos | Sin corrección | `formant(0)` |
  | --- | --- | --- |
  | +7 | 13,7 dB (0,5) | 1,3 dB (14,4) |
  | +12 | 16,5 dB (0,1) | 1,8 dB (17,1) |
  | −5 | 16,1 dB (6,3) | 6,6 dB (9,6) |
  | −12 | 18,9 dB (0,5) | 1,4 dB (19,3) |

  A −5 el propio desplazador no sigue bien esta vocal (6,3 dB frente a
  la movida sin corrección): es su suelo, no el de los formantes.
  `%0` con `formant(+5)` queda a 1,3 dB de la envolvente desplazada y
  con `formant(-5)`, a 0,9. La caja: +7, 850 Hz con corrección (875 el
  original); −7, 950 Hz.
- **Los ajustes, medidos**: el análisis en f32 no vale (una envolvente de
  ~100 dB de rango, como la de la vocal, se queda sin precisión y al bajar
  la corrección no actuaba: 9,6 dB a −12); en f64 con ruido a −90 dB, sí.
  Más ruido blanco (−60, −40 dB), una ventana de retardo gaussiana o
  preénfasis empeoraban la vocal sin arreglar nada. Orden: 16 se queda
  corto (6,3 dB a −12); con 24 la vocal va bien pero una caja con su tono
  de 190 Hz a −7 no se corrige (la línea del tono se lleva los polos);
  con 32, las dos; 40 y 48 no mejoran. Un solo filtro sobre la suma de
  los dos granos, con los coeficientes mezclados, en vez de uno por grano:
  más barato (4,8 % de un núcleo por voz en vez de 5,7 %; sin formantes,
  2,8 %) y mejor a −12 (1,4 dB en vez de 4,5), porque no arranca en frío
  en cada grano. Mezclar coeficientes de reflexión deja el filtro estable.
- **Dónde va**: en los pasos con `%` (`pista:formant(n)`, para toda la
  pista) y en los instrumentos con `keep` (`formant = n` en su tabla), que
  son los que usan granos. Con `^` no: el sampler clásico lee el corte
  entero más deprisa y sus formantes se mueven, como siempre. `0` los deja
  donde estaban; `n` los desplaza aunque el pitch no cambie.
- **Granos en el motor, por golpe.** Una nube no es un modo aparte de la
  pista sino lo que hace cada golpe: el corte dice de dónde salen los
  granos y el paso con sus `_` cuánto dura la emisión, así todo lo de las
  pistas sigue valiendo (`every`, `chance`, `fill`, retrigger, choke,
  `length`, efectos, envíos) y una textura larga es `"3 _ _ _ ..."`. El
  pitch del paso (`^`) se suma al de los granos. En vivo, los pads abren
  una nube mientras se mantienen (ver «Nube en vivo» más abajo).
- **Azar por posición**: cada grano sale de la semilla y del frame de la
  canción en que nace, no de un generador que avanza: la misma canción
  suena igual en cada render y desde cualquier punto, y recargar no lo
  cambia. Sin semilla, la del nombre de la pista: dos pistas con los mismos
  parámetros no se suman en fase.
- **Curvas**: los parámetros admiten `ramp()` y `lfo()` y se miran al
  nacer cada grano (un grano no cambia mientras suena), en escala
  logarítmica `size` y `density`. Como no hay suavizado que hacer, no van
  por las automatizaciones de `:fx` (que tienen 8 huecos por pista).
- **Memoria fija**: 32 granos por pista, en el `Track` del motor desde que
  se crea; la validación no deja pedir más a la vez (`density × size`),
  así que nunca se pierde uno. Cada grano guarda su `Arc<Sample>` (cambiar
  de sample no corta los granos que suenan).
- **Nivel**: `1/√(solape/2)` (con solape > 2): una nube densa con jitter
  suena más o menos como el sample; una congelada sin jitter, donde los
  granos se suman en fase, sale más fuerte y la para el limitador (medido:
  ocho nubes al máximo, volumen 4 de paso y de pista, nunca pasan de ±1).
- **Coste** (`cargo test -p engine --release --test granular -- --ignored
  --nocapture`): 80 ms × 20/s, 0,4 % de un núcleo; 32 granos a la vez sin
  pitch, 3,6 %; los mismos a +12 semitonos, 27,5 % (la sinc antialias
  lee 36 vecinos por grano). Empezar cada grano en un frame entero hace
  que sin pitch se copie el sample sin interpolar (antes, 12,4 %).
- **Pitch sin cambiar la duración: granos alineados en tiempo real.**
  Se midió (`cargo test -p engine --release --test pitch_keep -- --ignored
  --nocapture`) contra la alternativa de reaprovechar el WSOLA de
  `stretch` al cargar (estirar el sample ×2^(n/12) y leerlo a esa
  velocidad):

  | Semitonos | Técnica | Error de frecuencia (seno) | Pureza (seno) | Envolvente del break |
  | --- | --- | --- | --- | --- |
  | +7 | granos | +0,020 % | 41,2 dB | 3,75 dB |
  | +7 | WSOLA al cargar | +0,005 % | 43,4 dB | 4,07 dB |
  | −12 | granos | 0,000 % | 43,5 dB | 3,41 dB |
  | −12 | WSOLA al cargar | −0,014 % | 49,0 dB | 3,88 dB |
  | +12 | granos | −0,001 % | 42,9 dB | 4,32 dB |
  | +12 | WSOLA al cargar | 0,000 % | 40,1 dB | 4,12 dB |
  | −5 | granos | −0,059 % | 40,0 dB | 2,67 dB |
  | −5 | WSOLA al cargar | −0,006 % | 54,0 dB | 3,39 dB |
  | +24 | granos | 0,000 % | 46,0 dB | 4,96 dB |
  | +24 | WSOLA al cargar | −0,005 % | 37,9 dB | 4,36 dB |

  (Pureza: lo que explica una sinusoide ajustada por bloques frente al
  resto. Envolvente: desviación media en dB, cada 2 ms, respecto al break
  sin transponer, golpe a golpe.) Granos sin alinear daban 0 dB de pureza
  y hasta un 9 % de error de frecuencia aparente (los granos se cancelan):
  descartados. Con la misma calidad, en tiempo real gana: sirve para
  cualquier nota (pads, MIDI, acordes) sin una copia del sample por
  semitono ni esperar al cargar, y no gasta memoria; cuesta un 2,6 % de
  núcleo por voz (0,6 % con `^`). Granos de 30 ms y búsqueda de ±6 ms:
  con 20 ms los senos salían algo más puros y el break peor, con 10 ms al
  revés. Reanclar los últimos granos antes del final del corte (en vez de
  dejarlos leer silencio) mejora la cola a +12 y +24 (4,3 frente a
  5,2 dB, y 5,0 frente a 8,6).
- **Golpes**: el primer grano empieza en el golpe y sin fundido, así que
  el ataque de cada corte cae en su paso (medido: a menos de 1 ms a +7 y
  −12) y con el mismo nivel que con `^`. Lo que se oye peor son las colas:
  dentro de un golpe, a ±12 la envolvente se separa 3-4 dB de la original
  (como el WSOLA), y más allá de ±12 un grano lee hasta cuatro veces su
  duración y el sonido se vuelve metálico. Por eso se avisa a partir de
  ±12; el rango sigue siendo ±36, como `^`.
- **Sintaxis**: `%` (corto, libre en la notación y fácil de recordar como
  «tanto por ciento de tono»), en el sitio del primer `^`; `^` sigue siendo
  el pitch de sampler clásico. En un instrumento, la opción `keep = true`
  vale para todas sus notas (`melody`, acordes, pads, MIDI), porque las
  notas se escriben sin sufijos de pitch.
- **Interpolación**: sinc con ventana de Kaiser (8 cruces por cero a cada
  lado, β = 8) en tabla, con el corte escalado a `0,9 / r` al subir el
  pitch. Se eligió midiendo un seno agudo (`engine/tests/interpolation.rs`):
  a +12 y +24 semitonos la proporción es exactamente 2 y 4, las posiciones
  de lectura son enteras y la lineal, la cúbica (Hermite) y una sinc de
  corte fijo dan las mismas muestras; el aliasing que se oye ahí es el
  plegado de lo que pasa del Nyquist (15 kHz +12 → 18 kHz a 0 dB), y solo
  lo quita un filtro que baje con la proporción (-82 dB). Con proporciones
  no enteras (una quinta) la lineal deja imágenes a -36 dB, Hermite a
  -49 dB y la sinc a -102 dB. Lo que cae entre 0,9 y 1 del Nyquist nuevo
  se atenúa solo en parte (banda de transición). Con pitch 0 se lee la
  muestra tal cual (bit a bit); bajando, la sinc pasa por las muestras.
  Cuesta unos `18 · max(1, r)` vecinos por frame y canal: ocho pistas a
  +24 gastan ~14 % de un núcleo. Las huellas de `golden.rs` con pitch son
  nuevas; las mismas canciones sin pitch conservan las de antes.
- **Instrumento como pista**: `track(nombre, sample(...):instrument{...})`
  hace una pista de un solo corte (el sample entero) cuyos golpes son
  notas, así que todo lo de las pistas sirve (`every`, `chance`, `fill`,
  `length`, swing, efectos, envíos, choke). El pitch de cada paso son
  semitonos sobre la raíz; `melody("c e g e")` es azúcar que escribe esos
  pasos (`1^0 1^4 1^7 1^4`). En una pista de cortes `melody` también vale
  (C4 es el corte 1 tal cual, como en `scale`).
- **Duración de las notas**: el paso más sus `_`, un token nuevo de la
  notación que alarga el golpe anterior (en el modo gate también; en
  one-shot no cambia nada). Después, el release. Se eligió `_` (como en
  Tidal) en vez de un sufijo de duración porque se lee en el tracker
  (`│` en las filas alargadas). Dos notas de la misma pista solo se
  solapan por su release o con acordes.
- **Acordes**: varios `^` en un paso (`1^0^4^7`, hasta 4 notas), guardados
  como intervalos sobre la primera, así `c:pitch()`, `fill("pitch")` y
  `chance("pitch")` transponen el acorde entero. En `melody`, `c+e+g`; las
  notas sin octava van justo por encima de la anterior. En una pista de
  cortes un acorde es un error con línea (no hay polifonía).
- **Voces y robo**: por pista, hasta 16, en arrays fijos del motor. Una
  nota nueva sin voz libre roba la más antigua (por orden de llegada), que
  pasa a una cola aparte con el fundido mínimo (1,5 ms) y deja de contar:
  el número de voces que suenan nunca pasa del límite. `Status` publica,
  por pista, el máximo de voces a la vez desde la lectura anterior.
- **ADSR**: rectas calculadas desde el número de frames de la nota (sin
  acumular: exactas y deterministas); cada tramo dura al menos el
  fundido mínimo, y el release empieza desde el nivel en que esté.
- **Bucle sostenido**: `loop = {inicio, fin}` en ms. La nota sigue en
  bucle mientras dure, también en el release; los últimos `crossfade` ms
  antes del fin se funden (linealmente) con los de antes del inicio, así
  el bucle conserva sus puntos exactos. El fundido lineal va bien con
  material tonal (dos trozos parecidos no bajan de nivel); si no cabe
  antes del inicio, se acorta y se avisa. Al revés (`'`) no hay bucle.
- **Raíz**: nota (`"C3"`) o número MIDI; C4 = 60. Sin octava en `melody`,
  la de la raíz.
- **En vivo** (punto 5): la nota la decide la pista seleccionada; en un
  instrumento, los pads son cromáticos desde la raíz (1 = raíz, cada tecla
  un semitono: 18 teclas, octava y media) y el MIDI toca la nota real
  respecto a la raíz (sin `nota_base`). Las notas en vivo llevan su
  sample y su instrumento en el comando, como los pads, así suenan con el
  secuenciador parado y antes de que entre el patrón. Saber cuándo se
  suelta una tecla depende de la terminal: con el protocolo de teclado de
  kitty se usa el evento de soltar; sin él (tmux por SSH, lo habitual
  aquí), la nota dura mientras llegan las repeticiones de la tecla
  mantenida (0,7 s tras pulsar, que cubre el retardo de repetición, y
  0,15 s tras la última). Lo malo: repetir la misma tecla en menos de
  0,7 s alarga la nota en vez de volver a tocarla. No se ha probado en una
  terminal real: la lógica, con instantes simulados. Grabar un
  instrumento escribe su tono y su duración con `_`. Las notas en vivo no
  salen por MIDI (como los pads).
- **Salida MIDI** (punto 6): la nota MIDI es la real (raíz + semitonos)
  y dura lo que el paso y sus `_`, no el release, que es del sample: el
  synth hace el suyo. Robar o cortar una nota la apaga ya, para que el
  synth no se quede con notas colgadas.
- **Diagnóstico de `melody`**: el editor no sabe la raíz del instrumento
  de cada pista (sale de una tabla de Lua que puede ser cualquier
  expresión), así que comprueba todo lo que no depende de ella y deja la
  distancia a la raíz para la carga. Tampoco sabe si la pista es un
  instrumento: un acorde en una pista de cortes lo dice la carga.

## Fase J: EQ y choques de frecuencia entre pistas ✅

Hechos los seis puntos: el efecto `eq`, el análisis de choques, su informe, la prioridad con las sugerencias, aplicarlas desde la paleta y, después, el EQ dinámico con sidechain por bandas.

- **EQ paramétrico** (punto 1, `engine/src/fx.rs`): `Fx::Eq { bands: [EqBand; EQ_BANDS] }` (4 bandas en serie), `EqBand { kind: EqKind (Bell, LowShelf, HighShelf), freq_hz, gain_db, q }`, `Fx::eq(&[bandas])` (las que faltan, `EqBand::flat(i)`: ganancia 0 y la frecuencia de `EQ_DEFAULT_FREQS` = 100, 500, 2000 y 8000 Hz, q 1). Rangos `EQ_FREQ_RANGE` (20-20000 Hz, logarítmico), `EQ_GAIN_RANGE` (±24 dB) y `EQ_Q_RANGE` (0,1-18, logarítmico) en `validate` y `param_range`. Sus 12 parámetros automatizables son `freqN`, `gainN` y `qN` (índice `3·(N−1) + 0/1/2`); `MAX_PARAMS` = 12 es ahora el tamaño de `Track::mods` (antes 2 por efecto). Coeficientes del Audio EQ Cookbook (`Biquad::eq`: campana y estantes con `alpha = sin(w0)/(2q)`; el agudo es el grave con `cos` cambiado de signo), forma directa II transpuesta (`Biquad::run`). Estado: `FxState::Eq { bands: [EqState; 4], sample_rate }`, cada banda con `target` (lo del patrón o la automatización), `current` (lo que suena), coeficientes y memoria. Una banda con ganancia 0 y sin camino hacia otra no se procesa: un EQ sin ganancias es bit a bit lo mismo que no tenerlo. Al entrar otro EQ en la misma ranura (recarga al compás), cada banda de la misma forma conserva `current` y memoria y va hacia el destino con un polo de 5 ms, recalculando los coeficientes cada 32 frames (`EQ_BLOCK`; ganancia lineal en dB, frecuencia y q en escala logarítmica; a menos de una milésima, llega). Al llegar a ganancia 0 vacía la memoria y deja de procesar. La automatización (`ramp`, `lfo`) ya llega suavizada por `modulate` y entra tal cual (`set_param` pone destino y valor). En silencio (`reset`), todo llega ya. Lua: `fx("eq", { freq1, gain1, q1, type1, ... })` (`parse_fx`; `typeN` es `"bell"`, `"lowshelf"` o `"highshelf"`); registro: efecto `eq` con sus 16 parámetros (macro `eq_params!` en `api.rs`) y `automatable`. Interfaz: `model::fx_text` escribe `eq 300 -3.0 ≥8k +2.0` (`hz_label`; `≤`/`≥` los estantes) o `eq plano`. Tests en `engine/tests/eq.rs` (ganancia medida con senos en régimen: campana +6 dB a ±0,2 dB en el centro y ±0,3 dB a 50 Hz y 16,7 kHz, media octava entre 2 y 5,5; corte −4 dB con q 4 y nada a una octava; estantes grave +6 y agudo −9 dentro y 0 lejos; varias bandas a la vez; plano bit a bit, también al entrar al compás; una ganancia nueva al compás llega suavizada, sin escalón mayor que el del propio seno; un LFO en `gain1` y determinista; rangos), `no_alloc.rs` (EQ con LFO en una banda, estante agudo y un cambio de ganancias al compás que suaviza), `script/tests/eq.rs` (igual muestra a muestra que el motor, valores por defecto, `ramp`/`lfo`, errores con línea) y `script/tests/registry.rs` (que ahora pone cada valor de las listas cerradas en su propia pista).

- **Análisis de choques** (punto 2): la parte de señal en `engine/src/masking.rs` (pura, sin motor ni Lua; `rustfft`, que ya usaba el detector de cortes): `BandSpectrum::analyze(audio, rate, opciones)` mezcla a mono y saca la potencia de 30 bandas de tercio de octava (`BANDS`, de 25 Hz a 20 kHz, centros `1000·10^(k/10)`, nombres nominales en `BAND_NAMES`, bordes a ±1/6 de octava) en ventanas de Hann de 50 ms cada 25 ms, con la FFT rellenada hasta al menos 8192 puntos (bins cada 5,9 Hz a 48 kHz) y el bin más cercano en una banda sin ninguno; por encima de Nyquist, cero. `BandSpectrum::analyze_cycle(audio, ciclo, ...)` pone ventanas solo donde empiezan dentro del ciclo y las últimas leen el audio que sigue. `clashes(a, b, opciones)` da por banda `Option<BandClash { score, overlap, level_db }>`. Criterio y valores por defecto (`MaskingOptions::default()`): una pista está activa en una banda y una ventana si su nivel no baja más de `active_range_db` = 30 dB de su propio máximo (en cualquier banda y ventana; una pista muda nunca lo está); si las dos lo están, su cercanía es `max(0, 1 − |La − Lb| / level_gap_db)` con `level_gap_db` = 9 dB; la puntuación es la media de la cercanía ponderada por la potencia de las dos en cada ventana (de 0 a 1); una banda en la que una de las dos casi no suena (su energía queda más de `empty_band_db` = 30 dB por debajo de la de su propia banda más fuerte) es `None`; se informa desde `report_score` = 0,25. Si una pista es más corta, lo que falta es silencio. En el script, `script/src/masking.rs`: `Song::analyze_masking(rate, opciones)` renderiza cada pista en solo con `Song::render_track` (con sus efectos, envíos y el limitador, como el remuestreo) durante `length_bars(1)` compases y uno más (para que las últimas ventanas del ciclo lean cómo vuelve a empezar y no un corte en seco) (el ciclo entero: el mínimo común múltiplo de las vueltas de cada pista, o la línea de tiempo), con el tope de `MAX_SONG_BARS` = 256 de la exportación, y puntúa cada pareja: `MaskingReport { bars, tracks, options, pairs: Vec<PairClashes> }`, con `score(a, b, banda)`, `clashes()` (los que pasan del umbral, de más a menos), `summary` («0.82 fuerte: a la vez el 95 % del tiempo, bass 3 dB más fuerte»), `table()` (pareja × banda) y `text()`. Tests en `engine/tests/masking.rs` (bandas, ventanas, dos senos a 300 Hz chocan en 315 Hz, 100 Hz y 5 kHz no, la puntuación baja con la diferencia de nivel —la mitad a 4,5 dB, nada a 12— y si no suenan a la vez, pista muda, más corta, banda casi vacía, un chasquido a la vez en bandas donde ninguna suena) y `script/tests/masking.rs` (las mismas parejas desde Lua y un ciclo de 3 compases con `every(3)` que da 0,5).
- **Informe** (punto 3): `tui --script canción.lua --analyze-masking` (`main::masking_report`: carga con un `ScriptHost` y un motor sin dispositivo, a 48 kHz, imprime los avisos de la carga y `MaskingReport::text()`; no se combina con `--render`, `--track`, `--bars` ni `--allow-audio`). En la interfaz, la acción de la paleta «Choques de frecuencia» (`Action::AnalyzeMasking`, `App::open_masking`) analiza lo último cargado bien (a la frecuencia del motor; avisa si el texto ha cambiado) y abre `MaskingPicker { report, clashes, query, selected }`, que se dibuja con el popup de los bloques y las versiones (`ViewModel::masking: Option<BlockPickerView>`): una fila por choque (`a × b  315 Hz` y la puntuación), el resumen del elegido y la tabla pareja × banda como vista previa; escribir filtra por pareja o banda, Esc cierra. Sin choques, o con una sola pista, solo lo dice en la línea de estado. Va en el hilo de la interfaz, como `exportar`: con una canción larga se nota. Tests en `tui/src/tests/masking.rs` (dos senos a 300 Hz: informe con `a × b  315 Hz`, resumen, tabla, filtro y Esc, y el motor suena igual que el de otra interfaz que no ha analizado; 100 Hz y 5 kHz y una sola pista, solo el mensaje; la orden da el mismo informe) y la tabla de la paleta de `docs/GUIA.md`.
- **Prioridad y sugerencias** (punto 4): `priority({ ... })` en Lua (global, fuera de las secciones; `Song::priority`, `Builder::priority`; errores con línea: nombre repetido, algo que no es texto, lista vacía; aviso en `Song::warnings` si un nombre no es de ninguna pista; registro: función `priority`). En `script/src/masking.rs`, `MaskingReport::winner`/`loser` deciden quién cede en cada choque: la de más prioridad; una de la lista sobre una que no está; sin lista, la de más energía en la banda (`level_db`), y a menos de 0,5 dB, la que se definió antes. `suggest()` (al analizar, en `MaskingReport::suggestions`): por cada pista que cede, el choque más fuerte de cada banda; las bandas seguidas son un solo `Cut` en campana (centro geométrico de los bordes; una banda sola, su frecuencia nominal, `nominal`), Q = centro / ancho (4,3 una banda, sin bajar de `MIN_CUT_Q` = 0,7), ganancia de −2 dB (`MIN_CUT_DB`, puntuación en el umbral de informe) a −4 dB (`MAX_CUT_DB`, desde `FULL_CUT_SCORE` = 0,75) en medios dB, contra qué pistas; como mucho `MAX_CUTS` = 4 (las bandas del EQ), los de más puntuación, de grave a agudo. `eq_table`, `lua_line` (`track("b"):fx("eq", { freq1 = 315, gain1 = -4, q1 = 4.3 })`), `cut_lines` y `suggestions_text` (en `text()`, y el resumen de cada choque dice quién cede). Tests en `script/tests/masking.rs` (cede la más baja; con `priority`, la otra, también nombrando solo una; cinco zonas que chocan dan 4 recortes de 2 a 4 dB en medios dB, y su línea carga; errores y aviso de `priority`) y `script/tests/registry.rs`.
- **Aplicar** (punto 5): en el informe de la paleta, la vista previa empieza por la sugerencia de la pista que cede en el choque elegido (`MaskingReport::loser`, `lua_line`, `cut_lines`). Enter pone `MaskingPicker::confirm` (la pista) y la línea de confirmación («Enter otra vez inserta el EQ de «b» en el script (sin guardar) · Esc cancela»; si la pista ya tiene `MAX_FX` efectos en lo último cargado, `App::fx_count`, lo avisa); Esc, moverse o escribir la cancelan sin cerrar. El segundo Enter llama a `App::apply_suggestion`: busca la línea que define la pista (`track("b",` fuera de un comentario; si no, la primera con `track("b"`), salta las líneas de su cadena (las que empiezan por `:`) e inserta debajo, con su sangría, `track("b"):fx("eq", {...})` en un solo cambio (`grouped`: Ctrl-Z lo deshace de una vez); sin definición, al final. No guarda ni carga. Tests en `tui/src/tests/masking.rs` (el primer Enter no cambia el texto, Esc y moverse cancelan, el segundo inserta tras la definición y antes de lo que sigue, el archivo no cambia, Ctrl-Z y Ctrl-R, y al cargar la puntuación de 315 Hz baja más de 0,3; una pista definida en varias líneas dentro de un `if`, con su sangría, y carga) y `script/tests/masking.rs` (con la línea sugerida, la puntuación baja más de 0,3 y el choque más fuerte también). La pantalla, con `cargo test -p tui pantalla_choques -- --ignored --nocapture`.

- **EQ dinámico y sidechain por bandas** (punto 6, de las ideas de `CLAUDE.md`): `Fx::DynEq(DynEqParams { kind, freq_hz, q, threshold_db, ratio, range_db, attack_ms, release_ms, sidechain: Option<usize> })` en `engine/src/fx.rs` (`DynEqParams::new(freq)`: q 1, −30 dB, 4:1, 6 dB, 5 ms y 120 ms; `with_sidechain(pista)`; rangos en `validate`; automatizables `freq`, `q`, `threshold` y `range`). `DynState`: un detector (paso de banda RBJ en `freq` y `q`, `Biquad::bandpass`, sobre la media de los dos canales de la señal que escucha) con envolvente de pico (ataque y release por muestra), y cada `DYN_BLOCK` = 8 frames la reducción `min((nivel − umbral)·(1 − 1/ratio), range)` mueve una campana (o estante) en la misma frecuencia (`retune`). Sin reducción no se procesa (por debajo del umbral, bit a bit lo de siempre), pero al volver a 0 la campana plana sigue filtrando `DYN_SETTLE_SECONDS` = 50 ms para que su memoria se asiente: saltársela en seco dejaba un escalón medible (la campana va unos ms por detrás de la reducción). Con recarga al compás conserva envolvente, reducción y memoria. Sidechain en el motor (`engine/src/engine.rs`): `mix` mira qué pistas escuchan a cuáles (`Fx::sidechain`), las ordena con `mix_order` (cada una después de las que escucha; sin sidechain, 0..8 como siempre, así que la suma del master no cambia ni un bit) y guarda la salida de las escuchadas (tras sus efectos, antes del volumen, el mute y el solo) en `Engine::side_buf` (8 buffers de un trozo, en el heap, creados en `new_engine`); `run_fx` pasa a cada efecto su trozo con `FxState::process_keyed` (en `modulate`, el trozo de cada bloque de automatización; si la escuchada calla en ese trozo, `SILENCE`). Script: `fx("dyneq", { freq, q, type, threshold, ratio, range, attack, release, sidechain })` (`parse_fx`; `sidechain` es el nombre de una pista y se guarda en `TrackDef::sidechains` (índice del efecto, nombre); `track_pattern(def, slots)` lo pasa a la ranura del motor; `check_sidechains` al acabar: la pista existe y no hay círculos; escucharse a sí misma, error con línea). Registro: efecto `dyneq` con sus 9 parámetros y `automatable`. Interfaz: `model::fx_text_named` (`dyneq 80 -6 ⟵kick`). Guía: receta «Que el break deje sitio al bombo». Tests en `engine/tests/dyneq.rs` (sobre el umbral, −10,5 dB con 14 de más a 4:1 y tope en `range`; por debajo y fuera de su banda, bit a bit; con sidechain, −6 dB durante cada golpe del bombo y 0 entre golpes, sin escalones mayores que los del seno, y con la clave callada, idéntico; mismo resultado con la clave antes o después y en cadena; rangos), `no_alloc.rs` (propio y con sidechain), `script/tests/dyneq.rs` (igual que el motor, clave definida después y en mute, `ramp`, errores) y `script/tests/registry.rs`. Probado con `amen.wav`: los graves del break bajan unos 4,7 dB en los golpes del bombo y vuelven entre ellos.

### Decisiones

- **Sidechain = la salida de la otra pista tras sus efectos y antes de su volumen**: es lo que se oye de ella sin depender de su fader, y sigue mandando en mute o fuera del solo (el «bombo fantasma», y el análisis de choques y el remuestreo, que renderizan en solo, oyen el efecto igual que la mezcla). Para tenerla en el mismo trozo, sin un trozo de retraso que dependería del tamaño de bloque, las escuchadas se mezclan antes; los círculos no tienen orden posible y el script los rechaza.
- **Una banda por `dyneq`**: varias bandas dinámicas son varios efectos (hasta 4 por pista), cada una con su umbral y su clave; así la tabla de parámetros es corta y cada uno se puede automatizar.
- **Aplicar inserta texto, no cambia el modelo**: el script sigue siendo lo que manda (el estado sale de él), así que la sugerencia entra como una línea más que se ve, se puede retocar y se deshace con Ctrl-Z; nada suena distinto hasta cargarla. Va como una sentencia aparte (`track("b"):fx(...)`) tras la definición de la pista, no dentro de su cadena: no depende de cómo esté escrita la cadena, y dentro de una sección queda en esa sección.
- **Por defecto manda la más fuerte en cada banda**, no la más fuerte en total: un bombo y unos platos que chocan en los agudos (por el chasquido del bombo) dan el recorte al bombo en los agudos y a los platos en los graves, que es lo que se haría a mano. Con `priority` se impone el orden en todas las bandas.
- **Recortes de 2 a 4 dB y en la pista que cede**: el análisis no oye, así que sugiere poco (lo justo para que la otra asome) y en una sola pista por choque; el usuario decide si basta. Las bandas seguidas en un solo recorte más ancho, para no gastar las 4 bandas del EQ en una sola zona.

- **Cercanía gradual en vez de «a menos de N dB, sí o no»**: con un corte fijo, recortar 3 o 4 dB a una de dos pistas igual de fuertes no cambiaría la puntuación, y la sugerencia parecería inútil. Con la rampa, la puntuación cuenta cuánto se pisan: a 9 dB una ya tapa a la otra y deja de ser un choque.
- **Umbral de actividad relativo al máximo de cada pista** (30 dB), no a un nivel absoluto: un charles flojo choca con otro charles flojo igual que dos bombos fuertes, y la fuga de la ventana o la cola casi muda de un golpe no cuentan. Bandas en las que una de las dos casi no suena (30 dB por debajo de su propia banda más fuerte) fuera: al principio el umbral era de 40 dB sobre la energía de la pareja, y dos ataques a la vez (o el corte en seco del final del render) daban choques «fuertes» en bandas donde ninguna tenía nada propio, solo el chasquido: con la energía como peso, esas pocas ventanas lo eran todo. Para chocar, las dos tienen que sonar ahí de verdad.
- **Ponderado por energía**: los momentos fuertes pesan más que las colas; por eso una pista que suena la mitad del tiempo que la otra da 2/3, no 1/2.
- **Ventanas de 50 ms con la FFT rellenada a 8192**: la ventana corta sigue bien los golpes de un break, y el relleno da al menos un bin a cada tercio de octava; por debajo de unos 60 Hz la resolución real es la de la ventana (~20 Hz), así que en los graves más bajos las bandas vecinas se emborronan (lo dicen la guía y el informe).
- **El análisis en el script**, no en el motor: necesita renderizar la canción (`Song`), y la parte de señal (`engine::masking`) queda aparte y probada con senos sin Lua.

- **EQ en el mismo `Fx`**, con parámetros planos (`freq1`, `gain1`...) en vez de una tabla de bandas: así cada uno es un parámetro más para `ramp`, `lfo`, el autocompletado y la guía, sin un segundo mecanismo. El coste es `MAX_PARAMS` = 12 en `Track::mods`, que sigue siendo de tamaño fijo.
- **Suavizado dentro del EQ** para los cambios de patrón (lo que no pasa por `modulate`): el filtro y el compresor saltan al recargar, pero un EQ que pasa de 0 a −6 dB en una banda estrecha se oiría. La automatización no se suaviza dos veces.
- **Bandas con ganancia 0 fuera del camino**: aunque la campana con 0 dB es la identidad en teoría, en coma flotante no lo es bit a bit; saltársela garantiza las huellas y no gasta CPU.

## Fase L: editor en ventana propia ✅

Hechas las dos partes: la separación en dos procesos y las vistas en el hueco del editor, con el cambio de ventana en caliente.

- **Transporte** (`tui/src/link.rs`): socket Unix en `link::runtime_dir()` (`$XDG_RUNTIME_DIR/sampler`, o `<tmp>/sampler-<uid>` sin él; `private_dir` la crea 0700, comprueba que es del usuario y la cierra si está abierta de más) con un nombre por script (`socket_path`: el nombre del archivo filtrado y un FNV-1a de 64 bits de la ruta canónica, estable entre binarios). `Server::bind` lo pone 0600; si ya hay otra principal viva en él, error, y si es de una que se cayó, lo sustituye; al soltarlo se borra. `Server::accept` mira el uid del otro extremo (`SO_PEERCRED` en Linux, `getpeereid` en el resto, por `libc`) y cierra las conexiones de otro usuario; un editor nuevo sustituye al anterior, que recibe `adios`. `Client` (el editor) reintenta conectar cada `RETRY` = 1 s, salvo tras un `adios` (otro editor, otra versión). `Conn`: lectura y escritura sin bloquear, juntando líneas enteras; un extremo que no lee (más de 8 MiB pendientes), una línea demasiado larga o algo que no se entiende sueltan esa conexión y nada más.
- **Protocolo** (versión `link::VERSION` = 3; la 2 añade el traspaso del editor, ver «Cambiar en caliente» en la parte 2, y la 3, `suena`, lo que suena para resaltarlo en el código, ver la fase M): una línea por mensaje, el verbo y campos separados por tabuladores con `\\`, `\t`, `\n` y `\r` escapados (texto y no JSON: sin dependencias y legible con `socat`). Del editor (`ToMain`): `hola` (versión), `texto` (revisión y texto, sin cargar; como mucho cada 100 ms mientras se escribe), `cargar` (revisión y texto), `accion` (el nombre de la variante de `palette::Action`) y `orden` (una línea de F2). De la principal (`ToEditor`): `hola` (versión, ruta del script, carpeta de samples, revisión y texto), `texto` (revisión del editor de la que parte y texto), `error` (el de la última carga o vacío), `aviso` (su mensaje), `estado` (marcha, BPM, compás, paso, pista y corte seleccionados), `etiquetas` (las que conoce, para el autocompletado y el diagnóstico), `suena` (desde la 3: la revisión del editor y un campo por token que suena, `fila,inicio,fin,color,seleccionada`) y `adios` (motivo). La principal manda solo lo que cambia, y tras cada carga el error y el aviso aunque sean los mismos; el texto, lo último, para que el aviso de un choque no lo tape su mensaje.
- **Dos roles de la misma `App`** (`tui/src/app/split.rs`, `App::link: Option<Link>`; sin él, `tui --edit` es exactamente lo de antes): `App::serve` (la principal: modo tocar fijo, `check_external` apagado porque el archivo es del editor) y `App::open_editor` + `attach` (el editor: abre el script o el de un proyecto sin cargarlo; su `Controller` es de un motor que no se mueve nunca). Ganchos en `app.rs`: `load` (en el editor manda `cargar`), `run_action` (`link_action`: en el editor, las acciones de `editor_side` se hacen aquí, `LabelSlice` pregunta aquí y las demás se mandan; en la principal, las de `needs_the_editor` —deshacer, comentar, insertar bloques, ranuras, guardar...— se rechazan y `PlayMode` también), `run_command` (`link_command`: en el editor, todo menos `bloque` va a la principal; en la principal, `abrir` no), `request_quit` (`quit_main`: con el editor conectado sale sin preguntar; sin él, pregunta si el texto no es el del disco), `known_labels` (las de la principal) y `tick` (`poll_link`). `ViewModel::link` lleva la línea del enlace; en el editor, `transport` es el de la principal. `ui::draw_editor_window` dibuja el editor entero, la línea del enlace y la de ayuda; los popups, compartidos con `ui::draw` (`draw_popups`).
- **Texto en los dos sentidos con revisiones**: el editor numera cada envío; la principal guarda la última revisión recibida y su texto (`synced`), y si su texto cambia (etiquetar, `bpm`, EQ sugerido, grabar, versiones) lo manda con esa revisión como base. El editor lo acepta solo si la base es su última revisión y no ha escrito nada desde entonces, y entonces sustituye el texto en un solo cambio (`grouped`; Ctrl-Z lo deshace, el portapapeles interno y la línea del cursor se conservan); si no, se queda el suyo, avisa y se lo manda. Al conectar por primera vez, el editor coge el texto de la principal si no ha escrito nada desde que se abrió (así un editor nuevo recupera lo que no estaba guardado); al reconectar, se queda el suyo y se lo manda (sin cargar).
- **Lanzadores** (`tui/src/main.rs`): `--edit x --split` escucha antes de abrir la interfaz (si ya hay una principal de ese script, el error sale normal) y abre el editor con `[editor] terminal` de `config.toml` (`Config::editor_terminal`, un texto que se parte por espacios o una lista) seguido de `<este binario> --window-editor <script o proyecto>` (`app::editor_command`; el hijo se recoge con `try_wait`). La acción de la paleta «Abrir el editor en su ventana» (`Action::EditorWindow`, `App::launch_editor`) lo vuelve a abrir; sin terminal configurada dice el comando para abrirlo a mano. `--window-editor x` no se combina con nada (ni `--allow-audio` ni `--allow-midi`: son de la principal). Las dos ventanas comparten `run_terminal` (el bucle de siempre con la función de dibujo de cada una), así que el pegado con corchetes y los eventos de soltar tecla se piden y se quitan igual (guarda `TerminalModes`, también con error o pánico).
- Tests: `tui/src/link.rs` (cada mensaje ida y vuelta con tabuladores, saltos, barras y UTF-8; basura sin pánico; líneas partidas y cuelgues con `UnixStream::pair`; el uid del otro extremo; carpeta 0700 y socket 0600, nombre por ruta, segunda principal rechazada, socket viejo sustituido; servidor y cliente: sustitución con `adios`, caída de la principal y reconexión) y `tui/src/tests/split.rs` (dos `App` por el socket de verdad, con el motor de la principal movido offline: el editor coge el texto y sigue compás, paso, pista y corte; una carga desde el editor a mitad de compás entra en el siguiente —el corte que suena cambia justo en el paso 0—; un error se ve en el editor con su línea y sigue sonando lo anterior; el editor se cierra, la principal sigue y uno nuevo recupera el texto sin guardar; cae la principal, el editor guarda solo y se reconecta solo quedándose con su texto; un cambio de texto de la principal llega y se deshace de una vez, y si choca con lo escrito gana lo escrito; acciones reenviadas y rechazadas; pegar varias líneas con sangría y un Ctrl-Z; segundo editor y otra versión del protocolo; el lanzador con una «terminal» de prueba), `config.rs` (`[editor] terminal`) y la tabla de la paleta de `docs/GUIA.md`. Las dos ventanas, con `cargo test -p tui pantalla_ventanas -- --ignored --nocapture`.

### Decisiones

- **La misma `App` con dos roles** en vez de partirla en dos tipos: el editor separado conserva todo lo de la fase A sin duplicar nada (deshacer, plantillas, autocompletado, ayuda, bloques, ranuras, versiones) y `tui --edit` no cambia; el coste es un puñado de ganchos en `app.rs` y una lista de qué acción va a qué lado (`editor_side`, `needs_the_editor`).
- **El editor no evalúa el script**: lo único del modelo que necesitan el autocompletado y el diagnóstico son las etiquetas y la carpeta de samples, y las manda la principal. Así no hay dos evaluaciones (ni dos decodificaciones de samples) ni un segundo motor sonando en silencio, y el error que se ve es el de la carga de verdad.
- **Sin hilos**: los dos extremos se atienden sin bloquear en cada vuelta del bucle de la interfaz (30 ms; 2 ms con MIDI). El hilo de audio no sabe nada del socket, así que ni una caída ni un extremo lento lo tocan; el motor no cambia (`no_alloc.rs` y `golden.rs` igual).
- **Un socket por script** (no uno por proceso): `--window-editor x.lua` encuentra la principal de ese script sin pasar rutas de sockets, y se pueden tener dos canciones abiertas a la vez.
- **Gana lo escrito en el editor** cuando la principal cambia el texto a la vez: perder un cambio de la principal (que se rehace con una orden) es mucho menos grave que perder lo que se acaba de escribir.
- **Las acciones con diálogo propio se abren en la principal** (explorador, tap tempo, choques, grabar...): llevarlas al editor obligaría a duplicar su estado por el protocolo; cambiar de ventana para usarlas es poco a cambio.

### Parte 2: vistas en el hueco del editor

Hechos los seis puntos: layout, forma de onda grande, tracker con todas las pistas, mezclador, tabla de choques y cambiar en caliente.

- **Layout sin editor** (`ui::draw_main_window`, cuando `ViewModel::editor_outside`, que solo pone la principal con el editor fuera, `App::link_view`): arriba una línea con el transporte (`transport_line`, la misma del panel del motor) y, a la derecha, el archivo y el enlace (`StatusLine`); la línea de tiempo si la hay; la forma de onda grande con al menos un tercio del alto; debajo, el tracker con sus 16 filas; los efectos y la ayuda abajo, como siempre. Con una sola ventana, `ui::draw` es el de siempre (las mismas funciones de la onda y los números, sacadas de `SamplePanel`: `draw_wave`, `slice_numbers`, `sample_title`).
- **Forma de onda grande** (`ui::BigSamplePanel`): de lado a lado, con una fila propia para los números de los cortes (y sus etiquetas) para que no tapen la onda: el que suena, negro sobre el acento, y el elegido, negro sobre el color de los cortes; las marcas cruzan la onda; al pie, dónde empieza cada corte en ms (del sample original con stretch), los que quepan sin pisar al anterior. Con menos de 5 filas, como el panel pequeño. Tests en `tui/src/tests/main_window.rs` (la ventana entera con las dos `App` por el socket: transporte arriba, sin el texto, sample de lado a lado; con una sola ventana, el editor a la izquierda y el sample en la columna 60 como siempre; el panel suelto: fila de números con sus colores, la onda debajo sin números, los ms al pie, y bajo).
- **Tracker con todas las pistas** (`ui::FullTrackerPanel`, a la derecha del panel del motor, con el 60 % del ancho): las columnas se ensanchan o estrechan para que quepan todas (`FullTrackerPanel::columns`: de 6 a 16 celdas; antes, 11 fijas, y las que no cabían se cortaban por el borde); si ni a 6 caben, se dibujan las que caben y el título dice «6 de 8 pistas». Las filas son las mismas que en el tracker de una ventana (`render_tracker`, con el ancho de columna y cuántas pistas como parámetros; `TrackerPanel` lo llama con 11 y todas, igual que antes). Tests en `tui/src/tests/main_window.rs` (anchos según pistas y sitio; ocho pistas en 70 de ancho, cada cabecera en su columna, el golpe del paso 0 en todas y la fila que suena resaltada entera; sin sitio, «6 de 8 pistas» y seis columnas; el tracker de una ventana sigue con 11; en la ventana principal, el título nuevo, y con una sola ventana, el de siempre).
- **Mezclador** (`ui::MixerPanel`, a la izquierda del tracker, donde estaba el panel del motor: el transporte va en la línea de arriba): una tira de 9 celdas por pista y otra para el master (`MixerPanel::width`, hasta la mitad del ancho). Cada tira: el nombre (con el acento si es la seleccionada, tenue si no se oye), un medidor vertical de dos columnas con octavos de celda (`ui::vertical_meter`; lo que no llega, una guía `·`; rojo si satura, tenue si no se oye), el nivel en dB, el volumen, `M` y `S` (resaltados si están puestos) con `SAT`, y los envíos (`d0.30 r0.15`). El master, con su pico antes del limitador (amarillo si limita, rojo si satura) y `LIM`/`SAT`. Si no caben todas, las que quepan más el master y el título `mezclador 6/8`. Lee lo mismo que el panel de efectos (`TrackView::level`, `gain`, `mute`, `solo`, `sends`, `clipping` y `MasterView`): nada nuevo del motor. Con una sola ventana, el panel del motor de siempre. Tests en `tui/src/tests/main_window.rs` (el medidor por octavos y por encima de 1; las tiras: llenado a la mitad, entero y a un cuarto, color de saturación, dB, volumen, mute resaltado, `SAT`, `LIM`, envíos y la seleccionada; sin sitio, `mezclador 1/2` y el master; en la ventana principal, mezclador a la izquierda y tracker a su derecha, sin panel del motor, y con una sola ventana, el panel del motor y sin mezclador).
- **Tabla de choques** (`ui::ClashPanel`, a la derecha de la forma de onda grande): la del último análisis de choques (`App::last_masking`, que guarda `open_masking` con el informe, también si no hay choques, y cuántas cargas se llevaban; `ViewModel::clash_table`, `ClashTableView`). Las celdas salen de `MaskingReport::grid` (bandas que chocan y una fila por pareja con `masking::Cell`: `Empty`, `Quiet` o `Clash(puntuación)`), que también usa `MaskingReport::table`, así que el texto de `--analyze-masking` y el panel no se separan. Puntuación en rojo si es fuerte, amarillo si es media y sin color si es leve (`masking::strength`, la misma de `summary`); las parejas de la pista seleccionada, con el acento. Ancho (`ClashPanel::width`): la tabla entera si cabe en dos quintos de la fila; si no, la lista de choques del más fuerte al más débil (pareja, banda, puntuación; los que no caben, «… y N más»); al menos 28, y nada si al sample le quedarían menos de 40. `App::loads` cuenta las cargas buenas y los cambios de variante: si no coincide con el del análisis, el borde va en el color de aviso y abajo pone «⚠ de una carga anterior» (se analiza otra vez para ponerla al día; analizar en cada carga costaría segundos de render offline por pista). Con una sola ventana, nada cambia (el informe sigue en su popup). Tests en `tui/src/tests/main_window.rs` (la tabla con sus colores y la pareja de la seleccionada; anchos; la lista estrecha y «… y 2 más»; el aviso y «sin choques entre 3 pistas»; en la ventana principal, nada sin analizar, la tabla a la derecha del sample tras la paleta y Esc, y el aviso tras otra carga; con una sola ventana, sin panel).
- **Cambiar en caliente** (`tui/src/app/split.rs`): «Abrir el editor en su ventana» (`Action::EditorWindow`) en una sola ventana ya no pide `--split`: `App::detach_editor` escucha en el socket del script (`link::runtime_dir`, o `App::socket_dir` en los tests), hace de esta la principal (`serve`, con `[editor] terminal`, que ahora guarda `set_config`, y `current_exe`) y lanza el editor como siempre (`launch_editor`; sin terminal, dice el comando). El estado del editor (`link::Handover`: texto, cursor y los textos enteros de lo que deshace y rehace, sacados de una copia del editor y sus grupos con las mismas `undo_in`/`redo_in` de `App::undo`/`redo`, como mucho `HANDOVER_STEPS` = 200 pasos y 4 MiB) espera en `MainLink::pending` y sale tras el saludo al primer editor que se conecte (`historial`); el editor se lo queda (`App::take_handover`) solo si su texto es el mismo, es decir, si acaba de coger el de la principal. «Volver a meter el editor en la principal» (`Action::EditorBack`): en el editor, `return_to_main` manda su `Handover` (`vuelvo`), espera a que salga (`Conn::drain`, hasta 2 s) y cierra la ventana (`Flow::Quit`; si no hay principal o no sale, se queda y lo dice); en la principal, `reclaim_editor` se lo pide al editor conectado (`devuelve`: contesta igual y se cierra, `App::leaving`, que mira el bucle de `main.rs`) o, sin editor, vuelve con su texto. Al llegar, `take_editor_back` suelta el enlace (el `Server` borra el socket; el hijo se recoge en otro hilo para no esperar aquí), se queda con el estado, toma el disco como lo guardado (lo guardó el editor) y vuelve al modo editor y al layout de siempre. `take_handover` rehace el historial desde el texto más antiguo con un cambio agrupado (`grouped`) por paso que solo toca lo que cambia (`change_lines`: lo de en medio entre lo común del principio y del final), deshace los de rehacer y pone el cursor; el portapapeles interno se queda. Tests en `tui/src/tests/swap.rs` (sacarlo de una sola ventana: el editor de fuera tiene el texto sin guardar, el cursor, lo que rehace Ctrl-R y lo que deshace Ctrl-Z paso a paso hasta el principio; volver desde el editor con lo escrito allí: el mismo texto, cursor e historial, sin guardar, el socket borrado, el layout de siempre y se puede volver a sacar; la principal lo pide y el editor se cierra; sin editor, vuelve con su texto; un editor sin principal se queda), `link.rs` (los mensajes nuevos ida y vuelta, y un historial que dice más textos de los que trae) y `split.rs` (el lanzador en una sola ventana).

### Decisiones (parte 2)

- **El historial viaja como textos enteros**, no como las operaciones de la `TextArea` (que no se pueden sacar ni meter): con un paso por texto, rehacerlo al otro lado es un cambio agrupado por paso. Lo tecleado sigue yendo letra a letra; lo que se pierde es solo la agrupación interna de la `TextArea`, y el cursor al deshacer queda donde estaba el cambio porque solo se toca lo que cambia.
- **Una sola acción para sacarlo** («Abrir el editor en su ventana»): en una sola ventana lo saca y con el editor fuera lo vuelve a abrir; para el usuario es lo mismo (quiere el editor en su ventana).
- **La tabla de choques no se recalcula sola**: analizar renderiza offline cada pista en solo (segundos); se queda la del último análisis, con un aviso si se cargó otra cosa después.

## Fase M: resaltado en vivo del código al sonar ✅

Hechos los diez puntos de [docs/ROADMAP.md](ROADMAP.md), en cinco partes (una por commit): posiciones de los tokens, paso publicado por el motor, dibujado en el editor, protocolo del socket y configuración y paleta. El uso, en «Resaltado en vivo» de [docs/GUIA.md](GUIA.md).

- **Dónde está escrito cada paso** (`script/src/written.rs`, `script::TokenSpan`): al llamar a `:steps("...")`, `:melody("...")` o, dentro de `every`, `c:steps`/`c:melody`, `written::spans` busca la cadena en el texto que se evalúa (`ScriptText`, en los datos de la aplicación de Lua desde `eval`): primero entre las cadenas de la línea de la llamada (`lua.inspect_stack`, la primera función de Lua cuyo trozo es el del script) y, si no está ahí, la única cadena igual de todo el texto (así vale `local p = "..."` con `:steps(p)`). Las cadenas se leen con comillas simples o dobles, sin comentarios; las largas (`[[...]]`) y las que se parten en varias líneas no cuentan. Cada paso separado por espacios da un `TokenSpan` (fila desde 0 y columnas en caracteres, como el diagnóstico del editor); si no se encuentra la cadena o no tiene tantos pasos como la pista, todos `None`. `TrackBuild::written` guarda los de la pista y `expand` los pasa a cada vuelta del ciclo (`TrackDef::spans`, en paralelo a `bars`); una vuelta en la que la función de `every` llama a `c:steps("...")` se queda con los de esa cadena (`BarHandle::written`). `TrackDef::sounding_span(índice)` (el índice del ciclo aplanado: `vuelta · length + paso`) da el token que se resalta: el del paso si dispara algo; en un paso que alarga un golpe (`_`, por el `hold` del golpe), el del golpe; en un silencio, nada. Tests en `script/src/written.rs` (cadenas con comentarios, escapes y sin cerrar; la línea de la llamada antes que el resto y la cadena única; columnas en caracteres) y `script/tests/spans.rs` (cada paso con su token, `_` en su golpe, `r3` y sufijos enteros, silencios sin nada; cadenas en varias líneas y en una variable; `euclid`, una cadena repetida y una concatenación, sin nada; `every` con su propia cadena y con `reverse`/`retrig`; `melody` y secciones).
- **El paso que suena, publicado por el motor** (`engine/src/status.rs`): cada definición de pista lleva una etiqueta única entre cargas (`TrackDef::tag`, de 1 a `engine::MAX_TAG` = 2²⁰ − 2, que da `eval` con `ScriptHost::last_tag` en todas las versiones y secciones) y su patrón la lleva al motor (`Pattern::with_tag`; no cambia el audio). En cada paso, `Engine::step` publica por pista con patrón el cursor (`SharedStatus::set_cursor`): la etiqueta, el paso del ciclo aplanado (`Pattern::index`, el mismo que lee `Pattern::hit`, con el origen de la sección) y la semicorchea, empaquetados en un solo `AtomicU64` (20 + 12 + 32 bits: se leen enteros, sin mezclar dos pasos); también en los silencios. Sin patrón, nada; al arrancar se borran (`clear_step`). La interfaz los lee en `TrackStatus::cursor` (`engine::Cursor`; la semicorchea entera se reconstruye con su diferencia con `Status::global`). Además, `Status::heard`: la posición del último bloque menos el retardo de salida (`Engine::set_output_delay`, que llama el callback de `cpal` con lo que dice su marca de tiempo entre el callback y la salida más el propio bloque; offline y con la salida nula, 0) pasado a pasos al tempo que suena. `Song::track_by_tag` busca la etiqueta en la canción cargada (pistas, secciones y la otra variante). Todo son atómicos sueltos: sin reservas, mutex ni E/S en el hilo de audio. Tests en `engine/tests/status.rs` (el cursor de cada paso con un ciclo de dos vueltas, nada sin patrón, la etiqueta nueva justo en el compás siguiente, desde 0 al volver a arrancar; el tope de la etiqueta; `heard` sin y con retardo y `None` parado), `engine/tests/no_alloc.rs` (un patrón con etiqueta y `set_output_delay` en cada bloque) y `script/tests/spans.rs` (etiquetas distintas por pista y por carga, la del motor encuentra su pista y su token, y la de la carga anterior ya no está).
- **El dibujado en el editor** (`tui/src/highlight.rs`): `Cursors` guarda los últimos 8 cursores distintos de cada pista del motor (`App::tick` los apunta con cada lectura del estado; si la semicorchea vuelve atrás, se ha arrancado otra vez y se olvidan). `highlights` elige, para la semicorchea que se oye (`floor(Status::heard)`), el último cursor que empezó en ella o antes, si no es de hace más de un paso (la interfaz puede perderse alguno; uno más viejo ya no es lo que suena), busca su etiqueta en la canción cargada (`Song::track_by_tag`, y que sea la de esa pista) y pide el token (`TrackDef::sounding_span`). `App::highlights` lo hace solo con el secuenciador en marcha y si el texto del editor es el de la última carga buena (`App::loaded_text`, que pone `load`); el resultado va a `ViewModel::highlights` en el orden en que se dibuja. `ui::draw_editor` pinta cada uno sobre el texto ya dibujado (como el subrayado del diagnóstico, que va después y queda encima; las dos con la misma cuenta de celdas visibles con el desplazamiento y los números de línea), con `ui::highlight_style`: el fondo del color de la pista en `Theme::highlight` (por su orden en el script, 8 colores de la paleta de 256 por defecto) y, en la seleccionada, el texto en negro y en negrita; en las demás, el fondo apagado (`Theme::highlight_dim`, que calcula `theme::dimmed`: el RGB del color a un 45 % de luz, al color más cercano de la paleta de 256 sin truecolor). Tests en `tui/src/highlight.rs` (el cursor del paso que se oye, uno perdido, uno demasiado viejo, otra pista, rearrancar), `theme.rs` (colores apagados) y `tui/src/tests/highlight.rs` (con el motor offline: el «1» de la seleccionada en su color entero y negrita y la «x» de otra en el suyo apagado, solo el token; silencios sin nada; el segundo «2» en el paso 6; nada parado; con un paso de retardo de salida, lo de un paso antes; al escribir, nada, y al deshacer, otra vez; tras otra carga, nada hasta el compás siguiente y luego el token movido; una cadena compartida por dos pistas con el color de la seleccionada, también tras cambiarla; tres pistas en una línea con su color cada una).
- **Con el editor en su ventana** (`tui/src/link.rs`, `tui/src/app/split.rs`): el protocolo pasa a la versión 3 con un mensaje nuevo de la principal, `suena` (`ToEditor::Sounding`): la revisión del editor a la que se refiere (`MainLink::editor_rev`, la del último texto recibido) y los tokens que suenan ya resueltos (fila, columnas, color y si es de la seleccionada), en el orden en que se dibujan. La principal los calcula con lo suyo (`App::highlights`: su motor, su canción y su copia del texto del editor, que compara con lo cargado) y los manda en `push_to_editor` solo cuando cambian ellos o la revisión (`MainLink::sent_sounding`; también tras conectar otro editor). El editor los guarda (`EditorLink::sounding`) y `App::remote_highlights` los da solo si sigue conectado, la revisión es la suya y su texto es el último que mandó: lo escrito desde entonces (aún sin mandar) apaga el resaltado al momento, sin esperar a la principal. Al perder la principal (caída o `adios`), se olvidan. Tests en `tui/src/link.rs` (`suena` ida y vuelta, vacío y con varios tokens; tokens rotos, error) y `tui/src/tests/split.rs` (las dos ventanas por el socket con el motor offline: los mismos tokens que en la principal, dibujados en la ventana del editor con el color de su pista; el paso siguiente; al escribir, nada en el editor ni en la principal, y al deshacer, otra vez; sin principal, nada).
- **Configuración y paleta** (`tui/src/config.rs`, `theme.rs`, `palette.rs`): `[resaltado]` en `config.toml` con `activo` (`Config::highlight`, `HighlightConfig::enabled`; por defecto, encendido), `estilo` (`Theme::highlight_mode`, `theme::HighlightMode`: `"fondo"`, `"subrayado"` o `"invertido"`), `colores` (uno o una lista, como los de `[colores]`; `Theme::set_highlight` los repite hasta las 8 pistas y calcula los apagados con o sin truecolor) y `retardo_ms` (`HighlightConfig::delay_ms`, de 0 a 1000: lo que se suma a la latencia que dice la salida, que `App::highlights` pasa a pasos al tempo que suena). Lo que no vale se avisa y se queda como estaba. `ui::highlight_style` pinta según el estilo: fondo (como en la parte 3); subrayado en el color de la pista (`underline_color`; la seleccionada, también el texto de ese color y en negrita); invertido (`REVERSED` con el texto del color entero en la seleccionada, en negrita, y apagado en las demás). «Resaltado en vivo» en la paleta (`Action::Highlight`, `App::toggle_highlight`; atable en `[teclas] resaltado`) lo apaga y lo enciende; con el editor separado, es de los de `editor_side`: se hace en la ventana del editor y solo cambia esa (en la principal, deja de calcularlo y de mandarlo). Las tablas de la paleta y de la configuración de `docs/GUIA.md` salen de `ACTIONS`, `BINDABLE` y `HIGHLIGHT_KEYS`. Tests en `config.rs` (las cuatro claves, colores que se repiten con y sin truecolor, valores por defecto, cinco avisos sin cambiar nada; la lista de teclas), `tui/src/tests/highlight.rs` (la paleta lo apaga y lo enciende, `activo = false`, `retardo_ms = 125` a 120 BPM es un paso, los tres estilos) y `tui/src/tests/split.rs` (apagado en la ventana del editor, la principal sigue calculándolo).

### Decisiones

- **La posición se busca en el texto, no la da Lua**: Lua solo sabe la línea de la llamada, no la columna ni de dónde salió la cadena. Buscar el literal igual en esa línea (o el único igual del texto) cubre lo que se escribe a mano sin tocar el lector de Lua; lo que no se encuentra no se resalta, que es mejor que marcar mal.
- **Patrones sin texto literal** (punto 5): se resalta el token escrito en la posición del paso que suena. `every`, `fill` y `chance` no mueven los pasos, así que se resalta el token del patrón base en esa posición (o el de `c:steps("...")` en las vueltas de `every` que lo escriben); si un `fill` pone un golpe donde había un `.`, se resalta el `.`. Lo generado por funciones (`euclid`, `rotate`, `mutate`, `shuffle`, `reverse_pattern`, concatenaciones, `string.rep`) no es un literal del código: esa pista (o esa vuelta) no resalta nada. Resaltar la llamada entera obligaría a saber sus columnas, que Lua no da.
- **Una etiqueta por definición de pista en vez de comparar patrones**: tras una carga, el motor sigue tocando el patrón viejo hasta el compás siguiente, y sus pasos se refieren al texto viejo. Con la etiqueta, la interfaz sabe exactamente de qué carga (y de qué sección o variante) es lo que suena, y si no es de la cargada no resalta nada.
- **Varias pistas a la vez** (punto 6): un color por pista según su orden en el script; la seleccionada con el color entero, el texto en negro y negrita, y las demás apagadas. Tokens distintos en la misma línea no chocan: cada uno con su color. Solo si dos pistas marcan las mismas celdas (una cadena compartida en una variable) hay que elegir: manda la seleccionada y, si no lo es ninguna, la definida antes. Se resuelve dibujando en orden, sin mezclar colores.
- **Texto distinto del cargado: no se resalta nada** (punto 4). Comparar el texto entero es simple y nunca marca mal; resaltar las líneas que no han cambiado obligaría a seguir cómo se mueven las filas y columnas con cada edición, y el resaltado vuelve en cuanto se carga (o se deshace).
- **Por el socket van los tokens ya resueltos, no el paso** (punto 9): el editor separado no evalúa el script (ver la fase L), así que no tiene la canción ni sabría pasar de un paso a una posición del texto. La principal, que sí, manda lo que hay que pintar; son pocos campos y solo cuando cambian (como mucho, una vez por paso). Que se refieran a una revisión concreta del texto evita marcar mal mientras se escribe en el editor.
- **Sin recuadro** (punto 8 y el objetivo): una terminal pinta celdas de un carácter con su color, su fondo y sus atributos; no hay forma de dibujar un borde alrededor de un trozo de texto sin quitarle sitio. Los tres estilos (fondo, subrayado, invertido) son los que cualquier terminal sabe pintar; el subrayado de color (`underline_color`) depende de la terminal (kitty, WezTerm y foot lo tienen; las que no, lo subrayan en el color del texto).
- **`retardo_ms` además de la latencia medida**: `cpal` da la marca de tiempo de la salida, pero algunas salidas (Bluetooth, servidores de sonido con búferes propios) dicen menos de lo que tardan. Un número a mano lo arregla sin adivinar.
- **El motor publica el paso, la interfaz decide qué se oye**: el hilo de audio solo guarda el último cursor de cada pista y el retardo; la interfaz guarda los últimos cursores que ha visto y elige el del paso que se oye. Así el motor no necesita un historial ni saber nada de la latencia más que un número.

## Fase N: automatización y efectos por paso en todo ✅

Hechas las cuatro partes: automatizar todo lo continuo, por golpe lo que la
voz lee al nacer, efectos por paso en la mini-notación y el tracker
editable.

### Parte 1: automatizar todo lo continuo

- **Registro** (`script/src/api.rs`): `FxParam::automate: Auto` en cada parámetro de sonido (`Auto::Smooth`, la curva se evalúa cada 32 frames y se suaviza ~5 ms; `PerHit`, cada golpe toma su valor al sonar; `PerGrain`, al nacer cada grano; `No(porqué)`). Tablas: `EFFECTS`, `BUSES`, `SLICE_PARAMS`, `GRAIN_PARAMS`, `INSTRUMENT_PARAMS`, `SAMPLE_PARAMS` y la nueva `TRACK_PARAMS` (`gain`, `pan`, `send`, `swing`; `api::track_param`). `Effect::param`/`automatable` y `api::automatable` leen de ahí (antes se preguntaba al motor); `api::sound_params()` las junta y la guía genera con ellas «Qué se puede automatizar» tras `lfo` (lo que admite curva, y cómo; lo que no, agrupado por sitio y porqué). Los porqués son constantes del registro (`DISCRETE`, `AT_LOAD`, `AT_BIRTH`, `STRUCTURE`, `SEED`, `INSTRUMENT`, `TRACK_REF`, `DELAY_TIME`).
- **Errores**: `no_curve` da `quién: clave no admite ramp() ni lfo(): porqué` con la línea; `table_curves` lo comprueba en cada tabla (`fx` de pista y de corte, `cortes:set`, `sample()`, `instrument()`, `grains`/`freeze`) antes de leerla, y `bus()`, `:swing()` y `number_or_curve` (gain, pan, send) en sus valores. Una curva fuera del rango, «la curva se sale del rango (a a b)».
- **Motor, efectos** (`engine/src/fx.rs`): `Fx::params` añade al final (los índices de antes no cambian) `ratio`, `attack`, `release` al compresor y al EQ dinámico y `bits`, `downsample` al bitcrusher, con su rango (`param_range`; ratio y tiempos en escala logarítmica) y su `set_param`: el compresor y el EQ dinámico recalculan la pendiente o el coeficiente de la envolvente (`time_coef`, el mismo cálculo que al crearlos); el bitcrusher acepta bits con decimales (`crush_levels`: con un entero, `1 << (bits − 1)` como siempre; si no, `2^(bits−1)`) y `downsample` a saltos enteros (redondeado; el contador se ajusta al nuevo).
- **Motor, mezcla** (`engine/src/automation.rs`, `engine.rs`): `MixCurves` = una curva por `MIX_GAIN`, `MIX_PAN` y `MIX_SEND + bus` (`Pattern::with_mix_automation`, validada con `check_mix`); `Pattern::with_pan` (pan de pista nuevo, de -1 a 1, ley de balance como el de los cortes, después del volumen y antes de los envíos). En `mix`, con curvas, el destino de volumen, pan y envíos se recalcula cada `MOD_BLOCK` frames (`mix_at`) y el suavizado por muestra de siempre lo sigue; sin curvas, el mismo destino fijo de antes. El pan solo se aplica si el destino o el actual no son 0 (al centro, nada: bit a bit). En silencio entran ya (`Track::pan`, como el volumen).
- **Motor, buses** (`engine/src/bus.rs`): `Buses::curves: BusCurves` (`DELAY_FEEDBACK`, `DELAY_LEVEL`, `REVERB_SIZE`, `REVERB_DAMP`, `REVERB_LEVEL`). Con alguna curva (o volviendo de una) `Rack::mix_into` procesa en bloques de 32 frames y `Rack::drive` da los parámetros de cada bloque, suavizados con un polo de ~5 ms (`Rack::moving`); sin curvas, un solo bloque con los parámetros fijos (bit a bit). Para dormir el delay cuenta el feedback más alto al que llega la curva.
- **Lua**: `:gain()`, `:pan()` y `:send(bus, nivel)` aceptan número o curva (`TrackDef::pan`, `TrackDef::mix_curves`); `bus()` con curvas en `feedback`, `level`, `size` y `damp` (el `time`, error con su porqué). Interfaz: el panel de efectos dice `vol 0.50∿`, `pan -0.30` / `pan ∿` y `→ r∿` (`TrackView::pan`, `mix_moving`); el autocompletado ofrece `ramp` y `lfo` tras cualquier clave que las admite según el registro, también en las tablas de `bus()` y de granos.
- **Tests**: `engine/tests/automate_all.rs` (cada parámetro de cada efecto, volumen, pan, envíos y parámetros de los buses: una rampa que recorre todo su rango sobre un tono con trémolo no salta entre muestras más que el efecto fijo en alguno de 33 valores de su recorrido, y cambia el audio; el pan al centro bit a bit y a -0,5 el lado contrario a la mitad exacta; rangos), `no_alloc.rs` (todo automatizado a la vez y quitar las curvas de los buses), `script/tests/automation.rs` (lo nuevo suena igual que el motor; lo que no se mueve dice por qué con la línea), `script/tests/registry.rs` (el motor mueve exactamente lo que dice el registro; cada parámetro de sonido acepta una curva o da su porqué), `tui/src/tests/automation.rs`.

### Parte 2: por golpe lo que la voz lee al nacer

- **Motor**: `SliceShape::curves: ShapeCurves` (`SHAPE_ATTACK`, `SHAPE_RELEASE`, `SHAPE_PAN`, `SHAPE_LENGTH`, `SHAPE_CROSSFADE`; validadas en `SliceShape::validate`); al ir dentro de la forma, la comparten la variable, sus pistas, `slice_at`, `add`/`remove` y el proyecto sin cambiar nada más. `Pattern::with_pitch_curve` (semitonos que se suman a los de cada golpe; ±36) y `with_formant_curve`. `Trigger::full_end` guarda el final del corte entero para recalcular `length`. `Engine::per_hit`, al empezar `play_hit` (secuenciador y subdisparos del retrigger): con alguna curva, evalúa en `song_position()` (la posición de la canción en el frame del golpe, desde el inicio de la sección, la de las demás curvas) y cambia la velocidad del golpe y de las notas de su acorde (el total, recortado a ±36 semitonos), el formante (`Playback::formant`, que manda sobre el fijo de la pista) y la forma (con `length`, el final). Sin curvas, el golpe tal cual (bit a bit). Instrumentos y granos leen la velocidad del golpe: el pitch de pista también les vale.
- **Lua**: `pista:pitch(semitonos)` (nuevo: número o curva; un número es una curva que no se mueve, así que `:pitch(7)` suena bit a bit como `^7` en cada paso), `:formant()` con curva, y `cortes:set` con curvas en `attack`, `release`, `pan`, `length` y `crossfade` (el valor fijo es su primer extremo; un número después quita la curva). Registro: `TRACK_PARAMS` con `pitch` y `formant` (`Auto::PerHit`) y los de `SLICE_PARAMS`, `PerHit`.
- **Tests**: `engine/tests/per_hit.rs` (cada golpe con el pitch, el formante o cada parámetro de la forma con curva suena bit a bit como ese golpe, y la cola del anterior, con el valor fijo de la curva en su paso; el pitch se suma al del paso y se queda en 36; rangos), `no_alloc.rs` (pitch, formante y las cinco curvas de forma con bucle y gate), `script/tests/automation.rs` (igual que el motor, `:pitch(7)` = `^7`, errores con la línea) y `script/tests/registry.rs` (que ya los da por buenos).

### Parte 3: efectos por paso en la mini-notación

- **Notación** (`engine/src/notation.rs`): tras el corte, en cualquier orden con los sufijos, `{efecto[=valor][,clave=valor]...}`, uno por par de llaves. Valores: número, palabra (`type=highpass`) o rampa del golpe `desde>hasta[:pasos]` (`FxValue`). El motor solo lee lo escrito (`WrittenFx { name, main, params }`) y se lo pasa a quien lo resuelve (`FxResolver`; `parse_pattern_with`, `parse_step_with`); sin él, error «las llaves ({...}) solo valen en steps("...") de una pista de cortes, en un script»; `melody` (`patterns::note_step`) da antes el suyo, que dice que no valen en `melody()` ni en un instrumento (`BRACES_IN_MELODY`). `step_tokens`/`step_texts`: los pasos se separan por espacios que no estén dentro de llaves (lo usan el parser, las posiciones de la fase M en `script/src/written.rs`, `script::patterns` y el diagnóstico); una llave sin cerrar da el error en su paso.
- **Resolución** (`script/src/step_fx.rs`): `resolve` busca el efecto por nombre o alias en el registro (alias nuevos `lp`, `hp`, `crush`), el valor sin clave es el de `Effect::main_param()` (el primer número de su tabla), comprueba claves, repetidos, `sidechain` (no en un paso) y que una rampa vaya en un parámetro que admite curvas (`Auto`), y construye el efecto con `build_fx`, la parte de `parse_fx` sin Lua que ahora comparten `:fx`, `cortes:fx` y los pasos (mismos valores por defecto, rangos y mensajes, con `{lp}` en vez de `fx("lowpass")`). Rampas: `Curve::Hit { from, to, steps }`.
- **Motor**: `Step::fx: Option<SliceFx>`; `Pattern::from_bars` guarda las cadenas distintas en `Pattern::step_fx` (sin repetir) y cada `Trigger` su índice (`Trigger::fx`) y, al dispararse, `Trigger::origin` (el paso de la canción del golpe; los subdisparos del retrigger lo heredan). `Pattern::hit_fx` junta la cadena del corte y la del paso con `SliceFx::with_step` (arrays fijos: vale en el hilo de audio): un efecto del paso del mismo tipo que uno del corte (`same_kind`: mismo efecto y, en los filtros, mismo paso bajo o alto) lo sustituye en su sitio con sus curvas; los demás, detrás. `with_slice_fx` comprueba que cada golpe cabe (4 efectos, 8 curvas); `with_instrument` y `with_granular` rechazan los efectos de paso. La voz los lleva como los de corte (`VoiceFx`, con `origin`); `drive_block` evalúa con `Curve::value_at(…, origin)`: la rampa del golpe cuenta desde su paso y se queda en `to` al acabar; las demás curvas, como siempre. `Curve::Hit` solo vale en una voz (`Automation::validate_in_voice`); en la pista, la mezcla, los buses, la forma, el pitch o los granos, error (`check_curve`).
- **Script**: `checked_steps` lee con el resolutor (`steps`, `c:steps` en `every`; `euclid`, `rotate`... conservan las llaves); errores con la línea: el del resolutor con el paso y el token, instrumento (en `steps`) y granos (en `steps` o en `grains`/`freeze`, en los dos órdenes). Que los efectos de un paso quepan con los de su corte se comprueba al acabar el script (`step_fx_fit`, en `expand`, con la línea del último `steps` guardada en `TrackBuild::steps_at`), así vale también si `cortes:fx` llega después.
- **Interfaz**: el tracker enseña los efectos del paso con su nombre más corto del registro (`model::short_name`) y `∿` si llevan rampa (`03{lp∿,crush}`); el diagnóstico en vivo los resuelve igual que la carga (subraya el paso, con su mensaje); el autocompletado, dentro de unas llaves abiertas de `steps("...")`, ofrece los efectos y sus alias con su valor principal (`lp=${3000}`) y, tras una coma, sus parámetros (`q=${0.707}`); el resaltado en vivo marca el token entero con sus llaves. `tui --pattern` también los entiende.
- **Tests**: `engine/tests/step_fx.rs` (solo cambia el golpe con efectos: con efectos sin memoria, todo lo demás bit a bit; con un filtro resonante, bit a bit pasada su cola de 100 ms; cadena vacía, igual que nada; misma cadena en dos pasos; la rampa del golpe sube en los subdisparos del retrigger, vuelve a empezar en otro paso, no hace clics y se queda arriba; manda el paso sobre el corte, sustituyendo o detrás, y error si no caben; instrumento, granos y `Curve::Hit` fuera de un paso rechazados; 8 pistas con todo a tope dentro de ±1), `engine/tests/notation.rs` (tokens con llaves, lo que llega al resolutor y su error con el paso), `no_alloc.rs` (efectos de paso con rampa, retrigger, reverse, pitch y los del corte), `script/src/step_fx.rs` (nombres, alias, valor principal, rampas y cada error), `script/tests/step_fx.rs` (igual que el motor y solo cambia su golpe, `every` y `rotate`, manda el paso, errores con la línea sin dejar de sonar lo anterior, entra al compás, posiciones con espacios dentro de las llaves) y `tui/src/tests/step_fx.rs` (tracker, diagnóstico, resaltado y autocompletado).

### Parte 4: el tracker editable

- **Antes**: el tracker era solo de lectura (sin cursor; seguía el paso que suena). Hacerlo editable cuesta poco gracias a la fase M: la carga ya sabe dónde está escrito cada paso (`TrackDef::spans`, por vuelta del ciclo).
- **Lo mínimo que cumple** (`tui/src/app/tracker_edit.rs`): «Editar los efectos de un paso en el tracker» (paleta, `Action::EditTracker`) abre `App::tracker_edit` (`TrackerCursor { track, row }`, en la pista seleccionada y el paso que suena). ↑↓ paso, ←→ pista (la seleccionada lo sigue), Enter abre la línea de órdenes con `efectos ` y los efectos del paso (`written_fx`: lo de dentro de cada par de llaves, sin espacios), Supr los quita, Esc sale; deshacer, rehacer, cargar, marcha y salir siguen funcionando. La orden `efectos ...` (`run_step_fx_command`) escribe en el paso que se editaba al pulsar Enter (`PendingEdit`: pista, vuelta y paso, aunque luego suene otra vuelta). `fx_groups` acepta `lp=800,q=4 crush=6` o `{lp=800}{crush=6}`; `with_step_fx` quita las llaves del token y pone las nuevas donde estaban las primeras (o al final): el corte, los sufijos y su orden no cambian. Antes de tocar el texto se lee el token nuevo con el resolutor y las etiquetas de la pista (una errata solo da el mensaje). Después, la sustitución de esos caracteres en el editor dentro de `App::grouped` (un paso de deshacer) y `App::load` (como Ctrl-L: entra al compás siguiente; si fallara, sigue sonando lo anterior y se ve el error).
- **Lo que no se edita, y por qué** (lo dice la línea de estado): el texto no es el cargado (las posiciones serían de otro texto), un paso sin posición (generado: `euclid`, `rotate`, `chance`, concatenaciones...), un silencio y las pistas de instrumento o de granos. Con `every`, la vuelta que se ve (si viene de un `c:steps("...")` literal, ese texto).
- **Vista**: `ViewModel::tracker_cursor`; la celda del cursor, invertida y en negrita; el tracker se desplaza siguiendo el cursor. La celda enseña los efectos del paso (parte 3).
- **Editor en su ventana**: la edición se hace en la principal (donde está el tracker), que cambia su copia del texto y la manda con el `ToEditor::Text` de siempre; el editor la pone como un paso de deshacer (`replace_text`). El protocolo no cambia: sigue en la versión 3.
- **Tests**: `tui/src/tests/tracker_edit.rs` (ida y vuelta: el texto es exactamente el original con ese token cambiado, el modelo cargado es el de ese texto y cargarlo otra vez da los mismos pasos y el mismo audio que escribirlo a mano; cambiar, quitar con Supr y deshacer cada cambio de una vez; el cursor se mueve, se ve invertido y mueve la selección; lo que no se edita dice por qué y no toca el texto; con el editor en su ventana el cambio llega como un paso de deshacer) y los unitarios de `tracker_edit.rs`. `cargo test -p tui pantalla_tracker_edit -- --ignored --nocapture` enseña la pantalla.

### Decisiones

- **Precedencia (resumen)**: efecto del paso → del corte → de la pista → mezcla. Paso y corte suenan en la voz del golpe (el paso sustituye el efecto del mismo tipo del corte y los demás van detrás); los de la pista, sobre la suma de sus golpes; luego volumen, pan y mute de la pista (con sus curvas), envíos, buses y limitador. `~` (el corte del primer filtro de la pista desde el golpe) manda sobre la automatización de la pista. Por golpe (pitch, formante, forma del corte): el valor de la curva al sonar, sumado al pitch del paso.
- **Combinaciones que no tienen sentido, y qué se hace**: efectos por paso en un instrumento o una pista de granos, error con la línea (no hay voces de corte); una rampa de golpe (`a>b`) fuera de un paso, imposible desde Lua y error en el motor; `sidechain` en un paso, error; curvas en lo discreto o lo que se prepara al cargar, error con el porqué (tabla de la guía); pads con efectos de paso, no aplica (un pad no es un paso; sí lleva los de su corte y el valor fijo de las curvas por golpe); MIDI de salida, los efectos solo tocan el audio.
- **Coste de memoria**: `Step` pasa de 56 a 632 bytes (lleva su cadena de efectos dentro; solo en el hilo de control: el modelo del script y al construir el patrón: unos 10 KB por pista de 16 pasos, y en el peor caso, 64 vueltas de 64 pasos, 2,6 MB por pista). El `Trigger` del motor pasa de 136 a 280 bytes (las curvas de la forma, 120; el final del corte entero, el índice de la cadena del paso y su origen, 24): 2,3 KB más por pista de 16 pasos. Cada voz, 120 bytes más (la forma con sus curvas); `Pattern`, 176 bytes más, y una tabla con las cadenas de paso distintas (576 bytes cada una). Nada se reserva en el hilo de audio.
- **Coste de CPU** (8 pistas a 170 BPM, render offline en release, en el i7 de desarrollo): sin nada nuevo, 1,9 % de un núcleo, igual que antes (las huellas no cambian y los caminos nuevos solo se toman con curvas o efectos); con envíos, 2,8 %; con 4 efectos de paso (uno con rampa) en cada golpe de las 8 pistas, 6,2 %; con todo automatizado (volumen, pan, envíos, buses, filtro y compresor de pista y curvas por golpe), 7,8 %. Lo que más cuesta es lo de siempre (filtros que recalculan coeficientes cada 32 frames y una cadena por voz).
- **Sintaxis `{...}`**: las llaves no las usa ningún sufijo (`'` `^` `%` `@` `r` `~` `_` `.`) ni los acordes, se leen bien dentro de una cadena de Lua y agrupan: `3{lp=800,q=4}` es un paso aunque lleve espacios dentro. Un efecto por par de llaves; el primer valor sin clave es el parámetro principal (lo más corto para lo más común: `{lp=800}`, `{crush=4}`, `{drive=8}`). Las rampas del golpe (`a>b`, `a>b:n`) cuentan desde el paso y no se repiten: para una rampa que se repita con la canción están `ramp()`/`lfo()` en `:fx` o `cortes:fx`. No hay LFO por paso (sería un `lfo()` de `cortes:fx`).
- **En la voz, como los de corte**: es el mismo camino (estado por voz, cola de 100 ms, nada de memoria dinámica) y por eso los demás golpes no cambian ni un bit. Las cadenas van en una tabla del patrón y cada golpe lleva su índice: el `Trigger` del hilo de audio solo crece 16 bytes.
- **Precedencia**: paso → corte → pista → mezcla. Entre el paso y su corte manda el paso (sustituye el del mismo tipo, en su sitio; los demás, detrás), como en un tracker un parámetro del paso sobre el de su instrumento. Los de la pista no se pueden quitar para un golpe: suenan sobre la suma de todos los golpes y separar uno obligaría a otra cadena por voz; para mover el filtro de la pista en un golpe sigue estando `~`, que manda sobre su automatización.
- **Combinaciones**: instrumento y granos, error con la línea (no tienen voces de corte; en `cortes:fx` los granos solo avisan porque la variable puede ser de otra pista, pero un paso es de esta). MIDI de salida: el golpe sale igual, los efectos solo tocan el audio. Pads: no tocan pasos, así que no llevan efectos de paso (sí los de su corte). Swing y humanize: el golpe se retrasa, pero su rampa cuenta desde su paso. `chance`, `fill`, `every`, `euclid`, `rotate`, `shuffle`, `mutate`: los pasos llevan sus llaves consigo (son texto o pasos copiados).
- **Por golpe y no dentro del golpe**: el pitch cambia la velocidad de lectura, y una voz que cambiara de velocidad a media lectura necesitaría otro lector (posición acumulada en vez de `frame · velocidad`, que es lo que hace que pitch 0 sea bit a bit lo de siempre) y una duración que no se sabe al nacer. La forma (ataque, cola, parte, bucle) la usa la voz al crearse. Así que estas curvas se miran al sonar cada golpe: una rampa de pitch sube golpe a golpe, como el `pitch` de `fill`. Los pads tocan con el valor fijo (el primer extremo).
- **La lista es del registro y el motor la cumple.** Antes `api::automatable` preguntaba al motor; ahora cada `FxParam` lo dice y un test comprueba que `Fx::param_index` coincide. Así la guía, los errores y el autocompletado no pueden separarse del motor.
- **Qué es continuo**: todo lo que es un número de un efecto o de la mezcla. `downsample` es un número de repeticiones y se mueve a saltos de 1 (no tiene sentido «repetir 2,4 veces»); `bits` sí admite decimales (la resolución cambia poco a poco). Los tiempos (`attack`, `release`) se mueven en escala logarítmica.
- **Qué no, y por qué** (la tabla de la guía): los tipos y `play` (valores con nombre: no hay nada entre medias), el `time` del delay (una figura; mover el retardo de una línea con repeticiones dentro suena a cambio de pitch), `sidechain` (una pista), `seed` (la semilla del azar), `swing` y `humanize` (se convierten al cargar en retrasos de cada paso), lo que se prepara al cargar (`sample()`, el bucle y el fundido del instrumento) y lo que cambia cuántas cosas suenan (`voices`). La raíz, el ADSR y el formante del instrumento los lee cada nota al nacer y el instrumento se valida entero al cargar: quedan fuera.
- **Mezcla tras los efectos**: el volumen y el pan con curva se evalúan cada 32 frames y los sigue el suavizado por muestra de siempre (un polo de ~5 ms), así que una rampa del volumen es tan suave como un cambio de volumen al recargar. El pan de pista es nuevo y usa la ley de balance de los cortes.

## Fase O: motor de síntesis y el acid ✅

Hechas las cuatro partes, una por commit: el núcleo de síntesis en el motor, su integración en las pistas, `synth("acid", {...})` en Lua y la interfaz. El uso, en «`synth(tipo, [opciones])`» y la receta «Un bajo ácido» de [docs/GUIA.md](GUIA.md).

### Parte 1: el núcleo (`engine/src/synth/`)

- **Osciladores** (`osc.rs`): `Osc { phase: f64 }` con `Wave::{Sine, Saw, Square, Triangle}`. Sierra y cuadrada con PolyBLEP de dos muestras (la cuadrada, un salto en 0 y otro en `pw`); triangular con PolyBLAMP en sus dos picos; seno con `f64::sin`. Ruido blanco con xorshift32 (`Noise`). Fase en f64 (en f32 la afinación derivaría a 20 Hz). **Sobremuestreo ×2**: cada voz genera dos muestras por cada una de salida y las diezma con un FIR de media banda (`Halfband`: seno cardinal de 63 coeficientes con ventana de Kaiser β = 8, unos 80 dB de rechazo; 16 coeficientes distintos de cero por lado, historia doble para que la ventana sea contigua; la tabla en un `LazyLock` que `Synth::new` fuerza fuera del hilo de audio). El residuo de PolyBLEP se pliega entonces por encima de lo audible y el diezmado lo quita.
- **Filtros** (`filter.rs`), en forma TPT (estables aunque corte y resonancia cambien cada muestra): `Svf` (Simper, 12 dB/oct; paso bajo, alto y banda; la resonancia lleva el amortiguamiento de 2 a 0,04, un pico de unas 25 veces sin autooscilar) y `Ladder` (4 polos, Zavalishin cap. 5: la realimentación `k = 4·res` se resuelve en la misma muestra con la respuesta lineal de la cascada, y la entrada de la cascada pasa por `soft`, una tanh racional de Padé recortada en ±3 donde vale 1 con pendiente 0; compensa en parte la caída de graves con `(1 + k/2)·x`). Con `res = 1` autooscila, acotado. `warp` acota el corte entre 20 Hz y el 45 % de la frecuencia de muestreo.
- **Envolventes** (`env.rs`): ADSR exponenciales (un polo hacia su destino; el ataque apunta a 1,3 para llegar a 1 en el tiempo pedido; decay y release cubren el 99 % en su tiempo; por debajo de -80 dB el release acaba). Se disparan y se sueltan desde el nivel en que estén: una nota nueva en la misma voz no salta. Coeficientes por bloque (`Rates`), no por muestra. La de filtro sin sustain sigue su decay aunque se suelte la nota (como la del 303).
- **Voces** (`Synth`, en `mod.rs`): 16 ranuras fijas (`POOL`: las que suenan, como mucho `MAX_SYNTH_VOICES` = 8, y las robadas que hacen su fundido de 1,5 ms). Cada voz: hasta `MAX_UNISON` = 7 osciladores desafinados (± `detune` cents los de los extremos, repartidos en estéreo con `spread`, normalizados por 1/√n), suboscilador (cuadrada una octava abajo) y ruido → diezmado → filtro (uno por canal si el unison está abierto) con su envolvente → amplitud → `level` → saturación (`drive`, la misma `soft` con la ganancia de entrada `1 + 15·drive²` y la salida normalizada). Polifónico: una nota nueva sin voz libre roba la más antigua. **Mono** (`Patch::mono`): una sola voz que se reutiliza (los osciladores siguen su fase; las envolventes salen de su nivel); una nota con slide sobre una voz que suena se liga (`Voice::tie`: va a su altura con el glide, sin volver a disparar las envolventes; la de amplitud vuelve a subir si estaba en su release). En un polifónico, el slide arranca la nota nueva en la altura de la anterior.
- **Parámetros** (`PARAMS`, `SynthParam { name, min, max, default, log }`): `CUTOFF`, `RESONANCE`, `ENV_AMOUNT` (octavas), `FILTER_DECAY`, `DRIVE`, `ACCENT`, `GLIDE`, `PW`, `DETUNE`, `SUB`, `NOISE`, `SPREAD`. `SynthCurves`: una curva por parámetro (`check_curves`). En `Synth::mix_into`, cada `MOD_BLOCK` (32 frames) se calcula el destino (la curva en la posición de la canción o el fijo del patch), se suaviza con el polo de 5 ms de siempre (en escala logarítmica las frecuencias y los tiempos) y cada voz **interpola linealmente muestra a muestra** desde el valor del bloque anterior (el corte, en octavas): ni una curva ni un patch nuevo saltan. En silencio, el siguiente bloque empieza ya en su destino.
- **Accent** (`Accent { gain, octaves, decay_ms, sweep_ms }`, por la parte de `ACCENT`): más amplitud, más octavas de filtro, otro decay de filtro y una memoria (la apertura se suaviza con `sweep_ms`: varios accents seguidos se acumulan). Es del núcleo: el acid solo pone sus números.
- **Determinismo**: la fase inicial de cada oscilador del unison, la del sub y la semilla del ruido salen de `Patch::seed` y del número de la nota (splitmix64). Mismo patch y mismas notas, mismo audio bit a bit.
- **Acid** (`acid.rs`): `acid::patch(raíz)` llena un `Patch` (sierra o cuadrada, ladder, mono, una voz, unison 1; VCA de 1,5 ms de ataque, 3 s de decay hacia 0 y 12 ms de release; envolvente de filtro sin sustain; accent: el doble de amplitud, dos octavas más con 200 ms de decay y 120 ms de memoria; `level` 0,4). `acid::PARAMS` dice cómo se llama cada parámetro del acid, a cuál del núcleo va y con qué escala (`envmod` de 0 a 1 son de 0 a 4 octavas); `acid::set` y `acid::curve` lo pasan.
- **`synth::render`**: renderiza notas sin el motor (tests y medidas).
- **Tests** (`engine/tests/synth.rs`): aliasing de la sierra a 4 kHz (medido a 44,1 kHz: a 48 kHz los armónicos de 4 kHz caen justo sobre otros armónicos y no se vería; espectro con Blackman-Harris de 65536 puntos, la componente no armónica más fuerte entre 20 Hz y 20 kHz respecto a la fundamental) por debajo de **-50 dB** (medido -52,3; con PolyBLEP sin sobremuestrear, -30; sin nada, -16,7), y sus armónicos 2-4 a ±1,5 dB de 1/k; cuadrada, pulso del 25 % y triangular con sus umbrales (-55, -47 y -80 dB; medidos -57,6, -49,2 y -83,2); los cuatro filtros con la resonancia al máximo y el corte en 20 Hz, 20 kHz y con un LFO de 50 Hz entre los dos, con unison, sub, ruido y sin y con saturación (ni un NaN; el ladder no pasa de 2, los de 12 dB/oct de 100, con saturación nada pasa de 1); el ladder autooscila acotado; una rampa de un paso de 100 a 8000 Hz y un LFO de 20 Hz en la resonancia no curvan un seno de 110 Hz más de 4 veces lo que se curva un seno (un clic, cientos); un cambio de patch en medio de una nota no salta más que antes o después (y un salto de 0,05 sí se detecta); misma semilla, mismo audio; 8 voces con unison de 7 tras el limitador dentro de ±1, y el unison suena como un oscilador (±3 dB); el acid deja margen (con accent, resonancia y filtro al máximo, con y sin saturación, por debajo de -1 dBFS; sin accent, por encima de -18); y el coste de CPU (ignorado).

### Parte 2: en el motor

- **Patrón** (`engine/src/sequencer.rs`): `Pattern::with_synth(Some(patch), curvas)` (`SendError::InvalidSynth`): los golpes son notas del banco de voces de la pista; el sample no suena (`Sample::silence`, un frame, para estas pistas). No vale con instrumento, granos ni efectos por corte o por paso; un mono con acordes, error con el paso. **Slide**: un golpe seguido de uno con slide dura un paso más (`hold + 1`, mirando el ciclo aplanado, así también entre vueltas): llega sonando al ligado aunque este llegue tarde con swing o humanize. Las notas MIDI de salida duran también ese paso más (legato para un synth de fuera). `Step::{accent, slide}` (`accented()`, `slid()`), y `Trigger` los lleva.
- **Notación** (`engine/src/notation.rs`): `!` accent y `/` slide, sufijos de una vez como los demás; en `melody` van detrás de la nota (`c!`, `eb/`).
- **Pista** (`engine/src/engine.rs`): `Track::synth: Synth` (las voces, reservadas al crear el motor: `Track::new(sample_rate)`) y `synth_patch` (el del último golpe o nota en vivo: con él siguen sonando las voces si el patrón ya no tiene sintetizador). `play_synth`: una nota por nota del acorde, `pitch = raíz + 12·log2(velocidad)` (así el pitch de pista con curva y `fill("pitch")` también valen), con el accent y el slide del paso; sus notas MIDI, como las de un instrumento. En la mezcla, tras las notas de los instrumentos, `Synth::mix_into` con el patch del patrón (o el que queda) y sus curvas, en la posición de la canción del trozo; después, los efectos de la pista. `has_notes` cuenta sus voces (el silencio, el mute/solo suavizado y la limpieza de efectos, igual que con un instrumento) y `TrackStatus::voices` también.
- **En vivo**: `Controller::synth_note_on(pista, patch, semitonos, volumen, tecla)` (`Command::SynthOn`; lleva el patch, así suena con el secuenciador parado y antes de que entre el patrón); `note_off` suelta también sus voces; paro y choke las cortan con el fundido mínimo.
- **Tests**: `engine/tests/synth_track.rs` (notas a su altura, accent el doble, slide sin volver a disparar la envolvente y con glide, la nota de antes de un slide no suelta, notas en vivo hasta su note off, paro, una rampa del corte abre el filtro, 8 voces con unison de 7 en dos pistas dentro de ±1, lo que el motor no deja), `engine/tests/notation.rs` (`!` y `/`), `no_alloc.rs` (un acid con accents, slides, rampa del corte, choke y efecto de pista y un polifónico con unison de 7, acordes, LFO y MIDI; 10 notas en vivo y sus note off; quitarlos). `golden.rs`, sin cambios.

### Parte 3: en Lua

- **`synth(tipo, opciones)`** (`script/src/lib.rs`, `synth_options`): devuelve un `SynthHandle` que `track()` acepta como un instrumento: `InstrumentDef` con `synth: Some(SynthDef { kind, patch, curves, seed })`, la raíz (C2 por defecto), y una pista de un corte sobre un frame de silencio. Así `melody`, `scale`, `every`, `chance`, `fill`, `:pitch()`, `:midi(canal)`, los pads y el MIDI de entrada funcionan como con un instrumento de sample. `track_pattern` manda el patch con `with_synth` (y ningún `Instrument`). La semilla, si no se da, sale del nombre de la pista (FNV-1a, `name_seed`), así dos pistas con el mismo synth no suenan en fase.
- **Registro** (`script/src/api.rs`): la función `synth`, `SYNTHS` (los tipos, como `Effect`; de momento `acid`) y `ACID_PARAMS` (`wave`, `root`, `cutoff`, `res`, `envmod`, `decay`, `accent`, `dist`, `glide`, `seed`) con su descripción, valor por defecto, rango y `Auto` (los continuos `Smooth`; `wave`, `root` y `seed`, no y por qué). La guía genera la tabla y la fila de «Qué se puede automatizar»; un test comprueba que rangos y valores por defecto son los de `engine::synth::acid::PARAMS`. `chance` tiene `accent` y `slide`.
- **Errores con la línea** (y sigue sonando lo anterior): tipo o clave desconocidos, rangos, `wave`, `root`, `seed`, curvas fuera de rango o en lo que no se mueve, acordes en un mono, efectos por paso, `fx` sobre el synth, y `!`, `/`, `chance("accent")` o `chance("slide")` en una pista que no es un sintetizador (`Plays`: cortes, instrumento o sintetizador, en `checked_steps` y en el compás de `every`).
- **Guía**: la sección generada, `!` y `/` en la tabla de la mini-notación, la receta «Un bajo ácido» (`scripts/ejemplos/09-bajo-acido.lua`; `guide.rs` comprueba que coincide y que carga) y el modo tocar.
- **Tests**: `script/tests/synth.rs` (el script suena igual que el motor con el mismo patch, muestra a muestra; curvas en las unidades del motor; la semilla del nombre; accent y slide desde `every` y `chance`; la nota de antes de un slide suena hasta el ligado con swing, igual en `steps` y en `melody`; MIDI; 16 errores con la línea; un error deja el modelo anterior), `script/tests/registry.rs` (cada parámetro con su plantilla, el registro coincide con el motor, cada parámetro de sonido acepta su curva o dice por qué), `patterns.rs` (`melody` con `!` y `/`).

### Parte 4: en la interfaz

- **Tracker**: las notas por su nombre con sus marcas detrás (`C2!`, `D#2/`; `model::suffixes`); el resaltado en vivo de la fase M marca el token entero (`eb/`).
- **Paneles**: en lugar de la forma de onda, el panel del sample (y el grande de la ventana sin editor) dice `sintetizador · pista · acid` y sus parámetros como se escriben, con `∿` en los que siguen una curva (`TrackView::synth`, `model::synth_text`); el de efectos, `♪ acid C2 1/1`; el del motor, `synth acid` en vez de los cortes. `tui --render` dice `pista "acid": synth "acid"`.
- **Tocar**: `App::live_note_on` manda `synth_note_on` con el patch de la pista (pads desde la raíz, MIDI a su altura, sus note off; la grabación, por el mismo camino que la de un instrumento). Editar un paso desde el tracker dice que es un sintetizador.
- **Editor**: autocompletado de `synth("...")` (los tipos con sus parámetros), de su tabla y de los valores de `wave`; ayuda contextual `synth("acid", { ... })` con la descripción de cada clave.
- **Tests**: `tui/src/tests/acid.rs` (tracker y paneles, pads y MIDI en vivo hasta soltar, resaltado del token con su marca), `complete.rs`; `cargo test -p tui pantalla_acid -- --ignored --nocapture` enseña la pantalla.

### Después: el sub (subgrave limpio)

- **Presets genéricos** (`engine/src/synth/preset.rs`): `Preset { name, waves, root, params, build }` y `PresetParam { name, target, scale, min, max, default }` con `Target::Param(índice)` (un parámetro continuo del núcleo: admite curvas), `Target::Attack` o `Target::Release` (la envolvente de amplitud: no). `PresetParam::set`/`get`/`curve`/`moves`; `preset::PRESETS` y `preset::preset(nombre)`. El acid pasa a ser uno (`acid::PRESET`, sin cambiar su sonido) y el script y la interfaz leen cualquiera: `synth_options` toma el patch, las formas de onda, la raíz y los parámetros del preset; `model::synth_text` también.
- **Núcleo**: `Patch::zero_phase` (cada voz que empieza desde el silencio arranca donde su onda cruza el cero subiendo, `Wave::zero_phase`: seno 0, triangular 0,75, sierra 0,5; sin fase al azar) y el parámetro continuo `PRE_DRIVE` (la misma saturación `soft` que `DRIVE`, pero sobre los osciladores, antes del filtro; con 0, nada: el acid suena igual bit a bit). `PARAM_COUNT` pasa a 13.
- **El sub** (`engine/src/synth/sub.rs`): seno (o triangular), mono, una voz, paso bajo de 12 dB/oct sin resonancia (`tone`, 250 Hz), sin envolvente de filtro, amplitud sostenida (ataque 5 ms, sustain 1, release 80 ms), glide de 40 ms, accent de +40 % sin filtro, `level` 0,5, `zero_phase`. Raíz en C1. Parámetros: `tone`, `drive` (el `PRE_DRIVE`), `glide`, `attack` y `release` (estos dos, de la envolvente).
- **Lua**: `synth("sub", { wave, root, tone, drive, glide, attack, release })` y el alias `"subbass"` (`api::synth` mira los alias). `SUB_PARAMS` en el registro; `attack` y `release` no admiten curvas (`ENVELOPE`: se fijan con el sintetizador al cargar; el volumen que se mueve es el de la pista). Receta «Un subgrave limpio» (`scripts/ejemplos/10-subgrave.lua`), con un EQ dinámico con sidechain del bombo.
- **Tests**: `engine/tests/synth.rs` (limpio: los armónicos 2-5 de un seno de 55 Hz por debajo de -60 dB; con `drive` 0,6 el 3 sube a -13 dB y el 21, a 1155 Hz, queda en -79; una línea con silencios, notas pegadas a otra altura, una ligada y el release no salta más que lo que sube un seno de la nota más aguda, y la primera muestra es 0, cuando con la fase al azar sí saltaría; nivel: -6,6 dBFS sin accent, 2,9 dB más con él, por debajo de 0 con `drive` al máximo; todos los presets válidos y dentro del núcleo), `no_alloc.rs` (un sub con `drive`, notas pegadas y ligadas y una rampa de `tone`), `script/tests/synth.rs` (el script con el alias suena igual que el motor; errores de `wave`, rangos, `attack` con curva y claves de otro tipo), `script/tests/registry.rs` (cada sintetizador del registro coincide con su preset; cada preset del motor está en el registro), `tui/src/tests/acid.rs` y `complete.rs` (paneles, tracker y autocompletado del sub).
- **Medido**: la receta (bombo, break y sub) pica en -1,8 dBFS sin entrar en el limitador, y el EQ dinámico baja el sub unos 3 dB en los 30 ms tras cada bombo. CPU del sub: 0,68 % de un núcleo. Sin verificar de oído: si el sub se oye bien (en el portátil, poco: un seno de 33-65 Hz casi no lo reproducen sus altavoces), cuánto `drive` hace falta, y si el ataque de 5 ms y el release de 80 ms son buenos valores por defecto.

### Después: el sh101 (monosynth de bajo y lead)

Seis commits: el pulso al 25 % en el bloque sub; el núcleo (oscilador, sub, ruido, voz mono con legato y portamento, ADSR/VCA); el ladder propio y el paso alto; `quality = 2`; LFO, PWM y deriva; y el registro, las recetas, la guía y esta documentación. El uso, en «`sh101{ opciones }`» y la receta «Un monosynth de bajo y lead (sh101)» de [docs/GUIA.md](GUIA.md).

- **Bloque sub** (`engine/src/synth/mod.rs`, `osc.rs`): `Wave::Pulse25`, el pulso alto un 25 % del periodo con PolyBLEP en sus dos saltos y sin continua (vale 1,5 y -0,5), como cuarta forma de onda del sub (`SUB_WAVES`); los tres modos del SH-101 son `sub_div = 2` y `4` con `"square"` y `4` con `"pulse25"`. El nivel y el pan suavizados del sub y su muestra salen a dos funciones de la voz (`sub_gains`, `sub_tick`), las mismas operaciones para el camino de siempre y el nuevo.
- **Voz analógica** (`Patch::analog: Option<Analog>`, `Voice::render_analog`): un camino propio de la voz, como el de las capas del subharm, así los otros synths no cambian. Un oscilador (`osc[0]`): la sierra y el pulso de la misma fase sumados (`SAW`, `PULSE`, `PW`), el sub de esa fase y ruido, al doble de la frecuencia y diezmados como siempre. Una sola envolvente (la de amplitud: `Target::Decay` y `Target::Sustain` se suman a `Attack` y `Release`) mueve el corte (`ENV_AMOUNT`, en octavas; `env_amt` de -1 a 1 son ±6) y el VCA; con `VcaMode::Gate`, el VCA es una puerta (1,5 ms de subida, 3 ms de bajada) que usa el estado de la envolvente de filtro, aquí libre. La voz se apaga cuando se cierra lo que abre el VCA.
- **Mono con prioridad a la última nota** (`Synth::analog_note_on`, `note_off`): una pila fija de 16 teclas pulsadas (tecla y altura) en el `Synth`. Una nota se liga (`Voice::tie`: solo cambia la altura) si la voz suena sin estar en su release y llega con otra tecla aún pulsada o con slide (en un patrón, las notas solo se solapan con `/`); si no, vuelve a disparar la envolvente desde su nivel. Al soltar la tecla que suena con otras pulsadas, la voz vuelve a la última de ellas, ligada. `GlideMode`: `Off` (ligada, salta a la altura), `On` (también entre notas separadas: `play` desde la altura de la anterior) y `Auto` (solo las ligadas). El glide es el de siempre: un polo en semitonos, el 99 % del camino en `glide` ms.
- **Filtro** (`filter.rs`, `AnalogLadder`): como el `Ladder` (realimentación resuelta en la misma muestra con la cascada lineal), con `soft` también a la entrada de cada etapa, sin iterar: cada etapa es un polo TPT con la entrada acotada a ±1, que no diverge con ningún corte ni resonancia. `k = 4,4·res` (algo más de 4: autooscila aunque las etapas le quiten ganancia). Sin la compensación fija de graves del `Ladder`: la ganancia en continua es 1/(1+k), como en el original; `RES_COMP` multiplica la entrada por `1 + res_comp·k`. `KBD` suma `kbd·(nota − raíz)/12` octavas al corte; `OnePoleHp` tras el filtro con `hpf` 1 a 3 (`HPF_HZ`: 80, 180 y 400 Hz). El corte se acota al 45 % de la frecuencia del motor.
- **`quality`** (`Analog::quality`, 2 por defecto): con 2, el ladder va dentro del bucle de los osciladores al doble, antes del diezmado (el estado del half-band es el de siempre: ningún buffer nuevo); con 1, tras el diezmado, como los otros synths. El paso alto va siempre a la frecuencia del motor.
- **LFO** (`Voice::lfo`): `LfoWave::{Tri, Square, Random}` (el aleatorio, un valor nuevo en cada ciclo de su generador); libre a `LFO_RATE` Hz, o con `lfo_sync = n`, la fase `(pos_en_el_compás · n mod longitud) / longitud` en enteros sobre el reloj del compás (en marcha, el de `SongClock::bar`; parado, frames desde que empezó la voz sobre `bar_length`). Va a la altura (`LFO_PITCH`, semitonos), al corte (`LFO_FILTER`, octavas) y al ancho de pulso con `PwmSource::Lfo` (`PWM_AMOUNT`: con 1, ±0,45; `Env`, la envolvente; `Manual`, nada).
- **Deriva** (`DRIFT`, `Voice::drift_step`): un valor al azar de -1 a 1 cada cuarto de segundo, seguido por un polo de ~0,4 s, por `DRIFT_CENTS` (10 cents). La altura que suena se recalcula cada muestra solo si se mueve (glide, LFO o deriva). Los generadores del LFO y la deriva se siembran en `Voice::start` después de todo lo de antes: las fases y el ruido de los otros synths no cambian.
- **Preset** (`engine/src/synth/sh101.rs`): sierra, filtro a 1500 Hz con res 0,3 y `env_amt` 0,4, `a` 2, `d` 300, `s` 0,6, `r` 100 ms, glide 0 (`off`), `quality` 2, sin accent, `level` 0,8, raíz en C2. `PARAM_COUNT` pasa a 26 (`SAW`, `PULSE`, `KBD`, `RES_COMP`, `PWM_AMOUNT`, `LFO_RATE`, `LFO_PITCH`, `LFO_FILTER`, `DRIFT`); `Target::{Hpf, Quality, LfoSync}` son enteros.
- **Lua y registro**: `sh101{...}` y `synth("sh101", {...})`; `SH101_PARAMS` (28 claves y el bloque sub detrás) con `glide_mode`, `vca`, `pwm_src` y `lfo_wave` por su nombre (`GlideMode`, `VcaMode`, `PwmSource`, `LfoWave`, con `from_name`); `quality`, `hpf` y `lfo_sync` enteros. Por paso, `!` (sin efecto: el SH-101 no tiene accent), `/` (el legato) y `{sub=...}`. Interfaz: el panel enseña `glide_mode`, `vca`, `pwm_src` y `lfo_wave` por su nombre, y oculta el LFO y `pwm_amt` mientras no muevan nada; autocompletado de `sh101{` y de sus valores.
- **Recetas**: `scripts/ejemplos/12-sh101-bajo.lua` (el ejemplo de la especificación en ms, con el break: pica en -1,7 dBFS) y `13-sh101-lead.lua` (en C4, PWM por LFO, vibrato y `kbd`: -2,3 dBFS); «Paso a paso» en la guía (sierra, barrido con resonancia para comparar `quality`, los tres subs, legato con `auto`, LFO al compás).
- **Tests**: `engine/tests/sh101.rs` (legato en vivo y con slide; prioridad a la última; glide en sus tres modos, a los 200 ms a 0,09 semitonos; la puerta del VCA; los tres subs; semilla; autooscilación en 110 Hz, 1 kHz y 18 kHz con quality 1 y 2; estabilidad sin NaN ni inf en los extremos en los dos; caída de graves y `res_comp`; `hpf`; `kbd`; nivel; aliasing de `quality`; LFO a la altura y al corte, sus formas, `lfo_sync` con 1, 3 y 4 ciclos, PWM por sus tres fuentes y la deriva), `engine/tests/synth.rs` (los tres modos del sub en el bloque general, el pulso al 25 % sin aliasing y el coste de CPU), `no_alloc.rs` (un sh101 en un patrón con ligadas, sub al 25 %, LFO al azar sincronizado al PWM, la altura y el corte, deriva, y teclas en vivo ligadas, con quality 1 y 2), `script/tests/synth.rs` (el script suena igual que el motor; 14 errores con la línea), `registry.rs`, `complete.rs` y `tui/src/tests/acid.rs`.
- **Medido**: afinación de la autooscilación, +0,2, +0,6 y +4,3 cents en 110 Hz, 1 kHz y 18 kHz con quality 1 (+0,2, +0,4 y +5,4 con 2); suena a unos -21 dBFS con el preset (9-12 dB por debajo de una nota). Con res 0,8, la fundamental de 55 Hz baja 10,5 dB (con `res_comp` 1, +1,1). `hpf`: -4,9, -10,7 y -17,3 dB a 55 Hz. Aliasing: en un barrido del corte de 500 Hz a 12 kHz con res 0,9 y la entrada saturada en una nota de 2,3 kHz, la energía fuera de los armónicos baja de -25,1 dB (quality 1) a -32,8 (quality 2). Deriva: de -5,7 a +4,3 cents en 4 s con `drift` 1. CPU (release, con todo sonando): **1,04 % de un núcleo con quality 1 y 1,29 % con 2**. Memoria: 24 KiB por pista (antes 23).
- **Decisiones**:
  - **Antialiasing, el de los demás**: PolyBLEP con los osciladores al doble y el half-band de siempre; la sierra y el pulso son las mismas `osc::shape` (medidas en `synth.rs`).
  - **Un camino propio en la voz** en vez de meter el sh101 en el de siempre: lo de antes no se toca (las recetas del acid, el sub y el subharm renderizan idénticas byte a byte en cada commit) y el sh101 no paga lo que no usa (unison, accent, capas).
  - **La saturación de las etapas sin iterar**: resolver la realimentación con las etapas no lineales pediría Newton por muestra; con la predicción lineal y `soft` en cada entrada, la afinación de la autooscilación queda a menos de 6 cents hasta 18 kHz y cada etapa está acotada por construcción.
  - **`k` máximo 4,4**: más realimentación (5, 6) apenas sube el nivel de la autooscilación (las etapas lo limitan) y la desafina (hasta +16 cents a 18 kHz con 6).
  - **Legato en un patrón = `/`**: las notas de un patrón no se solapan salvo con slide (el motor alarga un paso la anterior), así que el `/` de siempre es el legato del sh101; en vivo, las teclas pulsadas.
  - **Nombres**: uno por cosa (`lfo_pitch`, `lfo_filter`; el PWM por `pwm_src` y `pwm_amt`): ni `lfo_amt` ni `lfo_pwm`. `a`, `d`, `r` y `glide` en ms, como en el acid y el sub.
  - **Sin efectos por paso** (`{lp=800}`): como en los demás synths, que no tienen cortes; por paso, `!`, `/` y `{sub=...}`.
  - **El LFO de la voz**: empieza con la voz (desde el silencio), no es global; con una sola voz es lo mismo. `lfo_sync` cuenta ciclos por compás (1 a 16).
- **Sin verificar de oído**: si suena a SH-101 (el carácter del filtro y su saturación, el nivel de la autooscilación, la caída de graves con la resonancia), la diferencia de oído entre `quality` 1 y 2, la curva del glide en `auto`, los tres subs, el vibrato y el PWM, y si la deriva se nota sin desafinar. Ni las teclas en vivo por la tarjeta. ARM64, sin compilar (como siempre: falta la librería estándar de aarch64 en esta máquina).

### Decisiones

- **Código propio y no `fundsp`**. `fundsp` monta grafos de nodos que se construyen con memoria dinámica (`Box`, `Vec` en las redes), y cambiar sus parámetros en vivo pide un `Shared` atómico por nodo; el banco de voces (robar, ligar, reutilizar una voz mono) y las curvas por bloque habría que montarlos fuera igual. Su azar y sus aproximaciones son suyas, y una versión nueva podría cambiar el audio (las huellas de `golden.rs` y el «misma semilla, mismo audio»). Con código propio todo está en arrays fijos de la pista desde `new_engine`, el determinismo depende solo de la semilla y del orden de las notas, y cada pieza se prueba con el render de siempre. Cuesta unas mil líneas. `CLAUDE.md` decía «`fundsp` para synths y efectos cuando toque»: queda como opción para efectos, no para este núcleo.
- **Antialiasing: PolyBLEP al doble y diezmado**, elegido midiendo. PolyBLEP solo deja la sierra a 4 kHz en -30 dB (el residuo de dos muestras no basta tan arriba); las tablas por octava cuestan memoria y su interpolación añade ruido arriba. Con los osciladores al doble y un FIR de media banda, -52,3 dB por 2 veces el coste de los osciladores (el filtro y las envolventes siguen a la frecuencia de salida). Lo que queda (16,2 kHz) es el residuo de PolyBLEP al doble.
- **El ladder en TPT con la realimentación resuelta** (sin el retardo de una muestra de los ladders ingenuos, que desafina la resonancia y se vuelve inestable al moverlo) y **una sola saturación**, a la entrada de la cascada: basta para acotar la autooscilación y dar el color al empujarlo, y cuesta una división por muestra. Sin sobremuestrear el filtro (su saturación aliasa algo con mucha resonancia y el corte muy arriba; no se ha medido).
- **Parámetros por bloque e interpolados por muestra**: evaluar curvas y recalcular coeficientes de envolvente cada 32 frames, como el resto del motor, e interpolar dentro del bloque (el corte en octavas). Sin escalones (sin «zipper») y sin pagar una curva por muestra.
- **Sintaxis `!` y `/`**: libres en la notación (ningún sufijo, ni `^ % @ ' r ~ _ . { }`, ni el `+` de los acordes de `melody`, ni la `#`/`b` de las notas), de un carácter, y se leen como en las secuencias del 303 (acento; ligadura que sube o baja). Van en la nota que llega: `c eb/` es «eb ligada desde c» (en el 303 se marca la de antes; aquí se lee mejor en el tracker y en `melody`, y el motor alarga la anterior un paso para que llegue sonando). Un slide tras un silencio es una nota normal.
- **Mono: reutilizar la voz**. Una nota nueva sin slide vuelve a disparar las envolventes desde su nivel en la misma voz (con la fase de los osciladores siguiendo), como un sintetizador monofónico de verdad: sin robos ni fundidos, sin clics. Los acordes en un mono, error con la línea.
- **El acid como instrumento**: la pista es la de un instrumento con un frame de silencio de sample. Todo lo que ya sabía hacer un instrumento (notas por su nombre, pads, MIDI de entrada y salida, grabación, tracker, resaltado, diagnóstico de `melody`) vale sin tocarlo; solo cambian lo que suena y lo que se ve en el panel del sample.
- **Nivel**: el patch tiene una ganancia de salida (`level`, no se automatiza: para eso está `:gain()`), y el acid la pone en 0,4 (medido): con accent, resonancia y filtro al máximo queda en -2,2 dBFS y sin accent en -10,4. Así un acid con una batería no mete la mezcla en el limitador (la receta, con el break sintético a 0,6, pica en -1,8 dBFS).
- **Coste** (render offline en release, i7-7500U): 8 voces con unison de 7 por el ladder (56 osciladores al doble), **8,6 % de un núcleo**; el acid (una voz, un oscilador), **0,82 %**. Memoria: 19 KiB por pista (152 KiB las 8), reservada al crear el motor; `Track` crece eso. `Step` y `Trigger` llevan dos booleanos más (accent y slide). Sin sintetizadores, nada cambia: `golden.rs` da las mismas huellas.
- **Portabilidad**: Rust puro, sin intrínsecos ni código por arquitectura; solo `f32`/`f64` y sus funciones (`sin`, `tan`, `exp2`, `exp`). La tanh es racional (igual en todas partes); `tan` y `exp2` vienen de la libm del sistema, así que «misma semilla, mismo audio» es en la misma máquina (como las huellas de `golden.rs`, solo en x86_64). No se ha compilado para ARM64: la toolchain de esta máquina no tiene la librería estándar de aarch64.
- **El sub, limpio por construcción**: seno y un paso bajo sin resonancia (nada de lo que hace el acid: sin envolvente de filtro, sin pico), fase en el cero al arrancar (un seno que empieza a media onda da un golpe sordo, más grave cuanto más grave es la nota) y mono reutilizando la voz (una nota pegada sigue la fase de la anterior y no vuelve a subir: nada que haga clic). La saturación va antes del filtro (`PRE_DRIVE`, nuevo en el núcleo) para que `tone` decida cuántos armónicos pasan; la del acid (`DRIVE`) va a la salida y no sirve para esto. Raíz en C1 para que `melody("c")` ya esté en el registro del sub. `"subbass"` como alias porque es como lo llamó el usuario; el nombre corto es el de la tabla.
- **Sin verificar de oído**: si el acid suena a 303 (el carácter del filtro con mucha resonancia, el «chirrido» del accent y su acumulación, la curva del glide y el tiempo del slide de 60 ms), si la saturación del ladder y `dist` suenan bien, el equilibrio entre una nota con accent y sin él, si la triangular y el pulso estrecho suenan limpios, y si el ruido y el sub del núcleo (sin instrumento todavía que los use) sirven. Ni las notas en vivo por la tarjeta (solo con render offline y el motor de tests).

## Fase P: sub-armónicos ✅

Hechas las cinco partes, una por commit: el bloque sub, su documentación,
el núcleo de `subharm`, su redisparo rítmico y sus presets y guía. Antes
de empezar se decidió con el usuario lo que chocaba con la arquitectura
(ver la fase P de [docs/ROADMAP.md](ROADMAP.md)): el sub solo en los
sintetizadores, `{sub=...}` por paso como parámetros de la nota (sin
«por corte»), `div` divide también en los osciladores principales y, con
el transporte parado, `rdiv` cuenta un compás virtual desde la nota.

### Parte 1: el bloque sub

- **Motor** (`engine/src/synth/mod.rs`): el sub de antes (un oscilador aparte a la mitad de la frecuencia) sale ahora de la fase del oscilador central del unison (`n / 2`): `Voice::sub_count` cuenta sus reinicios módulo el divisor, y la fase del sub es `(cuenta + fase del central) / divisor`, en cada muestra del doble de frecuencia. Siempre en fase, sin deriva y siguiendo el glide y el pitch, porque cuelga de esa fase. La forma, con `osc::shape` (la de los osciladores, sacada de `Osc::next`): cuadrada con PolyBLEP de incremento `dt/n` en sus dos saltos, triangular con PolyBLAMP o seno. El contador vuelve a 0 cuando la voz empieza desde el silencio; en una nota pegada de un mono no (reiniciarlo con la fase del oscilador a medias haría saltar el sub: un clic).
- **Parámetros**: `SUB` (nivel, el índice de siempre), `SUB_PAN` y `SUB_DRIVE` (continuos, con curvas: `PARAM_COUNT` pasa a 15); `Patch::sub_div` (1 a `MAX_SUB_DIV` = 16) y `Patch::sub_wave` (`SUB_WAVES`: cuadrada, triangular, seno). Por nota, `SubNote { level, div, wave, pan, drive }` (`NoteEvent::sub`; del paso, `Step::sub` y `Trigger::sub`, validados al construir el patrón): lo que da manda sobre el patch mientras suena esa nota.
- **Suavizado y pan**: el nivel y el pan del sub se suavizan por voz y por muestra (un polo de 5 ms) hacia el del paso o el del patch. Pan de potencia constante (`constant_power`: √2·cos y √2·sin del ángulo, así en el centro cada canal queda como sin pan). Con el pan abierto la voz pasa a estéreo: la primera vez copia el estado del diezmado y del filtro de la izquierda a la derecha (hasta ahí eran iguales), sin salto. `SUB_DRIVE` satura la suma de los osciladores y el sub antes de `PRE_DRIVE` y del filtro, solo con el sub sonando.
- **Con `sub = 0`, nada cambia**: el contador se lleva siempre (no cambia el audio), y la suma, el estéreo y la saturación del sub solo se tocan con el sub sonando; la semilla sigue sacando la fase del suboscilador de antes para que las demás no cambien. Comprobado: las recetas del acid y del sub renderizadas antes y después, idénticas bit a bit, y `golden.rs` igual.
- **Presets** (`engine/src/synth/preset.rs`): `SUB_BLOCK` (sub, sub_div, sub_pan, sub_drive; `Target::SubDiv` para el divisor) se añade a los parámetros de cada preset con `with_sub_block` (una función `const`: la lista está una vez).
- **Notación** (`engine/src/notation.rs`): el resolutor de las llaves devuelve `StepBraces::Fx` (efectos) o `StepBraces::Sub` (el sub de la nota). `script::step_fx::resolve` mira las claves: si todas son del bloque sub, `sub_note` (rangos del motor; `sub_div` entero; `sub_wave` `square`, `tri`/`triangle` o `sine`; sin rampas: la nota lo lee al nacer); si se mezclan con efectos, error; si no, los efectos de siempre (`effects`). Así el diagnóstico en vivo, el tracker y `tui --pattern` los entienden sin saber de qué pista son. En `melody`, las llaves del sub sí valen y las de efectos siguen dando su error (`BRACES_IN_MELODY`, que ahora lo dice).
- **Script** (`script/src/lib.rs`): la tabla de `synth()` lee `sub_wave` y comprueba que `sub_div` sea entero; un `{sub=...}` en una pista que no es un sintetizador, error con la línea.
- **Registro** (`script/src/api.rs`): `SUB_BLOCK_PARAMS` (descripción, rango, valor por defecto, `Auto`: `sub`, `sub_pan` y `sub_drive` continuos; `sub_div` no, `SUB_DIVIDER`; `sub_wave` no, `DISCRETE`), añadido a `ACID_PARAMS` y `SUB_PARAMS` con otra función `const`. La guía genera las tablas; `registry.rs` comprueba que coinciden con el motor.
- **Interfaz**: el tracker marca `{sub}` en la nota que lo lleva; el panel del sintetizador enseña el bloque sub solo si suena o se mueve.
- **Antialiasing**: PolyBLEP en las transiciones del sub y no «aceptarlo en graves»: el sub de una nota aguda (div 2) está en los agudos, cuesta unas operaciones por muestra y no reserva nada. Medido: la cuadrada del sub a 2 kHz, -56 dB (test bajo -50).
- **Tests** (`engine/tests/synth.rs`): f/n contando cruces por cero del sub solo (la diferencia con y sin él: el paso alto sin resonancia es lineal) con n = 1, 2, 3, 5, 7 y 16, en seno y cuadrada; sigue el glide (en una nota que se desliza una octava, el oscilador hace 3 veces los ciclos del sub con n = 3); `sub = 0` con divisor, forma, pan y saturación cambiados da el audio por defecto bit a bit en tres patches; pan de potencia constante (L² + R² a ±1 % con seis pans, y nada a la izquierda con 1); cambios de sub entre notas pegadas y al abrir el pan, sin saltos; el aliasing. `no_alloc.rs` (el acid con el sub de algunas notas y el polifónico con unison de 7, sub abierto, triangular y dividido por 3), `script/tests/synth.rs` (tabla con curva y por paso en `melody`, nueve errores con la línea, en una pista de cortes), `script/tests/step_fx.rs`, `tui` (tracker y panel).

### Parte 2: documentación

- Guía: la fila de `{sub=...}` en la tabla de la mini-notación, «Un sub dentro del bajo (bloque sub)» en las recetas (el bloque de Lua lo carga `guide.rs`) y la sección generada de la API.

### Parte 3: `subharm`, núcleo

- **Capas** (`engine/src/synth/mod.rs`): `Patch::layers: Option<[Layer; LAYERS]>` (`LAYERS` = 6, `LAYER_NAMES` `o1 o2 s1 s2 s3 s4`, `LAYER_PARENT`), `Layer { div, rdiv, pan, level }`. Con capas, `Voice::layers_sample` toca, en cada muestra del doble de frecuencia, `osc[0]` y `osc[1]` a `dt / div` (la nota dividida: `div` divide también aquí) y los subs de cada uno con la fase `(cuenta + fase del oscilador) / div`, la cuenta de sus reinicios por sub (`layer_count`), como el bloque sub: siempre en fase con su oscilador, también con glide. Cada capa, con su nivel y su pan suavizados por muestra (`layer_level`, `layer_pan`, el polo de 5 ms) y pan de potencia constante; la voz es siempre estéreo; luego el camino de siempre (filtro por canal, amplitud, saturación). Sin unison ni bloque sub. El camino sin capas no cambia (el código está en la otra rama del `match`).
- **Preset** (`engine/src/synth/subharm.rs`): sierra (o cuadrada), ladder, raíz C2, `DEFAULT_LAYERS` (`o1` a la nota, `s1` una octava abajo a 0,6; las demás calladas con divisores de partida), `level` 0,3, accent +50 % y una octava de filtro. Parámetros: `cutoff`, `res`, `envmod`, `fdecay`, `glide`, `drive`, `attack`, `decay` y `voices` (`Target::Voices`: con 1, mono; hasta 8, polifónico, cada voz con sus seis capas).
- **Lua**: `subharm{ ... }` (y `synth("subharm", { ... })`): `layer_options` lee cada tabla de capa (`div` y `rdiv` enteros de 1 a 16, `pan` y `level`), con errores con la línea; las capas no admiten curvas (`LAYER`: cambian con el patrón, al compás). Un `{sub=...}` en un subharm, error (sus subs son las capas). Registro: la función `subharm`, `SUBHARM_PARAMS` y `LAYER_PARAMS`/`LAYER_TABLE` (la guía genera su tabla). El autocompletado entiende `nombre{ ... }` como una llamada (`Frame::Paren::braced`) y ofrece las capas y, dentro de una, sus claves. El panel enseña las capas que suenan (`s1 ÷3 ↻3 pan 0.5 level 0.7`).
- **Tests**: `engine/tests/synth.rs` (cada capa a su frecuencia, contando cruces, con siete combinaciones de capa y divisores; los subs atados a su oscilador durante un glide de una octava; pan de potencia constante en las seis capas; cambios de capas a media nota sin saltos; 8 voces con todo a tope dentro de ±1 tras el limitador), `no_alloc.rs` (un subharm de 8 voces con las seis capas, divisores y `rdiv` distintos, acordes), `script/tests/synth.rs` (`subharm{}` igual que `synth("subharm")` e igual que el motor; quince errores con la línea), `tui` (autocompletado y panel).

### Parte 4: redisparo rítmico (`rdiv`)

- **El reloj del compás** (`engine/src/sequencer.rs`): `Transport::current_bar` (su primer frame y su longitud, enteros) se fija al disparar el paso 0 de cada compás, con el tempo y el compás que entran ahí (rampas y otros compases incluidos); `bar_length` es lo que dura un compás al tempo de ahora. `SongClock` lleva al sintetizador el frame del transporte, el compás que suena (en marcha) y esa longitud (parado). El motor parte el audio en los pasos, así que un trozo nunca cruza dos compases (y aun así la voz lo aguanta).
- **En la voz** (`Voice::layer_rhythm`, en cada muestra): cada capa tiene su envolvente (`layer_env`: ataque y decay hasta 0, `LAYER_ATTACK` y `LAYER_DECAY`, parámetros continuos nuevos con curvas; `PARAM_COUNT` = 17) que vuelve a sonar desde su nivel cuando la posición en el compás llega a `(j + 1) · longitud / rdiv` (enteros: sin deriva). Al empezar un compás, todas en su marca 0. Una nota nueva las hace sonar a todas y sigue la rejilla desde su posición (`j = pos · rdiv / longitud`). Con el transporte parado, un compás virtual (`virtual_pos`, `virtual_bar`) que empieza con la nota, al tempo de ahora. Es redisparo de envolvente, sin memoria de audio: nada de delay.
- **Lote atómico**: el `rdiv` (y el `div`, el pan y el nivel) van en el patch del patrón, que entra al compás.
- **Tests** (`engine/tests/synth_track.rs`): con el motor de verdad, los golpes de una capa caen en `j · compás / rdiv` con `rdiv` 3, 4, 5 y 7 en dos compases (cada uno entre su frame y 48 frames después: lo que tardan el diezmado y el ataque de 0,5 ms en asomar), 3 contra 4 y 5 contra 7 durante cuatro compases (la unión de los dos, sin deriva), una nota en vivo con el transporte parado (compás virtual desde la nota), una nota a medio compás (suena ya y sigue la rejilla) y un `rdiv` nuevo que entra al compás siguiente.

### Parte 5: presets, guía y documentación

- Receta «Poliritmos de subarmónicos» (`scripts/ejemplos/11-subharm.lua`: tres contra cuatro con un bombo), con un fragmento de tutorial paso a paso (de una capa a cuatro, y acordes); todo lo carga `guide.rs`. La receta renderizada pica en -3,7 dBFS sin entrar en el limitador.

### Decisiones (subharm)

- **Capas en el patch, no instrumentos aparte**: el banco de voces, el robo, el mono con legato, el filtro, las curvas y la integración (melody, pads, MIDI, tracker, resaltado) ya estaban; las capas solo cambian lo que suena dentro de la voz.
- **El reloj, del transporte**: la voz no sabe del secuenciador, así que el motor le pasa el compás que suena en cada trozo. Las marcas se calculan desde su inicio con enteros y cada compás vuelve a empezar: los poliritmos cuadran al final de cada compás aunque cambie el tempo.
- **Una nota a medio compás suena ya** (todas sus capas) y luego sigue la rejilla del compás, no una rejilla propia: así varias notas y varias pistas comparten el mismo ritmo.
- **Polifonía**: cada voz lleva sus seis capas con todo su estado en el `Track` (medido: 23 KiB por pista, 4 KiB más que antes de la fase P, unos 250 bytes por ranura de voz; 186 KiB las 8 pistas, reservados al crear el motor). Medido (release, i7-7500U): 8 voces con las seis capas sonando y redisparándose, 11,7 % de un núcleo (1,47 % por voz). No hace falta bajar el límite de 8 voces por pista; lo que no cabe en este portátil son muchas pistas así a la vez (8 pistas × 8 voces llegarían a casi un núcleo entero).
- **Sin verificar de oído**: si los ritmos se oyen como se espera (con auriculares), el equilibrio entre capas y el nivel por defecto, si el ataque de 2 ms y el decay de 800 ms son buenos de partida, y cómo suenan los subs muy divididos (`div` 11-16, por debajo de lo audible con notas graves).
## Fase Q: MPE y expresión por nota (a medias)

Hecho lo que no depende del aparato: los mensajes de expresión según la
especificación MIDI, las zonas MPE según la especificación MPE, el motor,
Lua, la guía y una receta. **Falta comprobarlo con el Seaboard**: el
usuario no podía tocarlo; cuando pegue la salida de `aseqdump -p 24:0`, se
anota aquí (canales, rango de bend, CC74, presión de canal o por nota) y se
ajusta lo que no cuadre.

### Los mensajes y las zonas

- `tui/src/midi.rs`: `PitchBend` (14 bits, LSB primero, centrado en 0: de -8192 a 8191), `ChannelPressure`, `PolyPressure` y `Controller`, con canal; un byte de datos con el bit alto no es un mensaje.
- `tui/src/expression.rs` (`Expression`): el estado de cada canal (bend de -1 a 1, presión y slide de 0 a 1) y las notas que suenan con su canal. Con MPE (zona baja), lo de un canal es de su nota; el bend del canal 1 (el maestro, ±2 semitonos) se suma a todas; lo que el canal trae antes de la nota es con lo que empieza (`note_on` devuelve sus ejes). Sin MPE, lo de un canal va a todas sus notas y la presión por nota a la suya. El bend en semitonos: `[midi] bend` (48 con MPE, 2 sin él), acotado a ±96 (`engine::MAX_BEND`). `coalesce` deja el último cambio de cada eje de cada nota: lo que llega entre dos vueltas de la interfaz no satura la cola del motor (64 órdenes).
- `App`: con `mpe = true`, `canal` no filtra. La nota MIDI de un instrumento sale con `synth_note_on_expr`; los cambios esperan en `expr_changes` y se mandan en `poll_midi` y `tick` (`note_mod` a la pista de la nota en vivo, por su tecla).
- **Config**: `[midi] mpe = true|false` y `bend = 1..96`.

### El motor

- `engine::synth`: `AXES` = 3 (`BEND` en semitonos, `PRESSURE` y `SLIDE` de 0 a 1), `ExprTarget` (`Pitch`, `Gain`, `Param(k)`) y `ExprMap { target, range, curve }` en `Patch::expr`. Lo que suma un eje: `range · signo(x) · |x|^curve`, con `x` el eje (el bend, entre 12: una octava de bend es 1). Un parámetro logarítmico (el corte) se mueve en octavas; los demás, en sus unidades; siempre dentro de su rango. `Gain`: `1 - range + |suma|`, entre 0 y 1. Solo los parámetros que la voz lee muestra a muestra (`expressive`: no el decay del filtro, el glide, el desafinado, la apertura ni las envolventes de las capas, que se fijan por trozo).
- Cada voz lleva `expr` (suavizado) y `expr_goal`. En `mix_into`, si el patch asigna algún eje y la voz es expresiva, la voz se toca con una copia del bloque con sus parámetros movidos al principio y al final del trozo (interpolados muestra a muestra como siempre), sus semitonos de bend y su factor de volumen; el bend se suma a la altura (con glide; en la voz analógica, a la del LFO y la deriva). Suavizado: un polo de 8 ms por trozo de 32 frames (`EXPR_SMOOTHING_SECONDS`).
- **Solo las notas con expresión la siguen** (`Voice::expressive`, que pone `Synth::set_expr`): las del patrón y los pads del teclado suenan como sin `:mpe` (si no, con presión al volumen callarían). La expresión inicial entra sin suavizar en una nota nueva (`expr_fresh`) y suavizada en una ligada.
- `Command::SynthOn` lleva `expr: Option<[f32; AXES]>` (`None` en `synth_note_on`); `Command::NoteMod { track, key, axis, value }` (`Controller::note_mod`), inmediato. La voz se identifica por la tecla, como el note off: el motor no puede devolver un id sin esperar al hilo de audio, y en MPE cada nota tiene su tecla (dos dedos en la misma nota de canales distintos comparten la expresión de la última).
- Sin ejes asignados, nada cambia: el camino de siempre (`golden.rs` igual).
- **Tests**: `engine/tests/expression.rs` (el bend inicial suena desde el principio; un bend mueve solo la voz de su tecla, las dos juntas suenan como cada una sola; una octava de golpe sin clics; la presión abre el filtro de su nota y la curva; presión al volumen sin clics, y con `range` 0,5 a la mitad; el slide y los ejes sin asignar, bit a bit lo de siempre; el sh101 mono; los pads sin expresión; errores), `no_alloc.rs` (8 voces con bend, presión y volumen cambiando en cada bloque).

### Lua, guía y receta

- `pista:mpe{ bend = ..., pressure = ..., slide = ... }` (`mpe_options` en `script/src/lib.rs`): cada eje a `"pitch"`, `"gain"` o un parámetro del preset que vaya a un parámetro expresivo del núcleo; como texto o como tabla `{ destino, range = n, curve = n }`. Rangos por defecto: 12 el bend al pitch, 2 lo demás al pitch, 1 el volumen, 2 octavas los logarítmicos y medio rango (en las unidades del preset, por su escala) los demás; `curve` 1. `false` quita un eje; `:mpe()`, todos. Solo en sintetizadores. Errores con la línea.
- Registro (`api::FUNCTIONS`, `mpe`), la guía (sección generada, «MIDI» de la sección 2 y la receta «Tocar con expresión por nota (MPE)») y `scripts/ejemplos/14-mpe.lua` (un `subharm` de seis voces; renderizada, pica a -3,3 dBFS). Tests: `script/tests/mpe.rs` (los ejes con sus valores por defecto y sus unidades, las notas del patrón bit a bit iguales con y sin `:mpe`, nueve errores con la línea, una pista de cortes), `registry.rs`, `guide.rs`; en la interfaz, `tui/src/expression.rs` (zonas, maestro, sin MPE, rango, `coalesce`) y `tui/src/tests/mpe.rs` (de mensajes MIDI simulados al audio: el bend que trae el canal antes de la nota, otro canal no la mueve, el suyo sí; la configuración).

### Decisiones y pendientes

- **El bend va en semitonos al motor**: el rango es del controlador (la configuración), no del sonido. El bend a otro parámetro cuenta una octava de bend como el eje entero.
- **El synth polifónico, por decidir con el usuario**: la receta usa `subharm{ voices = 6 }`, que ya existía; `acid`, `sub` y `sh101` son mono (la expresión es la de la nota que suena).
- **Sin verificar con el aparato**: canales que usa el Seaboard, si manda la presión como aftertouch de canal o por nota, su rango de bend (y si lo manda por RPN 0) y si el slide es el CC74. Tampoco se ha oído.
- Pendiente opcional: `scripts/conectar-seaboard.sh`.

## Fase R: editor con ratón y modo vim (fase 1) ✅

### El ratón

- `run_terminal` pide la captura del ratón (`EnableMouseCapture`, y la quita al salir con los demás modos en `TerminalModes`) si `[editor] raton` no es `false`. Los eventos de crossterm pasan a `app::MouseAction` (`Down`, `Drag`, `Up`, `ScrollUp`, `ScrollDown`, solo el botón izquierdo) y `App::mouse(acción, columna, fila)` (`tui/src/app/mouse.rs`), que no sabe de crossterm: así se prueba.
- `ui::draw_editor` guarda dónde dibujó el texto (`App::editor_area`; `None` en la principal con el editor fuera). La celda pulsada pasa a fila y columna del texto con `TextArea::screen_to_data`, sumando `scroll_offset` y quitando `line_number_width`: con el texto desplazado en vertical y en horizontal (el editor no parte las líneas largas; se desplaza). Sobre los números de línea, la columna 0; más allá del final, el final de la línea.
- Clic: cursor ahí, sin selección (cierra el autocompletado y olvida los campos de la plantilla, como moverse con las teclas). Arrastrar: la selección empieza en el clic en cuanto el ratón se mueve a otro carácter; fuera del texto por arriba o por abajo, desplaza una línea. Rueda: tres líneas (`Scrolling::Delta`).
- Solo en el modo editor y sin nada encima (`pastes_in_editor`: la paleta, la línea de órdenes, los selectores...). En la ventana del editor de `--split`, igual (dibuja con el mismo `draw_editor`).
- Con la captura, la terminal deja de seleccionar sola: Shift y arrastrar (la guía lo dice). Dentro de tmux, si no llegan los clics, `set -g mouse on` (sin verificar aquí: no hay tmux en esta máquina).
- **Tests** (`tui/src/tests/mouse.rs`): clic en un carácter, más allá del final, sobre los números y fuera del editor; con el texto desplazado y la rueda; en una línea larga desplazada a la derecha; arrastrar (sin moverse no hay selección; a la derecha y a otra línea; Ctrl-C copia; un clic la quita); nada en el modo tocar, con la paleta ni con `raton = false`; en la ventana del editor sí y en la principal sin editor no.

### El modo vim

- `tui/src/app/vim.rs`: `Vim { mode: Normal | Insert, pending, linewise, exit, half_exit }` en `App::vim` (`None`, el editor de siempre). `App::handle` lo consulta en el modo editor, tras la paleta y el cursor del tracker y antes del autocompletado: en Normal, las letras sin Ctrl ni Alt (y Enter, Retroceso, Tab, Supr) son órdenes; lo demás (los atajos, las flechas, Esc) sigue el camino de siempre. En Insert, todo sigue el camino de siempre salvo la segunda tecla de salida.
- Órdenes sobre `ratatui-textarea`: los movimientos con `CursorMove` (`h` y `l` sin cambiar de línea ni pasar del último carácter, `$` sobre el último, `G` y `I` al primer carácter que no es espacio); `o`, `O`, `x`, `dd`, `p` y `P` dentro de `App::grouped` (un solo paso de deshacer); `dd` corta con una selección (la línea y su salto; la última, desde el final de la anterior; la única, se vacía); `yy`/`dd` copian la línea al portapapeles del widget y marcan `linewise`, y `p`/`P` la ponen en una línea propia; `x` copia su carácter. `u` y Ctrl-R son deshacer y rehacer de la app; `/`, `n` y `N`, las acciones de buscar (`buscar ` en la línea de órdenes, F6, F7); `:` abre la línea de órdenes con `:`, y `run_command` manda a `vim_command` lo que empieza por `:` (antes que a la principal, en la ventana del editor): `:w` es `save` (guardar y cargar); lo demás, un aviso.
- **Salir de Insert**: Esc va al modo tocar (decidido por el usuario), así que salen dos teclas seguidas, `jk` por defecto (`[editor] salir_insert`, dos letras): la primera se escribe; si la siguiente es la segunda, se borra la primera, se vuelve a Normal y el cursor va a la izquierda, como en vim. Sin tiempo de espera (todo determinista). **Ctrl-[ no puede ser**: la terminal lo manda como el byte de Esc (0x1B); solo con el protocolo de kitty llegaría distinto.
- El modo se ve en el borde del editor (`NORMAL`, `INSERT`; `ViewModel::vim`), también en la ventana del editor. «Modo vim» en la paleta lo enciende o apaga en la sesión (en la ventana del editor, allí). `[editor] modo = "vim" | "clasico"`.
- **Tests** (`tui/src/tests/vim.rs`): los movimientos y que Normal no escribe; Insert por `i a A I o O`, `jk` (una `j` sola se queda); `dd` en medio, al final y en la única línea, `p`, `P`, `x`, `yy`, deshacer y rehacer; buscar, `:w` y una orden que no está; Esc al modo tocar desde los dos modos y Ctrl-S en Normal; la configuración (`salir_insert = "fd"`, errores) y la paleta. Las demás pruebas de teclas (paleta, autocompletado, Alt-G, Ctrl-F, F6/F7, pantalla, guía) siguen igual: el modo por defecto es el de siempre.

### Decisiones

- **`jk` en vez de preguntar**: el prompt pedía preguntar entre `jk` y Ctrl-[; Ctrl-[ es imposible de distinguir de Esc sin el protocolo de kitty, así que queda `jk`, configurable. Por confirmar con el usuario.
- **Sin cuentas ni operadores con movimiento** (fase 1): `dd` y `yy` son órdenes de dos letras, no operador más movimiento; `gd`, `dw`... dan un aviso.

## Fase S: transporte con atajos y cue ✅

Ctrl-Espacio arranca y para; Shift-Ctrl-Espacio (o F9) fija un cue, el
compás en que empieza el siguiente arranque; el motor arranca en un compás
cualquiera como si la canción hubiera llegado hasta él; `--from` hace lo
mismo en el render offline. Hecha antes que la Q y la R, a petición del
usuario.

### El teclado

- Sin ayuda de la terminal, Ctrl-Espacio y Shift-Ctrl-Espacio llegan como el mismo byte NUL (crossterm lo da como `Char(' ')` con Control). `run_terminal` (`tui/src/main.rs`) pedía solo `REPORT_EVENT_TYPES`, y solo si `supports_keyboard_enhancement()` decía que sí; ahora pide además `DISAMBIGUATE_ESCAPE_CODES` y `REPORT_ALTERNATE_KEYS`: con el protocolo de kitty, Shift-Ctrl-Espacio llega con Shift. En tmux (que no responde a esa consulta) llegan distintas con `extended-keys always`, `extended-keys-format csi-u` (tmux 3.5) y `terminal-features ...:extkeys`: tmux manda `CSI 32;6u`, que crossterm entiende siempre. No se pide `modifyOtherKeys` (`CSI > 4;2m`): fuera de tmux, xterm contestaría con `CSI 27;...~`, que crossterm no entiende.
- Esta máquina de desarrollo no tiene tmux instalado, así que la combinación dentro de tmux no se ha probado de verdad; F9 hace lo mismo (y se puede atar otra en `[teclas]`).
- **Configuración** (`tui/src/config.rs`): `KeyCombo` tiene `shift` y la tecla `space` (o `espacio`; sola, error: escribiría texto). Se ve como `Shift-Ctrl-Espacio`. Una combinación sin `shift` vale también con Shift (las de siempre no cambian si la terminal empieza a decirlo), y `Keymap::lookup` busca antes las que lo piden: Shift-Ctrl-Espacio es el cue aunque Ctrl-Espacio sea marcha. `marcha` = `ctrl-space`, `ctrl-t`, `f8`; `cue` nueva, `shift-ctrl-space` y `f9` (F5 ya era cargar).

### El motor

- `Command::StartAt(bar)` y `Controller::start_at(bar)` (`MAX_START_BAR` = 65 536; más, `SendError::InvalidStartBar`). `Start` es `start_at(0)`, el mismo código (`Engine::start_at`).
- `Transport::start_at(bar, rate)` (`engine/src/sequencer.rs`): `bar` y `steps_before` (`bar` · pasos del compás) como si hubieran pasado esos compases, y `frame`/`bar_start` con los frames de los compases de antes, con el tempo, la rampa y el compás que van a sonar (los pendientes si los hay: al arrancar entran en el paso 0). Con rampa de tempo, el bucle es de sus compases como mucho (los de después, multiplicados): acotado y sin reservas. Así los patrones (que miran la semicorchea global), las curvas (posición en pasos y en segundos) y el redisparo del subharm (`current_bar`) siguen por donde irían. `first_bar` guarda dónde se arrancó: una rampa de tempo que entra al arrancar empieza en el compás 0 de la canción, no en el de arranque (como `Song::bpm_at`).
- **Línea de tiempo** (`engine/src/arrangement.rs`): `Arrangement::place_at(bar)` (da la vuelta al acabar, como el motor). Al arrancar (`playlist.restart`, o un arreglo que entra en el primer compás), `advance_playlist` va a esa entrada y ese compás, con el origen de la entrada (`origin_step`, `origin_frame`) los compases de antes: sus vueltas y sus curvas, por donde irían.
- **Tests**: `engine/tests/start_at.rs` (la vuelta del ciclo que cae; el compás tras `start_at` bit a bit igual que llegando cuando lo de antes no deja cola; `start_at(0)` = `start`; la rampa de tempo a mitad y pasada y la posición; con línea de tiempo, al principio de una entrada, a media entrada, pasado el final y con un arreglo que ya sonaba; el tope), `no_alloc.rs` (con rampa de tempo y compás de 7/8, y con secciones a media entrada), `script/tests/start_at.rs` (un `:gain(ramp(...))` sigue igual que llegando; `bpm_ramp` y `every` en el compás 4).

### La interfaz

- `App::cue: Option<u64>` (`tui/src/app/cue.rs`), un compás contando desde 0 como la barra de estado. `set_cue`: en marcha, el compás que suena (`Status::bar`); parada, el del cursor del tracker (su fila en la vuelta que se ve: `vuelta · longitud + fila`, entre los pasos del compás) o, sin cursor, donde se paró; con secciones, siempre el principio de una entrada (la que suena o en la que se paró). `toggle_transport` arranca con `start_at(cue)`; el cue se queda hasta «Borrar el cue» (paleta): cada arranque vuelve a él.
- Se ve en la barra de estado (`⏮ cue compás 8`, `⏮ cue drop (entrada 2)`; `ViewModel::cue`) y en la línea de tiempo, delante de su entrada (`TimelineView::cue`). Ctrl-Espacio y el cue funcionan en el modo tocar y con el cursor del tracker abierto (como marcha). En la ventana del editor van a la principal como las demás acciones del motor (`ToMain::Action`), y el estado que manda la principal lleva el texto del cue (un campo más en `estado`, opcional para una principal de antes), que el editor enseña en su línea del enlace.
- `--from N` (con `--script` y `--render`, sin `--track`): `start_at(N)` y los frames de `N` a `N + bars` con `Song::frames` (la rampa y el compás de la canción). Sin `--bars`, con línea de tiempo, hasta su final (si `N` se pasa, error).
- **Tests**: `tui/src/tests/transport.rs` (las teclas: Ctrl-Espacio marcha, con Shift el cue, F9, Ctrl-T con Shift sigue siendo marcha, cómo se leen y se ven; marcha/paro desde el editor sin escribir un espacio y desde el modo tocar; el cue en marcha, en la barra de estado, cada arranque vuelve y borrarlo; parada con el cursor del tracker en una pista de 32 pasos; con secciones, la entrada y la marca en la línea de tiempo), `tests/split.rs` (Ctrl-Espacio y el cue desde la ventana del editor), `link.rs` (el campo nuevo de `estado`). Comprobado a mano con el render: `--from 4` de una pista con `every(4, ...)` toca la variante, `--from 3` y `--from 5` no.

### Decisiones

- **El cue es un compás, no un paso**: el motor arranca en el paso 0 de un compás (los cambios entran al compás; arrancar a media vuelta no tiene sentido para ensayar). Con secciones, una entrada: es lo que se ensaya.
- **Persistente**: un cue de DAW, cada arranque vuelve a él hasta que se borra. Un `Start` de reloj MIDI de entrada sigue arrancando desde el principio (es lo que dice MIDI).
- **Las colas de antes no suenan**: arrancar en el compás N no reproduce los reverbs ni las notas largas que vendrían del N - 1. Lo que sí sigue es todo lo que depende de la posición: vueltas, `every`, `chance` (que se sortea al cargar, por compases), curvas, rampa de tempo y sección.
- **F9, no F5**: F5 ya era cargar.
- **Sin verificar**: Shift-Ctrl-Espacio dentro de tmux en la máquina del usuario (aquí no hay tmux).

## Fase T: vista de ocupación por paso ✅

Pedida por el usuario para ver los huecos entre pistas (breakcore, sonidos que se alternan). Hechas las cuatro partes, una por commit. El uso, en «Vista de ocupación» de [docs/GUIA.md](GUIA.md). El motor no cambia: todo sale de lo que ya publica (el cursor de cada pista de la fase M), así que `no_alloc.rs`, `golden.rs` y el protocolo del socket siguen igual.

### Parte 1: la rejilla

- **Antes**: el tracker enseña una pista por columna y un paso por fila, pero solo la vuelta que suena de cada una y sin cruzarlas; nada contaba cuántas pistas suenan en cada paso.
- **El cálculo** (`tui/src/occupancy.rs`, puro): `occupancy::grid` construye una página (`Grid`) con una fila por pista que suena (las de la sección, en su orden) y una columna por semicorchea de la canción. La página tiene los compases que hacen falta para que todas vuelvan a coincidir (`page_len`: el mínimo común múltiplo del compás y de `longitud · vueltas` de cada pista; si pasa de `MAX_PAGE` = 64, el del compás y las longitudes; si ni así, 64), alineada al principio de la sección, y es la que contiene el paso que se oye. Cada pista se coloca con su último cursor del motor (`Cursors::at` en el paso que se oye, o `Cursors::last`): con la etiqueta, la definición que de verdad suena (`Song::track_by_tag`) y, con su índice, en qué punto de su ciclo aplanado va en cada columna (`índice + (columna − semicorchea del cursor)`, módulo `longitud · vueltas`). Así valen las longitudes distintas, las vueltas de `every`/`chance`, las secciones y `start_at`. Parado (o sin cursor, o con una carga nueva que aún no ha entrado), como el tracker: `TrackDef::lap` desde el principio de la sección. Cada celda (`Cell`) sale de `TrackDef::bars`, el ciclo que recibe el motor: `Hit` (intensidad de 0 a 3 por el volumen del paso, `level`, y su retrigger), `Held` (sigue una nota alargada con `_`), `Dropped` (el patrón base tiene un golpe y esta vuelta no) o `Silence`; `Row::at` guarda la vuelta y el paso de cada columna. `Grid::counts`, cuántas pistas que se oyen (sin mute ni solo de otra) suenan en cada paso (golpe o nota alargada); `gaps`, los pasos a 0; `current`, la columna del paso que se oye (`Status::heard` menos `[resaltado] retardo_ms`, como `App::highlights`), solo en marcha.
- **En la aplicación** (`tui/src/app/occupancy.rs`): `App::occupancy_grid` la calcula en `App::view` (`ViewModel::occupancy`) solo si se ve (`Occupancy::visible`, apagada por defecto).
- **El panel** (`ui::OccupancyPanel`): arriba, el número del compás donde empieza cada uno, `:` en los tiempos y `▼` en el paso que se oye; una fila por pista con su nombre (hasta 8 caracteres) en su color del resaltado (`Theme::highlight`, por el orden de `Song::track_names`, como el resaltado; apagada la que no se oye); debajo, `Σ` con la cuenta de cada paso (`+` desde 10) y los pasos a 0 en negro sobre `Theme::gap`. La columna del paso que se oye, sobre el color del paso que suena (`Theme::playing`). Dos celdas por paso si caben; si no, una; si ni así, se desplaza (de 4 en 4) para que se vea el paso que se oye, y el título dice qué pasos se ven. Si no caben las pistas, las de alrededor de la elegida (y el título lo dice). El título dice los compases y cuántos huecos hay.
- **Tests**: `tui/src/tests/occupancy.rs` (con el motor offline: apagada por defecto; la cuenta y los huecos de dos pistas; `@`, `rN`, `_`, `:length(12)` contra 16 en una página de 48 y `euclid`, que no es texto, se ve; `every` que quita golpes y `chance(1, "mute")`, `×` que no cuentan; el paso que se oye con y sin retardo de salida y la página siguiente al cambiar de compás; una pista en mute, apagada y sin contar; el panel a dos celdas, a una, desplazado y bajo) y los unitarios de `occupancy.rs`.

### Parte 2: dónde se ve

- **Con una sola ventana** (`ui::draw`): en la columna derecha, entre el motor y el sample. Pide todo su alto (`OccupancyPanel::height`: una fila por pista, la de los compases, la de `Σ` y los bordes) si al sample le quedan 4 filas y al tracker 7 (`occupancy_height`); si no, lo que dejen, y con menos de 5 filas, 5 de todos modos (lo demás, apretado). El sample se aprieta a 4 filas y el tracker se queda lo que sobre (hasta sus 19) y se desplaza como siempre. Sin la vista, el reparto de siempre (el mismo `Layout`, para que no se mueva ni una fila).
- **Con el editor en su ventana** (`ui::draw_main_window`): en la principal, de lado a lado encima de la forma de onda grande y de la tabla de choques, con el mismo reparto (la onda se aprieta a 6 filas).
- **Paleta y teclas**: «Vista de ocupación» (`Action::Occupancy`, `App::toggle_occupancy`; atable en `[teclas] ocupacion`) la abre y la cierra. Desde la ventana del editor no es de `editor_side`, así que va a la principal por el `accion` de siempre (las acciones viajan por nombre): el protocolo no cambia, sigue en la versión 3.
- **Configuración**: `[ocupacion] visible` (`Config::occupancy`, `OccupancyConfig`; por defecto, cerrada) y dos colores en `[colores]`: `hueco` (`Theme::gap`, rojo oscuro 88 de la paleta de 256) y `sugerencia` (`Theme::suggestion`, verde 28, para la parte 4). Las tablas de la guía salen de `ACTIONS`, `BINDABLE`, `theme::NAMES` y `OCCUPANCY_KEYS`.
- **Tests** (`tui/src/tests/occupancy.rs`): la paleta la abre y la cierra, a 120×40, 100×30 y 80×24 (en la columna derecha, encima del sample; a 80×24, con una pista y sin sitio para el tracker); la configuración la abre al arrancar con su color de hueco; con el editor en su ventana, la acción desde el editor la abre en la principal, de lado a lado, a 120×40 y 80×24. `config.rs`: la sección, sus colores y dos avisos. `tracker_rows.rs` y `main_window.rs` siguen igual (sin la vista, el reparto de siempre).

### Parte 3: rellenar desde la vista

- **Lo mínimo que cumple** (`tui/src/app/occupancy.rs`), con la técnica del tracker editable: «Rellenar un hueco desde la vista de ocupación» (`Action::FillGap`) abre la vista y su cursor (`Occupancy::cursor`, `OccupancyCursor { row, col }`: pista y columna de la página) en la pista seleccionada y el primer hueco desde el paso que se oye. ←→ paso, ↑↓ pista (la seleccionada lo sigue: elegir la pista es elegir de quién es el golpe que entra), Tab el hueco siguiente (`Grid::gaps`), Enter abre la línea de órdenes con `rellenar ` y el golpe que se propone (`proposed_hit`: el anterior de esa pista en esa vuelta, como está escrito y sin sus llaves, `with_step_fx(token, "")`; si no hay, `1`), Esc sale; deshacer, rehacer, cargar, marcha y salir siguen funcionando. Al pulsar Enter se guardan la pista, la vuelta y el paso (`PendingFill`, de `Row::at`), aunque la página siga. La orden `rellenar <golpe>` (`run_fill_command`, `write_fill`) comprueba el golpe antes de tocar el texto (`parse_step_with` con las etiquetas de la pista y que el corte exista: una errata solo da el mensaje), sustituye los caracteres del `.` por el golpe dentro de `App::grouped` (un paso de deshacer) y carga como Ctrl-L (entra al compás siguiente; si fallara, sigue sonando lo anterior y se ve el error).
- **Lo que no se rellena, y por qué** (`fill_target`, `silence_span`; lo dice la línea de estado al mover el cursor y al pulsar Enter): un golpe que ya suena (eso es del tracker), un paso que sigue una nota alargada (`_`), un golpe que `chance`/`every` quitan en esta vuelta (`×`: está escrito, no es un silencio), un paso sin posición en el texto (generado), un token que no es `.`, las pistas de instrumento, sintetizador o granos, y un texto que no es el cargado.
- **Vista**: `ViewModel::occupancy_cursor`; la celda del cursor, invertida y en negrita; la vista se desplaza (pasos y pistas) siguiendo el cursor. Abrir el cursor del tracker cierra este, y al revés.
- **Editor en su ventana**: el cursor está en la principal (donde se ve la vista); la acción desde el editor va allí (y dice «se abre en la ventana principal», `opens_in_main`), y el cambio llega al editor con el `ToEditor::Text` de siempre como un paso de deshacer. El protocolo no cambia (versión 3).
- **Tests** (`tui/src/tests/occupancy.rs`): ida y vuelta (el cursor en el primer hueco, lo propuesto, el texto es exactamente el original con ese token cambiado, la rejilla lo enseña, releer el texto y cargarlo da los mismos pasos y el mismo audio que escribirlo a mano; otra pista con ↓ y Tab, con su golpe `x` propuesto; cada relleno se deshace de una vez); el cursor invertido y desplazado a 60 y 16 columnas, y que da la vuelta; lo que no se rellena dice por qué y no toca el texto (`euclid`, un golpe, `_`, `chance(1, "mute")`, un instrumento, un corte que no existe, el texto sin cargar); con el editor en su ventana, la acción desde el editor abre el cursor en la principal y el relleno llega al editor como un paso de deshacer.

### Parte 4: sugerencias de relleno

- **El criterio** (`occupancy::suggest`, puro): para cada hueco de la página (un paso donde no se oye nada) y cada pista que se oye y se puede rellenar ahí (lo dice la aplicación con `fill_target`, que da también su golpe propuesto), una puntuación (cuanto menos, mejor) que suma: el choque de frecuencia más fuerte con las pistas que suenan en el paso de antes y el de después (la página se repite, así que el de antes del primero es el último), del análisis de choques si lo hay (la puntuación de 0 a 1 de la pareja en su peor banda, `MaskingReport::score`; sin análisis, no cuenta); `REPEAT_COST` (0,5) si la pista suena ella misma en un paso vecino; y `DENSITY_COST` (0,25) por la fracción de la página en que suena. En cada hueco, la mejor pista; de los huecos, los `MAX_SUGGESTIONS` (4) de menos puntuación (a igualdad, los primeros), en el orden de la página. Cada `Suggestion` lleva la pista, la columna, el compás de la página, el golpe y por qué en una frase («alterna con kick y snare», «repite su sonido al lado», «choque con kick 0.42», «sin choque medido», «entre silencios»).
- **En la aplicación** (`App::suggest_fills`, «Sugerir rellenos de huecos» en la paleta, `Action::SuggestFills`): abre la vista, calcula sobre la página que se ve y lo dice en la línea de estado (cuántas y si con análisis de choques; sin él, cómo hacerlo). El análisis solo cuenta si es de lo que suena (`last_masking` con las mismas cargas que `App::loads`). Las sugerencias se guardan con las cargas (`Occupancy::suggestions`) y otra carga, o cerrar la vista, las borra. Nunca se calculan solas ni escriben nada.
- **En el panel**: una línea por sugerencia debajo de `Σ` (`→ compás 0 paso 14: drums «2» · alterna con kick, sin choque medido`), si queda sitio después de las pistas (el alto que pide el panel las cuenta), y en su página la celda de la pista y el `0` de `Σ` sobre `Theme::suggestion` (`[colores] sugerencia`).
- **Editor en su ventana**: la acción va a la principal (`opens_in_main`); el protocolo no cambia.
- **Tests**: los unitarios de `occupancy.rs` (sin análisis, la pista que no suena al lado y con sitio; con un choque fuerte, la otra; nada donde no se puede rellenar; nunca más de 4) y `tui/src/tests/occupancy.rs` (nada sin pedirlo; 4, solo en huecos, nunca en una pista de `euclid`, con su porqué; el texto no cambia; su línea y su color en el panel; rellenar ese paso propone el mismo golpe y otra carga las borra; con el análisis de choques de verdad, de dos senos de 100 Hz y uno de 5 kHz, junto a la pista grave gana la aguda aunque la otra grave tenga más sitio). `cargo test -p tui pantalla_ocupacion -- --ignored --nocapture` enseña la pantalla.

### Decisiones

- **Lo que sonará, no el texto**: la rejilla lee `TrackDef::bars` y no la cadena de `steps()`. Es lo mismo que recibe el motor (`Pattern::from_bars`), con `every`, `fill`, swing, humanize y los dados de `chance` ya tirados al cargar (con la semilla de la carga): no hay azar que se decida al sonar, así que lo que se ve es exactamente lo que se oirá mientras no se cargue otra cosa. Por eso `euclid`, `rotate`, `mutate` y las cadenas hechas a trozos se ven igual que un literal.
- **El motor no publica nada nuevo**: el cursor de la fase M (etiqueta, índice del ciclo y semicorchea) basta para colocar cada pista en toda la página sin preguntar al motor por cada paso. Una carga o una sección que entra al compás siguiente cambia la etiqueta y la página se recalcula sola.
- **Cómo se ven** (lo pedía el punto 1): la intensidad por el volumen del paso (`@`, y el de humanize) en cuatro sombreados, `░` hasta 0,3, `▒` hasta 0,6, `▓` hasta 0,9 y `█` más (no cuenta el volumen de la pista: es de la mezcla, no del patrón); el retrigger, la cifra de sus partes (`4` para `r4`, `+` desde 10) en vez del sombreado, porque llena el paso de golpes; la nota alargada (`_`), `─`, y cuenta como sonando (en el modo gate y en los instrumentos dura de verdad); un golpe del patrón base que en esta vuelta no suena (`chance(p, "mute")`, o `every` que lo quita), `×`, y no cuenta; los demás efectos de `chance` y `every` (reverse, pitch, retrig, volumen) se ven en el golpe, porque cambian el paso y no si suena. La cola de un corte que sigue sonando después de su paso no se pinta: la rejilla es de golpes, y en un break la cola es justo lo que se tapa al rellenar.
- **Mute y solo**: la pista se ve apagada y no cuenta en `Σ`. Un hueco es un paso donde no se oye nada.
- **Rellenar es elegir la pista, no copiar un corte de otra**: el punto 3 pide «un golpe de otra pista (a elegir)». Un número de corte solo tiene sentido en la pista de su sample, así que el golpe se escribe en la pista que se elige con ↑↓ (la otra respecto a las que ya suenan alrededor), con un corte suyo. Lo propuesto es su golpe anterior tal cual (su corte, volumen y sufijos), que es lo más probable en un break; se cambia en la misma línea.
- **Solo sobre un `.`**: es el único token que se puede sustituir sin cambiar lo que dice otro paso. Encima de un golpe, de un `_` o de un golpe que quita `chance`, rellenar cambiaría algo escrito con intención: eso se hace en el tracker o en el código. Con `every`, se escribe donde está escrita la vuelta que se ve (como en el tracker editable).
- **El criterio de las sugerencias**: lo que hace que un relleno «suene a hueco tapado» en un break es que lo que entra no pise lo de al lado: que sea otro sonido (alternar) y que no esté en las mismas frecuencias (el análisis de choques ya mide justo eso por pareja). Se usa el peor choque de la pareja en cualquier banda, no el de una banda concreta: un golpe es de banda ancha y la cola del vecino se solapa entera. Sin análisis, solo alternancia y sitio: es barato y no adivina frecuencias (analizar el espectro de cada corte al pedir sugerencias sería otro render; el análisis de choques ya existe y se pide aparte). El corte que se propone es el golpe anterior de esa pista, como al rellenar a mano, para que la sugerencia sea exactamente lo que escribiría «Rellenar un hueco».
- **Solo texto, sin «aplicar»**: el punto 4 lo pide; para escribir una está el cursor, que propone el mismo golpe (un paso más, pero se ve qué se escribe y dónde).
- **Coste**: nada en el hilo de audio (el motor no cambia: `no_alloc.rs` y `golden.rs` siguen igual). En el hilo de la interfaz, la rejilla se calcula en cada `App::view` solo con la vista abierta: con 8 pistas y una página de 64 pasos, unos 14 µs más por fotograma en release en el i7 de desarrollo (94 µs contra 79 µs de `view` sin ella) y unos 10 KB de memoria por fotograma (por celda, la celda y su vuelta y paso). Las sugerencias se calculan solo al pedirlas.
- **Encima del sample y no en lugar del tracker**: el tracker y la vista dicen cosas distintas (los cortes y efectos de una vuelta contra qué pistas coinciden) y se miran juntos; la forma de onda es lo que menos falta hace mientras se buscan huecos. Cerrada por defecto, para que la pantalla de siempre no cambie.

## Fase U: panel de dispositivos ✅

Pedida por el usuario: ver y cambiar en caliente la salida de audio y los puertos MIDI de entrada y salida, con el panel en su propia ventana como el editor. Hechas las cuatro partes, una por commit. El uso, en «Dispositivos: audio y MIDI» (sección 2) de [docs/GUIA.md](GUIA.md).

### Antes

`engine::open_output` abría siempre el dispositivo predeterminado de cpal con su configuración por defecto y *movía* el `Engine` dentro del callback: no se podía recuperar para abrir otro. `NullOutput` hacía lo mismo en un hilo. El MIDI (`midi_io`) eran solo los puertos virtuales «sampler entrada» y «sampler salida», con `--allow-midi`, y se conectaba con `aconnect`.

### Parte 1: lógica de dispositivos, sin interfaz

- **La ranura del motor** (`engine/src/device.rs`): `EngineSlot` es un `AtomicPtr<Engine>`. El callback que suena lo toma (`swap` a nulo), llama a `Engine::process` y lo devuelve en cada bloque; si lo encuentra nulo, da silencio. El hilo de control solo lo toca sin ningún stream activo (al abrir el primero, para poner la frecuencia, y al soltar el motor si no se pudo abrir nada).
- **El callback** (`Renderer::render`): lleva su `StreamShared` (todo atómicos: activo, parar, soltado, perdido, desviado, tropiezos, latencia y bloques servidos). No toca la ranura hasta que el control lo activa. Con `stop`, baja la ganancia a cero en `FADE_SECONDS` (5 ms), devuelve el motor y *después* marca `done` (en ese orden). Al activarse, sube desde cero en otros 5 ms. Cada bloque publica la latencia (lo que dice el backend más el bloque) en `Engine::set_output_delay`, así que el retardo del resaltado de la fase M (`Status::heard`) sigue al dispositivo nuevo sin hacer nada más. Sin reservas, mutex ni E/S: `no_alloc.rs` (`the_device_callback_and_its_handoff_never_allocate`) lo cuenta al sonar, con el nuevo en silencio esperando, con el viejo fundiendo y con el nuevo entrando.
- **El cambio** (`AudioOutput::switch`, en el hilo de control y sin bloquear):
  - se abre el nuevo, inactivo, a la frecuencia del motor (si no se abre, error y el viejo ni se entera);
  - se le pide al viejo que pare;
  - `AudioOutput::poll` (en cada `App::tick`) espera a que el viejo marque `done`, que lo dé por perdido o a que pasen `HANDOFF_TIMEOUT` (150 ms);
  - entonces suelta el viejo (al soltarlo, su stream se para) y activa el nuevo.

  Uno cada vez (otro cambio durante el traspaso es un error).
- **Si se va** (`StreamFault::Lost`: el error de cpal `DeviceNotAvailable` o `StreamInvalidated`): `poll` cambia al anterior, si no al predeterminado y, si tampoco, a la salida nula (`NullBackend`, que siempre abre). No espera al fundido del que se fue, que ya no llama a su callback. `DeviceChanged` (el backend lo llevó a otro solo) se avisa y sigue; los demás errores se cuentan como tropiezos.
- **Backends** (`AudioBackend`: `label`, `list`, `open`):
  - `CpalBackend`, con el nombre de `description()` y el identificador de `id()`; las frecuencias son las habituales que caben en los rangos, más sus extremos (`listed_rates`); al abrir a una frecuencia, busca el rango que la admite, mejor con los canales y el formato por defecto;
  - `NullBackend`, el hilo de siempre a ritmo real;
  - `device::fake::FakeAudio`, para los tests: dispositivos que se enchufan y desenchufan, latencia por dispositivo y streams que suenan solo cuando el test los mueve (`pump`, y `pump_into` sin reservas). Su mutex es solo de los tests.

  `open_output` y `NullOutput` siguen como estaban para `--script --allow-audio` y el reproductor de WAV.
- **Lo demás** (`tui/src/devices.rs`):
  - `Devices` junta la salida, el `MidiBackend` (`midi_io::Midir`, o el falso `devices::fake::FakeMidi`) y las listas, leídas a petición (`refresh`; nunca en bucle).
  - Cada lado del MIDI es el puerto virtual o un puerto del sistema, uno a la vez. La entrada vieja se cierra antes de abrir la nueva (midir no deja dos virtuales con el mismo nombre); si la nueva falla, vuelve a abrir la que había.
  - Los puertos se conocen por `port_key` (el nombre sin el ` 24:0` de ALSA) y los propios (cliente `sampler:`) no se listan.
  - Lo que la aplicación tiene que aplicar sale como `Change`: un mensaje, la cola nueva de la entrada o el destino nuevo de las notas.
  - `App::poll_devices` atiende la salida y, si la salida MIDI dejó de aceptar mensajes (`MidiSink::failed`, nuevo, con valor por defecto), vuelve al virtual.
  - Las notas que llegan cuentan para el indicador de actividad (`Devices::note_received`, desde `App::poll_midi`; el reloj no cuenta).
- **Tests**:
  - `engine/tests/devices.rs`: listar y abrir por nombre y por identificador; uno guardado que no está; el cambio, con el audio comparado bit a bit con un render sin cambiar, desplazado por lo que avanzó el fundido, y el mismo compás; frecuencia rechazada; desenchufar en cadena hasta la salida nula y volver; un viejo muerto que se suelta por tiempo; la latencia medida y su efecto en `Status::heard`.
  - `tui/src/tests/devices.rs`: con la App, cambiar sin perder el compás, desenchufar, MIDI de entrada por nombre con la nota que llega, un puerto roto, refrescar tras desenchufar, la salida que falla, el arranque con un puerto que no está, y sin permisos.

### Parte 2: persistencia

- **`[dispositivos]`** (`Config::devices`, un `devices::Choice`): `audio`, `midi_entrada` y `midi_salida`, por nombre (vacío o sin clave: el predeterminado o el virtual). La tabla de la guía sale de `DEVICES_KEYS`.
- **Guardar** (`config::save_devices`, `with_devices`): se reescriben a mano solo esas líneas de la sección: se cambian, se quitan las que pasan a por defecto y se añaden las que faltan tras la última clave. Si no hay sección, se añade al final. Comentarios, el orden y las demás secciones quedan igual. Se escribe en un temporal y se cambia por el de verdad. `App::choose_device` guarda solo la clave de la lista elegida, y solo si cambió.
- **Al arrancar** (`run_editor`): la configuración se lee antes de abrir el audio. `--audio-device`, `--midi-in` y `--midi-out` mandan sobre ella sin guardarse. `AudioOutput::open` prueba el pedido y, si no, el predeterminado con un aviso; `Devices::start_midi` hace lo mismo con el virtual. Los avisos van a la línea de estado junto a los de la configuración (`App::set_startup_warnings`). `--list-devices` lista con `CpalBackend` y midir y sale.
- **Validación**: `--audio-device` va con `--allow-audio` y `--edit`; `--midi-in`/`--midi-out` van con `--edit` y encienden el MIDI.
- **Tests**: `config.rs` (la sección, los avisos, guardar sin tocar lo demás, con escapes, en un archivo que no existía) y `tui/src/tests/devices.rs` (lo elegido se guarda, la vuelta automática no, volver al virtual quita la línea, el arranque con un guardado que no está).

### Parte 3: el panel, aquí y en su ventana

- **El panel** (`tui/src/devices_panel.rs`), el mismo en los dos sitios:
  - `DevicesPanel` dibuja el estado de la salida, las tres listas (una debajo de otra; desde 110 columnas, una al lado de otra), la marca `●` del que está en uso, el predeterminado, los detalles, el cursor `▸` y, en *MIDI de entrada*, el indicador `○ nota`/`● nota` (`Blink`: encendido `BLINK`, 150 ms, desde que cambia la cuenta). Si no cabe, cada lista se desplaza para que se vea el cursor.
  - `devices_panel::key` hace las teclas: ↑↓ Inicio Fin, ←→ y Tab (a la que está en uso de la otra lista), Enter, `r`, Esc/`q`.
- **En la principal** (`tui/src/app/devices.rs`): la paleta tiene «Dispositivos», «Refrescar los dispositivos», «Abrir el panel de dispositivos en su ventana» y «Volver a meter el panel de dispositivos en la principal». El panel se abre encima, como «Macros» (`App::devices_panel`, `ViewModel::devices`, `devices_panel::popup_area`). Desde la ventana del editor, «Dispositivos» va a la principal (`opens_in_main`).
- **En su ventana** (`tui --window-devices`, `tui/src/devices_window.rs`): un `DevicesWindow` con su bucle propio (`run_devices_window`), que solo dibuja el panel y la línea del enlace. Lo que elige lo pide a la principal y enseña la vista que ella le manda: nunca abre la tarjeta ni el MIDI.
- **El protocolo** (`link.rs`, versión 4): un socket aparte del del editor (`devices_socket_path`: el del script con `-disp`), con las mismas reglas (carpeta 0700, socket 0600, `SO_PEERCRED`). `Server::bind_with` y `Client::with_hello` generalizan el servidor y el cliente: el mensaje para el cliente que se queda fuera y el saludo. Del panel a la principal, `FromDevices` (`hola`, `refrescar`, `elegir`); de la principal al panel, `ToDevices` (`hola` con el título, `vista` con las tres listas, `adios`, `devuelve`). La vista se manda solo cuando cambia.
- **Lanzar** (`App::launch_devices_window`, `devices_command`): con `[dispositivos] terminal` o, si no, `[editor] terminal`, detrás `tui --window-devices <script>`. El panel se abre en la principal, con un mensaje claro, cuando no se puede lanzar:
  - no hay terminal configurada (con el comando para abrirlo a mano: el socket queda escuchando);
  - no se puede lanzar;
  - la terminal se cierra sin que el panel llegue a conectarse.

  Al conectarse, el panel de la principal se cierra. «Volver a meterlo» manda `devuelve`, suelta el socket y lo abre en la principal. Si la ventana se cierra, la principal sigue y se puede volver a abrir.
- **Tests** (`tui/src/tests/devices_panel.rs`):
  - el panel en varios tamaños (apilado, en columnas, bajo y desplazado, diminuto y vacío);
  - desde la paleta, elegir audio y MIDI con el indicador encendido, refrescar y cerrar;
  - la ventana por el socket de verdad, en el mismo proceso: listar, elegir, refrescar, en varios tamaños, volver a meterlo, cerrarla y reabrirla, y un panel de otra versión rechazado;
  - el lanzador falso que apunta sus argumentos, la terminal que se cierra sola, la que no existe y la que falta.

  `link.rs` prueba la ida y vuelta de los mensajes. `cargo test -p tui pantalla_dispositivos -- --ignored --nocapture` enseña el panel.

### Parte 4: documentación

Sección «Dispositivos: audio y MIDI» en la guía: el panel, `--list-devices`, el cambio, la latencia, el MIDI, lo que se guarda, la línea de órdenes y la ventana propia. Trae tres apartados: «Si no suena nada» (PipeWire, `pipewire`/`default` frente a `hw:`, `wpctl`, `pw-top`), «Si no llega el MIDI» (el indicador, `aconnect`, `aseqdump`) y «Auriculares y Bluetooth». Además: el MIDI de la sección 2 y el Seaboard citan el panel; el tutorial conecta el teclado desde el panel o con `--midi-in`; dos filas nuevas en «Si algo no suena»; un apartado en la sección 7. Las tablas de la paleta y la configuración, regeneradas.

### Decisiones

- **Al instante con fundido, no al compás**: abrir un dispositivo tarda lo que tarda el sistema (de milisegundos a más de un segundo con Bluetooth), así que no se puede hacer coincidir con un compás. Uno que desaparece no puede esperar. Y lo que importa musicalmente (dónde va la canción) no se pierde, porque el motor es el mismo objeto: la canción se queda quieta los milisegundos del traspaso (el que entra espera a que el que sale suelte el motor) y sigue en el mismo paso, sin saltar ni repetir. El reloj MIDI de salida y la posición en el tiempo real se retrasan ese hueco. Los 5 ms de cada fundido bastan para no oír el clic y no se notan como hueco.
- **Primero se abre el nuevo**: si falla, el viejo sigue sin haberse tocado. Por eso «sigue con el anterior» es gratis en el cambio pedido. Si el que se va es el que suena, se prueba el anterior, el predeterminado y, por último, la salida nula, para que el programa no se cuelgue ni se quede sin reloj.
- **La frecuencia del motor manda**: los samples se resamplean a ella al cargarlos (regla 6). Cambiarla obligaría a recargar y resamplear todo en caliente, con picos de memoria y de tiempo. Un dispositivo que no la admite se rechaza con un mensaje; con PipeWire (lo normal en Gentoo y Asahi) cualquier frecuencia vale.
- **La latencia es la medida, no una declarada aparte**: cpal no da la latencia de un dispositivo sin abrirlo, y el tamaño de bloque declarado (`SupportedBufferSize`) es un rango, no lo que se usa. Lo que mide el callback (las marcas de tiempo de cpal más el bloque) es lo que se oye de verdad, y ya movía el resaltado. Se enseña en el panel y en el aviso del cambio.
- **MIDI: un puerto por lado, o el virtual**: es lo más claro de enseñar y de guardar. Conectar a la vez el virtual y uno del sistema duplicaría las notas de un teclado conectado también por `aconnect`. El virtual sigue siendo el de siempre para quien ya usa `aconnect`.
- **Por nombre sin los números de ALSA**: midir nombra los puertos `Cliente:Puerto C:P`, y `C:P` cambia al reiniciar o al enchufar en otro orden. Se busca primero el nombre exacto y luego sin números. Para el audio se guarda el nombre que se ve y se acepta también el identificador de cpal (`alsa:hw:CARD=...`).
- **Solo se guarda lo que elige el usuario**: si unos auriculares se desenchufan y suena por los altavoces, lo guardado sigue siendo los auriculares, para que vuelvan a ser los buenos al arrancar con ellos. La línea de órdenes es para una vez y tampoco se guarda.
- **`--midi-in`/`--midi-out` encienden el MIDI** (pedir un puerto es pedir el MIDI); `--audio-device` exige `--allow-audio`, la regla de siempre de no abrir la tarjeta sin pedirlo. Sin `--allow-audio`, el panel no lista ni abre la tarjeta (listar con ALSA ya abre cada PCM para preguntarle qué admite).
- **Un socket aparte para el panel**: el editor y el panel pueden estar fuera a la vez, y el servidor de un editor solo atiende a uno. El protocolo sube a la versión 4 porque cambia (mensajes nuevos), aunque los del editor no cambian.
- **Desaparición del MIDI**: midir no avisa cuando un puerto se va (en ALSA, la conexión se queda muerta sin error). La entrada se comprueba al refrescar y la salida cuando falla un envío, sin sondear en bucle.

### Sin verificar con dispositivos reales

Todo lo de arriba está probado con los backends falsos y nunca con la tarjeta ni con puertos de verdad (las reglas del proyecto lo prohíben desde Claude Code). Falta comprobar a mano:

- **cpal con PipeWire** (Gentoo y Asahi): qué nombres salen en la lista (`pipewire`, `default`, los `hw:`), si al desenchufar unos auriculares USB llega `DeviceNotAvailable` o el stream se queda callado sin error (en ese caso el cambio automático no saltaría y habría que elegir otro), y si `DeviceChanged` llega al cambiar el sink por defecto en PipeWire.
- **Bluetooth**: la latencia que mide el callback con un auricular de verdad y si el resaltado coincide con lo que se oye.
- **El traspaso en una tarjeta de verdad**: que el fundido de 5 ms no se oiga y que cerrar el stream viejo no tarde (soltar un stream de ALSA espera a su hilo).
- **midir**: conectar a un teclado o al Seaboard por nombre, el indicador de actividad, desenchufarlo y refrescar, y conectar la salida a Surge XT.
- **La ventana de dispositivos** lanzada con una terminal de verdad (kitty, foot) y la regla de Hyprland.

## Fase V: afinación, mandos y congelar ✅

Pedida por el usuario: seis funciones, un commit por función (o por
parte). Hechas las seis. El plan, en «Fase V» de [ROADMAP.md](ROADMAP.md).

### Parte 1: cortes en el cruce por cero

- **Antes**: no había nada. Cada voz pone su fundido de entrada de 1,5 ms,
  que suaviza el salto de un inicio lejos del cero pero no lo quita (con
  graves y en rejilla se oye un clic corto).
- **Motor** (`engine/src/slice.rs`): `nearest_zero_crossing(sample, frame,
  máx, límites)` busca hacia los dos lados, de uno en uno, el frame más
  cercano en el que la mezcla mono `(l + r) / 2` vale 0 o cambia de signo
  con un vecino, quedándose con el de los dos lados del cambio que más se
  acerca a cero (a igual distancia, el de antes); `snap_frame` lo deja donde
  estaba si no hay ninguno. `Slices::snapped(sample, máx, separación)` lo
  hace con cada inicio, sin acercarse a menos de la separación mínima de su
  vecino anterior (ya movido) ni del final de su corte, y el anterior acaba
  donde empieza el siguiente.
- **Lua**: `sample:slice(n, { zero_cross = ms })` y `slice_grid(n, { ... })`
  (`true` = 2 ms, `DEFAULT_ZERO_CROSS_MS`; 0 o `false`, apagada; hasta 50).
  `cut_options` lee la tabla con las claves del registro (`api::CUT_PARAMS`,
  `CUT_TABLE` para el autocompletado y la ayuda; `Auto::No(AT_LOAD)`).
  `cut_sample` aplica la opción al final, después de las ediciones
  (`partir`, `unir`) y los ajustes del proyecto, así que todo inicio queda
  en un cruce. `SliceOrigin::zero_cross` la guarda y la heredan `slice_at`,
  `add` (el cruce más cercano en todo el sample, antes de partir) y `move`
  (el más cercano entre sus vecinos). La clave del corte (`Cut`) no cambia:
  etiquetas y ajustes del proyecto son los mismos con la opción o sin ella.
- **Interfaz**: `App::adjust_slice` mueve el ms guardado (no el inicio real)
  cuando la pista tiene la opción, y el mensaje dice dónde quedó el cruce
  («empieza en 260 ms (cruce por cero en 259,4 ms)»).
- **Tests**: `engine/tests/zero_cross.rs` (el cruce más cercano por los dos
  lados, máximo y límites, corriente continua sin cruces, una rejilla de 8
  sobre un seno de 113 Hz con cada inicio a menos de un paso del seno del
  cero y contigua, y que nunca pasa al otro lado de un vecino) y
  `script/tests/zero_cross.rs` (sin la opción, `{}`, `false` o 0, los mismos
  cortes y el mismo audio; con ella, cada inicio a menos de 3 ms y en el
  cruce; el golpe renderizado sale de cero —lo más alto de sus primeros 18
  frames, menos de 0,05 frente a más de 0,1 sin ella—; `add` y `move`; un
  ajuste del proyecto es el objetivo y su ms no cambia; errores con la
  línea). `registry.rs` prueba la tabla nueva.

#### Decisiones

- **Una opción del corte, no del sample**: es lo que se decide al cortar,
  la pide quien llama a `slice`/`slice_grid` y la heredan los cortes que
  salen de ahí (`add`, `move`, `slice_at`). En `sample()` afectaría
  también a los instrumentos y los granos, que no la necesitan.
- **El ajuste manual es el objetivo**: el ms que se guarda (con `move`, las
  teclas, arrastrar o el proyecto) es lo que elige el usuario, y el inicio
  real es el cruce más cercano a él. Así quitar la opción devuelve
  exactamente el ajuste, y las teclas no se quedan atascadas en un cruce
  (mueven el ms guardado). Lo malo: con la opción, el ms escrito no es
  exactamente donde empieza; el mensaje y la vista enseñan el real.
- **Mono para buscar el cruce**: los dos canales rara vez cruzan a la vez;
  el cruce de la mezcla deja los dos cerca de cero en material casi mono
  (lo normal en un break) y es lo que pesa en el salto que se oye.
- **Sin cruce, se queda**: una señal con corriente continua o un grave más
  lento que el máximo no se mueven (mejor que irse lejos del golpe).
- **Coste**: solo al cargar (hilo de control), como mucho `2·máx` lecturas
  por corte: 4800 con 50 ms a 48 kHz, nada al lado del detector.

### Parte 2: MIDI learn

- **Antes**: las macros (`Curve::Macro`, `Controller::set_macro`, `[midi]
  macros` para mover una macro con un CC) y la entrada MIDI de la fase U.
  No había forma de asignar un mando a un parámetro sin tocar el script.
- **Motor** (`engine/src/knob.rs`): `Knobs` = hasta `MAX_KNOBS` = 16
  `KnobMap { target, min, max, curve, latch }`; `KnobTarget::{Fx { pista,
  ranura, parámetro }, Mix { pista, índice }, Bus { parámetro }, Synth {
  pista, parámetro }}`; `KnobCurve::{Linear, Log, Square}`. La tabla va en
  el lote (`Batch::set_knobs`, validada con `check_knobs`) y entra al
  compás como las macros (`staged`/`pending`); `Controller::move_knob`
  manda el valor ya. `KnobState::advance` (al empezar cada trozo de
  `mix`) sigue el valor con un polo de 30 ms, sube el peso de 0 a 1 en
  30 ms desde que se mueve y, sobre una curva (`latch` falso), lo baja en
  0,5 s cuando lleva 1,5 s quieto; da `KnobLive` (destino, valor y peso de
  los que mandan), que va en el `Clock` de las pistas, el `BusClock` y el
  `SongClock` del sintetizador. En cada sitio, `KnobLive::apply` mezcla el
  valor de la curva (o el fijo) con el del mando según el peso (en escala
  logarítmica si el parámetro lo es) y el suavizado de siempre lo sigue:
  `drive_block` (efectos de pista; no en los de corte ni de grupo),
  `mix_target` (volumen, pan, envíos; y `sending` lo tiene en cuenta),
  `Rack::drive` (buses) y `Synth::mix_into`. Un mando que manda hace que
  la pista o los buses tomen el camino de las curvas (`moving`, `curved`):
  sin mandos, el camino fijo de siempre, bit a bit.
- **Script** (`script/src/learn.rs`): `Learned` (canal o cualquiera, CC,
  pista o `bus`, parámetro por nombre —`lowpass.cutoff`, `lowpass#2.q`,
  `gain`, `pan`, `send.reverb`, `synth.cutoff`, `reverb.size`—, mínimo,
  máximo y curva) en texto `1 21 drums lowpass.cutoff 200 8000 log`
  (`parse`, `to_line`). `learnable(song)`: lo que el registro dice
  `Auto::Smooth` de los efectos de cada pista (no congelada), su volumen,
  pan y envíos, los de su sintetizador y los de los buses, con su rango.
  `resolve` los traduce a la tabla del motor con los nombres de lo cargado
  (`latch` si el parámetro no tiene curva); lo que no encuentra, aviso.
  `ScriptHost::{set_learned, apply_learned, move_learned}`; cada carga
  manda la tabla en su mismo lote. Proyecto: `mando = ...`
  (`Project::learned`). `api::effect_name` (el nombre del registro de un
  efecto del motor; lo usa también `model::short_name`).
- **Interfaz** (`tui/src/app/learn.rs`): «MIDI learn» (lista filtrable,
  `BlockPickerPopup`), luego espera un CC (`App::learning`; Esc cancela);
  el CC llega por `midi_knob` antes que las macros y la expresión. Un CC
  solo mueve una cosa. «Mandos MIDI» (`KnobsPopup`): Supr, `c`, `l`, Enter
  (orden `mando N mín máx curva`). «Exportar los mandos MIDI a la
  configuración» (`config::save_learned`/`with_learned`: sustituye la
  lista `mandos` de `[midi]`, aunque ocupe varias líneas, o la añade, sin
  tocar lo demás). `[midi] mandos` se lee en `Config::learned` y es la
  lista de un proyecto que no tiene la suya. Cada cambio va al motor
  (`apply_learned`) y se guarda en el proyecto abierto.
- **Panel de dispositivos**: `DevicesView::knobs`, la línea «2 mandos ·
  último CC 21 (canal 1) = 64 · aprendiendo …» junto a `○ nota`; viaja por
  el socket del panel, así que el protocolo sube a la **versión 5**.
- **Tests**: `engine/src/knob.rs` (curvas, entrar y devolver, quedarse,
  suavizado), `engine/tests/knobs.rs` (el volumen se mueve sin escalones y
  se queda; sobre una curva manda y la devuelve sin escalones; la tabla
  entra al compás y el valor ya; un filtro, el nivel de la reverb —igual
  que sin envío— y el corte de un acid; una tabla que nadie mueve, bit a
  bit; destinos que no existen), `no_alloc.rs` (cinco mandos, uno por
  destino, moviéndose y soltándose, sin reservas),
  `script/src/learn.rs` y `script/tests/learn.rs` (qué se ofrece, la
  traducción con las pistas en otras ranuras y lo que no existe, que el
  mando mueve el sonido tras cargar y sin moverlo es bit a bit, el
  proyecto), `tui/src/config.rs` (la lista en `config.toml`) y
  `tui/src/tests/learn.rs` (asignar desde la paleta en dos tamaños y que
  suene, un CC una cosa, cancelar, la lista en dos tamaños, curva, rango,
  otro mando y borrar, proyecto, configuración por defecto y exportar, el
  panel de dispositivos). `link.rs` prueba la vista con la línea nueva.

#### Decisiones (MIDI learn)

- **La tabla al compás, el valor ya**: la tabla dice qué pista y qué
  ranura mueve cada mando, y eso es de la canción que suena; tiene que
  entrar con ella (si no, durante un compás un mando podría mover el
  efecto equivocado de una recarga que espera). El valor no: un mando que
  esperase al compás no serviría para tocar. Asignar un mando nuevo hace
  que mande desde el compás siguiente.
- **Suavizado doble**: 30 ms sobre el CC (sus 128 escalones en un corte de
  20 Hz a 20 kHz son saltos de un 5,6 %) y después el de 5 ms de siempre.
- **Cuándo devuelve el control**: un CC no dice si se suelta el mando
  (los táctiles mandan otro mensaje, pero no todos), así que el criterio es
  el tiempo: 1,5 s sin moverse. Solo sobre una curva: sobre un valor fijo
  no hay nada a lo que volver, y que el parámetro volviera solo al del
  script sorprendería (el mando se quedaría en un sitio y el sonido en
  otro). Sin «pickup» (esperar a que el mando cruce el valor): la primera
  vez el parámetro va a donde está el mando en 30 ms (sin clic, pero se
  oye el cambio).
- **Por nombre, no por índice**: en el proyecto y la configuración van
  `drums lowpass.cutoff`; los índices del motor se calculan en cada carga.
  Así sobreviven a reordenar pistas o efectos, y lo que ya no existe avisa
  en vez de mover otra cosa.
- **La configuración como plantilla**: un mismo controlador en varios
  proyectos; el del proyecto manda entero (no se mezclan, para que la lista
  que se ve sea la que suena).
- **Qué no**: los parámetros por golpe o por grano (se leen al nacer: un
  mando no se oiría mientras se mueve), los efectos de corte y de grupo
  (otra cadena; se añadirían igual) y lo discreto.
- **Coste**: sin mandos, nada (el estado avanza solo si hay alguno). Con
  ellos, por trozo de audio un `advance` de 16 entradas y, en cada sitio,
  una búsqueda lineal en 16 entradas cada 32 frames; una pista con un mando
  que manda toma el camino de las curvas (como con un `ramp`). Medido en
  release (8 pistas con filtro y compresor): 2,44 % de un núcleo sin
  mandos y 2,75 % con 16 mandos (filtro y volumen de cada pista)
  moviéndose. La tabla
  son 16 × 20 bytes y va dentro de `Command` sin agrandarlo (ya ocupaba
  2880 bytes).

### Parte 3a: tonalidad de un sample, el análisis

- **Antes**: `cortes:note(i)` (YIN) da la nota de un corte con un tono
  claro; de tonalidad no había nada.
- **Motor** (`engine/src/key.rs`, puro, sin motor ni Lua): `detect_key(sample)`
  mezcla a mono los primeros 60 s, saca FFT de 16384 puntos (2,9 Hz por bin
  a 48 kHz, por debajo del semitono más estrecho que se mira, 3,9 Hz en C2)
  con Hann y salto de media ventana, y de cada ventana suma la amplitud de
  los **picos** del espectro (máximos locales, afinados con una parábola en
  dB) a la nota MIDI más cercana, de C2 a C8. Las ventanas 60 dB por debajo
  de la más fuerte no cuentan (silencio, colas) y en cada una, tampoco los
  picos 60 dB por debajo del suyo. El cromagrama (12 clases, suma 1) se
  correlaciona (Pearson) con los perfiles de Krumhansl y Kessler girados a
  las 24 tonalidades. `KeyAnalysis { candidates: [KeyGuess; 3], confidence,
  root, chroma }`: las tres mejores, la confianza (la correlación de la
  mejor, rebajada hasta la mitad si la siguiente que no es su relativa se
  le acerca a menos de 0,1), la raíz propuesta (la tónica en la octava donde
  más suena) y el cromagrama. `low_confidence()` por debajo de
  `LOW_CONFIDENCE` = 0,5. `KeyGuess::{tonic_name, mode, label}` («A menor»).
- **Tests** (`engine/tests/key.rs`): cadencias I-IV-V-I y i-iv-V-i con
  armónicos y bajo en las 24 tonalidades (la buena, sin aviso y la raíz en
  su clase), una escala de do mayor de senos (C mayor, raíz C4), una nota
  sola (A3: la raíz exacta, y la entre las tres), ruido blanco y golpes de
  ruido (poca confianza), silencio y un sample de 100 frames (nada).
  Medido: cadencia de A menor 0,91, de C mayor 0,95; A3 sola 0,70 (A mayor
  y A menor casi empatan: con una nota no se sabe el modo).

#### Decisiones (análisis)

- **Picos y no el espectro entero**: la ventana de Hann reparte un tono
  entre bins vecinos que, en graves, caen en la nota de al lado; contar solo
  los máximos locales deja un pico por parcial. Los armónicos cuentan (el 3.º
  suma a la quinta), como en el oído; los perfiles ya lo absorben.
- **Krumhansl y Kessler**: simples, conocidos y sin entrenar nada; en
  material tonal funcionan bien. Con poco material (una nota, un acorde)
  el modo es ambiguo y se ve en la confianza y en las tres candidatas.
- **La confianza no castiga a la relativa**: C mayor y A menor comparten
  las notas; que estén cerca no dice que el material sea dudoso.
- **Coste**: un sample de 10 s son unas 60 FFT de 16384 puntos (unos ms en
  release); hasta 60 s, unas 350. Siempre en el hilo de control, al
  pedirlo.

### Parte 3b: tonalidad de un sample, el panel

- **Panel** (`tui/src/key_panel.rs`, sin estado de la aplicación):
  `KeyView` (sample, `KeyState::{Analyzing, Nothing, Done}` con las tres
  candidatas y su correlación, la confianza, el aviso y la raíz, la
  elegida y la tonalidad de la canción), `KeyPopup` (barras como las de las
  macros, el aviso en el color de los avisos, las tres acciones y las
  teclas) y `popup_area`, del alto de lo que enseña.
- **En la App** (`tui/src/app/key.rs`): «Tonalidad del sample» (paleta,
  `Action::SampleKey`; desde la ventana del editor, en la principal) toma el
  sample de la pista seleccionada (un sintetizador no tiene); Ctrl-K en el
  explorador, el elegido (lo lee como la preescucha). `analyze_key` lanza
  `engine::detect_key` en un hilo y `poll_key_analysis` (en `tick`) lo
  recoge por un canal: la interfaz no se para. ↑↓ eligen la candidata;
  `r` cambia o añade `root` en el `instrument()` de ese sample
  (`key_panel::with_instrument_root`: directo o por una variable `local`;
  la raíz es la tónica elegida en la octava donde más suena la mejor);
  `i` inserta `sample("x"):instrument({ root = "A3" })` en el cursor; `k`
  escribe `key("A", "minor")` (`with_key_line`: sustituye la línea `key()`
  o va detrás de `bpm()`). Cada una, un paso de deshacer (`replace_text`,
  ahora `pub(super)`), y ninguna carga: se aplica con Ctrl-S.
- **Tests**: `key_panel.rs` (la raíz cambiada, añadida, sin tabla, con
  tabla vacía, en varias líneas, por una variable, otro sample, `sroot`; la
  línea `key()`) y `tui/src/tests/sample_key.rs` (desde la paleta, en
  120×40, 80×24 y 60×18, A menor con la raíz A3 y las acciones; un break,
  confianza baja; `r` no carga hasta Ctrl-S, carga la raíz nueva y Ctrl-Z
  lo deshace de una vez; `i`; `k` propone sin aplicar, Ctrl-S la aplica y
  otra vez sustituye la línea; Ctrl-K en el explorador).

#### Decisiones (panel)

- **Un hilo y un canal**, no `rayon` ni un trabajador fijo: es una
  petición suelta del usuario; si cierra el panel antes de acabar, el
  resultado se tira.
- **Proponer, nunca aplicar**: `k` escribe `key()` en el editor y nada
  más; la tonalidad cambia cuando el usuario carga, como cualquier otro
  cambio del script (y entra al compás).
- **Letras y no un menú**: tres acciones fijas caben en una línea de
  ayuda y no hace falta elegir y confirmar.

### Parte 4: tonalidad global (`key`, `transpose`)

- **Lua** (`script/src/lib.rs`): `key(tónica [, modo])` (el modo, cualquier
  escala de `scale()`; por defecto `"major"`) y `transpose(n)` (enteros de
  -24 a 24), una vez cada uno y fuera de `section()` y `every` (en
  `GLOBAL_ONLY`). `Song::key: Option<SongKey { tonic, scale }>` y
  `Song::transpose`. `SongKey::{tonic_name, call, degrees, transposed}`.
- **`scale([nombre [, tónica]])`**: lo que falta lo pone `key()`; sin
  nombre y sin `key()`, error. `key()` tiene que ir antes que cualquier
  `scale()` sin tónica o sin nombre y que un grado de `melody()`
  (`Builder::key_used` guarda dónde se usó la tonalidad por defecto y
  `key()` después de eso es un error con las dos líneas).
- **Grados en `melody()`** (`patterns::Key`, `note_step(token, raíz,
  tonalidad)`): con `key()`, un paso que empieza por número (`1`, `-2`,
  `5@0.5`, `1+3+5`) es un grado: el 1 es la tónica más cercana a la raíz (de
  6 por debajo a 5 por encima: con la raíz C4 y A menor, A3), los demás como
  en `scale`. Se mezclan con notas. Sin `key()`, el error dice qué falta.
  Vale en `pista:melody` y en `c:melody` de `every` (`Timing::key`). El
  diagnóstico en vivo los lee en do mayor (no sabe la tonalidad; si falta
  `key()`, lo dice la carga).
- **`transpose(n)`** se aplica al acabar el script (`expand`, en cualquier
  sitio del script): suma `n` al `semitones` de cada paso (los acordes van
  sobre él) de las pistas con instrumento o sintetizador, y de las de cortes
  que han usado `melody()` (`TrackBuild::melodic`, también desde `every`).
  Una nota que se sale de ±36 da error con la pista. Al ir en los pasos, el
  tracker, la salida MIDI, la grabación y el resaltado lo ven sin más.
- **Pads** (`App::pad_semitones`): con `key()`, los pads de un instrumento
  o un sintetizador tocan los grados 1 a 18 de la tonalidad (movida por
  `transpose()`) desde su tónica más cercana a la raíz; sin ella, semitonos
  como siempre. El MIDI de entrada no cambia (cada tecla es su nota).
- **Registro y editor**: `key` y `transpose` en `FUNCTIONS`; `scale` y
  `melody` con su doc nueva; el autocompletado ofrece las tónicas y los
  modos dentro de `key("...")`.
- **Tests**: `script/tests/key.rs` (la tonalidad en el modelo; `scale()`
  con lo que falta de `key()` da lo mismo que escrito a mano y
  `transpose()` no la mueve; grados de `melody()`, también en `every`,
  suenan bit a bit como sus notas; `transpose(2)` suena bit a bit como
  escribirlo dos semitonos arriba en instrumentos —melody, steps con `^`,
  acordes— y en `melody()` de cortes, sin mover los `^` de la batería, y
  `transpose(0)` como nada; recargar con `transpose(12)` entra al compás
  siguiente y lo de antes queda igual; 11 errores con la línea),
  `patterns.rs` (grados) y `tui/src/tests/instrument.rs` (los pads con
  `key()` y con `transpose()`). `golden.rs` sin tocar: sin `key()` ni
  `transpose()` no cambia nada.

#### Decisiones (tonalidad global)

- **`key("A", "minor")`**: se lee como se dice («la menor») y el modo es
  el nombre de `scale()`, así que vale cualquier escala (`"dorian"`,
  `"minor_pentatonic"`). Tónica y modo con los mismos nombres y errores que
  `scale()`.
- **Qué mueve `transpose()`**: todo lo que es una nota (instrumentos y
  sintetizadores, sin importar cómo se escribieron, y `melody()` en
  cortes) y no los `^` de `steps()` en una pista de cortes, que son
  semitonos exactos sobre un corte (la caja `^-3` de un break no tiene que
  subir con la canción). `scale()` tampoco: sus grados acaban en `^` de
  `steps()` y, si los moviera, `escala:melody` en un instrumento subiría
  dos veces. Para una línea de cortes que se transponga, `melody()` con
  grados.
- **`transpose` al final y `key` antes**: `transpose()` solo suma a los
  pasos ya hechos, así que puede ir en cualquier sitio. `key()` cambia lo
  que devuelven `scale()` y los grados en el momento de llamarlos (Lua no se
  puede reevaluar hacia atrás): antes de ellos, o error.
- **Por lote al compás**: son parte del script, así que entran como todo lo
  demás, enteros al empezar el compás (test). No hay un `transpose` en vivo
  aparte: sería un segundo sitio donde vive la altura de las notas.
- **Grado 1 = la tónica más cercana a la raíz**: el instrumento suena tal
  cual en su raíz y lo natural es que el grado 1 quede cerca de ella (más
  calidad y menos cambio de duración), arriba o abajo.

### Parte 5: sidechain visual

- **Antes**: el compresor (`FxState::Comp::reduction`) y el EQ dinámico
  (`DynState::applied`) ya llevaban dentro su reducción en dB, pero no la
  veía nadie. El sidechain es del EQ dinámico (`Fx::sidechain`; el
  compresor se escucha a sí mismo).
- **Motor**: `FxState::reduction()` da la de los dos; tras la cadena de
  cada pista (con o sin curvas), `Engine::mix` la publica por ranura con
  `SharedStatus::add_reduction` (un `AtomicU32` por pista y ranura,
  `fetch_max` sobre los bits del f32 positivo, como los picos), y
  `Status::tracks[t].reduction` la da y la vacía al leerla: la más alta
  desde la lectura anterior. Sin reservas, mutex ni E/S (`no_alloc.rs`: el
  caso del EQ dinámico con sidechain cuenta también la publicación y
  comprueba que llega).
- **Interfaz** (`tui/src/sidechain.rs`): `Sidechain::observe` (en cada
  `tick`, con la instantánea que ya se leía para los medidores) apunta por
  cada EQ dinámico y compresor de la canción la reducción de su ranura y
  el pico de la pista que lo dispara (la de su `sidechain`, o la propia),
  en colas de `HISTORY` = 100 muestras; los que desaparecen del script se
  olvidan. `view` da `SidechainView` (pista, efecto, quién dispara, la
  reducción de ahora, la escala —el `range` del EQ dinámico, 24 dB en el
  compresor— y las dos curvas) y `SidechainPanel` lo dibuja con barras de
  octavos de celda (`sparkline`), en tres líneas por efecto o, sin sitio,
  en una. «Vista del sidechain» (paleta, `Action::SidechainView`) la abre y
  la cierra: con el editor aquí, encima del sample y el tracker (no
  captura teclas); con el editor en su ventana, a la derecha de la forma
  de onda grande (hasta la mitad, 64 columnas). El protocolo no cambia: la
  vista es de la principal.
- **Tests**: `engine/tests/dyneq.rs` (la reducción por ranura: el
  compresor con un tono 14 dB sobre el umbral, unos 10 dB; el EQ dinámico
  con el bombo, entre 6 y su `range`, y 0 cuando el bombo calla; 0 en las
  ranuras sin dinámica y en otras pistas), `sidechain.rs` (las curvas) y
  `tui/src/tests/sidechain.rs` (desde la paleta, quién dispara a quién, las
  curvas siguen al motor, en 140×45, 100×30 y 80×24, el editor sigue a la
  vista, cerrarla; sin dinámica lo dice; en la principal con el editor
  fuera, junto a la forma de onda). `cargo test -p tui pantalla_sidechain
  -- --ignored --nocapture` la enseña.

#### Decisiones (sidechain visual)

- **La reducción, no la envolvente del detector**: lo que se oye es cuánto
  baja; el nivel del detector depende del filtro de su banda y del umbral,
  y como curva del disparo basta el pico de la pista que dispara (el que
  ya se publicaba). Así el motor solo añade un `fetch_max` por efecto de
  dinámica y trozo.
- **La más alta desde la lectura anterior** (como los medidores): entre
  dos `tick` pasan varios trozos de audio y una bajada corta no se pierde.
- **El historial en la interfaz**: el motor no guarda nada más; las curvas
  empiezan al abrir la vista y viven solo mientras está abierta.
- **También el compresor**: es la otra dinámica de pista y cuesta lo
  mismo; sin sidechain, dice «← sí misma».
- **Grupos fuera**: sus efectos se procesan en otro sitio (`Engine::mix`,
  después de las pistas); se añadirían igual, con otro array de atómicos.
- **Coste**: en el motor, 32 atómicos (8 pistas × 4 ranuras) y un
  `fetch_max` por efecto de dinámica y trozo de hasta 256 frames; en la
  interfaz, 2 × 100 `f32` por efecto.

### Parte 6: congelar pista

- **Motor**: `Engine::set_tap(Some(pista))` (para render offline) hace que
  la salida sea la de esa pista tras sus efectos y antes de su volumen,
  pan, envíos y mute (la que se guarda para los sidechains, `side_buf`; la
  pista se marca como escuchada), sin limitador; las demás suenan dentro.
  `Pattern::with_raw()` hace que sus golpes suenen sin las rampas de 1,5 ms
  de entrada y salida (`Voice::raw`; la de interrumpir, sí). `no_alloc.rs`
  no cambia: la toma es una copia entre buffers fijos y solo se usa
  offline. `offline::write_wav_float`: WAV de 32 bits en coma flotante,
  que vuelve bit a bit al leerlo a la misma frecuencia.
- **Script** (`script/src/freeze.rs`):
  - `Song::render_tap(frecuencia, compases, pista)` renderiza esa toma.
  - `Song::render_frozen(frecuencia, pista)` comprueba que se puede (sin
    secciones ni `bpm_ramp`, que exista, que no esté ya congelada y que no
    tenga `choke`), calcula la vuelta (el ciclo de la canción, `length_bars`,
    y el de las curvas que suenan dentro de la pista —efectos, pitch,
    formante, cortes, forma y sintetizador—: las rampas y los LFO en pasos
    se repiten; un LFO en Hz o una macro, aviso), renderiza dos vueltas y
    se queda con la segunda (con las colas de la primera), y da la huella.
  - La huella (`fingerprint`): FNV-1a de lo que suena dentro de la pista (el
    WAV y sus cortes, pasos de cada vuelta, efectos y sus curvas,
    sidechains, formas, efectos de corte, instrumento, granos, pitch y
    formante, gate y stretch) con el tempo y el compás.
  - `frozen(pista, wav [, huella])` (en `GLOBAL_ONLY`) carga el WAV con la
    caché de `sample()` (relativo a la carpeta de samples) y, al acabar el
    script (`apply_frozen`), cambia la pista por una que toca el WAV en el
    paso 0 de la primera de sus N vueltas (N = lo que dura el WAV a este
    tempo), sin efectos, automatización, sidechains, instrumento, granos,
    forma, choke ni MIDI, con `TrackDef::frozen` y el patrón `with_raw`; el
    volumen, el pan, los envíos, el mute, el solo, las curvas de mezcla, el
    grupo y el nombre son los de la original (`..original.clone()`). Si la
    huella no coincide o el WAV no dura compases enteros, aviso y
    `FrozenDef::stale`. Errores con la línea: pista que no existe, WAV que
    no está, dos veces, en `section()`, con secciones, con choke.
- **Interfaz** (`tui/src/app/freeze.rs`): «Congelar la pista seleccionada»
  (con el texto cargado) renderiza, escribe `congelado/<pista>.wav` en la
  carpeta de samples y añade al final del texto `frozen("pista",
  "congelado/pista.wav", "huella") -- congelada: F1 «Descongelar»` (un paso
  de deshacer, `replace_text`), y carga. «Descongelar la pista
  seleccionada» quita las líneas que empiezan por `frozen("pista"` y carga:
  el texto vuelve a ser exactamente el de antes. Desde la ventana del
  editor, las dos van a la principal. El panel del motor marca `drums❄`
  y «❄ congelada» (o «(ha cambiado)»).
- **Tests**: `script/tests/freeze.rs`:
  - la salida de la pista congelada es bit a bit la segunda vuelta de la
    original, en cada vuelta;
  - una pista que se repite (cortes, filtro, bitcrush, `every`, volumen y
    pan, con otra pista al lado): la mezcla, bit a bit desde la segunda
    vuelta;
  - con todo (rampa, bitcrush, EQ dinámico con sidechain hacia ella y desde
    ella, envío a la reverb) y un `acid` con LFO en compases: la mezcla se
    separa de la original como mucho 1,5 veces lo que la original se separa
    de sí misma entre su segunda y su tercera vuelta (medido: 0,0113 frente
    a 0,0113 la batería; 1,75 frente a 1,75 el acid, por la fase libre de
    su oscilador);
  - el aviso de huella y la marca, lo que no se congela, los errores con la
    línea, y el coste (ignorado).

  `tui/src/tests/freeze.rs`: congelar escribe el WAV (float, estéreo) y
  solo añade esa línea, la marca en 120×40, 90×28 y 70×24, descongelar
  deja el texto idéntico y el WAV, Ctrl-Z congela y descongela de una vez,
  con el texto sin cargar no hace nada, y con secciones lo dice.
  `registry.rs` prueba la plantilla de `frozen`.

#### Decisiones (congelar)

- **Una línea en el script, no el proyecto**: el script es la fuente de la
  verdad de lo que suena (editor y paneles coinciden), así que lo que
  congela tiene que verse en él. Una sola línea al final deja intacto todo
  lo que escribió el usuario (descongelar es borrarla, exactamente), es un
  paso de deshacer y se lleva con el `.lua` sin depender del `.sampler`.
- **Antes del volumen**: el WAV es lo que se calcula caro (voces,
  instrumento, efectos); el volumen, el pan, los envíos y el mute siguen
  vivos (se pueden mezclar con la pista congelada) y quien la escucha por
  sidechain oye lo mismo que antes. Por eso no hay opción de meter los
  envíos en el WAV: los buses suman todas las pistas y su coste no baja
  por congelar una (y lo congelado se podría oír dos veces).
- **La segunda vuelta**: con las colas de la anterior, que es como suena
  la pista en bucle. La primera vez que suena tras arrancar lleva esas
  colas aunque no haya vuelta anterior.
- **Sin fundidos y fijado al compás**: el WAV dura exactamente una vuelta
  (los compases duran siempre lo mismo sin rampa de tempo) y suena en el
  paso 0 de cada una sin las rampas de la voz, así que muestra a muestra es
  la toma. Con rampa de tempo o secciones cada vuelta es distinta: no se
  puede.
- **La huella avisa, no impide**: lo congelado sigue sonando aunque la
  pista cambie (es lo que el usuario oyó al congelar); el aviso dice que
  hay que congelar otra vez.
- **Bit a bit, cuando se puede**: la toma repetida es bit a bit; la mezcla,
  cuando la pista se repite igual cada vuelta. Un EQ dinámico o un
  compresor no recalculan su reducción por menos de 0,005 dB (guardan algo
  de toda su historia), la fase de un oscilador sigue libre, el redondeo de
  la posición de una rampa cambia un bit que un bitcrush hace escalón y un
  envío lleva a las vueltas siguientes la cola de lo que recibió en la
  primera: ahí la tolerancia es lo que la propia pista cambia sola.
- **Coste**: un `subharm` de 6 voces con distorsión y filtro, en release:
  6,45 % de un núcleo vivo y 0,23 % congelado. Memoria: el WAV de una
  vuelta en estéreo f32 (8 compases a 140 BPM, 13,7 s: 5,3 MB). Congelar
  tarda dos vueltas de render offline (en release, unas 30 veces más
  rápido que en tiempo real con 8 pistas) y bloquea la interfaz mientras.

## Fase W: piano roll editable ✅

Pedida por el usuario: ver y editar las líneas melódicas, los bajos y los patrones de cortes en una cuadrícula, y que cada cambio se escriba de vuelta en el texto Lua. Hechas las seis partes, en cuatro commits: la carga (parte 0), la vista con el teclado, el ratón y la preescucha (partes 1 a 4, que comparten los mismos archivos), la ventana propia (parte 5) y la documentación con la configuración ya hecha (parte 6). El uso, en «Piano roll» de [docs/GUIA.md](GUIA.md). El motor no cambia: todo sale de lo que ya publica (el cursor de la fase M), así que `no_alloc.rs` y `golden.rs` siguen igual.

### Antes

Ya había lo necesario para leer y escribir: `TrackDef::bars` (el ciclo que toca el secuenciador, con `every`, `chance`, `fill`, swing y humanize ya aplicados), `TrackDef::spans` (dónde está escrito cada paso, por vuelta, de la fase M), el tracker editable y el relleno de huecos (fases N y T: cambiar solo los caracteres de un token en `App::grouped` y cargar), `melody()` con nombres, acordes y grados (`script::patterns::note_step`), `key()`/`transpose()` en `Song`, la preescucha de cortes y las notas en vivo (`live_note_on`), y el panel de dispositivos en su ventana (socket aparte, protocolo 5). Faltaba saber si una vuelta estaba escrita con `melody()` o con `steps()`: `1` es el corte 1 en `steps()` y el grado 1 en `melody()` con `key()`.

### Parte 0: la carga

- `TrackDef::melody`: por vuelta del ciclo (como `spans`), si su texto es de `melody()`; `TrackBuild::written_melody` y `BarHandle::written_melody` lo siguen en `:steps`/`:melody` de la pista y en `c:steps`/`c:melody` dentro de `every`. `TokenSpan` se ordena (fila y columna). No cambia el audio.

### Partes 1 a 4: vista, teclado, ratón y preescucha

- **El modelo** (`tui/src/roll.rs`, puro): `roll::build` hace un `Roll` de la definición cargada de la pista seleccionada. Eje: notas (MIDI: raíz + semitonos + acorde) en un instrumento, un sintetizador o una pista con alguna vuelta de `melody()` (raíz C4); si no, cortes. Una `Column` por paso del ciclo (vuelta · longitud + paso) con su golpe, si cambia respecto al patrón base (`varies`), si es generado y su `Target` (posición, token y si es de `melody`) o por qué no se edita. Las `Note`: columna, duración (1 + `hold`, dentro de su vuelta), nota, sombreado por el volumen, marca (retrigger, reverse, pitch de un corte, accent, slide) y estilo (`Scale`, `Outside` con `key()`, `Variation`, `Generated`, `Dropped`: un golpe del patrón que la vuelta quita). La escala que se marca es la que suena: `key().transposed(transpose)`.
- **Qué se puede reescribir** (`roll::build` y `literal`): el texto es el cargado; el paso tiene posición; su cadena no la usa otra pista (las posiciones de las demás pistas, de todas las secciones); y releer el token (con `note_step` en `melody` y las etiquetas de la pista en `steps`, más `transpose()`) da el paso que suena en esa vuelta (sin contar el retraso del swing ni, con humanize, el volumen). Si no, el porqué: texto sin cargar, generado, cadena compartida, variación de `every`/`chance`/`fill`, golpe quitado o pista congelada.
- **Las ediciones** (`roll::Edit`, `roll::apply`): insertar (en un `.`; sobre una nota del mismo paso, al acorde, hasta 4), quitar (de un acorde, esa nota; si no, el token y sus `_`), transponer (la nota del cursor, o con selección todas las que empiezan en ella; en cortes, ±1 es el corte de al lado y la octava, el `^`), alargar y acortar varios pasos, volumen (`@` ±0,1, sin `@` en 1), mover en el tiempo y de tono a la vez (fuentes a `.`, destino solo sobre `.` o sobre la propia nota, sin cruzar la vuelta) y duplicar (sobre silencios; un `_` al principio pasa a `.`). Cada una da un `Change`: los tokens con su texto nuevo (`Plan`: dos vueltas que comparten un token tienen que pedir lo mismo), el cursor y la selección. Antes de devolverlo, cada token nuevo se lee como lo leería la carga.
- **Los tokens**: `with_pitches` escribe las notas como estaban: en `melody`, por su nombre con el estilo del token (`NoteStyle`: mayúsculas, bemoles, octava), sin octava solo la primera si no la llevaba y sigue en la de la raíz, y cada grado sigue siendo grado si la nota nueva es de la escala (`degree_of`); en `steps`, `^n` sobre la raíz (el primero con `%` si lo llevaba). `split_step`/`suffixes`/`replace_parts` cambian solo el tono, el volumen o el corte (`with_slice`): el resto de sufijos (`'`, `r4`, `{...}`, `~800`) se queda donde estaba.
- **El panel** (`tui/src/roll_panel.rs`): `RollView` son solo datos (los dibuja aquí y se mandan por el socket) y `layout` dice dónde cae cada columna y cada nota en un área (lo usan el dibujo y el ratón). Una fila por semitono si cabe la tesitura (con una nota de margen y al menos una octava); si no, dos por fila con medias celdas (pares impar/par: C#/C, D#/D, F/E...; el nombre es el de la natural con `▀`/`▄`). Zoom 1/32 (dos columnas por paso), 1/16, 1/8 y 1/4 (cuadrantes, `quadrant`: dos subcolumnas y dos subfilas por celda) o automático (el más detallado con el que cabe el ciclo). Con una fila y una columna por paso, la cabeza lleva `█▓▒░` o su marca y la cola `━`. Fondo de la tónica y de la escala; `┆` al empezar cada vuelta, `┊` cada compás y `·` cada tiempo; arriba, compases, `v2` en las vueltas y `▼` el paso que se oye; la selección y el paso que se oye, de fondo; el cursor, invertido; abajo, el token del cursor (o por qué no se edita) y la rejilla.
- **En la aplicación** (`tui/src/app/roll.rs`): `RollState` (visible, cursor, ancla de la selección, zoom, rejilla, desplazamiento, área dibujada, arrastre, preescucha, teclas, enlace). `current_roll` guarda el último modelo (`Rc`) por la etiqueta de la definición, la pista y un hash del texto: solo se recalcula al cargar, cambiar de pista o escribir. `roll_playhead`: el cursor del motor de esa pista en el paso que se oye (`Status::heard` menos `[resaltado] retardo_ms`, como el resaltado) si su etiqueta es la de lo cargado; si no (una carga que aún no ha entrado), por la vuelta. Las teclas (`handle_roll`, `tui/src/roll_keys.rs`): las flechas, Shift y las flechas, RePág/AvPág e Inicio/Fin, y las órdenes de `ROLL_KEYS`, cambiables en `[roll.teclas]`; pasan deshacer, rehacer, cargar, marcha, cue, pista siguiente, salir y el pánico. `write_roll_change`: los tokens de atrás adelante (`delete_str` e `insert_str`, sin tocar el portapapeles) dentro de `App::grouped`, y `App::load`.
- **Ratón** (`roll_mouse`, desde `App::mouse` si el piano roll se ve aquí, o desde su ventana): clic en un hueco, nota nueva; agarrar la cabeza y soltar en otro sitio, `Edit::Move` con el tono (a la rejilla); agarrar otro paso, `Edit::Resize`; derecho, quitar; rueda, arriba y abajo; Shift y rueda, a los lados; Ctrl y rueda (`MouseAction::ZoomIn`/`ZoomOut`, nuevos), el zoom. Se escribe al soltar.
- **Preescucha** (`roll_preview`): en un instrumento o un sintetizador, `live_note_on` con las teclas 200 a 203 del motor (las notas del acorde) y se suelta a los pasos que dura (al menos 150 ms) en `tick_roll`; en cortes, `preview_slice`. Nada más suena: ni moverse, ni editar, ni el ratón (`[roll] escuchar_al_insertar` hace sonar la nota que se inserta con el teclado). El pánico corta las notas (`release_pads` y el motor) y olvida la preescucha.
- **Dónde se ve**: en una sola ventana, en el sitio del sample y del tracker (debajo del motor y de la vista de ocupación, si se ve); con el editor en su ventana, en el de la onda grande (sin la vista del sidechain ni la tabla de choques). Paleta: «Piano roll» (`Action::PianoRoll`) y «Editar en el piano roll» (`Action::EditRoll`), atables en `[teclas] piano_roll` y `editar_roll`; desde la ventana del editor van a la principal (`opens_in_main`). Abrir el cursor del tracker o de la ocupación cierra el del piano roll, y al revés.
- **Configuración**: `[roll]` (`Config::roll`, `RollConfig`: `visible`, `zoom`, `rejilla`, `ajustar`, `escuchar_al_insertar`, `terminal`) y `[roll.teclas]`; colores `roll_tonica`, `roll_escala`, `roll_fuera`, `roll_variacion`, `roll_generado` y `roll_seleccion`. Las tablas de la guía salen de `ROLL_CONFIG_KEYS` y `ROLL_KEYS`.
- **Tests**: `tui/src/tests/roll.rs` (la vista con duración, acordes y escala; el panel a 120×40, 100×30 y 80×24; una pista de cortes con sus marcas; cada edición con el texto exacto esperado y la ida y vuelta: lo cargado tiene los mismos pasos que el texto escrito a mano y el mismo audio; comentarios y sangría, la misma línea fuera del token; `steps()` de un instrumento y de cortes con etiquetas; `:length(12)` con `every` y `c:melody`, cada vuelta en su cadena y sin cruzar la vuelta; lo que no se edita y su porqué sin tocar el texto; selecciones que transponen, se duplican y se mueven; grados con `transpose()`; que mover, alargar, transponer, insertar y el ratón no suenen y la preescucha sí, hasta el pánico; el ratón; la rejilla y el zoom), `roll.rs` (tokens, estilos, grados), `roll_panel.rs` (cuadrantes, zoom, `layout` en los dos sentidos y siguiendo al cursor), `roll_keys.rs` y `config.rs`. `cargo test -p tui pantalla_roll -- --ignored --nocapture` enseña la pantalla.

### Parte 5: en su ventana

- `tui --window-roll <script>` (`tui/src/roll_window.rs`): solo el panel, con el ratón si `[editor] raton`. Socket aparte (`link::roll_socket_path`, `-roll.sock`) con las reglas de siempre (carpeta 0700, socket 0600, `SO_PEERCRED`). Del piano roll a la principal, `FromRoll` (`hola`, `tamano`, `tecla`, `raton`); de la principal al piano roll, `ToRoll` (`hola`, `vista` con la `RollView` entera, `adios`, `devuelve`). Protocolo 6.
- **La ventana no evalúa ni suena**: manda su tamaño (el área para el ratón y para desplazar), las teclas y el ratón; la principal los aplica como los suyos (`roll_window_key`: pasan deshacer, cargar, marcha, cue y el pánico; las demás abren el cursor si no estaba y van al piano roll) y manda la vista cuando cambia. Así, con el editor también en su ventana, una edición la hace la principal en su copia del texto y llega al editor con el `ToEditor::Text` de siempre como un paso de deshacer; lo que se escribe en el editor llega a la principal y el piano roll dice «carga antes» hasta que se carga.
- **Lanzar** (`launch_roll_window`, `roll_command`): `[roll] terminal` o `[editor] terminal`; sin terminal, si no se lanza o si se cierra sin conectar, el piano roll se queda en la principal con el porqué. «Volver a meter el piano roll en la principal» manda `devuelve`, suelta el socket y lo enseña aquí; Ctrl-Q cierra la ventana (la principal sigue).
- **Tests** (`tui/src/tests/roll.rs`, `mod window`): la principal, el editor y el piano roll cada uno en su ventana por los sockets de verdad en el mismo proceso: permisos del socket y la carpeta, la vista, una nota puesta desde la ventana que llega al texto de la principal y al editor, lo escrito en el editor sin cargar que no deja editar, el zoom con el ratón, varios tamaños y volver a meterlo; otra versión rechazada; el lanzador sin terminal y con una que no existe; los mensajes de ida y vuelta.

### Parte 6: documentación

«Piano roll» en la sección 2 de la guía (qué se ve, teclas, ratón, nada suena, cómo se escribe, qué no se edita, errores típicos, la ventana y la configuración), una línea en «Qué se puede hacer», un recuadro en el paso 9 del tutorial (y su referencia), y las tablas de la paleta y la configuración regeneradas.

### Decisiones

- **Lo que sonará, no el texto**: como la vista de ocupación, se lee `TrackDef::bars`, así que `euclid`, `rotate`, `mutate`, `shuffle` y los dados de `chance` (tirados al cargar) se ven exactamente como sonarán. El texto solo se lee para escribir.
- **Releer el token antes de escribir**: comparar el paso que dice el token con el que suena en esa vuelta es lo que hace seguro escribir. Cubre de una vez lo que la fase M no sabe distinguir: las vueltas que `every` cambia con `c:pitch()` o `c:reverse()` sin escribir otra cadena, los golpes que añade `fill` sobre un `.`, los que quita `chance`. Lo que no cuadra no se toca; se edita la vuelta escrita o el código.
- **Mover es dos tokens (o más)**: el `.` de destino recibe la nota y sus `_`, y el origen pasa a `.`, todo en un `grouped` y una carga: un paso de deshacer. Solo se escribe sobre silencios para no pisar lo escrito con intención (como el relleno de huecos). Una nota no cruza el final de su vuelta porque cada vuelta puede ser otra cadena (o la misma, en otra posición).
- **Notas por su nombre, con el estilo del token**: un `eb` sigue en bemoles y un `C4` en mayúsculas; los grados siguen siendo grados mientras la nota sea de la escala. Cambia menos el texto que lo que uno escribiría a mano. En `steps()`, `^` sobre la raíz, que es lo que escribe «Grabar lo tocado».
- **El ratón no suena**: lo pide el punto 3; un clic o un arrastre escriben y cargan, pero la nota solo se oye con la preescucha (o al sonar la canción). Al insertar con el teclado, a elegir (`escuchar_al_insertar`, apagado).
- **La ventana es una terminal fina**: mandarle la canción para que pinte sola exigiría evaluar el script allí (cargar y resamplear todos los samples) o un modelo serializado de la canción entera. Con la vista ya calculada (unos 9 KB en el peor caso medido) y las teclas de vuelta, la lógica está una sola vez, en la principal, que es la dueña del texto, y las dos ventanas no pueden separarse.
- **En el sitio del sample y del tracker**: un piano roll necesita alto y ancho; la forma de onda y el tracker dicen otra cosa y vuelven al ocultarlo. La vista de ocupación, si se ve, se queda encima.
- **Una pista a la vez, la seleccionada**: es la que tocan las teclas (Ctrl-O pasa a la siguiente y el cursor empieza en ella), la que suenan los pads y la que se preescucha.

### Coste

- **Hilo de audio**: nada; el motor no cambia (`no_alloc.rs` y `golden.rs`, igual).
- **CPU** (release, el i7 de desarrollo, `cargo test --release -p tui coste_roll -- --ignored --nocapture`, 8 pistas y la seleccionada con 64 pasos, `every` y `chance`, un ciclo de 1024 columnas y 576 notas): `App::view` con el piano roll, 79 µs, contra 72 µs sin él (el modelo se guarda: solo se rehace al cargar, cambiar de pista o escribir; rehacerlo cuesta unos 0,5 ms); dibujar la pantalla de 100×30, 635 µs contra 618 µs. En una pista de 16 pasos, nada que se note.
- **Memoria**: el modelo guardado, 744 bytes por columna (el paso del motor va dentro: 664 bytes) y 32 por nota: unos 12 KB para 16 pasos y 760 KB para el ciclo más largo medido; la vista, las notas y unos campos (8,7 KB por el socket en ese caso, una vez por paso como mucho, y solo si cambia).

### Qué no se edita

Lo generado (`euclid`, `rotate`, `mutate`, `shuffle`, `reverse_pattern`, cadenas hechas a trozos, `string.rep`, cadenas largas `[[...]]` o partidas en varias líneas), las vueltas que cambian `every` (sin `c:steps`/`c:melody` literal), `chance` o `fill`, los golpes que quita `chance`, las cadenas compartidas entre pistas, las pistas congeladas, un texto distinto del cargado, y los grados que `key()` no tiene (se escriben por su nombre). En una pista de cortes no hay acordes (un golpe por paso). Una nota no cruza el final de su vuelta, y mover o duplicar solo escribe sobre silencios.

### Sin verificar en una terminal de verdad

Todo está probado con `TestBackend` y el ratón simulado; nada en una terminal de verdad (Claude Code no puede verla):

- **El ratón**: que la terminal mande los arrastres (`Drag`) y que Ctrl con la rueda llegue como Ctrl (kitty, foot y WezTerm deberían; en tmux depende de `mouse on` y de la versión, y muchas terminales se quedan Ctrl y la rueda para su propio zoom). Si no llega, quedan `z`/`Z`.
- **La legibilidad de la rejilla**: las medias celdas (`▀▄`) y los cuadrantes (`▘▝▖▗▌▐▚▞▙▛▜▟`) dependen de la fuente; `━`, `┆` y `┊` también. Sobre todo, si en 100×30 se distinguen bien las notas de semitonos vecinos en media celda y si los fondos de la escala (235 y 238 de la paleta de 256) se ven en temas claros.
- **La ventana propia** lanzada con una terminal de verdad (kitty, foot) y la regla de Hyprland, como la de dispositivos.
- **La preescucha** por la tarjeta: lo medido es el audio offline.

## Mejoras sueltas

- **Pegado multilínea**: ya lo hacían el pegado con bracketed paste (`App::paste`, `editing::burst_text` para terminales sin él) y la parte 1 de la fase L en la ventana del editor; verificado con sus tests (`pasting_keeps_the_lines_and_the_indent_and_undoes_at_once`, `pasting_with_the_completion_open_accepts_nothing`, `a_burst_of_keys_with_line_breaks_is_a_paste`, `split::pasting_in_the_editor_window_keeps_the_indent_and_undoes_at_once`): líneas y sangría tal cual, sin autocompletado ni sangrado automático, un solo Ctrl-Z.
- **Portapapeles del sistema** (`tui/src/clipboard.rs`): `Clipboard` con un `Runner` (el de verdad, `SystemRunner` con `std::process`; en los tests, `NoRunner` o uno falso). Al crearlo mira una vez si `wl-copy` y `wl-paste` están en el PATH. Ctrl-C y Ctrl-X en el editor (`App::handle_plain`), si había selección, hacen lo de siempre y además mandan el texto por OSC 52 (`ESC ] 52 ; c ; base64 BEL`, que `run_terminal` escribe tras dibujar: lo guarda la terminal, también por SSH; dentro de tmux hace falta `set-clipboard on`) y por `wl-copy` (lanzado sin esperar; un hilo le escribe el texto y lo mata si tarda más de 2 s). Ctrl-Y lee `wl-paste --no-newline` con un límite de 250 ms (un hilo lee su salida; si no llega a tiempo se mata y no cuenta); si devuelve texto, se pega como un pegado del terminal (un solo Ctrl-Z) y pasa a ser también el portapapeles interno; si no está, falla, tarda o está vacío, pega el interno de siempre. En el modo tocar no hacen nada. Sin dependencias nuevas (base64 propio). Tests en `tui/src/clipboard.rs` (base64 y OSC 52; el ejecutor real lee, abandona a tiempo un programa lento y no falla sin él) y `tui/src/tests/clipboard.rs` (con un ejecutor falso: copiar y cortar por OSC 52 y `wl-copy`, nada sin selección; pegar de `wl-paste` y deshacerlo de una vez; sin programas o sin respuesta, el interno; el modo tocar no lo toca).
- **Tracker con 16 filas** (`ui::tracker_first`, `TRACKER_HEIGHT`): el panel de encima del tracker (el sample en una ventana, la forma de onda grande en la principal sin editor) pide su mínimo de siempre (6 filas; un tercio en la principal), salvo que así el tracker no quepa entero y con un mínimo menor sí (4 filas el sample, bordes y dos de onda; 6 la forma de onda grande, con su fila de números y la de los ms): entonces se queda con lo que deja el tracker. Si ni así cabe, el reparto de antes y el tracker se desplaza. Con 120 columnas y dos pistas, el tracker entero aparece desde 40 filas en una ventana (antes, 44) y desde 36 en la principal (antes, 40). Tests en `tui/src/tests/tracker_rows.rs` (las filas que se ven y el alto del sample a varios altos, en las dos ventanas).
- **Diagnóstico de `steps()` con `:length(n)`**: ya estaba (`diagnose::track_lengths`, fase G): cuenta los pasos según el `:length(n)` escrito en la misma cadena de `track(...)` o en la que define la pista (también dentro de `every`), o 16. Verificado con 12, 16 y 24 (un motivo de cuatro pasos repetido, de menos, de más y con `:length` en otra línea); un patrón hecho con `string.rep(...)` no es una cadena escrita y lo cuenta la carga. La mini-notación no tiene operador de repetición. Sin cambios.
- **Fundido en el salto de los bucles de corte** (`engine/src/engine.rs`: `SliceShape::crossfade_ms`, `Seam`, `seam_mix`, `loop_seam_room`): con `play = "loop"` y un fundido de `x` frames, cada vuelta funde sus últimos `x` frames hacia lo que suena justo antes del inicio del corte en el sentido de lectura (al revés con `'`, lo de después del final), como el bucle de un instrumento: al saltar ya suena lo que sigue, el inicio, y la vuelta dura lo mismo que el corte. Si no hay `x` frames antes (el corte 1 al derecho), funde los primeros `x` frames de cada vuelta salvo la primera desde lo que seguiría tras el final; si tampoco, con el lado que más sitio tenga, y si no hay ninguno (un corte con el sample entero), salto seco con aviso (`dry_loop_seam` en `Song::warnings`). Como mucho el corte entero. Lo de fuera del corte se lee con la interpolación de siempre sobre el sample entero; lo de dentro, como siempre. Vale con pitch (`^`), reverse, `%` (cada grano lee con el fundido), los pads y la parte del corte (`length`: lo que queda del corte cuenta como lo de después). El motor tiene 0 por defecto (`SliceShape::DEFAULT`: sin `Seam`, bit a bit lo de siempre); el script, 10 ms (`script::DEFAULT_SHAPE`, `DEFAULT_SLICE_CROSSFADE_MS`, el del instrumento), que solo actúa en los cortes en `loop`: las huellas de `golden.rs` no cambian. El ida y vuelta no salta (da la vuelta leyendo hacia atrás: la onda es continua), así que no lleva fundido. Lua: `crossfade` en `cortes:set` (0 a 1000 ms, registro `api::SLICE_PARAMS`). Tests en `engine/tests/loop_crossfade.rs` (un seno de 444 Hz cuyo salto va de un cero a una cresta: con 0, el corte repetido tal cual y un salto más de 10 veces mayor que el paso del seno; con 10 ms, nunca más de 1,2 veces el paso del seno, la vuelta dura lo mismo, fuera del fundido el corte tal cual, con lo de antes y con lo de después, al derecho y al revés; sin sitio, como 0; ida y vuelta sin salto y sin cambios; rangos), `no_alloc.rs` (bucles fundidos con pitch, reverse y `%`, un pad y cambios de patrón), `script/tests/loop_crossfade.rs` (por defecto sin salto, 0 como antes, aviso) y `script/tests/shapes.rs`.
- **Ayuda del modo tocar**: `Keys::help(tocando, editor)`; la principal con el editor en su ventana (`App::link_view`, `Link::Main`) pide la del modo tocar sin la tecla del editor (allí Esc no hace nada), y la línea de reserva de `HelpLine` también la quita con `editor_outside`. En una sola ventana, igual que antes. Test en `tui/src/tests/main_window.rs` (`the_play_help_says_esc_editor_only_with_an_editor`).
- **Navegación del editor** (`tui/src/tests/navigation.rs`). Ya las daba el widget (`TextArea::input`), verificadas por `App::handle`: Ctrl-←/→ por palabras (con Shift, selección por palabras), Inicio/Fin y Ctrl-A/Ctrl-E de la línea, Ctrl-↑/↓ por párrafos, RePág/AvPág y Ctrl-V/Alt-V por pantallas (ya arriba del todo, RePág deja el cursor en la primera pantalla, no en la línea 1: es del widget), y Shift con todas ellas. Nuevo: Ctrl-Inicio/Ctrl-Fin al principio y al final del texto (`editing::jump_to_edge`, en `App::handle_plain` antes que el widget, que los tomaba por Inicio/Fin; con Shift seleccionan; en el modo tocar nada), y en la paleta «Ir al principio del texto» y «Ir al final del texto» (acciones del lado del editor en la fase L). Ir a la línea: Alt-G (`ir_a_linea` en `[teclas]`; Ctrl-G sigue siendo ir al error) y «Ir a la línea» en la paleta abren «linea » en la línea de órdenes; `linea N` (`App::goto_line`) lleva al principio de la línea N, más allá del final a la última con aviso, y un texto que no es un número no mueve nada. En el editor separado se hace allí (`moves_the_cursor` en `link_command`); en la principal sin editor, «eso se hace en la ventana del editor». Buscar: Ctrl-F (`buscar`; era un sinónimo de → del widget) y «Buscar» en la paleta abren «buscar »; `buscar TEXTO` (`App::find`) pone el patrón del widget (feature `search` de `ratatui-textarea`, que trae `regex`, Rust puro y ya en el `Cargo.lock`) con `editing::search_pattern` (el texto escapado; sin mayúsculas, `(?i)`), va a la primera desde el cursor (`search_forward(true)`) y dice cuál es de cuántas (`editing::search_count`); F6 y F7 (`buscar_siguiente`, `buscar_anterior`; `App::find_again`) la siguiente y la anterior, con vuelta; lo que no sale ni mueve ni resalta; sin texto quita el resaltado (fondo del color `aviso`). Se hace en el editor separado como `linea`. Tests en `tui/src/tests/navigation.rs` (también el resaltado en el buffer de `TestBackend` y las dos ventanas).
- **Tutorial «Tu primera canción»** (`docs/PRIMERA-CANCION.md`; nació como la sección 8 de `docs/GUIA.md` y se separó después, ver «Documentación en dos archivos» más abajo; enlazado desde el principio de la guía): trece pasos que montan una canción entera (carpeta y samples, abrir el proyecto y el editor en su ventana, trocear el break, patrón, segundo sample con `stretch`, efectos por corte y por paso, efectos de pista y automatización, instrumento, melodías y grabar lo tocado, efectos sobre la melodía, bajo con el análisis de choques y un EQ dinámico con sidechain, variación con secciones y variantes, guardar, versiones y exportar), con una tabla de «si algo no suena» y enlaces a la referencia. Solo documentación: sin cambios de código ni de funcionalidad. El script crece con bloques marcados en la apertura: `lua entero` (el script entero: el primero lo empieza y los siguientes deben ser exactamente lo que llevan los pasos), `lua añade` (un párrafo al final) y `lua cambia` (sustituye el párrafo, hasta la línea en blanco, que empieza como el bloque hasta su primer `)`). Tests: `script/tests/guide.rs` (`the_tutorial_song_grows_step_by_step`) sigue los pasos, carga y renderiza cada estado (tiene que sonar) y compara cada `lua entero`; los bloques `lua` de siempre del tutorial (pruebas aparte) los ejecuta el test de siempre. La carpeta de samples de esos tests trae además `perc.wav` (el break sintético a 120 BPM), `nota.wav` (un do central con cuatro armónicos, 2 s) y `bajo.wav` (un do dos octavas abajo), todos sintéticos. En `tui/src/palette.rs`, `the_tutorial_names_real_actions`: cada `F1 «acción»` del tutorial existe en la paleta. Los mensajes de error que cita se han sacado del programa (render offline), y el render de la canción final con los samples de prueba dura 24 compases (33,9 s).
- **«1 compás» en singular** (`script::bars_text`): el render de la línea de comandos (`--render`, también con `--pattern` y `--track`), la alternativa de `--detect-bpm`, `exportar`, `remuestrear` y el informe de choques (texto, mensaje, título de la lista y de la tabla) decían «1 compases». Tests: `one_bar_is_singular` en `script/src/lib.rs`, `export_renders_the_mix_to_wav` (exportar 1 compás) y el informe de choques de un compás en `tui/src/tests/masking.rs`.
- **`--edit` no abre en silencio una ruta que no existe** (`check_edit_target` en `tui/src/main.rs`, antes de abrir la interfaz y el audio): un script que no existe es un error que dice dónde se ha buscado y propone `--new`; con `--new` se abre la plantilla de siempre (se crea al guardar) si su carpeta existe, y si no, error; un script que existe se abre tal cual, con `--new` o sin él. Un proyecto `.sampler` que no existe es siempre un error (se crea con la orden `guardar`). `--new` solo va con `--edit`; `--window-editor` no lo lleva y sigue abriendo la plantilla (lo abre la principal, que ya decidió). Test `edit_refuses_a_missing_script_unless_new` en `tui/src/tests.rs`; la sección 1 y el paso 2 del tutorial lo explican.
- **Nota de cada corte** (`engine/src/pitch.rs`): `detect_note(sample, rango) -> Option<NoteGuess { hz, midi (con decimales), clarity }>` y `detect_notes(sample, cortes)`; `NoteGuess::note`/`cents`/`name` y `note_name(midi)` en cifrado americano con sostenidos, C4 = 60 (`patterns::note_name` la usa). YIN por ventanas: la ventana dura el periodo más largo que se busca (`MIN_HZ` = 40, `MAX_HZ` = 2100: de E1 a C7; en un corte corto, menos), la diferencia `d(τ)` sale de la autocorrelación con `rustfft` y las energías por sumas acumuladas, se normaliza por su media acumulada y el primer valle bajo `THRESHOLD` = 0,15, hasta su fondo y afinado con una parábola, da el periodo. Hasta `MAX_WINDOWS` = 48 ventanas repartidas por el corte, sin los primeros 15 ms (el ataque). Votan las ventanas con periodo y al menos el 10 % del nivel de la más fuerte, con peso `nivel · (1 − d')`; la nota es su mediana pesada (con dos notas en el corte, la que más suena), y si las que tienen periodo no llegan a la mitad del nivel de las que suenan, no hay nota. No corre en el hilo de audio. Lua: `cortes:note(i)` devuelve el nombre y los cents, o `nil` (registro: `note` de `Receiver::Slices`, con ejemplo; error con línea si el corte no existe); el nombre vale tal cual en `melody` y en `root`. Interfaz: `App::slice_notes` (caché por puntero del sample y rangos de los cortes, `slice_notes_key`; se rellena con `update_waveforms`, al cargar; los synth no se analizan), `TrackView::notes` y `TrackView::note(i)`; la vista del sample escribe la nota en cada marca (`ui::slice_mark`: `3 G#4`, `3* voz G#4`), y «Notas de los cortes» (`Action::SliceNotes`) deja en el mensaje la lista (`1 C4 · 2 E4 · 3 — · 4 A4 -30¢`, cents desde `CENTS_SHOWN` = 10). Guía: «La nota de cada corte» en la sección 3 y el mapa. Tests en `engine/tests/pitch.rs` (de E1 a C7, senos y con 8 armónicos, a 44,1 y 48 kHz, a menos de 3 cents; una nota 25 cents alta; ruido, silencio, caja y charles sin nota, y el bombo del break sintético entre F#1 y G1, también bajo el charles del corte siguiente; una melodía de seis notas cortada por transitorios y en rejilla; un corte de dos notas; cortes muy cortos), `engine/src/pitch.rs` (nombres y cents), `script/tests/notes.rs` (nombres y cents desde Lua, en `melody` y `root`, `nil` con ruido, error con línea) y `tui/src/tests/notes.rs` (las marcas, la paleta con cents y sin notas, mover un corte la recalcula).
- **Arrastrar las marcas de los cortes con el ratón** (lo de abajo es como se hizo; desde «Cortes independientes y guardar su edición», arrastrar mueve un borde sin los vecinos, Alt o Ctrl la marca compartida, y todo se escribe en el código en vez del proyecto) (`tui/src/app/mouse.rs`: `SliceDrag`, `App::grab_slice`, `App::mouse_sample`): en la vista del sample (la pequeña, en los dos modos, y la forma de onda grande de la principal sin editor; sin nada abierto encima, `App::nothing_on_top`), un clic a menos de una columna y media de una marca agarra ese corte y lo selecciona; arrastrar mueve su inicio (y el final del anterior) por lo que se mueve el ratón, en frames, sin saltar a la columna pulsada, y se para a 1,2 ms de sus vecinos (algo más que el margen de 1 ms del script, para que el redondeo a 0,1 ms no lo pise). Mientras se arrastra solo cambia la vista (`App::view` pone la marca donde va); al soltar, `App::place_slice` (lo que ya hacía `adjust_slice`, ahora con una posición absoluta en ms del original, también con stretch) lo guarda en el proyecto, recarga y preescucha. Un clic lejos de las marcas selecciona el corte que hay ahí; agarrar y soltar sin moverse, igual. Los cortes añadidos con `:add()` no se arrastran (se mueven en el script). La resolución es la de una columna (un sample de 2 s en 80 columnas, 25 ms); el ajuste fino sigue en `<`/`>`. Un clic nuevo suelta un arrastre que se quedó a medias (soltado fuera de la terminal). `ui::draw` guarda `App::sample_area`. Usa `[editor] raton`. Tests en `tui/src/tests/mouse.rs` (arrastrar y guardar, topes a los dos lados, seleccionar sin mover, con la paleta nada, la ventana principal).
- **Crear y quitar cortes y zoom en la vista del sample** (`tui/src/app/zoom.rs`, `App::edit_slices` con `SliceOp`): `n` en el modo tocar parte el seleccionado en su mitad (lo de «Partir»), Supr o Retroceso lo quita (lo de «Unir»; el corte 1 no), y el clic derecho en la onda pone un corte justo donde se pulsa (`App::split_at`: parte el corte donde cae, o añade uno antes del primero; se guarda como `partir` al microsegundo, en ms del original también con stretch) y sobre una marca (a la distancia de agarrar) lo quita (`App::delete_slice`). Si al cargar no cabe (a menos de 1 ms de un vecino), se deja como estaba, como en la paleta. Zoom: `WaveZoom { track, level, center }`, se ve `frames >> level` (mínimo 256 frames, ~6 ms a 44,1 kHz) centrado en `center` o, sin él, en el inicio del corte seleccionado, sin salirse del sample. `+`/`=` y `-` en el modo tocar y tres acciones de paleta; Shift-←/→ desplaza un cuarto. La rueda sobre la onda acerca y aleja con el punto apuntado fijo y fija `center`; Shift y la rueda o la rueda horizontal desplazan (`MouseAction::RightDown`, `ScrollLeft`, `ScrollRight`). Elegir o mover el corte con las teclas vuelve a seguirlo (`follow_selected_slice`); tocar la onda con el ratón fija la vista antes (`pin_wave_view`), para que no salte al cambiar de corte o al soltar un arrastre. Solo la pista seleccionada (por nombre; en otra se ve entera). El resumen del trozo se calcula del sample con su resolución (`Waveform::window`, hasta 2048 tramos) y se guarda en `App::zoom_cache` mientras no cambien sample ni trozo; `TrackView::window` dice qué frames se ven y `ui` coloca marcas, números, ms del pie (con un decimal por debajo de 200 ms) y el corte que suena relativos a él; el título dice `zoom ×N a-b ms`. Ratón (agarrar, arrastrar, seleccionar) con la misma ventana. Tests en `tui/src/tests/wave_zoom.rs` (teclas, desplazar, tope, otra pista; marcas en su sitio con zoom; rueda con el punto fijo y sin saltar al pulsar; clic derecho pone y quita y se guarda; `n` y Supr) y `app::zoom::tests`.
- **Grupos de pistas** (`engine/src/group.rs`): `Group { fx, automation, gain, pan, sends, mix_curves }` con los mismos `with_*` y validaciones que el patrón, y `Groups { groups: [Option<Group>; MAX_GROUPS], member: [Option<usize>; MAX_TRACKS] }` (`MAX_GROUPS` = 4; `validate`: cada pista va a un grupo que existe). Entran por `Batch::set_groups` (`Command::SetGroups`, guardado aparte en el lote y aplicado al empezar el compás, como los buses). En `Engine::mix`, una pista de un grupo suma su salida (después de su volumen, pan, mute y solo) a `group_buf[g]` (fijo, en el heap) en vez de al master; sus envíos siguen saliendo de ella. Después de todas las pistas, `Engine::mix_groups` pasa cada grupo despierto por su cadena (`run_fx`, o con automatización `modulate_chain`, que es lo que hacía `modulate` sacado de la pista para usarlo también aquí) y lo suma al master y a los buses con su volumen, pan y envíos suavizados y con curvas (`mix_target`, la misma función que usan ahora las pistas, con la misma aritmética). El EQ dinámico de un grupo puede escuchar cualquier pista por sidechain (los grupos se mezclan cuando ya están todas). Estado por grupo (`GroupState`): efectos, parámetros movidos, volumen, pan y envíos suavizados; al entrar otro grupo, `update` conserva el estado de los efectos del mismo tipo; un grupo que pasa `GROUP_TAIL_SECONDS` (1 s) sin recibir nada y sin sonar se duerme (`sleep`: efectos sin memoria, todo en su sitio) y no se procesa, así que sin grupos la salida es la de siempre bit a bit. `Status::groups`: el pico de cada uno. Lua: `group(nombre, { pistas })` (global, fuera de `section()`; devuelve `GroupHandle`; `group(nombre)` sin lista da el ya creado) con `:fx`, `:gain`, `:pan` y `:send`, que comparten con los de la pista `fx_arg`, `gain_arg`, `pan_arg` y `send_arg`; errores con línea: pista repetida o ya en otro grupo, lista vacía, más de 4, uno que no existe o que ya existe; al acabar, `check_groups` comprueba que cada pista y cada sidechain existen en alguna sección o variante. `Song::groups: Vec<GroupDef>`; `song_groups` los pasa al motor con la ranura de cada pista (`resolve_sidechains`, compartido con las pistas). Registro: `Receiver::Group` («grupo») y sus cuatro métodos. Interfaz: una columna por grupo detrás de las pistas en el panel de efectos (`GroupView`, `ui::group_column`: `▣ bateria ← kick, caja`, volumen, pan, envíos, nivel con `App::group_meters` y su cadena) y `· ▣ bateria` en el título de cada pista (`TrackView::group`); la ayuda de `grupo:fx` dice el efecto como la de la pista. Tests en `engine/tests/groups.rs` (un grupo sin nada es bit a bit lo mismo; volumen y pan exactos; solo cambian sus pistas; una distorsión en el grupo suena como una pista que tocara la suma y no como cada una distorsionada aparte; los envíos de la pista no pasan por el grupo y los del grupo mandan la suma, al frame exacto del eco; entran al compás; errores), `no_alloc.rs` (dos grupos con compresor, filtro con macro, EQ dinámico con sidechain, envíos y volumen con curva, y macros moviéndose), `script/tests/groups.rs` y `tui/src/tests/groups_macros.rs`.
- **Macros** (`engine::Curve::Macro { index, from, to }`): una curva que no depende de la posición sino de un valor de 0 a 1 (`MAX_MACROS` = 8; con escala logarítmica para las frecuencias, como las demás). `Curve::value`/`value_at` reciben ahora las macros (`Macros = [f32; MAX_MACROS]`), que el motor pasa en los relojes de cada trozo (`Clock`, `BusClock`, `SongClock`, `grain::Position`) y en `SliceShape::at`, `Knob::at` y `drive_block`: así vale en todo lo que admite `ramp`/`lfo` (efectos de pista, de corte y de grupo, volumen, pan y envíos, buses, sintetizadores, granos y lo de por golpe), sin más código en cada sitio. El valor entra con el lote (`Batch::set_macros`, `Command::SetMacros`, al compás) y se mueve en vivo con `Controller::set_macro` (`Command::SetMacro`, inmediato; también cambia los lotes que esperan, para que una recarga mandada antes no deshaga lo movido después). Los parámetros lo siguen con su suavizado de siempre (~5 ms). Además, una voz que empieza desde el silencio en una pista con volumen, pan o envíos con curva suena ya con el valor de la curva (como hacían los efectos automatizados con `snap`) y no hace un fundido desde el valor fijo; sin curvas no cambia nada. Lua: `macro(nombre, desde, hasta)` devuelve una curva (`CurveHandle`, como `ramp`/`lfo`): la macro se numera por orden de aparición (`Builder::macro_index`); `macro_init(nombre, valor)` (fuera de `section()`) da el valor inicial (0 sin él). `Song::macros: Vec<MacroDef { name, value }>`. `ScriptHost::set_macro` la mueve en el motor y en el modelo (las dos variantes) y la recuerda por nombre (`macro_values`: el valor del script en la última carga y el de vivo); al recargar, se queda el de vivo salvo que el script cambie su `macro_init`, y lo que se exporta suena con los valores de ahora. Para no gastar sitio en la cola de comandos, `song_batch` solo manda los grupos si la canción tiene (o tenía, para quitarlos) y las macros si tiene. Interfaz (`tui/src/app/macros.rs`): «Macros» en la paleta (`Action::Macros`, atable en `[teclas] macros`; con el editor en su ventana se abre en la principal) abre `MacrosPopup` (barra y valor de cada una): ↑↓ elige, ←→ ±0,05, `-`/`+` ±0,01, 0-9 salta a 0..0,9, Inicio y Fin a 0 y 1, Esc o Enter cierra; `[midi] macros = [21, 22]` (`MidiConfig::macros`, `macro_for`, que respeta `canal` salvo con MPE): cada CC de la lista mueve su macro (0-127 → 0-1) y no llega como expresión. El título del panel de efectos lista las macros con su valor. Tests en `engine/tests/macros.rs` (el volumen sigue a la macro, se mueve ya y sin saltos, un lote que espera conserva lo movido después, rangos, escala logarítmica), `script/tests/macros.rs` (números por orden, `macro_init`, suena como el número que da, recargar conserva o manda el script, en todos los sitios de las curvas, errores con línea), `script/tests/registry.rs` y `tui/src/tests/groups_macros.rs` (panel, teclas, recarga, CC, aviso de configuración, ayuda de `grupo:fx`). Guía: «Grupos de pistas» y «Macros» en el mapa, las columnas de los grupos y la lista de macros en «Leer el panel de efectos», `[midi] macros` en «MIDI» y dos recetas, «Pegar la batería con un grupo» y «Una macro para el subidón» (`scripts/ejemplos/15-grupo-bateria.lua` y `16-macro-subidon.lua`).
- **Cada ajuste de corte conserva su posición** (`engine::Slices::with_starts`, `script::cut_sample`, `App::place_slice`). Causa: los ajustes del proyecto (`corte = ...`) se aplicaban uno a uno en el orden de la lista, que `ScriptHost::set_position` deja con el último tocado al final, y cada uno se comparaba con los vecinos tal como estaban en ese momento, a menudo todavía los del detector. Un ajuste que solo cabía con su vecino ya movido (el 2 más allá de donde el detector ponía el 3, con el 3 movido a la derecha) se rechazaba en cuanto se tocaba otra vez el vecino y quedaba detrás en la lista: el corte volvía a su sitio original con un aviso que `place_slice` no miraba (solo buscaba el del corte que se movía). Ahora `with_starts` pone todos los inicios a la vez y solo después busca choques (dos inicios a 1 ms o menos, o el último pasado el final): de cada pareja que choca deshace el ajuste de la derecha (o el otro, si ese no está ajustado) y repite hasta que todo cabe; los deshechos se avisan como antes, con el sitio que tendrían con los demás puestos. En la interfaz, `,` `.` `<` `>` y soltar una marca ya no se niegan a moverse al llegar a un vecino: se quedan a 1,2 ms de él (como el arrastre) y el mensaje lo dice («tope: el corte N»); nunca mueven ni reinician a los demás. El proyecto ya guardaba cada ajuste como posición absoluta y lo lee tal cual. Tests: `script/tests/mixer.rs` (`slice_adjustments_do_not_depend_on_their_order`: tres órdenes dan los mismos cortes; dos que chocan, el de la derecha avisa y el otro se aplica) y `tui/src/tests.rs` (`each_adjusted_slice_keeps_its_place`: el 3, el 1 y el 2 en ese orden, otro cambio en el 3, los topes a los dos lados, el proyecto y al reabrirlo; con el código anterior el 2 volvía a su sitio).
- **Mover un corte no suena** (`App::place_slice`, `App::browse_slice`, `tui/src/app/mouse.rs`). Causa: `place_slice`, por donde pasan `,` `.` `<` `>`, «Quitar el ajuste» y soltar una marca arrastrada, terminaba con `preview_selected()`, que manda `Controller::preview_slice` (la voz de preescucha); y ← → (`browse_slice`) también preescuchaban cada corte al elegirlo. Ahora mover, quitar el ajuste y elegir con ← → solo cambian la posición o la selección. Suena: la tecla del corte en el modo tocar (el pad, con el volumen, los efectos y el choke de la pista), espacio y «Preescuchar el corte seleccionado» (la preescucha, pedida a propósito) y un clic sin arrastrar en la onda: lejos de las marcas, al pulsar (selecciona y preescucha el corte de ahí); sobre una marca, al soltar sin haberla movido (al pulsar todavía puede ser un arrastre). Sin ratón en la terminal, ← → y espacio. Las líneas de ayuda del modo tocar lo dicen. Tests: `moving_a_slice_is_silent_and_its_key_plays_it` (parado: `,` `.` `<` `>` y quitar el ajuste dejan el render a cero; la tecla `2` y espacio, no), `arrows_in_play_mode_browse_and_space_previews_slices` y `mouse::a_click_on_the_wave_selects_and_plays_the_slice_there` (el clic suena; agarrar sin soltar no; arrastrar la marca no). Sin probar en una terminal de verdad: el clic depende de que la terminal mande los eventos del ratón (`[editor] raton`, y dentro de tmux `set -g mouse on`).
- **Botón de pánico y límite de voces** (`engine/src/engine.rs`: `Command::Panic { hold }`, `Command::Resume`, `Command::SetVoiceLimit`, `Engine::panic`, `steal_oldest`, `make_room_for_synth`; `engine::synth::Synth::stamp`; `tui/src/app.rs`: `App::panic`; `[voces]` en `config.rs`). No había forma de callar lo que suena sin parar el secuenciador (Stop lo paraba todo) ni límite de voces entre pistas: solo el de cada pista (`voices` del instrumento, 1-16, y del sintetizador; los granos, `MAX_GRAINS` = 32 por pista), con robo de la más antigua dentro de la pista. Pánico: por la cola de comandos lock-free de siempre, inmediato (no espera al compás). Para cada pista hace lo de `Stop` sin parar el transporte (quita el retrigger pendiente, la voz pasa a su fundido de 1,5 ms, `kill_notes` corta notas, sintetizador y nube, y sus notas MIDI se apagan) y además `Voice::hush` gasta la cola de los efectos de corte (`VoiceFx::spend_tail`; sin eso, hasta 100 ms); la preescucha también. Los buses de envío y los efectos de pista siguen (su cola es de la pista, no de la voz). Con `hold`, `Engine::hushed`: `play_hit` vuelve sin disparar (el secuenciador sigue contando, también `every`, las curvas y la línea de tiempo) y `Play`, `NoteOn`, `GrainOn` y `Preview` sueltan su sample a la cola de basura sin sonar (`SynthOn`, que no lleva nada en el heap, se ignora) hasta `Resume`; el metrónomo sigue. Un pánico sin `hold` no quita la pausa. Se publica en `Status::hushed` (un atómico) y la barra de transporte dice `PÁNICO: pausa`. Límite de voces: `DEFAULT_VOICE_LIMIT` = 64 (hasta `MAX_VOICE_LIMIT` = 256) notas de instrumento y voces de sintetizador que cuentan, entre todas las pistas. Antes de abrir una nota (si su pista no está ya llena, que entonces roba la suya como siempre) o una voz de sintetizador que ocupe una más (no la de un monofónico que ya suena), mientras se llegue al límite se roba la más antigua de todas: las notas llevan como `born` un contador global del motor (`Engine::stamp`; antes uno por pista, solo se usaba para ordenar y casar las notas MIDI) y las voces del sintetizador un `stamp` aparte (su `born` siembra las fases y no se toca: el sonido no cambia). Bajar el límite roba ya lo que sobra. La voz de cortes de cada pista (una) y los granos no cuentan. Interfaz: F12 (`panico`) y Shift-F12 (`pausa_panico`, pone y quita la pausa) en `[teclas]` y en la paleta; se miran al principio de `App::handle`, antes que la paleta, el explorador, los cursores o el modo vim, y desde la ventana del editor van a la principal como las demás acciones del motor. Sueltan antes las notas de los pads. `[voces] limite` se manda al motor en `set_config`; el render offline usa el de por defecto. Tests: `engine/tests/panic.rs` (todo a la vez: secuenciador, pad, nota, sintetizador y preescucha; tras el pánico, ninguna voz y el render a cero en 200 ms tras el fundido, la posición intacta, un pad y el compás siguiente vuelven a sonar; la cola de un efecto de corte se corta; en pausa no entra nada en un compás entero y el secuenciador cuenta, un pánico puntual no la quita y reanudar vuelve a sonar; el límite roba la nota más antigua de otra pista y bajar el límite roba ya), `no_alloc.rs` (límite con robo entre pistas, pánico puntual y en pausa con todo lo que no entra, reanudar), `tui/src/tests/panic.rs` (F12 con la paleta abierta, Shift-F12 dos veces con la barra de estado, F12 en pausa, teclas y `[voces]` desde la configuración con sus avisos) y `split::the_panic_keys_from_the_editor_window_reach_the_main_one`. `golden.rs`, idéntico. Sin probar en una terminal de verdad: que F12 y Shift-F12 lleguen (dentro de tmux y por SSH, y en terminales desplegables que usan F12).
- **Edición de cortes**: duraciones (`dur`, `fit`), el corte como instrumento y deformaciones; tiene su propia sección, «Edición de cortes», más abajo.
- **Cortes independientes y guardar su edición**: lo editado en la onda se escribe en el código (`slice({ {inicio, fin}, ... })`) y se guarda con Ctrl-S; cada corte con su inicio y su final; su propia sección, «Cortes independientes y guardar su edición», más abajo.

### Decisiones

- **Grupos: un nivel, sin enrutado libre.** CLAUDE.md deja fuera el
  mezclador completo con buses; lo que no se podía hacer de ninguna
  manera era comprimir o saturar varias pistas juntas. Un grupo es eso:
  una suma con su cadena, su volumen, su pan y sus envíos. Sin grupos
  dentro de grupos ni pistas a varios sitios, el orden de mezcla sigue
  siendo fijo (pistas y después grupos) y no hay círculos que comprobar.
  Los envíos de las pistas no pasan por el grupo (como en una mesa: el
  envío sale del canal), así la reverb de la caja no se comprime con la
  batería; quien quiera la suma en la reverb, `grupo:send`.
- **Macros como una curva más.** Una macro es otra forma de dar un valor
  en lugar de un número, igual que `ramp` y `lfo`: hacerla una variante de
  `Curve` hace que valga en todos los sitios donde valen ellas, con sus
  rangos, su escala logarítmica y su suavizado, sin tocar cada parámetro.
  El precio es pasar las macros al evaluar las curvas (un argumento más y
  un campo en los relojes de cada trozo). Recargar no las devuelve a su
  `macro_init` porque se mueven en vivo, a menudo mientras se edita y se
  guarda; si se cambia el número en el script, es que se quiere ese.
- **Nota de los cortes: YIN y no la FFT.** Buscar el pico de la FFT da
  muchos errores de octava con material real (el segundo armónico suele
  ser más fuerte que la fundamental en un bajo o una voz); YIN busca el
  periodo y con el primer valle bajo el umbral evita los de octava abajo.
  La mediana pesada de muchas ventanas aguanta un ataque, una cola que se
  apaga o un trozo de la nota siguiente. Solo nombres con sostenidos (`C#`,
  no `Db`): son los que escribe `note_name` y los que acepta `melody` tal
  cual. Se calcula al cargar y se guarda por sample y cortes, porque la
  vista se dibuja muchas veces por segundo; no se guarda en el proyecto
  (sale igual cada vez).
- **`--edit` y `--new`.** Antes, una errata en la ruta abría la plantilla
  como si fuera un script nuevo, y al guardar creaba otro archivo. Ahora
  hay que pedirlo. `--new` con un script que ya existe no es un error (lo
  abre): así el mismo comando sirve la primera vez y las siguientes.
- **Tutorial: un script que crece, comprobado paso a paso.** Repetir el
  script entero en cada paso lo haría muy largo; enseñar solo trozos sin
  comprobarlos deja que el texto y lo que lleva el lector se separen. Con
  `añade` y `cambia` cada paso enseña solo lo nuevo, y el test reconstruye
  lo que tendría el lector y lo compara con los scripts enteros de los
  pasos 3, 4, 7, 11 y el final. El loop entero va con `slice_grid(1)` y no
  con `slice(1)`, que empieza en el golpe más fuerte (en el break
  sintético, la caja a 428,6 ms).
- **Portapapeles: OSC 52 siempre y `wl-copy` si está**, sin crates de
  portapapeles (arrastran X11/Wayland en C). OSC 52 sirve por SSH, que es
  como se trabaja aquí; `wl-copy` cubre las terminales que no lo admiten.
  Pegar no puede usar OSC 52 (leer el portapapeles por la terminal casi
  nunca está permitido): o `wl-paste`, o el pegado de la propia terminal
  (Ctrl-Shift-V, bracketed paste), o el interno. El límite de 250 ms es lo
  que como mucho se para la interfaz al pegar; copiar no la para nunca.
- **Fundido de los bucles fuera del corte, no dentro.** Fundir el final
  con el principio del propio corte obliga a acortar cada vuelta (lo
  fundido ya ha sonado), y el bucle cambiaría de duración. Fundir con lo
  que suena justo antes del inicio (como el instrumento) conserva la
  duración y el ataque de cada vuelta; lo de fuera es la cola del golpe
  anterior, a un salto de 10 ms no se distingue. 10 ms por defecto desde
  el script porque el objetivo es que no haga clic sin pedirlo; en el
  motor, 0, para que todo lo que no lo pide siga bit a bit igual.

### Documentación en dos archivos

Sin tocar el código ni la funcionalidad. `docs/GUIA.md` es la referencia (secciones 1 a 7, que conservan su numeración: la 8 era el tutorial, así que no hubo que renumerar nada y los «sección N» del registro de la API y de los mensajes siguen valiendo) y `docs/PRIMERA-CANCION.md` el tutorial, con título propio, un enlace a la guía, una nota de lo que hay que saber antes de empezar y los pasos como `##` (las anclas `#paso-N-...` no cambian).

- **Enlaces de ida**: cada paso empieza con una línea `> **Referencia:**` con enlaces con ancla a los apartados de la guía que usa (funciones de Lua, atajos, recetas...); los `[sección N](#...)` que ya había en el texto ahora llevan `GUIA.md#...`.
- **Enlaces de vuelta**: al final de cada apartado de la guía que usa el tutorial, `> **Usado en el tutorial:**` con los pasos. Los apartados de la API generada por el registro no llevan el suyo (se reescribirían al regenerar): hay uno solo después de `api:fin` con los pasos y las funciones que usan.
- **Tests** (`script/tests/guide.rs`): los bloques de Lua de los dos archivos se ejecutan y las recetas de `scripts/ejemplos/` coinciden con alguno; el tutorial se reconstruye paso a paso desde `PRIMERA-CANCION.md`; `every_internal_link_points_to_an_existing_anchor` comprueba que todo enlace interno (dentro de cada archivo y entre los dos, y a otros archivos de `docs/`) apunta a un archivo y un ancla que existen (anclas al estilo de GitHub, con `-1` en los títulos repetidos), y `every_tutorial_step_links_to_the_guide_and_back` que cada paso tiene su línea de referencia y la guía enlaza de vuelta a cada paso. `tui/src/palette.rs` (`the_tutorial_names_real_actions`) lee el tutorial. Las tablas generadas de la paleta y la configuración siguen en `GUIA.md`.

## Edición de cortes

Pedida por el usuario, sin fase propia: lo que se puede hacer con un corte
sin copiar su audio (duraciones), tocarlo como instrumento y deformarlo.
Sigue la línea de las formas por corte (`SliceShape`), el pitch sin
cambio de duración (`Shift`), los granos y los efectos por corte y por
paso. Antes de empezar se comparó lo pedido con lo que ya había y se
decidió con el usuario: la sintaxis va en `cortes:set` y en las llaves de
un paso (no `corte[3]:len()`), el modo «pitch» del instrumento es el
`keep` de siempre (no un `mode` nuevo), `stutter` se queda fuera (ya lo
hacen `r4` y `chance("stutter")`), `rev` existía como `'`, el `formant`
precalculado no se hace (ya está en tiempo real) y lo precalculado se
calcula al cargar, con caché, como `stretch`.

### Parte 1: variantes de duración

- **Motor** (`engine/src/engine.rs`): `SliceShape` suma `dur:
  Option<Dur>`, `fit: Fit` y `loop_start_ms`/`loop_end_ms`. `Dur::{Ms,
  Note { num, den }, Bars}`: una figura es fracción de redonda (16 pasos:
  `"1/8"` son 2 pasos en cualquier compás) y los compases son los del
  compás que suena; `Dur::frames(bpm, pasos del compás, frecuencia)` la
  pasa a frames de una vez desde los frames por paso del transporte
  (redondeo único, nada acumulado: sin deriva), al sonar cada golpe
  (`Engine::dur_frames`, en `play_hit` y en los pads), con el tempo de ese
  momento (también en una rampa de BPM). `Voice::lasting` (lo que era
  `Voice::new`, que sigue con `dur = None`): con `Fit::Trim` la voz dura
  `min(dur, lo del corte)` y su rampa de salida (`release`, al menos el
  fundido mínimo de 1,5 ms) cae al final; con `Fit::Stretch`, un `Shift`
  con `advance` = (frames del corte − 1)/(dur − 1) (acotado a ×16 y
  ÷16, `MAX_FIT_STRETCH`): los granos alineados de `%` recorren el corte
  a esa velocidad y leen al pitch del golpe, así que el tono no cambia (y
  `^` lo mueve sin tocar la duración); con `Fit::Loop`, `Sustain { start,
  end, fade }` dobla los desplazamientos de la voz al tramo
  (`Sustain::fold`) hasta `dur` y funde en lineal los últimos `fade`
  frames antes de `end` con los de antes de `start` (como el bucle de un
  instrumento; `fade` = `crossfade`, como mucho lo que hay antes del
  inicio y el tramo). Sin puntos, el tramo es la mitad central
  (`SliceShape::loop_span`: de un cuarto a tres cuartos). Va en el eje de
  la voz, así que vale con `'`, `^` y `%`. `Shift::with_advance` (con 1,
  la multiplicación es exacta: lo de siempre bit a bit). `dur` con `play`
  distinto de `"normal"` es un error (el bucle ya no acaba solo).
- **Por paso** (`engine::HitShape`, `Step::hit`, `Trigger::hit`): lo que
  el paso dice manda sobre la forma de su corte (`SliceShape::with_hit`,
  en `resolve_bar` y `with_shapes`, con la validación y el número del
  paso). En un instrumento, un sintetizador o una pista con granos, error
  (`Pattern::no_hit_shapes`). Notación: un valor `n/d` en las llaves
  (`FxValue::Fraction`) y `StepBraces::Hit { fx, hit }`; las llaves cuyas
  claves son de la forma (`api::STEP_SHAPE_KEYS`: `dur`, `fit`,
  `loop_start`, `loop_end`) van aparte de las de efectos
  (`step_fx::hit_shape`), así que `3{dur=1/8}{lp=800}` lleva las dos
  cosas y `{dur=1/8,lp=800}` es un error.
- **Lua**: `cortes:set(i, { dur = 250 | "1/8" | "2 compases", fit =
  "stretch", loop_start = 40, loop_end = 120 })` (`step_fx::dur_text`
  acepta `"n/d"`, `"n compases"`, `"1 compás"`, `"n bars"` y `"2bars"`;
  en las llaves, `dur=250`, `dur=1/8` o `dur=2bars`). Registro: cuatro
  claves más en `api::SLICE_PARAMS` (autocompletado, ayuda y guía; no
  admiten curvas, y la tabla de la guía dice por qué).
- **Tests**: `engine/tests/slice_dur.rs` (la duración en ms, figuras y
  compases a 120 y 170 BPM, a ±1 frame; recortar más que el corte; la
  cola de `release`; el bucle se sostiene sin saltar más que el propio
  seno y sin fundido sí salta; el tramo central por defecto, muestra a
  muestra; estirar ×2, ÷2 y ×3 a ±1 frame con el tono a menos de 0,5 %
  por YIN, también con `^7`; el paso sobre el corte; sin `dur`, igual
  bit a bit; rangos), `no_alloc.rs` (las tres variantes con `^`, `%`,
  reverse, retrigger, gate, lo del paso y un pad), `script/tests/
  slice_dur.rs` (ms, figuras y compases desde Lua; suena como el motor;
  llaves; errores con línea), `script/tests/registry.rs` (las claves por
  paso son de `SLICE_PARAMS`) y los de siempre: `golden.rs` sin cambios.

### Decisiones

- **Figuras de redonda, no del compás**: `"1/8"` es una corchea también
  en 7/8 (en 4/4 es lo mismo que «un octavo del compás»); para «el compás
  entero» está `"1 compás"`.
- **La duración se calcula al sonar**, no al cargar: con una rampa de
  tempo cada golpe dura lo que toca en su momento.
- **`trim` más largo que el corte acaba con el corte**: no se rellena con
  silencio ni se repite (para eso, `fit = "loop"`).
- **`stretch` con los granos del pitch sin cambio de duración**, no con
  la nube de granos (`Cloud`, que hace texturas) ni con el WSOLA de
  `sample:stretch()` al cargar: en tiempo real, sin copias, y la misma
  calidad medida en la fase K.

### Parte 2: el corte como instrumento

- **Lua** (`script/src/lib.rs`): `cortes:instrument(i, {...})` o
  `cortes:slice_at(i):instrument{...}` (método de `SlicedHandle`, con el
  número como `set`; con varios cortes y sin número, error que dice cómo
  elegir uno; con `stretch`, error como en el sample) con la misma tabla
  que `sample:instrument` (`instrument_options`, ahora sobre una
  `InstrumentSource`: el sample, el rango que toca y si es un corte). El
  instrumento guarda el corte (`InstrumentHandle::slice`: sus `Slices`
  de un corte, su origen y su etiqueta), y `track_source` hace la pista
  con ese corte en vez del sample entero: las notas leen solo su rango.
  Los ms de `loop` cuentan desde el inicio del corte y tienen que caber en
  él. **Raíz**: sin `root` en un corte, la de YIN (`detected_root`, con
  `engine::detect_note` sobre el corte, como `cortes:note`): la nota más
  cercana y lo que se desvía va a `Instrument::tune`; si no hay tono claro
  (un golpe de ruido), error con la línea que pide `root`. `root = "auto"`
  hace lo mismo, también en `sample:instrument`. Con `root` a mano, `tune`
  es 0. `SlicedHandle` lleva los avisos del sample (los del bucle).
- **Motor** (`engine/src/instrument.rs`): `Instrument::tune` (semitonos,
  ±0,5, `MAX_TUNE`; validado): `open_note` multiplica la velocidad de
  cada nota por `2^(−tune/12)` (secuenciador y notas en vivo), así una
  nota pedida suena en su frecuencia aunque el sample esté desafinado.
  Con 0 no se toca nada (bit a bit lo de siempre).
- **Los dos modos**: el de siempre (sampler clásico: tono y duración a la
  vez) y `keep = true` (granos alineados: la misma duración). No hay
  `mode` nuevo. Con `loop`, sostiene notas largas (también en el release).
- **Registro**: función `instrument` del receptor `Receiver::Slices`
  (después de las demás de los cortes; plantilla `instrument(1, { root =
  "C3" })`), y `"auto"` en la clave `root`.
- **Tests**: `script/tests/slice_instrument.rs` (la raíz y los cents de
  YIN: un la3 da 57 y 0, un do4 subido 30 cents da 60 y +0,30; `root` a
  mano y `"auto"`; do3, mi3, sol3 y do5 sobre ese do4 medidos con YIN: error
  máximo 0,004 % como sampler y 0,065 % con `keep`; la duración cambia
  como sampler (el doble a −12, la mitad a +12, con los 30 cents) y no con
  `keep`; bucle en el corte con un acorde de tres voces; errores con
  línea), `no_alloc.rs` (un corte como instrumento con `tune`, acordes,
  bucle, con y sin `keep`, y notas en vivo con sus note off) y la medida
  de coste (ignorada) en `engine/tests/instrument.rs`.

### Coste y polifonía

`cargo test -p engine --release --test instrument -- --ignored
--nocapture coste`, en el i7 de desarrollo, con bucle y `tune`:

| Voces | Sampler clásico | `keep = true` |
| --- | --- | --- |
| 1 | 0,7 % | 3,9 % |
| 4 | 2,5 % | 15,3 % |
| 8 | 5,2 % | 30,6 % |
| 16 | 12,0 % | 64,4 % |

(Porcentaje de un núcleo.) Como sampler, cada voz cuesta ~0,7 % y caben
de sobra las 16 de una pista y las 64 del límite entre pistas (~45 %).
Con `keep`, ~3,9 % por voz: el límite de 64 se pasaría de un núcleo (no
es un límite que proteja la CPU), así que con `keep` conviene no pasar de
8 voces por pista y de unas 20 en total. No se añade otro límite: el
de voces entre pistas (`[voces] limite`) ya se puede bajar.

### Parte 3: deformaciones en tiempo real

- **Motor** (`engine/src/engine.rs`): `SliceShape` suma `bend`
  (semitonos, ±36, `MAX_SLICE_BEND`), `bend_ms` (1 a 10 000 ms, 100 por
  defecto) y `reverse`; `bend` y `bend_ms` admiten curvas por golpe
  (`SHAPE_BEND`, `SHAPE_BEND_TIME`; `SHAPE_PARAMS` pasa a 7), y
  `HitShape` lleva los tres por paso (el paso quita la curva del corte).
  `Bend { rate, k, frames }`: la velocidad de lectura va de la del golpe
  a `· 2^(bend/12)` en `bend_ms`, recta en semitonos, y luego se queda;
  la posición es la integral exacta (`Bend::offset`: `r·T/k·(e^(k·n/T) −
  1)` durante la curva, recta después), calculada desde el número de
  frame (sin acumular), y la interpolación usa la velocidad de ese frame
  (`Bend::rate_at`) para su filtro antialias. La duración de un golpe que
  se lee una vez sale de invertirla (`Bend::frames_for`): al bajar, el
  corte dura más, como con `^`. La curva se acorta si la velocidad final
  pasaría de ×8 o ÷8 (`MAX_BEND_RATE`: más, la interpolación cuesta
  demasiado). Vale con `^`, `'`, `play = "loop"`/`"pingpong"` y `fit =
  "trim"`/`"loop"`; con `%` o `fit = "stretch"` los granos leen a un tono
  fijo, así que es un error al hacer el patrón (`check_bend`, con el
  número del paso; si solo llegase por una curva, la voz lo ignora). Sin
  `bend` (0) la voz no crea nada y lee como siempre: bit a bit.
  `reverse` da la vuelta a la lectura en la voz (`Voice::lasting`: también
  en los pads) y en el golpe (`with_shapes`: también en las nubes de
  granos); con `'` a la vez, sigue al revés.
- **Lua y llaves**: `cortes:set(i, { bend = -12, bend_time = 200, rev =
  true })` (números, `ramp()` o `lfo()` en `bend` y `bend_time`; `rev`,
  `true`/`false` o 1/0) y `3{bend=-12,bend_time=200}`, `3{rev=1}`
  (`api::STEP_SHAPE_KEYS`, sin rampas de golpe: la propia curva ya lo es).
  Registro: tres claves más en `api::SLICE_PARAMS`.
- **`stutter`, fuera**: ya lo hacen `r4` (el paso se repite 4 veces) y
  `chance(p, "stutter", n)` (alias de `retrig`); decidido con el usuario.
- **Tests**: `engine/tests/slice_bend.rs` (200 Hz que caen una octava en
  200 ms: la frecuencia medida por cruces por cero sigue `200 · 2^(−n/9600)`
  a menos de 0,3 % en cuatro instantes, y desde `bend_time` es 100 Hz a
  menos de 0,3 % por YIN; subida con `^-12`; la duración del corte con
  `bend` es la integral exacta a ±2 frames; `rev` es `'` muestra a
  muestra y la rampa al revés exacta; el paso manda; una rampa de `bend`
  cambia golpe a golpe; con los valores por defecto, bit a bit; con `%` y
  `stretch`, error; rangos), `no_alloc.rs` (bend fijo, con curva y por
  paso, con `^`, ida y vuelta, `fit = "loop"`, `rev` del corte y del paso
  con `'`, retrigger, gate y un pad; y las dos curvas nuevas en la prueba
  de todas las curvas por golpe) y `script/tests/slice_bend.rs` (suena
  como el motor, curvas, llaves, errores con línea).

### Parte 4: deformaciones precalculadas y formant por corte

- **Precalculado** (`engine/src/deform.rs`): `Warp` (de 1 a 6 puntos
  interiores `(x, y)` crecientes; `Warp::midpoint(y)` es uno en la mitad;
  `Warp::at(x)`, recta entre puntos por (0, 0) y (1, 1)), `RenderKey {
  start, end, warp, smear }` y `render(sample, clave)`, que reserva y es
  para el hilo de control. `warp`: un `Shift` a velocidad 1 con el ancla
  de cada grano donde dice la curva (`Shift::with_warp`, en vez de
  `advance`): la duración y el tono no cambian, el tiempo sí. `smear`:
  una `Cloud` de granos de 60 ms (66 por segundo, solape 4, semilla fija)
  que recorre el corte a velocidad 1 con la posición desplazada al azar
  hasta `smear · 80 ms`, igualada en RMS al original y mezclada con él en
  esa proporción (`(1 − s)·seco + s·granos`), con 5 ms de fundido al
  final. Primero `warp`, luego `smear`. No hay técnicas nuevas: son los
  granos de `%` y la nube de `grains`.
- **El patrón** (`engine/src/sequencer.rs`): guarda el rango de cada corte
  y sus formas (`with_shapes`); `Pattern::render_keys()` dice qué hay que
  precalcular (cada forma de corte y cada golpe con `warp` o `smear`, sin
  repetir) y `Pattern::with_rendered(buffers)` los pone: cada golpe de esos
  cortes suena de su buffer (`Trigger::variant`, con su rango rehecho
  sobre el buffer y su `length`), y los pads de esos cortes también
  (`Pattern::slice_rendered`, que mira `Command::Play` si el sample es el
  del patrón: un `Arc` más, sin reservas; el original vuelve por la cola de
  basura). Si falta alguno, error. La interfaz sigue viendo la posición
  del corte original (`Pattern::hit_position`). Todo va en el patrón: entra
  al compás en su lote, la voz que suena sigue con su buffer (lleva su
  `Arc`) y el patrón viejo se suelta por la cola de basura como siempre.
- **Al cargar** (`ScriptHost::apply_deform`, en `finish` tras expandir
  cada pista): monta el patrón con sus formas, le pide las claves y
  calcula los buffers (`TrackDef::rendered`) en el hilo del script, con
  caché por sample y clave (`Deformed`: la de la carga anterior y las de
  otras pistas; mismo `Arc`). `track_pattern` los pone con
  `with_rendered`. Mientras se calcula, suena lo de antes (el motor no se
  entera hasta el lote); si la carga falla, el modelo y el patrón de antes
  siguen.
- **`formant` por corte y por paso**, en tiempo real: `SliceShape::formant`
  (con curva por golpe, `SHAPE_FORMANT`; `SHAPE_PARAMS` pasa a 8) manda
  sobre el de la pista (`pista:formant`) en la voz (`Voice::lasting`); en
  un golpe sin pitch, la voz pasa por los granos como un `%0`, así que
  el timbre se mueve sin cambiar la nota (con 0, nada cambia). Con `^`,
  error al hacer el patrón (los formantes se mueven con el tono, como
  siempre). No hay `formant` precalculado: el de tiempo real ya estaba.
- **Lua y llaves**: `cortes:set(i, { warp = 0.3 | { {x, y}, ... }, smear
  = 0.5, formant = 5 })` y `3{warp=0.3}`, `3{smear=0.5}`, `3{formant=-3}`.
  Registro: tres claves más en `api::SLICE_PARAMS` (`warp` y `smear` no
  admiten curvas: se calculan al cargar; `formant`, por golpe).
- **Tests**: `engine/tests/deform.rs` (con `warp = 0.3` un corte de 300 Hz
  y luego 600 Hz cambia de nota en el 64,3 % del tiempo y no en la mitad,
  y cada nota queda a menos de 0,5 % por YIN; la duración es la del
  corte; los puntos y sus errores; `smear` dura lo mismo, sale igual dos
  veces, a menos de 2 % de la nota y con su nivel; un buffer nuevo entra
  al compás: el primero es el de antes muestra a muestra, el segundo el
  nuevo, y la unión no salta más que el seno; las claves sin repetir y el
  error si falta un buffer; `formant(7)` sube el formante del armónico 6
  al 8-10 con la fundamental a menos de 0,5 %, con 0 bit a bit, con `^`
  error; la medida del coste, ignorada), `no_alloc.rs` (cortes
  precalculados del corte y del paso con `fit = "stretch"`, `%`, reverse
  y retrigger, cambiarlos al compás con los golpes sonando, pads de esos
  cortes, `formant` por corte y su curva) y `script/tests/deform.rs`
  (suena como el motor, caché entre cargas y entre pistas, llaves y
  `formant`, errores con línea y el modelo de antes sigue).

### Coste del precálculo

`cargo test -p engine --release --test deform -- --ignored --nocapture
coste`: por cada segundo de corte, `warp` 20,7 ms, `smear` 5,0 ms y los
dos 24,0 ms (en debug, 130, 56 y 178). Con la caché, solo cuando cambia
el corte o sus valores: hacerlo al cargar, síncrono, no se nota.

### Parte 5: guía, recetas y snippets

- **Guía**: «Editar un corte: duraciones, notas y deformaciones» en las
  recetas (sección 6), un tutorial corto en cuatro pasos (duraciones,
  instrumento, `bend`/`rev`, `warp`/`smear`/`formant`) con las
  limitaciones (transponer percusión o ruido con granos suena
  artificial: para melodías con bombos y cajas, el sampler clásico;
  `formant`, `stretch` y `smear` van mejor con material tonal; `bend` no
  va con `%` ni `stretch`; el coste de `keep`; la vista del sample enseña
  el original); una línea en «Qué se puede hacer» y la fila de las llaves
  de forma en la tabla de la mini-notación. La API (las claves de
  `cortes:set` y `cortes:instrument`) sale del registro como siempre.
- **Recetas**: `scripts/ejemplos/17-corte-duraciones.lua`,
  `18-corte-instrumento.lua`, `19-corte-bend.lua` y `20-corte-warp.lua`,
  iguales a sus bloques de la guía (`script/tests/guide.rs`, ahora 20).
- **Snippets** (los «presets» del editor, en `api::SNIPPETS`):
  `duraciones`, `corte-melodia`, `bombo-bend` y `warp`
  (`script/tests/registry.rs` los ejecuta).

### Sin probar de oído

Lo medido está arriba (duraciones a ±1 frame, tono por YIN, saltos en las
uniones); que suene bien lo dice el usuario con auriculares.

## Lanes de automatización

Pedida por el usuario, sin fase propia: una curva de puntos que lleva un
parámetro a lo largo del progreso de la pista, con duración propia (como
las envolventes de clip no enlazadas de un DAW). Decidido antes de
empezar: la sintaxis es `track("x"):auto("lowpass.cutoff", puntos, {...})`
(las pistas no se indexan), y un nombre ambiguo va al primero con aviso;
la lane es una curva más (`Curve::Lane`, que apunta a una tabla fija del
motor), así que vale donde vale `ramp()` y se suaviza y combina igual; las
posiciones van en compases; `once` vuelve al valor fijo al acabar y `hold`
se queda en el último; el panel es aparte (el tracker es vertical) y solo
escribe en el código si la tabla de puntos está escrita tal cual.

### Parte 0: el contexto de evaluación

`automation::CurveEnv { macros, lanes }`: lo que mira una curva además de
la posición, por referencia, en los relojes de cada trozo (`Clock`,
`BusClock`, `SongClock`, `grain::Position`); `Curve::value_in` lo recibe
(`value` y `value_at` siguen como atajos con las macros). El motor lo arma
al empezar cada trozo con copias locales, sin tocar `self`. Commit aparte,
con `golden.rs` idéntico y `no_alloc.rs` en verde.

### Parte 1: el modelo y la evaluación

- **`engine/src/lane.rs`**: `LanePoint { at (pasos desde el inicio de la
  lane), value, curve }` y `LaneCurve::{Linear, Exp, Smooth, Step}` (cómo va
  ese punto hacia el siguiente: recta; exponencial, la media geométrica,
  con un extremo en 0 o negativo, recta; smoothstep; o se queda y salta).
  `Lane::new(puntos, length, log)` valida (de 1 a `MAX_LANE_POINTS` = 64
  puntos, posiciones crecientes y sin repetir dentro de `0..=length`,
  `length` de más de 0 a `MAX_LANE_STEPS` = 1024 pasos, valores > 0 con
  `log`) y guarda los puntos en un array fijo (Copy, ~1 KB). `Lane::at(t)`
  busca el tramo por bisección (`partition_point`, sin reservas): antes del
  primer punto, el primero; después del último, el último; en los puntos,
  su valor exacto. Con `log`, `linear` y `smooth` interpolan en
  logaritmos (frecuencias). `Lane::value(steps)` la repite cada `length`
  pasos.
- **La tabla**: `Lanes { lanes: [Option<Lane>; MAX_LANES] }` (32 huecos,
  ~32 KB; las dos constantes, fáciles de subir). Entra entera por el lote
  (`Batch::set_lanes`, `Command::SetLanes(Box<Lanes>)`, con su `staged` y
  `pending` como las macros) al empezar el compás, en el mismo que los
  patrones que la siguen; la vieja vuelve por la cola de basura
  (`Garbage::Lanes`). Una tabla vacía es ninguna.
- **`Curve::Lane { index, min, max }`**: sigue a la lane `index`; `min` y
  `max` son sus extremos, así que la validación de rangos de siempre sirve
  sin mirar la tabla (y `check_curve` rechaza un índice de 32 o más). Si
  el hueco está vacío, `min`. Como cualquier curva, vale en los efectos de
  la pista, de un corte y de un grupo, la mezcla, los buses, el
  sintetizador, los granos y la forma de un corte por golpe; se evalúa cada
  32 frames y se suaviza (~5 ms) como siempre. Los parámetros del
  sintetizador escalan sus extremos (`Param::curve`); los puntos los escala
  quien hace la lane.
- **En la mezcla**: `Engine::mix` saca la tabla de su `Option<Box>` durante
  el trozo (`mix_with` la presta a los relojes) y la devuelve al acabar:
  solo se mueve un puntero. Los golpes (`per_hit`, fuera de la mezcla) la
  leen en su sitio.
- **Tests**: `engine/tests/lanes.rs` (las cuatro curvas en los puntos y a
  un cuarto y a mitad de tramo, la escala log, exp con un extremo negativo,
  antes del primer punto y la repetición; validación; una lane mueve el
  volumen de una pista en el motor a ±0,004 en seis pasos; un hueco vacío
  da el mínimo; una tabla vacía, bit a bit lo de siempre; un índice que no
  existe, error) y `no_alloc.rs` (cinco lanes con las cuatro curvas en dos
  filtros, el volumen y el bend de los cortes, y la tabla cambiada al
  compás sonando).

### Parte 2: modos, reloj y prioridad

- **Modos** (`LaneMode`, `Lane::with_timing(modo, offset, rest)`): `Loop`
  se repite cada `length` pasos y `offset` corre su fase; `Once` solo
  actúa de `offset` a `offset + length` y fuera da `rest`, el valor fijo
  del parámetro (el que pone el script: así, al acabar, el parámetro vuelve
  a donde estaba); `Hold` da el primer punto antes de empezar y el último
  después. `Lane::phase(steps)` dice dónde va (o que no actúa) y
  `Lane::bounds` cuenta `rest` en `once` (la validación de rangos lo ve).
  `offset` va de ±1024 pasos.
- **Reloj**: como todas las curvas, la posición es la de la canción (o de
  su sección) en pasos, calculada en cada bloque desde el contador de
  frames del transporte, cuyos compases son un número entero de frames:
  no se acumula nada, así que no deriva. Sigue al tempo (también en una
  rampa de BPM) y a `start_at`.
- **Prioridad** (la de las curvas, sin nada nuevo): el `~` de un golpe
  manda sobre la lane del primer filtro hasta el golpe siguiente; un mando
  MIDI (MIDI learn) se aplica encima según su peso; en un parámetro que se
  lee al nacer el golpe (la forma de un corte: `bend`, `attack`...), lo
  que diga el paso en sus llaves manda en ese golpe (`SliceShape::with_hit`
  quita la curva); `{lp=800}` en un paso es otro filtro, en la voz de ese
  golpe (antes de los de la pista), y no toca la lane. Un parámetro tiene
  una sola curva: una lane o un `ramp`/`lfo`.
- **Tests**: `engine/tests/lane_modes.rs` (los tres modos con `offset`;
  sin deriva: una lane de 3 compases con las cuatro curvas contra un
  patrón de 4 a 170 BPM, 2000 vueltas en release y 200 en debug, cada
  muestra frente a la cuenta exacta sobre la rejilla entera de compases a
  menos de 0,008 (el suavizado de 5 ms en las rampas) y la vuelta 5 igual
  a cada una de las siguientes en la misma fase del patrón a menos de
  1e-5; arrancar en el compás 30 000 y en el 65 535 da el valor exacto;
  saltos `step` de 0,1 a 1 sin pasar del salto de un polo de 5 ms, ni al
  cambiar la tabla al compás; `~` gana a la lane del filtro; `{bend=0}`
  en un paso gana a una lane de bend) y `no_alloc.rs` (los tres modos con
  `offset`).

### Parte 3: la API de Lua

- **`script/src/lanes.rs`**: `pista:auto(parámetro, puntos [, opciones])`,
  `pista:auto_clear([parámetro])`, `pista:auto_list()` y `lane(puntos [,
  opciones])`. Cada punto es `{posición en compases, valor [, curva]}`
  (`"linear"`, `"exp"`, `"smooth"`, `"step"`); las opciones (`len`,
  `mode`, `offset`, `range`, `scale`, `rest`, registro `api::LANE_PARAMS`
  y `LANE_TABLE` para el autocompletado) aceptan compases (número),
  `"N compases"`, `"N pasos"`, una figura (`"3/4"`) o `"N ms"` (al tempo
  de la canción al cargar). Sin `len`, hasta el último punto redondeado a
  compases. Los compases son los de `meter()` en ese momento.
- **El destino** (`lanes::candidates`, `resolve`): todo lo que en la pista
  admite curvas según el registro, en este orden: los parámetros de sus
  efectos (`"lowpass.cutoff"`, también `"lp.cutoff"`, `"filter.cutoff"` y
  `"2.cutoff"` por su posición), `"gain"`, `"pan"`, `"send.delay"` y
  `"send.reverb"`, `"pitch"`, `"formant"`, los del sintetizador
  (`"synth.cutoff"`) y los de la forma de los cortes (`"slice.bend"`...).
  Un nombre suelto de varios sitios (`"cutoff"` con dos filtros, `"pan"`)
  va al primero y avisa: en `Song::warnings` con la línea y como segundo
  valor de `auto` (`local t, aviso = pista:auto(...)`; encadenar sigue
  valiendo). Un nombre completo o un alias no es ambiguo. Lo que no admite
  curvas, error con la lista de lo que sí.
- **Dónde va la curva** (`lanes::assign`): en el sitio de siempre de las
  curvas de ese parámetro, que sustituye: la automatización del efecto
  (`TrackDef::automation`), `mix_curves`, `pitch_curve`, `formant_curve`,
  las curvas del sintetizador (sus puntos, escalados a las unidades del
  núcleo) o, para la forma de los cortes, `TrackBuild::shape_lanes`, que
  se pone al acabar en la copia de las formas de esa pista (las de la
  variable son de todas sus pistas). `once` toma como `rest` el valor fijo
  del parámetro (el de su `fx`, su volumen...).
- **La tabla**: `Song::lanes: Vec<Option<LaneDef>>` (el índice es el del
  motor; `LaneDef` guarda la pista, el parámetro, la lane, los puntos tal
  como se escribieron, el rango y dónde se llamó) y `TrackDef::lanes` (sus
  parámetros e índices). Volver a poner el mismo parámetro reusa su hueco;
  `auto_clear` lo libera. Con 32 llenos, error con la línea («no caben más
  lanes») y la carga anterior sigue. `song_batch` manda la tabla entera si
  hay lanes o las había (para quitarlas); sin lanes, ni se manda.
- **Congelar**: una lane en `loop` se repite cada `len` (si son pasos
  enteros); `once` y `hold` avisan de que la pista congelada no suena
  igual.
- **Registro**: funciones `auto`, `auto_clear` y `auto_list` (pista) y
  `lane` (global); tabla de opciones con su autocompletado y su ayuda.
- **Tests**: `script/tests/lanes.rs` (cada clase de destino; el aviso de
  un nombre ambiguo y los nombres completos sin aviso; opciones; `lane()`
  en `:fx` y en `:gain`; `auto_clear`, `auto_list` y la reutilización de
  huecos; 12 errores con línea, más de 32 lanes, y el modelo de antes
  sigue; congelar).

### Parte 4: el panel, de solo lectura

- **Lo que publica el motor** (`engine/src/status.rs`): `Status::lanes:
  [Option<LaneLive>; 32]`, con `LaneLive { value, phase }` de cada lane de
  la tabla. `Engine::publish_lanes`, al empezar cada trozo (a tasa de
  control), calcula el valor y la fase de cada lane en la posición de la
  canción y los escribe con `SharedStatus::set_lane`: los dos f32 en un
  solo `AtomicU64` por lane (así nunca se leen de trozos distintos), sin
  bloqueos; sin lanes no hace nada, y al quitarlas las borra una vez. La
  fase es `None` fuera del tramo de una `once` (y el valor, el fijo).
- **El panel** (`tui/src/lanes_panel.rs`): `LanesView::new(song, pista,
  status)` (puro) junta las lanes de la pista seleccionada
  (`TrackDef::lanes` y `Song::lanes`) con lo publicado; `LanesPanel`
  dibuja por lane su título (parámetro, duración en compases, modo y
  offset) y su curva en 4 filas de octavos de celda (cada columna, rellena
  hasta el valor de la lane en su centro, entre el mínimo y el máximo de
  sus puntos; en escala log si la lane lo es), sus puntos (`●`) y el
  cursor (la columna de su fase: la curva en el color de acento y lo vacío
  con `│`). Paleta: «Lanes de automatización» (`Action::LanesView`, abre y
  cierra; desde la ventana del editor va a la principal). Va abajo del
  sample y el tracker, o abajo de la forma de onda grande con el editor en
  su ventana.
- **Tests**: `engine/tests/lanes.rs` (lo publicado: la fase en el último
  bloque, el valor de la lane en esa fase, una `once` pasada con su valor
  fijo y sin fase, un hueco vacío y que se borran al quitarlas) y
  `tui/src/tests/lanes.rs` (la paleta lo abre y lo cierra con las lanes y
  sus títulos; el cursor sigue al motor; en pantalla; la curva, los puntos
  y el cursor en sus celdas). `cargo test -p tui pantalla_lanes --
  --ignored --nocapture` lo enseña.

### Parte 4b: la lectura en vivo

En el mismo panel, de lo que publica el motor (`Status::lanes`, un
atómico por lane a tasa de control): a la derecha del título de cada lane,
el valor de ahora (`▸ 631`, `▸ 0.80 fijo` fuera del tramo de una `once`;
`lanes_panel::number`: sin decimales de 100 en adelante, uno de 10 a 100,
dos por debajo) y una barra de 12 columnas con su nivel entre el mínimo y
el máximo de la lane; y sobre la curva, un `◆` en la columna del cursor a
la altura del valor (encima de los puntos). Es el valor de la lane, antes
del suavizado de 5 ms y de un mando MIDI o un `~` que manden sobre ella.
Test en `tui/src/tests/lanes.rs` (el valor de la lane del gain a mitad de
su segundo compás, su texto, la barra llena, el rombo en la columna del
cursor, «fijo» y el formato de los números).

### Parte 5: editar con el ratón y escribir en el código

- **Dónde está escrita la tabla** (`script/src/written.rs`:
  `call_table`, `TableSpan`, `span_text`): al llamar a `auto("param",
  puntos, ...)`, en la línea de la llamada (o en las de alrededor, para una
  cadena de métodos) se busca `auto(` seguido de esa cadena, una coma y una
  `{`, y su `}` (también en varias líneas, saltando lo de dentro de las
  cadenas). `lanes::written_table` exige que el texto sea literal
  (`script::parse_points`: solo `{posición, valor [, "curva"]}`, espacios,
  saltos de línea y una coma final; nada de variables, cuentas ni
  comentarios) y que diga lo mismo que se leyó; si no, `LaneDef::written`
  guarda el motivo y cómo pasarla a literal. `script::points_text` la
  vuelve a escribir con su forma: en una línea si estaba en una, o un
  punto por línea con la sangría que tenía; `number_text`, lo más corto
  (hasta 4 decimales).
- **La escala de la vista** (`LaneDef::scale`): la de `range` o la del
  parámetro, logarítmica en las frecuencias (las del registro de efectos);
  el panel dibuja y edita con ella (`LaneRow::level`, `value_at`).
- **La edición, pura** (`tui/src/lanes_edit.rs`): `LaneEdit::{Add, Move,
  Delete, Cycle, Nudge}` sobre una lane da los puntos tal como se escriben
  (compases y, con `range`, de 0 a 1) y el elegido. Las posiciones van por
  pasos enteros, crecientes y sin repetir (mover se queda entre sus
  vecinos), los valores dentro de la escala, y una lane se queda con al
  menos un punto (para quitarla, `auto_clear`). `Cycle`: recta → exp →
  suave → salto → recta. `Nudge`: un paso y un cuarentavo de la escala.
- **En la interfaz** (`tui/src/app/lanes.rs`): clic en un hueco de la
  curva añade un punto (en el paso y el valor del centro de esa celda);
  clic sobre un punto (a una columna) lo elige y arrastrarlo lo mueve (la
  vista lo enseña mientras, y se escribe al soltar); clic derecho lo
  borra. Con un punto elegido: `c` cambia su curva, Supr o Retroceso lo
  borra, ← → un paso, ↑ ↓ un cuarentavo, Tab el siguiente y Esc lo suelta;
  un clic fuera del panel también. Cada edición reescribe la tabla de su
  `auto()` en el texto (`replace_text`: un paso de deshacer), carga y deja
  elegido el punto. No toca nada, y dice por qué, si la tabla no es
  literal (el título lo marca, «solo lectura»), si el texto ha cambiado
  desde la última carga o si esa carga falló. Del código al panel no hace
  falta nada: el panel lee el modelo que sale de cada carga. Con el editor
  en su ventana, la principal guarda el texto y le manda el cambio, como
  con el piano roll.
- **Tests**: `script/tests/lanes.rs` (dónde está escrita en una línea y en
  varias, por qué no con una variable y con una cuenta; leer y escribir la
  tabla), `tui/src/lanes_edit.rs` (añadir en orden y en su paso, mover
  entre vecinos, borrar, un punto como mínimo, cambiar la curva, afinar,
  con `range` de 0 a 1) y `tui/src/tests/lanes.rs` (un clic añade y Ctrl-Z
  lo quita; arrastrar no escribe hasta soltar y la vista lo enseña; `c`,
  flechas, Supr y clic derecho; solo lectura con su motivo; el texto
  cambiado espera a cargar; una tabla en varias líneas conserva su forma).

### Parte 6: grabar en vivo («touch»)

- **La lógica, pura** (`tui/src/touch.rs`): una toma son (fase de la
  lane, valor) en el orden en que se tocaron. `last_pass` se queda con la
  última vuelta (un salto atrás de más de media lane es una vuelta nueva,
  en `loop`) y con lo último que se tocó en cada posición (volver atrás
  dentro de la vuelta borra lo de después); `simplify` es Ramer-Douglas-
  Peucker en la altura de la escala de la vista (tolerancia 1 %); `merge`
  deja los puntos de la lane de fuera del tramo tocado y pone dentro la
  toma simplificada en rectas, subiendo la tolerancia si no cabe en 64
  puntos.
- **En la interfaz** (`tui/src/app/lanes.rs`): «Grabar lanes (touch)» en
  la paleta la arma y la desarma (`Action::LaneTouch`). Armada y con el
  transporte en marcha, se graba en la fase de ahora de la lane (la que
  publica el motor):
  - un mando MIDI aprendido (MIDI learn) cuyo parámetro tiene lane en su
    pista (los nombres son los mismos: `lowpass.cutoff`, `gain`,
    `send.reverb`, `synth.cutoff`): su CC llevado a su rango y curva. Se
    oye mientras se mueve, porque el mando ya manda sobre la curva; la
    toma se cierra con el mando 0,6 s quieto (`tick_touch`);
  - arrastrar en vertical sobre la curva de una lane en el panel (la
    posición la pone el cursor, no el ratón); se cierra al soltar. Con el
    ratón no se oye hasta que se escribe.
  Al cerrarse, la toma se escribe en la tabla de su `auto()` por el mismo
  camino que las ediciones (un paso de deshacer, y carga); desarmar cierra
  lo que quede abierto.
- **Limitación**: el segundo efecto con el mismo nombre (`lowpass#2` en
  los mandos) no se casa con su lane.
- **Tests**: `tui/src/touch.rs` (una recta son dos puntos y una esquina
  tres, lo que se desvía menos que la tolerancia fuera, la última vuelta,
  la toma en su tramo y lo de fuera intacto, una toma muy movida cabe en 64
  puntos) y `tui/src/tests/lanes.rs` (con el ratón, arriba un rato y abajo
  otro, se escribe al soltar con los dos valores y el punto de fuera
  intacto; con un mando asignado al volumen, se escribe tras 0,6 s quieto;
  armar y desarmar).

### Parte 7: documentación

«Automatizar con lanes» en las recetas de la guía: un tutorial en cinco
pasos (un barrido de 4 compases sobre un patrón de 3, pan suave y saltos
`step`, `once` y `hold` con `range` y `scale`, el panel y la edición con
el ratón, y la grabación «touch») con sus límites; una línea en «Qué se
puede hacer». Recetas `scripts/ejemplos/21-lane-barrido.lua`,
`22-lane-pan-y-saltos.lua` y `23-lane-una-vez.lua`, iguales a sus bloques
(`script/tests/guide.rs`, ahora 23), y snippets `lane-barrido` y
`lane-pan`.

### Sin probar de oído ni en una terminal de verdad

Lo medido está arriba (curvas en sus puntos, sin deriva en 2000 vueltas,
saltos acotados por el suavizado); que suene bien y que el ratón y los
mandos se sientan bien lo dice el usuario.

### Después: el valor en vivo visible

Pedido aparte. La publicación ya existía (parte 4b): el canal es el del
resaltado en vivo (atómicos en `SharedStatus` que la interfaz lee en una
instantánea en cada `tick`, cada 30 ms) y cada lane va en un solo
`AtomicU64` con el valor y la fase (dos f32), mejor que un `AtomicU32` por
valor: el par nunca se lee de trozos distintos.

- **Motor, tests** (`engine/src/status.rs`, `engine/tests/lanes.rs`,
  `no_alloc.rs`): un escritor en otro hilo pone valor y fase iguales en las
  32 lanes sin parar y el lector, en 20 000 instantáneas, nunca los ve
  distintos (sin tearing); valor y fase vuelven exactos, también `None`;
  en una lane de escalones el valor publicado a mitad de cada compás es el
  de su punto exacto, y en una rampa exponencial el de la curva en la fase
  publicada; y `no_alloc.rs` comprueba que lo publicó dentro del tramo sin
  reservas.

- **En la interfaz**: `TrackView::live` (`App::live_params`): por cada
  lane de la pista, a qué va (`script::LaneTarget`, el destino público de
  `LaneDef::target`), lo que vale ahora (lo publicado, en las unidades de
  Lua) y su nivel en la escala de la lane. En el panel de efectos, junto a
  cada parámetro con lane, `∿cutoff ▸631 Hz ██░░░` (el valor con su
  unidad tras `▸`, en el color de acento: en vivo, no lo escrito, que sigue
  en el texto del efecto: `LP 3000 Hz`); el volumen `vol ▸0.30∿` con su
  barra en el valor de ahora; el pan `pan ▸+0.30`; los envíos `→ d▸0.30`.
  Un parámetro con `ramp`/`lfo` sigue con su `∿` solo. El panel de lanes
  dice la unidad y «en vivo» en su lectura. Unidades (`tui/src/units.rs`,
  solo texto: lo que suena no se redondea): Hz (kHz desde mil) en los
  cortes y frecuencias, dB en umbrales, `makeup`, `range` y las ganancias
  del EQ, ms en ataques, colas y `bend_time`, semitonos en `pitch`, `bend`
  y `formant`. Se redibuja cada 30 ms (~33 Hz).
- **Arreglado**: las lanes de un parámetro del sintetizador con escala
  (`envmod` del acid y sus parientes) llevan sus puntos en las unidades del
  núcleo; `LaneDef::unit_scale` lo dice y la vista y la edición los pasan a
  las de Lua (antes, el panel los enseñaba y los habría escrito mal).
- **Tests**: `tui/src/units.rs` (cada unidad) y `tui/src/tests/lanes.rs`
  (en el panel de efectos, `∿cutoff ▸…` con el valor publicado y `vol
  ▸0.30∿`; sin lanes, nada; el `envmod` de un acid en sus unidades de Lua,
  en la vista y en los puntos del panel).

- **Guía**: «Lo que vale ahora» en «Automatizar con lanes»: cómo leer
  `▸` (en vivo) frente a lo escrito y frente a `∿` sin `▸` (`ramp`/`lfo`).

## Visualizaciones

Pedidas por el usuario, sin fase propia: visualizaciones en la terminal al
estilo de Strudel, por pista o del master, activables desde Lua. Decidido
antes de empezar: `punchcard` y `pianoroll` salen del modelo del piano roll
(no se duplican la vista de ocupación ni el piano roll editable); la
sintaxis es `track("x"):scope{...}` y `master():spectrum{...}` (las pistas
no se indexan); las muestras van por colas `rtrb` (las del proyecto) con
el flag por el lote al compás; la FFT es propia; y la degradación a ASCII,
por configuración y por `LANG`.

### Parte 1: las muestras

- **`engine/src/viz.rs`**: una cola `rtrb<f32>` (lock-free, un productor y
  un consumidor) por pista del motor y otra del master (`VIZ_SOURCES` = 9,
  `VIZ_MASTER` = 8), de `VIZ_CAPACITY` = 16 384 muestras mono (unos 0,34 s
  a 48 kHz) cada una: 576 KB entre todas, reservados al crear el motor.
  `VizWriter::push_stereo`/`push` copian lo que cabe y cuentan lo que
  descartan (nunca bloquean ni reservan); `VizReader::drain_into` lo pasa
  a un `Vec` del hilo de control. `VizTap` (en el motor): las colas y qué
  fuentes están encendidas.
- **En el motor**: cada pista copia, si su visualización está encendida,
  su salida en mono tras su volumen, su pan y su mute (lo que va al
  master); el master, tras el limitador (lo que sale). Apagada, no se toca
  la cola: solo se mira un `bool`. Qué fuentes copian entra por el lote
  (`Batch::set_viz`, `Command::SetViz`, con su `staged` y `pending`) al
  empezar el compás. La interfaz vacía con `Controller::viz_samples`.
  El cálculo (ventanas, FFT, disparo) y el dibujo, en el hilo de la
  interfaz.
- **Tests**: `engine/src/viz.rs` (llena, descarta y cuenta; da la vuelta a
  la cola veinte veces; un productor y un consumidor en dos hilos con
  400 000 muestras: siempre en orden y lo perdido es solo lo descartado),
  `engine/tests/viz.rs` (apagadas, nada en ninguna cola; encendidas, la
  del master es la salida en mono muestra a muestra y la de la pista lo que
  suena; llena, se queda en su capacidad y vuelve a llenarse al vaciarla;
  el encendido entra al compás) y `no_alloc.rs` (apagadas no escriben;
  las nueve encendidas, llenas y vaciándose, sin reservas). `golden.rs`,
  idéntico.

### Parte 2: `punchcard` y `pianoroll`

- **`tui/src/viz/`**: `VizKind` (las seis), `VizSource` (una pista por su
  nombre o el master), `VizSpec` (qué, de dónde, `size` en filas y
  `bands`) y `VizView`, lo ya calculado que dibuja `VizPanels`: los
  paneles apilados, cada uno con su borde y su título (`punchcard ·
  drums`).
- **La rejilla** (`viz/grid.rs`): de `roll::build` (el modelo puro del
  piano roll: el ciclo entero que sonará, con `every`, `chance`, `fill` y lo
  generado ya resueltos), una fila por nota o corte y una columna por paso
  del ciclo, estirada al ancho: `█` donde empieza un golpe, `▒` mientras
  dura (sus `_`), `·` vacío. `punchcard` solo con las filas que suenan;
  `pianoroll`, todas las del rango (de la más alta a la más baja) con su
  nombre a la izquierda. El cursor es la columna que se oye, con la
  sincronía del resaltado en vivo (`App::viz_playhead`, como el del piano
  roll), en vídeo inverso. Con `theme::ascii()`, `#`, `=`, `.` y `|`.
- **En la interfaz** (`tui/src/app/viz.rs`): `App::set_viz(specs)` las
  pone (la API llega después) y manda encender en el motor las colas de
  las de audio (un lote, al compás); `App::viz_views` las calcula para la
  vista. Van apiladas arriba del sample y el tracker, o de la forma de onda
  grande con el editor en su ventana.
- **Tests**: `tui/src/tests/viz.rs` (el punchcard de `drums`: dos filas,
  los golpes en sus pasos, el cursor y en pantalla; el pianoroll de un acid
  de do2 a sol2, las ocho filas con su nombre; en ASCII). `cargo test -p tui
  pantalla_viz -- --ignored --nocapture` las enseña.

### Parte 3: `scope`

- **Las muestras en la interfaz** (`App::tick_viz`): en cada `tick`, las
  colas de las fuentes encendidas se vacían (con un `Vec` que se reutiliza)
  a un historial por fuente de `viz::HISTORY` = 16 384 muestras (lo
  último que ha llegado).
- **`viz/scope.rs`**: `trigger(muestras, ventana)` es el último cruce por
  cero de subida (habiendo bajado de −0,01 antes: sin esa histéresis, el
  ruido dispara en cualquier sitio) que deja la ventana entera detrás
  (`WINDOW` = 2048 muestras, ~43 ms a 48 kHz). Con una señal periódica,
  cada cuadro empieza en la misma fase y la onda se queda quieta; sin
  disparo, lo último (y el título lo dice). El dibujo, en el `Canvas` de
  ratatui con `Marker::Braille` (2×4 puntos por celda: dos puntos por
  columna, unidos con rectas) entre −1 y 1, con el cero tenue; en ASCII,
  un `*` por columna.
- **Tests**: `viz/scope.rs` (diez cuadros de un seno de periodo no entero,
  cada uno con lo último desplazado otra cantidad: empiezan en la misma
  fase a menos de una muestra, y subiendo por el cero; sin cruces, lo
  último y «sin disparo») y `tui/src/tests/viz.rs` (un seno de 220 Hz en
  bucle: el scope del master dispara, cinco cuadros seguidos con la misma
  fase, Braille en pantalla y `*` en ASCII).

### Parte 4: `spectrum`

- **`viz/spectrum.rs`**: FFT compleja radix-2 en el sitio, propia (Rust
  puro, sin dependencias: `fft`), de `SIZE` = 2048 puntos con ventana de
  Hann sobre lo último de la fuente; las bandas, `bands` en escala
  logarítmica de 30 Hz a 20 kHz (o al Nyquist; `edges`); cada bin va a una
  sola banda (la de su frecuencia central) y una banda más estrecha que un
  bin (las graves, de 23,4 Hz a 48 kHz) toma el más cercano a su centro;
  cada banda, lo más fuerte de sus bins en dB, de −90 (vacía) a 0 (llena).
  `fall`: sube de golpe y baja `FALL` = 0,025 por cuadro (algo más de un
  segundo de arriba abajo a 30 cuadros), para que no parpadee. Barras de
  octavos de celda (`▁▂▃▄▅▆▇█`), cada banda en las columnas que le tocan;
  en ASCII, `#`.
- **En la interfaz**: el análisis y la caída se hacen en `App::tick_viz`
  (un cuadro por `tick`), en `App::viz_spectra`; la vista solo los copia.
- **Tests**: `viz/spectrum.rs` (un seno de 80, 440, 1000 y 5000 Hz tiene
  su pico en la banda que lo contiene, fuerte, y lejos se queda bajo; un
  impulso da un espectro plano; la caída) y `tui/src/tests/viz.rs` (el
  spectrum del master de un seno de 220 Hz: el pico en su banda y barras en
  pantalla).

### Parte 5: la API, la ayuda y la guía

- **Lua** (`script/src/viz.rs`): `pista:punchcard`, `pianoroll`, `scope`,
  `spectrum`, `pitchwheel` y `spiral`, cada una con `{ size, bands }`
  (`api::VIZ_PARAMS`; `size` de 1 a 40 filas, 6 por defecto; `bands` de 4 a
  128, 32), y `pista:viz_off()`; `master()` (receptor nuevo del registro,
  `Receiver::Master`) con las de audio y su `viz_off()`; y `viz_off()`
  global. Cada llamada apunta un `VizDef { kind, track (None: el master),
  size, bands }` en `Song::viz` (la misma de la misma fuente se
  sustituye); las de rejilla en el master, error con la línea. Si el script
  falla, el modelo de antes (y sus visualizaciones) sigue.
- **El lote**: `script::viz_flags` dice qué fuentes copian (las pistas con
  una de audio, por su hueco del motor, y el master); `song_batch` lo
  manda si hay alguna o las había (para apagarlas). Las de rejilla no
  encienden nada.
- **La interfaz**: `App::sync_viz`, tras cada carga buena, pone las del
  modelo (la cola ya la enciende el lote de la carga). El autocompletado y
  la ayuda de la tabla de opciones (`api::VIZ_TABLE`) salen del registro;
  la guía, generada, tiene cada función.
- **Tests**: `script/tests/viz.rs` (lo que llega al modelo, sustituir,
  `viz_off` de una pista, del master y de todo, las fuentes que enciende el
  lote, errores con línea y el modelo de antes sigue), `registry.rs` (las
  plantillas, también en `master()`) y `tui/src/tests/viz.rs` (desde el
  script: las tres en la vista y el scope dispara con la cola que encendió
  la carga).

### Parte 6: `pitchwheel`, `spiral`, ASCII y refresco

- **`viz/wheel.rs`**: `chroma` es un cromagrama (FFT de `CHROMA_SIZE` =
  8192 puntos con Hann, 5,9 Hz por bin a 48 kHz, para separar las notas
  graves: cada bin de 55 Hz a 5 kHz suma su energía a su clase de nota,
  normalizado a la más fuerte). `pitchwheel`: una rueda en Braille con do
  arriba y un radio por nota tan largo como suena; la más fuerte, en el
  color de acento y nombrada. `spiral`: la onda enrollada en una espiral de
  `TURNS` = 6 vueltas cuyo periodo es el de la nota que se oye (YIN,
  `engine::detect_mono`, sobre las últimas 4096 muestras; sin nota, 1024
  muestras), con el radio empujado por la muestra; el título dice la nota.
  En ASCII, la rueda es una línea de notas con `#` y la espiral, `*`.
- **ASCII**: `theme::ascii()` (por hilo, como el tema); `[interfaz] ascii`
  en `config.toml` y, si no lo dice, al arrancar se mira `LC_ALL`,
  `LC_CTYPE` o `LANG`: sin UTF-8, ASCII (`config::utf8_locale`). Lo usan las
  seis visualizaciones.
- **Refresco**: `[interfaz] refresco` (cuadros por segundo, de 5 a 120; 30
  por defecto). `run_terminal` dibuja al ritmo del refresco o en cuanto
  llega una tecla o el ratón, y espera la entrada como mucho hasta el
  cuadro siguiente: la entrada no espera al dibujo, y con MIDI (que mira su
  cola cada 2 ms) ya no se dibuja en cada vuelta.
- **Coste medido** (`cargo test -p tui --release
  coste_de_las_visualizaciones -- --ignored --nocapture`, calcular y
  dibujar un cuadro de 140×45 en el i7, restado lo demás): `punchcard`,
  `pianoroll` y `scope`, menos de 0,05 ms; `spectrum`, 0,27 ms (0,8 % de un
  núcleo a 30 cuadros); `spiral`, 0,37 ms (1,1 %); `pitchwheel`, 1,04 ms
  (3,1 %, por la FFT de 8192). En el hilo de audio (`cargo test -p engine
  --release --test viz -- --ignored --nocapture`): las nueve colas
  encendidas, +0,025 % de un núcleo (vaciado incluido). Memoria: 576 KB de
  colas en el motor y 64 KB de historial por fuente encendida en la
  interfaz.
- **Tests**: `viz/wheel.rs` (un la, un do, un la grave, un mi grave y un
  sol agudo encienden su clase; el silencio, nada; la espiral de 220 Hz
  vuelve cada 218 muestras y dice A3), `tui/src/tests/viz.rs` (las dos del
  master de un seno de 220 Hz: la rueda en el la, la espiral con A3, en
  pantalla, y en ASCII sin Braille) y `config.rs` (`[interfaz]` y sus
  avisos).

### Parte 7: junto al código y en la ventana del editor

- **Junto al código**: `{ inline = true }` (registro `api::VIZ_PARAMS`)
  apunta en `VizDef::inline` la fila del texto donde se pidió (la de la
  llamada en curso, `written::current_row`, el mecanismo de posiciones del
  resaltado en vivo). La vista lleva cada visualización con su sitio
  (`viz::Placed { line, view }`): las de `inline` se dibujan encima del
  editor, a la derecha de su fila si está a la vista (`ui::draw_inline_viz`:
  dos columnas detrás del final de esa línea si caben 16; si no, pegadas a
  la derecha, de `INLINE_WIDTH` = 40 columnas como mucho), y no en los
  paneles apilados. El texto del código no se toca.
- **En la ventana del editor** (protocolo 7): la principal dibuja cada
  visualización en un buffer aparte (`viz::to_text`: las de junto al
  código en 40 columnas y las demás en 60, sin colores) y le manda sus
  filas, con su fila del texto o ninguna (`ToEditor::Viz`, mensaje
  `visual`, solo cuando cambia); la ventana las pone como
  `VizView::Text` (en el color de acento): las de `inline` junto a su
  línea en su editor y las demás abajo, hasta la mitad de la ventana. Así
  no viajan muestras por el socket, solo unas decenas de líneas de texto
  por cuadro.
- **Tests**: `link.rs` (el mensaje, con Braille, comillas y barras, y
  vacío, sobrevive al viaje), `script/tests/viz.rs` (`inline` apunta su
  fila; un valor que no es booleano, error) y `tui/src/tests/viz.rs`
  (junto a su línea sin taparla y la del master en su panel; en la ventana
  del editor, las dos que dibuja la principal, cada una en su sitio).
  `pantalla_viz_inline` la enseña.

### Parte 8: documentación

«Ver lo que suena» en las recetas de la guía: un tutorial en tres pasos
(el espectro del master con un barrido de filtro, el scope y la rueda de
notas de un bajo del sh101, el punchcard de la batería con un scope junto
a su línea) y cómo quitarlas, el ASCII y el refresco; una línea en «Qué se
puede hacer». Recetas `scripts/ejemplos/24-viz-espectro.lua`,
`25-viz-scope-bajo.lua` y `26-viz-punchcard.lua`, iguales a sus bloques
(`script/tests/guide.rs`, ahora 26), y snippets `viz-espectro` y
`viz-pista`.

### Sin probar en una terminal de verdad

Que el Braille y los bloques se vean bien (y la degradación a ASCII) en la
terminal del usuario, por SSH y en tmux, y que 30 cuadros por segundo no
pesen por la red.

## Guía y tutorial al día

Repaso de `docs/GUIA.md` y `docs/PRIMERA-CANCION.md` frente al código. La
guía ya tenía una receta por novedad y la API generada del registro; lo que
faltaba eran los límites y algunos enlaces:

- **Sintetizadores**: el bloque sub vale también en `sh101`; los límites
  de cada uno (rangos de `sub_div`, `div` y `rdiv`, hasta 8 voces del
  `subharm`, `sh101` solo monofónico, qué no admite curvas) y una nota de
  que el MPE está a medias, sin probar con un Seaboard.
- **Cortes**: para transponer un golpe de batería sin granos, remuestrear
  la pista y volver a cortarla.
- **Lanes**: «Quién manda» (una curva por parámetro; el `~` de un paso,
  las llaves de un golpe, `{lp=...}` y un mando de MIDI learn frente a la
  lane) y los límites: 32 lanes de 64 puntos, solo se editan con la tabla
  literal en la llamada, `lane()` suelta no se edita, y el touch con el
  ratón no se oye hasta escribirse.
- **Lo demás**: «El ratón, de un vistazo» en la sección 2 (editor, vista
  del sample, piano roll y lanes; el tracker, la ocupación y el mezclador
  se añadieron después, ver «El ratón en el tracker, la ocupación y el
  mezclador») y los
  límites de grupos (4, de un nivel), macros (8) y visualizaciones (`size`,
  `bands`, la cola que descarta si la interfaz se atrasa).
- **Tutorial**: el bajo del paso 11 es un `sh101` con una lane de cutoff
  en bucle de 2 compases (ya no hace falta `bajo.wav`: se quitó del paso 1
  y de `script/tests/guide.rs`), y el paso 8 añade la caja del break
  tocada como melodía con `cortes:instrument`. Los scripts enteros, las
  secciones (la intro silencia la caja) y los enlaces de ida y vuelta
  siguen la canción; el análisis de choques del paso 11 sigue dando
  primero `drums × perc` con los samples de prueba (comprobado con
  `--analyze-masking`), y la mezcla final no satura más que antes (render
  offline).

### Sin probar de oído

El bajo `sh101` con su lane y la caja afinada del tutorial: el equilibrio
con la batería lo decide el usuario.

## El ratón en el tracker, la ocupación y el mezclador

- **Geometría única** (`tui/src/ui.rs`): `TrackerLayout`,
  `OccupancyLayout` y `MixerLayout` calculan lo que antes calculaba cada
  `render` (columnas, pistas que se ven, primer paso por el
  desplazamiento, ventana de pasos y de pistas de la ocupación, tiras del
  mezclador) y tienen su `hit(x, y)` (`TrackerHit`, `OccupancyHit`,
  `MixerHit`). El dibujo los usa para pintar y `ui::draw` los guarda en
  `App::panels` (`app/panels.rs`, `PanelAreas`): lo que se ve y lo que
  pulsa el ratón no pueden separarse. Lo que va encima (visualizaciones,
  sidechain, lanes) se apunta en `covers` y ahí el clic no llega abajo.
- **Qué hace** (`App::panels_mouse`, tras el piano roll y las lanes y
  antes de la vista del sample; en los dos modos, sin nada encima, y solo
  pulsar y la rueda: arrastrar y soltar siguen siendo del editor):
  - tracker: el nombre elige la pista y el clic derecho le pone o le
    quita el mute en vivo (`toggle_live_at`, el de `m`); una celda abre
    ahí el cursor de edición y otro clic en la del cursor hace lo de Enter
    (`start_step_edit`); la rueda mueve el cursor;
  - ocupación: lo mismo con su cursor para rellenar (`start_fill` en el
    segundo clic), la rueda de pista en pista y la horizontal (o Shift y
    la rueda) de paso en paso;
  - mezclador: la tira elige la pista y su `M` y su `S` son mute y solo en
    vivo.
  Nada suena ni escribe en el código por un clic: lo que escribe es la
  línea de órdenes del segundo, como con las teclas.
- **Tests**: `tui/src/tests/panels_mouse.rs` (cada clic, en la posición
  de lo dibujado; la paleta abierta lo apaga).

### Sin probar en una terminal de verdad

Con el ratón de la terminal del usuario y dentro de tmux.

## Rediseño de la interfaz

Diseño aprobado en [docs/DISENO-TUI.md](DISENO-TUI.md) (qué se toma de
la referencia y qué no, maquetas y plan). La referencia tiene una
licencia copyleft: de ella solo se toman ideas de disposición y de
interacción; no se ha mirado ni copiado su código, sus textos ni sus
assets.

- **Rejilla pura** (`tui/src/layout.rs`): `layout::grid(pantalla, needs)`
  da el rectángulo de cada panel (menús, cabecera, regla, editor, onda,
  tracker, mezclador, efectos, scope, espectro, referencia, dock, mensaje
  y teclas) sin dibujar ni mirar la aplicación. Cuatro tamaños
  (`Size::of`): diminuto (menos de 40×10), pequeño (menos de 100×30),
  mediano y ancho (desde 160×45). `stack` reparte el alto entre la onda,
  el tracker (sus 16 filas si caben) y el dock, que crece hasta lo que
  pide su pestaña (`Needs::dock_want`) apretando antes la onda (hasta 6) y
  luego el tracker (hasta 7). La misma rejilla sirve con el editor dentro
  o fuera (`Needs::editor`).
- **La ventana** (`tui/src/screen.rs`): dos pasadas de la rejilla (la
  segunda con el alto que pide la pestaña que se va a ver, `grid_of`);
  cada panel sigue siendo su widget de `ui`. `Header` (transporte, error,
  avisos y el master a la derecha), `Ruler` (16 pasos y la línea de
  tiempo), `MessageLine`, `KeysLine` (el dueño de las teclas en mayúsculas
  y sus teclas) y `Reference` (las teclas una por fila, en el ancho). El
  panel «motor» desaparece: el resumen de la pista elegida
  (`ui::track_summary`) va en el título del tracker, lo congelado en el
  título del sample y en la cabecera del tracker, y la variante, la
  grabación y el master en la cabecera. Un solo tracker
  (`FullTrackerPanel`, todas las pistas que quepan), con `▲n ▼n` abajo
  si se desplaza.
- **Pestañas** (`layout::Tab`, `Dock`): lo que no tiene sitio propio,
  nunca encima de otro panel (se acabaron los `covers`). `App::show_tab`
  elige la de la vista que se abre (ocupación, sidechain, choques, lanes,
  las primeras visualizaciones de panel del script); `screen::shown_tab`
  enseña la del panel con el foco si está ahí, si no la elegida, si no la
  primera. Los nombres largos se acortan empezando por los más largos
  (`Dock::labels`) y la fila se desplaza hasta la elegida (`‹` `›`).
- **Foco** (`tui/src/focus.rs`, `app/focus.rs`): `focus::derive` lo saca
  del modo y de los cursores que ya había (tracker, piano roll,
  ocupación, punto de lane); solo la mezcla, los efectos, el scope y el
  espectro se apuntan aparte (`chosen_focus`). `Action::Focus(panel)`
  (Alt-1…9, `foco_*` en `[teclas]`, menú Ver) abre el cursor de ese panel
  como la paleta; Esc vuelve. Con el foco en un panel sin cursor, sus
  teclas (`handle_chosen_focus`) y las letras que no usa no llegan al
  editor; Ctrl, Alt y F siguen. `ui::panel_block` pinta el borde en
  `borde_foco` o `borde` (colores nuevos de `[colores]`, con `cabecera`).
- **Menús** (`tui/src/menu.rs`, `app/menu.rs`): el menú de cada acción
  sale de `menu_of`, un `match` sin comodín (una acción nueva no compila
  sin su menú), con el nombre de la paleta y la tecla de `[teclas]`.
  `MenuBar`, `Dropdown` y `hit` comparten `bar_layout` y
  `dropdown_area`: lo que se ve y lo que pulsa el ratón no se separan.
  F10 (`menu` en `[teclas]`) los abre; sin fila propia (pequeña), la
  barra tapa la cabecera mientras está abierta. Ayuda › «Qué sabe hacer
  esta terminal».
- **Scope y espectro del master** (`app/viz.rs`; apagados por defecto
  desde «Visualizaciones apagadas por defecto», en «Consumo de la
  interfaz»): `sync_ui_viz` pide al motor la fuente del master junto con
  lo del script, solo mientras el último cuadro dibujó el scope o el
  espectro del master (un lote con
  `set_viz`, que entra al compás; tras cada carga, el lote del script
  solo trae lo suyo y se vuelve a pedir). El motor ya copiaba a esa cola:
  sin cambios en el hilo de audio ni publicación nueva (`no_alloc.rs` y
  `golden.rs` igual). `master_views` arma el scope (con el RMS de lo
  último en el título) y el espectro (con su caída), que se calcula al
  dibujar (ver «Consumo de la interfaz»).
- **Ratón**: `menu_mouse` antes que nada; luego `focus_mouse` (una
  pestaña se elige; un clic en la mezcla, los efectos, el scope o el
  espectro les da el foco; el primer clic en el editor sin el foco solo
  se lo devuelve y no mueve el cursor); luego lo de siempre.
- **Degradación**: los menús sin fila por debajo del tamaño mediano, la
  regla desde 20 filas, el mensaje desde 16; con `ascii`,
  `screen::to_ascii` pasa al final del cuadro el dibujo de cajas, los
  bloques y los símbolos a ASCII.
- **Dibujo a prueba de fallos** (`tui/src/safe_draw.rs`): un pánico de un
  widget se recoge con `catch_unwind` (su aviso no se escribe en la
  terminal: el gancho lo guarda mientras se dibuja) y ese cuadro sale con
  una pantalla mínima; un error de la terminal se salta el cuadro. El
  bucle, la entrada y el motor siguen.
- **Ventana del editor** (`ui::draw_editor_window`): con alto, menús,
  cabecera con lo que suena en la principal, mensaje y teclas. El
  protocolo pasa a la versión 8: `estado` lleva la variante, la
  grabación y el master en dB enteros, en campos que una principal de
  antes puede no mandar.
- **Tests**: `layout::tests` (sin solapes ni salidas en doce tamaños, con
  y sin editor y piano roll; los tamaños; el dock que crece; los nombres
  de las pestañas), `tests/screen.rs` (cada disposición, la diminuta,
  ASCII, un barrido de 1×1 a 300×90 sin fallos, el master sin pedirlo y
  tras una recarga, el tracker desplazado, las pestañas que se
  desplazan), `tests/menu.rs`, `tests/focus.rs`, `safe_draw::tests` y la
  ventana del editor en `tests/split.rs`. Los tests de antes que miraban
  posiciones fijas se pasaron a la disposición nueva; los del ratón
  siguen con la posición de lo dibujado.

### Decisiones

- F1 sigue siendo la paleta; los menús van en F10 (libre). Alt y un
  número para el foco porque el editor usa Alt con letras, flechas y
  signos; Tab no, que es del autocompletado, las plantillas y los
  cursores.
- Las pestañas en una fila propia encima de su panel (no dentro del
  borde): así cada panel conserva su título.
- RMS en vez de LUFS: la cola del master es mono y sin ponderar; un
  número que se llama LUFS sin serlo engañaría.
- En el modo tocar, el primer clic en el editor solo cambia el foco: así
  no se edita sin querer mientras se toca.
- Los grupos no tienen tira en el mezclador (siguen en el panel de
  efectos).

### Sin probar en una terminal de verdad

Todo se ha visto con el backend de test de ratatui: falta cambiar el
tamaño en vivo, Alt y un número dentro de tmux y en la terminal del
usuario, el ratón en cada panel y los menús, `LANG=C` y una terminal sin
truecolor, y el editor en su ventana con los menús.

## Consumo de la interfaz

Una sola sección para todo lo del consumo de la interfaz, hecho en dos
máquinas y unido después: las cifras de cada tabla dicen si son de
**x86_64** (la laptop de desarrollo, i7) o de **ARM64** (el MacBook M2 con
Asahi); no se comparan tal cual entre ellas (ver «Cómo leer los %» en
ARM64). Orden: la primera tanda (x86_64), la medición y los arreglos en
ARM64, las visualizaciones del master apagadas por defecto (x86_64), y
la unión de los dos trabajos con la tabla final (x86_64).

### Primera tanda (x86_64)

Tras el rediseño, el hilo de la interfaz gastaba mucho más que antes: en
el portátil ARM64 llegaba a un núcleo entero. Medido en la laptop de
desarrollo (x86_64, release, 200×50, `top -H`; el hilo de audio, con la
salida nula, igual antes y después: ~7 % sonando, ~1 % parado):

| Escenario | Antes | Después |
|---|---|---|
| sin MIDI, sonando, ratón quieto | 14,6 % | 10,7 % |
| sin MIDI, sonando, ratón moviéndose | 45,3 % | 11,5 % |
| sin MIDI, parado, ratón quieto | 14,6 % | 1,0 % |
| sin MIDI, parado, ratón moviéndose | 41,6 % | 2,0 % |
| `--allow-midi`, sonando, ratón quieto | 36,1 % | 12,2 % |
| `--allow-midi`, sonando, ratón moviéndose | 44,8 % | 12,5 % |
| `--allow-midi`, parado, ratón quieto | 37,0 % | 2,7 % |
| `--allow-midi`, parado, ratón moviéndose | 46,0 % | 3,0 % |

Cuatro cambios, todos en el hilo de la interfaz (sin tocar el motor):

- **El espectro, una vez por cuadro** (`App::frame_viz`, al empezar
  `screen::draw`): antes la FFT del master y las de los espectros del
  script iban en cada `tick`, que es cada vuelta del bucle (con MIDI la
  espera baja a 2 ms: ~300 FFT por segundo para ~26 cuadros), y copiaban
  las 16 384 muestras del historial a un `Vec` nuevo. Ahora el `tick`
  solo vacía las colas; la FFT mira las últimas 2048 muestras del
  historial sin copiarlo, y la caída de las barras es por cuadro de
  verdad. El master solo se pide al motor mientras se ve su scope o su
  espectro (`master_shown`, de los paneles del último cuadro); al
  apagarse, su historial se olvida.
- **El ratón que solo se mueve no es entrada** (`only_motion` en
  `main.rs`): la captura del ratón pide a la terminal todos los
  movimientos (hace falta para arrastrar), y cada uno forzaba un dibujo
  completo fuera del ritmo de cuadros.
- **La FFT con todo calculado una vez** (`spectrum::Analyzer`: ventana,
  tabla de giros en f64, búferes y bordes de las bandas): de ~190 µs a
  ~53 µs por espectro.
- **En reposo no se redibuja lo mismo** (`app/idle.rs`): parado, sin
  entrada y sin nada que suene, el bucle se salta los cuadros mientras la
  huella de lo que se ve (`idle_key`: medidores como se dibujan, compás,
  paso, tempo, mensaje, error, cargas y mensajes MIDI) no cambie. La
  marcha, las notas tenidas, las grabaciones y los avisos de saturación
  la anulan; la ventana del editor siempre dibuja. Un cuadro cada 500 ms
  igualmente (`IDLE_REFRESH`), por si cambia algo que no está en la
  huella.

### Decisiones

- El master se apaga cuando no se ve, aunque al volver a verlo el lote
  entre al compás (hasta un compás con «esperando audio del master»): así
  lo pidió el usuario, y el motor no copia para nada.
- En reposo, un cuadro cada medio segundo en vez de ninguno: lo que no
  entre en la huella se ve como mucho medio segundo tarde, por menos de
  un 1 % de CPU.
- Sigue pendiente: la caída de los medidores es por `tick`, no por
  cuadro, así que con MIDI bajan más deprisa.

### Medición y arreglos en ARM64 (Asahi)

Medido en un MacBook Pro M2 (13 pulgadas, 2022) con Asahi Linux (Arch
Linux ARM, núcleo 7.2), release, sin `--allow-audio` (salida nula). El
emulador de la sesión es kitty 0.49, pero la medición no pasa por él:
`scripts/medir-consumo.py` arranca la interfaz en un pseudoterminal de
tamaño fijo, cuenta lo que escribe (lo que kitty tendría que pintar) y
lee la CPU de cada hilo de `/proc` durante 10 s por escenario. La
canción es `scripts/consumo/cancion.lua` (el break sintético y un bajo
`sh101`); «visual», `scripts/consumo/visual.lua` (además, scope y
espectro por pista). Al cargar, la canción ya suena: «parado» es tras
Ctrl-Espacio. El ratón, 60 movimientos por segundo sin botón.

**Cómo leer los %**: es tiempo de un núcleo, y en el M2 depende mucho
de dónde y a qué frecuencia corre. Con poca carga, el planificador deja
los dos hilos en los núcleos de eficiencia (el 98-100 % del tiempo, ver
la última columna) y schedutil los baja hacia 600 MHz: el mismo trabajo
cuenta unas cuatro o cinco veces más que en un núcleo de rendimiento a
3,2 GHz. Por eso el hilo de audio marca ~7 % sonando cuando el mismo
render offline, a toda máquina, gasta ~1 % (90 s de audio en 0,9 s), y
por eso las cifras no se comparan tal cual con las de x86_64. Para
comparar trabajo, el banco `tests::consumo` (abajo) y el tiempo del
render offline.

Las cifras en vivo cambian de una tanda a otra con el mismo binario (a
240×70 sonando, de 5 % a 10 % según lo que haga el resto del sistema):
para comparar dos binarios, `--bin antes,ahora --repeticiones 3` los
alterna y da la mediana, y aun así solo se distinguen diferencias de
más del 20 %. El banco es la medida fina.

**Antes** (HEAD `e8a50c8`; ui, el hilo principal; audio, el de la
salida nula; ningún otro hilo pasa del 0,1 %):

| Escenario | Tamaño | ui | audio | kB/s a la terminal | cuadros/s |
|---|---|---|---|---|---|
| parado | 80×24 | 0,3 % | 0,4 % | 0,1 | 1,9 |
| sonando | 80×24 | 3,0 % | 7,0 % | 6,3 | 28,4 |
| visual | 80×24 | 4,7 % | 7,0 % | 16,1 | 27,9 |
| sonando, ratón | 80×24 | 3,3 % | 6,9 % | 6,3 | 28,6 |
| parado, ratón | 80×24 | 0,5 % | 0,3 % | 0,1 | 2,0 |
| parado | 120×40 | 0,7 % | 0,4 % | 0,4 | 2,7 |
| sonando | 120×40 | 6,2 % | 7,0 % | 18,5 | 27,5 |
| visual | 120×40 | 7,9 % | 7,2 % | 27,7 | 27,0 |
| sonando, ratón | 120×40 | 6,2 % | 6,9 % | 18,3 | 27,6 |
| parado, ratón | 120×40 | 0,7 % | 0,4 % | 0,2 | 2,0 |
| parado | 240×70 | 1,2 % | 0,4 % | 0,4 | 2,8 |
| sonando | 240×70 | 10,1 % | 6,7 % | 37,2 | 27,1 |
| visual | 240×70 | 14,5 % | 7,1 % | 82,9 | 25,6 |
| sonando, ratón | 240×70 | 10,6 % | 7,0 % | 37,3 | 26,6 |
| parado, ratón | 240×70 | 1,0 % | 0,3 % | 0,2 | 1,9 |
| `--allow-midi`, parado | 80×24 | 0,7 % | 0,4 % | 0,1 | 2,0 |
| `--allow-midi`, sonando | 80×24 | 3,3 % | 7,2 % | 6,4 | 28,7 |
| `--allow-midi`, sonando, ratón | 80×24 | 3,3 % | 7,1 % | 6,4 | 28,8 |
| `--allow-midi`, parado | 240×70 | 1,6 % | 0,3 % | 0,2 | 2,9 |
| `--allow-midi`, sonando | 240×70 | 10,8 % | 7,1 % | 37,6 | 26,6 |
| `--allow-midi`, sonando, ratón | 240×70 | 10,7 % | 7,1 % | 37,9 | 26,5 |

Lo que ya hacía bien la optimización de x86_64: parado no se redibuja
(2-3 cuadros por segundo, el de seguridad y poco más), el ratón que
solo se mueve no cuesta nada y el MIDI no dispara la interfaz. Lo que
cuesta es sonar: ~27 cuadros por segundo, y el hilo de la interfaz
crece con el área (de 3 % a 80×24 a 10 % a 240×70).

**Banco del cuadro** (`cargo test --release -p tui consumo -- --ignored
--nocapture`, `tui/src/tests/consumo.rs`): el bucle de `run_terminal`
sin terminal (el motor offline al ritmo de los cuadros, `tick` y
`Terminal::draw` con el backend de crossterm sobre un sumidero que
cuenta bytes), en un núcleo de rendimiento. µs por cuadro, antes:

| Canción | Estado | Tamaño | widgets | diff y escritura | total | bytes por cuadro |
|---|---|---|---|---|---|---|
| cancion | parado | 80×24 | 74 | 23 | 97 | 25 |
| cancion | sonando | 80×24 | 75 | 22 | 98 | 216 |
| cancion | sonando | 120×40 | 180 | 54 | 236 | 631 |
| cancion | sonando | 200×50 | 229 | 103 | 333 | 1185 |
| cancion | parado | 240×70 | 215 | 152 | 367 | 25 |
| cancion | sonando | 240×70 | 288 | 163 | 452 | 1383 |
| visual | sonando | 80×24 | 162 | 25 | 189 | 584 |
| visual | sonando | 240×70 | 545 | 190 | 739 | 3342 |

El reparto de los widgets a 240×70 sonando (medido aparte con un
cronómetro por panel, sin dejarlo en el código): `App::view` 56, el
editor 52, el tracker 37, la onda 25, los efectos 23, el espectro del
master (`frame_viz`) 21 y lo demás (cabecera, regla, pestañas, teclas)
~75. Ningún panel se lleva la mayor parte: es dibujar todas las celdas
en cada cuadro. Con `CONSUMO_DIFF=1` el banco cuenta las celdas que
cambian: a 240×70 sonando, ~290 por cuadro (tracker 122, espectro 69,
scope 64, onda 18, mezclador 14), a unos 5 bytes por celda; casi todo es
movimiento de verdad, no repintado de más.

**El audio** (render offline de 128 compases, 180 s, en un núcleo de
rendimiento): el break solo, 0,26 s; el `sh101` solo, 1,61 s (con
`quality = 1`, 1,02 s). La voz analógica es casi todo el hilo de audio
de esta canción: ~190 ns por frame, con el oscilador, el sub y el filtro
al doble y el diezmado.

**Causas arregladas** (una por commit; cada optimización del motor,
comprobada con `scripts/comparar-audio.sh` contra el binario de antes:
las 26 canciones deterministas de `scripts/ejemplos/` y
`scripts/consumo/`, idénticas bit a bit; `no_alloc.rs` en verde):

- **Los coeficientes del diezmado, copiados en cada diezmador**
  (`synth::osc::Halfband`): `output` leía el `LazyLock` de `HALFBAND`
  dos veces por frame y voz, y cada lectura es una carga atómica con
  adquisición (`ldar` en ARM64, que en x86_64 es un `mov` sin más). El
  `sh101` solo, 128 compases: de 1,61 s a 1,41 s en un núcleo de
  rendimiento (−12 %), de 3,00 s a 2,94 s en uno de eficiencia (−2 %).
  En vivo no se ve (el hilo de audio va en los de eficiencia: 7,0 % →
  6,8-7,0 %, dentro del ruido); ayuda cuando una canción pesada sube el
  hilo a los de rendimiento. La otra tabla con `LazyLock`, la de la
  interpolación sinc, se lee una vez por frame entre 16 vecinos o más:
  ahí no compensa.
- **Los avisos de la notación, en caché** (`App::diagnostics`):
  `diagnose::steps_diagnostics` releía en cada cuadro todas las cadenas
  de `steps` y `melody` del texto para los avisos del pie del editor.
  Ahora se guardan con la huella del texto y de las etiquetas conocidas,
  y solo se recalculan si cambian. Con la canción corta eran ~18 µs por
  cuadro; con `scripts/consumo/larga.lua` (118 líneas, ocho pistas, como
  las canciones del usuario), ~110 µs, la cosa más cara de `App::view`.
  Banco, µs por cuadro sonando: `larga` a 80×24 de 214 a 105 (−51 %), a
  240×70 de 646 a 533 (−17 %); `cancion` a 80×24 de 97 a 80. En vivo
  (mediana de tres, alternando los binarios): `larga` a 80×24, el hilo
  de la interfaz de 3,5 % a 2,7 %; a 240×70 no se distingue del ruido
  (9,3 % y 9,2 %).
- **El scope y el RMS del master, sin copiar el historial**
  (`App::master_views`, `viz::scope::build`): en cada cuadro con el
  master a la vista se copiaban sus 16 384 muestras a un `Vec`, se
  buscaba el disparo en la copia y se sumaban en cadena los cuadrados de
  un tercio de segundo (cada suma espera a la anterior: Rust no
  reordena la coma flotante). Ahora el disparo recorre las dos mitades
  del `VecDeque` y la suma de cuadrados va en ocho sumas parciales, que
  el compilador vectoriza (NEON en ARM64); lo mismo los scopes por pista.
  El RMS que se lee puede cambiar en la última décima respecto a antes
  (otro orden de suma, más exacto); el audio no se toca. `App::view` a
  240×70 sonando, de ~38 a ~22 µs; el banco, `cancion` 240×70 de 435 a
  419 µs y 120×40 de 220 a 200, `visual` 240×70 de 643 a 618 y 80×24 de
  169 a 158. En vivo, por debajo del ruido.
- **La biblioteca de bloques, releída solo si cambia**
  (`blocks::Library::refresh`): cada 500 ms, `tick` releía y analizaba
  todos los bloques de las dos carpetas, también parado (por si cambian
  fuera: otro editor, git). Ahora mira el nombre, la fecha y el tamaño de
  cada `.lua` y solo relee si la huella cambia. La medición usa una
  configuración vacía, así que no se veía: con `--bloques 300` (300
  bloques en la biblioteca global), parado a 80×24, el hilo de la
  interfaz de 1,3 % a 0,4 % (el de sin bloques es 0,2-0,3 %); sonando,
  de 3,6 % a 2,6 %.

**Modo de bajo consumo** (`[interfaz] bajo_consumo = true`, apagado por
defecto): con todo lo anterior, sonando sigue costando dibujar la
pantalla entera 30 veces por segundo (el número de dB del master, con su
décima, cambia en cada cuadro: sonando no hay cuadros iguales que
saltarse, ver `CONSUMO_DIFF` en el banco). El modo:

- dibuja como mucho a 15 cuadros por segundo (`LOW_POWER_REFRESH`; un
  `refresco` más bajo se respeta);
- recalcula el scope, los espectros y las demás visualizaciones uno de
  cada dos cuadros (`App::frame_viz` y `App::frame_viz_views`, con lo
  del último en `viz_cache`);
- dibuja la onda en barras y el scope y las ruedas sin Braille
  (`theme::set_low_power`, `theme::plain_viz`);
- y dejaba el espectro del master sin sitio fijo, como una pestaña más
  (`layout::Needs::low_power`). Tras la unión con «Visualizaciones
  apagadas por defecto» el scope y el espectro del master son siempre
  pestañas y van a su ritmo (`visuales_hz`): el modo ya no toca la
  rejilla y baja ese ritmo (ver «La unión de los dos trabajos»).

La entrada no espera más (cada tecla sigue dibujando en el acto) y el
audio no cambia. Medido: banco, lo que cuesta un segundo de interfaz
sonando a 240×70, de 12,5 a 5,7 ms (`cancion`) y de 18,4 a 7,7 ms
(`visual`); en vivo (mediana de 3), a 240×70 los bytes a la terminal de
38 a 14 kB/s (`cancion`) y de 88 a 23 kB/s (`visual`), y el hilo de la
interfaz de 6,0 % a 4,7 % y de 7,5 % a 4,5 %; a 80×24, de 2,0 % a 1,1 %.

Cuándo conviene: con batería, con la terminal grande o con las
visualizaciones abiertas, o si el emulador de terminal gasta (kitty y
los demás pintan cada cuadro: a la mitad de bytes, la mitad de trabajo
para ellos). No conviene para mirar el scope o el espectro con detalle
(van a 7,5 cuadros por segundo) ni para la onda en puntos.

**Después** (todo lo anterior, `bajo_consumo` apagado; la misma
medición que la de antes, una tanda de 10 s por escenario):

| Escenario | Tamaño | ui antes | ui después | audio antes | audio después | kB/s |
|---|---|---|---|---|---|---|
| parado | 80×24 | 0,3 % | 0,3 % | 0,4 % | 0,3 % | 0,1 |
| sonando | 80×24 | 3,0 % | 2,5 % | 7,0 % | 6,8 % | 6,4 |
| visual | 80×24 | 4,7 % | 4,2 % | 7,0 % | 7,0 % | 16,0 |
| sonando, ratón | 80×24 | 3,3 % | 2,7 % | 6,9 % | 7,0 % | 6,4 |
| parado, ratón | 80×24 | 0,5 % | 0,3 % | 0,3 % | 0,3 % | 0,1 |
| parado | 120×40 | 0,7 % | 0,7 % | 0,4 % | 0,4 % | 0,4 |
| sonando | 120×40 | 6,2 % | 5,3 % | 7,0 % | 6,9 % | 18,5 |
| visual | 120×40 | 7,9 % | 7,0 % | 7,2 % | 7,1 % | 27,7 |
| sonando, ratón | 120×40 | 6,2 % | 5,5 % | 6,9 % | 7,0 % | 18,5 |
| parado, ratón | 120×40 | 0,7 % | 0,6 % | 0,4 % | 0,3 % | 0,2 |
| parado | 240×70 | 1,2 % | 1,2 % | 0,4 % | 0,3 % | 0,4 |
| sonando | 240×70 | 10,1 % | 10,1 % | 6,7 % | 7,1 % | 37,6 |
| visual | 240×70 | 14,5 % | 14,2 % | 7,1 % | 7,2 % | 83,5 |
| sonando, ratón | 240×70 | 10,6 % | 10,0 % | 7,0 % | 7,1 % | 37,6 |
| parado, ratón | 240×70 | 1,0 % | 0,9 % | 0,3 % | 0,3 % | 0,2 |
| `--allow-midi`, parado | 80×24 | 0,7 % | 0,6 % | 0,4 % | 0,3 % | 0,1 |
| `--allow-midi`, sonando | 80×24 | 3,3 % | 2,8 % | 7,2 % | 7,0 % | 6,3 |
| `--allow-midi`, sonando, ratón | 80×24 | 3,3 % | 2,8 % | 7,1 % | 7,0 % | 6,4 |
| `--allow-midi`, parado | 240×70 | 1,6 % | 1,5 % | 0,3 % | 0,3 % | 0,2 |
| `--allow-midi`, sonando | 240×70 | 10,8 % | 10,4 % | 7,1 % | 7,0 % | 37,9 |
| `--allow-midi`, sonando, ratón | 240×70 | 10,7 % | 10,5 % | 7,1 % | 7,0 % | 38,1 |

Y alternando los dos binarios (mediana de 3), con la canción larga:

| Escenario | Tamaño | ui antes | ui después |
|---|---|---|---|
| sonando | 80×24 | 3,0 % | 2,5 % |
| visual | 80×24 | 4,9 % | 4,2 % |
| larga | 80×24 | 5,6 % | 3,3 % |
| sonando | 240×70 | 10,7 % | 9,9 % |
| visual | 240×70 | 14,6 % | 14,0 % |
| larga | 240×70 | 14,1 % | 12,1 % |

Banco, µs por cuadro (núcleo de rendimiento), antes → después:
`cancion` sonando 80×24 97 → 79, 120×40 236 → 198, 240×70 452 → 415;
`visual` 80×24 189 → 157, 240×70 739 → 611; `larga` 80×24 214 → 104,
240×70 646 → 504. Con `bajo_consumo`, un segundo de interfaz a 240×70:
`cancion` 12,4 ms, `visual` 7,7 ms (sin él, 12,4 y 18,3).

**Comparado con x86_64**: allí, sonando a 200×50, el hilo de la interfaz
quedó en ~11 % y el de audio en ~7 % (otra canción). En el M2, a 240×70,
~10 % y ~7 %: lo mismo en porcentaje, pero en núcleos de eficiencia a
baja frecuencia, que es donde el planificador los deja; en trabajo, el
M2 hace un cuadro de 240×70 en ~0,4 ms en un núcleo de rendimiento. El
ARM64 no tiene un coste propio grande: lo único que salió más caro que
en x86_64 fue la carga con barrera del `LazyLock` del diezmado (arriba).

**Lo que se probó y no ayudó** (no entra):

- `lto = "fat"` y `codegen-units = 1`: el audio igual (1,61 s el `sh101`
  solo) y la interfaz ~5 % menos en el banco, a cambio de un minuto más
  por cada build de release. No compensa en el portátil de desarrollo.
- `-C target-cpu=native`: lo mismo bit a bit y en tiempo (1,61 s y
  1,62 s). Las voces son recurrencias (filtros, osciladores, envolventes)
  que no se pueden vectorizar sin cambiar los bits; lo vectorizable
  (copias, mezclas, rellenos) ya va en NEON con el objetivo genérico.
- `panic = "abort"`: no se prueba, rompería el dibujo a prueba de fallos
  (`catch_unwind`).
- Saltarse los cuadros iguales sonando (como en reposo): no hay; el
  número de dB del master cambia en cada cuadro. Cambiarlo es cambiar lo
  que se ve: para eso está `bajo_consumo`.
- Quitar la consulta del `LazyLock` de la interpolación sinc: una por
  frame entre 16 vecinos o más, por debajo de lo que se puede medir.

**Lo que sigue caro**:

- Dibujar la pantalla entera en cada cuadro sonando. El diff de ratatui
  (calcula el ancho Unicode de cada celda, la compara y vacía el búfer)
  cuesta ~160 µs a 240×70, tenga o no cambios, y no se toca sin cambiar
  ratatui. Los widgets: el editor (ratatui-textarea) ~50-70 µs, el
  tracker ~37 µs (~77 con ocho pistas), la onda ~25, los efectos ~23-39,
  el espectro del master ~21. Todo escala con el área y con los cuadros
  por segundo: el remedio es `bajo_consumo` o una terminal más pequeña.
- La voz analógica (`sh101`), ~0,8 % de un núcleo de rendimiento por voz
  con `quality = 2` (el de siempre); con `quality = 1`, un 37 % menos.
  Con varias voces o `subharm{ voices = 6 }` (la receta de MPE: 2,7 %),
  es lo que más pesa del audio.
- Lo que no se mide aquí: el emulador de terminal (kitty pinta cada
  cuadro: a 240×70 sonando, 38 kB/s; con las visualizaciones, 84 kB/s) y
  la salida de audio real (cpal con PipeWire; sus xruns no se pueden ver
  con la salida nula).

### Visualizaciones apagadas por defecto (x86_64)

Las mediciones de `docs/BASE-LIGERA.md` (rama `base-ligera`) dicen que el
salto de consumo del rediseño llega con `634f8eb` (el scope y el espectro
del master siempre a la vista): la interfaz pasaba de ~7,8 % a ~13,4 % y
lo escrito a la terminal sonando, de ~12,6 a ~31 KiB/s. Los arreglos de
arriba bajaron el reposo, pero sonando quedaba ~9-10 %.

**Cómo se mide** (`scripts/medir-consumo.py`): el binario de release en
una pseudo-terminal de 200×50 o de 80×24, con una configuración vacía, la
misma canción (un break sintético de `engine --example synth_break` en 8
cortes, `steps("1 . . 1  . . 3 .  1 . 5 .  . 3 . 6")` a 170 BPM, que
arranca al abrir) y **sin `--allow-audio`**: el motor avanza con la
salida nula a ritmo real. Con audio de verdad no se ha medido: Claude
Code no abre nunca la tarjeta (ver `CLAUDE.md`); el hilo de la interfaz
no depende de la salida, pero falta comprobarlo con `top -H` en una
terminal real. Se mide el hilo de la interfaz (el principal) sonando 20 s,
luego F8 y parado 20 s, y los bytes por segundo que escribe a la
terminal. La variación entre repeticiones es de ±0,7 puntos. (Esta tabla
se hizo con la primera versión del script, con el tamaño y las teclas
como argumentos sueltos; tras la unión son `--tamanos` y `--teclas`, en
la versión del ARM64, y el escenario `espectro` manda Alt-M.)

| Commit | Tamaño | Visualizaciones | Interfaz sonando | Interfaz parado | Terminal sonando |
|---|---|---|---:|---:|---:|
| `8fb818b` (antes del rediseño) | 200×50 | — | 7,8 % | 7,5 % | 12,6 KiB/s |
| `main` (`e8a50c8`), antes | 200×50 | siempre | 9,1 % | 0,9 % | 34,6 KiB/s |
| `main`, antes | 80×24 | pestañas sin elegir | 3,6 % | 0,5 % | 5,3 KiB/s |
| apagadas por defecto | 200×50 | apagadas | 7,4 % | 0,7 % | 13,7 KiB/s |
| apagadas por defecto | 80×24 | apagadas | 3,5 % | 0,4 % | 5,3 KiB/s |
| apagadas por defecto | 200×50 | espectro (Alt-M) | 9,1 % | 0,9 % | 28,9 KiB/s |
| apagadas por defecto | 200×50 | scope (Alt-7) | 9,4 % | 0,9 % | 34,3 KiB/s |
| apagadas por defecto | 80×24 | espectro (Alt-M) | 5,3 % | 0,5 % | 12,2 KiB/s |
| `visuales_hz = 15` | 200×50 | espectro (Alt-M) | 8,7 % | 0,8 % | 21,3 KiB/s |
| `visuales_hz = 15` | 200×50 | scope (Alt-7) | 9,0 % | 0,9 % | 24,4 KiB/s |
| `visuales_hz = 15` | 80×24 | espectro (Alt-M) | 4,3 % | 0,6 % | 7,3 KiB/s |

Dos cambios, solo en la interfaz (el motor, `no_alloc.rs` y `golden.rs`
sin tocar):

- **Apagadas hasta que se piden** (`App::master_viz`, `toggle_master_viz`;
  `Needs::master_viz` solo si se han pedido): sin pedirlas no hay pestañas
  ni paneles, así que `master_shown` es falso, `sync_ui_viz` no manda
  `set_viz`, el motor no copia el master y `frame_viz` no hace la FFT. Se
  piden con «Scope y espectro del master» (Alt-M, `visuales` en
  `[teclas]`, menú Ver y paleta) o con su foco (Alt-7, Alt-8), y son dos
  pestañas más del dock (`layout::master_tabs`) en todos los tamaños; el
  alto lo piden con `screen::tab_height` (7 el scope, 9 el espectro, para
  que quepa el espectrograma). Ya no hay paneles fijos del master en el
  mediano ni en el ancho: la columna derecha del ancho es el mezclador y
  la referencia. Tests: `the_master_scope_and_spectrum_are_off_until_asked_and_need_nothing_in_the_script`
  (apagado, ni un lote ni una FFT aunque suene; desde el menú Ver, un
  lote y FFT; Alt-M los quita, otro lote que apaga la cola y ni una FFT
  más), `the_master_is_only_copied_while_its_panels_are_seen` (con otra
  pestaña elegida, ni FFT) y los de la rejilla. `App::viz_cost` (solo en
  los tests) da los lotes `set_viz` de la interfaz, las fuentes que copia
  el motor y las FFT (`Analyzer::runs`).
- **Su propio refresco** (`[interfaz] visuales_hz`, 15 por defecto, de 1
  a 60): `tick_viz` cuenta las muestras del master que llegan
  (`viz_pending`) y, cuando son las de un cuadro (`sample_rate /
  visuales_hz`), rehace el scope (`master_cache`, del que lo saca
  `master_views`) y marca que toca la FFT (`viz_fresh`, que gasta
  `frame_viz`). Los cuadros de en medio dibujan lo mismo y ratatui no
  escribe nada de esos paneles. Parado y en silencio (el historial a
  cero, el scope ya plano y el espectro caído), no se rehace nada, para
  que el reposo siga en reposo. Tests:
  `the_master_scope_and_spectrum_follow_their_own_rate` (el mismo audio a
  15 y a 60: 12 cuadros frente a 48),
  `stopped_and_silent_the_master_spectrum_stops_computing` y
  `the_interface_section`.

Decisiones:

- **Pestañas siempre, nunca paneles fijos**: lo pidió así el usuario, y
  así solo se calcula la que se ve (el scope o el espectro, no los dos).
  Se pierde ver el espectro a la vez que los efectos en el tamaño mediano.
- **Alt-M y no Alt-V**: Alt-V es «una pantalla arriba» en el editor
  (`ratatui-textarea`), y no se cambia ningún atajo. Alt-M (de master) no
  lo usa nadie. El foco del scope o del espectro (Alt-7, Alt-8, el menú
  Ver › Foco) también los enciende: es lo que se espera al pulsarlo.
- **El ritmo en muestras, no en tiempo de reloj**: en vivo es lo mismo
  (llegan a tiempo real) y en los tests es determinista (lo que se
  renderiza). La FFT sigue yendo al dibujar, con la caída por cuadro de
  ellas: a 15 la caída se ve más lenta que a 30.
- **`visuales_hz` solo para el master**: las de Lua «siguen como están»
  (lo pidió el usuario), al refresco general.
- **No se recuerda entre sesiones**: arrancan apagadas siempre. Si se
  quiere, sería un `[interfaz] visuales = true`.
- **Lo que no ayudó**: con el espectro pedido, el CPU sonando baja poco
  (de ~9,1 % a ~8,7 %, cerca del ruido); lo que baja de verdad son los
  bytes a la terminal (un 25-40 %). Lo que queda caro sonando sin
  visualizaciones (~7,4 % a 200×50) es dibujar la ventana entera 30 veces
  por segundo (el tracker, los medidores, el resaltado y la cabecera
  cambian en cada paso); bajarlo pide no rehacer los paneles que no
  cambian, que ratatui no hace por sí solo.

### La unión de los dos trabajos

Los dos trabajos se hicieron a la vez en dos máquinas (las
visualizaciones apagadas en x86_64, en `main`; la medición, los arreglos
y `bajo_consumo` en ARM64, en `origin/main`) y se unieron con un merge
normal. Lo que tocaban los dos y cómo quedó:

- **`scripts/medir-consumo.py`**: la versión del ARM64 (escenarios,
  `--tamanos`, `--bin` con varios binarios alternados y mediana,
  `--repeticiones`, `--config`, `--bloques`, las preguntas de la
  terminal), con lo de la de x86_64: `--teclas` (al abrir, con escapes:
  `'\x1bm'` es Alt-M) y un escenario `espectro` (sonando, con Alt-M).
  `scripts/consumo/*.lua` sin cambios salvo el comentario de `visual.lua`
  (sus visualizaciones son las de Lua; las del master se piden aparte).
- **El scope del master**: la caché a su ritmo de x86_64
  (`master_cache`, rehecha en `tick_viz` cada `sample_rate / visuales_hz`
  muestras), construida como en ARM64, sin copiar el historial
  (`scope::build` sobre el `VecDeque` y el RMS en sumas parciales).
- **`visuales_hz` y `bajo_consumo`, sin lógica repetida**: un solo sitio
  dice a qué ritmo va cada cosa, `InterfaceConfig::frame_rate` (el
  refresco general: `refresco`, o como mucho 15 con `bajo_consumo`) y
  `InterfaceConfig::viz_rate` (el del scope y el espectro del master:
  `visuales_hz`, o con `bajo_consumo` como mucho la mitad del refresco,
  7). `App::set_config` los copia en `refresh_hz` y `viz_hz`, y nada más
  mira `bajo_consumo` para eso. Cada visualización tiene un solo freno:
  las del master, el reloj de muestras (`tick_viz`/`frame_viz`, sin el
  «uno de cada dos» del ARM64, que las habría frenado dos veces); las del
  script, el «uno de cada dos cuadros» de `bajo_consumo` (`viz_turn`,
  `frame_viz_views`, con `viz_cache` solo para ellas), como en ARM64. Lo
  demás del modo (la onda en barras, el scope y las ruedas sin Braille)
  sigue igual.
- **La rejilla**: la de x86_64. El scope y el espectro del master son
  siempre pestañas, así que `Needs::low_power` (que quitaba el espectro
  fijo en bajo consumo) sobraba y se quitó con su test
  (`low_power_puts_the_master_spectrum_in_a_tab`), y `ViewModel::low_power`
  con él.
- **`[interfaz]`**: `refresco`, `visuales_hz` y `bajo_consumo`, cada uno
  con su nombre, en una sola prueba (`the_interface_section`, con
  `viz_rate`).
- **Los tests**: se quedan todos (miden cosas distintas): los de
  x86_64 (apagado sin `SetViz` ni FFT, el menú Ver, el ritmo propio, el
  silencio), el de ARM64 de las visualizaciones del script en bajo
  consumo, y el de ritmo ahora también con `bajo_consumo` (de 60 baja a
  7: 5 o 6 cuadros en 0,8 s).
- **El motor**: el cambio del diezmado (`synth::osc::Halfband`) entra:
  `golden.rs` igual y `scripts/comparar-audio.sh` contra el binario de
  antes de la unión, 27 canciones iguales bit a bit (8 compases; las dos
  con azar sin semilla no se comparan).

**Tabla final** (x86_64, la laptop de desarrollo; `scripts/medir-consumo.py
--bin x86,arm,union --repeticiones 3 --segundos 10`, mediana de 3, sin
`--allow-audio`; hilo de la interfaz y kB/s a la terminal). «x86» es
`main` antes de la unión (visualizaciones apagadas, `visuales_hz`);
«arm», `origin/main` (arreglos de ARM64 y `bajo_consumo`, el master
siempre a la vista); «unión», el resultado. En «espectro», Alt-M en «arm»
no hace nada: en 80×24 su espectro es una pestaña sin elegir y no se
dibuja, así que no se compara con los otros dos. El hilo de audio, ~7,7 %
sonando en los tres.

| Escenario | Tamaño | x86 | arm | unión |
|---|---|---|---|---|
| parado | 80×24 | 0,5 % · 0,1 | 0,5 % · 0,1 | 0,5 % · 0,1 |
| sonando | 80×24 | 3,9 % · 6,4 | 3,5 % · 6,3 | 3,5 % · 6,3 |
| visual (Lua) | 80×24 | 6,3 % · 15,7 | 5,6 % · 15,8 | 5,7 % · 15,7 |
| espectro (Alt-M) | 80×24 | 4,7 % · 7,9 | (3,5 % · 6,3) | 4,0 % · 7,9 |
| parado | 200×50 | 0,9 % · 0,3 | 1,2 % · 0,4 | 0,8 % · 0,3 |
| sonando | 200×50 | 8,2 % · 14,0 | 9,0 % · 32,5 | 7,8 % · 13,8 |
| visual (Lua) | 200×50 | 11,4 % · 44,7 | 12,2 % · 62,0 | 10,9 % · 44,9 |
| espectro (Alt-M) | 200×50 | 9,0 % · 21,9 | 9,1 % · 32,7 | 8,7 % · 22,0 |

Con `bajo_consumo = true` (`--config`), «arm» frente a «unión» (en
«arm», bajo consumo pone el espectro del master en una pestaña sin
elegir: «espectro» no lo dibuja):

| Escenario | Tamaño | arm | unión |
|---|---|---|---|
| parado | 80×24 | 0,5 % · 0,1 | 0,5 % · 0,1 |
| sonando | 80×24 | 2,0 % · 5,5 | 1,9 % · 5,5 |
| visual (Lua) | 80×24 | 2,5 % · 3,9 | 2,6 % · 3,9 |
| espectro (Alt-M) | 80×24 | (1,9 % · 5,5) | 2,2 % · 4,8 |
| parado | 200×50 | 2,5 % · 0,9 | 0,9 % · 0,3 |
| sonando | 200×50 | 4,5 % · 12,8 | 4,2 % · 11,3 |
| visual (Lua) | 200×50 | 5,4 % · 17,2 | 5,2 % · 15,7 |
| espectro (Alt-M) | 200×50 | (4,6 % · 12,8) | 4,6 % · 15,0 |

Lectura: la unión no es peor que lo mejor de cada lado en ningún caso
comparable (las diferencias de menos de medio punto son ruido). Sonando
a 200×50 gana lo de x86_64 (el master apagado: de 32,5 a 13,8 kB/s) y a
80×24 lo de ARM64 (los avisos en caché y el scope sin copia: 3,9 % →
3,5 %). Con `bajo_consumo`, parado a 200×50 la unión gasta menos que
«arm» (0,9 % frente a 2,5 %, y 2 cuadros por segundo frente a 6,8): en
«arm» el espectro del master seguía calculándose en reposo.

Decisiones de la unión:

- **`bajo_consumo` baja `visuales_hz` en vez de añadir otro freno**: así
  cada cosa tiene un solo ritmo y se lee en un sitio. La mitad del
  refresco es lo que hacía el «uno de cada dos cuadros» del ARM64.
- **Las de Lua, con el «uno de cada dos» del ARM64 en bajo consumo**: no
  tienen un reloj de muestras propio (las de rejilla no tienen audio) y
  el freno por cuadros ya existía y estaba probado.
- **Sin `Needs::low_power`**: la rejilla ya no depende del modo; un
  ajuste menos que mantener.

### Las visualizaciones en otra ventana (sin hacer)

Se pidió como opcional, si encajaba en el mecanismo de la ventana del
editor (`--split`, protocolo 8). No encaja sin más: la ventana del editor
es el editor (recibe las visualizaciones del script por el protocolo 7,
pero solo como añadido al código), y una ventana solo de visualizaciones
es un proceso y una conexión nuevos, como la del piano roll
(`--window-roll`, protocolo 6) o la de los dispositivos
(`--window-devices`). Diseño, por si se hace:

- `tui --window-viz <script>` abre una terminal nueva
  (`[ventanas] terminal`, como las otras) con un socket propio junto al
  de la principal; un mensaje nuevo en `link.rs` (protocolo 9) con el
  scope y el espectro del master ya dibujados en texto (como `viz_frames`
  para el editor) a `visuales_hz`, y de vuelta solo «cerrar» y «otro
  dibujo del espectro».
- La principal sigue calculando (el motor está en ella): lo que se ahorra
  es el dibujo en la principal, no la FFT; con la ventana abierta, el
  master cuenta como visto (`master_shown`).
- Coste estimado: unas 500-700 líneas (la ventana y su bucle ~200, el
  protocolo y sus tests ~250, la acción, la paleta, el menú Ventanas y la
  guía ~150), en la línea de la ventana del piano roll.

## Explorador y editor abajo

Continuación del rediseño de la interfaz, pedida por el usuario con un
editor modal y su distribución para programar como referencia de estilo
(solo ideas; nada de su código, configuración ni textos). Diseño,
maquetas a 80×24, 120×40 y 180×50 y decisiones en la sección 12 de
[docs/DISENO-TUI.md](DISENO-TUI.md). No toca el motor: `no_alloc.rs` y
`golden.rs` no cambian.

- **Rejilla** (`tui/src/layout.rs`): `Needs` dice si se quiere el
  explorador, su ancho, si va encima (`explorer_overlay`), dónde va el
  editor (`EditorPlace`: `Docked`, `Maximized`, `Floating(Option<Rect>)`)
  y su alto. `grid` corta a la izquierda el explorador (desde
  `EXPLORER_SCREEN`, 120 columnas), abajo el editor (`editor_rows`: un
  tercio por defecto, mínimo `EDITOR_MIN`, dejando `SOUND_MIN` al
  sonido) y coloca el sonido en lo que queda con `small`, `medium` o
  `wide`, que bajan de tamaño si el alto no les llega (`MEDIUM_SOUND`,
  `WIDE_SOUND`). El flotante, `float_rect`, siempre dentro del cuerpo y
  de al menos `FLOAT_MIN`. Lo que va encima (el flotante y el explorador
  abierto a mano) sale en `Grid::overlays`, y `Grid::covered` dice si un
  punto lo tapa algo: el ratón no llega a lo de debajo. El espectro del
  master tiene sitio fijo en el mediano (junto a las pestañas) y en el
  ancho (la columna de la derecha); en el pequeño, pestaña. Los efectos
  del ancho pasan a la primera pestaña.
- **Explorador de archivos** (`tui/src/files.rs`, puro; el panel en
  `tui/src/files_panel.rs` con `FilesLayout::hit`; la aplicación en
  `tui/src/app/files.rs`): `files::Tree` lee solo la carpeta que se abre
  (las primeras `MAX_ENTRIES`, 500) y enseña carpetas, `.lua`,
  `.sampler` y `.wav`. Con el foco (`Panel::Files`, Alt-0), flechas o
  `hjkl`, Enter abre (`open_script` para un `.lua`, la orden `abrir`
  para un proyecto, la preescucha para un sample), `a` escribe su
  `sample()`, `n`/`r`/`Supr` crean, renombran y borran archivos (con
  confirmación), `<`/`>` el ancho, `R` vuelve a leer. Alt-E lo esconde o,
  en una terminal estrecha, lo abre encima. La preescucha es la del
  explorador de samples, ahora `App::preview_file` con una caché
  compartida (`preview_cache`).
- **Editor abajo, maximizado y flotante** (`tui/src/app/dock.rs`): el
  borde de arriba se arrastra (`EdgeDrag::EditorTop`; sin moverlo, es un
  clic que da el foco), Ctrl-Alt-↑↓ cambian el alto (`resize_editor`),
  Alt-Z maximiza (`toggle_maximized`), Alt-W lo hace flotar
  (`toggle_float`, vuelve a `last_float`), su título lo mueve y sus bordes
  de abajo y de la derecha cambian su tamaño (`EdgeDrag::FloatMove` y
  `FloatResize`), Alt-Shift-flechas lo mueven y Ctrl-Alt-←→ cambian el
  ancho del flotante o del explorador (`widen`). `screen::draw` dibuja el
  flotante al final (con `Clear`) y el explorador de encima antes que
  él; en `App::mouse`, sobre el flotante solo llega al editor (el primer
  clic sin el foco solo se lo da; arrastrar y soltar siguen llegando a lo
  que se arrastraba). Lo que se mueve se guarda con
  `config::save_keys` en `[interfaz]` (`alto_editor`,
  `ancho_explorador`, `explorador`), sin tocar el resto del archivo.
- **Onda con puntos** (`ui::wave_dots`, `ui::braille`): dos columnas y
  cuatro filas de puntos por celda, de lo más bajo a lo más alto de cada
  medio carácter; las marcas de los cortes, la columna izquierda de
  puntos de su celda. La usan la onda de la pista, la grande y la vista
  previa del explorador (`ui::draw_waveform`). `[interfaz] onda =
  "barras"` (`theme::set_wave_bars`) o `ascii`: las barras de antes.
- **Espectro del master** (`tui/src/viz/master.rs`): `MasterSpectrumPanel`
  dibuja `relleno`, `linea`, `barras` o `espectrograma`
  (`[interfaz] espectro`, `viz::master::Look`) con el eje de frecuencias
  logarítmico (`axis_marks`). Las bandas (ahora 64) se interpolan al ancho
  (`level_at`); el espectrograma guarda los últimos
  `SPECTROGRAM_FRAMES` cuadros reutilizando los `Vec` y tramado 2×4 para
  la densidad. Con menos de tres filas, el espectrograma se dibuja
  relleno. El análisis no cambia: una FFT por cuadro dibujado, y el
  master solo se pide mientras el panel o el scope se ven. En reposo
  (`app/idle.rs`) se dibuja hasta que el espectro ha caído; el
  espectrograma se queda quieto.
- **Menús** (`tui/src/menu.rs`): `submenu_of` agrupa las acciones en
  submenús y `rows` arma las filas (la lista plana de `items` sigue para
  el test de que toda acción está en un menú). `MenuState::sub`, el
  submenú abierto; `sub_area`, dónde va (a la derecha o, si no cabe, a la
  izquierda); `hit` devuelve también `MenuHit::Sub`. Ver › Dibujo tiene
  «Onda: puntos o barras» y «Espectro: otro dibujo» para la sesión.
- **Teclas** (`config::BINDABLE`, todas en `[teclas]`): `foco_archivos`
  (Alt-0), `explorador_archivos` (Alt-E), `editor_mas_alto` y
  `editor_menos_alto` (Ctrl-Alt-↑↓), `mas_ancho` y `menos_ancho`
  (Ctrl-Alt-←→), `maximizar_editor` (Alt-Z), `editor_flotante` (Alt-W) y
  `flotante_arriba`… (Alt-Shift-flechas). `KeyCombo` lee flechas
  (`ctrl-alt-up`), siempre con Ctrl o Alt. En el editor, Ctrl-Alt-flechas
  iban al principio o al final del texto o de la línea (lo anota la
  guía; sigue con Ctrl-Inicio/Fin, Alt-</> e Inicio/Fin).
- **Tests**: la rejilla por tamaños con el explorador visible, oculto y
  encima y el editor acoplado, maximizado y flotante (`layout.rs`); el
  árbol (`files.rs`: perezoso, una carpeta de 520 archivos, refrescar,
  nombres); `tests/files.rs` (dibujo plegado y desplegado, abrir un
  `.lua` y los cambios sin guardar, un sample, el ratón y el borde, Alt-E
  y la terminal estrecha, crear, renombrar y borrar); `tests/dock.rs`
  (alto con teclas y borde, maximizar, la configuración, flechas en
  `[teclas]`); `tests/float.rs` (mover, redimensionar, teclas y que el
  ratón no llegue a lo tapado); los puntos de la onda y del espectro
  (`tests.rs`, `viz/master.rs`, `tests/screen.rs`); los submenús
  (`menu.rs`, `tests/menu.rs`); el foco del explorador
  (`tests/focus.rs`); y que ningún tamaño rompe el dibujo con el
  explorador encima, flotante o maximizado, en ASCII y no. Las pantallas:
  `pantalla_tamanos` y `pantalla_flotante`.

### Consumo

Medido en la laptop de desarrollo (x86_64, `--release`, salida nula sin
abrir la tarjeta, `scripts/ejemplos/08-varias-pistas.lua` con el break
sintético, el ratón quieto): el % de CPU del hilo de la interfaz, leído de
`/proc/<pid>/task/<pid>/stat` durante 10 s, en un pseudo-terminal del
tamaño dado. «Antes» es el binario de justo antes de este trabajo
(`2685d92`), con la misma canción; dos tandas, se dan las dos cifras.

| Escenario | Antes | Ahora |
|---|---|---|
| 200×50, sonando | 10,3–10,6 % | 10,1–10,2 % |
| 200×50, parado | 1,0–1,1 % | 1,0 % |
| 200×50 sin lo nuevo (`explorador = false`, onda y espectro en barras), sonando / parado | — | 9,5–10,2 % / 0,6–1,0 % |
| 200×50, espectrograma, sonando / parado | — | 10,8 % / 1,2 % |
| 200×50, editor flotante, sonando / parado | — | 11,6 % / 1,1 % |
| 120×40, sonando | 5,9–6,6 % | 7,6–8,0 % |
| 120×40, parado | 0,7 % | 0,8 % |

- **Los puntos no se notan**: con la onda y el espectro en barras y sin
  explorador sale lo mismo dentro del ruido de la medida (en una de las
  tandas, dos medidas salieron a la mitad, ~5 %, sin causa clara: se
  descartan como ruido). Por eso los puntos son lo de siempre, y
  `onda = "barras"` y `espectro = "barras"` quedan para el ARM64 si allí se
  notan.
- **A 120×40 sube algo** (de ~6 % a ~8 %) porque el espectro del master
  tiene ahora sitio fijo: antes era una pestaña que no se veía, y sin
  verse el master no se pedía ni se hacía su FFT. A la terminal salen
  20 KiB/s en vez de 12.
- **Parado, el reposo se mantiene** (~1 %): el espectro se sigue
  dibujando solo hasta que ha caído del todo. El espectrograma se
  desplazaba en reposo (11 % parado): ahora se queda quieto.

### Decisiones

Ver la sección 12.2 de [docs/DISENO-TUI.md](DISENO-TUI.md). Las de más
peso: el explorador solo en su columna desde 120 columnas (así es lo
primero que se va); solo se borran archivos, nunca carpetas ni el script
del editor; no se vigila el disco; lo maximizado y la posición del
flotante no se guardan; los puntos por defecto (no cuestan más que las
barras).

### Fuera, por ahora

- El popup de una tecla líder con las acciones agrupadas («which-key»):
  no se ha hecho; reutilizaría `menu::rows`.
- Carpetas desde el árbol y vigilar el disco.
- Abrir un `.lua` desde la principal con el editor en su ventana.

### Sin probar en una terminal de verdad

Todo se ha visto con el backend de test de ratatui y con el binario en un
pseudo-terminal (salida nula): falta en el Asahi ARM64 el consumo sonando
y parado, arrastrar los bordes del editor y del explorador y el flotante
con el ratón de verdad, Alt-E/Alt-0/Alt-W/Alt-Z y Ctrl-Alt-flechas dentro
de tmux y en su terminal, los puntos Braille en su fuente, el espectro
con un barrido de cutoff, y la terminal pequeña y `LANG=C`.

## Límites del script

Hallazgo A1 de `docs/AUDITORIA.md`: un script que reservaba memoria sin
parar tumbaba el programa entero (con el audio). No había límite de
memoria, y el de tiempo solo se miraba cada 10 000 instrucciones: una
instrucción cara (`string.rep`, concatenar) hace mucho trabajo entre dos
comprobaciones. Medido en la auditoría: `while true do t[#t+1] =
string.rep("x", 1e7) .. #t end` llegó a 6 GB (el tope puesto con
`RLIMIT_AS`) y abortó el proceso a los 62,8 s.

Ahora una carga tiene dos límites, y pasado cualquiera de los dos el
script se corta con su error y sigue sonando lo anterior
(`ScriptHost::load` no toca el motor hasta que la evaluación termina bien;
el intérprete es nuevo en cada carga, así que lo que reservó se suelta al
fallar y la carga siguiente empieza de cero):

- **Memoria** (`script/src/lib.rs`, `ScriptHost::set_memory_limit`;
  `DEFAULT_MEMORY_LIMIT`, **256 MB**): el asignador del intérprete de mlua
  (`Lua::set_memory_limit`) falla en cuanto una reserva pasaría del límite
  y Lua lo convierte en un error (`Error::MemoryError`, también dentro de
  una función de Rust: `is_memory_error` lo busca dentro de
  `CallbackError`). Cuenta todo lo que vive en el intérprete: cadenas,
  tablas, cierres y lo que crean las funciones de Rust con la API de Lua
  (las tablas de `scale`, las cadenas de los generadores). **No** cuenta lo
  que guarda Rust: los samples (`sample()`), los patrones y los modelos;
  eso tiene sus propios topes (8 pistas, 256 compases, 64 lanes...).
- **Tiempo** (`set_time_limit`; `DEFAULT_TIME_LIMIT`, 2 s): el hook se
  dispara ahora cada **1000** instrucciones (`HOOK_INSTRUCTIONS`); además
  de mirar el reloj, apunta la línea en curso (para el mensaje) y el pico
  de memoria (`ScriptHost::memory_peak`).

Los dos se configuran en `config.toml`, `[script] memoria` (MB, de 16 a
4096) y `[script] tiempo` (segundos, de 0,5 a 60) (`config::ScriptConfig`,
que los pone a la interfaz en `App::set_config` y a las órdenes sin
interfaz, como el render, `--analyze-masking` y `--script`, en
`main::script_host`). Los mensajes llevan la línea si el hook llegó a verla
y el ajuste que hay que cambiar: `cancion.lua:14: el script usa más de 256
MB de memoria (el límite es [script] memoria)` y `cancion.lua:2: el script
tarda más de 2.0 s (¿bucle infinito?; el límite es [script] tiempo)`.

**Medidas** (x86_64, release):

- **Lo que usan los scripts de verdad**
  (`cargo test --release -p script --test guide medida_de_los_limites --
  --ignored --nocapture`): los 162 de la guía, del tutorial, de
  `scripts/ejemplos/` y de `scripts/consumo/`, mediana 33 KB y máximo 37
  KB de pico (casi todo, las funciones de la API ya cargadas), en 4 ms de
  mediana y 30 ms como mucho. 256 MB son unas 7000 veces el mayor: deja
  sitio a un script que genere patrones a lo grande sin acercarse a lo
  que una máquina de 8-16 GB nota. Con 256 MB, el script de la auditoría
  se corta en 0,85 s con 275 MB de RSS en todo el proceso (antes, 62,8 s
  y 6 GB, y el proceso abortaba).
- **Lo que cuesta comprobar el tiempo**
  (`cargo test --release -p script --test lua medida_del_hook --
  --ignored --nocapture`; un bucle de 30 millones de vueltas que solo
  hace cuentas, lo peor para el hook, y otro de 10 millones de llamadas):

  | Hook | Cuentas | Llamadas |
  |---|---|---|
  | sin hook | 484 ms | 215 ms |
  | cada 10 000 instrucciones (antes) | +70 % | +172 % |
  | cada 1000 (ahora) | +78 % | +181 % |
  | cada 100 | +106 % | +239 % |
  | cada 1000 y en cada llamada | +76 % | +730 % |

  Casi todo el coste es tener un hook (con uno puesto, Lua 5.4 pasa cada
  instrucción por el trazado), y ya estaba. Bajar de 10 000 a 1000 añade
  8 o 9 puntos más sobre el bucle sin hook, en scripts que pasan casi todo su
  tiempo en Rust (la carga normal tarda milisegundos). Con 1000, una
  instrucción cara (`s:upper()` de 100 KB, en debug) se corta antes de los
  0,5 s del test. A 100 cuesta un tercio más sin ganar nada que se note.

Decisiones:

- **256 MB por defecto**: dentro de lo propuesto (128-512), con mucho
  margen sobre lo medido y lejos de lo que pone en peligro la máquina.
- **1000 instrucciones** y no menos: lo que se gana en precisión no
  compensa el coste en los scripts con cuentas.
- **Sin hook en las llamadas** para saber la línea siempre: multiplica por
  ocho el coste de un script con muchas llamadas. La consecuencia: si el
  script se queda sin memoria antes de las primeras 1000 instrucciones
  (por ejemplo, unas pocas reservas de decenas de MB), el error sale sin
  línea. En Lua 5.4 el manejador de errores no se llama en los errores de
  memoria, así que no hay otra forma barata de saberla.
- **Un solo límite por carga**, no por función: el intérprete es nuevo en
  cada carga.

Lo que no se cortaba (una función de C que tarda mucho, como un patrón de
`string.find` con vuelta atrás; un `pcall` que se traga el error del
límite; un `__gc`) se arregló después, con la segunda auditoría: ver N4 en
«Arreglos de la segunda auditoría».

Tests: `runaway_scripts_hit_the_limits_and_the_previous_keeps_playing`
(`script/tests/lua.rs`: cadenas que se guardan, tablas que crea Rust,
concatenar en un bucle, instrucciones caras y un bucle vacío, cada uno
cortado a tiempo y con su línea; lo que suena es idéntico a no haber
cargado nada, y la carga siguiente entra), `the_default_memory_limit_holds`,
`the_script_section` (`config.rs`) y `the_script_limits_come_from_the_config`
(`tui/src/tests/config.rs`: en la interfaz, con el error en el editor y el
transporte en marcha).

## Guardado atómico

Hallazgo M3 de `docs/AUDITORIA.md`: guardar el script, el proyecto o los
bloques hacía `fs::write` directo sobre el archivo, así que un corte o el
disco lleno a mitad dejaba la canción truncada. Ahora todo lo que escribe
el usuario pasa por un solo sitio, `script::atomic` (`script/src/atomic.rs`):

- `write(ruta, bytes)` y `write_with(ruta, |temporal| ...)` (para lo que
  tiene su propia función de escribir, como los WAV del motor, que no se
  tocan: se les da la ruta del temporal).
- El temporal va **en la misma carpeta** que el destino (el renombrado es
  atómico solo dentro del mismo sistema de archivos), oculto y con un
  nombre que nunca pisa otro: `.<nombre>.<pid>-<n>.sampler-tmp`, creado
  con `create_new`.
- Se llena, se le ponen **los permisos del archivo de antes**, se vuelca al
  disco (`sync_all`) y se renombra sobre el destino; luego se vuelca la
  carpeta (en Unix; si no se puede, no es un error). Si algo falla antes
  del renombrado, se borra el temporal y el archivo de antes no se ha
  tocado.
- **Solo lectura**: un archivo de solo lectura no se sustituye
  (`PermissionDenied`), como hacía `fs::write`: si no, renombrar encima lo
  habría cambiado sin avisar.
- **Enlaces simbólicos**: se siguen (en cadena, hasta 40) y se escribe en
  el destino, con el temporal junto a él; el enlace sigue siendo un
  enlace. Un enlace roto crea su destino. Es lo que espera quien tiene la
  canción enlazada desde otra carpeta (git, Syncthing).
- **Huérfanos**: antes de cada guardado se borran los temporales del mismo
  archivo con más de un minuto (`ORPHAN_AGE`): los de un guardado que no
  terminó porque el programa murió. Los recientes no se tocan (podría ser
  otro guardado en curso de otra instancia).
- **El dueño** del archivo no se conserva (se guarda como el usuario que
  corre el programa, como cualquier editor).

Qué escrituras tenían el problema y ahora van por aquí:

| Escritura | Antes | Ahora |
|---|---|---|
| El script (Ctrl-S, `App::save`) | `fs::write` directo | `atomic::write` |
| La copia `<nombre>.sin-guardar.lua` al recargar del disco | directo | `atomic::write` |
| El proyecto (`Project::save`) | directo | `atomic::write` |
| Los bloques (guardar y actualizar, `blocks::Library`) | directo | `atomic::write` |
| Las ranuras (`ranuras/N.txt`) | directo | `atomic::write` |
| Exportar a WAV (`exportar`, `--render`, `--track`, `remuestrear`) | `write_wav` directo | `atomic::write_with` |
| Congelar una pista (el WAV de 32 bits) | directo | `atomic::write_with` |
| `config.toml` (mandos, la disposición, los dispositivos) | temporal fijo `config.toml.tmp` y renombrar, sin `fsync` ni permisos | `atomic::write` |
| El historial de versiones | temporal `.lua.part` y renombrar, sin `fsync` | `atomic::write` |

Lo que no hacía falta tocar: crear un archivo nuevo vacío en el
explorador (`create_new`, no pisa nada) y renombrarlo (`fs::rename`, ya
atómico). El motor no se ha tocado: `engine::offline::write_wav` sigue
igual y recibe la ruta del temporal. Unos `.lua.part` de un historial que
murió con la versión de antes no se limpian solos (no se leen: el
historial solo lista `.lua`).

Tests: en `script/src/atomic.rs`, el disco que se llena a mitad, el
renombrado denegado, la carpeta de solo lectura de verdad, el archivo de
solo lectura, los permisos, los enlaces (y el roto) y los huérfanos; y
`saving_never_leaves_the_script_half_written` (`tui/src/tests/config.rs`:
Ctrl-S sobre un script de solo lectura avisa y no lo toca; al poder,
guarda sin dejar temporales).

## Arreglos de la segunda auditoría

`docs/AUDITORIA-2.md` (nota C+) encontró 17 hallazgos nuevos (N1-N17).
Se arreglaron por tandas, de menor a mayor riesgo, con un commit por
hallazgo y un test que falla sin el arreglo; la tabla de hallazgos del
informe dice el commit de cada uno. Las reglas de siempre: `golden.rs`
idéntico (las cinco huellas de antes, sin tocar) y `no_alloc.rs` en verde
y ampliado; además, `scripts/comparar-audio.sh` con el binario de antes de
los arreglos: las 27 recetas deterministas, iguales bit a bit.

### Pánicos en la carga (N1, N5)

- **N1**: con secciones sin `arrange()` (o una variante B con una pista
  nueva), una pista tenía ranura pero no estaba en lo que suena, y un
  `unwrap` de `song_batch` mataba el programa. Dos secciones sin
  `arrange()` bastaban. Esa ranura queda sin patrón (es lo que dice el
  aviso «suena solo "…"»). No es un error: el script es válido.
- **N5**: `script::panics::catch` recoge un pánico sin escribir en la
  terminal (un gancho de pánicos que, dentro de `catch`, guarda el
  mensaje) y lo devuelve como texto. Lo usan `ScriptHost::load`, `eval` y
  `set_variant` (un pánico es «error interno del programa (no es de tu
  script)», con la línea del script por la que iba: el gancho corre antes
  de deshacer la pila y mira la de Lua con `catch_at`), `Project::load`,
  la preescucha de un archivo y el hilo de la tonalidad. Lua se crea con
  `catch_rust_panics(false)`: un `pcall` del script no puede tragarse un
  pánico y seguir con el estado roto. El intérprete es nuevo en cada
  carga, así que tras un pánico no queda nada a medias; la caché de
  samples se devuelve igual. Tests: `script/src/panic_tests.rs` (con una
  función `__panico` que solo existe en los tests): el error lleva la
  línea, `pcall` y `xpcall` no lo atrapan, dentro de una sección también,
  y con la canción sonando el compás siguiente suena igual y la carga
  siguiente entra.

### Samples con NaN o infinitos (N2, N3)

- **Decisión**: un sample con muestras no finitas **se carga con esas
  muestras a 0 y un aviso** («x.wav: 3 muestras no válidas (NaN o
  infinito) puestas a 0»), en vez de rechazarlo: un NaN suelto en un
  sample bueno no debería impedir usarlo, y el aviso dice cuántas.
  `Sample::from_interleaved` sanea antes de resamplear: lo no finito a 0,
  lo que pasa de `MAX_SAMPLE_LEVEL` (64, +36 dBFS: un `1e30` desbordaba el
  resampleo) recortado, y los subnormales a 0 (inaudibles y lentos en los
  filtros; no se avisan). `Sample::repairs` cuenta lo arreglado y
  `sample()` lo avisa. Un WAV de 64 bits en coma flotante sigue siendo un
  error de `hound`, sin pánico. Con esto el pánico de `rubato` (N2) no es
  alcanzable.
- **N3, el mecanismo exacto** (medido con renders offline): el NaN llega
  al **filtro de la pista** (un biquad, con memoria), que lo devuelve en
  cada muestra siguiente; la pista toca en cada compás, así que su voz
  nunca acaba y su cadena nunca se vacía (la memoria de los efectos solo
  se vacía con la pista en silencio); su salida NaN, sumada al master, lo
  hace NaN entero, y `soft_limit` convierte NaN en 0: silencio total, para
  siempre. La **reverb** guarda el NaN en sus combs hasta que se duerme
  (un segundo sin entrada): un segundo de silencio. El delay y el
  compresor no lo guardaban. Los medidores no lo veían: `f32::max`
  ignora el NaN.
- **La red** (en el hilo de audio, sin reservas): tras los efectos de
  cada pista y de cada grupo, `scrub` pone a cero lo no finito y, si
  había algo, vacía la memoria de esos efectos y calla esa pista como el
  pánico (`hush_track`, solo ella; el secuenciador sigue). En el master,
  lo que quede (buses, preescucha) también a cero y los buses se vacían
  (`Rack::clear`). Con entradas normales no cambia nada (solo toca lo no
  finito): `golden.rs` igual. `MasterStatus::non_finite` cuenta las
  muestras y la interfaz lo avisa en el mensaje. Coste: un recorrido de
  `is_finite` por bloque y pista, que no se nota en la medida.
- Tests: `engine/tests/bad_samples.rs` (NaN, infinitos y ambos, con y sin
  resampleo, valores enormes, subnormales, un archivo limpio idéntico,
  64 bits), `engine/src/engine/nan_tests.rs` (un sample envenenado que no
  pasa por la carga: con filtro, con reverb, y los medidores) y
  `a_float_wav_with_nan_loads_with_a_warning_and_the_mix_keeps_sounding`
  (`script/tests/import.rs`).

### Liberar memoria en el callback (N6)

`(!lanes.is_empty()).then_some(lanes)` soltaba el `Box<Lanes>` en
`Engine::step` al quitar la última lane. Ahora vuelve por la cola de
basura como las demás tablas. Revisado el resto de lo que el motor suelta:
patrones, arreglos, samples y tablas pendientes ya iban por la cola, y lo
demás que llega por comandos es `Copy`. `no_alloc.rs` tiene un test nuevo
(`removing_and_replacing_live_never_frees_in_the_callback`) que quita y
sustituye en vivo la tabla de lanes (también la última lane), dos tablas
antes del compás, patrones de samples y de sintetizador, un arreglo, los
grupos y la preescucha; con el código de antes falla en «quitar la
última lane».

### El límite de tiempo de Lua (N4) y la memoria de Rust (N7)

- `script/src/sandbox.rs`: `pcall` y `xpcall` son envoltorios que, pasado
  el límite (el hook guarda su mensaje), devuelven ese error en vez de
  atraparlo; `setmetatable` rechaza una metatabla con `__gc` (un
  finalizador corre con los hooks apagados, también al cerrar Lua; en Lua
  5.4 un `__gc` añadido después no cuenta); y `string.find`, `match`,
  `gmatch` y `gsub` rechazan un patrón cuyo coste en el peor caso
  (`longitud^(cuantificadores + 1)`, o `^cuantificadores` anclado) pasa
  de `MAX_PATTERN_COST` (1e12), solo con dos o más cuantificadores (con
  uno, como mucho cuadrático: `s:gsub("%s+", "")` nunca se rechaza).
- **El vigilante** (para lo que el hook no ve): `ScriptHost::eval` corre la
  evaluación en un hilo con una copia de su estado (las cachés de samples
  son `Arc`) y espera. Si pasado el límite y `WATCHDOG_GRACE` (1 s) la
  evaluación sigue en Lua o en C, **se abandona**: la carga falla con su
  error, la interfaz sigue y sigue sonando lo anterior. Matar el hilo no
  es seguro en Rust; se deja acabar solo (tiene el límite de memoria del
  intérprete y acaba con el proceso); como mucho dos abandonados a la vez
  (con más, la carga dice que hay que guardar y reiniciar). El trabajo de
  Rust (cargar, preparar y cortar samples, montar la canción) se marca
  (`Busy`) y no se vigila: con un archivo largo tarda, pero acaba, y antes
  también se esperaba. **Coste medido**: 41 µs por carga para crear el
  hilo (con pila de 16 MB, solo memoria virtual), frente a 650 µs de
  evaluar una canción corta (`medida_del_vigilante`, ignorado).
- Con el binario real: el bucle con `pcall` y el `__tostring` que no
  vuelve se cortan a los 2,0 s; el patrón de la auditoría y el `__gc`, al
  momento; un patrón que el filtro deja pasar pero tarda minutos en C
  (`.-.-x` sobre 10 000 caracteres), a los 3,0 s por el vigilante. En la
  interfaz, con `tui_hostil.py`, sigue respondiendo.
- **N7**: partir un patrón en pasos reserva memoria de Rust, fuera del
  límite del intérprete: `steps(("1 "):rep(2e7))` llegaba a 1,6 GB.
  `parse_pattern_with` y las funciones de patrones rechazan un texto de
  más de `MAX_PATTERN_BYTES` (32 KiB) antes de partirlo: 87 MB (la propia
  cadena y su copia) y 0,15 s. Los demás argumentos (cortes, longitud,
  `every`, `euclid`, `arrange`) ya tenían tope. El error de un paso
  larguísimo enseña sus 40 primeros caracteres.
- **N9**: `dofile` y `loadfile` ya no existen (ni la guía ni las recetas
  los usaban); `print` deja su línea en los avisos de la carga, con dónde
  se llamó, como mucho 20 de 200 caracteres (en vez de escribir encima de
  la pantalla).
- Tests: `the_time_limit_cannot_be_dodged` (ocho rodeos, cada uno en su
  hilo para que el test no se cuelgue), `print_goes_to_the_warnings_and_there_is_no_dofile`
  (`script/tests/lua.rs`), `a_huge_pattern_is_refused_before_splitting_it`
  (`script/tests/patterns.rs`) y el coste de patrones en `sandbox.rs`.

### El resto

- **N8**: los umbrales del detector de transitorios son absolutos. Ahora,
  si salen menos cortes de los pedidos, se detecta otra vez con el sample
  normalizado y se queda la que encuentre más; lo que ya daba sus cortes
  no cambia (por eso no se normaliza siempre). Test: el mismo break de 0
  a −30 dB da los mismos 8 cortes.
- **N13 y M6**: el render de la línea de órdenes se escribe a trozos
  (`engine::offline::render_into`, los mismos bloques que `render`: el
  mismo archivo) y el tope sube de 600 s a dos horas (lo que cabe en un
  WAV de 32 bits en coma flotante sin pasar de 4 GB): 14 minutos, 11 MB de
  pico y 3 s. Formato: `--bits 16|24|32f` y `--dither` (TPDF de un
  escalón, determinista) en la línea de órdenes, `24bits`, `32f` y
  `dither` en `exportar`. Por defecto, 16 bits sin dither, como siempre.
- **N14**: un note off que no cabe en la cola de comandos queda en
  `stuck_offs` y se reintenta en cada `tick`.
- **N16**: `rotate(p, math.mininteger)`, un WAV extensible con «bits
  válidos» enormes (`hound` los acepta) y un ajuste de corte de `1e308`
  desbordaban en depuración; ahora resto, error y saturación.
- **N10**: cuatro huellas nuevas en `golden.rs` (sintetizadores, buses,
  lanes, grupos con macros), sin tocar las cinco de antes; como ellas, se
  comprueban en x86_64 (iguales en depuración y en release).
- **N11**: el test de los límites aguantaba mal una máquina cargada: los
  casos de memoria llevan 30 s de límite de tiempo y los de tiempo un
  margen absoluto de 2 s. Otros tres tests con poco margen (la salida
  nula, el explorador, el patrón enorme), con más.
- **N17**: tests para los dos bordes de `resume` y para `euclid` más largo
  que `n`; cazan los mutantes que antes escapaban.
- **M4**: samples en FLAC, MP3, OGG Vorbis y AIFF con `symphonia` 0.5
  (Rust puro; dependencia nueva, la que ya decía CLAUDE.md), por la
  extensión; los WAV siguen con `hound` igual. En MP3 se quita el retardo
  del códec. El explorador y el autocompletado enseñan esas extensiones
  (`engine::SAMPLE_EXTENSIONS`).
- **B1, B2, B4**: fuera `Grid::scope` y `Grid::spectrum`; los comentarios de
  `song_batch` en su sitio; los sockets de los tests en una ruta corta.

### Sin arreglar

- **N12** (cargar un sample muy largo congela la interfaz: un WAV de 20
  minutos tarda 12 s): arreglarlo es cargar sin esperar en la interfaz
  (la carga en su hilo y aplicarla en un `tick` posterior), un cambio de
  `App` que va con la partición de M5. El vigilante no abandona esas
  cargas (es trabajo de Rust), así que no empeora.
- **N15** (`meter()` no cambia la longitud de las pistas) va con M2 (qué
  es «compás»): fuera de esta tanda, en «Preguntas para Roi» del informe.
- M1 (el limitador), M2 y M5: fuera de alcance por decisión de Roi.

## Cortes independientes y guardar su edición

Continuación de la edición de cortes con el ratón (sin fase propia). Cinco
commits: el arreglo del guardado, el modelo y el motor, el ratón y el
teclado, la API de Lua y esta documentación.

### El fallo: la edición en la onda no se guardaba

- **Reproducido** antes de arreglarlo en `tui/src/tests/slice_save.rs`:
  arrastrar la marca de un corte (con las coordenadas de la geometría de la
  onda), Ctrl-S, abrir de nuevo (el script, o el proyecto y el script) y
  comparar los cortes. Los tres primeros tests fallaban con el código de
  antes (volvía `[0..3000, 3000..6000]`).
- **Causa raíz**: los ajustes hechos con el ratón o las teclas (`,` `.`
  `<` `>`, arrastrar, `n`, Supr, clic derecho) iban a `ScriptHost::positions`
  y `edits` y se guardaban solo en el `.sampler` (`App::save_project_file`), y
  solo con un proyecto abierto; sin él, el mensaje decía «se guardará con
  «guardar»» y se quedaban en memoria. Ctrl-S (`App::save`) escribe solo el
  script, y abrir el `.lua` (`App::open`) no lee el proyecto: sin proyecto
  se perdían al cerrar, y con él, al abrir solo el script. La recarga al
  guardar no pisaba nada (el script no decía nada de ellos); el problema era
  que la fuente de verdad estaba partida entre el código y un archivo aparte
  que no siempre se usaba. Con el editor en su ventana pasaba lo mismo: la
  principal guardaba el proyecto y la del editor solo el script.
- **Decisión: el código es la fuente de verdad** (la preferencia del
  usuario, y lo que ya hacen el tracker, el piano roll y las lanes). Lo
  editado reescribe la llamada que corta el sample en la forma explícita,
  `slice({ {inicio, fin [, "nombre"]}, ... })` en ms del sample sin estirar
  (`script::slice_call`). La primera edición de un `slice(n)` o un
  `slice_grid(n)` lo convierte con los cortes que daba (con los ajustes,
  partir, unir y etiquetas del proyecto y los cruces por cero) y lo avisa. Se
  descartó el `.sampler` como fuente única: obligaría a abrir siempre el
  proyecto y dejaría el script diciendo `slice(8)` con otros cortes.
- **Dónde está la llamada**: `slice_call::find`, al evaluar `slice` o
  `slice_grid`, busca `:slice(`/`:slice_grid(`/`:slice{` fuera de cadenas y
  comentarios en la línea que da Lua (en Lua 5.4, la del principio de la
  cadena de métodos) y las tres siguientes, y luego las tres anteriores;
  vale la que tiene escrito el mismo primer argumento (el número, o la tabla
  leída con `parse_ranges`, que solo acepta números, cadenas y llaves). Dos
  iguales en una línea, un número de una variable o una tabla con cuentas:
  `SliceOrigin::call` es `Err` con el motivo. `SliceCall` guarda el sitio
  (`TableSpan`), los cortes de la llamada (frames sin estirar), sus
  etiquetas y lo que va tras el primer argumento (`, { zero_cross = 2 }`, que
  se conserva).
- **Escribirla** (`tui/src/app/slices.rs`, `App::write_slices`): solo con el
  texto cargado (si no, «carga antes (Ctrl-S)»), en un paso de deshacer
  (`replace_text`), y se carga. Al releerla, las pistas de esa llamada (las
  que la usan entera o con `slice_at`) tienen que tener los cortes nuevos que
  han cambiado (con su estiramiento y, con `zero_cross`, a esa distancia) y
  las demás, los de antes; si no (un `:move()` o `:add()` después, que
  manda), se deshace y se dice. Los ms se escriben con los menos decimales
  (hasta tres) que vuelven al mismo frame (`frame_ms_text`): `67.2` y no
  `67.208`. Una línea si cabe en 72 caracteres; si no, cuatro cortes por
  línea con la sangría de la llamada.
- **El proyecto**: lo que guardaba de unos cortes convertidos (`corte =`,
  `partir =`, `unir =`, `etiqueta =` de ese sample cortado así) no se
  guarda más (`App::project_with` lo filtra con `dropped_from_project`) si
  ninguna llamada del detector o de la rejilla de la canción lo usa ya; en
  memoria se conserva, así que Ctrl-Z vuelve a `slice(8)` con sus ajustes.
  Ctrl-S guarda el script y, si hay algo que quitar, el proyecto (los dos
  con `script::atomic`). Las etiquetas de cortes del detector sin editar
  siguen en el proyecto, como antes (`etiquetar`); en cortes explícitos van
  en su entrada de la tabla.
- **Si no se puede escribir**: con cortes del detector cuya llamada sale de
  código, la marca de inicio se mueve como antes (en memoria o en el
  proyecto abierto) y el mensaje dice «no se puede guardar en el código»,
  por qué y cómo pasarlo a literal; los demás movimientos (final, entero) y
  las tablas generadas no se cambian, con el mismo mensaje.
- **La ventana del editor aparte**: la principal reescribe su copia del
  texto y la manda con `ToEditor::Text` (como el tracker); se guarda desde
  la ventana del editor. Test `with_the_editor_in_its_window_the_edit_reaches_it`.
- Tests (`tui/src/tests/slice_save.rs`): sin proyecto y con él (abriendo
  el proyecto y solo el script), tres ciclos de editar, guardar y abrir, un
  proyecto antiguo con `corte =` y `etiqueta =` que pasan al código y salen
  del proyecto, una llamada generada (se aplica y avisa) y una tabla en una
  variable (no se cambia), el texto sin cargar y Ctrl-Z; y en
  `script/tests/slice_ranges.rs`, dónde se encuentra cada llamada y que la
  reescrita da los mismos cortes.

### Cortes independientes

- **Modelo**: `engine::Slices` sigue siendo un rango por corte; los del
  detector y la rejilla son contiguos (`is_contiguous`) y los de
  `Slices::from_ranges` tienen cada uno su inicio y su final, en el orden
  dado, con huecos o solapes. `from_ranges` da el índice y el motivo del
  primero que no vale (`RangeError`: vacío, fin antes del inicio, fuera del
  sample, más corto que el mínimo). `with_range` cambia uno sin tocar los
  demás y `snapped_starts` lleva solo los inicios al cruce por cero.
- **Mínimo de longitud**: 1 ms en el código (el margen de siempre,
  `min_slice_gap`) y 1,2 ms al editar en la onda (como el arrastre de
  antes: que el redondeo a 0,1 ms no lo pise). Los solapes no tienen
  límite: dos cortes pueden ser casi el mismo.
- **Los scripts de siempre suenan igual**: `slice(n)` y `slice_grid(n)` no
  cambian (siguen por `project_cuts`, el código de antes), y una tabla con
  los mismos cortes contiguos suena bit a bit igual
  (`contiguous_ranges_sound_like_the_detector`). golden.rs, idéntico.
- **Motor**: los cortes ya viajaban como rangos resueltos en el patrón
  (`resolve_bar`, fuera del hilo de audio) y entran con el lote al compás;
  no hace falta capacidad nueva. La voz que suena guarda su rango y su
  `Arc` del sample: si cambian los cortes, acaba su corte con su fundido
  (`new_slices_while_one_sounds_let_it_end_without_a_click`), y si un golpe
  nuevo la interrumpe, el fundido cruzado de siempre. `no_alloc.rs` toca
  cortes con huecos y solapes, los cambia mientras suenan y los usa en un pad
  y en la preescucha. Con `stretch`, los finales de los cortes explícitos
  también son límites del WSOLA.
- **La interfaz** dice qué corte suena por su inicio: con dos cortes que
  empiezan igual, el primero.

### Ratón y teclado

- **Geometría**: `ui::WaveLayout` (el área de la onda y los frames que se
  ven, que guarda el dibujo en `App::wave_layout`, en lugar de
  `sample_area`) con `hit`: el borde más cercano a 1,5 columnas; si un
  inicio y un final caen juntos, el inicio pulsando en la celda de la marca
  o a su derecha y el final a su izquierda (se probó primero dar prioridad
  al corte elegido, pero pulsar en la marca «2» agarraba el final del 1);
  dentro de un corte, el cuerpo (el elegido si lo cubre; si no, el que
  empieza más tarde); si no, un hueco. Los tests piden dónde pulsar con
  `point_of`.
- **Arrastrar** (`app/mouse.rs`, `SliceDrag`): un borde mueve ese borde, el
  cuerpo mueve el corte entero, y con **Alt o Ctrl** (`App::mouse_with`; en
  `main.rs`, `mouse_held`) la marca de inicio se mueve con los cortes que
  empiezan o acaban justo ahí, lo de antes. Shift no: la terminal lo usa
  para seleccionar. La vista enseña los cortes del arrastre
  (`SliceDrag::preview`) y al soltar se escribe (`App::move_slice_edge`).
  Pulsar dentro de un corte ya no suena al pulsar sino al soltar sin mover
  (todavía puede ser un arrastre). El clic derecho en un hueco pone un corte
  desde ahí hasta el inicio siguiente.
- **Teclado**: `b` en el modo tocar (y «Cambiar qué mueven las teclas del
  corte» en la paleta, `Action::SliceEdge`) elige qué mueven `,` `.` `<` `>`:
  inicio (al empezar), final, entero o marca compartida. Todo pasa por
  `app::slices::moved` (pura, con sus tests): topes en el principio y el
  final del sample, en su propio otro borde y, con la marca compartida, en
  los vecinos; el mensaje dice cuál.
- **Dibujo**: la onda con un color por cobertura (apagado en un hueco, el de
  los avisos donde se solapan dos o más, sin colores nuevos en el tema), `]`
  en la fila de los números para cada final que no es el inicio de otro, y
  el título con «N huecos · N solapes» y el inicio y el final del corte
  elegido; en ASCII, lo mismo (`]` es ASCII).
- Tests: `tui/src/tests/mouse.rs` (inicio sin mover al vecino, topes, final
  a la izquierda de la marca, cuerpo entero, Alt como antes),
  `tui/src/tests/slice_edit.rs` (la geometría, huecos y solapes en palabras
  y en ASCII, clic derecho en un hueco) y `tui/src/tests.rs` (`b` y las
  teclas con cada modo; la marca compartida da los mismos cortes que antes).

### API de Lua

- `sample:slice({ {inicio, fin [, "nombre"]}, ... } [, opciones])`: mismo
  nombre que `slice(n)` (el primer argumento dice cuál), hasta 64 cortes,
  errores con el número del corte (`script/tests/slice_ranges.rs`: fin antes
  del inicio, fuera del sample, negativo, menos de 1 ms, vacía, con claves,
  nombre repetido o que no es texto); si falla, sigue sonando lo anterior.
  Las etiquetas del proyecto no se aplican a una tabla.
- `cortes:move(i, ms, fin)` mueve solo ese corte (con cualquier corte); con
  los de una tabla, `move(i, ms)` mueve solo su inicio. `cortes:range(i)` da
  el inicio y el final en ms.
- En el registro (`api.rs`), con la guía regenerada; los enlaces del
  tutorial al ancla nueva de `slice`.

## La ventana aparte del editor abre el mismo proyecto

Fallo: con un proyecto abierto y código escrito, «Abrir el editor en su
ventana» (menú Ventanas) abría una ventana sin ese progreso, con la
plantilla o con otro contenido. Tests en `tui/src/tests/editor_window.rs`
(fallaban antes del arreglo): un script o proyecto abierto con ruta
relativa, con y sin texto sin guardar, la ventana arrancada en otra
carpeta; con ruta absoluta (ya funcionaba); un archivo que no existe; un
script nuevo sin guardar; dos principales con scripts del mismo nombre;
una principal de otro script en el socket; lo escrito antes de conectar; y
`[editor] terminal` con `--directory`.

### Causas (con las hipótesis de partida)

1. **Ruta relativa y carpeta: confirmada, la causa principal.**
   `launch_editor` pasaba `self.project` o `self.path` tal como se abrió
   (`tui --edit cancion.sampler` desde la carpeta del proyecto, o lo que
   abriera el explorador). La terminal nueva no tiene por qué arrancar en
   esa carpeta (`kitty --directory`, servidores de terminal, o el comando a
   mano que dice la principal sin `[editor] terminal`), así que la ventana
   resolvía otra ruta y además buscaba el socket con un resumen de esa otra
   ruta: nunca se unía a la principal y nunca recibía su texto. Igual en
   las ventanas de dispositivos y del piano roll.
2. **El búfer sin guardar: descartada como causa, con un fallo al lado.**
   Unida a la principal, la ventana ya recibía su texto en el saludo
   (`Welcome`) y el historial (`History`). Pero al conectar mandaba su texto
   antes del saludo, y con el texto leído antes de atender los mensajes: lo
   escrito en la ventana mientras se conectaba pisaba lo que la principal
   tenía sin guardar.
3. **La plantilla: confirmada.** `App::build` usa `read_script`, que da la
   plantilla si el archivo no existe (lo correcto para `tui --edit x --new`,
   no para la ventana aparte). Con un proyecto, `Project::load` fallaba y la
   terminal se cerraba sin dejar leer el error.
4. **Varios proyectos: descartada.** El socket va por un resumen de la
   ruta canónica del script; dos principales con scripts del mismo nombre
   en carpetas distintas no se pisan (test). Faltaba que el editor
   comprobara que quien contesta es la principal de su script.
5. **`--directory` en la config: confirmada como desencadenante** (con la
   causa 1); una carpeta que no existe hacía fallar la terminal sin aviso.

### Arreglos

- `tui/src/target.rs`: `full_path` (canónica; si el archivo aún no existe,
  la carpeta canónica y el nombre), `window_target` (lo que abre una
  ventana aparte: error con el motivo y la carpeta desde la que se buscó si
  no existe el archivo o el script del proyecto) y `terminal_problem`
  (`--directory`, `--working-directory`, `--workdir`, `--cwd`, `--dir`, con
  `=` o aparte y `~`). `link::socket_path` usa `full_path`.
- `editor_command`, `devices_command` y `roll_command` pasan la ruta
  completa; los mensajes de «ábrelo a mano» también.
- `App::open_editor` abre solo lo que existe; `--window-editor`,
  `--window-devices` y `--window-roll` dejan el error en la terminal hasta
  pulsar Enter (`in_own_window` en `main.rs`).
- `launch_editor` llama a `ensure_on_disk`: un script nuevo que aún no está
  en el disco se guarda con `script::atomic` y lo dice; uno que existe no
  se toca. Un proyecto que ya no está es un error con la orden para
  guardarlo.
- En el editor (`poll_editor`, `editor_message`): nada se manda antes del
  saludo; el texto que se manda es el de después de atender los mensajes;
  si el saludo trae una ruta absoluta que no es la de su script, se
  desconecta con el motivo; si los dos lados cambiaron el texto antes de
  unirse, se queda el de la principal.

### Decisiones

- **Lo no guardado viaja por el socket, no se guarda solo.** Guardar antes
  de abrir la ventana cambiaría el archivo sin pedirlo; el texto ya iba en
  el saludo. Solo se guarda un script que no existe (sin archivo, la
  ventana no tiene qué abrir), y se dice.
- **Si los dos lados tienen cambios al unirse, manda la principal**: lo
  suyo es lo que suena y lo que había antes de abrir la ventana; lo de la
  ventana queda a un Ctrl-Z (un paso de deshacer), nunca se pierde.
- **Un solo texto, como hasta ahora**: las dos ventanas tienen una copia
  sincronizada por revisiones (el protocolo de la fase L); no se cambia a
  un búfer compartido de verdad, que pediría otro protocolo.
- **Sin cambiar el protocolo (sigue el 8)**: la ruta del saludo ya iba;
  ahora es la completa. Si una principal de antes manda una relativa, el
  editor no la comprueba (no puede) y se une como antes.
- **La carpeta de la terminal no se usa**: con la ruta completa da igual;
  solo se avisa si la que se pide no existe, en vez de lanzar.

## Explorador de samples y samples del proyecto

Pedido por el usuario: el árbol de archivos salía siempre vacío, y quería
añadir samples (los suyos están en `~/songsproject/samples`, compartida
entre las canciones de `~/songsproject/1`, `/2` y `/3`) y que se quedaran
en el proyecto.

### Dónde abría el árbol y por qué salía vacío

- **Dónde abría**: en `App::base_dir()`, la carpeta del `.sampler` o, sin
  proyecto, la del script: `path.parent()`, con `.` si no había. Nada de
  `config.toml` ni rutas fijas.
- **La causa** (test `a_script_opened_by_its_bare_name_shows_its_folder`,
  falla con el código de antes): con `tui --edit song.lua` desde la carpeta
  de la canción, `Path::new("song.lua").parent()` es `Some("")`, no `None`;
  el `unwrap_or(".")` no saltaba y el árbol leía `read_dir("")`, que falla.
  La raíz salía sin nombre y con «no se puede leer: N…» cortado por el
  ancho del panel: vacío a la vista. Además, los samples de la carpeta
  compartida no estaban en ninguna raíz (no hay forma de ver una carpeta
  fuera de la del script).
- **Hipótesis descartadas**: el filtro de extensiones ya aceptaba `.WAV`
  (minúsculas al comparar) y los cinco formatos del motor; la carga
  perezosa sí se disparaba al abrir; no hay hilo de lectura; el panel tenía
  su alto. Sí se escondía en silencio lo que no era audio cargable (m4a...).
- **Arreglo**: `base_dir()` da `.` si `parent()` es vacío y el árbol pasa
  cada raíz a completa y canónica (`target::full_path`); `reveal`, el
  script marcado `◂`, abrir, renombrar y borrar comparan rutas completas.

### El árbol (`tui/src/files.rs`, `tui/src/app/files.rs`)

- **Raíces** (`Tree::with_roots`, `App::files_roots`), en orden y con su
  nombre: `[proyecto]` (`samples/` junto al script si existe, y la de
  `samples =` si es otra), `[global]` (`[samples] carpeta`) y `[script]`
  (la carpeta del proyecto o del script). Una carpeta repetida sale una
  vez. Las filas llevan su sitio en el árbol (`Row::at`) en vez de
  buscarse por ruta, porque una misma carpeta puede estar bajo dos raíces.
  Con más de una raíz, cada una se pliega. El árbol se rehace al abrir un
  proyecto, cambiar la carpeta de samples o la global, al refrescar si
  cambió qué raíces hay, y al abrir un script de fuera de las raíces.
- **`[samples] carpeta`** en `config.toml` (`Config::samples_dir`): `~` y
  `~/…` con `$HOME`; relativa, respecto a la carpeta de `config.toml` (la
  resuelve `Config::load`).
- **Lo que se ve**: el audio que el motor no carga
  (`files::UNSUPPORTED_AUDIO`: m4a, aac, opus...) como `Kind::Unsupported`,
  atenuado y con «formato no soportado: m4a» (Enter lo explica y propone
  ffmpeg); una nota cuenta los archivos que no son audio ni scripts; una
  carpeta vacía o que no existe y un error de lectura, con la ruta
  completa. Un enlace a una carpeta que contiene a la suya (o a sí misma)
  se marca y no se lee.
- **Refrescar**: `R` con el foco y la acción «Refrescar el explorador»
  (menú Ver). Solo se releen las carpetas abiertas; en reposo no se lee el
  disco (las raíces solo se miran al abrir proyecto o refrescar).

### Dónde busca `sample("...")` (`script/src/sample_path.rs`)

En orden: (1) la carpeta de antes (la de `samples =` o `muestras`; si no
hay, la del script), (2) la del script, (3) `samples/` junto al script,
(4) la global. Gana la primera. Así **un script de antes carga los mismos
archivos que antes** (lo que antes se encontraba estaba en la primera) y
`golden.rs` no cambia. Si una carpeta de después tiene un archivo distinto
con el mismo nombre (se compara el contenido hasta 64 MB), aviso. Si no
está en ninguna, aviso con dónde se buscó y diez segundos de silencio
(`missing_sample`, en la caché); `slice(n)` de ese silencio da `n` cortes
en rejilla, para que los pasos existan y callen, y `stretch()` sin tempo no
hace nada. Un archivo que existe pero está roto sigue siendo un error (y
sigue sonando lo anterior). El render por la línea de órdenes también usa
la carpeta global (`script_host` en `main.rs`). No hace falta API nueva:
`sample()` es la misma, con su documentación del registro al día.

### Añadir samples al proyecto (`tui/src/add_samples.rs`)

- `plan` decide qué se copia: el audio cargable de cada archivo o carpeta
  (recursivo, hasta 2000 archivos y 16 niveles, sin ocultos, sin enlaces
  que salgan de la carpeta ni vueltas), con su sitio dentro de `samples/`.
- `start` copia en un hilo aparte; la interfaz lo mira en cada vuelta
  (`poll_adding`) y pone una barra de progreso en la línea de mensajes
  (solo cambia el mensaje cuando cambia el porcentaje: el dibujo en reposo
  sigue sin redibujar). Cada copia: nombre válido (UTF-8, sin control, no
  oculto, ≤ 255 bytes, sin `..`), se prueba a cargar con
  `engine::Sample::load_wav` (los pánicos, atrapados), espacio libre con
  `statvfs` (más 16 MB) y `script::atomic::write_with`.
- Decisiones:
  - **Copiar, no referenciar, por defecto** (preferencia del usuario): el
    proyecto se mueve entero. Referenciar (`A`) existe con aviso.
  - **Mismo nombre**: igual, se reutiliza; distinto, `nombre-2.ext` (el
    primer libre, o uno igual que ya esté), sin preguntar: nunca se
    sobrescribe y el mensaje lo dice. Preguntar en mitad de una copia de
    una carpeta entera sería una pregunta por archivo.
  - **Qué se escribe**: el nombre más corto que, con el orden de búsqueda,
    encuentra justo esa copia (`sample_name_for`): `amen.wav` si nada lo
    tapa; si otro `amen.wav` distinto junto al script lo taparía,
    `samples/amen.wav`. Una carpeta o varios marcados no escriben nada.
  - **Lo que ya está en el proyecto** no se copia; solo se escribe.
- «Recoger los samples en el proyecto»: los `sample("...")` literales del
  texto (`sample_literals`, sin comentarios) que se resuelven fuera de la
  carpeta del script se copian igual y, al acabar, su texto se cambia en un
  paso de deshacer (`replace_text`) y se carga. Los generados por código y
  los que faltan se cuentan en el mensaje.
- Teclas: `a` añade, `A` referencia, `m` marca; arrastrar del árbol al
  editor añade. Acciones «Añadir sample al proyecto», «Referenciar sample
  sin copiar» y «Recoger los samples en el proyecto» (menú Archivo).
- **Orden de los commits**: la resolución de rutas va antes que añadir
  (añadir escribe nombres relativos a `samples/`, que solo se encuentran
  con la resolución nueva).
- El motor no cambia: el sample nuevo llega con la carga de siempre, fuera
  del audio y al compás; `no_alloc.rs` no tiene camino nuevo que probar.

## Arrastrar un corte no quedaba fijado

Continuación de «Cortes independientes y guardar su edición» (sin fase
propia), encontrado al probar en el MacBook con Asahi Linux (ARM64, kitty
sobre Hyprland, sin tmux).

### La causa: el final del sample escrito con pocos decimales

- **Síntoma**: al soltar el borde de un corte, el corte volvía a su sitio.
  La línea de mensajes decía «no se cambia: el código escrito no carga
  (cancion.lua:5: slice({...}): el corte 16 acaba en 5680.46 ms, pasado el
  final de amenidm.wav (5680.5 ms))», pero el siguiente clic la pisaba.
- **No es de la arquitectura**: depende del largo del sample. `amenidm.wav`
  tiene 250 508 frames a 44,1 kHz (5680,4535 ms). Al reescribir la llamada,
  `frame_ms_text` escribe el final del último corte con los menos decimales
  que vuelven a su frame: `5680.46`. Ese número pasa unas milésimas del largo
  exacto en ms, y `cut_explicit` comparaba en ms (`end > length + 1e-6`):
  el código escrito no cargaba, `write_slices` lo deshacía y el corte
  volvía. Pasaba con cualquier arrastre (cada reescritura incluye el final
  del último corte) y con cualquier sample cuyo largo en ms no sea redondo
  al redondear hacia arriba; el break de los tests (6000 frames a 48 kHz,
  125 ms justos) no lo veía, por eso los tests estaban en verde en las dos
  máquinas.
- **Arreglo**: la comprobación del final va en frames
  (`ms_to_frame(end) > frames`), como ya hacía `cortes:move`. Lo que de
  verdad pasa del final (aunque sea medio frame) sigue siendo un error.
- **Tests**: `script/tests/slice_ranges.rs`
  (`the_end_of_a_sample_written_with_few_decimals_still_fits`: 6007 frames a
  48 kHz, que se escriben `125.15`) y `tui/src/tests/slice_save.rs`
  (`every_slice_of_a_sample_with_an_odd_length_can_be_dragged_saved_and_reopened`:
  el inicio y el final de cada corte, con `slice_grid(4)`, `slice(4)`, la
  tabla explícita y `stretch(130):slice_grid(4)`, cada arrastre se fija;
  Ctrl-S y abrir de nuevo dan los mismos cortes). Los dos fallaban antes del
  arreglo con el mensaje de arriba. Con una copia de la canción del usuario
  (`slice_grid(16)` y `stretch(140):slice_grid(16)`, abriendo el `.lua` y
  el `.sampler` con sus `corte =` de antes), los 64 arrastres de inicio y
  final de las dos pistas se fijan y sobreviven a guardar y abrir.

### El aviso se perdía: ahora se queda en el panel del sample

- **Problema de usabilidad** (aparte de la causa): cuando un cambio de
  cortes no se fija (el código reescrito no carga, el texto no está
  cargado, un `:move()` después manda) o no se puede guardar (`slice(n)` con
  una variable, una tabla generada), el motivo solo iba a la línea de
  mensajes: una fila, que se trunca con los mensajes largos (el de arriba
  pasa de 170 caracteres) y que pisa el siguiente clic o la siguiente carga.
  El corte volvía a su sitio sin nada a la vista.
- **Arreglo**: `App::slice_warning` guarda el motivo
  (`not_written_message` lo pone: «el cambio de cortes no se fijó: …» o
  «los cortes movidos no se guardan: …») y el panel del sample (el normal y
  el grande) lo dibuja en su borde de abajo con `⚠` en el color de los
  avisos. Se quita con el siguiente cambio que se fija (`write_slices`) o
  con la siguiente carga que va bien (el texto es otro: el motivo ya no
  vale). La línea de mensajes sigue diciendo lo mismo que antes.
- Test: `a_slice_change_that_does_not_stick_stays_visible_on_the_sample_panel`
  (`slice(n)` con una variable, una tabla en una variable y `:move()`
  después; tras un clic que cambia el mensaje, el aviso sigue en pantalla; y
  se va con una carga y con un cambio que se fija).

### El ratón por la terminal real

- **En una PTY** (un script de Python con `pty` y `pyte` como terminal,
  fuera del repositorio; sin `--allow-audio`, salida nula), con secuencias
  SGR 1006 reales: pulsar, movimientos con el botón (`32`) columna a
  columna y soltar. Llegan Down, Drag y Up en orden; el arrastre se fija y
  Ctrl-S lo escribe en el archivo, también soltando fuera del panel (en la
  fila del editor) y con Alt, Ctrl o Shift en los bits del evento.
- **Modos**: `EnableMouseCapture` de crossterm pide 1000, 1002, 1003, 1006 y
  1015, y `DisableMouseCapture` los quita al salir (también con error o
  pánico, con la guarda `TerminalModes`). Comprobado en la salida de la PTY.
- **El bucle de eventos** no pierde el Up: lo leído de una vez se procesa
  entero, y solo se salta un lote si todo él son movimientos sin botón
  (`only_motion`). Un clic nuevo suelta un arrastre que se quedó sin Up
  (soltado fuera de la ventana).
- **Lo que la terminal se queda**: en kitty, Shift y arrastrar selecciona
  texto aunque el programa tenga el ratón (por eso Shift no es modificador
  en la onda), y Ctrl-Alt y arrastrar es la selección rectangular; Alt o
  Ctrl solos llegan al programa. Hyprland mueve ventanas con Super y
  arrastrar (su configuración de ejemplo). No hay tmux en esta máquina.
- **Clic o arrastre**: soltar en la misma columna es un clic (suena); una
  columna de movimiento ya es arrastre. No se cambia: es lo que permite
  afinar columna a columna.
- **Sin traza**: la causa se reprodujo con tests, así que no se añadió el
  modo de traza de eventos del ratón.
## Editor: rueda, linter y mejoras ligeras

Pedido por el usuario: la rueda del ratón (y los gestos del touchpad) en
el editor, un linter que avise mientras se escribe y unas cuantas
comodidades. Una sola implementación sirve para el editor integrado, el de
su ventana (`--window-editor`, que corre la misma `App` con `run_terminal`
y ya pedía el ratón) y el flotante (se dibuja con el mismo `draw_editor`).

### Qué había ya

- La rueda vertical en el editor (tres líneas, con `TextArea::scroll`, que
  arrastra el cursor con la vista); la horizontal llegaba como
  `ScrollLeft`/`ScrollRight` (también Shift y la rueda, en `mouse_action`)
  pero el editor la ignoraba. Clic, arrastrar y seleccionar (fase R).
- El reparto del ratón por paneles (`App::mouse_with`: menús, bordes,
  flotante, pestañas y foco, piano roll, lanes, tracker y mezclador, onda,
  y el editor al final), y la geometría de cada panel en `ui.rs`.
- Diagnóstico en vivo de la notación (`tui/src/diagnose.rs`: `steps`,
  `melody` y los nombres de `:fx`), con caché por huella del texto,
  subrayado y la primera línea al pie; Ctrl-G va a la línea del error de
  la última carga.
- Comentar y descomentar líneas (`editing::toggle_comment`, en la paleta)
  y el resaltado de todas las coincidencias de la búsqueda (el `search` del
  widget pinta todas con `search_style`): de la parte 3, no hacían falta.
- El ajuste de línea: el widget lo tiene (`WrapMode`), pero la aplicación
  no lo usa (los subrayados, el resaltado en vivo y el ratón cuentan con
  líneas sin partir). No se añade.

### Rueda y barras (`tui/src/app/scroll.rs`)

- **Vista libre.** `TextArea::scroll` mueve el cursor para que quede en
  la vista, y al dibujarse el widget se desplaza hasta el cursor; no hay
  una vista aparte. La rueda guarda la suya (`EditorView::free`: primera
  fila y primera columna, en las columnas del widget, con los números de
  línea) y `render_editor` dibuja con ella: mueve el viewport del widget
  (con `scroll` y devolviendo el cursor a su sitio), y si el cursor no
  cae dentro, lo pone un momento en una celda a la vista, sin estilo de
  cursor ni de su línea, dibuja y lo devuelve. La selección se conserva
  (su inicio no cambia) y se pinta encima donde de verdad está. Nada del
  texto ni del historial se toca.
- **Volver al cursor.** `App::handle` y `App::paste` comparan una huella
  barata (cursor, número de líneas y largo de la línea del cursor) antes y
  después: si cambió, se suelta la vista libre y, al dibujar, la vista se
  mueve lo justo para dejar `[editor] margen` líneas (2) por encima y por
  debajo del cursor (`keep_margin`). Solo tras una tecla: un clic no
  desplaza la vista bajo el ratón. Una tecla que no mueve nada (F6 sin
  búsqueda) no suelta la vista. El modo vim pasa por `handle`: `j` vuelve.
- **Límites.** Abajo, la última línea abajo del todo; a la derecha, el
  final de la línea más ancha **a la vista** en el borde (sin líneas
  anchas a la vista no se va de lado).
- **Ráfagas.** Un paso suelto mueve `rueda_lineas` (3) o `rueda_columnas`
  (6); dos pasos a menos de 40 ms son una ráfaga (touchpad, o una rueda
  girada deprisa) y cada uno mueve un tercio (1 y 2): el gesto se desliza
  en vez de saltar de tres en tres. Se decide con el instante que llega
  con el evento (`App::mouse_at`, así se prueba).
- **Barras** (`[editor] barras`, apagadas): la última columna del editor
  y, si alguna línea no cabe, la última fila; guía tenue y la parte que se
  ve con el acento (`│┃` y `─━`, en ASCII `|#` y `-=`). Pulsar salta ahí y
  arrastrar las lleva (centrando la vista en el punto); no mueven el
  cursor. Apagadas no cuestan nada (ni se mira la línea más ancha).
- **Reparto del ratón.** La rueda solo desplaza el editor si cae dentro de
  su texto; en la onda sigue el zoom, y en el tracker, la ocupación, el
  explorador y los menús, lo de siempre (van antes en `mouse_with`).
- Tests: `tui/src/tests/wheel.rs` (vertical, horizontal, límites, la línea
  ancha, ráfagas y su ajuste, margen y su ajuste, clics y arrastres sobre
  la vista desplazada, selección conservada, vim, barras y ASCII) y
  `tui/tests/pty_wheel.rs`: `tui --edit` en un pseudoterminal con las
  secuencias SGR 1006 de una terminal (botones 64, 65, 67 y 68, que es
  Shift y la rueda), leyendo la pantalla con un intérprete mínimo de lo
  que escribe ratatui; lo escrito después va a la línea 1 (el cursor no se
  movió) y se guarda.

### Linter (`script/src/lint.rs`, `tui/src/app/lint.rs`)

- **Sintaxis sin ejecutar.** `lint::syntax` compila el texto con un
  `Lua::new_with(StdLib::NONE, ...)` recién hecho, con el límite de
  memoria de `[script] memoria`, en `ChunkMode::Text` (un trozo
  precompilado se rechaza) y con `into_function`: compila y no llama a
  nada. Un bucle infinito arriba del todo se analiza en lo que tarda en
  compilarse. El tiempo: el análisis va en su hilo y, si pasa de `[script]
  tiempo` (más medio segundo), se abandona sin esperarlo y ese texto no se
  vuelve a mirar (el compilador de Lua es lineal: no debería pasar; tras
  cuatro abandonos el linter se apaga hasta reiniciar). El mensaje de Lua
  se traduce (`lint::spanish`: «falta «)» antes de «amen»», «cadena sin
  cerrar», «número mal escrito»...; lo que no se conoce sale como viene) y
  se sitúa: el token de «near» en su línea, y lo que no se cierra, en la
  línea que lo abre.
- **API, contra el registro.** Un léxico de Lua propio (comentarios y
  cadenas largas incluidos) y, sobre los tokens:
  - funciones globales que no son de la API, de Lua ni del script
    (definidas de cualquier forma: asignadas, locales, `function`,
    parámetros, variables de `for`, campos) y métodos que no tiene ningún
    objeto de la API, ni las cadenas, ni define el script: **error**, con
    «¿quisiste decir…?» si hay un nombre único a 1 de distancia (2 en los
    de más de cinco letras, con trasposiciones, o igual sin distinguir
    mayúsculas);
  - las tablas `{ ... }` literales de las llamadas que tienen una tabla de
    parámetros (`api::call_table`, que ahora usan también el
    autocompletado y la ayuda: antes estaba en `complete.rs`; y las capas
    del subharm, `api::layer_call`): clave desconocida, valor de una lista
    cerrada que no está en ella, texto que no es un número o `true`/`false`
    en un parámetro numérico, y `ramp()`/`lfo()`/`lane()`/`macro()` en uno
    que no admite curvas (con el porqué del registro): **error**; un número
    fuera del rango `a–b` de `valid`: **aviso**;
  - el nombre del sintetizador de `synth("...")` y del bus de `bus("...")`:
    **error** (el del efecto de `:fx("...")` ya lo decía el diagnóstico de
    la notación);
  - un método que existe, pero no en el objeto sobre el que se llama,
    cuando el objeto se sabe seguro (una cadena de llamadas de la API, o
    una variable `local` asignada una sola vez con una cadena así, sin
    `or` ni nada detrás): **pista**.
- **Qué se deja sin analizar.** Todo lo que no es literal (variables,
  expresiones, tablas construidas), los parámetros de las funciones (su
  tipo no se sabe: el compás de `every` tampoco), los valores de los
  argumentos posicionales (sus rangos no están en el registro como datos,
  solo en texto), las variables asignadas más de una vez, y todo nombre
  desconocido si el script usa `_G`, `_ENV`, `load`, `rawset`,
  `setmetatable`, `getmetatable` o `require`. Tampoco hay «variable sin
  usar»: los ejemplos de la guía definen locales que no usan, y avisar ahí
  sería avisar en código válido.
- **Severidades.** Error: falla seguro (un test lo comprueba: cada caso de
  error de las tablas, más de 200, y los nombres de sintetizador y bus,
  falla de verdad al cargar; también cada aviso de rango).
  Aviso: casi seguro (el rango documentado); los avisos de la notación de
  `diagnose.rs` entran como avisos en el recuento y en la navegación.
  Pista: lo que puede no ser un fallo.
- **Cuándo.** La huella del texto se toma al construir la vista de un
  cuadro que se dibuja (la misma pasada que la caché de los avisos de la
  notación, que ahora reutiliza esa huella); `tick` lanza el análisis si
  la huella es distinta de la analizada y lleva `linter_ms` (300) sin
  cambiar. Un solo análisis a la vez; el resultado se queda hasta que
  llega el siguiente. En reposo no se dibuja, así que no se toma ninguna
  huella; `tick` solo compara dos números. El resultado cambia
  `generation`, que entra en la huella del reposo (`idle_key`) para que
  se dibuje. Con `linter = false`, ni huella ni hilo.
- **Pánicos.** El hilo corre el análisis dentro de `script::panics::catch`
  (que además calla el aviso en la terminal): con un pánico, no se marca
  nada y la interfaz sigue.
- **Cómo se ve** (`ui::draw_editor`): la marca de la gravedad mayor de cada
  línea en la primera columna de los números de línea (si se ven: con el
  texto desplazado de lado, no), el subrayado del color de su gravedad
  (los errores y avisos, también el texto), el mensaje entero en la línea
  `ⓘ` con el cursor en esa línea (manda sobre la ayuda de la función), y
  en la cabecera, `●N ▲N •N` (en ASCII, `E`, `W`, `i`). El pie con el
  primer aviso de la notación sigue como estaba.
- **Navegación.** «Diagnóstico siguiente» y «anterior» (Alt-J y Alt-K,
  `diagnostico_siguiente` y `diagnostico_anterior` en `[teclas]`; en vim,
  `]d` y `[d`), con el mensaje «error 1 de 2, línea 4: ...». Ctrl-G no se
  duplica: va al error de la carga si lo hay y, si no, al siguiente error
  del texto (o al siguiente diagnóstico si no hay errores).
- `cargo run -p script --example lint -- *.lua` revisa archivos fuera del
  editor (para comprobar canciones sin abrirlas).
- Tests: `script/src/lint.rs` (cada categoría con su ejemplo bueno y malo,
  la sintaxis en español, que nunca ejecuta, las sugerencias, el léxico),
  `script/tests/guide.rs` (`the_linter_finds_nothing_in_working_scripts`:
  ningún diagnóstico en los bloques de la guía, cada paso del tutorial,
  las recetas y los scripts de medir el consumo), `script/tests/registry.rs`
  (`every_linter_error_in_a_table_fails_to_load`) y
  `tui/src/tests/lint.rs` (la pausa y no en cada tecla, sin cambios no se
  relanza, configurable y apagado, margen, subrayado, línea ⓘ, cabecera,
  ASCII, el bucle infinito, Alt-J/Alt-K, Ctrl-G, `]d` `[d` y los avisos
  de la notación).

### Mejoras ligeras

Comentar y descomentar (la línea o la selección) y el resaltado de todas
las coincidencias de la búsqueda ya estaban (ver «Qué había ya»): no se
tocan. Lo nuevo, cada una en su commit y en `[editor]`:

- **La pareja del paréntesis** (`pareja`, encendida; `tui/src/brackets.rs`):
  el que está bajo el cursor o, si no, el de justo antes, y su pareja, con
  el fondo del acento. Busca hacia delante o hacia atrás hasta 500 líneas,
  sin contar los de las cadenas y comentarios de cada línea (las cadenas
  largas de varias líneas sí cuentan: haría falta leer el texto entero en
  cada cuadro). Solo cuesta algo con el cursor junto a uno de los seis
  caracteres.
- **Cierre de pares** (`cerrar_pares`, **apagado**; `tui/src/app/pairs.rs`):
  `(`, `[`, `{`, `"` y `'` ponen su pareja (un solo paso de deshacer, con
  `insert_str`); el que cierra delante del cursor se salta (una comilla,
  solo si cierra una cadena abierta en la línea); Retroceso entre una
  pareja vacía borra las dos (un paso). Solo dobla si detrás no hay nada,
  un espacio o algo que cierra, y una comilla no tras una letra, un número
  o `_` ni dentro de una cadena. Con selección, lo de siempre. Apagado por
  defecto: cambia lo que sale al teclear en directo, y quien escribe todo
  (como hacen los tests de siempre) no debe encontrarse caracteres que no
  escribió; quien lo quiera, lo enciende.
- **Guías de sangría** (`guias`, apagadas; en `ui::draw_editor`): `│` (`|`
  en ASCII) tenue en los espacios de la sangría de cada línea a la vista,
  una por nivel; el nivel es la sangría más pequeña del texto (de 2 a 8).
  Las líneas en blanco no llevan. Apagadas, no se mira nada.

## Audio de prueba y problemas conocidos

- **Audio de prueba repetible**: `scripts/render-demo.sh [entrada.wav]` renderiza `renders/demo.wav` sin usar la salida de audio. Sin argumento genera antes `renders/synth_break.wav` con `cargo run -p engine --example synth_break` (break sintético determinista: 8 golpes en corcheas a 140 BPM, 44,1 kHz). Variables: `PATTERN`, `SLICES`, `BPM`, `BARS`, `MIN_GAP` (ms), `DETECTOR` (`flux`|`energy`). `renders/` está en `.gitignore`. Para escucharlo: `scp <laptop>:~/sampler/renders/demo.wav .`
- Pendiente conocido: cpal avisa de un underrun al arrancar el stream (una vez, ~200 ms). PipeWire estaba inactivo y cpal iba directo a ALSA.
