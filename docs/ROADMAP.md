# Hoja de ruta

Fases de trabajo después de los pasos 1-7 de `CLAUDE.md`. Se hacen en
orden, una fase cada vez; el estado detallado de lo hecho está en el
Estado de `CLAUDE.md` y el uso en `docs/GUIA.md`.

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

Hecha: ver «Fase A» en el Estado de `CLAUDE.md` y las secciones de atajos,
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

Hecha: ver «Fase B» en el Estado de `CLAUDE.md` y la mini-notación,
`cortes:name` y los proyectos en `docs/GUIA.md`.

- Etiquetas para los cortes (`kick`, `snare`, `hat`...), guardadas en el
  proyecto.
- Usables en la notación: `steps("kick . snare .")`.

## Fase C: generadores y transformaciones ✅

Hecha: `euclid`, `rotate`, `reverse_pattern`, `mutate`, `shuffle`,
`scale` (con `:pitch` y `:melody`), `pista:swing`, `pista:humanize` y
`pista:fill`, en el registro de la API y en la sección 4 de
`docs/GUIA.md`. Ver «Fase C» en el Estado de `CLAUDE.md`.

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
proyecto). Ver «Fase D» en el Estado de `CLAUDE.md`.

- Mute y solo por pista.
- Medidores y aviso de saturación.
- Metrónomo opcional.
- Detección del BPM del break.
- Preescucha de cortes con las flechas.
- Ajuste manual de los cortes en ms, guardado en el proyecto.

## Fase E: estructura y versiones

- Secciones con nombre y línea de tiempo.
- Historial de versiones del script con marca de tiempo.
- A/B de variantes.
- Grabar lo tocado en el modo tocar como un patrón de `steps`.

## Fase F: biblioteca de bloques ✅

Hecha (antes que la E): `script::blocks`, paleta («Guardar la selección
como bloque», «Insertar un bloque», «Actualizar un bloque con la
selección», «Borrar un bloque», ranuras 1-9) y autocompletado. Ver
«Fase F» en el Estado de `CLAUDE.md` y «Biblioteca de bloques» en
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
notación. Ver «Fase G» en el Estado de `CLAUDE.md` y las secciones 3 y 4
de `docs/GUIA.md`. Decisiones:

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

Reglas de siempre, con especial cuidado en el motor: nada de reservas ni
mutex en el hilo de audio (cada cosa nueva entra en `no_alloc.rs`),
cambios en lotes atómicos al compás, código portable (x86_64 y ARM64, sin
intrínsecos ni `target-cpu=native`; dependencias de C/C++ solo tras una
feature opcional), tests de render offline con semilla fija, y la guía y
el Estado de `CLAUDE.md` al día si cambia la notación o el motor.

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
fade_out })`. Ver «Fase H» en el Estado de `CLAUDE.md`. Decisiones:

- **Por pista o por corte**: el modo (gate/one-shot) y el grupo de choke
  son de la pista, como en un pad de una caja de ritmos; la forma
  (envolvente, pan, parte, bucle) es de cada corte y se comparte entre la
  variable y sus pistas, como las etiquetas. Con los valores por defecto,
  todo suena igual que antes bit a bit (huellas).
- **Gate**: la voz se cierra al acabar su paso (o su parte del
  retrigger) y hace la cola del corte. **Choke**: el corte callado hace el
  fundido mínimo; vale también para los pads.
- **Bucles**: suenan hasta el siguiente golpe de la pista, la puerta o el
  choke; el salto no lleva fundido (se oye si el corte no acaba como
  empieza). `length` es una fracción del corte, no ms.
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

- Grupos de choke y modos one-shot/gate, con envolvente por corte (ataque
  y cola).
- Cortar por rejilla (8 o 16 divisiones iguales) y añadir o borrar cortes a
  mano, además de moverlos.
- Pan y duración por corte, ping-pong y bucle dentro de un corte.
- Resampling: grabar la salida de una pista (o del master) como un sample
  nuevo reutilizable en el proyecto, por render offline.
- Normalizar, recortar el silencio y poner fundidos al importar un sample.

## Fase I: comodidad ✅

Hecha: explorador de samples, tap tempo, `bpm_ramp()` y `meter()`,
`config.toml` (teclas, colores y `[midi]`), entrada MIDI (notas y reloj),
`pista:midi()` hacia un synth y ranuras que persisten. Ver «Fase I» en el
Estado de `CLAUDE.md`. Decisiones:

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

- Explorador de samples en la interfaz, con preescucha.
- Entrada MIDI (notas que disparan cortes) y reloj MIDI; salida MIDI hacia
  Surge XT.
- Tap tempo, rampas de BPM y cambios de compás.
- Teclas y colores configurables en un archivo.
- Ranuras 1-9 que se guardan entre sesiones.
