# Segunda auditoría independiente

Fecha: 9 de noviembre de 2026. Commit auditado: `d6844d6` (`main`). Hecha en
la laptop de desarrollo (x86_64, Gentoo), en solo lectura: no se ha tocado
código, tests, `Cargo.toml` ni configuración. Lo nuevo: este informe,
`scripts/auditoria/` (seis guiones) y `auditoria-fuzz/` (un crate aparte,
fuera del workspace).

> **Alcance.** Hechos y verificados: los hallazgos, el fuzzing, el mutation
> testing (una muestra), la cobertura, las medidas del limitador y del
> compás y la comparación con la primera auditoría. **No hecho** (sección
> 8): el repaso línea a línea de `ESTADO.md` y `DISENO-TUI.md` frente al
> código, los parámetros del registro sin efecto y los ajustes sin lector.

> **Arreglos (10 de noviembre de 2026).** Después del informe se
> arreglaron 15 de los 17 hallazgos, cada uno en su commit, con un test
> que falla sin el arreglo; en la tabla de la sección 3, ✅ y el commit.
> También M4 (`0f90a04`), M6 (`25dcd4b`), B1 (`37d5e8d`), B2 (`8808fc6`) y
> B4 (`8859146`) de la primera auditoría. Sin arreglar: **N12** (cargar un
> sample muy largo congela la interfaz; necesita cargar sin esperar en
> `App`, que va con M5) y **N15** (va con M2, fuera de alcance). `golden.rs`
> sigue idéntico en sus cinco huellas (más cuatro nuevas, N10) y las 27
> recetas deterministas suenan bit a bit igual que antes de los arreglos
> (`scripts/comparar-audio.sh`). Decisiones, mecanismos y costes en
> «Arreglos de la segunda auditoría» de `docs/ESTADO.md`.

Orden seguido: primera pasada propia sin leer `docs/AUDITORIA.md`; solo
después se leyó y se comparó (sección 2).

Confianza de cada hallazgo: **verificado** (ejecutado o con traza),
**probable** (leído, sin ejecutar) o **sospecha**.

---

## 1. Resumen ejecutivo

**Nota global: C+** (confianza media-alta en los hallazgos, media en la
nota). La primera auditoría
dio B−. La bajo porque su conclusión central, «la corrección del núcleo no
preocupa, lo único que tumba el programa es A1», no se sostiene: en unas
horas de fuzzing aparecieron tres formas más de matar o colgar el programa
en directo con entradas normales, un silencio permanente de la mezcla y una
liberación de memoria en el hilo de audio que `no_alloc.rs` no ve.

Tu miedo («demasiadas funciones sin base sólida») es **medio cierto**. El
motor aguanta muy bien (29 000 canciones mutadas con órdenes en vivo, sin un
pánico ni un NaN nacido dentro; bit a bit igual en el compás 65 536 que en
el 0). Lo que no es sólido es **la frontera**: lo que entra (samples, Lua)
no tiene red, y un pánico en la carga mata el audio.

**Cinco riesgos**

1. Un script corriente de siete líneas mata el programa (N1): `section` +
   `variant` con una pista nueva y sin `arrange`. No es hostil: es lo que
   hay a medio escribir.
2. Un WAV en coma flotante con un NaN mata el programa si hay que
   resamplearlo (N2) o deja **toda la mezcla en silencio para siempre** si
   no (N3). Sin aviso en los medidores.
3. El límite de tiempo de Lua (A1) se rodea con un `pcall` dentro de un
   bucle, con `string.find` y con `__gc` (N4): la interfaz se cuelga y no
   se puede guardar.
4. Ningún pánico de la carga está atrapado (solo el dibujo): cualquier
   `unwrap` futuro en `script` tumba el audio (N5, estructural).
5. `golden.rs` solo protege la API de antes de la fase G: nada de
   sintetizadores, buses, lanes, grupos ni granos tiene huella (N10).

**Cinco puntos fuertes**

1. El motor, bajo fuzzing: 0 pánicos, 0 muestras no finitas, 0 fuera de ±1
   y, salvo N6, 0 llamadas al asignador en el callback.
2. Sin deriva: el render desde el compás 65 536 (unas 25 h a 170 BPM) es
   idéntico muestra a muestra al del compás 0, en cuatro canciones.
3. La validación de parámetros de Lua: 30 casos límite (NaN, infinitos,
   tamaños absurdos) dan error con su línea.
4. Parsers de texto: 514 000 patrones al azar, 176 000 proyectos, 22 000
   bibliotecas de bloques y 4 716 `config.toml` hostiles, sin un fallo en
   el binario real.
5. El guardado atómico (M3) está bien hecho y no se rodea.

---

## 2. Comparación con la primera auditoría

**Coincidencias (reproducidas con datos)**

- **M1**: reproducida exacta su cifra, 8,401 % de frames sobre 0,9 y
  5,136 % sobre 0,99, con su misma canción. Pero ver «discrepancias».
