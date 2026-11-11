# Sampler de breaks con live coding (terminal)

Sampler + secuenciador para remixes con amen breaks, controlado por código en vivo. Proyecto en Rust. El desarrollo se hace en una laptop x86_64 con Gentoo.

Guía de uso (inicio rápido, atajos, mini-notación, API de Lua, recetas y problemas): [docs/GUIA.md](docs/GUIA.md); el tutorial «Tu primera canción», aparte en [docs/PRIMERA-CANCION.md](docs/PRIMERA-CANCION.md). Sus bloques de Lua y las recetas de `scripts/ejemplos/` se comprueban en `script/tests/guide.rs`. La sección de la API se genera desde el registro `script/src/api.rs`: al cambiar la API, actualiza el registro y ejecuta `cargo run -p script --example guia`.

Hoja de ruta (fases A-W después de los pasos 1-7, con sus reglas comunes; la F se hizo antes que la E y la J, EQ y choques de frecuencia entre pistas, después de la K; la S antes que la Q y la R, y la R con la Q a medias, esperando al Seaboard): [docs/ROADMAP.md](docs/ROADMAP.md). Estado detallado y decisiones de cada fase: [docs/ESTADO.md](docs/ESTADO.md). Resumen abajo, en «Estado (resumen)».

## Idea central

- Panel izquierdo: editor de código. Panel derecho: vistas del estado. Panel inferior: efectos por pista.
- El código NO es la única fuente de verdad: el script produce un **modelo de estado** (pistas, patrones, cortes, efectos) y las vistas lo leen. Editor y paneles siempre coinciden.
- Layout objetivo:
  - Izquierda: editor de código.
  - Derecha arriba: sample (forma de onda, cortes, loop, slicer).
  - Derecha abajo: secuencia de pistas en vertical, estilo tracker (Renoise).
  - Inferior: efectos de cada pista.

## Stack

- Audio: `cpal`. Decodificación: `hound` (WAV) y `symphonia` (FLAC, MP3, OGG Vorbis, AIFF). Resampleo: `rubato`.
- Interfaz: solo terminal (TUI) con `ratatui` 0.30 + `ratatui-textarea` 0.9 (la continuación de `tui-textarea` dentro de la organización de ratatui; `tui-textarea` 0.7 obligaba a quedarse en ratatui 0.29). Decisión del usuario: nada de GUI por ahora, experiencia 100% terminal.
- Scripting: mini-notación para patrones (estilo Tidal) + Lua con `mlua` para la lógica. Recarga en caliente.
- DSP: los synths con código propio (`engine::synth`: determinista y sin reservas; `fundsp` no, ver «Fase O» en `docs/ESTADO.md`); `fundsp` queda como opción para efectos.

## Reglas de arquitectura (no negociables)

1. El callback de audio NO reserva memoria, NO usa mutex ni bloquea, NO hace E/S.
2. El script y la UI nunca tocan el audio directamente: envían comandos por una cola sin bloqueos (lock-free) al hilo de audio.
3. Motor separado de la interfaz (crate de librería + frontend TUI). No se hace GUI, pero la separación mantiene el código limpio y testeable.
3b. Pensar la UI para terminal: rejilla de caracteres, barras de bloques para formas de onda, atajos de teclado y ratón opcional. Usar truecolor con degradación elegante.
4. Los cambios evaluados entran cuantizados al final del compás.
5. Si el código es inválido, el patrón anterior sigue sonando y el error se muestra en el editor.
6. Los samples se decodifican a buffer en memoria y se resamplean a la frecuencia del motor al cargarlos.

## Orden de construcción

Un paso cada vez; probar con tests (audio sintético, renderizado offline) antes de seguir.

