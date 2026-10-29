# Hoja de ruta

Fases de trabajo después de los pasos 1-7 de `CLAUDE.md`: qué falta y en
qué orden. Se hacen en orden, una fase cada vez. Lo hecho, con su detalle
y las decisiones de diseño de cada fase, está en
[docs/ESTADO.md](ESTADO.md); el uso, en [docs/GUIA.md](GUIA.md) (referencia) y
[docs/PRIMERA-CANCION.md](PRIMERA-CANCION.md) (tutorial).

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

## Fase L: editor en ventana propia ✅

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

### Parte 2: vistas en el hueco del editor ✅

Hecha: ver «Parte 2» en «Fase L» de [docs/ESTADO.md](ESTADO.md).

1. ✅ **Layout sin editor en la principal**: con el editor fuera, su panel lo
   ocupan las vistas; con el editor dentro, el layout de siempre.
2. ✅ **Forma de onda grande** con los cortes numerados en su propia fila y
   dónde empieza cada uno (el loop, todavía no).
3. ✅ **Tracker con todas las pistas** a la vez, con las columnas al ancho
   que quepa.
4. ✅ **Mezclador** con medidores, volumen, mute/solo y envíos por pista.
5. ✅ **Tabla de choques de frecuencia** (la de `--analyze-masking`), si se
   ha calculado.
6. ✅ **Cambiar en caliente**: sacar el editor a su ventana y volver a
   meterlo desde la paleta sin perder el texto, el cursor ni el historial
   de deshacer.

## Fase M: resaltado en vivo del código al sonar ✅

Hechos los diez puntos, en cinco partes: posiciones de los tokens, paso
publicado por el motor, dibujado en el editor, protocolo del socket
(versión 3) y configuración y paleta. Detalle y decisiones (qué se
resalta en los patrones generados, la prioridad entre pistas, por qué
no hay recuadro) en «Fase M» de [docs/ESTADO.md](ESTADO.md); el uso, en
«Resaltado en vivo» de [docs/GUIA.md](GUIA.md).

Objetivo: como en Strudel, que en el editor se resalten, sobre el propio
código Lua, los trozos de la mini-notación de `steps()` que están sonando
en ese momento, con un color de fondo (o un recuadro, si la terminal lo
permite). Hoy el corte que suena se ve en la forma de onda, pero no en el
texto.

Reglas de siempre: el hilo de audio no reserva, no bloquea ni hace E/S;
todo con test (backend de test de ratatui y render offline); las huellas
de `golden.rs` idénticas; `no_alloc.rs` sin cambios, o ampliado si se toca
el motor; nunca la salida de audio real.

1. ✅ **Posición de cada token**: al cargar el script, cada token de la
   mini-notación guarda su posición en el texto (línea y columna de inicio
   y de fin). El motor ya sabe qué paso suena en cada pista; la interfaz
   une las dos cosas.
2. ✅ **El hilo de audio no cambia**: solo publica el paso actual por pista
   con atómicos (como el resto de `engine/src/status.rs`: sin reservas,
   mutex ni E/S). La interfaz lo lee a su ritmo.
3. ✅ **Sincronía**: usar la posición real del audio, no el reloj de la
   interfaz, y compensar la latencia de salida.
4. ✅ **Texto que ya no coincide con lo cargado** (cambios sin cargar): no
   resaltar, o resaltar solo lo que siga siendo seguro, para no marcar mal.
5. ✅ **Patrones sin texto literal** (`euclid`, `rotate`, `mutate`, `shuffle`,
   `fill`, `chance`, `every`): decidir qué se resalta (la llamada entera o
   nada) y documentarlo.
6. ✅ **Varias pistas a la vez**: un color por pista y la seleccionada, más
   intensa. Definir la prioridad si dos pistas coinciden en una línea.
7. ✅ **Retrigger (`rN`), notas alargadas (`_`) y acordes**: el token entero
   resaltado mientras dure.
8. ✅ **Configurable**: colores y estilo (fondo, subrayado, invertido) en
   `config.toml`; se activa y desactiva desde la paleta (F1).
9. ✅ **Editor en ventana propia**: el protocolo del socket (era la versión 2,
   `link::VERSION`) envía el paso actual a la ventana del editor; sube la
   versión del protocolo.
10. ✅ **Tests** con `TestBackend` y render offline; `golden.rs` intacto;
    `no_alloc.rs` sin cambios, o ampliado si se toca el motor.