- **M2**: confirmada **ejecutando** (ella solo leyó). En `meter(3, 4)`,
  `gain(ramp(0, 1, "1 compás"))` vuelve a empezar en los pasos 16 y 32, no
  en el 12 y el 24 (`script/src/lib.rs:6428`).
- **M4, M6**: `symphonia` no está en `Cargo.lock`; exportar es de 16 bits.
- **M5**: confirmada y **peor**: `App` tiene ahora 137 campos (eran 132),
  `install_api` 955 líneas, `App::view` 483; 65 funciones de más de 100
  líneas de 2 527. `clippy::too_many_lines`: 44.
- **A1 y M3 resueltos**: sí en su caso original. Todas las cargas pasan por
  un único `Lua::new_with` (`script/src/lib.rs:1147`) con los límites por
  defecto, también `--render`, `--script`, el análisis de choques y la
  ventana del editor. M3: no hay escrituras fuera de `script::atomic` salvo
  crear y renombrar en el explorador (atómicas por sí mismas). **Pero A1 se
  rodea** (N4, N7).
- B1 y B2 siguen ahí (`tui/src/layout.rs:187-188`,
  `script/src/lib.rs:2569-2575`).

**Discrepancias (afirmaciones suyas que no se sostienen)**

| Decía | Qué hay | Veredicto |
|---|---|---|
| «Lo único que tumba el programa es A1» | N1 y N2 lo matan; N4 lo cuelga | **Refutada** |
| «15 `unwrap` en interfaz y script, todos protegidos» | `script/src/lib.rs:2678` no lo está | **Refutada** |
| «14 WAV raros, sin un solo pánico» | Cierto para sus 14; con un NaN dentro, pánico | **Sobrevalorada** |
| «El motor no reserva en el callback» (punto fuerte 1) | Una liberación en `engine.rs:3024` | **Sobrevalorada** |
| «La mezcla satura por defecto» | 8,4 % solo con dos pistas cuyos bombos coinciden; en las 26 recetas, 0,12 % de media y 0,4 % como mucho | **Generalizó el peor caso** |
| Tests: A− | `golden.rs` cubre poco (N10), hay un test que falla con carga (N11) | **Sobrevalorada** |
| B6 (la cola de basura se desborda) | Cada elemento sale de un comando y la cola de comandos es de 64: la cota del comentario se cumple | Sospecha sin base (no reproducida) |

**Qué cubrió bien**: inventario, mantenibilidad, la lectura del `unsafe`,
el flujo real, las preguntas. **Qué dejó fuera**: toda prueba dinámica
(fuzzing), el contenido de los samples, la superficie del sandbox, qué
protege de verdad `golden.rs`, la interfaz ante un script que no vuelve.

**Hallazgos nuevos**: 17 (N1 a N17).

---

## 3. Hallazgos

Esfuerzo: S (horas), M (un día o dos), L (más). «Audio»: el arreglo toca el
hilo de audio (exige `golden.rs` y `no_alloc.rs`).