1. ✅ Motor de audio: cargar un WAV y reproducirlo (por la tarjeta solo con `--allow-audio`; queda el aviso de underrun al arrancar, ver Estado (resumen)).
2. ✅ Cortes: detección de transitorios (flujo espectral por defecto); cada corte se reproduce por número (`Controller::play_slice_on`) y desde el teclado en el modo tocar de la interfaz (1-0 y q-i).
3. ✅ Secuenciador de 16 pasos con BPM, loop y cuantización al compás; varias pistas y lotes atómicos.
4. ✅ Efectos por paso (volumen, pitch, reverse, retrigger/stutter) y por pista (volumen, filtro paso bajo/alto, bitcrusher), con limitador suave al final.
5. ✅ Mini-notación + Lua (`bpm`, `sample`, `slice`, `slice_at`, `track`, `steps`, `gain`, `fx`, `every`, `chance`) con recarga al compás y recarga automática al guardar.
6. ✅ Interfaz de paneles en la terminal (`tui --edit`): editor, sample en barras, tracker vertical, efectos por pista y estado del motor.
7. ✅ Exportar la mezcla a WAV (render offline: `--render` en la línea de comandos y la orden `exportar` en la interfaz) y guardar/cargar proyectos `.sampler` (script, carpeta de samples y tempo).

## Ideas para después

- Deformar y modificar el sample al importarlo (pitch, time-stretch, granular, etc.) y tocar melodías con él.
- Más instrumentos sobre el núcleo de síntesis de la fase O (pads, wavetable).
- MIDI: probar con dispositivos reales; enviar las notas con marca de tiempo (cola de ALSA) para quitar el jitter; reloj MIDI de salida; que los pads y las notas de entrada también se puedan grabar como `steps` (fase E).
- Alojar plugins CLAP/LV2: solo si hiciera falta; es costoso.
- Time-stretch de más calidad que el WSOLA de `sample:stretch()` para proporciones grandes o material tonal: Rubber Band tras una feature opcional.
- Cortes: gate y choke también por corte; remuestrear a WAV de 32 bits en coma flotante y con la cola entera.
- Automatizar también el volumen de pista, los envíos y el pan; rampas de una sola vez (que empiecen al entrar el patrón) además de las que se repiten; `~` para otros parámetros.
- Bloques (fase F): compartir la biblioteca entre máquinas (es una carpeta: git o Syncthing bastan), e insertar bloques también desde el modo tocar o por etiqueta.

## Fuera de alcance realista

Alojar VST, mezclador completo con buses (los grupos de un nivel de `group()` sí están), automatización compleja. No intentarlo.

## Ejemplo de la API objetivo (Lua)

Toda esta API existe ya (pasos 4 y 5; ver [docs/ESTADO.md](docs/ESTADO.md)). Cada llamada solo construye el modelo de estado; el motor aplica los cambios al final del compás.

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

Hecho: `steps(...)` reproduce los cortes, y `fx`, `every` y `chance` funcionan (ver el paso 5 en [docs/ESTADO.md](docs/ESTADO.md)).

## Estado (resumen)

El detalle de lo hecho (dónde está cada cosa en el código, cómo funciona, sus tests) y las decisiones de diseño de cada fase están en [docs/ESTADO.md](docs/ESTADO.md); lo que falta, en [docs/ROADMAP.md](docs/ROADMAP.md). Al terminar algo: detalle y decisiones en `docs/ESTADO.md`, marca en `docs/ROADMAP.md` y una línea aquí si cambia el resumen.

