# Hoja de ruta

Fases de trabajo después de los pasos 1-7 de `CLAUDE.md`: qué falta y en
qué orden. Se hacen en orden, una fase cada vez. Lo hecho, con su detalle
y las decisiones de diseño de cada fase, está en
[docs/ESTADO.md](ESTADO.md); el uso, en [docs/GUIA.md](GUIA.md).

## Reglas comunes

- **Un registro único de la API** (`script/src/api.rs`): funciones, efectos,
  snippets y ayuda. Lo usan el autocompletado, la ayuda contextual, la
  validación y la guía (la sección de la API de `docs/GUIA.md` se genera
  desde él). Nada de listas duplicadas a mano.
- **Paleta de comandos** para las acciones nuevas, en vez de gastar atajos
  de teclado (quedan muy pocos libres).
- **Nada que reserve memoria en el hilo de audio** (y las demás reglas de
  arquitectura de `CLAUDE.md`).
- **Todo lo nuevo con test**: render offline para el audio, backend de test
  de ratatui para la interfaz. Nunca la salida de audio real desde Claude
  Code.

## Fase A: editor ✅

Hecha: ver «Fase A» en [docs/ESTADO.md](ESTADO.md) y las secciones de atajos,
paleta, autocompletado y ayuda de `docs/GUIA.md`.

- Deshacer y rehacer.
- Comentar, duplicar y mover líneas.
- Snippets con salto entre campos con Tab: `amen`, `fill`, `ghost`,
  `intro`.
- Diagnóstico en vivo de la notación de `steps("...")`, con el paso erróneo
  subrayado.
- Autocompletado contextual con popup: tras `:`, dentro de `fx("...")`,
  nombres de WAV y nombres de pistas. Esc solo cierra el popup.
- Ayuda contextual de la función bajo el cursor.
- Paleta de comandos filtrable.

## Fase B: etiquetas de corte ✅

Hecha: ver «Fase B» en [docs/ESTADO.md](ESTADO.md) y la mini-notación,
`cortes:name` y los proyectos en `docs/GUIA.md`.

- Etiquetas para los cortes (`kick`, `snare`, `hat`...), guardadas en el
  proyecto.
- Usables en la notación: `steps("kick . snare .")`.

## Fase C: generadores y transformaciones ✅

Hecha: `euclid`, `rotate`, `reverse_pattern`, `mutate`, `shuffle`,
`scale` (con `:pitch` y `:melody`), `pista:swing`, `pista:humanize` y
`pista:fill`, en el registro de la API y en la sección 4 de
`docs/GUIA.md`. Ver «Fase C» en [docs/ESTADO.md](ESTADO.md).

- `euclid(k, n)`, `rotate`, `mutate(p)`, `shuffle` (con semilla).
- `scale("minor", "C")` para tocar melodías con cortes.
- Swing y humanize.
- `fill("stutter", cada N compases)`.

## Fase D: mezcla y cortes ✅

Hecha: mute y solo (`pista:mute()`, `pista:solo()`, paleta y `m`/`s`),
medidor del master con avisos de limitador y saturación, metrónomo
(`metronome()`, paleta; fuera de lo exportado salvo que se pida),
detección de BPM (`--detect-bpm`, `sample:detect_bpm()`, paleta),
preescucha de cortes por una voz aparte (←/→ en el modo tocar) y ajuste
manual de cortes en ms (`,` `.` `<` `>`, `cortes:move()`, guardado en el
proyecto). Ver «Fase D» en [docs/ESTADO.md](ESTADO.md).

- Mute y solo por pista.
- Medidores y aviso de saturación.
- Metrónomo opcional.
- Detección del BPM del break.
- Preescucha de cortes con las flechas.
- Ajuste manual de los cortes en ms, guardado en el proyecto.

## Fase E: estructura y versiones ✅

Hechas la grabación de lo tocado, las secciones con línea de tiempo
(`section()`, `arrange()`), el historial de versiones y el A/B de
variantes (`variant()`); detalle y decisiones en
[docs/ESTADO.md](ESTADO.md).

- Secciones con nombre y línea de tiempo: `section(nombre, función)` y
  `arrange({ {sección, compases}, ... })`, con el arreglo en el motor. ✅