| Id | Área | Sev. | Archivo:línea | Descripción | Verificación | Conf. | Impacto | Propuesta | Esf. | Riesgo del arreglo |
|---|---|---|---|---|---|---|---|---|---|---|
| N1 ✅ `6df2488` | Script | **Crítico** | `script/src/lib.rs:2678` | `song.tracks.iter().find(...).unwrap()`: con `section` + `variant(sección, "B", ...)` que crea una pista nueva y **sin `arrange`**, esa pista tiene ranura pero no está en `song.tracks`. | Fuzzer (`auditoria-fuzz lua`, primer hallazgo); reproducido con `tui --script` y **en la interfaz** (`tui_hostil.py --solo variante_sin_arrange`): proceso muerto. | Verificado | El programa muere a mitad de canción al guardar un script a medio escribir. | Devolver un `ScriptError` en vez de `unwrap`; test con esa canción. | S | Bajo |
| N2 ✅ `34daf55` | Samples | **Alto** | `engine/src/sample.rs:64-91`; `rubato` `synchro.rs:185` | Un WAV flotante con NaN o infinito a otra frecuencia que el motor: `rubato` hace `unwrap` de un error. | Fuzzer `wav` (4 978 pánicos de 352 402); reproducido con el binario real y **en la interfaz** (`wav_nan_44k`: muerto). | Verificado | Un archivo raro mata el audio. Probable también al preescucharlo en el explorador (misma carga). | Sanear en `from_interleaved`: rechazar o poner a cero lo no finito y acotar la magnitud. | S | Bajo (no es el hilo de audio; `golden.rs` igual) |
| N3 ✅ `1a07e20` | Audio | **Alto** | `engine/src/sample.rs:64-91`; `engine/src/limiter.rs:14` | El mismo WAV a la frecuencia del motor carga. Un solo NaN que suene envenena la reverb y la mezcla: silencio total desde ahí, también en las demás pistas. `soft_limit` convierte NaN en 0 y los medidores no lo ven. | Render offline de 4 compases: compás 0 con audio, compases 1 a 3 **todo ceros**; lo mismo con compresor. | Verificado | Silencio permanente en directo, sin pista de por qué. Solo se arregla reiniciando. | El saneado de N2 lo corta de raíz. Además, una guarda en el master: si el bloque trae algo no finito, reiniciar buses y filtros y avisar por `Status`. | S + M | La guarda: **Audio** |
| N4 ✅ `533a1cb` | Lua (A1) | **Alto** | `script/src/lib.rs:1166-1187` | El error del hook se atrapa con `pcall`: `while true do pcall(function() while true do end end) end` no acaba nunca. Igual `string.find` con vuelta atrás (C, sin hook) y un `__gc` con bucle (Lua apaga los hooks en los finalizadores). | `hostil.py`: 45 s sin volver en los tres. En la interfaz: no responde a F1, hilo al 50-100 %. | Verificado | Interfaz colgada para siempre: no se puede guardar ni parar. El audio sigue. | `pcall`/`xpcall` propios que relancen el error del límite; `setmetatable` que rechace `__gc`; y evaluar en un hilo aparte con vigilante, que es lo único que cubre las funciones de C. | M | Bajo-medio |
| N5 ✅ `4ef5011` | Estructura | **Alto** | `tui/src/safe_draw.rs:69` (único `catch_unwind`); `tui/src/app.rs:1888` | Solo el dibujo atrapa pánicos. La carga y los eventos, no: N1 y N2 son dos casos de una clase. | Lectura + N1, N2. | Verificado | La regla 5 («sigue sonando lo anterior») no vale para un pánico. | `catch_unwind` alrededor de `host.load`: un pánico pasa a ser un error en el editor. | S | Bajo |
| N6 ✅ `c955640` | Audio | Medio | `engine/src/engine.rs:3024` | `(!lanes.is_empty()).then_some(lanes)` evalúa su argumento siempre: con la tabla vacía, el `Box<Lanes>` se suelta ahí, **en el callback**. Pasa al quitar la última lane de una canción que tenía (`script/src/lib.rs:2632`). | Asignador que cuenta dentro de `Engine::process` en el fuzzer, con traza: `Box<Lanes>::drop` ← `then_some` ← `Engine::step`. | Verificado | Rompe la regla 1. `no_alloc.rs` solo prueba tablas no vacías. | `if lanes.is_empty() { self.discard_lanes(lanes); None } else { Some(lanes) }`, y ese caso en `no_alloc.rs`. | S | **Audio** (sin cambio de sonido) |
| N7 ✅ `2802ea2` | Lua (A1) | Medio | `script/src/patterns.rs:11-20` | La memoria de Rust no cuenta para el límite: `steps(("1 "):rep(2e7))` (40 MB de cadena) llega a 1,6 GB y 4,2 s antes de dar el error de «necesita 16 pasos». Lineal: 330 MB con 4e6. | `hostil.py --solo steps_memoria`. | Verificado | Con 250 MB de cadena (bajo el límite de Lua), unos 10 GB: OOM. | Rechazar por longitud del texto antes de partirlo. | S | Bajo |
| N8 ✅ `d8fe85b` | Cortes | Medio | `engine/src/slice.rs:10`, `27` (`LOG_K`, `MIN_FLUX`) | La detección depende del nivel absoluto: el mismo break da 8 cortes a 0 y −3 dB, 7 a −6 dB y 4 a −12 y −20 dB. | Ejecución (el mismo WAV escalado, `slice(8)`). La causa (el umbral), probable. | Verificado | Un sample grabado bajo se corta mal y la canción falla con «el corte 5 no existe». | Normalizar por el pico antes de detectar. | S-M | Cambia cortes de samples bajos; `golden.rs` igual si su break está al mismo nivel (comprobar) |
| N9 ✅ `b1946fb` | Lua | Medio | `script/src/lib.rs:1147-1150` | La librería base queda entera: `dofile` y `loadfile` leen cualquier archivo como Lua (y enseñan parte en el error); `print` escribe en la salida estándar y **ensucia la pantalla**. `io`, `os`, `require`, `debug`, `package` y `coroutine` no existen, y `load` rechaza bytecode (bien). | `hostil.py` (globales); `tui_hostil.py --solo print`: texto ajeno en el panel del mezclador. | Verificado | `print(x)` para depurar, lo primero que se le ocurre a cualquiera, rompe el dibujo. `dofile` contradice «el script es uno solo» y el vigilante no ve esos archivos. | `print` al mensaje de la interfaz; quitar `dofile` y `loadfile`. Documentar qué hay. | S | Bajo |
| N10 ✅ `c0cec13` | Tests | Medio | `script/tests/golden.rs:57-134`, `151` | 5 canciones, todas de antes de la fase G: pasos, filtro, bitcrush, `every`, `chance`, swing, humanize, fill, melody. Sin huella: envíos, compresor, distorsión, EQ, sintetizadores, lanes, rampas, grupos, macros, granos, stretch, warp, secciones. En ARM64 no comprueba huellas. | Lectura y recuento de la API usada. | Verificado | Una regresión de sonido en un sintetizador o un bus pasa `golden.rs`. | Huellas de las 26 recetas (ya se renderizan en `comparar-audio.sh`). | S | Ninguno |
| N11 ✅ `0a82cb9` | Tests | Medio | `script/tests/lua.rs:257` | `assert!(took < time + ms(400))`: con la máquina cargada tardó 972 ms y falló. `cargo test` para en el primer binario que falla: no corrió ningún test de `tui`. | Ejecución (1 fallo con otros procesos en marcha; no repetido en vacío). | Verificado (una vez) | Falsos fallos; y una suite de 12 min 21 s, de los que `automate_all.rs` son 336 s y `no_alloc.rs` 130 s. | Margen proporcional o medir instrucciones, no tiempo. | S | Ninguno |
| N12 (sin arreglar) | Samples | Medio | `script/src/lib.rs` (`sample`, `stretch`); `tui/src/app.rs:1888` | La carga de samples es síncrona en el hilo de la interfaz, sin tope y fuera del límite de tiempo: un WAV de 20 min (201 MB) tarda 12 s y ocupa 1 256 MB; con `stretch`, 95 s. | Ejecución con `--render`. En la interfaz, probable (misma ruta). | Verificado / probable | La interfaz se congela ese tiempo. | Tope de duración con aviso, o cargar en otro hilo. | M | Bajo |
| N13 ✅ `25dcd4b` | Exportar | Bajo-medio | `tui/src/main.rs:109` | El render tiene un tope de 600 s. | `--bars 2052`: «render demasiado largo». | Verificado | No se puede exportar un set de más de 10 minutos. | Escribir por bloques al disco y subir el tope. | M | Bajo |
| N14 ✅ `3a5b05d` | Interfaz | Bajo-medio | `tui/src/app.rs:2639-2643` | `live_note_off` quita la nota de `held` y tira el resultado de `note_off` (`let _ =`): si la cola de 64 comandos está llena, la nota se queda colgada sin reintento. | Lectura. No reproducido. | Probable | Nota pegada tras una ráfaga (MPE, cambio de dispositivo); F12 la quita. | Reintentar en la vuelta siguiente. | S | Bajo |
| N15 (sin arreglar (va con M2)) | Lua | Bajo | `script/src/lib.rs:6428`; `meter()` | Con `meter(3, 4)` una pista sigue pidiendo 16 pasos: hay que poner `:length(12)` a mano. | Ejecución. | Verificado | Sorpresa; va con M2. | Que `meter()` fije la longitud por defecto. | S | Cambia canciones con `meter` |
| N16 ✅ `c77c5a1` | Varios | Bajo | `script/src/patterns.rs:77`; `engine/src/sample.rs:47`; `engine/src/slice.rs:247` | Tres desbordes de enteros: `rotate(p, math.mininteger)`, un WAV con 0 bits por muestra, `corte = ... 1e308` en el proyecto. | Fuzzer con `overflow-checks`. Con el perfil real, 0 pánicos. | Verificado | Pánico solo en compilaciones de depuración (los tests); en release dan la vuelta sin romper. | `checked_`/`saturating_`. | S | Ninguno |

