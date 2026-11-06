# Auditoría del proyecto

Fecha: 6 de noviembre de 2026. Commit auditado: `c94d4f3` (`main`, tras unir
el trabajo de consumo de x86_64 y ARM64). Hecha en la laptop de desarrollo
(x86_64, Gentoo), en solo lectura: no se ha cambiado código, tests ni
configuración. Lo único nuevo es este informe y
`scripts/auditoria/sesion.py` (el flujo real de la sección 5.4).

Cada hallazgo dice cómo se comprobó: **lectura** (archivo y línea leídos),
**ejecución** (lo que se lanzó y lo que salió) o **herramienta**. Lo que no
está comprobado se marca como **sospecha**.

---

## 1. Resumen ejecutivo

**Estado general.** El núcleo de sampler de cortes (cargar, cortar,
secuenciar, efectos, exportar, proyectos, recargar el código en vivo) es
sólido y está bien probado. El motor de audio cumple sus reglas: no
reserva memoria en el callback (lo comprueba un asignador que cuenta), el
único `unsafe` del motor es correcto, el reloj no deriva y todo lo que
llega de Lua se valida antes de tocar el audio. Lo que preocupa no es la
corrección del núcleo, sino **el tamaño y la superficie que lo rodean**:
de las áreas que se pueden separar, las extras (piano roll, ventanas
aparte, visualizaciones, análisis de choques, ocupación, sintetizadores)
suman unas 16 000 líneas, el doble que los cortes, el secuenciador y los
efectos juntos (~8 200). Hay además tres objetos y archivos centrales
enormes (`App` con 132 campos, `script/src/lib.rs` con 7123 líneas en un
solo archivo, `api.rs` con 5173) y una API de 95 funciones y 299
parámetros. El núcleo no se ha degradado, pero cada función nueva cuesta
más de mantener que la anterior.

**Los 5 riesgos mayores**

1. ✅ *(resuelto en `26892f3`)* **Un script de Lua que reserva memoria sin parar tumba el programa
   entero, audio incluido** (A1). No hay límite de memoria, y el límite de
   tiempo solo se mira cada 10 000 instrucciones. Comprobado: llegó al
   tope de 6 GB que le puse y abortó a los 62,8 s, sin que saltara el
   límite de 2 s.
2. **La mezcla satura por defecto** (M1). El «limitador» es un recortador
   `tanh` por muestra. Con una canción de dos pistas, el 8,4 % de las
   muestras cae en la zona del recortador y la cabecera marca «SATURA»
   todo el rato.
3. **«1 compás» significa cosas distintas según la función** (M2):
   `ramp()` y `lfo()` usan siempre 16 pasos; las lanes y `dur`, el compás
   de la canción. En 3/4 o 7/8 las rampas se desfasan.
4. ✅ *(resuelto en `8f3b21a`)* **Guardar el script y el proyecto no es atómico** (M3): un corte de luz
   o el disco lleno a mitad de escritura deja el archivo truncado (el
   historial sí escribe a un temporal y renombra).
5. **Mantenibilidad** (M5): `App` (132 campos, `view()` de 482 líneas),
   `script/src/lib.rs` (7123 líneas) y `Engine::handle`/`mix_with` (~380
   líneas cada una) concentran casi todo; cada función nueva los engorda.

**Los 5 puntos fuertes**

1. **Disciplina del hilo de audio**: `engine/tests/no_alloc.rs` cuenta las
   reservas *y* las liberaciones del hilo de audio en un recorrido muy
   amplio (cada efecto, sintetizador, lane, grano, visualización, grupo,
   macro, mando, pánico, traspaso de dispositivo); cola de basura para no
   liberar en el callback; el único `unsafe` del motor
   (`engine/src/device.rs`) es correcto.
2. **Validación de entrada**: 14 casos de NaN e infinitos desde Lua (en
   el volumen, el pitch, el pan, los efectos, las rampas, los LFO, los
   envíos y los pasos) dan un error con su línea, sin que llegue nada al
   motor; 14 WAV raros o corruptos, sin un solo pánico.
3. **«Si el código falla, sigue sonando lo anterior»**: comprobado en vivo
   (sección 5.4) y por lectura: `ScriptHost::load` solo cambia de estado
   después de que el lote entra en el motor.
4. **Estabilidad en sesiones largas**: 8 minutos sonando con la interfaz,
   351 compases sin deriva, la memoria plana en 14,9 MB y la CPU estable.
   Contadores en `u64` y un redondeo por compás.
5. **Tests y documentación con disciplina**: 1272 tests (28 ignorados, a
   propósito), la guía generada del registro y sus ejemplos ejecutados,
   `cargo fmt` y `clippy -D warnings` limpios.

**Nota global: B−** (confianza media). Por área:

| Área | Nota | Por qué |
|---|---|---|
| Motor de audio | A− | Reglas cumplidas y comprobadas; falta un limitador de verdad (M1). |
| Cortes y secuenciador | B+ | Completo y probado; solo WAV (M4) y exportar solo a 16 bits (M6). |
| Script (Lua y API) | B− | Errores bien tratados, pero sin límite de memoria (A1), semántica de duraciones incoherente (M2) y un `lib.rs` de 7000 líneas. |
| Interfaz | C+ | Funciona y es estable, pero un `App` de 132 campos y 40 000 líneas para una TUI. |
| Tests | A− | Muchos, deterministas y con intención; pocos en el piano roll para su tamaño. |
| Documentación | B | Muy completa y casi siempre al día; algunos desfases (sección 6) y 12 500 líneas que pesan. |
| Foco de producto | C+ | El núcleo está bien; los extras son el doble de grandes y casi todos a medio probar con hardware. |

La confianza es media: revisé a fondo los caminos de riesgo (el hilo de
audio, la carga de Lua, los archivos, el guardado) y medí y ejecuté lo que
pude; no leí las 72 000 líneas una a una, ni pude probar con audio real ni
compilar para ARM64 (sección 9).

---

## 2. Inventario

**Crates** (líneas con los tests; «código» sin los archivos de tests):