## Fase N: automatización y efectos por paso en todo ✅

Hechas las cuatro partes. Objetivo: que se pueda automatizar todo lo que suena, no solo el filtro, y
poner cualquier efecto en un golpe concreto, desde el patrón y desde el
tracker. Detalle y decisiones en «Fase N» de [docs/ESTADO.md](ESTADO.md).

Reglas de siempre: registro único de la API (autocompletado, ayuda, guía,
paleta); el hilo de audio sin reservas, mutex ni E/S (`no_alloc.rs`
ampliado); `golden.rs` idéntico si no se usa nada nuevo; cambios por lotes
atómicos al compás; error con la línea y sigue sonando lo anterior;
portable (x86_64 y ARM64); el resaltado en vivo de la fase M con la
sintaxis nueva; versión nueva del protocolo del socket si cambia.

1. ✅ **Automatizar todo lo continuo**: cada parámetro de sonido del
   registro dice si admite `ramp()`/`lfo()` y cómo (`api::Auto`: continuo,
   por golpe, por grano o no, con el porqué); de ahí salen los errores, el
   autocompletado y la tabla «Qué se puede automatizar» de la guía. Nuevos:
   `ratio`, `attack` y `release` del compresor y del EQ dinámico, `bits` y
   `downsample` del bitcrusher, `:gain()`, el `:pan()` nuevo y `:send()` de
   la pista, y `feedback`, `level`, `size` y `damp` de los buses.
2. ✅ **Lo que se lee al nacer cada voz, por golpe**: el `:pitch()` nuevo
   de pista, la forma del corte (`attack`, `release`, `pan`, `length`,
   `crossfade` en `cortes:set`) y `:formant()`.
3. ✅ **Efectos por paso en la mini-notación**: `3{lp=800,q=4}{crush=6}`,
   con rampas del golpe (`200>4000`, `200>4000:4`); manda el paso sobre su
   corte, y el tracker, el diagnóstico, el autocompletado y el resaltado
   en vivo los entienden.
4. ✅ **El tracker editable**: cursor, añadir, cambiar y quitar los efectos
   de un paso, escribiéndolo en su token del texto (un paso de deshacer) y
   cargando; también con el editor en su ventana (protocolo sin cambios,
   versión 3).

## Fase O: motor de síntesis y primer instrumento (acid) ✅

Hechas las cuatro partes. Objetivo: un núcleo de síntesis en el motor que
sirva para varios instrumentos (después: pads, leads y wavetable sobre el
mismo núcleo, sin nada del 303 dentro) y el primero, un bajo ácido como el
del 303. Detalle y decisiones en «Fase O» de [docs/ESTADO.md](ESTADO.md).

Reglas de siempre: registro único de la API (autocompletado, ayuda,
guía); el hilo de audio sin reservas (`no_alloc.rs` ampliado); `golden.rs`
idéntico si no se usa nada nuevo; cambios por lotes atómicos al compás;
error con la línea y sigue sonando lo anterior; portable (Rust puro).

1. ✅ **Núcleo** (`engine::synth`, código propio y no `fundsp`):
   osciladores sine, saw, square con PWM, triangle y ruido, suboscilador y
   unison con detune; antialiasing con PolyBLEP al doble de frecuencia y
   diezmado (la sierra a 4 kHz, por debajo de -50 dB en un test); filtros
   paso bajo, alto y banda y un ladder de 4 polos con resonancia que
   autooscila y saturación, sin clics al moverlo y con `ramp`/`lfo`; ADSR
   de amplitud y de filtro; glide; voces reservadas al crear el motor;
   fases y ruido deterministas con semilla.
2. ✅ **En el motor**: `Pattern::with_synth`, accent (`!`) y slide (`/`)
   en la mini-notación, mono con legato en los slides, notas en vivo,
   paro, choke y MIDI de salida.
3. ✅ **Acid en Lua**: `synth("acid", { cutoff, res, envmod, decay,
   accent, dist, glide, wave, root, seed })` con `melody`, `scale`,
   `every`, `chance`, curvas en lo continuo y errores con la línea; la
   receta «Un bajo ácido» en la guía.
4. ✅ **Interfaz**: tracker con `!` y `/`, paneles, pads y MIDI de
   entrada, resaltado en vivo y autocompletado.
