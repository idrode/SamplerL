# Sampler de breaks con live coding (terminal)

Sampler + secuenciador para remixes con amen breaks, controlado por código en vivo. Proyecto en Rust. El desarrollo se hace en una laptop x86_64 con Gentoo.

Guía de uso (inicio rápido, atajos, mini-notación, API de Lua, recetas y problemas): [docs/GUIA.md](docs/GUIA.md). Sus bloques de Lua y las recetas de `scripts/ejemplos/` se comprueban en `script/tests/guide.rs`. La sección de la API se genera desde el registro `script/src/api.rs`: al cambiar la API, actualiza el registro y ejecuta `cargo run -p script --example guia`.

Hoja de ruta (fases A-K después de los pasos 1-7, con sus reglas comunes; la F se hizo antes que la E y la J, EQ y choques de frecuencia entre pistas, después de la K): [docs/ROADMAP.md](docs/ROADMAP.md). Estado detallado y decisiones de cada fase: [docs/ESTADO.md](docs/ESTADO.md). Resumen abajo, en «Estado (resumen)».

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
- Instrumentos propios (bajo y synths con osciladores, ADSR, filtro; evitar aliasing con PolyBLEP o wavetables).
- MIDI: probar con dispositivos reales; enviar las notas con marca de tiempo (cola de ALSA) para quitar el jitter; reloj MIDI de salida; que los pads y las notas de entrada también se puedan grabar como `steps` (fase E).
- Alojar plugins CLAP/LV2: solo si hiciera falta; es costoso.
- Time-stretch de más calidad que el WSOLA de `sample:stretch()` para proporciones grandes o material tonal: Rubber Band tras una feature opcional.
- Cortes: fundido en el salto de los bucles; gate y choke también por corte; remuestrear a WAV de 32 bits en coma flotante y con la cola entera.
- Automatizar también el volumen de pista, los envíos y el pan; rampas de una sola vez (que empiecen al entrar el patrón) además de las que se repiten; `~` para otros parámetros.
- EQ dinámico y sidechain por bandas (que una pista baje una banda solo cuando suena otra), tras la fase J.
- Bloques (fase F): compartir la biblioteca entre máquinas (es una carpeta: git o Syncthing bastan), e insertar bloques también desde el modo tocar o por etiqueta.

## Fuera de alcance realista

Alojar VST, mezclador completo con buses, automatización compleja. No intentarlo.

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
- **Fases hechas**: A (editor: paleta, deshacer, snippets, diagnóstico en vivo, autocompletado y ayuda), B (etiquetas de corte), C (generadores, escalas, swing, humanize, fill), D (mute/solo, medidores, metrónomo, detección de BPM, preescucha, ajuste de cortes), E (grabar lo tocado, secciones con línea de tiempo en el motor, historial de versiones, variantes A/B), F (biblioteca de bloques y ranuras), G (longitud variable, time-stretch, delay y reverb como envíos, compresor y distorsión, automatización), H (gate/choke, forma por corte, rejilla, remuestreo, preparar al importar), I (explorador, tap tempo, rampas de BPM y compases, configuración, MIDI de entrada y salida, ranuras que persisten), J (EQ paramétrico, choques de frecuencia entre pistas con `--analyze-masking` y la paleta, `priority()` y sugerencias de EQ que se insertan tras confirmarlas) y K (melodías con samples: interpolación sinc, instrumento con `melody`, en vivo y por MIDI, pitch sin cambiar la duración con `%` y `keep`, granular y congelar, formantes).
- **En curso**: ninguna fase; todas las de la hoja de ruta están hechas.
- **Comprobaciones que no hay que romper**:
  - `engine/tests/no_alloc.rs`: cero reservas en `Engine::process`; todo lo nuevo del motor entra ahí.
  - `script/tests/golden.rs`: huellas del audio (solo en x86_64); sin usar nada nuevo, idénticas bit a bit.
  - `script/tests/guide.rs`: la guía coincide con el registro (`cargo run -p script --example guia`) y sus bloques de Lua y las recetas de `scripts/ejemplos/` se ejecutan. Las tablas de la paleta y de la configuración se reescriben con `ACTUALIZAR_GUIA=1 cargo test -p tui the_guide_lists_the_palette` (y `the_guide_lists_the_config`).
  - `script/tests/registry.rs`: cada función, efecto, parámetro y snippet del registro existe y funciona en Lua.
- **Ver la interfaz sin terminal**: `cargo test -p tui pantalla -- --ignored --nocapture` (y `pantalla_autocompletado`, `pantalla_paleta`, `pantalla_bloques`, `pantalla_choques`).
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