| Crate | Archivos `.rs` | Líneas | Código | Tests (líneas) | `#[test]` | Ignorados |
|---|---|---|---|---|---|---|
| `engine` | 104 | 36 673 | 19 449 | 17 199 | 411 | 7 |
| `script` | 65 | 28 931 | 17 314 | 11 602 | 296 | 1 |
| `tui` | 130 | 55 362 | 39 986 | 15 376 | 565 | 20 |
| **Total** | 299 | 120 966 | 76 749 | 44 177 | 1272 | 28 |

Los 28 ignorados son a propósito y se lanzan a mano: las vistas de
pantalla (`pantalla_*`, para mirar la interfaz sin terminal; unas 18) y
las medidas de coste o de calidad (bancos: `medida_de_coste`,
`coste_en_el_hilo_de_audio`, `cpu_cost_of_eight_voices...`, el banco de
consumo de la interfaz; unas 10).

**Los 15 archivos más largos** (código, sin tests):

| Archivo | Líneas |
|---|---|
| `script/src/lib.rs` | 7123 |
| `script/src/api.rs` | 5173 |
| `tui/src/app.rs` | 5117 |
| `engine/src/engine.rs` | 4702 |
| `tui/src/ui.rs` | 2976 |
| `engine/src/synth/mod.rs` | 2393 |
| `tui/src/config.rs` | 1760 |
| `tui/src/roll.rs` | 1461 |
| `engine/src/sequencer.rs` | 1312 |
| `tui/src/link.rs` | 1296 |
| `tui/src/main.rs` | 1195 |
| `tui/src/app/split.rs` | 1181 |
| `tui/src/app/roll.rs` | 1163 |
| `engine/src/fx.rs` | 1131 |
| `tui/src/complete.rs` | 1130 |

Funciones más largas (herramienta: script de la auditoría sobre los
`fn`): `App::view` 482 líneas (`tui/src/app.rs:4552`), `add_methods`
467 (`script/src/lib.rs:6539`), `Engine::handle` 384
(`engine/src/engine.rs:2584`), `Engine::mix_with` 376 (`engine.rs:3722`),
otro `add_methods` 369 (`lib.rs:5429`), `ScriptHost::eval` 308
(`lib.rs:1107`), `Config::parse` 289 (`tui/src/config.rs:986`).
Estructuras: `App` con 132 campos, `ViewModel` con 57, `Engine` con 44.

**Dependencias directas**: `engine`: cpal, hound, rtrb, rubato, rustfft;
`script`: mlua (lua54, vendored); `tui`: libc, midir (opcional, por
defecto), ratatui, ratatui-textarea, regex, rtrb, toml. 113 crates
distintos en el árbol de dependencias normales (`cargo tree -e normal`).
Duplicados en dos versiones (`cargo tree -d`): `hashbrown` 0.16 y 0.17,
`syn` 2 y 3 (este, solo al compilar). `cargo machete`, `cargo udeps` y
`cargo audit` no están instalados: no hay escaneo de vulnerabilidades
(sección 9). A mano: todas las directas se usan (`regex` solo para
`regex::escape` en la búsqueda del editor, `tui/src/editing.rs:404`; la
feature `search` de ratatui-textarea ya depende de él, así que no añade
nada; `libc` solo para
`SO_PEERCRED`/`getpeereid` en `tui/src/link.rs:1014-1047`).

**Documentación**: `docs/GUIA.md` 5723 líneas, `docs/ESTADO.md` 3518,
`docs/PRIMERA-CANCION.md` 1407, `docs/ROADMAP.md` 888,
`docs/DISENO-TUI.md` 828, `CLAUDE.md` 147: 12 500 líneas.

**API de Lua** (registro `script/src/api.rs`): unas 95 funciones y
métodos, 9 efectos y 299 parámetros entre todos (funciones, efectos,
cortes, instrumentos, sintetizadores, lanes, granos, visualizaciones).

---

## 3. Hallazgos

Ordenados por severidad. No se ha encontrado ninguno **crítico** (nada que
pierda datos del usuario ni que rompa el audio por sí solo). «Esfuerzo»:
S (horas), M (un día o dos), L (más). «Audio»: si arreglarlo toca el hilo
de audio (entonces exige `golden.rs` y `no_alloc.rs`).