| N17 ✅ `f779eab` | Tests | Bajo-medio | `engine/src/arrangement.rs:212`, `219` | `resume` (dónde sigue la canción al recargar un arreglo a mitad) no tiene test para sus dos bordes: el compás que ya no cabe en la entrada y la vuelta al principio desde la última. Con `<` cambiado por `<=` o `%` por `+`, pasan todos los tests de `engine`, `script` y `tui` sobre secciones y variantes. Igual `euclid` con `length` mayor que `n` (`script/src/patterns.rs:60`). | `cargo mutants` y, en una copia, los tests de las tres crates con cada mutante. | Verificado | Es código que corre **en el hilo de audio** (`engine.rs:4314`): con el segundo mutante, el índice de entrada se sale de la línea de tiempo. El código de hoy está bien; nada lo protege. | Dos tests: recargar acortando la sección que suena, y hacerlo en la última entrada. | S | Ninguno |

Menores, sin número: los mensajes de error repiten el token entero aunque
mida 300 000 caracteres; el nombre de una pista admite secuencias de escape
de terminal y 1 MB de largo; `Conn::receive` (`tui/src/link.rs:967-990`) no
acota lo que lee si la línea ya trae un salto (solo el mismo usuario puede
conectarse); los tres crates no declaran `license` en su `Cargo.toml`.

**Por severidad**: crítico 1, alto 4, medio 7, bajo-medio 3, bajo 2. Total
17, **todos nuevos** respecto a la primera auditoría. De la primera siguen
abiertos M1, M2, M4, M5, M6 y B1-B5, B7, B8.

---

## 4. Fuzzing, miri, mutation testing y cobertura

No hay `rustup` ni nightly en esta máquina: **ni `cargo fuzz` ni miri**. El
fuzzing es en estable, con generador y mutaciones propios
(`auditoria-fuzz/`). Instalé `cargo-mutants`, `cargo-machete`,
`cargo-audit` y `cargo-llvm-cov` en `~/.cargo/bin`.

