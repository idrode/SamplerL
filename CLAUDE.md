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
- Interfaz: solo terminal (TUI) con `ratatui` + `tui-textarea`. Decisión del usuario: nada de GUI por ahora, experiencia 100% terminal.
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

1. Motor de audio: cargar un WAV y reproducirlo sin glitches.
2. Cortes: detección de transitorios, cada corte asignado a una tecla.
3. Secuenciador de 16 pasos con BPM y loop.
4. Efectos por paso/pista: volumen, pitch, reverse, retrigger/stutter, filtro, bitcrusher.
5. Mini-notación + Lua con recarga en caliente.
6. Interfaz de paneles (editor, sample, tracker vertical, efectos).
7. Exportar a WAV (render offline) y guardar/cargar proyectos.

## Ideas para después

- Deformar y modificar el sample al importarlo (pitch, time-stretch, granular, etc.) y tocar melodías con él.
- Instrumentos propios (bajo y synths con osciladores, ADSR, filtro; evitar aliasing con PolyBLEP o wavetables).
- Salida MIDI hacia Surge XT (o similar) y captura del audio por PipeWire/JACK.
- Alojar plugins CLAP/LV2: solo si hiciera falta; es costoso.
- Time-stretch de calidad con una librería (p. ej. Rubber Band).

## Fuera de alcance realista

Alojar VST, mezclador completo con buses, automatización compleja. No intentarlo.

## Ejemplo de la API objetivo (Lua)

Diseño al que apuntar; todavía no existe. Cada llamada solo emite comandos al motor, que los aplica al final del compás.

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

Primer objetivo: que `steps(...)` reproduzca los cortes. Después `fx`, `every` y `chance`, uno a uno.

## Estado

- **Paso 1 hecho.** Workspace con `engine` (librería) y `tui` (binario mínimo: `tui <archivo.wav>` reproduce y sale; aún sin ratatui).
- Decisiones técnicas:
  - Samples en memoria como estéreo f32 entrelazado (mono se duplica, >2 canales se usan los dos primeros), resampleados al cargar con `rubato::Fft`. La longitud de salida se calcula con enteros porque rubato 5 puede dar un frame de más.
  - Carga de WAV con `hound`; `symphonia` todavía no se usa.
  - `new_engine()` devuelve `Controller` (control) y `Engine` (audio). Comandos por cola SPSC lock-free (`rtrb`). Los `Arc<Sample>` que el motor deja de usar vuelven por una segunda cola ("basura") y se liberan en el hilo de control: el callback nunca libera memoria.
  - Salida con `cpal` 0.18, dispositivo por defecto; el callback convierte formato con un bloque intermedio en la pila.
  - Test de render offline en `engine/tests/`: se llama a `Engine::process` por bloques sin dispositivo de audio.
- **Paso 2 hecho.** `Slices::detect(&sample, n)` (`engine/src/slice.rs`): envolvente RMS por saltos de 256 frames con compresión `ln(1 + 100·rms)`, subida positiva como función de detección, máximos locales por encima de un umbral, los `n` más fuertes con separación mínima de 40 ms, y afinado al frame donde empieza el ataque. Los cortes son contiguos (hasta el siguiente transitorio o el final); lo anterior al primer transitorio queda fuera. Si hay menos golpes que `n`, devuelve menos cortes.
  - `Controller::play_slice(&sample, &slices, i)` resuelve el rango en el hilo de control y envía `Play { sample, start, end }`; el motor solo recorre frames. Errores: `SendError::{QueueFull, NoSuchSlice}`.
  - Tests en `engine/tests/slicing.rs` con golpes sintéticos en posiciones conocidas (error medido ~0,2–0,3 ms, tolerancia 2 ms).
  - La distancia mínima de 40 ms se compara en saltos de 256 frames redondeando hacia arriba: en la práctica exige entre 40 y ~45 ms.
- **Fundidos**: cada voz tiene rampas de entrada y salida de 1,5 ms. Una voz interrumpida (nuevo disparo o `stop`) pasa a una segunda ranura y hace su fundido mientras entra la nueva. Tests en `engine/tests/fades.rs`.
- **Render offline**: `tui <archivo.wav> --render <salida.wav>` renderiza a 48 kHz sin abrir el dispositivo (WAV PCM 16 bits). Código en `engine::offline`.
- **Paso 3 hecho.** Secuenciador de 16 pasos (`engine/src/sequencer.rs`):
  - `Pattern::new(sample, &slices, [Option<usize>; 16])` resuelve los cortes a rangos en el hilo de control; el motor recibe un array fijo, sin reservas. Un paso `None` no dispara nada (lo que sonaba sigue).
  - `Controller::{set_pattern, set_bpm, start, stop}`. `set_pattern` y `set_bpm` quedan pendientes y entran en el paso 0 del siguiente compás (o al arrancar). `stop` es inmediato.
  - El paso k cae en `inicio_compás + round(k · frames_por_paso)`, y cada compás empieza donde acabó el anterior. No hay deriva aunque los frames por paso no sean enteros. El bloque de audio se parte en los límites de paso.
  - `new_engine(sample_rate)`: el motor necesita su frecuencia; `open_output` la ajusta a la del dispositivo.
  - `Engine::discard` suelta el `Arc` directamente si el motor guarda otra referencia al mismo sample (solo baja el contador). Así las voces que dispara el secuenciador no llenan la cola de basura.
  - Una sola voz por motor por ahora: el secuenciador y `play_slice` comparten la voz.
  - Tests en `engine/tests/sequencer.rs`: frame exacto de cada paso a 120 y 170 BPM, silencios, loop y cuantización.
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

- REA (morluto/rea) no se usa en este proyecto: es para ingeniería inversa, no para escribir un sampler.
- Claude Code no puede oír el resultado: puede renderizar offline y analizar picos, longitud y alineación de cortes; el criterio de "suena bien" es del usuario.
- Respeta los derechos de los samples usados en tracks publicados.