| Id | Área | Sev. | Archivo:línea | Descripción | Cómo se verificó | Impacto | Propuesta | Esf. | Riesgo del arreglo |
|---|---|---|---|---|---|---|---|---|---|
| A1 | Lua | **Alto** · ✅ resuelto en `26892f3` | `script/src/lib.rs:89`, `1108-1129` | Sin límite de memoria para el script (`set_memory_limit` no se usa en ningún sitio) y el límite de tiempo solo se comprueba cada `HOOK_INSTRUCTIONS = 10 000` instrucciones. Una instrucción cara (`string.rep`, concatenar, `table.concat` de cadenas grandes) hace mucho trabajo y reserva mucha memoria entre dos comprobaciones. | **Ejecución**: `while true do t[#t+1] = string.rep("x", 10000000) .. #t end` con `tui --script ... --render` y `RLIMIT_AS` de 6 GB: RSS de 6130 MB, 62,8 s y `SIGABRT` («memory allocation of 10000025 bytes failed»). El bucle vacío sí se corta a los 2,0 s y `string.rep(3e9)` da error al momento. | En la interfaz, la carga corre en el hilo principal: el proceso entero (con el audio) muere o la máquina entra en swap (12 GB). Un error al escribir un bucle, en directo, lo tumba todo. | `lua.set_memory_limit(...)` (p. ej. 256-512 MB; existe en mlua 0.12.2, `src/state.rs`) y el hook cada ~1000 instrucciones. Un test: el script que reserva da error y sigue sonando lo anterior. | S | Bajo (fuera del audio). |
| M1 | Audio / producto | **Medio** | `engine/src/limiter.rs:8-25`, `engine/src/engine.rs:4075-4086` | El «limitador suave» es un recortador sin memoria: lineal hasta 0,9 y `tanh` por encima, por muestra, sin reducción de ganancia ni anticipación. No hay margen por defecto: dos pistas de cortes normalizados pasan de 1,0 en el master. | **Ejecución**: render offline de 8 compases de una canción de dos pistas (break, filtro, envío al delay y bombo a 0,9): el 8,4 % de las muestras por encima de 0,9 (zona del `tanh`) y el 5,1 % por encima de 0,99. En la sesión en vivo (5.4), «LIMITADOR» y «SATURA» encendidos casi siempre. **Lectura**: `SATURA` es el pico > 1,0 *antes* del recortador (`tui/src/app.rs:4544`). | Distorsión audible por defecto en lo más básico (sumar un bombo a un break). El usuario ve un aviso permanente al que acaba por no hacer caso. | Decidir con Roi: margen por defecto (p. ej. −6 dB en el master o en cada pista) y/o un limitador de verdad (detector de pico, ataque y relajación, sin anticipación para no añadir latencia). | M | **Audio**: cambia `golden.rs` (hay que regenerar las huellas, a propósito). |
| M2 | Lua / semántica | **Medio** | `script/src/lib.rs:2331-2361` y `6348-6361`; frente a `script/src/lanes.rs:342-367` y `script/src/step_fx.rs:153-176` | «N compases» se traduce distinto según la función: `ramp()`, `lfo()` y `bpm_ramp()` usan `n * STEPS` (16 fijos, `engine/src/sequencer.rs:25`) porque `ramp`/`lfo` no ven el estado; las lanes (`len`) usan los pasos del compás de la canción y `dur` usa «compases del compás que suena». Son tres parseadores con sintaxis distinta (comas decimales, `ms`, figuras `n/d`, `"2bars"` pegado solo en `dur`). | **Lectura** de los tres. No ejecutado en 3/4 (la diferencia es aritmética directa del código). | En `meter(3, 4)`, `ramp(..., "1 compás")` dura 16 pasos = 1⅓ compases y se desfasa del compás; una lane de `len = "1 compás"` dura 12. Un usuario que mezcla los dos no entiende por qué no coinciden. | Un solo parseador de duraciones en `script` (o `engine::notation`), con el compás de la canción como en las lanes, y `ramp`/`lfo` resueltos al final de la carga (cuando ya se sabe el compás). Preguntar a Roi si «compás» debe ser siempre el de la canción. | M | Bajo en 4/4 (`golden.rs` no cambia, todas sus canciones son 4/4: **sospecha**, comprobarlo); cambia el resultado de canciones en otros compases. |
| M3 | Archivos | **Medio** · ✅ resuelto en `8f3b21a` | `tui/src/app.rs:1803`, `script/src/project.rs:424`, `script/src/blocks.rs:397` y `415` | Guardar el script (Ctrl-S), el proyecto y los bloques hace `fs::write` directo sobre el archivo. En cambio el historial (`tui/src/history.rs:119-120`) escribe a un temporal y renombra. | **Lectura**. | Un corte, un cuelgue o el disco lleno a mitad de escritura deja la canción truncada o vacía. El historial de versiones mitiga (hay copia de la última carga buena), pero no del texto sin cargar. | Una función `write_atomic` (temporal en la misma carpeta, `sync_all` y `rename`) para los cuatro sitios. | S | Bajo. |
| M4 | Producto / samples | **Medio** | `engine/src/sample.rs:41-60`; `CLAUDE.md:21` | Solo se cargan WAV (`hound`). `symphonia` no está en ninguna dependencia (`engine/Cargo.toml`), aunque CLAUDE.md lo da como el decodificador de WAV, MP3, FLAC y OGG. ESTADO (línea 16) y el tutorial (`PRIMERA-CANCION.md:107-117`, «conviértelo con `ffmpeg`») sí lo dicen bien. | **Lectura** y `grep` en los `Cargo.toml`. | Para un sampler, los samples llegan en FLAC, MP3, AIFF u OGG: un paso manual con `ffmpeg` por cada uno. | `symphonia` (Rust puro, portable) en `Sample::load`, por la extensión o el contenido; o, si no se quiere ahora, arreglar CLAUDE.md. | M | Bajo (fuera del audio; la decodificación es al cargar). |
| M5 | Mantenibilidad | **Medio** | `tui/src/app.rs` (struct `App`, 132 campos; `view()` en la línea 4552, 482 líneas), `script/src/lib.rs` (7123 líneas, un archivo), `engine/src/engine.rs:2584` y `3722` | Tres «objetos dios». `App` mezcla el editor, el transporte, el piano roll, las lanes, las visualizaciones, los dispositivos, MIDI, el explorador, la ocupación, la tonalidad, el sidechain, las ventanas... `lib.rs` tiene la evaluación, toda la API (dos `add_methods` de ~400 líneas), la validación y la conversión a lotes. | **Herramienta**: conteo de líneas y campos (sección 2); `clippy::pedantic` da 45 `too_many_lines`. | Cada función nueva toca los mismos archivos (los conflictos de la última fusión fueron en `app.rs`, `app/viz.rs`, `config.rs` y `layout.rs`); el estado duplicado entre submódulos de `App` es fácil de desincronizar. | Partir por áreas, sin cambiar el comportamiento: en `script`, la API por grupos (pistas, cortes, efectos, sintetizadores, lanes) en archivos propios; en `App`, subestructuras con su estado (`RollState`, `VizState`, `DevicesState`, `FilesState`...) como ya se hizo con `files`, `roll`, `occupancy`. | L | Medio (mucho código movido; los tests ayudan). Hacerlo por trozos, después de las demás tandas. |
| M6 | Exportar | **Medio** | `engine/src/offline.rs:38-55` | El render de la mezcla final (`--render`, la orden `exportar`) siempre escribe WAV de 16 bits enteros, con recorte y redondeo, sin dither. Ya existe `write_wav_float` (32 bits en coma flotante, línea 60), que solo usa congelar. | **Lectura**; **ejecución**: el render de la sección M1 es «formato 1/16 bits». | Para publicar o masterizar fuera se pierde resolución (y los colas largas de la reverb a −80 dB se cuantizan sin dither). | Opción de formato al exportar (`--bits 16|24|32f`, `exportar ... 24`), y dither TPDF en 16. | S | Bajo (no toca el hilo de audio; comprobar que `golden.rs` mide las muestras y no el WAV: **sospecha**). |
| B1 | Interfaz / código muerto | Bajo | `tui/src/layout.rs:187-188`, `208-209`, `265-266`, `414-415`; `tui/src/screen.rs:179-190` | `Grid::scope` y `Grid::spectrum` ya no los asigna nadie (desde que el scope y el espectro del master son siempre pestañas): siempre `None`, y el bucle de `screen.rs` los recorre para nada. | **Lectura** y `grep` de asignaciones (`g.scope =`, `g.spectrum =`: ninguna). | Confunde a quien lea el layout («¿dónde se colocan?»). | Quitar los dos campos, sus brazos en `area_of`/`rects` y los `assert!` de los tests que los miran. | S | Muy bajo. |
| B2 | Código raro | Bajo | `script/src/lib.rs:2497-2503` y `2526` | Tres comentarios de documentación apilados encima de `viz_flags`: los dos primeros describen otra función (el lote de la canción, «El lote que hace sonar `song`...»), y `song_batch` (línea 2526) se quedó sin documentar. | **Lectura**. | `cargo doc` y quien lea atribuyen a `viz_flags` lo que hace `song_batch`. | Mover esos dos comentarios a `song_batch`. | S | Ninguno. |
| B3 | Redundancia | Bajo | `tui/src/viz/spectrum.rs:1-6`, `28-60` | La interfaz tiene su propia FFT radix-2 «propia: Rust puro y sin dependencias», pero `rustfft` ya está enlazado a través de `engine` (`pitch.rs`, `slice.rs`, `key.rs`, `masking.rs`). | **Lectura**. | Dos FFT que mantener; la justificación del comentario ya no vale. | Usar `rustfft` (con el plan y los búferes creados una vez, como hace ahora el `Analyzer`) o dejarla y corregir el comentario. Medir antes con el banco. | S | Bajo (solo dibujo). |
| B4 | Tests | Bajo | tests de `tui/src/link.rs` y de `tui/src/tests/{split,swap,main_window,navigation,...}` | Los tests de las ventanas crean sockets bajo `target/tmp/...` del repo: si el repo está en una ruta larga, la ruta pasa de los 108 bytes de `sun_path` y fallan 28 tests con `path must be shorter than SUN_LEN`. La ruta real del programa no tiene el problema (`link.rs:1090-1106` recorta el nombre a 24 caracteres en `$XDG_RUNTIME_DIR`). | **Ejecución** (en la sesión de la rama `base-ligera`, con un worktree bajo `/tmp/claude-1000/...`: 28 fallos con ese mensaje; en `~/sampler`, 0). | Falsos fallos según dónde se clone el repo (un CI o un worktree). | Sockets de test en `std::env::temp_dir()` con un nombre corto. | S | Ninguno. |
| B5 | Archivos / UX | Bajo | `tui/src/app/files.rs:466-480` | Borrar un archivo desde el explorador se confirma con Enter (o `s`/`y`) y es `remove_file` directo, sin papelera. Solo archivos, nunca carpetas (bien). | **Lectura**. | Supr seguido de un Enter por reflejo borra un sample o un script sin vuelta atrás. | Confirmar solo con `s` (no con Enter) o mover a una papelera (`.sampler-papelera/`). Pregunta para Roi. | S | Ninguno. |
| B6 | Audio (diseño) | Bajo | `engine/src/engine.rs:4229-4284`, `29-37` | Si la cola de basura (128 entradas) se llena, el motor «olvida» la memoria (`mem::forget`) en vez de liberarla en el callback. Solo la vacía el hilo de la interfaz (`collect_garbage`, `tui/src/app.rs:4548`, y al mandar comandos). | **Lectura**. **Sospecha** del impacto (no medido): con la interfaz parada (p. ej. una carga de Lua de 2 s, o el caso A1), las voces que terminan con un sample que el motor ya no guarda podrían desbordarla. | Fuga de memoria pequeña en casos raros; nunca afecta al audio. Es una decisión deliberada y está comentada. | Medir con un test que llene la cola; si pasa en casos reales, más capacidad o vaciarla desde otro hilo. | S | **Audio** (si se cambia la cola). |
| B7 | Mensajes | Bajo | `script/src/lib.rs` (`slice_grid`) | Un sample de un solo frame con `slice_grid(4)` deja 0 cortes y el error dice «el corte 1 no existe ("d" tiene 0)», sin decir por qué tiene 0. | **Ejecución** (WAV de 1 frame). | Despiste menor ante un archivo casi vacío. | Avisar al cortar: «el sample es demasiado corto para 4 cortes». | S | Ninguno. |
| B8 | Dependencias | Bajo | `Cargo.lock` | `hashbrown` en 0.16 y 0.17; `syn` en 2 y 3. | **Herramienta**: `cargo tree -d`. | Algo más de tiempo de compilación y de binario; nada funcional. | Esperar a que se alineen al actualizar (no hace falta forzar). | S | Ninguno. |