```sh
cd auditoria-fuzz
cargo run --release -- <wav|notacion|proyecto|bloques|lua|motor> [segundos] [semilla]
cargo run --profile real -- ...      # como el binario de verdad (sin overflow-checks)
cargo run --release --example ranura_exhaustivo
cargo run --release --example send
scripts/auditoria/hostil.py           # Lua hostil contra --render
scripts/auditoria/tui_hostil.py       # lo mismo en la interfaz, en una pseudo-terminal
scripts/auditoria/fuzz_config.py --segundos 90
scripts/auditoria/fuzz_protocolo.py --segundos 100
scripts/auditoria/limitador.py [canción.lua ...]
scripts/auditoria/sesion-larga.py canción.lua
```

| Objetivo | Iteraciones | Resultado |
|---|---|---|
| Notación, notas, duraciones, puntos de lanes, cabeceras de bloque | 514 042 en 60 s | 0 pánicos (aparte de `rotate(i64::MIN)`, N16) |
| Cargador de WAV | 352 402 en 60 s | 4 978 pánicos: N2 (`rubato`) y N16; 2 295 con el perfil real, todos N2 |
| `.sampler` (cargar, aplicar, guardar y releer) | 176 629 en 40 s, perfil real | 0 pánicos; 27 con `overflow-checks` (N16) |
| Biblioteca de bloques | 22 439 en 30 s | 0; nunca escribe fuera de su carpeta |
| Canciones mutadas (176 de partida: recetas, guía y tutorial) | 16 738 + 9 093 | 1 pánico (N1); 0 con `overflow-checks` |
| Lo mismo con órdenes en vivo y bloques de 1 a 4 096 frames | 14 600 | 0 pánicos, 0 no finitas, 0 fuera de ±1, 0 motores envenenados |
| Reservas, liberaciones y `realloc` dentro de `Engine::process` (asignador que cuenta, con traza) | 38 680 con la canción limpia cargada detrás; 24 780 sin ella | Al quitar las lanes, siempre N6 (1 012 veces; las 16 trazas, `engine.rs:3024`). Sin ese caso: **0** en 7 337 canciones cargadas, con órdenes en vivo y recargas |
| `config.toml` | 4 716 en 90 s | 0 fallos; nunca impide cargar |
| Protocolo de ventanas (v8) contra una principal real | 1 050 mensajes, 695 conexiones | 0 muertes, 0 cuelgues. Poco profundo: la principal corta al primer mensaje malo |

- **`unsafe`**: 4 bloques en `engine/src/device.rs` (104, 116, 205, 459),
  todos de `EngineSlot`, y 3 en `tui/src/link.rs` (1014, 1028, 1047). Sin
  miri. `Engine` y `Controller` son `Send` (`examples/send.rs`; el
  `AtomicPtr` no lo exige, así que nadie lo comprobaba). El protocolo de la
  ranura, por enumeración exhaustiva: 98 209 estados, 26 534 entrelazados
  de tres hilos, nunca dos con el motor y el puntero siempre vuelve.
- **loom: no concluyente.** `loom` 0.7.3 da por perdido el puntero también
  en un modelo mínimo de dos hilos con todo `SeqCst`
  (`examples/loom_min.rs`), una ejecución imposible (el control con `Mutex`
  pasa). Trata `SeqCst` como `AcqRel` y da falsas alarmas. No lo cuento ni
  a favor ni en contra.
- **Sesión larga**: `sesion-larga.py` con cuatro canciones (cortes, acid,
  subharm, LFO): diferencia 0 de 32 767 desde los compases 64 a 65 536.
  Límite: `--from` coloca el motor, no acumula estado; el render continuo
  de horas lo impide N13.
- **Mutation testing** (`cargo mutants`, muestra): hay 19 188 mutantes y
  cada uno cuesta de 2 a 5 minutos, así que es una muestra de 47, no un
  barrido. Sin `automate_all.rs`, `no_alloc.rs` ni `guide.rs` (tardan 336,
  130 y 91 s cada vez).

  | Crate | Archivos | Probados | Cazados | Escapan | No compilan |
  |---|---|---|---|---|---|
  | `engine` | `limiter.rs`, `arrangement.rs`, `metronome.rs`, `lane.rs` (1/8) | 26 | 18 | 6 | 2 |
  | `script` | `atomic.rs`, `project.rs`, `patterns.rs` (1/14) | 21 | 20 | 1 | 0 |

  Cazados: 38 de 45 viables (84 %). Los 6 de `engine` son todos de
  `arrangement::resume`; con los tests de `script` y `tui` cae uno de los
  tres que probé a mano y escapan dos (N17). El de `script` es `euclid`.
  El limitador, el metrónomo, las lanes, el guardado atómico y el proyecto:
  todo cazado. Los tests comprueban de verdad; el hueco es concreto.

  ```sh
  cargo mutants -j 1 --test-package engine -f engine/src/limiter.rs \
    -f engine/src/arrangement.rs -f engine/src/metronome.rs \
    -f engine/src/lane.rs --shard 1/8 --timeout 200   # más un --cargo-test-arg por test
  ```