- Historial de versiones del script con marca de tiempo: una por carga
  buena, sin duplicar, fuera del repositorio, «Versiones» en la paleta. ✅
- A/B de variantes: `variant(sección o pista, "B", función)`, alternar
  con `v` o la paleta al compás siguiente. ✅
- Grabar lo tocado en el modo tocar como un patrón de `steps`. ✅

## Fase F: biblioteca de bloques ✅

Hecha (antes que la E): `script::blocks`, paleta («Guardar la selección
como bloque», «Insertar un bloque», «Actualizar un bloque con la
selección», «Borrar un bloque», ranuras 1-9) y autocompletado. Ver
«Fase F» en [docs/ESTADO.md](ESTADO.md) y «Biblioteca de bloques» en
`docs/GUIA.md`.

Bloques de código reutilizables: trozos de script guardados en archivos
que se insertan como texto en el editor (nunca se cargan ni se enlazan
entre archivos: el script sigue siendo uno solo).

- Carpeta `blocks/` global (`~/.config/sampler/blocks/`) y otra del
  proyecto (`blocks/` junto al proyecto o al script), con archivos `.lua`
  con cabecera de nombre, descripción y etiquetas. Si falta la carpeta, se
  crea sin error.
- Paleta: guardar la selección como bloque (pide nombre y etiquetas),
  insertar un bloque (lista filtrable con vista previa), actualizarlo con
  la selección y borrarlo.
- Ranuras rápidas 1-9, estilo registros de vim, para guardar y pegar
  fragmentos al vuelo.
- Los bloques salen en el autocompletado junto a los snippets, con campos
  `${nombre}` que se rellenan saltando con Tab.
- Un bloque con Lua no válido no rompe nada: al cargarlo da el error de
  siempre y sigue sonando lo anterior.

## Fase G: lo que más cambia el sonido ✅

Hecha: `pista:length(n)`, `sample:stretch()`, `bus()` y `pista:send()`,
`:fx("compressor")` y `:fx("distortion")`, `ramp()`, `lfo()` y `~` en la
notación. Detalle y decisiones en «Fase G» de [docs/ESTADO.md](ESTADO.md);
uso en las secciones 3 y 4 de `docs/GUIA.md`.

Reglas de siempre, con especial cuidado en el motor: nada de reservas ni
mutex en el hilo de audio (cada cosa nueva entra en `no_alloc.rs`),
cambios en lotes atómicos al compás, código portable (x86_64 y ARM64, sin
intrínsecos ni `target-cpu=native`; dependencias de C/C++ solo tras una
feature opcional), tests de render offline con semilla fija, y la guía y
`docs/ESTADO.md` al día si cambia la notación o el motor.

1. **Longitud variable de patrón por pista** (`pista:length(n)`: 12, 24,
   32 pasos...), con polirritmia entre pistas. Toca el motor, la
   mini-notación, el diagnóstico en vivo, el tracker y el autocompletado.
   Con `length(16)` la salida es idéntica bit a bit a la de antes.
2. **Time-stretch al BPM del proyecto sin cambiar el tono**:
   `sample:stretch(bpm_origen)`, o automático con `detect_bpm`. En Rust
   puro (Rubber Band, si algún día, tras una feature opcional).
3. **Delay sincronizado al tempo y reverb como envíos**: divisiones 1/4,
   1/8 y 1/16 con feedback; `pista:send("delay", nivel)` y parámetros
   globales de cada bus en el registro. Buffers fijos.
4. **Compresor y distorsión** como efectos de pista.
5. **Automatización**: `ramp(desde, hasta, "4 compases")`,
   `lfo(frecuencia, mín, máx)` y valores por paso (parameter locks) en la
   mini-notación, por ejemplo el cutoff. Cuantizada al compás,
   determinista y sin clics (parámetros suavizados).

## Fase H: cortes y breaks ✅

Hecha: `pista:mode()`, `pista:choke()`, `cortes:set()` (ataque, cola,
pan, parte que suena, bucle e ida y vuelta), `sample:slice_grid()`,
`cortes:add()`/`remove()` y partir/unir desde la paleta, remuestreo
(`remuestrear`, `--track`) y `sample(ruta, { trim, normalize, fade_in,
fade_out })`. Detalle y decisiones en «Fase H» de
[docs/ESTADO.md](ESTADO.md).