5. ✅ **Sub** (después, a petición): `synth("sub", {...})` (alias
   `"subbass"`), un subgrave limpio (seno por un paso bajo suave, mono,
   que arranca en el cruce por cero, con `tone`, `drive` antes del filtro,
   `glide`, `attack` y `release`), sobre presets genéricos en el motor.
6. ✅ **sh101** (después, a petición): `sh101{...}` (o `synth("sh101",
   {...})`), un monosynth de bajo y lead como el SH-101, sobre una voz
   analógica nueva en el núcleo: sierra y pulso de la misma fase con PWM
   (manual, LFO o envolvente), el bloque sub con sus tres modos (el pulso
   al 25 %, `sub_wave = "pulse25"`, nuevo en el bloque general), ruido; un
   ladder propio con saturación en cada etapa que autooscila afinado,
   pierde graves con la resonancia (`res_comp` lo compensa), sigue al
   teclado (`kbd`) y va al doble con `quality = 2`; paso alto fijo
   (`hpf`); una sola envolvente para el filtro y el VCA (`vca = env |
   gate`); mono con prioridad a la última nota y legato; portamento
   (`glide_mode = off | on | auto`); LFO (tri, square, random; libre o
   `lfo_sync` al compás) a la altura, al corte y al PWM; y `drift`. Dos
   recetas (bajo y lead) y un «Paso a paso» en la guía. Detalle y
   decisiones en «Después: el sh101» de [docs/ESTADO.md](ESTADO.md).

Queda para después, sobre el mismo núcleo: pads (polifónico con unison,
ataque y release largos) y wavetable.

## Fase P: sub-armónicos ✅

Dos capas: un bloque sub general en los sintetizadores de bajo y un
synth propio, `subharm`, inspirado en el Moog Subharmonicon (dos ejes de
división: frecuencia, f/n, y tiempo, 1/n del compás). Detalle y
decisiones en «Fase P» de [docs/ESTADO.md](ESTADO.md).

Reglas de siempre: el hilo de audio sin reservas (`no_alloc.rs` con subs
y con `subharm` a plena carga); `golden.rs` idéntico y, con `sub = 0`, el
audio de antes bit a bit; cambios por lotes atómicos al compás; error con
la línea y sigue sonando lo anterior; Rust puro; registro único de la API.

Decidido con el usuario antes de empezar: el sub va solo en los
sintetizadores (necesita la fase de un oscilador: un bajo de sample no la
tiene); por paso se escribe `{sub=...}` como parámetros de la nota (un
sintetizador no tiene cortes, así que «por corte» no aplica, y `{lp=800}`
sigue sin valer en un synth: sus voces no llevan cadena de efectos); en
`subharm`, `div` divide también en los osciladores principales (f/div); y
con el transporte parado, `rdiv` cuenta un compás virtual desde la nota.

1. ✅ **Bloque sub** (motor, registro, tests): `sub`, `sub_div`,
   `sub_wave`, `sub_pan`, `sub_drive`, sacado de la fase del oscilador con
   un contador, con PolyBLEP en la cuadrada; en la tabla y por paso.
2. ✅ **Documentación** del bloque sub.
3. ✅ **`subharm`, núcleo**: 2 osciladores de sierra y 2 subs por
   oscilador (6 capas), cada una con `div`, `level` y `pan`.
4. ✅ **Redisparo rítmico** (`rdiv`) y poliritmos sobre el reloj del
   compás.
5. ✅ **Presets, guía y documentación** de `subharm`.

## Fase Q: MPE (Roli Seaboard) y expresión por nota

Pedida por el usuario: pitch bend y presión **por nota** a parámetros de
los synths. Se hace después de la S. **A medias**: hecho lo que no depende
del aparato (los mensajes según la especificación MIDI y MPE, el motor,
Lua, la guía y una receta); falta comprobarlo con la salida de `aseqdump`
del Seaboard y decidir el synth polifónico. Detalle en «Fase Q» de
[docs/ESTADO.md](ESTADO.md).

Hecho:

1. ✅ **Mensajes**: `PitchBend`, `ChannelPressure`, `PolyPressure` y
   `Controller`, con canal (`tui/src/midi.rs`).
2. ✅ **Zonas MPE** (`tui/src/expression.rs`): canal ↔ nota, el maestro
   (canal 1) suma su bend, lo que trae el canal antes de la nota;
   `[midi] mpe` y `bend`.