**Por severidad**: crítico 0, alto 1, medio 6, bajo 8 (15 en total).

**Resueltos después de la auditoría**: A1 (`26892f3`, «Límites del
script»: 256 MB de memoria del intérprete y el tiempo cada 1000
instrucciones, en `[script]`; el script de la verificación se corta ahora
en 0,85 s con 275 MB, y el proceso sigue) y M3 (`8f3b21a`, «Guardado
atómico»: `script::atomic` para el script, el proyecto, los bloques, las
ranuras, la configuración, el historial y las exportaciones). Detalle en
esas secciones de `docs/ESTADO.md`. Queda de A1 lo que el hook no puede
cortar: una sola función de C muy larga sin reservar memoria (un patrón
de `string.find` con vuelta atrás).

### Lo que se revisó y salió bien (no son hallazgos)

- **Hilo de audio**: los `expect` del motor (`engine.rs:3353`, `3424`,
  `3425`, `3444`, `3484`, `3574`; `synth/mod.rs:2017`) dependen de
  invariantes que se cumplen: `voices` está validado en 1..=MAX
  (`instrument.rs:97`), `[voces] limite` en 1..=MAX
  (`engine.rs:2412-2416`, `config.rs:1146`), y `counted_voices` cuenta lo
  mismo que `steal_oldest` puede robar (`engine.rs:3457-3491`), así que los
  bucles de robo terminan. El único `println!`/`eprintln!` del motor está
  en el callback de *error* de cpal (`output.rs:73`), no en el de datos.
  Los `Mutex` de `device.rs:887-1018` son del dispositivo falso de los
  tests.