- Grupos de choke y modos one-shot/gate, con envolvente por corte (ataque
  y cola).
- Cortar por rejilla (8 o 16 divisiones iguales) y añadir o borrar cortes a
  mano, además de moverlos.
- Pan y duración por corte, ping-pong y bucle dentro de un corte (con
  fundido en el salto, añadido después: ver «Mejoras sueltas»).
- Resampling: grabar la salida de una pista (o del master) como un sample
  nuevo reutilizable en el proyecto, por render offline.
- Normalizar, recortar el silencio y poner fundidos al importar un sample.
- ✅ **Efectos por corte** (añadido después de la fase K):
  `cortes:fx(cuál, nombre, parámetros)` pone un efecto de pista (o el nuevo
  `gain`) solo en un corte o en varios, por número, etiqueta o lista, con
  `ramp()`/`lfo()`; estado propio por voz y bit a bit igual sin ellos. Los
  envíos por corte se quedaron fuera (ver «Fase H» de
  [docs/ESTADO.md](ESTADO.md)).

## Fase I: comodidad ✅

Hecha: explorador de samples, tap tempo, `bpm_ramp()` y `meter()`,
`config.toml` (teclas, colores y `[midi]`), entrada MIDI (notas y reloj),
`pista:midi()` hacia un synth y ranuras que persisten. Detalle y decisiones en
«Fase I» de [docs/ESTADO.md](ESTADO.md).

- Explorador de samples en la interfaz, con preescucha.
- Entrada MIDI (notas que disparan cortes) y reloj MIDI; salida MIDI hacia
  Surge XT.
- Tap tempo, rampas de BPM y cambios de compás.
- Teclas y colores configurables en un archivo.
- Ranuras 1-9 que se guardan entre sesiones.

## Fase J: EQ y choques de frecuencia entre pistas ✅

Hecha: `:fx("eq")` (4 bandas, campana y estantes, con `ramp`/`lfo`), el
análisis de choques (`--analyze-masking` y «Choques de frecuencia» en la
paleta), `priority()` con sus sugerencias de recorte y aplicarlas desde la
paleta tras confirmarlo; después, el EQ dinámico con sidechain por bandas
(`:fx("dyneq")`). Ecualizar cada pista y encontrar dónde se pisan
dos pistas (masking): el análisis se hace offline, en el hilo de control, y
nunca cambia el script sin confirmación. Detalle y decisiones en «Fase J»
de [docs/ESTADO.md](ESTADO.md); uso en «Choques de frecuencia entre
pistas» de `docs/GUIA.md`.

Reglas de siempre: registro único de la API (autocompletado, ayuda y guía),
nada de reservas en el hilo de audio (lo nuevo del motor en `no_alloc.rs`),
las huellas de `golden.rs` idénticas si no se usa nada nuevo, código
portable en Rust puro (x86_64 y ARM64), tests de render offline y nunca la
salida de audio real.

1. ✅ **Efecto de pista `eq`**: EQ paramétrico de hasta 4 bandas (campana,
   estante grave y agudo) con biquads; parámetros en el registro con rangos
   y valores por defecto, con `ramp` y `lfo`, suavizados y sin reservas en el
   callback. Tests de respuesta en frecuencia medida y salida idéntica bit a
   bit sin cambios.
2. ✅ **Análisis offline de choques** (`rustfft`): cada pista renderizada en
   solo, espectro en tercios de octava en ventanas de ~50 ms y, por pareja de
   pistas y banda, una puntuación de choque ponderada por energía. Un ciclo
   completo de la canción, con el tope de compases de la exportación.
3. ✅ **Informe**: `tui --script x.lua --analyze-masking` y una acción de la
   paleta con la tabla pareja × banda, sin tocar el audio.
4. ✅ **Prioridad entre pistas**: `priority({ "kick", "snare", ... })` (por
   defecto manda la más fuerte) y sugerencias de recorte (como mucho 4, de 2
   a 4 dB) en la pista de menor prioridad.