3. ✅ **Motor**: `Command::NoteMod` (`Controller::note_mod`) y la
   expresión inicial en `SynthOn` (`synth_note_on_expr`), suavizadas y
   sin reservas (`no_alloc.rs`). La voz se identifica por su tecla (como el
   note off), no por un id devuelto.
4. ✅ **Lua**: `:mpe{ bend = "pitch", pressure = { "cutoff", range = 3,
   curve = 1.5 }, slide = "res" }`, registro, guía y la receta
   `14-mpe.lua`.

Falta:

- ⏳ **Datos reales**: el usuario ejecuta `aseqdump -p 24:0`
  (Seaboard RISE = cliente 24:0 en ALSA) tocando y pega la salida:
  canales, rango de bend, CC74, presión de canal o por nota. Anotarlo en
  `docs/ESTADO.md`. No construir sobre suposiciones.
- `tui/src/midi.rs` solo parsea notas y reloj: añadir `PitchBend`,
  `ChannelPressure`, `PolyPressure` y `Controller` (con canal), con tests
  de mensajes simulados.
- Zonas MPE: cada nota en su canal (2-16; el 1 es el maestro); asociar
  canal ↔ voz al entrar la nota y soltarla en el note off. `[midi] mpe =
  true|false` y rango de bend por nota (±48 en MPE por defecto).
- Orden `NoteMod` (voz, eje, valor) por la cola lock-free, suavizada, sin
  reservas (`no_alloc.rs`). `Engine::synth_note_on` devuelve el id de la
  voz.
- Lua: asignar ejes a parámetros, p. ej. `:mpe{ bend = "pitch", pressure
  = "cutoff", slide = "pw" }` (sintaxis propuesta, ajustable), con rango y
  curva. Los asignables son los continuos que admiten `ramp()`/`lfo()`
  (`api::Auto`). Registro, guía y receta.
- ⏳ `acid` y `sh101` son monofónicos (con acordes siguen a la última
  nota): decidir con el usuario si basta `subharm{ voices = n }` (lo que
  usa la receta) o hace falta otro synth polifónico.
- Si el Seaboard manda su rango de bend por RPN 0, leerlo de ahí en vez
  de `[midi] bend`.
- Pendiente opcional: `scripts/conectar-seaboard.sh`, que busque los
  puertos y conecte solo (a mano: `aconnect 24:0 129:0`; 129 es «sampler
  entrada», 128 «sampler salida»; los números cambian al reiniciar).

## Fase R: editor con ratón y modo vim (fase 1) ✅

Pedida por el usuario. Se hizo con la Q a medias (esperando al Seaboard).
Detalle y decisiones en «Fase R» de [docs/ESTADO.md](ESTADO.md).

1. ✅ **Ratón**: clic, arrastrar y rueda en el editor (también en la
   ventana de `--split`), `[editor] raton`.