- **`unsafe`**: el motor tiene cuatro bloques, todos en `EngineSlot`
  (`engine/src/device.rs:104`, `116`, `205`, `459`): un `AtomicPtr` que el
  callback toma (lo deja nulo) y devuelve en cada bloque; quien lo
  encuentra nulo da silencio. El patrón da exclusividad (no hay ABA:
  nadie compara contra un valor anterior) y el `Arc` del `Renderer`
  mantiene viva la ranura mientras haya callbacks. Necesario para cambiar
  de dispositivo en caliente sin bloquear. La interfaz tiene tres, en
  `tui/src/link.rs:1014-1047` (`geteuid`, `getsockopt(SO_PEERCRED)`,
  `getpeereid`), para que solo el mismo usuario pueda conectarse al
  socket: necesarios y acotados. Los `unsafe` de `engine/tests/no_alloc.rs`
  son el asignador que cuenta.
- **Reloj y precisión**: `Transport` cuenta frames en `u64`
  (`sequencer.rs:1138-1166`) y calcula cada paso desde el inicio de su
  compás (`step_offset`, sin acumular redondeos); la posición en pasos y
  las fases de los LFO, en `f64` (`automation.rs:149-153`). En la sesión
  de 8 minutos, 21,2 compases cada 30 s a 170 BPM, lo exacto. A `u64`
  frames y 48 kHz, el contador da para millones de años.
- **Cotas de los bucles del audio**: los comandos por bloque, como mucho
  `COMMAND_CAPACITY = 64`; `start_at` con rampa, como mucho 64 compases
  (`bpm_ramp` valida «hasta 64», `lib.rs:1783-1790`).
- **Rampas y LFO**: `check_curve` (`automation.rs:238-256`) exige al menos
  un paso y un LFO entre 0,01 y 50 Hz: no hay divisiones por cero.
- **Lua que falla**: `ScriptHost::load` (`script/src/lib.rs:953-1025`)
  evalúa sin tocar el motor, y solo cambia `slots`, `song` y `macro_values`
  después de que `controller.apply(batch)` entra. Un error en cualquier
  punto deja lo que sonaba.
- **Archivos**: 14 WAV raros (8 bits, 24 bits, 32 bits enteros y en coma
  flotante, 6 canales, 1 Hz, 384 kHz, 0 Hz, 0 canales, vacío, truncado, un
  tamaño de datos falso de 2 GB, texto) no dan ningún pánico: o suenan con
  el pico correcto, o dan un error legible.
- **`unwrap`/`expect` en la interfaz y el script, fuera de los tests**: 15
  sitios, todos protegidos por una comprobación justo antes (lectura de
  cada uno). Los `let _ =` de la interfaz (~25) son limpieza o avisos
  opcionales (portapapeles, sockets que se cierran).
- **Colas**: los comandos (64), la basura (128), las notas MIDI de salida
  (512) y las de las visualizaciones son `rtrb` (SPSC sin bloqueos); al
  llenarse, el comando da `QueueFull` al hilo de control (la carga falla
  entera, no a medias) y las notas MIDI se descartan.
- **Estado del motor**: todo con atómicos `Relaxed`; una lectura puede
  mezclar dos bloques, y `snapshot` ya lo tiene en cuenta
  (`status.rs:341-344`). Solo lo lee la interfaz.
- **Tests sin comprobación**: 14 tests no tienen ningún `assert!` dentro,
  pero todos llaman a funciones que comprueban (`check_onsets`,
  `ramps_without_clicks`...): falsos positivos.
- **Registro único**: el autocompletado, la ayuda y el diagnóstico salen de
  `api.rs` (`tui/src/complete.rs`, 60 referencias a `api::`), y los
  parámetros de los 9 efectos del registro aparecen todos en el parser de
  `lib.rs` (comprobación por texto, no semántica).
- **Portabilidad**: sin `target_arch`, sin intrínsecos, sin `target-cpu`,
  sin suponer páginas de 4 KiB; `libc` solo en Unix (Linux y macOS).

---

## 4. Redundancias y código muerto

| Qué | Dónde | Qué hacer |
|---|---|---|
| Tres parseadores de duraciones con sintaxis y semántica distintas | `script/src/lib.rs:6348` (`parse_duration`), `script/src/lanes.rs:342` (`steps_of`), `script/src/step_fx.rs:153` (`dur_text`) | Unificar en uno (ver M2). Conservar el de las lanes, el más completo (compases del compás de la canción, pasos, figuras y ms). |
| Dos FFT | `tui/src/viz/spectrum.rs` (propia) y `rustfft` en `engine` | Ver B3. |
| Dos ADSR | `engine/src/instrument.rs:32` (`Adsr`, en forma cerrada por muestra, para los instrumentos de sample) y `engine/src/synth/env.rs:59` (`Env`, iterativa, con accent y legato) | **Justificado**: hacen cosas distintas (la de sample se evalúa en cualquier frame, para el time-stretch y el `keep`). Dejarlas; un comentario en cada una que explique por qué no es la otra. |
| `Grid::scope`/`Grid::spectrum` muertos | `tui/src/layout.rs:187-188` | Ver B1. |
| La documentación de `song_batch` encima de `viz_flags` | `script/src/lib.rs:2497-2503` | Ver B2. |
| Varios filtros | `engine/src/fx.rs:672` (`Biquad`, efectos de pista y EQ), `engine/src/synth/filter.rs` (`Svf`, `Ladder`, `AnalogLadder`, `OnePoleHp`) | **Justificado**: los de los sintetizadores son de otra familia (con resonancia y saturación propias, al doble). No es duplicado. |
| `note_name` | `engine/src/pitch.rs:42` y `script/src/patterns.rs:288` | **No es duplicado**: el de `script` delega en el del motor. |
| Formas de hacer lo mismo en la API | `synth("sh101", {...})` y `sh101{...}`; `synth("sub", ...)`/`"subbass"`; `bar`/`bars`/`compás`/`compases`/`step`/`paso` en las duraciones | Es azúcar deliberada y documentada; no la quitaría, pero la guía podría enseñar una sola forma. |
| Dependencias | `regex` (solo `regex::escape`) | Lo pide la búsqueda de `ratatui-textarea` (feature `search`): se queda. |