5. ✅ **Aplicar**: la paleta inserta el `fx("eq", {...})` sugerido en el
   script, tras confirmarlo.
6. ✅ **EQ dinámico y sidechain por bandas** (de las ideas): `:fx("dyneq",
   { freq, threshold, range, sidechain = "kick" })`, una banda que baja solo
   mientras suena fuerte en ella la propia pista u otra; sin reservas en el
   callback y con las huellas de siempre si no se usa.

## Fase K: melodías con samples ✅

Hechos los ocho puntos: la interpolación del pitch, el instrumento
(`sample:instrument()`, `pista:melody()`, `_` y acordes en la notación),
el pitch sin cambiar la duración (`%` y `keep`), granular y congelar
(`:grains()`, `:freeze()`), tocarlo en vivo, la salida MIDI de sus notas,
el diagnóstico en vivo de `melody` y los formantes (`formant`). Detalle y decisiones en «Fase K»
de [docs/ESTADO.md](ESTADO.md).

Reglas de siempre: registro de la API (autocompletado, ayuda y guía), nada
de reservas en el hilo de audio (cada cosa nueva en `no_alloc.rs`), las
huellas de `golden.rs` idénticas si no se usa nada nuevo, código portable
(x86_64 y ARM64), tests de render offline y nunca la salida de audio real.

1. ✅ **Mejor interpolación para el pitch**: cúbica o sinc corta, elegida
   midiendo el aliasing de un seno agudo subido +12 y +24 semitonos. Con
   pitch 0, idéntico bit a bit.
2. ✅ **Instrumento**: `sample("x.wav"):instrument({ root = "C3", loop =
   {inicio, fin}, voices = 4, adsr = {a, d, s, r} })`. Polifonía por pista
   con las voces limitadas y robo de la más antigua, envolvente ADSR por
   voz, bucle sostenido con fundido cruzado en el salto (sin clic) y nota
   raíz para que `:melody("c e g e")` toque cromáticamente respecto a ella.
3. ✅ **Pitch sin cambiar la duración**: transponer un corte o una nota
   manteniendo su duración: `3%7` por paso y `keep = true` en un
   instrumento (granos alineados en tiempo real, elegidos midiendo frente
   al WSOLA de `stretch`; bien hasta ±12).
4. ✅ **Granular y congelar**: leer el sample en granos (posición, tamaño,
   densidad, dispersión) para texturas y para congelar un instante:
   `pista:grains({ ... })` y `pista:freeze(pos)`, con semilla y con
   `ramp`/`lfo` en sus parámetros. Y en vivo: los pads y el MIDI de entrada
   abren una nube mientras se mantiene la tecla (ver «Nube en vivo» en
   [docs/ESTADO.md](ESTADO.md)).
5. ✅ **Tocar un instrumento en vivo** desde el modo tocar y desde el MIDI
   de entrada, con note on y note off: la nota arranca la envolvente y el
   note off dispara el release. Decide la pista seleccionada; en las
   pistas de cortes, todo igual.
6. ✅ **Salida MIDI de las notas de un instrumento** con `pista:midi(canal)`,
   con note on y note off al frame exacto.
7. ✅ **Diagnóstico en vivo de `melody("...")`** en el editor, subrayando la
   nota errónea.
8. ✅ **Formantes**: mantener o desplazar los formantes al transponer
   (que una voz subida no suene a ardilla): `pista:formant(n)` para los
   pasos con `%` y `formant = n` en un instrumento con `keep`; LPC por
   grano, elegido midiendo una vocal y una caja sintéticas.

## Fase L: editor en ventana propia

Objetivo: poder abrir el editor de código en una ventana de terminal
independiente, para que el gestor de ventanas (Hyprland, por ejemplo) la
trate como cualquier otra: flotante, redimensionable y movible entre
workspaces. Y que el espacio que deja libre en la ventana principal lo
ocupen otras vistas.

Reglas de siempre: el motor y el modelo `Song` viven en un solo proceso (el
de la ventana principal); el editor nunca toca el audio, solo envía el
texto y sus órdenes. Todo con test (dos extremos simulados del protocolo,
backend de test de ratatui para las dos ventanas) y nunca la salida de
audio real.

### Parte 1: separación en dos procesos ✅