2. ✅ **Modo vim**, fase 1: `[editor] modo = "vim"`, «Modo vim» en la
   paleta, el modo en el borde del editor; `jk` sale de Insert
   (`[editor] salir_insert`): Ctrl-[ es el mismo byte que Esc, y Esc va al
   modo tocar.

Lo pedido:

- **Ratón en el editor** (hoy no hay captura: `ratatui::init()` sin
  `EnableMouseCapture`): clic mueve el cursor (pantalla → fila/columna,
  con desplazamiento y líneas largas), rueda, arrastrar para seleccionar;
  también en `--split`. Documentar Shift+arrastrar para copiar con la
  terminal.
- **Modo vim** con `[editor] modo = "vim"` en la configuración (por
  defecto, el de ahora), y el modo en la barra de estado. Sobre
  `ratatui-textarea`, que no tiene modos.
- Fase 1 de vim: `h j k l w b e 0 $ gg G`; `i a o O A I`; `x dd yy p P`;
  `u` y Ctrl-R; `/` con `n` y `N`; `:w`. Sin macros, registros, `:s///`
  ni marcas; los operadores compuestos, Visual y `.` quedan para después.
- **Decidido por el usuario**: Esc sigue cambiando del editor al modo
  tocar, también en vim. Así que en Insert, Esc no vuelve a Normal: otra
  tecla sale de Insert (`jk` o Ctrl-[; preguntar cuál) y, en Normal, Esc
  pasa al modo tocar.
- Revisar la paleta, el autocompletado, Alt-G, Ctrl-F, F6/F7, los tests de
  pantalla y los de la guía que simulan teclas. Regenerar la guía
  (`ACTUALIZAR_GUIA=1 cargo test -p tui the_guide_lists_the_config`).

## Fase S: transporte con atajos y cue ✅

Pedida por el usuario; hecha antes que la Q y la R. Detalle y decisiones
en «Fase S» de [docs/ESTADO.md](ESTADO.md).

1. ✅ **Teclado**: Ctrl-Espacio y Shift-Ctrl-Espacio llegan como el mismo
   byte NUL sin ayuda de la terminal. Se piden también
   `DISAMBIGUATE_ESCAPE_CODES` y `REPORT_ALTERNATE_KEYS` (protocolo de
   kitty) y la guía explica las teclas extendidas de tmux; F9 hace lo
   mismo que Shift-Ctrl-Espacio cuando no se distinguen.
2. ✅ **Motor**: `Command::StartAt` (`Controller::start_at`), sin reservas
   (`no_alloc.rs`): arranca en un compás cualquiera con `every`, `ramp`,
   `lfo`, la rampa de tempo y la entrada de la línea de tiempo contando
   desde ahí como si la canción hubiera llegado.
3. ✅ **Interfaz**: Ctrl-Espacio marcha/paro desde el editor, el modo
   tocar y la ventana del editor (`--split`, por el socket);
   Shift-Ctrl-Espacio (o F9) fija el cue (el compás que suena o, parado,
   el del cursor del tracker; con secciones, la entrada); el cue en la
   barra de estado y en la línea de tiempo; «Borrar el cue» en la paleta.
4. ✅ **`--from <compás>`** en el render offline.

## Fase T: vista de ocupación por paso ✅

Hechas las cuatro partes. Pedida por el usuario, para ver los huecos entre pistas (breakcore,
sonidos que se alternan). Detalle y decisiones en «Fase T» de
[docs/ESTADO.md](ESTADO.md).

Reglas de siempre: hilo de audio sin reservas, mutex ni E/S (si se publica
algo del motor, con atómicos y `no_alloc.rs` ampliado); `golden.rs`
idéntico; cambios por lotes atómicos al compás; error con la línea y sigue
sonando lo anterior; portable (x86_64 y ARM64); versión nueva del
protocolo del socket si cambia; registro único de la API; tests con
`TestBackend` en varios tamaños y de ida y vuelta.

1. ✅ **Rejilla de ocupación** (solo lectura): una fila por pista, una
   columna por paso, golpe o silencio; debajo, cuántas pistas suenan en
   cada paso y los pasos vacíos resaltados. Con `:length(n)` distintos,
   varias vueltas y patrones generados (lo que sonará, del secuenciador y
   no del texto); el paso actual con la sincronía del resaltado de la
   fase M; retrigger, `chance`/`every` y silencios por probabilidad con
   su marca; intensidad por volumen (`@`).
2. ✅ **Dónde se ve**: panel propio, y en la ventana principal con el editor
   en la suya; desplazamiento o compresión si no cabe; en la paleta (F1)
   para mostrarla y ocultarla, y colores en `config.toml`.
3. ✅ **Rellenar desde la vista**: un cursor sobre un hueco y una acción que
   pone ahí un golpe de otra pista (a elegir), escrito en el texto Lua
   como en el tracker editable (un paso de deshacer, solo ese token). Lo
   generado, los instrumentos y un texto sin cargar no se editan, y la
   línea de estado dice por qué.
4. ✅ **Sugerencias de relleno** (solo texto, a petición desde la paleta,
   nunca automático): hasta 4 huecos y qué corte de qué pista los
   rellenaría, con el criterio documentado (el análisis de choques, si
   lo hay).

## Fase U: panel de dispositivos ✅

Hechas las cuatro partes. Pedida por el usuario: la salida de audio y los
puertos MIDI, vistos y cambiados en caliente, con el panel en su ventana
como el editor. Detalle y decisiones en «Fase U» de
[docs/ESTADO.md](ESTADO.md); el uso, en «Dispositivos: audio y MIDI» de
[docs/GUIA.md](GUIA.md).

Reglas de siempre: hilo de audio sin reservas, mutex ni E/S (`no_alloc.rs`
ampliado); `golden.rs` idéntico; portable (x86_64 y ARM64); la feature
`midi` sigue siendo opcional; versión nueva del protocolo del socket.

1. ✅ **Lógica de dispositivos** (sin interfaz): listar audio de salida
   (nombre, canales, frecuencias, predeterminado) y puertos MIDI; cambiar
   en caliente (el audio, al instante con un fundido de 5 ms y sin perder
   la canción ni el compás; el MIDI, sin tocar el audio); si un
   dispositivo se va, el anterior, el predeterminado o la salida nula, con
   aviso; refrescar a petición; la latencia medida mueve el resaltado.
2. ✅ **Persistencia**: `[dispositivos]` en `config.toml` por nombre;
   `--audio-device`, `--midi-in`, `--midi-out` y `--list-devices`.
3. ✅ **Panel**: en la principal desde la paleta y en su ventana
   (`tui --window-devices`, socket propio, protocolo 4), con el indicador
   de actividad MIDI y vuelta a la principal si la terminal no se lanza.
4. ✅ **Documentación**: la guía (también PipeWire, auriculares y
   Bluetooth), el tutorial y las tablas de la paleta y la configuración.

Queda por comprobar con dispositivos reales (ver «Sin verificar» en
ESTADO): PipeWire, auriculares USB y Bluetooth, un controlador MIDI.

## Fase V: afinación, mandos y congelar

Pedida por el usuario: seis funciones, un commit por función (o por parte
si es grande). Detalle y decisiones en «Fase V» de
[docs/ESTADO.md](ESTADO.md); el uso, en [docs/GUIA.md](GUIA.md) y un
recuadro en el tutorial donde encaja cada una.

Reglas de siempre: hilo de audio sin reservas, mutex ni E/S (`no_alloc.rs`
ampliado); `golden.rs` idéntico; cambios por lotes atómicos al compás;
error con la línea y sigue sonando lo anterior; portable (Rust puro);
registro único de la API; resaltado de la fase M y tracker compatibles;
versión nueva del protocolo si cambia.

1. ✅ **Cortes en el cruce por cero**: `zero_cross` en `slice` y
   `slice_grid` (ms máximos de desplazamiento), que vale también para
   `add`, `move` y los ajustes del proyecto; apagada por defecto.
2. **MIDI learn**: elegir un parámetro automatizable desde la paleta,
   mover un mando y queda asignado (rango y curva), en el proyecto y
   exportable a `config.toml`; lista para verlas, cambiarlas y borrarlas;
   el mando manda sobre `ramp`/`lfo` mientras se mueve.
3. ✅ **Tonalidad de un sample**: cromagrama y perfiles mayor/menor fuera
   del hilo de audio; un panel con las tres más probables y la confianza,
   desde la paleta y el explorador; usarla como raíz del instrumento.
4. ✅ **Tonalidad global**: `key()` y `transpose()`; `scale()`, `melody()` e
   instrumentos la usan por defecto; la detección la propone.
5. **Sidechain visual**: reducción de ganancia en tiempo real y su curva,
   y quién dispara a quién, publicadas con atómicos.
6. **Congelar pista**: render de una pista a un WAV del proyecto que la
   sustituye; reversible y en un paso de deshacer.

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
7. ✅ **Navegación del editor**: lo que ya daba el widget (palabras,
   línea, párrafos, páginas y selección con Shift), verificado, y
   Ctrl-Inicio/Ctrl-Fin al principio y al final del texto; ir a la
   línea N con Alt-G; buscar con Ctrl-F y F6/F7 siguiente y anterior.
8. ✅ **Tutorial «Tu primera canción»** ([docs/PRIMERA-CANCION.md](PRIMERA-CANCION.md),
   archivo aparte de la guía desde la separación de la documentación): una
   canción entera en trece pasos, para quien nunca ha usado el programa;
   un test sigue los pasos y carga cada estado del script.
9. ✅ **Arreglos tras el tutorial**: la guía ya no dice que `slice(1)` es
   el sample entero (es `slice_grid(1)`); las llaves en `melody()` dicen
   dónde valen; «1 compás» en singular; y `tui --edit` con una ruta que
   no existe es un error, salvo con `--new`.
10. ✅ **Grupos y macros**: `group(nombre, pistas)` suma varias pistas y
    las pasa juntas por su cadena de efectos, volumen, pan y envíos (4
    grupos, un nivel: no es el mezclador con buses de «fuera de
    alcance»); `macro(nombre, desde, hasta)` es un valor de 0 a 1 que
    siguen los parámetros en lugar de un número, y se mueve en vivo desde
    la paleta («Macros») o con un CC (`[midi] macros`).