No se encontraron `#[allow(dead_code)]` sin motivo, ni parámetros del
registro de efectos sin lector, ni ajustes de `[interfaz]` sin uso (las
tablas de la guía y de la configuración se comprueban contra el código en
`the_guide_lists_the_config` y `the_guide_lists_the_palette`).

---

## 5. Evaluación de producto

### 5.1 Qué es núcleo y qué es extra

Líneas de código (sin tests) y tests por área, **aproximadas**: se
asignaron por archivo y por nombre de test. `engine.rs`, `script/src/lib.rs`,
`api.rs`, `app.rs`, `ui.rs` y el resto de la interfaz son comunes a todas
y van aparte (42 131 líneas y 495 tests).

| Área | Líneas | Tests | Madurez | Papel |
|---|---|---|---|---|
| Cortes y samples (detección, rejilla, YIN, deformaciones, stretch, interpolación) | 2585 | 162 | Completo (sin MP3/FLAC, M4) | **Núcleo** |
| Secuenciador y notación (pasos, secciones, variantes, efectos por paso) | 3539 | 109 | Completo | **Núcleo** |
| Efectos, buses y automatización | 2087 | 63 | Completo (falta un limitador de verdad, M1) | **Núcleo** |
| Bloques, proyecto, historial, congelar | 1699 | 37 | Completo | Apoyo |
| Granos | 481 | 9 | Completo | Apoyo |
| Lanes de automatización | 2855 | 44 | Completo (la grabación «touch» sin probar con un mando real) | Apoyo |
| Grupos y macros | 350 | 27 | Completo | Apoyo |
| MIDI, MPE y mandos | 2291 | 59 | **Parcial** (MPE a medias, sin probar con el Seaboard ni con hardware) | Apoyo / extra |
| Dispositivos de audio en caliente | 2805 | 25 | **Parcial** (sin probar con PipeWire, auriculares ni un controlador real) | Apoyo |
| Sintetizadores (acid, sub, subharm, sh101) | 3854 | 82 | Completo, sin validar de oído | Extra (útil para bajos) |
| Visualizaciones | 2380 | 41 | Completo (apagadas por defecto) | Extra |
| Piano roll | 3978 | 29 | Completo, con **pocos tests para su tamaño** (1 por cada 137 líneas, frente a 1 por cada 16 en los cortes) | Extra |
| Análisis (choques de frecuencia, tonalidad, sidechain) | 1942 | 44 | Completo | Extra |
| Ventanas aparte (editor, piano roll, dispositivos: sockets y 8 versiones del protocolo) | 2759 | 28 | Completo | Extra |
| Ocupación | 1013 | 18 | Completo | Extra |

En lo que se puede separar: núcleo ~8 200 líneas, apoyo ~10 500, extra
~15 900. **Los extras son el doble que el núcleo**, y casi todo lo que
está a medias o sin probar con hardware está fuera del núcleo.

### 5.2 Complejidad para el usuario

Una canción mínima necesita cinco cosas: `bpm`, `sample`, `slice`, `track`
y `steps` (más la mini-notación: número de corte, `.`, y los sufijos `'`,
`^`, `@`, `r`). Eso está bien. Pero la API completa son ~95 funciones y
métodos y **299 parámetros**; la guía tiene 5723 líneas y el tutorial
1407. El riesgo no está en que sea difícil empezar, sino en que el
autocompletado, la ayuda y la paleta (117 acciones) enseñan todo a
la vez. Parámetros que dudo que use alguien que hace remixes de breaks:
los seis `rdiv` y `div` por capa del `subharm`, `pwm_src`/`pwm_amt` y la
deriva del `sh101`, las opciones del espectrograma, el análisis de
tonalidad del sample con Krumhansl, los grupos de un nivel (para 8
pistas), y la mayoría de los modos de las lanes (`once`/`hold`,
`offset`, `scale`).

### 5.3 Recomendaciones

**Lo que falta de verdad para un sampler** (comprobado que no existe):

1. Límite de memoria de Lua (A1): un error en directo no debería poder
   tumbar el audio.
2. Margen o limitador de verdad en el master (M1).
3. Cargar FLAC, MP3, AIFF y OGG (M4).
4. Exportar a 24 bits o a coma flotante (M6).
5. Guardar de forma atómica (M3).

**Lo que ya existe** (comprobado; no hace falta añadirlo): exportar a
WAV (`--render`, la orden `exportar`), guardar y cargar proyectos
(`.sampler`, `script/src/project.rs`), deshacer en el editor, historial de
versiones del script (una por carga buena, `tui/src/history.rs`),
estabilidad en una sesión larga (5.4), recarga al compás y «sigue sonando
lo anterior».

**Aparcar** (dejar como está, sin funciones nuevas ni mantenimiento
activo):