- **Cobertura** (`cargo llvm-cov --workspace --ignore-run-fail`, con el
  `llvm-cov` 22 del sistema): 91,9 % de las líneas, 93,0 % de las funciones.
  `engine` 93,7 %, `script` 94,8 %, `tui` 90,0 %. Lo peor: `tui/src/main.rs`
  7,2 % (los modos de la línea de órdenes y el bucle principal no tienen
  test: `--render`, `--script`, `--from`, las ventanas), `engine/src/device.rs`
  65,1 % (el backend de cpal, sin tarjeta), `tui/src/app/roll.rs` 80,2 %.
  La cobertura alta no evitó N1: la línea se ejecuta, pero nunca con esa
  entrada. Es la diferencia entre cobertura y fuzzing.
- **Clippy**: `cargo clippy --workspace --all-targets`, limpio. Con
  `-W clippy::pedantic` (librerías y binario), 2 346 avisos: engine 1 051,
  script 445, tui 850. `cast_possible_truncation` 483,
  `cast_precision_loss` 390, `must_use_candidate` 303, `cast_lossless` 249,
  `cast_sign_loss` 191, `missing_errors_doc` 146, `cast_possible_wrap` 77,
  `format_push_string` 64, `too_many_lines` 44, `float_cmp` 29 (5 en
  `engine/src/fx.rs`). `cargo fmt --check`: limpio.
- **Dependencias**: `cargo audit`, 0 avisos en 301 crates. `cargo machete`,
  ninguna sin usar. `cargo tree -d`: `hashbrown` 0.16/0.17 y `syn` 2/3.
  `cargo deny` no está; por `cargo metadata`, todas las licencias son
  permisivas (MIT, Apache-2.0, Zlib, ISC, BSD, Unlicense, WTFPL),
  compatibles con la AGPL-3.0 del proyecto.
- **Tests**: 1 287 `#[test]`, 30 ignorados (pantallas y medidas, a
  propósito). La tanda de base paró en el fallo de N11 con 554 pasados. En
  la de cobertura (que no para): **1 256 pasan, 1 falla (N11, otra vez),
  30 ignorados**. N11 ha fallado las dos veces que la máquina tenía otra
  carga. Frágiles además: 21 `thread::sleep` en 6 archivos de test
  (sockets y dispositivos) y unas 30 coordenadas de pantalla fijas.

---

## 5. Evaluación de producto

### El limitador (decisión 1)

Medido antes del limitador (bajando el volumen de las pistas a un cuarto,
que con efectos lineales escala la mezcla igual; el método reproduce el
8,401 % exacto):

| Canción | Pico antes del limitador | Frames sobre 0,9 con el master a 0 dB | a −3 dB | a −6 dB |
|---|---|---|---|---|
| La de la primera auditoría (break + bombo a 0,9) | 1,808 (+5,1 dBFS) | 8,401 % | 2,182 % | 0,004 % |
| El ejemplo de `CLAUDE.md` | 1,686 (+4,5 dBFS) | 6,434 % | 1,524 % | 0,000 % |
| Las 26 recetas | de 0,34 a más de 1,74 | 0,119 % de media, 0,395 % como mucho, rachas de 2,3 ms | | |

Una pista sola apenas roza el umbral (0,17 %: el break tiene el pico en
0,968). Satura cuando **dos pistas golpean a la vez**. No es «por defecto»,
pero sí en el ejemplo canónico.

**Propuesta**: 6 dB de margen en el master por defecto (un ajuste, con
`master():gain()` para quien quiera el sonido de ahora) y dejar el
recortador como red. Es lo más barato y quita el 99,9 % de la saturación.
Un limitador de verdad, después y solo si lo echas de menos de oído.
**Impacto en `golden.rs`**: las 5 canciones entran en la zona del
recortador (4 de ellas entre el 4 % y el 7 % de los frames), así que
**cualquier** cambio de margen o de curva cambia las 5 huellas. Hay que
regenerarlas a propósito. Aviso: `amen.wav` es el break sintético; con tus
samples las cifras serán otras (`limitador.py` acepta tus canciones).

### «Compás» en 3/4 y 7/8 (decisión 2)

Hoy: `ramp`, `lfo` y `bpm_ramp`, 16 pasos fijos; lanes, `dur`, `arrange` y
`--bars`, el compás de `meter()`; la longitud de pista, 16 salvo
`:length()` (N15). **Recomendación**: «compás» = el de `meter()` en todas
partes, y «pasos» para quien quiera 16 fijos. Es lo que ya hacen la línea
de tiempo y la barra de estado, que es lo que ves. En 4/4 no cambia nada.

### Complejidad de la API