Hecha: ver «Fase L» en [docs/ESTADO.md](ESTADO.md) y «El editor en su
propia ventana» en [docs/GUIA.md](GUIA.md).

1. **`tui --edit x.lua` sigue igual**, en una sola ventana: sin socket ni
   nada nuevo si no se pide.
2. **Dos procesos, un motor**: la ventana principal (motor, estado, vistas,
   modo tocar, MIDI) y el editor (texto, deshacer, autocompletado,
   diagnóstico en vivo, paleta, ayuda contextual, bloques, ranuras,
   versiones) en procesos distintos, cada uno en su terminal, con las
   librerías `script` y `tui` compartidas.
3. **Socket Unix local** en `$XDG_RUNTIME_DIR/sampler/` (carpeta 0700,
   socket 0600, se rechaza a cualquier otro usuario por sus credenciales;
   nada de red). Protocolo de líneas de texto, versionado. Del editor a la
   principal: el texto, cargarlo y las acciones y órdenes de la paleta que
   tocan el motor; de la principal al editor: el texto que cambie ella
   (etiquetas, EQ sugerido, grabación...), los errores, avisos, compás,
   paso, pista y corte seleccionados y las etiquetas conocidas.
4. **Caídas y reconexión**: si el editor se cierra o se cae, la principal
   sigue sonando y se puede volver a abrir (recupera el texto); si cae la
   principal, el editor sigue editando y guardando y se reconecta solo
   cuando vuelve. El audio no depende del socket.
5. **Cargar igual que hoy**: una carga desde el editor entra al compás
   siguiente; si el código es inválido, sigue sonando lo anterior y el
   error se ve en el editor (regla 5).
6. **Lanzadores**: `tui --edit x.lua --split` arranca la principal y abre
   el editor en otra terminal con el comando de `[editor] terminal` de
   `config.toml` (p. ej. `kitty --class sampler-editor -e`);
   `tui --window-editor x.lua` conecta con una instancia ya en marcha (por
   SSH, en otro panel de `tmux`). Regla de ejemplo para Hyprland en la
   guía.
7. **Pegar** con la selección nativa del terminal: bracketed paste activado
   al entrar y quitado al salir (también con error o pánico), varias
   líneas con sangría tal cual y un solo Ctrl-Z para deshacer el pegado.

### Parte 2: vistas en el hueco del editor

1. ✅ **Layout sin editor en la principal**: con el editor fuera, su panel lo
   ocupan las vistas; con el editor dentro, el layout de siempre.
2. ✅ **Forma de onda grande** con los cortes numerados en su propia fila y
   dónde empieza cada uno (el loop, todavía no).
3. ✅ **Tracker con todas las pistas** a la vez, con las columnas al ancho
   que quepa.
4. ✅ **Mezclador** con medidores, volumen, mute/solo y envíos por pista.
5. ✅ **Tabla de choques de frecuencia** (la de `--analyze-masking`), si se
   ha calculado.
6. **Cambiar en caliente**: sacar el editor a su ventana y volver a
   meterlo desde la paleta sin perder el texto, el cursor ni el historial
   de deshacer.

## Mejoras sueltas

Arreglos pequeños fuera de las fases, con las reglas de siempre. Detalle
en «Mejoras sueltas» de [docs/ESTADO.md](ESTADO.md).

1. ✅ **Pegado multilínea** (bracketed paste): ya estaba (pegar en el
   editor y la parte 1 de la fase L); verificado, sin cambios.
2. ✅ **Portapapeles del sistema**: copiar y cortar por OSC 52 y `wl-copy`;
   pegar con `wl-paste` o, si no, el interno.
3. ✅ **Tracker con sus 16 filas** sin desplazarse en cuanto caben dejando
   al sample lo mínimo; si no, como antes.
4. ✅ **Diagnóstico de `steps()` con `:length(n)`**: ya estaba; verificado
   con 12, 16 y 24, sin cambios.
5. ✅ **Fundido en el salto de los bucles de corte**: `crossfade` en
   `cortes:set` (10 ms por defecto; con 0, el salto seco de siempre).
6. ✅ **Ayuda del modo tocar** sin «Esc editor» en la principal con el
   editor en su ventana.