- **Crates**: `engine` (motor, sin interfaz), `script` (Lua con `mlua`, modelo `Song`, proyectos, bloques, registro de la API) y `tui` (binario: interfaz `ratatui`, render offline, MIDI tras la feature `midi`).
- **Pasos 1-7 hechos**: motor con cola de comandos lock-free y cola de basura, cortes por transitorios, secuenciador con lotes atómicos al compás, efectos por paso y pista, mini-notación y Lua con recarga al guardar, interfaz de paneles, exportar a WAV y proyectos `.sampler`.
- **Fases hechas**: A (editor: paleta, deshacer, snippets, diagnóstico en vivo, autocompletado y ayuda), B (etiquetas de corte), C (generadores, escalas, swing, humanize, fill), D (mute/solo, medidores, metrónomo, detección de BPM, preescucha, ajuste de cortes), E (grabar lo tocado, secciones con línea de tiempo en el motor, historial de versiones, variantes A/B), F (biblioteca de bloques y ranuras), G (longitud variable, time-stretch, delay y reverb como envíos, compresor y distorsión, automatización), H (gate/choke, forma por corte, rejilla, remuestreo, preparar al importar; y después, efectos por corte con `cortes:fx` y el efecto `gain`), I (explorador, tap tempo, rampas de BPM y compases, configuración, MIDI de entrada y salida, ranuras que persisten), J (EQ paramétrico, choques de frecuencia entre pistas con `--analyze-masking` y la paleta, `priority()` y sugerencias de EQ que se insertan tras confirmarlas, EQ dinámico con sidechain por bandas), K (melodías con samples: interpolación sinc, instrumento con `melody`, en vivo y por MIDI, pitch sin cambiar la duración con `%` y `keep`, granular y congelar, formantes) y M (resaltado en vivo del código al sonar: posiciones de los tokens, paso por pista publicado con atómicos y retardo de salida, `[resaltado]` y la paleta, también en el editor en su ventana con el protocolo 3).
- **Nube de granos en vivo**: en el modo tocar y por MIDI, una pista con granos abre una nube mientras se mantiene la tecla (la tecla elige el corte, o el pitch si la pista tiene uno solo); ver «Nube en vivo» en `docs/ESTADO.md`.
- **Mejoras sueltas** (fuera de las fases; detalle en «Mejoras sueltas» de `docs/ESTADO.md`): portapapeles del sistema en el editor (OSC 52, `wl-copy`/`wl-paste`), tracker entero en cuanto cabe, fundido en el salto de los bucles de corte (`crossfade` en `cortes:set`), la ayuda del modo tocar sin «Esc editor» con el editor fuera, la navegación del editor (Ctrl-Inicio/Ctrl-Fin, ir a la línea con Alt-G, buscar con Ctrl-F y F6/F7), el tutorial «Tu primera canción» (en su propio archivo; con un bajo `sh101`, una lane del filtro y la caja tocada como melodía) y los límites de cada novedad en la guía, el ratón también en el tracker, la vista de ocupación y el mezclador (elegir pista, mute y solo, y los cursores de edición y de relleno; `tui/src/app/panels.rs`, con la geometría de cada panel en `ui.rs`), `tui --edit` con una ruta que no existe, error salvo con `--new`, la nota de cada corte en cifrado americano (`cortes:note(i)`, en las marcas de la vista del sample y «Notas de los cortes» en la paleta), arrastrar con el ratón las marcas de los cortes en la vista del sample (se guarda como el ajuste de `,`/`.`), crear y quitar cortes desde la vista del sample (`n` y Supr en el modo tocar, clic derecho en la onda o en una marca) con zoom (`+`/`-`, Shift-←→, la rueda del ratón), cada ajuste de corte conserva su posición (los del proyecto se aplican todos a la vez) y mover un corte no suena (suena su tecla, espacio o un clic en la onda), botón de pánico (F12 silencia todas las voces sin tocar el secuenciador, Shift-F12 además las deja en pausa; límite de 64 voces entre pistas en `[voces] limite`, roba la más antigua), y grupos y macros (`group(nombre, pistas)` suma pistas y las pasa juntas por su cadena, volumen, pan y envíos, 4 grupos de un nivel; `macro(nombre, desde, hasta)` es una curva más que sigue un valor de 0 a 1 movido en vivo desde «Macros» en la paleta o un CC de `[midi] macros`).
- **Fase L hecha** (editor en ventana propia): el editor en otro proceso y otra terminal (`tui --edit x --split`, `tui --window-editor x`), unido a la principal por un socket Unix privado con un protocolo de líneas (si uno cae, el otro sigue); sin el editor, la principal llena la ventana con la forma de onda grande, el tracker con todas las pistas, el mezclador y la tabla del último análisis de choques; y el editor se saca y se vuelve a meter en caliente desde la paleta con su texto, cursor e historial.
- **Fase O hecha** (síntesis): núcleo en `engine::synth` (osciladores con PolyBLEP al doble y diezmados, unison, SVF y ladder de 4 polos, ADSR, glide, accent; voces fijas por pista, deterministas con semilla) y dos instrumentos, `synth("acid", {...})` y el subgrave limpio `synth("sub", {...})`, con accent (`!`) y slide (`/`) en la mini-notación; se toca como un instrumento (melody, pads, MIDI) y sus parámetros admiten `ramp`/`lfo`. Después, `sh101{...}`, un monosynth de bajo y lead (voz analógica: sierra y pulso, sub, ladder propio con `quality` 1 o 2, una envolvente, legato con prioridad a la última nota, portamento, LFO y deriva).
- **Fase P hecha** (sub-armónicos): el bloque sub de los sintetizadores (`sub`, `sub_div`, `sub_wave`, `sub_pan`, `sub_drive`, sacado de la fase del oscilador, también por paso con `{sub=...}`) y `subharm{...}`, seis capas que dividen la frecuencia (`div`) y el compás (`rdiv`, redisparo de envolvente sobre el reloj del compás).
- **Fase N hecha** (automatización y efectos por paso en todo): todo lo continuo admite `ramp()`/`lfo()` (también `:gain()`, el `:pan()` nuevo, `:send()` y los buses) y el registro dice qué se puede automatizar y por qué no lo demás (`api::Auto`, tabla «Qué se puede automatizar» de la guía); por golpe lo que la voz lee al nacer (`:pitch()` nuevo, `:formant()` y la forma de `cortes:set`); efectos por paso en la mini-notación (`3{lp=800,q=4}{crush=6}`, con rampas del golpe `{lp=200>4000:4}`; manda el paso sobre su corte); y el tracker editable (paleta: cursor, Enter escribe los efectos del paso en su token del código, en un paso de deshacer, y carga).
- **Fase S hecha** (transporte con atajos y cue): Ctrl-Espacio marcha/paro (editor, modo tocar y ventana del editor), Shift-Ctrl-Espacio o F9 fija el cue (el compás que suena, el del cursor del tracker parado, o la entrada con secciones) y la paleta lo borra; `Controller::start_at` arranca en un compás como si la canción hubiera llegado (vueltas, curvas, rampa de tempo, sección); `--from N` en el render. Para Shift-Ctrl-Espacio hace falta el protocolo de kitty o las teclas extendidas de tmux (ver la guía).
- **Fase Q a medias** (MPE y expresión por nota): bend, presión y CC74 por nota (`tui/src/expression.rs`, `[midi] mpe` y `bend`), `Controller::note_mod` y `synth_note_on_expr` en el motor (suavizado, sin reservas, solo en notas MIDI en vivo) y `pista:mpe{ bend = "pitch", pressure = "cutoff" }` en Lua. Falta comprobarlo con la salida de `aseqdump -p 24:0` del Seaboard y decidir el synth polifónico (la receta usa `subharm{ voices = 6 }`).
- **Fase R hecha** (editor con ratón y modo vim, fase 1): clic, arrastrar y rueda en el editor (`[editor] raton`), y `[editor] modo = "vim"` con Normal e Insert (`h j k l w b e 0 $ gg G`, `i a o O A I`, `x dd yy p P`, `u` Ctrl-R, `/ n N`, `:w`); Esc sigue llevando al modo tocar y `jk` sale de Insert (`salir_insert`).
- **Fase T hecha** (vista de ocupación por paso): una fila por pista y una columna por paso de una página de hasta 4 compases, leída de lo que toca el secuenciador (`TrackDef::bars` y el cursor de la fase M; sin cambiar el motor ni el protocolo), con `Σ` y los huecos resaltados (`tui/src/occupancy.rs`, F1 «Vista de ocupación», `[ocupacion]` y los colores `hueco` y `sugerencia`); rellenar un hueco con un cursor que escribe en el `.` de ese paso (un paso de deshacer) y sugerencias de relleno que usan el análisis de choques si lo hay.
- **Fase U hecha** (panel de dispositivos): la salida de audio cambia en caliente con un fundido de 5 ms sin perder el compás (el motor vive en una ranura atómica, `engine::device`; si el dispositivo se va, el anterior, el predeterminado o la salida nula); MIDI de entrada y salida por puerto o virtual (`tui/src/devices.rs`); `[dispositivos]` en `config.toml`, `--audio-device`, `--midi-in`, `--midi-out`, `--list-devices`; el panel desde la paleta o en su ventana (`tui --window-devices`, socket propio, protocolo 4; el 5 desde la fase V). Sin probar aún con PipeWire, auriculares ni un controlador de verdad.
- **Fase V hecha** (afinación, mandos y congelar): cortes en el cruce por cero (`zero_cross` en `slice`/`slice_grid`, también para `add`, `move` y los ajustes del proyecto); MIDI learn (`engine::knob`: tabla al compás, valor ya y suavizado, manda sobre una curva mientras se mueve; `script::learn`, por nombre, en el proyecto y en `[midi] mandos`; panel de dispositivos, protocolo 5); tonalidad de un sample (`engine::detect_key`, cromagrama y Krumhansl; panel desde la paleta y Ctrl-K en el explorador, que pone la raíz del instrumento o propone `key()`); tonalidad global (`key()`, `transpose()`, grados en `melody()`, pads en la tonalidad); la vista del sidechain (la reducción de cada EQ dinámico y compresor publicada con atómicos, `tui/src/sidechain.rs`); y congelar pista (`frozen()` al final del script, WAV tras los efectos con `Engine::set_tap`, reversible).
- **Fase W hecha** (piano roll editable): la pista seleccionada en una cuadrícula (notas o cortes en vertical, pasos en horizontal), de lo que toca el secuenciador (`tui/src/roll.rs`, puro; `TrackDef::melody` dice qué vueltas son de `melody()`), con duración, volumen, acordes, la escala de `key()`, variaciones y lo generado; medias celdas y cuadrantes según el zoom (`tui/src/roll_panel.rs`). Cada edición (teclas en `[roll.teclas]` y ratón, que nunca suena) se escribe solo en sus tokens si es seguro (texto cargado, con posición, cadena no compartida y que al releerlo diga lo que suena), en un paso de deshacer, y carga; preescucha con Espacio (el pánico la corta). Panel en la principal y en su ventana (`tui --window-roll`, socket propio, protocolo 6: la ventana solo dibuja y la principal aplica).
- **Edición de cortes** (sin fase propia; detalle en «Edición de cortes» de `docs/ESTADO.md`): variantes de duración sin copiar audio (`dur` en ms, figura o compases y `fit` `trim`/`stretch`/`loop`, en `cortes:set` y en las llaves `3{dur=1/8}`), el corte como instrumento (`cortes:instrument(i, {...})`, raíz por YIN con los cents corregidos, `keep` para no cambiar la duración), `bend`/`bend_time` y `rev` en tiempo real, y `warp`/`smear` precalculados al cargar con caché (`engine::deform`, viajan en el patrón y entran al compás) más `formant` por corte.
- **Cortes independientes y guardar su edición** (sin fase propia; detalle en «Cortes independientes y guardar su edición» de `docs/ESTADO.md`): lo editado en la onda (ratón, teclas, partir, unir, etiquetar) se escribe en el código, en la llamada que corta el sample, como `slice({ {inicio, fin [, "nombre"]}, ... })` en ms (un paso de deshacer; se guarda con Ctrl-S; antes iba solo al `.sampler`, que ya no guarda esos cortes), con `script::slice_call` y `tui/src/app/slices.rs`; cada corte tiene su inicio y su final (`Slices::from_ranges`, huecos y solapes): arrastrar un borde o el cuerpo no mueve a los vecinos, Alt o Ctrl mueve la marca compartida, `b` elige qué mueven `,` `.` `<` `>`; la onda sigue `ui::WaveLayout::hit`; en Lua, `move(i, ms, fin)` y `range(i)`.
- **La ventana aparte del editor abre el mismo proyecto** (sin fase propia; detalle en «La ventana aparte del editor abre el mismo proyecto» de `docs/ESTADO.md`): las ventanas aparte (editor, dispositivos, piano roll) se lanzan con la ruta canónica (`tui/src/target.rs`), no abren la plantilla ni otro archivo si no existe (error claro), el editor no manda su texto hasta el saludo de la principal ni pisa lo que ella tenga sin guardar, y no se une a la principal de otro script.
- **Lanes de automatización** (sin fase propia; detalle en «Lanes de automatización» de `docs/ESTADO.md`): `pista:auto(parámetro, puntos, { len, mode, offset, range, scale })` y `lane()` (una curva más, `Curve::Lane`, que apunta a una tabla fija del motor de 32 lanes de 64 puntos que entra al compás; las curvas se evalúan con un `CurveEnv`), con curvas `linear`/`exp`/`smooth`/`step` y modos `loop`/`once`/`hold`; panel «Lanes de automatización» con el cursor y el valor en vivo (un atómico por lane), que también sale con su unidad junto a cada parámetro automatizado en el panel de efectos, edición con el ratón que reescribe la tabla literal de `auto()` en el código, y grabación «touch» con un mando MIDI o el ratón.
- **Visualizaciones** (sin fase propia; detalle en «Visualizaciones» de `docs/ESTADO.md`): `pista:punchcard`/`pianoroll` (del modelo del piano roll, con el cursor), `scope` (disparo por cruce por cero, Braille), `spectrum` (FFT propia, bandas logarítmicas), `pitchwheel` y `spiral`, y `master()` con las de audio; el motor solo copia a una cola `rtrb` por pista y del master si están encendidas (por el lote, al compás), y todo lo demás va en la interfaz; `inline = true` junto al código, en la ventana del editor por el protocolo 7, ASCII y refresco en `[interfaz]`.
- **Rediseño de la interfaz** (sin fase propia; diseño en `docs/DISENO-TUI.md`, detalle en «Rediseño de la interfaz» de `docs/ESTADO.md`): rejilla pura por tamaños (`tui/src/layout.rs`: diminuta, pequeña, mediana y ancha) con cabecera, regla, pestañas (lo que no cabe, nunca encima de otro panel) y la referencia de teclas; barra de menús con F10 (`tui/src/menu.rs`, cada acción de la paleta en un menú por un `match` sin comodín); foco entre paneles con Alt-1…9 (`tui/src/focus.rs`, deducido de los cursores); scope y espectro del master sin pedirlos en Lua; ASCII de verdad sin Unicode; un fallo del dibujo nunca para el audio (`tui/src/safe_draw.rs`); protocolo 8 para la cabecera de la ventana del editor. Después, menos CPU en la interfaz: el espectro se calcula una vez por cuadro y el master solo se pide si se ve, el ratón que solo se mueve no redibuja, la FFT precalcula sus giros y en reposo no se redibuja lo mismo (ver «Consumo de la interfaz», abajo).
- **Explorador y editor abajo** (continuación del rediseño; diseño en la sección 12 de `docs/DISENO-TUI.md`, detalle en «Explorador y editor abajo» de `docs/ESTADO.md`): explorador de archivos a la izquierda desde 120 columnas (`tui/src/files.rs`, perezoso por carpeta; Alt-0 foco, Alt-E mostrar o abrir encima; abre `.lua`, `.sampler` y preescucha samples; crea, renombra y borra archivos), el editor abajo con su alto (borde arrastrable, Ctrl-Alt-↑↓, `[interfaz] alto_editor`), maximizado (Alt-Z) y flotante dentro de la ventana (Alt-W; `Grid::overlays`/`covered`: el ratón no llega a lo tapado), la onda con puntos Braille (`[interfaz] onda`), el espectro del master en su panel (`tui/src/viz/master.rs`, `[interfaz] espectro`: relleno, línea, barras o espectrograma) y submenús (`menu::rows`). Falta el popup de tecla líder.
- **Consumo de la interfaz** (detalle y cifras, de x86_64 y de ARM64, en «Consumo de la interfaz» de `docs/ESTADO.md`): el scope y el espectro del master no cuestan nada hasta pedirlos con Alt-M (menú Ver, paleta o Alt-7/Alt-8), y entonces son pestañas del dock a su ritmo (`[interfaz] visuales_hz`, 15); `[interfaz] bajo_consumo` baja el refresco a 15 y el de las visualizaciones a la mitad (la onda en barras, sin Braille); en ARM64, cuatro causas arregladas (diezmado, avisos de la notación, scope y RMS sin copiar, bloques). Se mide con `scripts/medir-consumo.py` (escenarios, tamaños, `--config`, `--teclas`), el banco `tests::consumo` y `scripts/comparar-audio.sh` (el audio bit a bit donde golden.rs no corre).
- **Límites del script** (de la auditoría, `docs/AUDITORIA.md`; detalle en «Límites del script» de `docs/ESTADO.md`): una carga se corta con su error, y sigue sonando lo anterior, si pasa de 256 MB de memoria del intérprete o de 2 s (hook cada 1000 instrucciones); los dos en `[script]` (`memoria`, `tiempo`).
- **Guardado atómico** (de la auditoría; detalle en «Guardado atómico» de `docs/ESTADO.md`): todo lo que escribe el usuario (script, proyecto, bloques, ranuras, configuración, historial, exportaciones) pasa por `script::atomic`: temporal en la misma carpeta, `sync_all` y renombrar; conserva permisos, sigue enlaces y limpia los huérfanos.
- **Arreglos de la segunda auditoría** (`docs/AUDITORIA-2.md`; detalle en «Arreglos de la segunda auditoría» de `docs/ESTADO.md`): un pánico al cargar es un error en el editor (`script::panics`); samples saneados al cargar (NaN e infinitos a 0, con aviso) y lo no finito en la mezcla a cero, con la pista callada (`scrub` en el motor); el límite de tiempo no se rodea (`script::sandbox`: `pcall`, `__gc`, patrones de `string`, sin `dofile`, `print` a los avisos) y `eval` corre en un hilo con vigilante; patrones de 32 KiB como mucho; quitar la última lane ya no libera en el callback; render a trozos de hasta 2 h en 16, 24 o 32 bits; FLAC, MP3, OGG y AIFF con `symphonia`. Sin arreglar: N12 (sample muy largo congela la interfaz) y N15 (con M2).
- **Comprobaciones que no hay que romper**:
  - `engine/tests/no_alloc.rs`: cero reservas en `Engine::process`; todo lo nuevo del motor entra ahí.
  - `script/tests/golden.rs`: huellas del audio (solo en x86_64); sin usar nada nuevo, idénticas bit a bit. Las cinco de antes de la fase G y cuatro más (sintetizadores, buses, lanes, grupos con macros).
  - En ARM64, donde golden.rs no corre: `scripts/comparar-audio.sh BINARIO_DE_ANTES` renderiza las recetas con los dos binarios y las compara bit a bit.
  - `script/tests/guide.rs`: la guía coincide con el registro (`cargo run -p script --example guia`) y sus bloques de Lua y las recetas de `scripts/ejemplos/` (los de la guía y los del tutorial) se ejecutan; todo enlace interno entre los dos archivos y dentro de cada uno apunta a un ancla que existe; el script del tutorial (bloques `lua entero`, `añade` y `cambia`) se reconstruye paso a paso y cada estado se carga. Las tablas de la paleta y de la configuración se reescriben con `ACTUALIZAR_GUIA=1 cargo test -p tui the_guide_lists_the_palette` (y `the_guide_lists_the_config`).
  - `script/tests/registry.rs`: cada función, efecto, parámetro y snippet del registro existe y funciona en Lua.
- **Ver la interfaz sin terminal**: `cargo test -p tui pantalla -- --ignored --nocapture` (y `pantalla_autocompletado`, `pantalla_paleta`, `pantalla_bloques`, `pantalla_choques`, `pantalla_ventanas`, `pantalla_acid`, `pantalla_ocupacion`, `pantalla_dispositivos`, `pantalla_sidechain`, `pantalla_roll`, `pantalla_lanes`, `pantalla_viz`, `pantalla_viz_inline`, `pantalla_tamanos`, `pantalla_ascii`, `pantalla_flotante`).
- **Audio de prueba**: `scripts/render-demo.sh` genera un break sintético y lo renderiza a `renders/` sin tocar la tarjeta (ver `docs/ESTADO.md`).
- **Pendiente conocido**: cpal avisa de un underrun al arrancar el stream (una vez, ~200 ms).

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