- **Las ventanas aparte del piano roll y de los dispositivos** (y su
  protocolo): la del editor sí tiene sentido para dos monitores, pero
  cada ventana nueva es un proceso, un socket y una versión de protocolo
  más. Quitarlas serían unos cientos de líneas (`roll_window.rs` 193,
  `devices_window.rs` 169, más su parte de `link.rs` y de `split.rs`; no
  lo medí exacto) y sus tests; aparcarlas no cuesta nada.
- **MPE**: a medias hasta que llegue el Seaboard; no seguir sin el
  hardware.
- **El análisis de choques en la interfaz**: es útil como orden
  (`--analyze-masking`), pero su panel y las sugerencias insertables
  añaden estado a `App`.

**Simplificar**:

- Las duraciones (M2): un parseador y una semántica.
- `App` y `script/src/lib.rs` (M5), antes de la próxima función grande.
- La guía: una sección «lo esencial» de una página delante del mapa
  completo, y el resto como referencia.

**Eliminar**: nada sin hablarlo. Si hiciera falta recortar de verdad, el
candidato es el **piano roll editable** (~4000 líneas, 29 tests, y una
ventana propia): para un sampler de cortes, el tracker editable, la vista
de ocupación y `melody()` cubren casi lo mismo. Coste de quitarlo: ~4000
líneas y sus atajos, la entrada del menú y la sección de la guía; riesgo
bajo (es una vista sobre el modelo; el motor no se entera).

### 5.4 Un flujo real

Con `scripts/auditoria/sesion.py` (nuevo en esta auditoría): un break
sintético, una canción con cortes, pasos, un filtro, un envío al delay y
un bombo; `tui --edit` en una pseudo-terminal de 160×45, sin
`--allow-audio` (salida nula a ritmo real), con un emulador de terminal
mínimo para leer la pantalla.

1. **Al abrir**: suena sola (compás 3 a los 5 s), 14 MB de memoria.
2. **Un error de Lua guardado** (una línea `= = 1` al final y Ctrl-S): el
   compás sigue avanzando (6 → 8), la cabecera marca `✗ error l.13` y el
   panel, `cancion.lua:13: syntax error near '='`; en el disco queda el
   texto con el error (Ctrl-S guarda y luego carga: es lo esperado).
3. **Arreglado** (borrar la línea y Ctrl-S): el error se va y sigue
   sonando.
4. **8 minutos sonando**: 351 compases sin saltos ni deriva (21,2 cada 30
   s, lo exacto a 170 BPM), la memoria plana en 14,9 MB, el hilo de la
   interfaz entre 6,4 % y 7,2 % y el resto ~3,5 %.

**Lo que sorprende**: la cabecera marca «LIMITADOR» y «SATURA» casi todo
el rato con una canción tan simple (M1). Y el render del ejemplo de la
API de CLAUDE.md llega también a pico 1,0.

**Lo que se rompe**: nada en este flujo. Lo único que tumba el programa
es el script que reserva sin parar (A1).

---

## 6. Desfases entre documentación y código

| Dónde | Qué dice | Qué hay | Propuesta |
|---|---|---|---|
| `CLAUDE.md:21` (pila) | «Decodificación: `symphonia` (WAV, MP3, FLAC, OGG) y `hound`» | Solo `hound`, solo WAV (`engine/Cargo.toml`, `engine/src/sample.rs:41`). ESTADO (línea 16) y el tutorial sí lo dicen bien. | Corregir CLAUDE.md o hacer M4. |
| `docs/DISENO-TUI.md:549`, `699`, `771`, `823` | El espectro del master «fijo» a la derecha de las pestañas en el tamaño mediano y en la columna del ancho | Desde «Visualizaciones apagadas por defecto», el scope y el espectro son siempre pestañas y no se ven sin pedirlos (Alt-M). La guía y ESTADO sí están al día. | Una nota arriba de la sección 12, o actualizar esos párrafos. |
| `tui/src/viz/spectrum.rs:3` | La FFT «propia: Rust puro y sin dependencias» | `rustfft` ya está enlazado (por `engine`). | Ver B3. |
| `script/src/lib.rs:2497-2503` | Documentación del lote de la canción encima de `viz_flags` | Es de `song_batch` (`lib.rs:2526`). | Ver B2. |
| La guía, `ramp()`/`lfo()` | «"1 compás"» sin decir qué compás | Siempre 16 pasos, aunque haya `meter()`; las lanes usan el compás de la canción. | Ver M2. |
| `CLAUDE.md` (paso 4, «con limitador suave al final») y la guía («LIMITADOR» en la cabecera, «el limitador») | Un limitador | Un recortador suave por muestra (`engine/src/limiter.rs`, que sí lo dice bien: «recortador suave sin memoria»). | Llamarlo «recortador suave» en la guía y en la cabecera, o hacer M1. |
| `CLAUDE.md` («Pendiente conocido: cpal avisa de un underrun al arrancar») | — | No se puede comprobar sin la tarjeta (sección 9). | — |

Lo que sí coincide (comprobado): los bloques de Lua de la guía y del
tutorial y las recetas de `scripts/ejemplos/` se ejecutan
(`script/tests/guide.rs`, verde); las tablas de la paleta y de la
configuración se comparan con el código; la sección de la API se genera
del registro. Las cifras de consumo de ESTADO están al día tras la última
fusión, y cada tabla dice si es de x86_64 o de ARM64.

---

## 7. Plan de arreglos propuesto

Tandas pequeñas e independientes, en el orden recomendado. «Audio» marca
las que tocan el hilo de audio (exigen `golden.rs` y `no_alloc.rs`).