95 funciones (31 globales, 32 métodos de pista, 10 de cortes). Las recetas
01 a 08 usan **11**: `bpm`, `sample`, `slice`, `slice_at`, `track`,
`steps`, `gain`, `fx`, `every`, `chance`, `reverse`. Las 26 recetas, 32; el
tutorial, 28. Unas 60 no salen en ninguna receta. **Capa mínima
propuesta**: esas 11 más `send`, `section` y `arrange`, como primera página
de la guía y primer nivel del autocompletado; el resto, detrás.

### Por área

Líneas sin tests, aproximadas (por archivo; `engine.rs`, `lib.rs`, `api.rs`,
`app.rs` y `ui.rs` son comunes y no están). La madurez es lo que vi aquí:
fuzzing, cobertura y lo que queda sin probar con hardware.

| Área | Líneas | `#[test]` | Madurez | Papel | Qué haría |
|---|---|---|---|---|---|
| Cortes y samples | 4 167 | 273 | Alta en el motor; floja en la entrada (N2, N3, N8, N12, solo WAV) | Núcleo | **Invertir aquí**: es lo que define un sampler y donde están tres de los cinco riesgos |
| Secuenciador y notación | 4 781 | 184 | Alta (514 000 patrones sin fallo) | Núcleo | Dejar; unificar «compás» (M2, N15) |
| Efectos y mezcla | 3 239 | 103 | Alta; el master sin margen (M1) | Núcleo | Dejar; decidir M1 |
| Sintetizadores | 3 854 | 82 | Estables bajo fuzzing; **sin huella** (N10) y sin validar de oído | Apoyo | Dejar; darles huellas |
| Lanes, automatización y mandos | 2 973 | 86 | Alta; un fallo real (N6) | Apoyo | Dejar; arreglar N6 |
| Grupos y macros | 350 | 27 | Alta, y pequeña | Apoyo | Dejar |
| Visualizaciones | 1 462 | 41 | Completa, apagada por defecto | Extra | Dejar: cuesta poco y no molesta |
| Piano roll | 3 519 | 29 | **La menos probada**: un test por cada 121 líneas (los cortes, uno por cada 15); `app/roll.rs` al 80 % | Extra | **Aparcar** (sin funciones nuevas). Quitarlo: unas 3 500 líneas, riesgo bajo (el motor no se entera), pero no lo haría sin saber si lo usas |
| MIDI, MPE y dispositivos | 2 805 | 63 | Parcial: `device.rs` al 65 %, nada probado con hardware | Apoyo | **Aparcar el MPE** hasta tener el Seaboard; probar el cambio de dispositivo con PipeWire antes de fiarse |
| Ventanas aparte | 2 268 | 35 | Robustas en lo que probé (1 050 mensajes hostiles) | Extra | Dejar la del editor; **no añadir más** (cada una es un socket y una versión de protocolo) |

**Mi lectura.** No sobran funciones: sobra confianza en la frontera. Las
áreas extra están razonablemente hechas y no son las que fallan. Lo que
falla está en el camino por el que pasa *toda* canción (cargar un sample,
evaluar el script), que es justo donde menos se ha invertido desde el paso
5. Simplificar no arregla eso; las tandas 1 a 3 del plan sí, y son unos
tres días. Coste de no hacer nada: un directo que se cae por un archivo o
por una línea a medio escribir.

**Flujo real** (cargar, cortar, secuenciar, efectos, guardar con error,
sesión larga): funciona, y el error de Lua guardado no para el audio (lo
repetí con `tui_hostil.py`: `bucle` da su error a los 2 s y el compás
sigue). Lo que sorprende: `print` ensucia la pantalla (N9), `meter()` no
cambia la longitud de las pistas (N15), un break bajo se corta distinto
(N8), y un `pcall` en un bucle largo deja la interfaz muerta (N4).

### Qué falta de verdad (comprobado que no existe)

Red ante pánicos en la carga (N5); saneado de samples (N2, N3); FLAC, MP3 y
OGG (M4); exportar a más de 16 bits (M6) y de más de 10 minutos (N13).
Existe y funciona: proyectos, deshacer, historial de versiones, guardado
atómico, recarga al compás.

---

## 6. Plan de arreglos

Tandas pequeñas e independientes, por riesgo para el directo.

| Orden | Tanda | Hallazgos | Esf. | Audio |
|---|---|---|---|---|
| 1 | El `unwrap` de las variantes y `catch_unwind` en la carga | N1, N5 | S | No |
| 2 | Sanear los samples al cargar | N2, N3 (raíz) | S | No |
| 3 | Lua: `pcall` propio, sin `__gc`, sin `dofile`/`loadfile`, `print` al mensaje, tope de longitud en los patrones | N4 (parte), N7, N9 | M | No |
| 4 | La tabla de lanes vacía, y su caso en `no_alloc.rs` | N6 | S | **Sí** (sin cambio de sonido) |
| 5 | Huellas de las 26 recetas; el test del tiempo; los dos bordes de `resume` | N10, N11, N17 | S | No |
| 6 | Guarda de no finitos en el master | N3 (red) | M | **Sí** |
| 7 | Evaluar Lua y cargar samples fuera del hilo de la interfaz | N4 (resto), N12 | M-L | No |
| 8 | Detección independiente del nivel | N8 | S-M | No (cambia cortes) |
| 9 | «Compás» único y `meter()` con su longitud | M2, N15 | M | No |
| 10 | Margen del master | M1 | S | **Sí** (regenerar `golden.rs`) |
| 11 | Nota colgada, exportar largo, desbordes | N14, N13, N16 | S-M | No |