| Orden | Tanda | Hallazgos | Riesgo | Tests necesarios | Audio |
|---|---|---|---|---|---|
| 1 | ✅ Límite de memoria y hook más frecuente en Lua (`26892f3`) | A1 | Bajo | Un script que reserva sin parar da error a tiempo y sigue sonando lo anterior; uno que tarda da error a los 2 s aunque haga instrucciones caras. | No |
| 2 | ✅ Guardar de forma atómica (`8f3b21a`) | M3 | Bajo | Guardar sobre un archivo de solo lectura o en una carpeta llena no deja el archivo a medias; el temporal no queda tirado. | No |
| 3 | Limpieza menor | B1, B2, B4, B7 y los desfases de la sección 6 | Muy bajo | Los que ya hay (los de la rejilla cambian sus `assert!`). | No |
| 4 | Exportar a 24 bits y coma flotante, y dither en 16 | M6 | Bajo | Render a cada formato: longitud, pico y que el de 32f vuelva bit a bit al leerlo. | No (es el render offline; comprobar que `golden.rs` mira las muestras y no el WAV) |
| 5 | Una sola forma de leer duraciones | M2 | Bajo-medio | Las mismas duraciones en `ramp`, `lfo`, lanes y `dur`, en 4/4 y en 3/4; `golden.rs` igual (todo en 4/4). | No (el motor recibe pasos igual) |
| 6 | Cargar FLAC, MP3, AIFF y OGG | M4 | Bajo | Un archivo de cada formato (generado en el test) carga con la misma longitud y el mismo pico que su WAV. | No |
| 7 | Margen y limitador del master | M1 | Medio | Medidas antes y después (muestras saturadas, pico, LUFS aproximado); `no_alloc.rs` con el limitador nuevo; **`golden.rs` cambia y hay que regenerarlo a propósito**. | **Sí** |
| 8 | FFT única | B3 | Bajo | El banco `tests::consumo` antes y después; los tests de las visualizaciones. | No |
| 9 | Partir `script/src/lib.rs` y `App` | M5 | Medio | Todos (no cambia el comportamiento); hacerlo por trozos, un módulo por commit. | No |
| — | Cola de basura | B6 | — | Primero medir si se llena en un caso real; solo si pasa, cambiarla. | Sí, si se toca |

---

## 8. Preguntas para Roi

1. **La saturación del master (M1)**: ¿es a propósito (el carácter sucio
   de los breaks) o prefieres margen por defecto y un limitador de verdad?
   Cambiarlo cambia cómo suenan todas tus canciones.
2. **«1 compás» en compases que no son 4/4 (M2)**: ¿debe ser siempre el
   compás de la canción (como en las lanes), o los 16 pasos de siempre?
3. **Exportar (M6)**: ¿16 bits te basta, o quieres 24 bits o coma
   flotante para masterizar fuera?
4. **Formatos (M4)**: ¿te llegan samples en FLAC o MP3 a menudo, o ya los
   conviertes siempre a WAV?
5. **Borrar en el explorador (B5)**: ¿confirmar solo con `s`, o una
   papelera?
6. **El límite de memoria de Lua (A1)**: ¿256 MB, 512 MB? ¿Alguna canción
   tuya genera tablas muy grandes a propósito?
7. **¿Usas el piano roll, las ventanas del piano roll y de los
   dispositivos, y el panel de choques?** Si no, son lo primero que
   aparcaría (5.3).
8. **El MPE**: ¿sigue en pie lo del Seaboard, o lo aparcamos?

---

## 9. Lo que no pude revisar

- **El audio real**: CLAUDE.md prohíbe a Claude Code usar `--allow-audio` o
  abrir la tarjeta. Todo va con la salida nula o con el render offline:
  sin comprobar el underrun al arrancar, los xruns con PipeWire, la
  latencia real ni el cambio de dispositivo en caliente con hardware. Ni
  si algo «suena bien», que es criterio tuyo.
- **ARM64**: la toolchain de esta máquina no tiene la librería estándar
  de `aarch64-unknown-linux-gnu`, así que no hice `cargo check` para ese
  objetivo. La última fusión trae commits compilados y medidos en el
  Asahi, y no hay nada no portable en el código (sección 3).
- **Vulnerabilidades y dependencias sin uso**: `cargo audit`,
  `cargo machete` y `cargo udeps` no están instalados; la revisión de
  dependencias fue a mano.
- **Cobertura de líneas**: sin `cargo llvm-cov` ni `tarpaulin`; la
  cobertura del hilo de audio se juzgó leyendo `no_alloc.rs`, no midiendo.
- **Hardware MIDI**: ni el Seaboard ni un controlador; el MIDI de entrada
  y salida, solo por sus tests.
- **Línea a línea**: el código son ~77 000 líneas sin tests. Revisé a fondo
  los caminos de riesgo (el callback y lo que alcanza, la carga de Lua, la
  lectura de archivos, el guardado, los sockets, las colas, el `unsafe`)
  y por muestreo el resto. Puede haber errores lógicos en áreas que solo
  miré por encima (el piano roll, la ocupación, la escritura de vuelta al
  código desde el tracker y las lanes).
- **Un emulador de terminal real**: el consumo y el flujo se midieron en
  una pseudo-terminal; lo que gasta kitty (o el que uses) pintando no se
  mide aquí.

---

## Apéndice: herramientas

- `cargo fmt --all -- --check`: limpio.
- `cargo clippy --workspace --all-targets -- -D warnings`: limpio (de la
  última fusión).
- `cargo clippy --workspace --all-targets -- -W clippy::pedantic`
  (informativo): 3743 avisos en total; en las librerías y el binario,
  engine 1051, script 439 y tui 844. Por categoría (los diez primeros):
  `cast_precision_loss` 832, `cast_possible_truncation` 681,
  `cast_lossless` 445, `must_use_candidate` 302, `cast_sign_loss` 247,
  `float_cmp` 228, `missing_errors_doc` 144, `format_push_string` 96,
  `cast_possible_wrap` 81, `too_many_lines` 45. Casi todo son
  conversiones numéricas esperables en DSP y en la interfaz; los que
  merecen mirarse son `too_many_lines` (M5) y los `float_cmp` fuera de los
  tests.
- `cargo test --workspace`: 1244 pasan, 0 fallan, 28 ignorados (de la
  última fusión, en `~/sampler`).
- `scripts/auditoria/sesion.py --minutos 8`: la sección 5.4.
- Renders offline con `tui --script ... --render`: los casos de NaN e
  infinitos, los WAV raros, la saturación del master y el ejemplo de
  CLAUDE.md.