Las tandas 1 y 2 son las que haría antes de tocar en directo otra vez.

---

## 7. Preguntas para Roi

1. ¿6 dB de margen en el master por defecto? Tus canciones sonarán 6 dB
   más bajas hasta que subas el master.
2. ¿«Compás» = el de `meter()` en todas partes?
3. ¿Usas `dofile` o `loadfile` en algún script? Si no, los quito.
4. ¿Qué debe hacer `print`: ir al mensaje de la interfaz o a un registro?
5. Un sample con NaN: ¿rechazarlo con un error o cargarlo con esas
   muestras a cero y un aviso?
6. ¿Cuánto dura lo más largo que quieres exportar?
7. ¿Cargas alguna vez archivos de muchos minutos (mezclas enteras)?

**Lo que se decidió al arreglar** (la opción más conservadora; cámbialo si
no te vale):

- 3: `dofile` y `loadfile`, **quitados** (ni la guía ni las recetas los
  usaban). Si los usas, dímelo y vuelven con una ruta limitada a la
  carpeta del script.
- 4: `print` va a los **avisos de la carga** (el mensaje del editor), como
  mucho 20 líneas.
- 5: un sample con NaN se **carga con esas muestras a 0 y un aviso**.
- 6: el render llega a **dos horas** (se escribe a trozos).
- 7: sigue abierta (N12, sin arreglar).

**Preguntas nuevas tras los arreglos:**

8. `meter()` y la longitud de las pistas (N15) va con M2: ¿«compás» es el
   de `meter()` y una pista sin `:length()` mide eso? Cambia las canciones
   con `meter()`.
9. El vigilante abandona una carga atascada en una función de C, pero el
   hilo sigue gastando un núcleo hasta que acaba (no se puede matar sin
   riesgo). ¿Te vale, o prefieres que la interfaz avise para guardar y
   reiniciar?
10. `exportar` y `--render` siguen en 16 bits **sin dither** por defecto
    (los archivos salen bit a bit como antes). ¿Quieres dither por
    defecto en 16 bits? Cambiaría todos los WAV exportados.
11. Los patrones de `string.find`, `match` y `gsub` con dos o más
    cuantificadores sobre cadenas largas se rechazan si podrían tardar
    demasiado. ¿Haces algo así en tus scripts (procesar texto largo)?

---

## 8. Lo que no pude revisar y por qué

- **Documentación frente a código, a fondo**: solo comprobé lo que tocaba
  mis hallazgos. `CLAUDE.md:21` sigue diciendo `symphonia` (no está);
  `DISENO-TUI.md:549` y `699` siguen con el espectro «fijo» (ya es una
  pestaña); la guía no dice que `ramp`/`lfo` cuentan 16 pasos, ni el tope
  de 600 s del render, ni qué hay de la librería base de Lua. No repasé
  `ESTADO.md` línea a línea ni las cifras de consumo.
- **Parámetros del registro sin efecto y ajustes sin lector**: no lo hice.
  `registry.rs` y los tests de la guía lo cubren en parte.
- **B3 a B8 de la primera auditoría**: sin volver a comprobar.
- **El mutation testing es una muestra** (47 de 19 188) y deja fuera
  `engine.rs`, `sequencer.rs`, `fx.rs` y los sintetizadores.
- **miri y `cargo fuzz`**: no hay nightly. **loom**: no concluyente.
- **ARM64**: no está la librería estándar de `aarch64-unknown-linux-gnu`;
  `cargo check --target` falla al empezar. En el código no hay
  `target_arch`, intrínsecos ni bytes en orden nativo.
- **Audio real, PipeWire, MIDI**: prohibido o sin hardware.
- **El protocolo de ventanas**, solo por encima (1 050 mensajes), y solo el
  socket del editor; no los del piano roll ni los dispositivos.
- Ninguna salvaguarda de seguridad limitó el trabajo.

---

## 9. Pruebas a mano, con auriculares

1. Una canción de dos pistas con el master a 0 y a −6 dB (subiendo el
   volumen de escucha): ¿se oye la saturación? Decide M1.
2. Quitar la última `auto()` de una canción con lanes y guardar, varias
   veces, con el búfer pequeño: ¿algún clic al compás? (N6)
3. El mismo break a −12 dB: ¿corta donde debe? (N8)
4. En 3/4, una rampa de «1 compás» y una lane de un compás a la vez (M2).
5. Tocar notas largas mientras cambias de dispositivo de audio: ¿se queda
   alguna pegada? (N14)
6. El underrun al arrancar y el cambio de dispositivo con PipeWire.
7. Cada sintetizador contra una referencia: no tienen huella (N10) y nadie
   los ha validado de oído.
