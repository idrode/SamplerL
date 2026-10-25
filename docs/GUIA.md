# Guía de uso

Sampler de breaks con live coding en la terminal: cortas un break (el amen,
por ejemplo) en sus golpes, los secuencias con una mini-notación y lo
controlas todo desde un script de Lua que se recarga en caliente.

Todos los bloques de código Lua de esta guía se comprueban en los tests
(`cargo test -p script --test guide`): se cargan contra el break sintético
y deben funcionar sin errores. Las recetas están también como archivos en
`scripts/ejemplos/`.

**¿Nunca has usado el programa?** Empieza por la sección 8, [Tu primera
canción, de principio a fin](#8-tu-primera-canción-de-principio-a-fin): un
tutorial paso a paso que monta una canción entera.

## Índice

- [Qué se puede hacer](#qué-se-puede-hacer)
- [1. Inicio rápido](#1-inicio-rápido)
- [2. Atajos de teclado](#2-atajos-de-teclado)
- [3. Mini-notación](#3-mini-notación)
- [4. API de Lua](#4-api-de-lua)
- [5. Cuantización, recarga atómica y errores](#5-cuantización-recarga-atómica-y-errores)
- [6. Recetas para amen breaks](#6-recetas-para-amen-breaks)
- [7. Solución de problemas](#7-solución-de-problemas)
- [8. Tu primera canción, de principio a fin](#8-tu-primera-canción-de-principio-a-fin)

## Qué se puede hacer

Un mapa de todo, con dónde se explica. Las funciones de Lua están todas en
la sección 4 (con sus parámetros, rangos y un ejemplo cada una).

**Cortar el sample**
- Cortar por transitorios (`sample:slice(n)`) o en partes iguales
  (`slice_grid(n)`), y detectar el tempo del break (`detect_bpm`, o
  «Detectar el BPM del sample» en la paleta).
- Prepararlo al importar: recortar silencios, normalizar y fundidos
  (`sample(ruta, { trim, normalize, fade_in, fade_out })`), y estirarlo al
  tempo de la canción sin cambiar el tono (`sample:stretch()`).
- Etiquetar cortes (`cortes:name`, o desde la interfaz), moverlos a oído
  (`,` `.` en el modo tocar, o `cortes:move`), partirlos y unirlos.
- Dar forma a cada corte (`cortes:set`: ataque, cola, pan, la parte que
  suena, bucle sin clic en el salto o ida y vuelta) y ponerle sus propios efectos
  (`cortes:fx`: ensuciar solo la caja, bajar solo el bombo).

**Secuenciar** (sección 3 y sección 4)
- Pasos con la mini-notación: reverse, pitch con duración (`^`) o sin ella
  (`%`), volumen, retrigger, corte del filtro por paso (`~`), notas largas
  (`_`) y acordes.
- Longitud por pista (polirritmos), `every`, `chance`, `fill`, swing y
  humanize, generadores (`euclid`, `rotate`, `mutate`, `shuffle`,
  `reverse_pattern`), escalas y `melody`.
- Modos gate y one-shot y grupos de choke (`pista:mode`, `pista:choke`).

**Sonido**
- Efectos de pista (`pista:fx`): filtros, bitcrusher, compresor,
  distorsión, volumen (`gain`), EQ de 4 bandas y EQ dinámico con
  sidechain; y los mismos por corte (`cortes:fx`).
- Moverlos en el tiempo con `ramp()` y `lfo()`, o por paso con `~`.
- Delay y reverb como envíos (`bus`, `pista:send`).
- Instrumentos con polifonía y ADSR (`sample:instrument`), transponer sin
  cambiar la duración (`%`, `keep`) y conservar los formantes
  (`pista:formant`).
- Sintetizadores sin sample (`synth`): el bajo ácido del 303
  (`synth("acid", {...})`) y un subgrave limpio (`synth("sub", {...})`),
  con accent (`!`) y slide (`/`) por paso, y en cualquiera de ellos un sub
  dividido de su oscilador (`sub`, `sub_div`...; también por paso:
  `{sub=0.5,sub_div=3}`).
- Texturas con granos y congelar un instante (`pista:grains`,
  `pista:freeze`).
- Ver dónde se pisan las pistas en frecuencia y que el EQ sugerido se
  escriba solo («Choques de frecuencia», `priority`).

**Estructura**
- Secciones y línea de tiempo (`section`, `arrange`), saltar y repetir
  secciones en vivo, variantes A/B (`variant`), rampas de tempo y otros
  compases (`bpm_ramp`, `meter`).
- Historial de versiones del script («Versiones» en la paleta).

**Tocar en vivo** («Modo tocar», «MIDI»)
- Disparar cortes y notas con el teclado o por MIDI, preescuchar, mute y
  solo al instante, tap tempo, metrónomo.
- Grabar lo tocado como `steps("...")`, seguir el tempo de un reloj MIDI
  y mandar las notas por MIDI a un synth (`pista:midi`).

**Editor** (sección 2)
- Autocompletado, ayuda contextual y errores subrayados mientras escribes;
  paleta de comandos (F1) y línea de órdenes (F2); biblioteca de bloques y
  ranuras; el editor en su propia ventana (`--split`).

**Sacarlo** («Renderizar a WAV», «Proyectos»)
- Exportar la mezcla a WAV, remuestrear una pista para volver a cortarla y
  guardar proyectos `.sampler` (script, carpeta de samples, tempo,
  etiquetas y ajustes de corte).

Los cambios del script entran siempre al empezar el compás siguiente y, si
algo está mal, sigue sonando lo anterior (sección 5).

## 1. Inicio rápido

### Generar el break sintético

El repositorio no trae samples con derechos. Para probar hay un break
sintético (bombo, charles y caja, 8 golpes):

```sh
scripts/render-demo.sh
```

Genera `renders/synth_break.wav` y, de paso, renderiza una demo en
`renders/demo.wav`. Si solo quieres el break:

```sh
cargo run -p engine --example synth_break -- renders/synth_break.wav
```

Sus 8 cortes son: 1 bombo, 2 charles, 3 caja, 4 charles, 5 bombo, 6 bombo,
7 caja, 8 charles. Las recetas de esta guía usan esa numeración; con un
amen de verdad los golpes caen en otros números (mira la vista del sample o
la lista de cortes que imprime `--render`).

### Saber el tempo del break

```sh
cargo run -p tui -- renders/synth_break.wav --detect-bpm
```

Propone el `bpm()` y cuántos compases dura el sample, suponiendo que es un
loop de compases enteros (lo normal en un break preparado para samplear).
Si dos tempos encajan igual (un break en corcheas vale a 86 y a 172), da
el más cercano a 130 y menciona el otro. En la interfaz, «Detectar el BPM
del sample» (paleta) hace lo mismo con el sample de la pista seleccionada
y deja escrita la orden `bpm 140`: Enter cambia (o añade) la línea `bpm()`
del script, sin guardar. En Lua, `sample(...):detect_bpm()`.

Si prefieres marcarlo tú, «Tap tempo» (paleta) cuenta golpes: pulsa
espacio al pulso (cuenta la media de los últimos 8; una pausa de más de
2 s empieza otra cuenta) y Enter pone `bpm(N)` en el script; Esc cancela.
Para que el tempo cambie a lo largo de la canción, `bpm_ramp(desde,
hasta, "8 compases")`, y para otro compás, `meter(3, 4)` o `meter(7, 8)`
(sección 4).

Para elegir un WAV sin salir de la interfaz, «Explorador de samples»
(paleta) lista los de la carpeta de muestras y sus subcarpetas: escribe
para filtrar por nombre, ↑ ↓ eligen y preescuchan cada uno en la voz
aparte (sin tocar las pistas), Tab lo vuelve a sonar, Enter escribe
`sample("ruta")` en el cursor y Esc cierra.

Para tocar el break **a otro tempo sin cambiar su tono**, estíralo:
`sample("amen.wav"):stretch(140):slice(8)` lo pasa de 140 al tempo de la
canción (sin el número, usa el que detecta). Cada golpe cae en la rejilla
nueva y la vista del sample lo dice (`140→170 BPM`); los ajustes de cortes
siguen en ms del sample original. Ver `sample:stretch` en la sección 4.

### Usar un sample propio

Solo se leen archivos **WAV** (cualquier frecuencia; se resamplean al
cargar). `sample("archivo.wav")` busca la ruta relativa a la carpeta del
script. Pon el WAV junto al `.lua`:

```lua
bpm(170)
local amen = sample("amen.wav"):slice(8)
track("drums", amen):steps("1 . 2 .  3 . 4 .  5 6 2 .  7 . 8 .")
```

o crea un proyecto que diga dónde están los samples (ver más abajo).

Si el WAV trae silencio delante o detrás, está bajo de nivel o acaba con
un chasquido, prepáralo al importarlo:
`sample("amen.wav", { trim = true, normalize = true, fade_out = 10 })`
recorta el silencio de los extremos, lleva el pico a 0 dB y pone un
fundido de salida de 10 ms (ver `sample` en la sección 4). El archivo no
se toca: se prepara al cargar y se guarda para la carga siguiente.

Recuerda respetar los derechos de los samples si publicas lo que hagas.

### Abrir la interfaz

```sh
cargo run -p tui -- --edit mi-cancion.lua --new
```

Con `--new`, si el script no existe se abre una plantilla (el archivo se
crea al guardar, en una carpeta que ya tiene que existir); si existe, se
abre tal cual. Sin `--new`, una ruta que no existe es un error que dice
dónde se ha buscado (así una errata en el nombre no abre en silencio un
script vacío):

```text
error: mi-canción.lua: el script no existe (buscado en /home/yo/sampler/mi-canción.lua); para empezar uno nuevo con la plantilla: tui --edit mi-canción.lua --new
```

Un proyecto (`.sampler`) que no existe es siempre un error: se crea desde
la interfaz con la orden `guardar`. A la izquierda está el
editor; a la derecha, el estado del motor, la forma de onda del sample y el
tracker; abajo, los efectos de cada pista. Ctrl-S guarda y carga. El
tracker enseña sus 16 pasos enteros en cuanto caben (le quita filas a la
forma de onda hasta dejarle cuatro); en una terminal más baja se desplaza
siguiendo el paso que suena.

**Sin `--allow-audio` no suena nada**: el motor avanza igual (el tracker se
mueve), pero el audio se tira. Para oírlo por la tarjeta:

```sh
cargo run -p tui -- --edit mi-cancion.lua --new --allow-audio
```

### Leer el panel de efectos

Abajo, una columna por pista. Cada una dice:

| Lo que ves | Qué es |
| --- | --- |
| `drums` (resaltado) | La pista seleccionada (Ctrl-O cambia) |
| `MUTE`, `SOLO`, `(solo de otra)` | Mute o solo en vivo o del script; una pista que no suena porque otra está en solo |
| `· swing 58` | Swing de la pista |
| `· gate` | Modo gate (cada golpe dura su paso) |
| `· MIDI 1/36` | Manda sus golpes por MIDI (canal y nota del corte 1; en un instrumento, solo el canal) |
| `· ♪ C3 2/4` | Instrumento: su raíz, y las voces que suenan de las que tiene |
| `· ♪ acid C2 1/1` | Sintetizador (`synth`): su tipo, su raíz y sus voces (el panel del sample enseña sus parámetros, `∿` en los que se mueven) |
| `· formant 0`, `· formant -3` | Formantes al transponer con `%` |
| `· ⁘ 80 ms · 20/s`, `· ❄ 0.4 · 120 ms · 25/s` | Granos (tamaño y densidad) o congelado (posición); `∿` si se mueve con `ramp`/`lfo` |
| `· choke 1` | Grupo de choke |
| `vol 0.70 ███░░` y `→ d0.30 r0.15` | Volumen de la pista (`:gain`) y sus envíos al delay y a la reverb |
| `nivel ███░░`, `SAT` | Lo que suena ahora; `SAT` si la pista sola pasa de ±1 |
| `◇ 3 snare › crush 6b ÷1 · LP 800 Hz` | Efectos de ese corte (`cortes:fx`), que suenan antes que los de la pista. Si no caben, una línea: `◇ efectos en 1, 3 snare` |
| `› LP 3000 Hz q0.71` | Cada efecto de la pista, en orden |
| `∿cutoff` | Ese parámetro se mueve (`ramp`, `lfo` o `~` por paso) |
| `sin efectos` | Ni la pista ni sus cortes tienen efectos |

### El editor en su propia ventana

Con `--split`, el editor sale a una ventana de terminal aparte, que el
gestor de ventanas trata como cualquier otra (flotante, redimensionable,
en otro workspace). La ventana de siempre se queda con el motor, las
vistas, el modo tocar y el MIDI; el editor, con el texto, deshacer, el
autocompletado, el diagnóstico, la ayuda, los bloques, las ranuras y las
versiones. Son dos procesos que se hablan por un socket local; si se
cierra el editor, la principal sigue sonando, y si se cae la principal, el
editor sigue editando y guardando y se reconecta solo cuando vuelve.

```sh
cargo run -p tui -- --edit mi-cancion.lua --split [--new] [--allow-audio] [--allow-midi]
```

La principal abre el editor con el comando de `[editor] terminal` de
`config.toml`, al que añade `tui --window-editor mi-cancion.lua`:

```toml
[editor]
terminal = "kitty --class sampler-editor -e"
# terminal = "foot --app-id sampler-editor"
# terminal = ["alacritty", "--class", "sampler-editor", "-e"]
```

Sin esa línea, o por SSH (en otro panel de `tmux`), se abre a mano y
conecta con la principal de ese script que esté en marcha:

```sh
cargo run -p tui -- --window-editor mi-cancion.lua
```

Si el editor se cierra, F1 «Abrir el editor en su ventana» en la
principal lo vuelve a abrir; el nuevo recupera el texto, también lo que
estaba sin guardar. Solo hay un editor a la vez: uno nuevo sustituye al
anterior.

**Cambiar en caliente**, sin volver a arrancar y sin que deje de sonar:

- Con una sola ventana (`tui --edit`, sin `--split`), F1 «Abrir el editor
  en su ventana» lo saca a una terminal nueva, como `--split`: el editor
  de allí se lleva el texto (también sin guardar), el cursor y el
  historial de deshacer y rehacer, y esta ventana se queda con las vistas.
  Sin `[editor] terminal` en `config.toml`, dice el comando para abrirlo a
  mano; el primer editor que se conecte se lo lleva todo.
- F1 «Volver a meter el editor en la principal», desde cualquiera de las
  dos ventanas, cierra el editor de fuera y lo vuelve a poner en la
  principal con su texto, su cursor y su historial (Ctrl-Z sigue
  deshaciendo lo que se escribió allí), sin guardar nada. Sin editor
  conectado, la principal vuelve con el último texto que recibió.
- El historial que se lleva es el del editor: como mucho los últimos 200
  pasos (lo tecleado cuenta letra a letra; un cambio compuesto, como
  duplicar líneas o insertar un bloque, es uno).

- **Cargar** (Ctrl-L o F5) y **guardar y cargar** (Ctrl-S) mandan el
  texto a la principal, que lo carga como siempre: entra en el compás
  siguiente, y si tiene errores sigue sonando lo anterior y el error sale
  en el editor. Al pie del editor, una línea dice si está conectado, la
  marcha, el compás y el paso, y la pista y el corte seleccionados en la
  principal.
- **Las acciones del motor** (marcha/paro, mute, solo, metrónomo, cortes,
  secciones, variantes, exportar, las órdenes de F2 menos `bloque`, `linea` y `buscar`) se
  pueden lanzar desde la paleta del editor y se hacen en la principal.
  El explorador, el tap tempo, el informe de choques y las preguntas de
  grabar, saltar a una sección o remuestrear se abren en la principal:
  hay que ir a esa ventana. `etiquetar` pregunta en el editor.
- **El modo tocar** es de la principal, que con el editor fuera está
  siempre en él (Esc no vuelve al editor, y la línea de ayuda no lo
  ofrece). Sin el editor, sus vistas
  ocupan toda la ventana: arriba el transporte; la forma de onda grande de
  la pista seleccionada, con los cortes numerados en una fila aparte (el
  que suena y el elegido, resaltados) y al pie dónde empieza cada uno en
  ms; debajo, a la izquierda el mezclador (una tira por pista con su
  medidor vertical, el nivel en dB, el volumen, mute `M`, solo `S`, `SAT`
  si satura y los envíos, y la del master con `LIM` y `SAT`) y a la
  derecha el tracker con todas las pistas a la vez (las columnas se
  estrechan para que quepan; si ni así, el título dice cuántas se ven); y
  abajo los efectos. Tras analizar los choques de frecuencia (F1
  «Choques de frecuencia»), su tabla se queda a la derecha de la forma de
  onda al cerrar el informe: una fila por pareja de pistas y una columna
  por banda, con la puntuación en rojo si el choque es fuerte y en
  amarillo si es medio (`·`: suenan las dos sin chocar; `-`: banda vacía)
  y resaltadas las parejas de la pista seleccionada; si no cabe, la lista
  de choques del más fuerte al más débil. Al cargar otra cosa, avisa de
  que es de una carga anterior hasta que se vuelve a analizar.
- **Lo que cambia la principal en el texto** (etiquetar, `bpm`, el EQ
  sugerido, lo grabado, restaurar una versión) llega al editor como un
  solo cambio (Ctrl-Z lo deshace), sin guardar. Si en ese momento estás
  escribiendo en el editor, se queda lo tuyo y se avisa: haz el cambio
  otra vez.
- **Pegar**: con el editor en su ventana se usa la selección del propio
  terminal (el pegado con corchetes se pide al entrar y se quita al
  salir, también si hay un error). Varias líneas con su sangría entran tal
  cual y un solo Ctrl-Z lo deshace.
- **Seguridad**: el socket está en `$XDG_RUNTIME_DIR/sampler/` (carpeta
  0700, socket 0600, uno por script) y la principal rechaza conexiones de
  otro usuario; nada va por la red.

Para que Hyprland abra el editor flotante y de un tamaño cómodo, una regla
que lo reconozca por su clase (la que pone `--class` o `--app-id`). Con la
configuración en Lua (`~/.config/hypr/hyprland.lua`; va como `text`
porque no es un script del sampler):

```text
hl.window_rule({
  match = { class = "^sampler-editor$" },
  float = true,
  size = { 900, 700 },
})
```

O, en un `hyprland.conf` clásico:

```ini
windowrule = float, class:^(sampler-editor)$
windowrule = size 900 700, class:^(sampler-editor)$
```

(Los nombres de las reglas cambian entre versiones de Hyprland:
compruébalos en la documentación de la tuya.)

### Renderizar a WAV

Sin abrir la interfaz, un script de Lua:

```sh
cargo run -p tui -- --script scripts/ejemplos/01-ritmo-basico.lua --render renders/ritmo.wav --bars 4
```

Un WAV con un patrón en mini-notación, sin Lua:

```sh
cargo run -p tui -- renders/synth_break.wav --render renders/patron.wav \
  --pattern "1 . 2 .  3 . 4 .  5 6 2 .  7 . 8 ." --slices 8 --bpm 170 --bars 4
```

Opciones de `--render` con `--pattern`: `--slices N` (cortes a detectar, 8
por defecto), `--bpm B` (120), `--bars M` (4), `--min-gap MS` (separación
mínima entre cortes, 40) y `--detector flux|energy` (flux). `--metronome`
(también con `--script`) añade el clic del metrónomo al render; sin él, el
render nunca lo lleva, aunque el script llame a `metronome(true)`. Imprime los
cortes detectados con su posición en ms.

Desde la interfaz, la orden `exportar` (F2) hace lo mismo con lo que está
cargado.

**Remuestrear** es grabar la salida de una pista (o del master) como un
sample nuevo para volver a cortarlo y tocarlo. Con `--track pista` (y
`--script`), `--render` graba solo esa pista, como si estuviera en solo:
con sus efectos, su volumen, sus envíos y la cola de los buses (y el
limitador). Sin `--bars`, dura lo que la pista tarda en repetirse. En la
interfaz, «Remuestrear la pista seleccionada» (paleta) o la orden
`remuestrear` (ver la línea de órdenes) lo dejan en la carpeta de samples,
listo para `sample("drums-remuestreo.wav")`.

Para escuchar en otra máquina: `scp laptop:~/sampler/renders/ritmo.wav .`

### Choques de frecuencia entre pistas

Dos pistas que suenan a la vez en la misma zona del espectro se tapan (el
bajo y el bombo en 60-100 Hz, la caja y una voz en 1-3 kHz). El análisis lo
mide sin escuchar y sin tocar lo que suena:

```sh
cargo run -p tui -- --script canción.lua --analyze-masking
```

En la interfaz, «Choques de frecuencia» (paleta) analiza lo último cargado
y abre el informe: un choque por fila (pareja, banda y puntuación), el
resumen del elegido, el EQ sugerido para la pista que cede y debajo la
tabla pareja × banda (`·` sin choque, `-` banda en la que una de las dos
casi no suena). Escribir filtra por pista o banda; Esc cierra.

**Aplicar una sugerencia**: Enter sobre un choque propone el EQ de la pista
que cede (sin tocar nada todavía) y Enter otra vez inserta la línea
`track("bass"):fx("eq", {...})` en el editor, justo después de donde se
define la pista (y de su cadena de `:steps`, `:fx`...). No se guarda ni se
carga: Ctrl-S lo guarda y lo carga, Ctrl-Z lo quita de una vez y Esc antes
del segundo Enter lo cancela. Si la pista ya tiene 4 efectos, lo avisa (al
cargar, el EQ no cabría; sigue sonando lo anterior). Vuelve a analizar
después para ver cuánto ha bajado el choque.

Cómo se mide: cada pista se renderiza offline en solo (con sus efectos,
envíos y el limitador, como al remuestrear) durante un ciclo entero de la
canción (lo que tarda en repetirse, o la línea de tiempo), con el mismo
tope de 256 compases que la exportación. Su espectro se mide en 30 bandas de
tercio de octava (25 Hz a 20 kHz) en ventanas de 50 ms cada 25 ms. En cada
ventana:

- Una pista está **activa** en una banda si no queda más de **30 dB** por
  debajo de su propio máximo (el de esa pista en cualquier banda y momento).
  Así lo flojo de una pista cuenta igual que lo fuerte de otra, y las colas
  casi mudas no.
- Si las dos están activas, su **cercanía** vale 1 con el mismo nivel y baja
  en línea recta hasta 0 cuando las separan **9 dB** (a esa distancia una
  ya tapa a la otra).
- La **puntuación** de la banda (0 a 1) es la media de la cercanía
  ponderada por la energía de las dos en cada ventana: pesan más los
  momentos fuertes.
- Una banda en la que una de las dos casi no suena (su energía queda
  **30 dB** por debajo de la de su propia banda más fuerte) no se puntúa:
  para chocar, las dos tienen que sonar ahí de verdad, no solo con el
  chasquido de un ataque. Se informa desde **0,25**; el resumen dice
  «leve» hasta 0,4, «medio» hasta 0,6 y «fuerte» desde ahí.

**Sugerencias.** En cada choque, una pista manda y la otra cede: la
primera de `priority({ "kick", "bass", ... })` (una de la lista manda sobre
las que no están) y, sin lista o entre dos que no están, la que suena más
fuerte en esa banda. La que cede recibe un `fx("eq", {...})` con como mucho
4 recortes en campana: uno por zona que choca (bandas seguidas, un recorte
más ancho, con menos Q; una sola banda, Q 4,3), de 2 dB en un choque de
0,25 a 4 dB desde 0,75, y si hay más de 4 zonas, las 4 que más chocan. El
informe los escribe como una línea de Lua lista para pegar:

```
sugerencias (priority: kick; las demás, por su nivel):
  track("bass"):fx("eq", { freq1 = 63, gain1 = -3.5, q1 = 1.4 })
    -- 50-80 Hz: -3.5 dB, q 1.4 (frente a kick, 0.68)
```

Por debajo de unos 60 Hz la ventana de 50 ms no separa bien los tercios de
octava: un choque allí sale también en las bandas vecinas. Analizar una
canción larga tarda (cada pista se renderiza entera).

### Proyectos

Un proyecto (`.sampler`) es un archivo de texto con el script, la carpeta
de samples y el tempo:

```text
# proyecto del sampler
script = cancion.lua
samples = /home/yo/samples/breaks
bpm = 170
etiqueta = amen.wav 8 1 kick
etiqueta = amen.wav 8 3 snare
partir = amen.wav 8 180.25
unir = amen.wav 8 7
corte = amen.wav 8 3 412.5
```

Las rutas relativas lo son a la carpeta del proyecto. Con `samples`,
`sample("amen.wav")` busca en esa carpeta. El `bpm` es el tempo por
defecto: si el script llama a `bpm()`, manda el script. Cada `etiqueta`
nombra un corte: sample (como se escribe en `sample("...")`), número de
cortes (la `n` de `slice(n)`: con otro número los cortes son otros y no se
aplica, y al cargar se avisa: `⚠ la etiqueta «kick» de amen.wav es de
slice(8) y este script usa slice(6): no se aplica`), corte y nombre. Así `steps("kick . snare .")` funciona sin
`:name()` en el script; si el script nombra el mismo corte, manda el
script. Cada `corte` es un ajuste manual (ver el modo tocar): el corte 3 de
`slice(8)` empieza en 412,5 ms, diga lo que diga el detector; si no cabe
entre sus vecinos, la carga lo avisa y no lo aplica. `partir` añade un
corte que empieza en esos ms y `unir` quita la marca de un corte (el
anterior se alarga); se aplican en orden, justo después de detectar, y los
`corte` y las `etiqueta` van con la numeración que queda. Para un sample
cortado en rejilla (`slice_grid(16)`), el número de cortes se escribe
`rejilla16`. Se abre con `tui --edit proyecto.sampler` o con la orden
`abrir`.

## 2. Atajos de teclado

### Interfaz

| Tecla | Qué hace |
| --- | --- |
| Ctrl-S | Guardar el script y cargarlo (entra en el siguiente compás) |
| Ctrl-L o F5 | Cargar el texto del editor sin guardar |
| Ctrl-T o F8 | Marcha / paro del secuenciador |
| Ctrl-G | Ir a la línea del error |
| Alt-G | Ir a una línea (escribe el número y Enter) |
| Ctrl-F | Buscar en el script (escribe el texto y Enter) |
| F6 / F7 | Siguiente / anterior de lo buscado |
| Ctrl-O | Siguiente pista (la de la vista del sample y del modo tocar) |
| Esc | Entrar / salir del modo tocar (con el autocompletado o la paleta abiertos, solo los cierra) |
| F1 | Paleta de comandos (ver más abajo) |
| F2 | Línea de órdenes |
| F3 | Recargar el archivo si cambió fuera del editor |
| F4 | Abrir el autocompletado (ver más abajo) |
| Ctrl-Q | Salir (dos veces si hay cambios sin guardar) |

Con un cambio externo pendiente, Ctrl-S pide un segundo Ctrl-S antes de
sobrescribir. F3 guarda antes tu texto sin guardar en
`<nombre>.sin-guardar.lua`.

### Ayuda contextual

En el modo editor, la última línea del editor (`ⓘ ...`) explica lo que hay
bajo el cursor: sobre el nombre de una función o un método, su firma y qué
hace; dentro de una llamada, el parámetro en el que estás (y el efecto
elegido en `fx("...")`, en `cortes:fx(cuál, "...")` o en
`chance(p, "...")`); dentro de la tabla de `fx("efecto", { ... })` (o de
`cortes:fx(cuál, "efecto", { ... })`), los parámetros del efecto y, tras
`clave =`, el rango y el valor por defecto de ese parámetro. Sale del mismo registro que
la sección 4.

### Paleta de comandos (F1)

F1 abre una lista con todas las acciones del editor. Escribe para
filtrarla (vale cualquier trozo de cada palabra, sin tildes: `baj lin`
encuentra «Bajar líneas»), elige con ↑ ↓, Enter ejecuta y Esc cierra. La
última acción usada sale la primera, así que repetirla es F1 y Enter. Las
acciones nuevas van en la paleta en vez de en atajos de teclado.

<!-- paleta:inicio (generado desde tui/src/palette.rs) -->

| Acción | Qué hace | Atajo |
| --- | --- | --- |
| Deshacer | Deshace el último cambio del texto | Ctrl-Z o Ctrl-U |
| Rehacer | Rehace lo último deshecho | Ctrl-R |
| Comentar o descomentar líneas | Pone o quita `-- ` en la línea del cursor o en las seleccionadas | — |
| Duplicar líneas | Copia la línea del cursor (o las seleccionadas) debajo | — |
| Subir líneas | Mueve la línea del cursor (o las seleccionadas) una arriba | — |
| Bajar líneas | Mueve la línea del cursor (o las seleccionadas) una abajo | — |
| Ir al principio del texto | Lleva el cursor a la primera línea del script (con Shift, selecciona hasta allí) | Ctrl-Inicio |
| Ir al final del texto | Lleva el cursor al final de la última línea (con Shift, selecciona hasta allí) | Ctrl-Fin |
| Ir a la línea | Abre «linea » para escribir el número de línea; Enter lleva el cursor a su principio | Alt-G |
| Buscar | Abre «buscar » para escribir el texto (tal cual; sin mayúsculas, no las distingue); Enter va al primero desde el cursor y los resalta todos; «buscar» sin texto quita el resaltado | Ctrl-F |
| Buscar el siguiente | Va a la siguiente vez que sale lo buscado (al final, vuelve al principio) | F6 |
| Buscar el anterior | Va a la vez anterior que sale lo buscado (al principio, vuelve al final) | F7 |
| Autocompletar | Abre el autocompletado en el cursor | F4 |
| Guardar y cargar | Guarda el script y lo carga (entra en el siguiente compás) | Ctrl-S |
| Cargar sin guardar | Carga el texto del editor sin guardarlo | Ctrl-L o F5 |
| Marcha / paro | Arranca o para el secuenciador | Ctrl-T o F8 |
| Ir al error | Lleva el cursor a la línea del error | Ctrl-G |
| Siguiente pista | Selecciona la pista siguiente (vista del sample y modo tocar) | Ctrl-O |
| Modo tocar | Entra o sale del modo tocar | Esc |
| Recargar del disco | Recarga el archivo si cambió fuera del editor | F3 |
| Exportar la mezcla | Renderiza la canción entera a `<script>-mezcla.wav` | — |
| Línea de órdenes | Abre la línea de órdenes (exportar, guardar, abrir, muestras) | F2 |
| Mute de la pista | Silencia la pista seleccionada, o la vuelve a dejar sonar (en vivo, sin esperar al compás) | m (modo tocar) |
| Solo de la pista | Pone la pista seleccionada en solo, o se lo quita (en vivo) | s (modo tocar) |
| Quitar mute y solo | Quita todos los mute y solo en vivo (los del script siguen) | — |
| Detectar el BPM del sample | Propone el tempo y los compases del sample de la pista; Enter pone bpm() en el script | — |
| Metrónomo | Enciende o apaga el clic en cada pulso (no entra en lo exportado salvo que se pida) | — |
| Reiniciar medidores | Pone a cero los medidores y quita los avisos de limitador y saturación | — |
| Etiquetar el corte seleccionado | Abre «etiquetar N » para escribir la etiqueta (kick, snare...); se guarda en el proyecto | — |
| Preescuchar el corte seleccionado | Lo toca en una voz aparte, sin pasar por la pista ni tocar el script | espacio (modo tocar) |
| Adelantar el corte 5 ms | Mueve el inicio del corte seleccionado 5 ms antes (se guarda en el proyecto y manda sobre el detector) | , (modo tocar; < 1 ms) |
| Retrasar el corte 5 ms | Mueve el inicio del corte seleccionado 5 ms después | . (modo tocar; > 1 ms) |
| Quitar el ajuste del corte | El corte seleccionado vuelve a empezar donde lo pone el detector | — |
| Corte siguiente | Selecciona el corte siguiente de la pista (sin tocarlo) | — |
| Corte anterior | Selecciona el corte anterior de la pista (sin tocarlo) | — |
| Partir el corte seleccionado | Añade un corte en su mitad (se guarda en el proyecto; las etiquetas y ajustes siguientes se corren) | — |
| Unir el corte seleccionado con el anterior | Quita su marca: el anterior se alarga hasta donde acababa este (se guarda en el proyecto) | — |
| Grabar lo tocado | Abre «grabar 1»: graba N compases de lo que tocas (pads o MIDI) en la pista seleccionada, desde el compás siguiente, y lo escribe como steps("...") en el cursor; otra vez, para antes | — |
| Tap tempo | Golpea el pulso con espacio (media de los últimos 8 golpes); Enter pone bpm(N) en el script | espacio (al golpear) |
| Editar los efectos de un paso en el tracker | Cursor en el tracker: ↑↓ paso, ←→ pista, Enter escribe los efectos del paso (lp=800,q=4 crush=6) en su token del código y carga, Supr los quita, Esc sale; un paso de deshacer | — |
| Explorador de samples | Los WAV de la carpeta de muestras: filtra, ↑↓ preescucha (sin tocar las pistas), Enter inserta sample("...") | — |
| Remuestrear la pista seleccionada | Abre «remuestrear pista »: graba su salida (render offline) como un WAV nuevo en la carpeta de samples | — |
| Guardar la selección como bloque | Abre «bloque » para escribir nombre, etiquetas y `-- descripción`; va a la biblioteca global | — |
| Insertar un bloque | Lista filtrable (nombre, etiquetas, descripción) con vista previa; Tab salta entre sus campos `${...}` | — |
| Actualizar un bloque con la selección | Elige un bloque y sustituye su texto por la selección (conserva nombre, descripción y etiquetas); Enter dos veces | — |
| Borrar un bloque | Elige un bloque y borra su archivo; Enter dos veces | — |
| Guardar en una ranura (1-9) | Guarda la selección (o la línea del cursor) en la ranura del número que se pulse, como los registros de vim | — |
| Pegar una ranura (1-9) | Pega la ranura del número que se pulse; las líneas enteras van encima de la del cursor | — |
| Saltar a la sección siguiente | Con arrange(): la entrada siguiente de la línea de tiempo empieza en el compás siguiente | ] (modo tocar) |
| Saltar a la sección anterior | Con arrange(): la entrada anterior de la línea de tiempo empieza en el compás siguiente | [ (modo tocar) |
| Saltar a una sección | Abre «seccion » para escribir su nombre: su primera entrada en la línea de tiempo empieza en el compás siguiente | — |
| Repetir la sección | La sección que suena se repite en vez de pasar a la siguiente; otra vez, vuelve a la línea de tiempo | l (modo tocar) |
| Volver a la línea de tiempo | Deja de repetir la sección: al acabar, sigue la siguiente | — |
| Resaltado en vivo | Enciende o apaga el resaltado en el código de los pasos que suenan (al arrancar, el de [resaltado] activo) | — |
| Alternar variante A/B | Con variant(): pasa a la otra variante; entra entera al empezar el compás siguiente (con secciones, por la misma sección y compás) | v (modo tocar) |
| Versiones | El historial del script (una versión por carga buena): lista con fecha y vista previa; Enter la restaura en el editor sin guardar (Ctrl-Z lo deshace) | — |
| Choques de frecuencia | Renderiza offline cada pista en solo (un ciclo entero de la canción) y muestra en qué bandas se pisan: tabla pareja × banda con su puntuación (0 a 1), un resumen y el EQ sugerido para la que cede; Enter lo propone y Enter otra vez lo inserta en el script sin guardar; no toca lo que suena | — |
| Abrir el editor en su ventana | Saca el editor a una terminal nueva ([editor] terminal en config.toml) con su texto, cursor e historial de deshacer; con el editor ya fuera, lo vuelve a abrir si no está conectado | — |
| Volver a meter el editor en la principal | Con el editor en su ventana: lo cierra y vuelve a la principal con su texto, cursor e historial de deshacer (sin guardar) | — |
| Salir | Sale (pide confirmación si hay cambios sin guardar) | Ctrl-Q |
| Snippet: `amen`, `fill`, `ghost`, `intro`, `euclid`, `melodia`, `polirritmo`, `barrido`, `groove`, `instrumento` | Inserta el snippet en el cursor (ver «Snippets» en la sección 4) | — |

<!-- paleta:fin -->

### Autocompletado

Mientras escribes, junto al cursor sale una lista con lo que puede ir ahí,
filtrada por lo que llevas escrito, con la firma y la descripción de la
opción elegida. F4 la abre aunque no hayas escrito nada.

| Tecla | Qué hace |
| --- | --- |
| ↑ ↓ | Elegir opción |
| Tab | Aceptar |
| Enter | Aceptar, solo si has elegido con ↑ ↓; si no, cierra la lista y es un salto de línea normal |
| Esc | Cerrar (con la lista abierta, Esc solo la cierra: no entra en modo tocar) |
| Tab (tras aceptar una función o un snippet) | Saltar al siguiente campo y, tras el último, al final |

Qué ofrece según dónde esté el cursor:

| Dónde | Opciones |
| --- | --- |
| Inicio de línea | Funciones globales (`bpm`, `sample`, `track`), snippets (`amen`, `fill`, `ghost`, `intro`, `euclid`, `melodia`, `polirritmo`, `barrido`, `groove`), bloques de tu biblioteca (ver «Biblioteca de bloques»), variables locales y palabras clave de Lua |
| Tras `:` | Los métodos del objeto: de sample, de cortes, de pista o del compás de `every` (si no se sabe qué es, todos) |
| Dentro de `fx("...")` | Los efectos; dentro de la tabla, sus parámetros, y en `type = "..."` sus valores |
| Dentro de `cortes:fx("...")` | Primero las etiquetas de corte (también dentro de una lista `{ "...", 3 }`), luego los efectos y luego los parámetros de su tabla |
| Dentro de `sample("...")` | Los WAV de la carpeta de muestras (la del proyecto o la del script) |
| Dentro de `track("...")` | Las pistas ya definidas en el script |
| Dentro de `chance(p, "...")` | Los efectos de `chance` |
| Dentro de `steps("...")` | Las etiquetas de corte, al escribir un paso que empieza por letra (nada tras un número o en un sufijo) |
| Otras cadenas y comentarios | Nada |

Al aceptar una función se inserta con sus parámetros de ejemplo y el
primero queda marcado: lo que escribas lo sustituye. Por ejemplo, tras
`track("drums"):f` y Tab queda
`fx("filter", { type = "lowpass", cutoff = 3000 })` con `filter` marcado;
Tab pasa a `lowpass`, luego a `3000` y luego al final de la línea. Si
sobre `filter` escribes otro efecto y lo aceptas (por ejemplo `b` y Tab
para `bitcrush`), la tabla se rehace con los parámetros del nuevo
(`{ bits = 6, downsample = 2 }`) y queda marcado el primero. Los
parámetros que tienen los dos efectos conservan su valor (de `lowpass` a
`highpass`, `cutoff` y `q` siguen igual) y solo los nuevos salen con su
valor por defecto; lo mismo al
elegir otro efecto con F4 dentro de un `fx("...", { ... })` ya escrito.
Un efecto escrito entero deja la lista abierta para poder aceptarlo así. Los
snippets funcionan igual, en varias líneas (las nuevas llevan la sangría de
la actual); están en la sección 4 y también en la paleta. Aceptar algo se
deshace de una vez con Ctrl-Z. Las
funciones, efectos y parámetros salen del mismo registro que la sección 4
de esta guía.

### Editar los efectos de un paso desde el tracker

«Editar los efectos de un paso en el tracker» (paleta, F1) pone un cursor
en el tracker, en la pista seleccionada y el paso que suena: ↑↓ cambian de
paso y ←→ de pista. **Enter** abre la línea de órdenes con los efectos que
tiene el paso (`efectos lp=800,q=4 crush=6`): se cambian, se añaden o se
borran y Enter los escribe; cada efecto va como dentro de sus llaves y
separado por espacios, o con las llaves (`{lp=800}{crush=6}`); vacía, los
quita. **Supr** quita los efectos del paso. **Esc** sale.

El cambio se escribe en el código, en el token de ese paso de su
`steps("...")` (solo cambian sus caracteres: `2` pasa a ser
`2{lp=800,q=4}{crush=6}`; los sufijos y lo demás del texto se quedan
como estaban), es un solo paso de deshacer (Ctrl-Z) y se carga como con
Ctrl-L: entra en el compás siguiente. Una errata no toca el texto (la
línea de estado dice por qué). No se puede editar desde aquí un paso que
no está escrito tal cual (sale de `euclid`, `rotate`, `chance`, una cadena
hecha a trozos...), un silencio, una pista de instrumento o de granos, ni
nada si el texto ya no es el cargado (carga antes). Con `every`, se edita
la vuelta que se ve (si sale de un `c:steps("...")`, ese texto). Con el
editor en su ventana, se edita desde la principal y el cambio llega al
editor como un solo paso de deshacer.

### Modo tocar (Esc)

| Teclas | Cortes de la pista seleccionada |
| --- | --- |
| 1 2 3 4 5 6 7 8 9 0 | cortes 1 a 10 |
| q w e r t y u i | cortes 11 a 18 |
| ← → | recorre los cortes de la pista y preescucha cada uno |
| espacio | preescucha otra vez el corte seleccionado |
| , . | adelanta o retrasa el inicio del corte seleccionado 5 ms |
| < > | lo mismo, de 1 en 1 ms |
| m | mute de la pista seleccionada (en vivo, sin esperar al compás) |
| s | solo de la pista seleccionada (en vivo) |
| [ ] | con `arrange()`: salta a la sección anterior o siguiente de la línea de tiempo (en el compás siguiente) |
| l | con `arrange()`: repite la sección que suena; otra vez, vuelve a la línea de tiempo |
| v | con `variant()`: pasa a la otra variante, A o B (en el compás siguiente) |

El corte suena al instante (no espera al compás), con el volumen, los
efectos, la forma del corte (`cortes:set`) y el grupo de choke de la
pista, y corta lo que sonaba en ella. El texto no se toca.

**En una pista con instrumento** (`sample:instrument()`), las mismas teclas
tocan notas: `1` es la raíz y cada tecla, un semitono más (`2` un
semitono, `8` una quinta, `q` una séptima menor...). La nota arranca su
envolvente al pulsar y hace el release al soltar, sin cortar las que
suenan (con el robo de voces de siempre). Saber cuándo se suelta una tecla
depende de la terminal: si admite el protocolo de teclado de kitty (kitty,
foot, WezTerm, Ghostty), la nota dura hasta que la sueltas; si no (tmux y
la mayoría), suena mientras llegan las repeticiones de la tecla
mantenida y se suelta cuando dejan de llegar (unos 0,7 s tras una
pulsación corta, 0,15 s tras la última repetición). Volver al editor o
cambiar de pista suelta las notas de los pads. Una pista con un
sintetizador (`synth("acid")`) se toca igual, desde su raíz; el acid es
monofónico, así que una tecla nueva vuelve a empezar la nota que suena.

**En una pista con granos** (`pista:grains()`, `pista:freeze()`), cada tecla
abre una nube de granos que dura mientras la mantienes (se suelta como las
notas de un instrumento: con el aviso de la terminal o cuando dejan de llegar
las repeticiones) y no tiene que sonar la canción. La tecla elige:

- **con varios cortes, el corte** de donde salen los granos (`2` es el corte
  2, como el número de un paso);
- **con un solo corte** (lo normal en una textura, `slice(1)`), **el pitch**:
  `1` deja la nube como es y cada tecla sube un semitono (`2` uno, `8` una
  quinta, `e` una octava). Por MIDI, la nota menos la nota base (36), también
  hacia abajo.

El resto (tamaño, densidad, posición, `jitter`, `spread`...) es lo que diga
el script, y lo que cambies entra al compás como siempre, aunque la tecla
siga apretada. Hay una nube por pista: la última tecla manda, y soltar la
otra no la corta. El azar sale de `seed`, así que tocar lo mismo suena igual
(cada pulsación, un poco distinta de la anterior). Con la velocidad MIDI como
volumen. «Grabar lo tocado» las apunta como `agudo _ _` (el corte y lo que
duró). Una pista sin granos sigue tocando el corte tal cual.

La **preescucha** (← →, espacio, o «Preescuchar el corte seleccionado» en
la paleta) toca el corte en una voz aparte: no corta lo que suena en las
pistas, no pasa por su volumen, efectos ni mute, suena también con el
secuenciador parado y no toca el script. Sirve para buscar el corte que
quieres (y etiquetarlo o ajustarlo, ver más abajo).

El **ajuste manual de cortes** (`,` `.` `<` `>`, o «Adelantar/Retrasar el
corte 5 ms» en la paleta) mueve el inicio del corte seleccionado y lo
preescucha. Se guarda en el proyecto como posición absoluta (`corte =
amen.wav 8 3 412.5`) y manda sobre el detector: si el detector se equivoca
o corta tarde un golpe, se corrige a oído. El corte anterior acaba donde
empieza el ajustado, y no puede acercarse a menos de 1 ms de sus vecinos.
La vista del sample marca los cortes ajustados con `*` y da la posición
del seleccionado; «Quitar el ajuste del corte» lo devuelve al detector. En
Lua, `cortes:move(i, ms)`.

**Grabar lo tocado.** «Grabar lo tocado» (paleta) deja escrita la orden
`grabar 1`; cambia el número por los compases que quieras (Enter). La
grabación empieza al principio del compás siguiente (el panel del motor
dice `● REC drums: al compás` y luego `● REC drums 1/2`) y registra los
cortes que disparas en la pista seleccionada, con las teclas del modo
tocar o por MIDI, cada uno en el paso más cercano (un golpe hasta medio
paso antes del compás cuenta como el primero; dos en el mismo paso, el
último). Al acabar escribe en el cursor del editor `steps("...")` con lo
tocado: cada corte por su etiqueta o su número, con `@volumen` si no fue a
tope (la velocidad MIDI), agrupado por tiempos. Con más de 16 pasos (dos
compases, o otro compás), escribe `length(32):steps("...")`. Se deshace con
un Ctrl-Z. «Grabar lo tocado» otra vez para antes y escribe lo que haya;
parar el secuenciador la cancela. Pon el cursor detrás de
`track("drums"):` antes de grabar y Ctrl-S lo carga.

«Partir el corte seleccionado» y «Unir el corte seleccionado con el
anterior» (paleta) añaden un corte en la mitad del seleccionado o quitan
su marca. Se guardan en el proyecto (`partir`, `unir`) y las etiquetas y
ajustes de los cortes siguientes se corren con ellos. En Lua,
`cortes:add(ms)` y `cortes:remove(i)`.

`m` y `s` (o «Mute de la pista» y «Solo de la pista» en la paleta) son el
mute y el solo en vivo: entran ya, con un fundido de 5 ms para que no
haya clic, y las voces siguen su curso (al quitar el mute suena por donde
iba). Se suman a `:mute()` y `:solo()` del script, que entran al compás.
Mientras alguna pista está en solo, solo suenan las que lo están; un mute
gana a un solo. El tracker marca la pista con `M` o `S` y atenúa las que
no se oyen; el panel de efectos pone `MUTE`, `SOLO` o «(solo de otra)».
Sobreviven a las recargas (van por el nombre de la pista).

### Línea de órdenes (F2)

Enter ejecuta y Esc cancela. Las rutas son relativas a la carpeta del
proyecto o, si no hay, a la del script.

| Orden | Qué hace |
| --- | --- |
| `exportar [archivo.wav] [compases] [metronomo]` | Renderiza la mezcla a WAV. Por defecto `<script>-mezcla.wav` y la canción entera (con `arrange()`, la línea de tiempo hasta 256 compases; si no, el ciclo de `every`/`chance`, al menos 4 compases). Con `metronomo`, lleva el clic; si no, nunca |
| `guardar [proyecto.sampler]` | Guarda el script y el proyecto |
| `abrir proyecto.sampler` | Abre un proyecto (`abrir!` descarta los cambios sin guardar) |
| `muestras [carpeta]` | Cambia la carpeta de los samples (sin argumento, dice cuál es) |
| `bpm N` | Cambia la línea `bpm(...)` del script por `bpm(N)` (o la añade al principio), sin guardar |
| `etiquetar corte [nombre]` | Pone una etiqueta a un corte de la pista seleccionada (sin nombre, la quita). Se guarda en el proyecto abierto y se recarga |
| `grabar [N]` | Graba N compases (1 por defecto) de lo que tocas en la pista seleccionada, desde el compás siguiente, y lo escribe como `steps("...")` en el cursor (ver «Modo tocar») |
| `remuestrear [pista\|master] [compases] [archivo.wav]` | Graba por render offline la salida de la pista (por defecto la seleccionada, en solo) o del master como un WAV en la carpeta de samples (`<pista>-remuestreo.wav`; con `!`, sustituye uno que ya exista). Sin compases, lo que tarda en repetirse |
| `seccion nombre` | Con `arrange()`: salta a la primera entrada de esa sección en la línea de tiempo, en el compás siguiente (y sigue desde ahí) |
| `linea N` | Lleva el cursor al principio de la línea N (más allá del final, a la última). Es lo que abre Alt-G |
| `buscar texto` | Busca el texto tal cual (sin mayúsculas, no las distingue), resalta todas las veces que sale y va a la primera desde el cursor; sin texto, quita el resaltado. Es lo que abre Ctrl-F; F6 y F7, la siguiente y la anterior |
| `bloque nombre [etiquetas] [-- descripción]` | Guarda el texto seleccionado como bloque en la biblioteca global (`bloque-proyecto`: en la del proyecto; con `!`, sustituye uno que se llame igual). Ver «Biblioteca de bloques» |

### Biblioteca de bloques

Los bloques son trozos de script tuyos que guardas para reutilizar: una
pista de ghost notes que te gusta, una cadena de efectos, un fill... Se
insertan **como texto** en el editor, igual que los snippets: el script
nunca carga ni enlaza otros archivos, así que si luego cambias o borras el
bloque, las canciones donde lo pegaste no cambian.

Viven en dos carpetas, como archivos `.lua`:

- **Global**, para todos los proyectos: `~/.config/sampler/blocks/` (o
  `$XDG_CONFIG_HOME/sampler/blocks/`). Se crea sola al abrir la interfaz.
- **Del proyecto**: `blocks/` junto al proyecto (o junto al script si no
  hay proyecto). Se crea al guardar el primer bloque en ella. Si un bloque
  del proyecto se llama como uno global, se usa el del proyecto.

Cada archivo empieza con una cabecera de comentarios; lo demás es el
texto del bloque. Los `${...}` son campos: al insertarlo, el primero queda
marcado y Tab salta al siguiente, como en los snippets.

```text
-- nombre: fantasma
-- descripción: Ghost notes suaves en contratiempos
-- etiquetas: ghost, hats

track("${ghost}", amen:slice_at(${3}))
  :steps(". x@0.2 . .  . x@0.2 . .  . x@0.2 . .  . . x@0.2 .")
  :chance(${0.4}, "mute")
```

Sin `nombre`, el bloque se llama como el archivo. Los nombres llevan
letras, números y `_`, y empiezan por letra (así se pueden escribir en el
autocompletado). Puedes escribirlos o editarlos con cualquier editor: la
interfaz relee las carpetas cada medio segundo. Un archivo que no se puede
leer (nombre no válido, vacío, repetido) se avisa al abrir la lista y no
impide usar los demás.

Desde la paleta (F1):

| Acción | Qué hace |
| --- | --- |
| Guardar la selección como bloque | Abre la línea de órdenes con `bloque `: escribe el nombre, las etiquetas y, tras `--`, la descripción. Ejemplo: `bloque fantasma ghost hats -- Ghost notes suaves`. Se guarda sin la sangría común de las líneas. Para la carpeta del proyecto, cambia la orden a `bloque-proyecto`; para sustituir uno que ya existe, `bloque!` |
| Insertar un bloque | Lista de bloques con su carpeta y sus etiquetas; escribe para filtrar por nombre, `#etiqueta` o palabras de la descripción. Abajo, la descripción y el texto del elegido. Enter lo inserta |
| Actualizar un bloque con la selección | Elige un bloque y su texto pasa a ser el seleccionado (conserva nombre, descripción y etiquetas). Pide un segundo Enter |
| Borrar un bloque | Borra su archivo. Pide un segundo Enter. Si era del proyecto y tapaba a uno global, vuelve a verse el global |

Los bloques salen también en el **autocompletado**, al inicio de línea,
junto a los snippets (con el tipo «bloque»). Para crear campos desde el
editor, escribe `${valor}` en el texto antes de guardarlo como bloque.

Si pegas un bloque con Lua no válido, no se rompe nada: al cargar sale el
error de siempre en el editor y sigue sonando lo anterior.

#### Ranuras rápidas (1-9)

Para mover trozos al vuelo sin ponerles nombre, como los registros de vim:

- «Guardar en una ranura (1-9)» y luego un número guarda la selección o,
  sin selección, la línea del cursor entera.
- «Pegar una ranura (1-9)» y luego un número la pega. Las líneas enteras
  van encima de la línea del cursor (como `P` en vim); un trozo, en el
  cursor. Se deshace con un Ctrl-Z.

Mientras espera el número se ve lo que hay en cada ranura; cualquier otra
tecla cancela. Las ranuras se guardan de una sesión a otra en
`~/.config/sampler/ranuras/` (`1.txt` a `9.txt`, texto tal cual: también
se pueden editar a mano). Para guardar algo con nombre y etiquetas, hazlo
bloque. Con F1 Enter se repite la última
acción de la paleta, así que pegar otra vez es F1, Enter y el número.

### Historial de versiones

Cada vez que una carga sale bien (Ctrl-S, Ctrl-L, recargar del disco), el
texto se guarda como una versión con su fecha en
`~/.local/state/sampler/historial/<script>-<id>/` (o `$XDG_STATE_HOME`),
fuera del proyecto y de git. Si el texto es igual que la última versión,
no se repite; las cargas con errores no cuentan. Se guardan como mucho 200
versiones y 20 MB por script: al pasarse, se borran las más antiguas.

«Versiones» en la paleta abre la lista, de la más nueva a la más antigua,
con la fecha (UTC), cuánto hace, sus líneas y la vista previa del texto;
escribir filtra por la fecha o por el texto. Enter la restaura: la pone en
el editor como un cambio más (Ctrl-Z lo deshace entero) y no toca el
archivo hasta que guardes. Si el disco falla (sin sitio, sin permisos), se
avisa en la línea de estado y todo lo demás sigue igual.

### Configuración: teclas y colores

`~/.config/sampler/config.toml` (o `$XDG_CONFIG_HOME/sampler/config.toml`)
cambia los atajos y los colores. Si falta o está roto, la interfaz usa los
de siempre y lo dice al arrancar; lo que no entiende lo avisa y lo ignora.

```toml
[teclas]
guardar = "alt-s"            # una tecla
marcha = ["ctrl-t", "f8"]    # o varias
paleta = "ctrl-p"            # además de F1

[colores]
acento = "#ff8700"           # nombre, #rrggbb o número de la paleta de 256
sonando = 22
```

La sección `[editor]` tiene `terminal`, el comando que abre el editor en
su ventana con `--split` (ver «El editor en su propia ventana»).

Las teclas son `ctrl-` y/o `alt-` con una letra o un número, `f1` a `f12`
o `esc`. Una letra sola no vale (escribiría texto). F1 abre siempre la
paleta, aunque se le dé otra tecla: así ninguna configuración la deja
fuera de alcance. Si una tecla va a dos acciones, se avisa y se queda con
la primera; si la tecla de siempre de una acción la coge otra, esa acción
se queda con las demás que tenga. La paleta y la línea de ayuda enseñan
las teclas que hay. Los colores `#rrggbb` se ven tal cual en una terminal
con truecolor (`COLORTERM=truecolor`); si no, como el más parecido de la
paleta de 256.

<!-- config:inicio (generado desde tui/src/config.rs) -->

| En `[teclas]` | Acción | Por defecto |
| --- | --- | --- |
| `paleta` | Abrir la paleta de comandos (F1 la abre siempre) | `f1` |
| `guardar` | Guardar y cargar | `ctrl-s` |
| `cargar` | Cargar sin guardar | `ctrl-l`, `f5` |
| `marcha` | Marcha / paro | `ctrl-t`, `f8` |
| `ir_al_error` | Ir al error | `ctrl-g` |
| `ir_a_linea` | Ir a la línea | `alt-g` |
| `buscar` | Buscar | `ctrl-f` |
| `buscar_siguiente` | Buscar el siguiente | `f6` |
| `buscar_anterior` | Buscar el anterior | `f7` |
| `siguiente_pista` | Siguiente pista | `ctrl-o` |
| `recargar` | Recargar del disco | `f3` |
| `ordenes` | Línea de órdenes | `f2` |
| `completar` | Autocompletar | `f4` |
| `salir` | Salir | `ctrl-q` |
| `modo_tocar` | Modo tocar | `esc` |
| `deshacer` | Deshacer | `ctrl-z`, `ctrl-u` |
| `rehacer` | Rehacer | `ctrl-r` |
| `explorador` | Explorador de samples | — |
| `tap_tempo` | Tap tempo | — |
| `metronomo` | Metrónomo | — |
| `exportar` | Exportar la mezcla | — |
| `seccion_siguiente` | Saltar a la sección siguiente | — |
| `seccion_anterior` | Saltar a la sección anterior | — |
| `repetir_seccion` | Repetir la sección | — |
| `variante` | Alternar variante A/B | — |
| `resaltado` | Resaltado en vivo | — |

| En `[colores]` | Para |
| --- | --- |
| `acento` | bordes, la pista elegida, la paleta |
| `tenue` | texto secundario |
| `error` | errores |
| `linea_error` | fondo de la línea con error |
| `aviso_pasos` | paso con error en `steps("...")` |
| `aviso` | avisos (limitador, cambios fuera del editor) |
| `onda` | forma de onda |
| `cortes` | marcas de los cortes |
| `sonando` | fondo del paso que suena en el tracker |
| `nivel` | medidores de nivel |

| En `[resaltado]` | Para | Por defecto |
| --- | --- | --- |
| `activo` | resaltar lo que suena al arrancar (`true`/`false`; la paleta lo cambia en vivo) | `true` |
| `estilo` | `"fondo"`, `"subrayado"` o `"invertido"` | `"fondo"` |
| `colores` | un color o una lista, uno por pista en el orden del script (si hay menos, se repiten) | 8 de la paleta de 256 |
| `retardo_ms` | ms que se suman a la latencia que dice la salida de audio (de 0 a 1000) | `0` |

<!-- config:fin -->

### MIDI

Con `--allow-midi` (`tui --edit canción.lua --allow-midi`), la interfaz
abre dos puertos MIDI virtuales de ALSA. A **sampler entrada** conéctale un
teclado, unos pads o un secuenciador con `aconnect` (`aconnect -l` lista
los puertos; `aconnect "Mi teclado" "sampler"`) o desde su programa. Cada
nota dispara un corte de la pista seleccionada, igual que los pads del
modo tocar (con su forma, su choke y su volumen), y la velocidad de la
nota es el volumen del golpe. La nota 36 (el bombo de General MIDI) es el
corte 1, la 37 el 2... Si la pista seleccionada es un instrumento, cada
nota suena a su altura real respecto a la raíz (`root = "A3"` y la nota 64,
E4, suenan siete semitonos por encima) y dura hasta su note off, que
dispara el release; `nota_base` no cuenta. Se puede cambiar en la
configuración:

```toml
[midi]
nota_base = 60     # la nota del corte 1
canal = 10         # solo este canal (sin él, todos)
reloj = true       # el reloj MIDI pone el tempo
```

Con `reloj = true`, el reloj MIDI que llegue (24 pulsos por negra) pone el
tempo: se calcula con los pulsos de las últimas cuatro negras (por mínimos
cuadrados, así el jitter no lo mueve) y solo cambia si cambia de verdad;
entra al empezar el compás, como un `bpm()`. Start y Continue arrancan el
secuenciador y Stop lo para. Sin `--allow-midi` no se abre nada.

**Salida hacia un synth (Surge XT).** Con `--allow-midi` también se abre
**sampler salida**. Una pista con `:midi(canal, nota_base)` manda cada
golpe como una nota: el corte 1 es `nota_base`, el 2 la siguiente... La
velocidad sale del volumen del paso y cada nota dura lo que su paso. En
un instrumento va sin `nota_base` (`:midi(2)`): cada nota, también las de
un acorde, sale a su altura (la raíz más sus semitonos) y dura su paso y
sus `_`. El sample sigue sonando; para oír solo el synth, `:gain(0)`. Para tocar con
Surge XT:

1. Abre Surge XT (la versión independiente, o en tu DAW) y, en sus
   opciones de audio y MIDI, elige como entrada **sampler salida**. O
   conéctalos a mano: `aconnect -l` lista los puertos con sus números y
   `aconnect 128:1 129:0` une el del sampler (cliente:puerto) con el de
   Surge.
2. Pon en Surge el canal que use la pista (o que escuche todos).
3. Para grabar lo que suena en Surge por PipeWire: `wpctl status` o
   `pw-cli ls Node` dan el nombre de su nodo, y `pw-record --target
   "Surge XT" surge.wav` lo graba a un WAV, que luego puedes cortar como
   cualquier otro sample.

Las notas salen en cuanto el motor las da, con su frame exacto; el retraso
y el jitter son los del bloque de audio y del bucle de la interfaz (unos
pocos ms). No lo he podido probar con dispositivos reales: la lógica está
probada con un destino simulado.

### Editor

Los del widget de texto (`ratatui-textarea`): flechas para moverse, Tab
indentar (salvo con el autocompletado o un parámetro pendiente), Ctrl-Z o
Ctrl-U deshacer, Ctrl-R rehacer, Ctrl-C copiar, Ctrl-X cortar, Ctrl-Y pegar,
Ctrl-K borrar hasta el final de la línea, Ctrl-W borrar la palabra
anterior.

**Moverse.** Con Shift, todas seleccionan desde donde estaba el cursor:

| Tecla | Qué hace |
| --- | --- |
| Ctrl-← / Ctrl-→ | Palabra anterior / siguiente (con Shift, selecciona por palabras) |
| Inicio / Fin (o Ctrl-A / Ctrl-E) | Principio / final de la línea |
| Ctrl-Inicio / Ctrl-Fin | Principio / final del texto |
| Alt-G | Ir a la línea N (abre `linea ` en la línea de órdenes; Ctrl-G sigue siendo ir al error) |
| Ctrl-↑ / Ctrl-↓ | Párrafo anterior / siguiente (hasta la línea en blanco) |
| RePág / AvPág (o Alt-V / Ctrl-V) | Una pantalla arriba / abajo |

**Buscar.** Ctrl-F abre `buscar ` en la línea de órdenes: escribe el
texto y Enter lleva el cursor a la primera vez que sale desde donde está y
resalta todas; el mensaje dice cuál es y cuántas hay (`«track»: 2 de 5`).
F6 va a la siguiente y F7 a la anterior (dan la vuelta al llegar al final
o al principio). El texto se busca tal cual, no es una expresión regular
(`fx("` vale), y si no tiene mayúsculas no las distingue (`track` encuentra
`Track`; `Track`, solo `Track`). `buscar` sin texto quita el resaltado.
Ctrl-F era en el widget un sinónimo de → (al estilo de emacs); → sigue
igual. Las tres teclas se cambian en `[teclas]` (`buscar`,
`buscar_siguiente`, `buscar_anterior`).

**Portapapeles del sistema.** Ctrl-C y Ctrl-X mandan la selección
también al portapapeles del sistema: por OSC 52 (la terminal lo guarda;
funciona por SSH, y dentro de tmux con `set -g set-clipboard on`) y, si
está instalado, por `wl-copy` (Wayland). Ctrl-Y pega lo que haya en el
portapapeles del sistema si `wl-paste` está instalado y contesta en un
cuarto de segundo (se deshace de una vez, como un pegado); si no, lo
último copiado en el editor. Sin ninguno de los dos, todo funciona igual
que siempre dentro del editor.

Pegar desde el terminal (Ctrl-Shift-V, el botón central o el menú del
terminal) pone el texto tal cual en el cursor: con sus líneas y su
sangría, sin abrir el autocompletado, sin aceptar la opción que tenga
abierta y sin disparar atajos ni plantillas. Un solo Ctrl-Z lo deshace
entero. La interfaz pide al terminal el pegado entre corchetes (bracketed
paste) al entrar y lo quita al salir; si el terminal no lo sabe hacer, una
ráfaga de teclas con saltos de línea también se toma como un pegado. En la
línea de órdenes (F2) solo entra la primera línea, y en el modo tocar no
se pega nada.

### Resaltado en vivo

Con el secuenciador en marcha, el paso de cada `steps("...")` que se oye
se resalta en el propio código, con el color de fondo de su pista (como en
Strudel). La pista seleccionada (Ctrl-O) va con el color entero y el texto
en negro y negrita; las demás, con su color apagado. Un `_` deja resaltado
el golpe que alarga mientras dura, y un retrigger (`2r4`) o un acorde se
resaltan enteros todo su paso. Los silencios (`.`) no se resaltan.

- Sigue a lo que se oye, no a lo que calcula el motor: descuenta la
  latencia de la salida de audio.
- Solo mientras el texto del editor es el de la última carga buena: al
  escribir algo deja de resaltar hasta que se carga (o se deshace el
  cambio). Tras cargar, empieza en el compás siguiente, cuando entra lo
  nuevo.
- Se resalta lo escrito tal cual en el código: la cadena de `steps`,
  `melody` o `c:steps` dentro de `every` (también en una variable:
  `local p = "..."` con `:steps(p)`, si no hay otra cadena igual). Con
  `every`, `fill` y `chance`, el paso del patrón escrito en esa posición.
  Lo que sale de una función (`euclid`, `rotate`, `mutate`, `shuffle`,
  `reverse_pattern`) o de juntar cadenas no está escrito: esa pista no se
  resalta.
- Varias pistas en la misma línea, cada una con su color; si dos marcan
  el mismo texto (una cadena compartida), manda la seleccionada y, si no,
  la definida antes.
- Con el editor en su propia ventana (`--split`) también se ve: lo calcula
  la principal y se lo manda al editor. Las dos ventanas tienen que ser de
  la misma versión del programa.

«Resaltado en vivo» en la paleta (F1) lo apaga y lo enciende. En
`config.toml` (ver «Configuración»), la sección `[resaltado]`:

```toml
[resaltado]
activo = true                      # encendido al arrancar
estilo = "fondo"                   # o "subrayado", o "invertido"
colores = ["#ff5faf", "#5fd7ff"]   # uno por pista (si hay menos, se repiten)
retardo_ms = 0                     # ms de más si se ve antes de oírse
```

Con `subrayado`, el token se subraya en el color de su pista (y la
seleccionada lleva también el texto de ese color, en negrita); con
`invertido`, el color de la pista hace de fondo y el del fondo, de texto.
Un recuadro alrededor del token no se puede dibujar en una terminal (cada
celda es un carácter), así que no hay ese estilo. `retardo_ms` sirve si
la salida de audio dice menos latencia de la que tiene (Bluetooth, una
tarjeta externa): el resaltado se adelanta a lo que se oye. Con el editor
en su ventana, cuentan el `retardo_ms` de la principal y el «Resaltado en
vivo» de cada ventana.

## 3. Mini-notación

Un patrón son **16 pasos** (semicorcheas de un compás de 4/4) separados por
espacios. Los espacios de más no cuentan, así que se pueden agrupar por
tiempos.

Cada pista puede tener **otra longitud** con `pista:length(n)` (de 1 a 64
pasos, antes de `:steps()`): sus pasos siguen siendo semicorcheas, pero el
patrón da la vuelta cada `n`. Una pista de 12 pasos contra una de 16 hace
polirritmia (3 contra 4: vuelven a coincidir cada 3 compases) y una de 32
ocupa dos compases. Las vueltas van alineadas con la canción desde que
arranca el secuenciador, así que recargar no las desplaza; los cambios
siguen entrando al empezar un compás.

```lua
local amen = sample("amen.wav"):slice(8)
track("drums", amen):steps("1 . 2 .  3 . 4 .  5 6 2 .  7 . 8 .")
track("tres", amen):length(12):steps("3 . . 3@0.5  . . 3 .  . 3@0.5 . .")
```

En una pista así, `every`, `fill`, `chance` y `humanize` cuentan vueltas
de su patrón en vez de compases, y el swing sigue cayendo en los
contratiempos de la canción (con una longitud impar, una vuelta empieza en
tiempo y la siguiente a contratiempo). El tracker tiene tantas filas como
la pista más larga y resalta el paso de cada pista; el panel del motor
dice `12 pasos` junto a las pistas que no son de 16. Exportar la canción
entera dura hasta que todas las pistas vuelven a coincidir (como mucho 256
compases).

| Símbolo | Significado | Ejemplo |
| --- | --- | --- |
| `3` | Corte 3 (se cuenta desde 1) | `3` |
| `.` | Silencio: no dispara nada; lo que sonaba sigue sonando | `.` |
| `_` | Alarga un paso más el golpe de antes: en el modo gate y en los instrumentos, la nota sigue sonando; en one-shot no cambia nada | `1 _ _ _` |
| `x` | Igual que `1` (cómodo con `slice_at`) | `x` |
| `kick` | El corte con esa etiqueta (ver `cortes:name` en la sección 4) | `kick`, `snare` |
| `'` | Reverse: el corte suena del final al principio | `3'` |
| `^n` | Pitch en semitonos (signo y decimales, de -36 a 36). Cambia también la duración | `3^7`, `3^-5`, `3^0.5` |
| `%n` | Pitch en semitonos que **no** cambia la duración: el corte suena transpuesto y dura lo mismo (granos alineados; bien hasta ±12, más allá avisa). Va en vez del primer `^`; detrás pueden ir las notas de un acorde | `3%7`, `3%-12`, `1%0^4^7` |
| `^a^b^c` | Acorde: varios `^` en un paso suenan a la vez (hasta 4 notas; solo en pistas con instrumento) | `1^0^4^7` |
| `@g` | Volumen del paso (lineal, de 0 a 4) | `3@0.5` |
| `rN` | Retrigger / stutter: el paso se divide en N partes y el corte vuelve a empezar en cada una (de 1 a 16) | `3r4` |
| `~Hz` | Corte del primer filtro de la pista desde este golpe hasta el siguiente (parameter lock, de 20 a 20000 Hz); un golpe sin `~` vuelve al corte de `:fx` | `3~800` |
| `{efecto=valor,clave=valor}` | Un efecto solo en este golpe, con sus parámetros; varios, cada uno en sus llaves | `3{lp=800,q=4}{crush=6}` |
| `!` | Accent (solo en un sintetizador): la nota suena más fuerte (en el acid, también más abierta) | `1!`, en `melody` `c!` |
| `/` | Slide (solo en un sintetizador): la nota llega ligada desde la anterior, deslizándose hasta su altura sin volver a disparar las envolventes | `1^3/`, en `melody` `eb/` |
| `{sub=n,sub_div=n,...}` | El sub de esta nota (solo en un sintetizador): manda sobre el de su tabla mientras suena; claves `sub`, `sub_div`, `sub_wave`, `sub_pan` y `sub_drive`, en sus propias llaves (sin efectos) | `1{sub=0.8,sub_div=3}`, en `melody` `c{sub=0}` |

Los sufijos se combinan en cualquier orden, cada uno una vez (salvo `^`,
que repetido hace un acorde): `3'^7@0.5r2` es el corte 3, al revés, una quinta arriba, a media
intensidad y repetido dos veces.

**`^` o `%`.** `^` es el pitch de un sampler clásico: lee el corte más
deprisa o más despacio, así que al subir dura menos y al bajar, más (una
octava abajo, el doble). `%` transpone sin tocar la duración: la caja a
`%-12` suena una octava más grave y ocupa lo mismo que antes. Por dentro
lee el corte en granos de 30 ms que se solapan, cada uno a la velocidad
del pitch y alineado con el anterior para que no se cancelen; el primer
grano empieza en el golpe, así que el ataque queda en su sitio. Hasta ±12
semitonos va bien; más allá los golpes se emborronan y suena metálico (la
carga avisa en la línea de estado). Con `%0` suena exactamente igual que
sin nada. En un instrumento, `keep = true` hace lo mismo con todas sus
notas.

**Formantes.** Al transponer, con `^` o con `%`, la envolvente del sonido
(sus formantes: lo que hace que una vocal sea una «a» o el cuerpo de una
caja) se mueve con el tono: una voz subida suena a ardilla y una caja
bajada, de juguete. `pista:formant(0)` los deja donde estaban en los
pasos con `%`, y `pista:formant(n)` los desplaza `n` semitonos por su
cuenta (también con `%0`: otro timbre, la misma nota). En un instrumento
va en su tabla: `sample:instrument({ keep = true, formant = 0 })`. Cada
grano se analiza con LPC y se le cambia la envolvente; cuesta otro 2 % de
un núcleo por voz. Con `^` no actúa (el sampler clásico lo mueve todo), y
si la pista no tiene pasos con `%`, la carga avisa.

**Efectos por paso (`{...}`).** Cualquier efecto de `:fx`, solo en ese
golpe: `3{lp=800}` lo filtra, `3{crush=4,downsample=3}{drive=8}` lo
ensucia. Dentro de las llaves va el efecto (su nombre o un alias: `lp`,
`hp`, `crush`, `comp`, `drive`, `gain`, `eq`, `dyneq`, `filter`...), `=` y
el valor de su parámetro principal (el primer número de su tabla: `cutoff`,
`bits`, `threshold`, `drive`, `level`...) y, tras comas, los demás
`clave=valor` (`{filter,type=highpass,cutoff=300}`). Lo que no se da, por
defecto. Un valor puede ser una **rampa del golpe**: `3{lp=200>4000}` abre
el filtro durante el paso, `3{lp=200>4000:4}` durante 4 pasos, y luego se
queda arriba; con retrigger (`3r4{lp=200>4000}`) cada subdisparo sigue la
rampa por donde va. Dentro de las llaves puede haber espacios. El tracker
los enseña (`03{lp∿,crush}`; `∿`, lo que lleva rampa), el autocompletado
ofrece efectos y parámetros dentro de las llaves y el diagnóstico en vivo
subraya el paso con un efecto mal escrito.

```lua
local amen = sample("amen.wav"):slice(8)
track("drums", amen)
  :steps("1 . 2 .  3 . 4{crush=4}{lp=900,q=6} .  5 6 2r4{lp=300>9000} .  7 . 8{hp=2000} .")
```

Cada golpe con efectos los lleva en su voz, con su propio estado: los
demás golpes suenan exactamente igual que sin ellos (y la cola del
efecto, como mucho 100 ms, sigue sonando con su golpe). **Orden y
precedencia**: la voz (volumen del paso, envolvente y pan del corte) →
efectos del paso y del corte → efectos de la pista → volumen, pan y mute
de la pista → mezcla (envíos, buses y limitador). Entre el paso y su
corte manda el paso: un efecto del mismo tipo que uno de `cortes:fx`
(el mismo filtro, otro bitcrusher...) lo sustituye en ese golpe, y los
demás van detrás de los del corte; entre los dos, como mucho 4 efectos (si
no caben, error con la línea del `steps`). Los efectos de la pista no se
pueden quitar para un golpe (suenan sobre la suma de todos): para mover su
filtro en un golpe está `~`, que manda sobre la automatización de la
pista. En un **instrumento** o una pista con **granos** no hay voces de
corte, así que un efecto de paso da error con la línea (usa los de la
pista).

**`~` (corte por paso).** Necesita un filtro en la pista
(`:fx("lowpass", { cutoff = 3000 })`): `"1~300 . 2 .  1 . 2~8000 ."` cierra
el filtro en el primer golpe y lo abre del todo en la segunda caja. Mueve
siempre el primer filtro de la pista, aunque el corte tenga su propio
filtro con `cortes:fx`.

**Automatización.** Todos los números de la tabla de `:fx` (de la pista o
de un corte con `cortes:fx`), de `bus()` y de `:grains`, y el volumen, el
pan y los envíos de la pista (`:gain()`, `:pan()`, `:send()`), admiten
`ramp(desde, hasta, "4 compases")` (una rampa que se repite) o
`lfo(frecuencia, mín, máx)` (un seno) en lugar de un número. Lo que no
(tipos, `time` del delay, lo que se prepara al cargar...) da error con la
línea y el porqué: la tabla completa, «Qué se puede automatizar», está en
la sección 4, tras `lfo`. Las dos dependen solo de la posición de la
canción: son deterministas y recargar no las desplaza. Todo lo que se
mueve (curvas y `~`) se suaviza en unos 5 ms y el filtro recalcula sus
coeficientes cada 32 muestras: no hay clics. El panel de efectos marca los
parámetros que se mueven (`› LP 200 Hz q0.71 ∿cutoff`) y el tracker enseña
el `~` de cada paso.

**Con etiquetas**, los sufijos igual: `kick^7@0.5`, `snare'r2`. Un `r` y
un número pegados al final de la etiqueta son el retrigger (`kickr4` es
`kick` repetido 4 veces), por eso ninguna etiqueta puede acabar así.

Mientras escribes, el editor revisa cada `steps("...")` sin esperar a
guardar: el paso con error sale subrayado en rojo y el aviso (`⚠ línea N:
...`) aparece al pie del editor. Comprueba la notación, las etiquetas, que
haya tantos pasos como la longitud de la pista (16, o el número de
`:length(n)` escrito en la misma cadena de `track(...)` o en la que define
esa pista; si `n` no es un número escrito tal cual, no los cuenta) y los
rangos de volumen, pitch y retrigger; los cortes que no existen se ven al
cargar. Las etiquetas que conoce son las de lo último
cargado, las del proyecto y las que el texto pone con `:name()`, de
cualquier pista: si una etiqueta no es de esa pista, lo dice la carga.
Igual con `melody("...")`: subraya la nota que no se entiende (`h`, un
acorde de más de 4 notas, un `^`, un sufijo roto), los `_` sin nota antes,
los pasos de más o de menos y los rangos de volumen y retrigger; lo lejos
que queda una nota de la raíz (±36 semitonos) depende del instrumento, así
que lo dice la carga. Y en cada `:fx(...)` subraya un efecto que no
existe y, en `cortes:fx("...", ...)`, una etiqueta que no conoce nadie
(los números de corte que no existen los dice la carga).

Con etiquetas, el tracker muestra el nombre en vez del número (`kick^7`) y
la vista del sample pone cada etiqueta junto al número de su corte.

### Etiquetar cortes desde la interfaz

Sin tocar el script: en el modo tocar, toca el corte (queda seleccionado,
resaltado en la vista del sample) o elígelo con «Corte siguiente» / «Corte
anterior» de la paleta. Luego «Etiquetar el corte seleccionado» (F1) abre
la línea de órdenes con `etiquetar N ` escrito: pon el nombre y Enter. La
etiqueta se guarda en el proyecto abierto (si no hay, con `guardar`), vale
para todas las pistas de ese sample cortado igual y se recarga al momento.
Si el script ya nombra ese corte con `:name()`, manda el script y el
mensaje lo avisa.

Ejemplos:

```lua
bpm(170)
local amen = sample("amen.wav"):slice(8)
-- Básico, agrupado por tiempos.
track("a", amen):steps("1 . . 1  . . 3 .  1 . 5 .  . 3 . 6")
-- Con efectos por paso.
track("b", amen):steps("1 . 3' .  1^7 . 5@0.5 .  . 3r4 . 6  x . . 7'^-12@0.8")
```

## 4. API de Lua

El script se ejecuta entero en cada carga. Cada llamada construye el modelo
de la canción; nada suena hasta que la carga termina bien (ver la sección
5). Solo están las librerías `math`, `string`, `table` y `utf8` (no `io` ni
`os`), y un script que tarda más de 2 s se corta.

El resto de esta sección se genera a partir del registro de la API
(`script/src/api.rs`), el mismo que usa el autocompletado del editor. Para
cambiarla, edita el registro y ejecuta `cargo run -p script --example guia`.

<!-- api:inicio (generado desde script/src/api.rs) -->

### `bpm(n)`

Tempo en pulsos por minuto, de 20 a 400. Si no se llama, se mantiene el que haya (o el del proyecto).

- `n`: pulsos por minuto (20 a 400, admite decimales)

```lua
bpm(174)
```

### `sample(ruta [, opciones])`

Devuelve: sample.

Carga un WAV (y, con opciones, lo prepara: recorte, normalizado, fundidos). La ruta es relativa a la carpeta del script o, en un proyecto, a su carpeta `samples`. Recargar el script no vuelve a leer el archivo si no ha cambiado. Las opciones (tabla de abajo) preparan el sample al cargarlo, en este orden: recortar el silencio de los extremos, normalizar el pico y los fundidos; se guarda para la carga siguiente. Los cortes, los ajustes en ms y `stretch` van sobre el sample ya preparado (recortar desplaza los ms).

- `ruta`: archivo WAV (cualquier frecuencia; se resamplea al cargar)
- `opciones`: tabla con las de la tabla de abajo

| Parámetro | Qué es | Por defecto | Valores |
| --- | --- | --- | --- |
| `trim` | recorta el silencio del principio y del final: `true` (por debajo de -60 dB) o el nivel en dB | `false` | `true`, `false` o -120–0 |
| `normalize` | lleva el pico a 0 dB (`true`) o al nivel en dB que se diga | `false` | `true`, `false` o -60–0 |
| `fade_in` | fundido de entrada, en ms | `0` | 0–10000 |
| `fade_out` | fundido de salida, en ms | `0` | 0–10000 |

```lua
local amen = sample("amen.wav")
-- Sin el silencio de los extremos, con el pico a -1 dB y sin chasquido al final.
local limpio = sample("amen.wav", { trim = true, normalize = -1, fade_out = 10 })
```

### `track(nombre, cortes) / track(nombre)`

Devuelve: pista.

Define una pista que toca esos cortes, o recupera una ya definida. Con dos argumentos define (o redefine) la pista; con uno, recupera una pista ya definida para seguir configurándola. Hay como mucho 8 pistas; suenan a la vez y se suman. Todos los métodos de la pista devuelven la pista, así que se pueden encadenar.

- `nombre`: nombre de la pista; conserva su pista del motor entre recargas
- `cortes`: lo que devuelve `sample(...):slice(n)` o `:slice_at(i)`

```lua
local amen = sample("amen.wav"):slice(8)
track("drums", amen)
track("drums"):steps("1 . . .  3 . . .  1 . . .  3 . . .")
```

### `bpm_ramp(desde, hasta, duración)`

Rampa de tempo: de desde a hasta BPM en esos compases, y se queda en hasta. Manda sobre `bpm()`. Cada compás suena a su tempo (cuantizado al compás: el compás `k` desde que entra, a `desde + (hasta − desde)·k/n`), así que los pasos siguen cayendo al frame exacto; el delay desliza su tiempo para no hacer clic. Entra al empezar el compás, como todo cambio del script, y recargar el mismo script no la vuelve a empezar. Quitarla (con un `bpm()`) la para.

- `desde`: tempo del primer compás (20 a 400)
- `hasta`: tempo al acabar (20 a 400)
- `duración`: `"N compases"` (hasta 64)

```lua
-- De 140 a 174 en 16 compases.
bpm_ramp(140, 174, "16 compases")
local amen = sample("amen.wav"):slice(8)
track("drums", amen):steps("1 . 2 .  3 . 4 .  5 6 2 .  7 . 8 .")
```

### `meter(n, d)`

Compás n/d (3/4, 7/8...): cuántas semicorcheas tiene cada compás. Por defecto 4/4 (16 pasos). 3/4 son 12 pasos, 7/8 son 14, 5/4 son 20. Los cambios entran al empezar el compás. Las pistas siguen contando semicorcheas: una de 16 pasos en un 3/4 se desfasa; dale `:length(12)`. El metrónomo marca un pulso cada negra (cada corchea en x/8, cada semicorchea en x/16) con acento en el primero.

- `n`: pulsos del compás
- `d`: figura del pulso: 4, 8 o 16

```lua
meter(3, 4)
local amen = sample("amen.wav"):slice(8)
track("vals", amen):length(12):steps("1 . . .  2 . 3 .  2 . 3 .")
```

### `metronome([sí])`

Enciende (o, con false, apaga) el metrónomo: un clic en cada pulso. El primer pulso del compás lleva acento (más agudo y más fuerte). Es para tocar y escuchar: el render exportado no lo lleva salvo que se pida (`exportar ... metronomo`, o `--metronome` en la línea de comandos). También se enciende y apaga desde la paleta. Sin llamarlo, se queda como esté.

- `sí`: `true` (por defecto) lo enciende; `false` lo apaga

```lua
bpm(170)
metronome(true)
```

### `priority(pistas)`

Qué pista manda cuando dos chocan en frecuencia: la primera de la lista, la que más. Solo la usa el análisis de choques de frecuencia («Choques de frecuencia» en la paleta, `--analyze-masking`): en cada choque, la sugerencia de recorte va a la pista que cede. Una pista de la lista manda sobre las que van detrás y sobre las que no están; entre dos que no están, manda la que suena más fuerte en esa banda (y, si suenan igual, la que se definió antes). Sin llamarla, siempre manda la más fuerte. Un nombre que no es de ninguna pista da un aviso. No cambia cómo suena nada.

- `pistas`: lista de nombres, de la que más manda a la que menos

```lua
local amen = sample("amen.wav"):slice(8)
track("kick", amen:slice_at(1)):steps("x . . .  x . . .  x . . .  x . . .")
track("drums", amen):steps("1 . 2 .  3 . 4 .  5 6 2 .  7 . 8 .")
priority({ "kick", "drums" })
```

### `section(nombre, función)`

Una sección con nombre (intro, drop, break...): lo que cambian las pistas mientras suena. La función se llama al acabar el script, sobre una copia de las pistas definidas fuera de las secciones: dentro, `track("drums")` cambia esa pista solo en esta sección (patrón, efectos, `mute`, `solo`, `every`...) y `track("nueva", cortes)` añade una que solo suena aquí. Entre todas las secciones caben 8 pistas. Lo que es de toda la canción (`bpm`, `bpm_ramp`, `meter`, `bus`, `metronome`) va fuera. Dentro de una sección todo cuenta desde su primer compás: las vueltas de `every`, `fill`, `chance`, `humanize` y `:length()`, y las curvas de `ramp` y `lfo`. Las secciones se encadenan con `arrange()`; sin él suena solo la primera, como una canción sin secciones.

- `nombre`: el de la sección (único)
- `función`: `function() ... end` con lo que cambia

```lua
local amen = sample("amen.wav"):slice(8)
track("drums", amen):steps("1 . 2 .  3 . 4 .  5 6 2 .  7 . 8 .")
section("intro", function()
  track("drums"):fx("lowpass", { cutoff = ramp(300, 8000, "4 compases") })
end)
section("drop", function() end)
arrange({ {"intro", 4}, {"drop", 8} })
```

### `arrange({ {sección, compases}, ... })`

La línea de tiempo: las secciones en orden, cada una con sus compases. El motor toca la canción entera: al empezar cada compás mira en qué sección está y, si cambia, pone sus patrones (siempre al empezar el compás y todo junto). Al acabar vuelve a empezar. Recargar el script conserva la sección y el compás por los que iba; si esa sección ya no está, vuelve al principio con un aviso. Desde el modo tocar y la paleta se puede saltar a una sección, repetir la actual y volver a la línea de tiempo. Exportar sigue la línea de tiempo entera (hasta 256 compases). Una misma sección puede salir varias veces.

- `línea`: lista de `{"sección", compases}` (de 1 a 256 compases cada una)

```lua
local amen = sample("amen.wav"):slice(8)
track("drums", amen):steps("1 . 2 .  3 . 4 .  5 6 2 .  7 . 8 .")
section("intro", function() track("drums"):steps("1 . . .  . . . .  1 . . .  . . . .") end)
section("drop", function() end)
section("break", function() track("drums"):mute() end)
arrange({ {"intro", 4}, {"drop", 8}, {"break", 2}, {"drop", 8} })
```

### `variant(nombre, letra, función)`

Variante A o B de una sección o de una pista, para alternarlas en vivo. Con `nombre` de una sección, la función cambia sus pistas como dentro de `section()` (sin argumentos: `track("drums"):mute()`); con el de una pista de fuera de las secciones, la recibe (`function(t) t:steps("...") end`) y el cambio vale en todas las secciones donde suene. La letra es `"A"` o `"B"`; la que no se define es lo que está escrito. Al cargar se preparan las dos versiones de la canción y suena la A (al recargar, la que sonaba). «Alternar variante A/B» en la paleta o `v` en el modo tocar pasa a la otra: entra entera al empezar el compás siguiente y, con secciones, sigue por la misma sección y el mismo compás. Exportar graba la que suena.

- `nombre`: una sección o una pista de fuera de las secciones
- `letra`: `"A"` o `"B"`
- `función`: lo que cambia (con una pista, la recibe)

```lua
local amen = sample("amen.wav"):slice(8)
track("drums", amen):steps("1 . 2 .  3 . 4 .  5 6 2 .  7 . 8 .")
-- B: otro patrón para la misma pista; v en el modo tocar alterna.
variant("drums", "B", function(t)
  t:steps("1 . . 1  . . 3 .  1 . 5 .  . 3 . 6")
end)
```

### `bus(nombre, parámetros)`

Parámetros de un bus de envío (delay o reverb), para toda la canción. Hay dos buses que comparten todas las pistas: un delay sincronizado al tempo y una reverb. Cada pista les manda una parte de su salida con `pista:send(nombre, nivel)` (después de su volumen, mute y solo) y lo que devuelven se suma al master antes del limitador. Sin `bus()`, los valores por defecto. Los cambios entran al empezar el compás, como el tempo; la cola del delay y de la reverb sigue sonando aunque se pare el secuenciador. Una clave desconocida da error.

- `nombre`: `"delay"` o `"reverb"`
- `parámetros`: tabla con los de la tabla de abajo (los que falten, por defecto)

| Bus | Parámetro | Qué es | Por defecto | Valores |
| --- | --- | --- | --- | --- |
| `"delay"` | `time` | tiempo entre repeticiones (negra, corchea con puntillo, corchea, semicorchea) | `"1/8"` | `"1/4"`, `"3/16"`, `"1/8"` o `"1/16"` |
|  | `feedback` | cuánto de cada repetición vuelve a entrar | `0.4` | 0–0.95 |
|  | `level` | volumen de lo que devuelve el bus | `0.8` | 0–1 |
| `"reverb"` | `size` | tamaño de la sala (cola más larga) | `0.7` | 0–1 |
|  | `damp` | cuánto se apagan los agudos de la cola | `0.5` | 0–1 |
|  | `level` | volumen de lo que devuelve el bus | `0.8` | 0–1 |

```lua
local amen = sample("amen.wav"):slice(8)
bus("delay", { time = "3/16", feedback = 0.5, level = 0.7 })
bus("reverb", { size = 0.8, damp = 0.4 })
track("drums", amen)
  :steps("1 . 2 .  3 . 4 .  5 6 2 .  7 . 8 .")
  :send("reverb", 0.2)
-- Solo la caja, al delay.
track("caja", amen:slice_at(2))
  :steps(". . . .  x . . .  . . . .  x . . .")
  :send("delay", 0.6)
```

### `euclid(k, n [, golpe [, pasos]])`

Patrón euclídeo: k golpes repartidos por igual en n pasos, repetido hasta llenar el patrón. Devuelve texto en mini-notación para `steps`. Empieza por un golpe (el paso `i` suena si `i·k mod n < k`); para desplazarlo, `rotate`. El ritmo se repite hasta llenar `pasos` (16 por defecto; para una pista con `:length(n)`, pasa su longitud) y se corta ahí si `n` no lo divide. El golpe puede ser cualquier paso que suene, con sufijos o una etiqueta.

- `k`: golpes (0 a n)
- `n`: pasos del ritmo (1 a 64)
- `golpe`: el paso que suena (por defecto "x", el corte 1)
- `pasos`: longitud del patrón que devuelve (1 a 64; por defecto 16)

```lua
local amen = sample("amen.wav"):slice(8)
-- "1 . . 1  . . 1 .  1 . . 1  . . 1 ."
track("kick", amen):steps(euclid(3, 8, "1"))
track("hats", amen):steps(euclid(5, 16, "2@0.6"))
-- Tres golpes en 12 pasos, contra el compás de 16.
track("tres", amen):length(12):steps(euclid(3, 12, "3", 12))
```

### `rotate(patrón, n)`

Desplaza un patrón n pasos: positivo, más tarde; negativo, antes. Los pasos que se salen por un lado entran por el otro. Devuelve texto en mini-notación; se puede combinar con las demás funciones de patrones.

- `patrón`: patrón en mini-notación (de 1 a 64 pasos)
- `n`: pasos que se desplaza (negativo: hacia antes)

```lua
local amen = sample("amen.wav"):slice(8)
-- El break empezando en su segundo tiempo.
track("drums", amen):steps(rotate("1 . 2 .  3 . 4 .  5 6 2 .  7 . 8 .", -4))
-- Un euclídeo que empieza más tarde.
track("kick", amen):steps(rotate(euclid(3, 8, "1"), 2))
```

### `reverse_pattern(patrón)`

El patrón al revés: el último paso pasa a ser el primero. Invierte el orden de los pasos, no el sonido de cada golpe: para que un golpe suene al revés está el sufijo `'` (y `c:reverse()` en `every`).

- `patrón`: patrón en mini-notación (de 1 a 64 pasos)

```lua
local amen = sample("amen.wav"):slice(8)
local base = "1 . 2 .  3 . 4 .  5 6 2 .  7 . 8 ."
track("drums", amen)
  :steps(base)
  :every(4, function(c) c:steps(reverse_pattern(base)) end)
```

### `mutate(patrón, p [, semilla])`

Cambia cada paso, con probabilidad p, por otro del mismo patrón al azar. Se queda con los golpes y silencios del patrón, solo los cambia de sitio aquí y allá. Con `semilla`, el resultado es siempre el mismo. Sin ella, el azar sigue la regla de `chance`: se tira al cargar (otro en cada carga, salvo con la semilla fija de los tests), y dentro de `every` cada compás sale distinto y el ciclo se repite.

- `patrón`: patrón en mini-notación (de 1 a 64 pasos)
- `p`: probabilidad de cambiar cada paso (0 a 1)
- `semilla`: número entero; con la misma, el mismo resultado

```lua
local amen = sample("amen.wav"):slice(8)
local base = "1 . 2 .  3 . 4 .  5 6 2 .  7 . 8 ."
-- Una variación fija (semilla 7) y, cada 2 compases, otra nueva.
track("drums", amen)
  :steps(mutate(base, 0.25, 7))
  :every(2, function(c) c:steps(mutate(base, 0.3)) end)
```

### `shuffle(patrón [, semilla])`

Los mismos pasos en otro orden al azar. Conserva cada golpe y cada silencio, solo los cambia de sitio. La semilla funciona como en `mutate`: con ella, siempre el mismo orden; sin ella, el azar de la carga (como `chance`).

- `patrón`: patrón en mini-notación (de 1 a 64 pasos)
- `semilla`: número entero; con la misma, el mismo orden

```lua
local amen = sample("amen.wav"):slice(8)
-- Los golpes del break en un orden nuevo, siempre el mismo (semilla 3).
track("drums", amen):steps(shuffle("1 . 2 .  3 . 4 .  5 6 2 .  7 . 8 .", 3))
```

### `ramp(desde, hasta, duración)`

Valor que va de desde a hasta en esa duración y vuelve a empezar (en lugar de un número). Se pone en lugar de un número: en la tabla de `:fx`, de `bus()` o de `:grains`, o en `:gain()`, `:pan()` y `:send()` (lo que lo admite y lo que no, con el porqué, en la tabla de «Qué se puede automatizar», justo después de `lfo`). Es una rampa en diente de sierra alineada con la canción: con `"4 compases"` sube en los compases 0-3, otra vez en los 4-7... (como `every`, cuenta desde que arranca el secuenciador, así que recargar no la desplaza). Las frecuencias suben en escala logarítmica (de 200 a 4000 Hz, la mitad del camino es 894 Hz). Entra con el patrón al empezar el compás y se suaviza: nunca hace clic. También vale en la tabla de `:grains` y `:freeze` (y en el `pos` de `freeze`): ahí se mira al nacer cada grano (`size` y `density`, en escala logarítmica).

- `desde`: valor al empezar
- `hasta`: valor al acabar
- `duración`: `"N compases"` o `"N pasos"` (hasta 64 compases)

```lua
local amen = sample("amen.wav"):slice(8)
track("drums", amen)
  :steps("1 . 2 .  3 . 4 .  5 6 2 .  7 . 8 .")
  -- El filtro se abre durante 4 compases y vuelve a cerrarse.
  :fx("lowpass", { cutoff = ramp(200, 8000, "4 compases") })
```

### `lfo(frecuencia, mín, máx)`

Valor que oscila (seno) entre mín y máx (en lugar de un número). Como `ramp`, va en lugar de un número (ver la tabla que sigue). La frecuencia es un número en Hz (de 0.01 a 50) o lo que dura un ciclo en el tempo de la canción (`"1 compás"`, `"8 pasos"`). Empieza en `mín` cuando arranca el secuenciador; las frecuencias oscilan en escala logarítmica.

- `frecuencia`: Hz, o la duración de un ciclo: `"1 compás"`, `"8 pasos"`
- `mín`: valor más bajo
- `máx`: valor más alto

```lua
local amen = sample("amen.wav"):slice(8)
track("drums", amen)
  :steps("1 . 2 .  3 . 4 .  5 6 2 .  7 . 8 .")
  :fx("lowpass", { cutoff = lfo("2 compases", 400, 6000), q = 2 })
  :fx("distortion", { drive = 6, mix = lfo(0.5, 0, 0.8) })
```

#### Qué se puede automatizar

*Continuo*: la curva se evalúa cada 32 frames y se suaviza (~5 ms), también dentro de un golpe. *Por golpe*: cada golpe toma el valor de la curva al sonar y lo mantiene. *Por grano*: al nacer cada grano. Una curva en un parámetro que no la admite da error con la línea.

| Dónde | Admiten curva |
| --- | --- |
| `pista` | `gain` (continuo), `pan` (continuo), `send` (continuo), `pitch` (por golpe), `formant` (por golpe) |
| `fx("filter")` | `cutoff` (continuo), `q` (continuo) |
| `fx("bitcrush")` | `bits` (continuo), `downsample` (continuo) |
| `fx("compressor")` | `threshold` (continuo), `ratio` (continuo), `attack` (continuo), `release` (continuo), `makeup` (continuo) |
| `fx("distortion")` | `drive` (continuo), `mix` (continuo) |
| `fx("gain")` | `level` (continuo) |
| `fx("eq")` | `freq1` (continuo), `gain1` (continuo), `q1` (continuo), `freq2` (continuo), `gain2` (continuo), `q2` (continuo), `freq3` (continuo), `gain3` (continuo), `q3` (continuo), `freq4` (continuo), `gain4` (continuo), `q4` (continuo) |
| `fx("dyneq")` | `freq` (continuo), `q` (continuo), `threshold` (continuo), `ratio` (continuo), `range` (continuo), `attack` (continuo), `release` (continuo) |
| `bus("delay")` | `feedback` (continuo), `level` (continuo) |
| `bus("reverb")` | `size` (continuo), `damp` (continuo), `level` (continuo) |
| `cortes:set` | `attack` (por golpe), `release` (por golpe), `pan` (por golpe), `length` (por golpe), `crossfade` (por golpe) |
| `:grains` | `size` (por grano), `density` (por grano), `pos` (por grano), `speed` (por grano), `jitter` (por grano), `pitch` (por grano), `spread` (por grano) |
| `synth("acid")` | `cutoff` (continuo), `res` (continuo), `envmod` (continuo), `decay` (continuo), `accent` (continuo), `dist` (continuo), `glide` (continuo), `sub` (continuo), `sub_pan` (continuo), `sub_drive` (continuo) |
| `synth("sub")` | `tone` (continuo), `drive` (continuo), `glide` (continuo), `sub` (continuo), `sub_pan` (continuo), `sub_drive` (continuo) |
| `synth("subharm")` | `cutoff` (continuo), `res` (continuo), `envmod` (continuo), `fdecay` (continuo), `glide` (continuo), `drive` (continuo), `attack` (continuo), `decay` (continuo) |

| Dónde | No admiten curva | Por qué |
| --- | --- | --- |
| `pista` | `swing` | se convierte al cargar en el retraso de cada contratiempo (como `humanize`): es parte del patrón |
| `fx("filter")` | `type` | elige entre valores con nombre (o sí/no): no hay nada entre uno y otro que recorrer |
| `fx("eq")` | `type1`, `type2`, `type3`, `type4` | elige entre valores con nombre (o sí/no): no hay nada entre uno y otro que recorrer |
| `fx("dyneq")` | `type` | elige entre valores con nombre (o sí/no): no hay nada entre uno y otro que recorrer |
| `fx("dyneq")` | `sidechain` | es una pista, no un número |
| `bus("delay")` | `time` | es una figura ("1/8"), no un número; además, mover el retardo de una línea con repeticiones dentro suena a cambio de pitch |
| `cortes:set` | `play` | elige entre valores con nombre (o sí/no): no hay nada entre uno y otro que recorrer |
| `:grains` | `seed` | es la semilla del azar: entre dos semillas no hay nada continuo |
| `instrument` | `root`, `adsr`, `formant` | cada nota lo lee al nacer y el instrumento se valida entero al cargar; para cambiar la nota, la del paso (`^`) |
| `instrument` | `loop`, `crossfade` | se aplica al cargar, en el hilo de control (el sample o el bucle se preparan una vez): no suena en el tiempo |
| `instrument` | `voices` | cambia cuántas cosas suenan, no cómo suenan: entra con el patrón al compás |
| `instrument` | `keep` | elige entre valores con nombre (o sí/no): no hay nada entre uno y otro que recorrer |
| `synth("acid")` | `wave`, `sub_wave` | elige entre valores con nombre (o sí/no): no hay nada entre uno y otro que recorrer |
| `synth("acid")` | `root` | cada nota lo lee al nacer y el instrumento se valida entero al cargar; para cambiar la nota, la del paso (`^`) |
| `synth("acid")` | `seed` | es la semilla del azar: entre dos semillas no hay nada continuo |
| `synth("acid")` | `sub_div` | es un divisor entero: entre dividir por 2 y por 3 no hay nada que recorrer |
| `synth("sub")` | `wave`, `sub_wave` | elige entre valores con nombre (o sí/no): no hay nada entre uno y otro que recorrer |
| `synth("sub")` | `root` | cada nota lo lee al nacer y el instrumento se valida entero al cargar; para cambiar la nota, la del paso (`^`) |
| `synth("sub")` | `attack`, `release` | la envolvente se fija con el sintetizador al cargar; para mover el volumen en el tiempo, el de la pista (`:gain()` con `ramp()` o `lfo()`) |
| `synth("sub")` | `sub_div` | es un divisor entero: entre dividir por 2 y por 3 no hay nada que recorrer |
| `synth("subharm")` | `o1`, `o2`, `s1`, `s2`, `s3`, `s4` | es una capa del subharm: sus divisores son enteros y su nivel y su pan entran con el patrón al compás |
| `synth("subharm")` | `wave` | elige entre valores con nombre (o sí/no): no hay nada entre uno y otro que recorrer |
| `synth("subharm")` | `root` | cada nota lo lee al nacer y el instrumento se valida entero al cargar; para cambiar la nota, la del paso (`^`) |
| `synth("subharm")` | `voices` | cambia cuántas cosas suenan, no cómo suenan: entra con el patrón al compás |
| `synth("subharm")` | `seed` | es la semilla del azar: entre dos semillas no hay nada continuo |
| `sample` | `trim`, `normalize`, `fade_in`, `fade_out` | se aplica al cargar, en el hilo de control (el sample o el bucle se preparan una vez): no suena en el tiempo |

### `scale(nombre [, tónica])`

Devuelve: escala.

Una escala, para tocar melodías con un corte: sus grados se convierten en pitch. El pitch se cuenta desde el tono original del corte, que hace de do (C): con tónica `"D"` todo sube 2 semitonos. Las escalas están en la tabla de abajo; las tónicas, de `C` a `B` con `#` o `b` (`F#`, `Bb`).

- `nombre`: la escala (ver la tabla)
- `tónica`: nota de partida (por defecto "C", el tono del corte)

| Escala | Qué es | Semitonos |
| --- | --- | --- |
| `"major"` | mayor (jónica) | 0 2 4 5 7 9 11 |
| `"minor"` | menor natural (eólica) | 0 2 3 5 7 8 10 |
| `"harmonic_minor"` | menor armónica | 0 2 3 5 7 8 11 |
| `"melodic_minor"` | menor melódica (ascendente) | 0 2 3 5 7 9 11 |
| `"dorian"` | dórica | 0 2 3 5 7 9 10 |
| `"phrygian"` | frigia | 0 1 3 5 7 8 10 |
| `"lydian"` | lidia | 0 2 4 6 7 9 11 |
| `"mixolydian"` | mixolidia | 0 2 4 5 7 9 10 |
| `"locrian"` | locria | 0 1 3 5 6 8 10 |
| `"major_pentatonic"` | pentatónica mayor | 0 2 4 7 9 |
| `"minor_pentatonic"` | pentatónica menor | 0 3 5 7 10 |
| `"blues"` | blues | 0 3 5 6 7 10 |
| `"chromatic"` | cromática (cada grado, un semitono) | 0 1 2 3 4 5 6 7 8 9 10 11 |

```lua
local amen = sample("amen.wav"):slice(8)
local menor = scale("minor", "C")
-- Grados 1 3 5 3... de la escala sobre el corte 3:
-- "3^0 . 3^3 .  3^7 . 3^3 .  ..."
track("melodia", amen)
  :steps(menor:melody("1 . 3 .  5 . 3 .  8 . 7 .  5 . 3 -2", "3"))
  :gain(0.6)
```

### `escala:pitch(grado)`

Semitonos de un grado de la escala (1 es la tónica, 8 la octava). Los grados siguen hacia arriba por octavas (en una escala de 7 notas, 8 es la tónica una octava arriba) y hacia abajo con negativos (-1 es el último grado una octava abajo). El 0 no existe. Sirve para escribir pitch a mano: `"3^" .. menor:pitch(5)`.

- `grado`: número de grado (1, 2, 3...; negativos, por debajo)

```lua
local amen = sample("amen.wav"):slice(8)
local quinta = scale("major"):pitch(5) -- 7 semitonos
track("d", amen):steps("3 . . .  3^" .. quinta .. " . . .  3 . . .  3^12 . . .")
```

### `escala:melody(grados [, golpe])`

Convierte un patrón de grados en uno de pasos con pitch: "1 3 5 3" → "1^0 1^3 1^7 1^3". Cada paso de `grados` es un grado (o `.`), con los sufijos que quieras salvo `^`: `5@0.5` da el golpe con el pitch del grado 5 a medio volumen. Devuelve texto en mini-notación para `steps`.

- `grados`: pasos: grados de la escala o `.`
- `golpe`: el corte que suena (número o etiqueta; por defecto "1")

### `sample:detect_bpm()`

Tempo y compases del sample, suponiendo que es un loop de compases enteros. Devuelve dos valores, el tempo (entero si sale muy cerca de uno) y los compases, o `nil` si no se ve (sin golpes, o con un tempo fuera de 60 a 200). Mira cómo encajan los golpes en la rejilla de semicorcheas para 1, 2, 4 y 8 compases; si dos encajan igual (un break en corcheas vale a 86 y a 172), elige el más cercano a 130. Lo mismo hacen `--detect-bpm` y «Detectar el BPM del sample» de la paleta.

```lua
local amen = sample("amen.wav")
local tempo, compases = amen:detect_bpm()
bpm(tempo)
track("drums", amen:slice(8)):steps("1 . 2 .  3 . 4 .  5 6 2 .  7 . 8 .")
```

### `sample:stretch([bpm_origen])`

Devuelve: sample.

Estira el sample del tempo de origen al de la canción sin cambiar el tono. Al cargar, el sample pasa de `bpm_origen` al tempo de la canción (el de `bpm()`, el del proyecto o 120), sin cambiar el tono: un break de 140 suena a 170 y sus golpes caen en la rejilla. Sin `bpm_origen`, usa el que detecta `detect_bpm()` (si no lo ve, da error: pásalo). Los cortes se buscan en el sample original y se escalan, así que `slice(n)`, las etiquetas y los ajustes en ms (`move`, proyecto) son los de siempre. Va bien entre ×0,7 y ×1,4 (de ×0,25 a ×4 se admite). Es WSOLA por tramos de golpe a golpe: cada golpe empieza justo en su sitio y el resto se estira copiando la forma de onda; estirar mucho repite o emborrona un poco las colas largas.

- `bpm_origen`: tempo original del sample (20 a 400); sin él, el detectado

```lua
bpm(170)
-- El break está a 140: suena a 170 con el mismo tono.
local amen = sample("amen.wav"):stretch(140):slice(8)
track("drums", amen):steps("1 . 2 .  3 . 4 .  5 6 2 .  7 . 8 .")
-- O con el tempo que detecta.
local otro = sample("amen.wav"):stretch():slice(8)
```

### `sample:slice_grid(n)`

Devuelve: cortes.

Corta el sample en n partes iguales (rejilla), sin mirar los golpes. Para un break bien cuadrado (un compás en 16 o en 8) o para material sin golpes claros. Las etiquetas y los ajustes del proyecto de un sample cortado en rejilla son distintos de los de `slice(n)` (en el proyecto, el número de cortes se escribe `rejilla16`).

- `n`: partes (1 a 64)

```lua
local amen = sample("amen.wav"):slice_grid(16)
track("drums", amen):steps("1 2 3 4  5 6 7 8  9 10 11 12  13 14 15 16")
```

### `sample:instrument([opciones])`

Devuelve: instrumento.

Toca el sample como un instrumento: notas con polifonía, ADSR y bucle sostenido. Una pista con un instrumento (`track(nombre, instrumento)`) toca notas en vez de cortes: cada golpe abre una voz sin cortar las que suenan (si no queda ninguna libre, se roba la más antigua), sube y baja con su envolvente ADSR y dura su paso más sus `_`, y luego el release. El sample suena tal cual en la nota `root`: `pista:melody("c e g e")` toca cromáticamente desde ahí, y en `steps` el pitch (`^`) son semitonos sobre la raíz (varios `^` hacen un acorde: `1^0^4^7`). Con `loop`, la nota sigue en bucle mientras dure (también en su release), con un fundido cruzado en el salto para que no haga clic: los últimos ms antes del fin del bucle se funden con los de antes de su inicio (si no hay tanto audio antes del inicio, el fundido se acorta y se avisa). Sin `loop`, la nota acaba con el sample. Al bajar mucho el tono el sample dura más (y al subir, menos): el pitch cambia la velocidad. Con `keep = true` no: cada nota conserva la duración del sample (granos alineados, como `%` en los pasos; bien hasta ±12 semitonos, más allá se avisa).

- `opciones`: tabla con `root`, `loop`, `voices`, `adsr` y `crossfade` (ver abajo)

| Parámetro | Qué es | Por defecto | Valores |
| --- | --- | --- | --- |
| `root` | la nota en la que suena el sample tal cual (`melody` toca desde ahí); C4 es el do central | `"C4"` | una nota (`"C3"`, `"F#2"`, `"Bb4"`) o un número MIDI (0–127) |
| `loop` | bucle sostenido `{inicio, fin}` en ms: la nota sigue en bucle mientras dure (sin él, acaba con el sample) | `nil` | dentro del sample, con el fin después del inicio |
| `voices` | notas que pueden sonar a la vez; con todas ocupadas, una nota nueva roba la más antigua | `4` | 1–16 |
| `adsr` | envolvente de cada nota `{ataque, decay, sustain, release}`: ms, ms, nivel (0–1) y ms (0 ms: el fundido mínimo de 1,5 ms) | `{ 0, 0, 1, 100 }` | ms de 0 a 10000; sustain de 0 a 1 |
| `crossfade` | fundido cruzado del salto del bucle, en ms (usa el audio de antes del inicio del bucle; si no cabe, se acorta y se avisa) | `10` | 0–1000 |
| `keep` | transponer las notas sin cambiar su duración (granos alineados, como `%` en los pasos): el sample dura lo mismo en cualquier nota; va bien hasta ±12 semitonos | `false` | `true` o `false` |
| `formant` | con `keep`, dónde quedan los formantes de las notas: 0 donde estaban (la voz no suena a ardilla al subir), n desplazados n semitonos; sin él, se mueven con la nota | `nil` | -24–24 (necesita `keep = true`) |

```lua
bpm(100)
-- El sample suena tal cual en C3; el bucle va de 200 a 600 ms.
local piano = sample("amen.wav"):instrument({
  root = "C3", loop = { 200, 600 }, voices = 4, adsr = { 5, 80, 0.7, 300 },
})
track("lead", piano):melody("c _ e _  g _ e _  c+e+g _ _ _  . . b2 .")
```

### `synth(tipo, [opciones])`

Devuelve: instrumento.

Un instrumento sintetizado, sin sample: osciladores, filtro y envolventes. Se toca como un instrumento de sample (`track(nombre, synth(...))`, `:melody()`, `scale`, los pads del modo tocar y el MIDI de entrada): el pitch de cada paso son semitonos sobre su `root`. Los parámetros continuos admiten `ramp()` y `lfo()` (se mueven también dentro de una nota, sin clics) y los efectos de la pista van detrás. En sus pasos, dos sufijos más: `!` accent (la nota suena más fuerte y más abierta) y `/` slide (la nota llega ligada desde la anterior: se desliza hasta su altura en `glide` ms sin volver a disparar las envolventes; la anterior sigue sonando hasta que llega). En `melody` van detrás de la nota: `"c!"`, `"eb/"`, `"g!/"`. Hay dos tipos, los dos monofónicos: `"acid"`, el bajo del 303 (un oscilador de sierra o cuadrada, paso bajo de 4 polos con mucha resonancia; en él, el accent también abre el filtro), y `"sub"` (o `"subbass"`), un subgrave limpio para dar profundidad (un seno por un paso bajo suave que empieza cada nota en el cruce por cero y sigue su fase de una nota a la siguiente: sin golpes al arrancar ni clics entre notas; en él, el accent es un 40 % más fuerte). Y `"subharm"`, dos osciladores con dos subs cada uno que dividen la frecuencia y el compás (ver `subharm`).

- `tipo`: `"acid"`, `"sub"` o `"subharm"`
- `opciones`: tabla con sus parámetros (ver abajo)

| Tipo | Parámetro | Qué es | Por defecto | Valores |
| --- | --- | --- | --- | --- |
| `"acid"` | `wave` | forma de onda del oscilador | `"saw"` | `"saw"` o `"square"` |
|  | `root` | la nota que suena con 0 semitonos (`melody` sin octava toca en la suya) | `"C2"` | una nota (`"C2"`, `"F#1"`) o un número MIDI (0–127) |
|  | `cutoff` | corte del filtro de 4 polos, en Hz | `400` | 20–20000 |
|  | `res` | resonancia: el pico en el corte (con 1, el filtro silba solo) | `0.7` | 0–1 |
|  | `envmod` | cuánto abre la envolvente del filtro el corte en cada nota (1: cuatro octavas) | `0.6` | 0–1 |
|  | `decay` | lo que tarda en cerrarse la envolvente del filtro, en ms | `300` | 10–5000 |
|  | `accent` | cuánto pesa el `!` de un paso: más volumen (el doble con 1), dos octavas más de filtro con un decay corto, y varios seguidos se acumulan | `0.6` | 0–1 |
|  | `dist` | saturación a la salida (0, limpio) | `0` | 0–1 |
|  | `glide` | lo que tarda el `/` de un paso en llegar a su nota, en ms | `60` | 0–1000 |
|  | `seed` | semilla de la fase del oscilador al empezar desde el silencio: con la misma, el mismo audio; sin ella, una sacada del nombre de la pista | `del nombre` | entero de 0 en adelante |
|  | `sub` | nivel del sub: una onda a la frecuencia de la nota dividida por `sub_div`, sacada de la fase del oscilador (siempre en fase, también con glide); 0 lo apaga | `0` | 0–1 |
|  | `sub_div` | divisor de frecuencia del sub: 2 una octava abajo, 3 una octava y una quinta, 4 dos octavas... | `2` | 1–16 |
|  | `sub_wave` | forma de onda del sub: cuadrada (con PolyBLEP en sus saltos), triangular o seno | `"square"` | `"square"`, `"tri"` o `"sine"` |
|  | `sub_pan` | pan del sub, de potencia constante (en el centro suena igual en los dos lados) | `0` | -1–1 |
|  | `sub_drive` | saturación suave de la suma del oscilador y el sub, antes del filtro (solo con el sub sonando) | `0` | 0–1 |
| `"sub"` (o `"subbass"`) | `wave` | forma de onda: el seno es el subgrave limpio; la triangular añade algún armónico impar (se oye algo más en altavoces pequeños) | `"sine"` | `"sine"` o `"triangle"` |
|  | `root` | la nota que suena con 0 semitonos (`melody` sin octava toca en la suya); C1 son 32,7 Hz | `"C1"` | una nota (`"C1"`, `"F#1"`) o un número MIDI (0–127) |
|  | `tone` | corte del paso bajo suave (12 dB/oct, sin resonancia), en Hz: más bajo, más redondo; más alto, deja pasar los armónicos de `drive` | `250` | 40–2000 |
|  | `drive` | saturación antes del filtro: le da armónicos graves para que se oiga en altavoces pequeños (0, un seno limpio) | `0` | 0–1 |
|  | `glide` | lo que tarda el `/` de un paso en llegar a su nota, en ms | `40` | 0–1000 |
|  | `attack` | lo que tarda en subir cada nota que empieza desde el silencio, en ms (las pegadas no vuelven a subir) | `5` | 0–2000 |
|  | `release` | lo que tarda en apagarse al acabar la nota, en ms | `80` | 0–5000 |
|  | `sub` | nivel del sub: una onda a la frecuencia de la nota dividida por `sub_div`, sacada de la fase del oscilador (siempre en fase, también con glide); 0 lo apaga | `0` | 0–1 |
|  | `sub_div` | divisor de frecuencia del sub: 2 una octava abajo, 3 una octava y una quinta, 4 dos octavas... | `2` | 1–16 |
|  | `sub_wave` | forma de onda del sub: cuadrada (con PolyBLEP en sus saltos), triangular o seno | `"square"` | `"square"`, `"tri"` o `"sine"` |
|  | `sub_pan` | pan del sub, de potencia constante (en el centro suena igual en los dos lados) | `0` | -1–1 |
|  | `sub_drive` | saturación suave de la suma del oscilador y el sub, antes del filtro (solo con el sub sonando) | `0` | 0–1 |
| `"subharm"` | `o1` | el primer oscilador de sierra: tabla con `div`, `rdiv`, `pan` y `level` (ver la tabla de capa) | `{ div = 1, level = 1 }` | tabla de capa |
|  | `o2` | el segundo oscilador de sierra | `{ div = 1, level = 0 }` | tabla de capa |
|  | `s1` | primer sub de `o1` (su frecuencia entre `div`) | `{ div = 2, level = 0.6 }` | tabla de capa |
|  | `s2` | segundo sub de `o1` | `{ div = 3, level = 0 }` | tabla de capa |
|  | `s3` | primer sub de `o2` | `{ div = 2, level = 0 }` | tabla de capa |
|  | `s4` | segundo sub de `o2` | `{ div = 4, level = 0 }` | tabla de capa |
|  | `wave` | forma de onda de todas las capas | `"saw"` | `"saw"` o `"square"` |
|  | `root` | la nota que suena con 0 semitonos (`melody` sin octava toca en la suya) | `"C2"` | una nota (`"C2"`) o un número MIDI (0–127) |
|  | `cutoff` | corte del paso bajo de 4 polos, en Hz | `1200` | 20–20000 |
|  | `res` | resonancia del filtro | `0.3` | 0–1 |
|  | `envmod` | cuánto abre la envolvente del filtro el corte en cada nota (1: cuatro octavas) | `0.3` | 0–1 |
|  | `fdecay` | lo que tarda en cerrarse la envolvente del filtro de cada nota, en ms | `400` | 10–5000 |
|  | `glide` | lo que tarda el `/` de un paso en llegar a su nota, en ms | `0` | 0–1000 |
|  | `drive` | saturación a la salida (0, limpio) | `0` | 0–1 |
|  | `attack` | ataque de la envolvente de cada capa (la que se redispara con su `rdiv`), en ms | `2` | 0.5–2000 |
|  | `decay` | lo que tarda en apagarse cada capa tras sonar, en ms: corto, golpes; largo, notas que se solapan | `800` | 10–10000 |
|  | `voices` | notas a la vez (1: mono, las notas se ligan; más: polifónico, cada voz con sus seis capas) | `1` | 1–8 |
|  | `seed` | semilla de la fase de los osciladores al empezar desde el silencio; sin ella, una sacada del nombre de la pista | `del nombre` | entero de 0 en adelante |

```lua
bpm(130)
local bass = synth("acid", { cutoff = 350, res = 0.8, envmod = 0.7, decay = 250 })
track("acid", bass)
  :melody("c c! eb c/  . c bb2 c/  g2! . c c/  eb! f/ eb c")
-- Debajo, un subgrave en C1 que sigue la tónica.
track("sub", synth("sub", { tone = 200, release = 120 }))
  :melody("c _ _ _  _ _ _ _  ab0 _ _ _  bb0 _ g0/ _")
```

### `subharm{ opciones }`

Devuelve: instrumento.

Inspirado en el Subharmonicon: seis capas que dividen la frecuencia (f/n) y el compás (1/n). Lo mismo que `synth("subharm", { ... })`. Dos osciladores de sierra (`o1`, `o2`), cada uno a la nota dividida por su `div`, y dos subs de cada uno (`s1`, `s2` de `o1`; `s3`, `s4` de `o2`), a la frecuencia de su oscilador dividida por el suyo y sacados de su fase (siempre en fase). Cada capa tiene su `pan` (de potencia constante) y su `level`, y vuelve a sonar (su envolvente, con `attack` y `decay`) cada 1/`rdiv` del compás: todas siguen el mismo reloj, así que los ritmos (3 contra 4, 5 contra 7...) vuelven a coincidir al empezar cada compás; con el transporte parado (pads, MIDI), el compás empieza con la nota. Todo pasa por un paso bajo de 4 polos. Se toca como cualquier instrumento (`melody`, pads, MIDI, `!` y `/`); sus capas cambian con el patrón, al compás.

- `opciones`: tabla con las capas (`o1`...`s4`, cada una `{ div, rdiv, pan, level }`) y el resto de parámetros (ver abajo)

| Tabla | Parámetro | Qué es | Por defecto | Valores |
| --- | --- | --- | --- | --- |
| `"capa"` | `div` | divisor de frecuencia: en `o1` y `o2`, de la nota (1 la nota, 2 una octava abajo, 3 una octava y una quinta...); en los subs, de su oscilador | `o1, o2: 1; s1: 2; s2: 3; s3: 2; s4: 4` | entero de 1 a 16 |
|  | `rdiv` | divisor rítmico: la capa vuelve a sonar (su envolvente, con `attack` y `decay`) cada 1/rdiv del compás; todas a la vez al empezar cada compás | `1` | entero de 1 a 16 |
|  | `pan` | pan de la capa, de potencia constante | `0` | -1–1 |
|  | `level` | nivel de la capa (0, callada) | `o1: 1; s1: 0.6; las demás: 0` | 0–1 |

```lua
bpm(110)
local sh = subharm{
  o1 = { div = 1, rdiv = 4, pan = -0.6 },
  s1 = { div = 3, rdiv = 3, pan = 0.5, level = 0.7 },
  cutoff = 900, res = 0.4,
}
track("sh", sh):melody("c _ _ _  _ _ _ _  eb _ _ _  g _ _ _")
```

### `sample:slice(n)`

Devuelve: cortes.

Corta el sample en sus n golpes más fuertes. Si hay menos golpes, devuelve menos cortes. Usa el detector por flujo espectral con 40 ms de separación mínima.

- `n`: cortes a buscar (1 a 64)

```lua
local amen = sample("amen.wav"):slice(8)
local cuantos = amen:count()
track("drums", amen):steps("1 . . .  . . . .  1 . . .  . . . .")
```

### `cortes:count()`

Número de cortes. Puede ser menor que el pedido en `slice(n)` si el detector encontró menos golpes.

### `cortes:slice_at(i)`

Devuelve: cortes.

Solo el corte i (desde 1), como fuente de un único corte. En el patrón de la pista es `1` o `x`.

- `i`: número del corte (desde 1)

```lua
local amen = sample("amen.wav"):slice(8)
local caja = amen:slice_at(3)
track("caja", caja):steps(". . . .  x . . .  . . . .  x . . .")
```

### `cortes:set([i,] parámetros)`

Devuelve: cortes.

Forma de un corte (o de todos, sin i): ataque, cola, pan, parte que suena y bucle. Vale para todas las pistas que usan esta variable, también las definidas antes (como `:name`). Una clave desconocida da error. Los parámetros están en la tabla de abajo; los que no se dan quedan como estaban. Los números admiten `ramp()` y `lfo()`: cada golpe toma el valor de la curva al sonar (la voz los lee al nacer; en los pads, el primer extremo).

- `i`: el corte (desde 1); sin él, todos
- `parámetros`: tabla con los de la tabla de abajo

| Parámetro | Qué es | Por defecto | Valores |
| --- | --- | --- | --- |
| `attack` | rampa de entrada, en ms (0: el fundido mínimo de 1,5 ms) | `0` | 0–2000 |
| `release` | cola al final del corte (o al cerrarse la puerta en el modo gate), en ms | `0` | 0–5000 |
| `pan` | panorama: -1 izquierda, 0 centro, 1 derecha (el lado contrario baja) | `0` | -1–1 |
| `length` | parte del corte que suena, desde su inicio (0.5: la primera mitad) | `1` | 0.01–1 |
| `play` | cómo se lee: una vez, en bucle o de ida y vuelta (los bucles suenan hasta el siguiente golpe, la puerta o el choke) | `"normal"` | `"normal"`, `"loop"` o `"pingpong"` |
| `crossfade` | con play = "loop", fundido en el salto del bucle en ms (0: salto seco); funde con lo que suena justo antes del corte o, si no hay, con lo que sigue tras él, y el bucle dura lo mismo | `10` | 0–1000 |

```lua
local amen = sample("amen.wav"):slice(8)
-- El charles (corte 3), corto y sin chasquido; todos, con algo de cola.
amen:set({ release = 20 }):set(3, { attack = 2, release = 5, pan = 0.4 })
-- El corte 8, la mitad y en bucle de ida y vuelta.
amen:set(8, { length = 0.5, play = "pingpong" })
track("drums", amen):steps("1 . 3 .  2 . 3 .  1 1 3 .  2 . 3 3"):mode("gate")
```

### `cortes:fx(cuál, nombre, parámetros)`

Devuelve: cortes.

Efecto que solo suena en un corte (o en varios): por número, etiqueta o lista. Los efectos son los de `pista:fx` (la misma tabla, con `ramp()` y `lfo()`), hasta 4 por corte, en orden. Cada golpe de ese corte pasa por ellos con su propio estado (el filtro de un golpe no arrastra el anterior) y luego por los de la pista: voz → efectos del corte → efectos de la pista → volumen de la pista → mezcla. Los demás cortes suenan exactamente igual que sin ellos. Vale para todas las pistas que usan esta variable, también las definidas antes (como `:set`), y para sus pads. `cuál` es el número del corte (desde 1), su etiqueta (ponla antes con `:name`) o una lista: `{"snare", 7}`. Una etiqueta o un corte que no existe, o un parámetro fuera de rango, da error con la línea y sigue sonando lo anterior. El EQ dinámico de un corte no admite `sidechain`. Con `:grains()` no suenan (los granos no son voces: se avisa) y un instrumento no tiene cortes que elegir (usa `pista:fx`). Con `~` en `steps` se sigue moviendo el primer filtro de la pista, no el del corte. Los envíos (`send`) son de la pista entera.

- `cuál`: número de corte (desde 1), etiqueta o lista de ellos
- `nombre`: el efecto (los de pista:fx)
- `parámetros`: tabla con los parámetros del efecto

```lua
local amen = sample("amen.wav"):slice(8)
amen:name(3, "snare"):name(1, "kick")
-- Solo la caja, sucia y con un filtro que se abre cada dos compases.
amen:fx("snare", "bitcrush", { bits = 6 })
amen:fx("snare", "lowpass", { cutoff = ramp(800, 8000, "2 compases") })
-- El bombo y el corte 7, más bajos.
amen:fx({ "kick", 7 }, "gain", { level = 0.7 })
track("drums", amen):steps("kick . snare .  3 . snare 7  kick kick snare .  7 . snare 3")
```

### `cortes:add(ms)`

Devuelve: cortes.

Añade un corte que empieza en ms (parte en dos el que lo contiene). Los cortes siguientes se corren un número. Como `move`, cambia la variable (y devuelve una copia: para seguir cambiándola, otra línea con la variable): las pistas definidas antes con ella no lo ven. Tiene que quedar a más de 1 ms de los cortes que ya hay. Desde la interfaz, «Partir el corte seleccionado» lo hace en el proyecto.

- `ms`: dónde empieza el corte nuevo, en ms desde el principio

```lua
local amen = sample("amen.wav"):slice(8)
-- Un corte más dentro del tercero, y sin la marca del sexto (que,
-- con el nuevo, es el séptimo).
amen:add(250)
amen:remove(7)
track("drums", amen):steps("1 . 3 4  5 . 6 .  7 . 8 .  1 . 3 4")
```

### `cortes:remove(i)`

Devuelve: cortes.

Quita el corte i (desde 2): el anterior se alarga hasta donde acababa este. Los cortes siguientes se corren un número. Como `move`, cambia la variable. Desde la interfaz, «Unir el corte seleccionado con el anterior» lo hace en el proyecto.

- `i`: el corte que se quita (desde 2)

### `cortes:name(i, nombre)`

Devuelve: cortes.

Pone una etiqueta al corte i para usarla en steps en vez del número. Con la etiqueta, `steps("kick . snare .")` es lo mismo que `steps("1 . 3 .")`, y admite los mismos sufijos (`kick^7@0.5`). Vale para las pistas de esa variable, también las definidas antes, y `slice_at` conserva la etiqueta de su corte. `nil` la quita. El nombre lleva letras sin tilde, números y `_`, y no puede ser un número, `x` ni acabar en `r` y un número (se confundiría con el retrigger); dos cortes no pueden llevar la misma. Las etiquetas también se pueden guardar en el proyecto (sección 1); las de `name` mandan.

- `i`: número del corte (desde 1)
- `nombre`: la etiqueta (`"kick"`, `"snare"`...), o `nil` para quitarla

```lua
local amen = sample("amen.wav"):slice(8)
amen:name(1, "kick"):name(2, "hat"):name(3, "snare")
track("drums", amen):steps("kick . hat .  snare . hat .  kick kick hat .  snare . hat^7@0.5 .")
```

### `cortes:move(i, ms)`

Devuelve: cortes.

Ajuste manual: el corte i empieza en ms (desde el principio del sample). Manda sobre el detector; el corte anterior acaba donde empieza este. Tiene que quedar a más de 1 ms de sus vecinos. Cambia la variable, así que hazlo antes de usarla en `track()`. Desde la interfaz, el mismo ajuste se hace con teclas en el modo tocar y se guarda en el proyecto (`corte = amen.wav 8 3 412.5`).

- `i`: número del corte (desde 1)
- `ms`: dónde empieza, en ms desde el principio del sample

```lua
local amen = sample("amen.wav"):slice(8)
-- El charles del corte 2, un poco antes para que entre con su ataque.
amen:move(2, 205.5)
track("drums", amen):steps("1 . 2 .  3 . 4 .  5 6 2 .  7 . 8 .")
```

### `pista:steps(texto)`

Devuelve: pista.

Patrón de la pista en mini-notación (16 pasos, o los de :length). La notación está en la sección 3. Lleva tantos pasos como la longitud de la pista (16, o la de `:length(n)`, que va antes). Los cortes que no existen dan error en esa línea. Un paso puede llevar efectos solo para su golpe entre llaves: `3{lp=800,q=4}{crush=6}` (los de `:fx`, con rampas del golpe: `{lp=200>4000:4}`).

- `texto`: pasos separados por espacios: 16, o los de :length(n)

```lua
local amen = sample("amen.wav"):slice(8)
track("drums", amen):steps("1 . 2 .  3 . 4 .  5 6 2 .  7 . 8 .")
```

### `pista:melody(notas)`

Devuelve: pista.

Patrón de notas por su nombre ("c e g e"), cromático respecto a la raíz del instrumento. Como `steps`, con notas en vez de cortes: `c`, `e`, `f#`, `bb3`... (la octava es opcional; sin ella, la de la raíz; C4 es el do central), acordes con `+` (`c+e+g`: las notas sin octava van justo por encima de la anterior), `.` silencio y `_` para alargar la nota de antes un paso. Cada nota admite los sufijos de los pasos salvo `^` (`e@0.5`, `gr2`). En una pista con instrumento la raíz es su `root`; en una de cortes, la nota C4 es el corte 1 tal cual (como en `scale`), y los acordes no valen (no hay polifonía).

- `notas`: notas, acordes (`c+e+g`), `.` o `_`: tantas como pasos tiene la pista

```lua
local piano = sample("amen.wav"):instrument({ root = "C3", adsr = { 5, 0, 1, 200 } })
track("bajo", piano)
  :melody("c2 _ . c2  eb2 _ . f2  g2 _ _ _  bb1 _ c2 .")
  :gain(0.8)
```

### `pista:length(n)`

Devuelve: pista.

Pasos de una vuelta del patrón (16 por defecto, de 1 a 64): polirritmia entre pistas. Va antes de `:steps()`, que entonces lleva `n` pasos. Los pasos siguen siendo semicorcheas: una pista de 12 pasos da la vuelta cada 12 y se desfasa contra el compás de 16 (cada 3 compases vuelven a coincidir); una de 32 ocupa dos compases. Las vueltas van alineadas con la canción desde que arranca, aunque se recargue. En una pista así, `every`, `fill`, `chance` y `humanize` cuentan vueltas del patrón en vez de compases, y los cambios del script siguen entrando al empezar un compás.

- `n`: pasos de una vuelta (1 a 64)

```lua
local amen = sample("amen.wav"):slice(8)
track("drums", amen):steps("1 . 2 .  3 . 4 .  5 6 2 .  7 . 8 .")
-- 3 contra 4: un charles de 12 pasos sobre el compás de 16.
track("hats", amen):length(12):steps("3 . 3@0.5 .  3 . . 3  . 3@0.5 . .")
-- Dos compases de bombo.
track("kick", amen:slice_at(1))
  :length(32)
  :steps("x . . .  . . . .  x . . .  . . . .  x . . .  . . x .  . . . .  . . . .")
```

### `pista:gain(g)`

Devuelve: pista.

Volumen de la pista, lineal de 0 a 4. Se aplica después de los efectos. Admite `ramp()` y `lfo()`: el volumen se mueve suavizado, también dentro de un golpe.

- `g`: volumen lineal (0 a 4; 1 lo deja igual), `ramp()` o `lfo()`

```lua
local amen = sample("amen.wav"):slice(8)
track("drums", amen):steps("1 . 2 .  3 . 4 .  5 6 2 .  7 . 8 ."):gain(0.7)
track("sube", amen):steps("x . x .  x . x .  x . x .  x . x ."):gain(ramp(0, 1, "8 compases"))
```

### `pista:pan(p)`

Devuelve: pista.

Pan de la pista: -1 izquierda, 0 centro, 1 derecha. Va después del volumen y antes de los envíos, con la ley de balance de los cortes (el lado contrario baja, el propio no sube). Admite `ramp()` y `lfo()`. El pan de cada corte, con `cortes:set(i, { pan = ... })`.

- `p`: de -1 a 1, `ramp()` o `lfo()`

```lua
local amen = sample("amen.wav"):slice(8)
track("hats", amen):steps("3 3 3 3  3 3 3 3  3 3 3 3  3 3 3 3"):pan(lfo("2 compases", -0.6, 0.6))
```

### `pista:swing(porcentaje)`

Devuelve: pista.

Swing: retrasa los contratiempos de semicorchea (pasos 2, 4, 6...). El porcentaje es el de las cajas de ritmos (MPC): dónde cae el contratiempo dentro de cada par de semicorcheas. 50 es recto, 54-58 un swing suave, 66 casi tresillo y 75 el máximo (el contratiempo a medio paso). Se aplica después de `every`, `chance` y `fill`, al frame exacto y igual en cada pasada.

- `porcentaje`: de 50 (recto) a 75

```lua
local amen = sample("amen.wav"):slice(8)
track("drums", amen)
  :steps("1 2 2 2  3 2 4 2  5 6 2 2  7 2 8 2")
  :swing(62)
```

### `pista:humanize(ms [, volumen [, semilla]])`

Devuelve: pista.

Imperfección humana: cada golpe se retrasa y se suaviza un poco al azar. Cada golpe se retrasa entre 0 y `ms` milisegundos (nunca se adelanta; como mucho, 0,9 pasos) y su volumen se multiplica por un factor entre `1 - volumen` y 1 (nunca sube). Cada compás sale distinto: la pista pasa a un ciclo de 16 compases, como con `chance`. Con `semilla`, siempre igual; sin ella, el azar de la carga (como `chance`), distinto para cada pista. Se suma al swing.

- `ms`: retraso máximo en milisegundos (0 a 100)
- `volumen`: cuánto puede bajar el volumen (0 a 1; por defecto 0)
- `semilla`: número entero; con la misma, el mismo resultado

```lua
local amen = sample("amen.wav"):slice(8)
track("drums", amen)
  :steps("1 2 2 2  3 2 4 2  5 6 2 2  7 2 8 2")
  :swing(56)
  :humanize(6, 0.2, 1)
```

### `pista:fill(efecto, n [, pasos [, valor]])`

Devuelve: pista.

Relleno: los últimos pasos del último compás de cada n, con un efecto. En los compases n, 2n, 3n... (contando desde 1: el último de cada grupo de n) los últimos `pasos` pasos (4 por defecto: el último tiempo) sufren el efecto. `stutter` y `roll` los rellenan con el golpe del principio del relleno (o el último antes, si ahí hay silencio). Se aplica en orden con `every` y `chance`, y alarga el ciclo como `every(n)`. Los efectos están en la tabla de abajo.

- `efecto`: el efecto (ver la tabla)
- `n`: cada cuántos compases (1 a 64)
- `pasos`: cuántos pasos del final (1 a 64, como mucho la longitud; por defecto 4)
- `valor`: el valor del efecto, si lo lleva

| Efecto | Valor | Qué hace |
| --- | --- | --- |
| `"stutter"` | retrigger, 1 a 16 (4) | Repite el último golpe en cada paso del relleno, con retrigger |
| `"roll"` | — | Como stutter, cada vez más rápido: retrigger 2, 4, 8, 16 |
| `"reverse"` | — | Invierte el sentido de los golpes del relleno |
| `"pitch"` | semitonos, -36 a 36 (12) | Sube (o baja) los golpes del relleno |
| `"mute"` | — | Silencio: un corte antes del compás siguiente |

```lua
local amen = sample("amen.wav"):slice(8)
track("drums", amen)
  :steps("1 . 2 .  3 . 4 .  5 6 2 .  7 . 8 .")
  :fill("roll", 4)
  :fill("pitch", 8, 2, 7)
```

### `pista:mute([sí])`

Devuelve: pista.

Silencia la pista (sigue avanzando, no se oye). `mute(false)` la deja sonar. Entra al empezar el compás, como cualquier cambio del script. Desde la interfaz hay además un mute en vivo, inmediato (paleta o `m` en el modo tocar), que se suma a este: la pista calla si cualquiera de los dos lo pide.

- `sí`: `true` (por defecto) silencia; `false` no

```lua
local amen = sample("amen.wav"):slice(8)
track("drums", amen):steps("1 . 2 .  3 . 4 .  5 6 2 .  7 . 8 .")
track("kick", amen:slice_at(1)):steps("x . . .  x . . .  x . . .  x . . ."):mute()
```

### `pista:solo([sí])`

Devuelve: pista.

Pone la pista en solo: mientras alguna lo esté, solo suenan las que están en solo. Como `mute`: entra al compás y se suma al solo en vivo de la interfaz (paleta o `s` en el modo tocar). Un mute gana a un solo.

- `sí`: `true` (por defecto) la pone en solo; `false` no

```lua
local amen = sample("amen.wav"):slice(8)
track("drums", amen):steps("1 . 2 .  3 . 4 .  5 6 2 .  7 . 8 ."):solo()
track("kick", amen:slice_at(1)):steps("x . . .  x . . .  x . . .  x . . .")
```

### `pista:fx(nombre, parámetros)`

Devuelve: pista.

Añade un efecto a la cadena de la pista (hasta 4, en orden). Un parámetro desconocido da error, para pillar las erratas. Los efectos y sus parámetros están en la tabla de abajo.

El filtro es un biquad de 12 dB/octava; `q = 0.707` es la respuesta más plana y valores altos resuenan en el corte. El bitcrusher reduce la resolución a `bits` y repite cada muestra `downsample` veces (reducción de la frecuencia de muestreo, sin filtro: suena sucio a propósito). El EQ tiene hasta 4 bandas en serie, cada una con su `freqN`, `gainN`, `qN` y `typeN` (`"bell"`, campana; `"lowshelf"` y `"highshelf"`, estantes que mueven todo lo que queda por debajo o por encima); una banda con `gainN = 0` no hace nada, y un EQ sin ganancias suena idéntico a no tenerlo. Al recargar con otras ganancias, el EQ va hacia ellas en unos 5 ms, sin clic. El EQ dinámico (`dyneq`) tiene una banda que escucha (un paso de banda en `freq` y `q`) y baja solo mientras lo que oye pasa de `threshold`: `1 − 1/ratio` dB por dB de más, como mucho `range`, con `attack` y `release`. Sin `sidechain` escucha la propia pista; con `sidechain = "kick"`, la salida de esa pista (tras sus efectos, antes de su volumen, aunque esté en mute o fuera del solo). Por debajo del umbral suena idéntico a no tenerlo. Todos los parámetros numéricos admiten `ramp()` y `lfo()` en lugar de un número (los tipos y el `sidechain`, no: ver «Qué se puede automatizar», tras `lfo`); el corte del primer filtro, además, puede cambiar en cada golpe con `~` en `steps` (`3~800`).

- `nombre`: el efecto (ver la tabla)
- `parámetros`: tabla con los parámetros del efecto

| Efecto | Parámetro | Qué es | Por defecto | Valores |
| --- | --- | --- | --- | --- |
| `"filter"` | `type` | tipo de filtro | `"lowpass"` | `"lowpass"` o `"highpass"` |
|  | `cutoff` | frecuencia de corte en Hz | obligatorio | 20–20000 |
|  | `q` | resonancia | `0.707` | 0.1–20 |
| `"lowpass"` (o `"lp"`) | `cutoff` | frecuencia de corte en Hz | obligatorio | 20–20000 |
|  | `q` | resonancia | `0.707` | 0.1–20 |
| `"highpass"` (o `"hp"`) | `cutoff` | frecuencia de corte en Hz | obligatorio | 20–20000 |
|  | `q` | resonancia | `0.707` | 0.1–20 |
| `"bitcrush"` (o `"bitcrusher"`) (o `"crush"`) | `bits` | resolución | `8` | 1–16 |
|  | `downsample` | repite cada muestra n veces | `1` | 1–64 |
| `"compressor"` (o `"comp"`) | `threshold` | umbral en dB: por encima empieza a comprimir | `-18` | -60–0 |
|  | `ratio` | cuántos dB de entrada por cada dB de salida sobre el umbral | `4` | 1–20 |
|  | `attack` | ms que tarda en comprimir (corto: aplasta los ataques) | `5` | 0.1–200 |
|  | `release` | ms que tarda en soltar | `100` | 1–2000 |
|  | `makeup` | dB que se suben después, para compensar | `0` | 0–24 |
| `"distortion"` (o `"drive"`) | `drive` | ganancia antes de saturar | `4` | 1–50 |
|  | `mix` | parte saturada (1 = solo la saturada) | `1` | 0–1 |
| `"gain"` | `level` | volumen (1 = igual, 0 = silencio) | `1` | 0–4 |
| `"eq"` | `freq1` | frecuencia de la banda 1 en Hz | `100` | 20–20000 |
|  | `gain1` | dB que sube (o baja, en negativo) la banda 1; 0 la apaga | `0` | -24–24 |
|  | `q1` | ancho de la banda 1 (más q, más estrecha) | `1` | 0.1–18 |
|  | `type1` | forma de la banda 1 | `"bell"` | `"bell"`, `"lowshelf"` o `"highshelf"` |
|  | `freq2` | frecuencia de la banda 2 en Hz | `500` | 20–20000 |
|  | `gain2` | dB que sube (o baja, en negativo) la banda 2; 0 la apaga | `0` | -24–24 |
|  | `q2` | ancho de la banda 2 (más q, más estrecha) | `1` | 0.1–18 |
|  | `type2` | forma de la banda 2 | `"bell"` | `"bell"`, `"lowshelf"` o `"highshelf"` |
|  | `freq3` | frecuencia de la banda 3 en Hz | `2000` | 20–20000 |
|  | `gain3` | dB que sube (o baja, en negativo) la banda 3; 0 la apaga | `0` | -24–24 |
|  | `q3` | ancho de la banda 3 (más q, más estrecha) | `1` | 0.1–18 |
|  | `type3` | forma de la banda 3 | `"bell"` | `"bell"`, `"lowshelf"` o `"highshelf"` |
|  | `freq4` | frecuencia de la banda 4 en Hz | `8000` | 20–20000 |
|  | `gain4` | dB que sube (o baja, en negativo) la banda 4; 0 la apaga | `0` | -24–24 |
|  | `q4` | ancho de la banda 4 (más q, más estrecha) | `1` | 0.1–18 |
|  | `type4` | forma de la banda 4 | `"bell"` | `"bell"`, `"lowshelf"` o `"highshelf"` |
| `"dyneq"` | `freq` | centro de la banda en Hz (la que se escucha y la que baja) | obligatorio | 20–20000 |
|  | `q` | ancho de la banda (más q, más estrecha) | `1` | 0.1–18 |
|  | `type` | forma de la banda que baja | `"bell"` | `"bell"`, `"lowshelf"` o `"highshelf"` |
|  | `threshold` | nivel en dB de la banda (en lo que se escucha) desde el que baja | `-30` | -60–0 |
|  | `ratio` | por cada dB de más, baja 1 − 1/ratio dB | `4` | 1–20 |
|  | `range` | lo más que baja, en dB | `6` | 0–24 |
|  | `attack` | ms que tarda en bajar | `5` | 0.1–200 |
|  | `release` | ms que tarda en volver | `120` | 1–2000 |
|  | `sidechain` | pista que se escucha en vez de la propia (aunque esté en mute) | `la propia` | nombre de otra pista |

```lua
local amen = sample("amen.wav"):slice(8)
track("drums", amen)
  :steps("1 . 2 .  3 . 4 .  5 6 2 .  7 . 8 .")
  :fx("filter", { type = "lowpass", cutoff = 3000 })
  :fx("highpass", { cutoff = 80, q = 0.9 })
  :fx("bitcrush", { bits = 6, downsample = 2 })
track("drums")
  :fx("eq", { freq1 = 300, gain1 = -3, q1 = 2, type2 = "highshelf", freq2 = 8000, gain2 = 2 })
```

### `pista:mode(modo)`

Devuelve: pista.

"gate": cada golpe suena lo que dura su paso; "oneshot" (por defecto): el corte entero. En el modo gate la voz se cierra al acabar su paso (o su parte, con retrigger) y hace la cola del corte (`cortes:set(i, { release = ms })`, o el fundido mínimo de 1,5 ms). En one-shot el corte suena hasta su final o hasta el siguiente golpe de la pista.

- `modo`: `"gate"` u `"oneshot"`

```lua
local amen = sample("amen.wav"):slice(8)
track("drums", amen):steps("1 1 2 2  3 3 4 4  5 5 6 6  7 7 8 8"):mode("gate")
```

### `pista:choke(grupo)`

Devuelve: pista.

Grupo de choke (1 a 16): un golpe de la pista calla a las demás del grupo. Como en las cajas de ritmos, el charles cerrado calla al abierto: ponlos en el mismo grupo. El corte callado hace el fundido mínimo (1,5 ms). Vale también para los pads del modo tocar. `choke(0)` o `choke()` lo quita.

- `grupo`: de 1 a 16 (0, ninguno)

```lua
local amen = sample("amen.wav"):slice(8)
track("abierto", amen:slice_at(3)):steps("x . . .  . . x .  x . . .  . . x ."):choke(1)
track("cerrado", amen:slice_at(5)):steps(". . x .  . . . x  . . x .  . . . x"):choke(1)
```

### `pista:midi(canal [, nota_base])`

Devuelve: pista.

Cada golpe sale también como una nota MIDI (corte 1 = nota_base, 36 por defecto). Con `--allow-midi`, las notas salen por el puerto virtual «sampler salida» (ALSA): conéctalo a Surge XT u otro synth (ver «MIDI» en la sección 2). La nota es `nota_base` más el número de corte menos 1, la velocidad sale del volumen del paso (`@`) y cada nota dura lo que su paso (o su parte, con retrigger). Una pista en mute, o fuera del solo, no manda nada. El sample sigue sonando: para oír solo el synth, `:gain(0)`. Los pads del modo tocar no mandan notas. En una pista con instrumento va sin `nota_base`: cada nota (también las de un acorde) sale a su altura, la raíz (`root`) más sus semitonos, y dura su paso y sus `_` (el release es solo del sample); una nota robada por falta de voces se apaga en ese momento.

- `canal`: canal MIDI, de 1 a 16
- `nota_base`: la nota del corte 1 (0 a 127; 36 por defecto); en un instrumento, no va

```lua
local amen = sample("amen.wav"):slice(8)
-- Una línea de bajo para el synth: el corte 1 es do (48), el 3, re.
track("bajo", amen)
  :steps("1 . . 1  . . 3 .  1 . . .  5 . 3 .")
  :midi(2, 48)
  -- Bajo, para que mande el synth (con 0, solo el synth).
  :gain(0.3)
```

### `pista:grains([parámetros])`

Devuelve: pista.

Toca el sample en granos: cada golpe abre una nube sobre su corte mientras dura su paso. Cada golpe de la pista, en vez de tocar su corte, hace nacer granos de él durante su paso (y sus `_`): trozos de `size` ms con ventana de Hann, `density` por segundo, leídos desde `pos` (fracción del corte) más lo que avanza la lectura (`speed`: 1 recorre el corte a su velocidad, 0 lo congela) más hasta `jitter` ms al azar, con el pitch del paso más `pitch` más hasta `spread` semitonos al azar. Al acabar el paso no nacen más y los que suenan acaban su ventana. Todos los números admiten `ramp()` y `lfo()` (se miran al nacer cada grano). El azar sale de `seed` y de la posición de la canción: la misma canción suena igual en cada render; sin `seed`, una por pista sacada de su nombre. Hasta 32 granos a la vez por pista (`density × size`). Una pista con granos no puede ser instrumento. En el modo tocar (y con el MIDI de entrada) la tecla abre una nube mientras se mantiene: con varios cortes elige el corte de donde salen los granos, y con uno solo (lo normal en una textura) es el pitch, un semitono por tecla; el resto son los parámetros de la pista.

- `parámetros`: tabla con `size`, `density`, `pos`, `speed`, `jitter`, `pitch`, `spread` y `seed` (ver abajo)

| Parámetro | Qué es | Por defecto | Valores |
| --- | --- | --- | --- |
| `size` | duración de cada grano, en ms (admite `ramp()` y `lfo()`) | `80` | 5–1000 |
| `density` | granos que nacen por segundo; con `size`, como mucho 32 a la vez (`density × size / 1000`) | `20` | 0.5–400 |
| `pos` | de dónde se leen, en fracción del corte (0 su inicio, 1 su final) | `0` | 0–1 |
| `speed` | cuánto avanza la lectura: 1 recorre el corte a su velocidad, 0 lo congela, negativo va hacia atrás | `1` | -4–4 |
| `jitter` | desplazamiento al azar de la posición de cada grano, en ms (±) | `0` | 0–2000 |
| `pitch` | pitch de los granos en semitonos (se suma al del paso) | `0` | -24–24 |
| `spread` | desplazamiento al azar del pitch de cada grano, en semitonos (±) | `0` | 0–24 |
| `seed` | semilla del azar (`jitter`, `spread`): con la misma, la misma nube; sin ella, una sacada del nombre de la pista | `del nombre` | entero de 0 en adelante |

```lua
local amen = sample("amen.wav"):slice(8)
-- Una nube larga sobre la caja, cada vez más densa.
track("nube", amen)
  :steps("3 _ _ _  _ _ _ _  _ _ _ _  _ _ _ _")
  :grains({ size = 60, density = ramp(5, 60, "1 compás"), jitter = 20, spread = 3 })
```

### `pista:freeze([pos [, parámetros]])`

Devuelve: pista.

Congela un instante del corte y lo sostiene en granos (`grains` con `speed = 0`). Como `grains`, pero la lectura no avanza: todos los granos salen de `pos` (fracción del corte, 0 por defecto; admite `ramp()` y `lfo()` para pasearse por él), más su `jitter`. Con granos largos y solapados, un instante se convierte en un sonido sostenido mientras dure el paso y sus `_`.

- `pos`: dónde se congela, en fracción del corte (0 a 1)
- `parámetros`: la tabla de `grains`, sin `speed`

```lua
local amen = sample("amen.wav"):slice(8)
-- La cola de la caja, sostenida dos compases y una octava abajo.
track("hielo", amen)
  :length(32)
  :steps("3 _ _ _  _ _ _ _  _ _ _ _  _ _ _ _  _ _ _ _  _ _ _ _  _ _ _ _  _ _ _ _")
  :freeze(0.4, { size = 120, density = 25, jitter = 15, pitch = -12 })
```

### `pista:pitch(semitonos)`

Devuelve: pista.

Transpone todos los golpes de la pista (se suma al pitch de cada paso). Como `^` en cada paso: cambia la velocidad de lectura (y la duración); con `%` en el paso, sin cambiar la duración. Admite `ramp()` y `lfo()`: cada golpe toma el valor de la curva en el momento en que suena y lo mantiene mientras dura (una voz no cambia de velocidad a media lectura). El total, con el del paso, como mucho ±36 semitonos. Vale también en los instrumentos y en los granos.

- `semitonos`: -36 a 36, `ramp()` o `lfo()`

```lua
local amen = sample("amen.wav"):slice(8)
-- Cada golpe, un poco más agudo que el anterior durante 4 compases.
track("sube", amen):steps("1 . 2 .  3 . 4 .  5 6 2 .  7 . 8 ."):pitch(ramp(0, 12, "4 compases"))
```

### `pista:formant(semitonos)`

Devuelve: pista.

Formantes de los pasos con % (pitch sin cambiar la duración): 0 los deja donde estaban, n los desplaza n semitonos. Al transponer, la envolvente del sonido sube o baja con el tono: una voz subida suena a ardilla y una caja bajada, a bombo de juguete. Con `formant(0)`, los pasos con `%` cambian de tono pero su envolvente se queda donde estaba (cada grano se analiza con LPC y se blanquea y se vuelve a colorear con la del original); con `formant(n)` se desplaza `n` semitonos por su cuenta, también con `%0` (cambiar el timbre sin cambiar la nota). Solo actúa en los pasos con `%` (si no hay ninguno, avisa); con `^` el sonido se lee más deprisa o más despacio entero, como un sampler clásico. Cuesta otro 2 % de un núcleo por voz. En un instrumento va en su tabla: `sample:instrument({ keep = true, formant = 0 })`. Admite `ramp()` y `lfo()`, por golpe.

- `semitonos`: -24 a 24 (0 deja los formantes donde estaban), `ramp()` o `lfo()`

```lua
local amen = sample("amen.wav"):slice(8)
-- La caja una quinta arriba sin que encoja su cuerpo.
track("drums", amen)
  :steps("1 . 3 .  . . 3%7 .  1 1 3%-5 .  . . 3%7 .")
  :formant(0)
```

### `pista:send(bus, nivel)`

Devuelve: pista.

Manda una parte de la pista al bus de delay o de reverb (0 a 1). El envío va después del volumen, el mute y el solo de la pista: una pista en mute no suena tampoco en el delay (pero lo que ya estaba en la cola sigue). Entra con el patrón, al empezar el compás, suavizado para que no haga clic. El nivel admite `ramp()` y `lfo()`. Los parámetros de los buses, con `bus()`.

- `bus`: `"delay"` o `"reverb"`
- `nivel`: cuánto se manda (0 a 1; 0 quita el envío), `ramp()` o `lfo()`

```lua
local amen = sample("amen.wav"):slice(8)
track("drums", amen)
  :steps("1 . 2 .  3 . 4 .  5 6 2 .  7 . 8 .")
  :send("delay", 0.3)
  :send("reverb", 0.15)
```

### `pista:every(n, función)`

Devuelve: pista.

En los compases 0, n, 2n... llama a función(compás). Como en Tidal, cuenta los compases de la canción (en una pista con `:length(n)`, las vueltas de su patrón). La función puede cambiar ese compás con los métodos de la tabla de abajo, que se pueden encadenar. Dentro de la función no se puede llamar a `track`, `bpm` ni `sample`.

- `n`: cada cuántos compases (1 a 64)
- `función`: recibe el compás (`function(c) ... end`)

| Método | Qué hace |
| --- | --- |
| `c:steps(texto)` | Sustituye los pasos del compás. |
| `c:melody(notas)` | Sustituye los pasos del compás por notas, como pista:melody. |
| `c:reverse()` | Invierte el sentido de cada golpe (lo que iba al revés vuelve al derecho). |
| `c:pitch(st)` | Pone el pitch de cada golpe (-36 a 36). |
| `c:retrig(n)` | Pone el retrigger de cada golpe (1 a 16). |
| `c:gain(g)` | Pone el volumen de cada golpe (0 a 4); no el de la pista. |

```lua
local amen = sample("amen.wav"):slice(8)
track("drums", amen)
  :steps("1 . 2 .  3 . 4 .  5 6 2 .  7 . 8 .")
  :every(4, function(c) c:reverse() end)
```

### `pista:chance(p, efecto [, valor])`

Devuelve: pista.

Cada golpe sufre el efecto con probabilidad p. El azar se tira al cargar, para 16 compases que se repiten. Cada carga tira otra vez los dados. Los efectos están en la tabla de abajo.

`every` y `chance` se aplican en el orden en que se escriben. La pista queda como un ciclo de compases: el mínimo común múltiplo de los `every`, con 16 si hay `chance`, hasta 64 (más da error).

- `p`: probabilidad (0 a 1)
- `efecto`: el efecto (ver la tabla)
- `valor`: el valor del efecto, si lo lleva

| Efecto | Valor | Qué hace |
| --- | --- | --- |
| `"retrig"` (o `"stutter"`) | n (1 a 16) | Retrigger del golpe |
| `"reverse"` | — | Invierte su sentido |
| `"pitch"` | semitonos (-36 a 36) | Pone su pitch |
| `"gain"` | volumen (0 a 4) | Pone su volumen |
| `"mute"` | — | Lo quita |
| `"accent"` | — | Le pone accent (`!`; solo en un sintetizador) |
| `"slide"` | — | Lo liga a la nota anterior (`/`; solo en un sintetizador) |

```lua
local amen = sample("amen.wav"):slice(8)
track("drums", amen)
  :steps("1 . 2 .  3 . 4 .  5 6 2 .  7 . 8 .")
  :chance(0.2, "retrig", 4)
  :chance(0.1, "mute")
```

### Snippets

Trozos de script con campos para rellenar. Se insertan desde el autocompletado (escribiendo su nombre al inicio de una línea) o desde la paleta de comandos; Tab salta de un campo al siguiente.

| Snippet | Qué inserta |
| --- | --- |
| `amen` | Tempo, el break cortado y una pista que lo toca en orden. |
| `fill` | Relleno al final de cada N compases en una pista existente (roll, stutter...). |
| `ghost` | Pista de ghost notes: un golpe muy suave en contratiempos que aparece al azar. |
| `intro` | Pista de intro: pocos golpes, filtrada y más baja. |
| `euclid` | Pista con un ritmo euclídeo (k golpes repartidos en n pasos). |
| `melodia` | Melodía con un corte sobre una escala: grados que se convierten en pitch. |
| `polirritmo` | Pista de otra longitud (12 pasos contra el compás de 16): polirritmia. |
| `barrido` | Filtro que se abre poco a poco (ramp) y un envío a la reverb en una pista existente. |
| `groove` | Swing y humanize en una pista existente, para que no suene a máquina. |
| `instrumento` | Un sample tocado como instrumento (raíz, bucle, voces y ADSR) con una melodía. |

`amen`:

```lua
bpm(170)
local amen = sample("amen.wav"):slice(8)

track("drums", amen)
  :steps("1 . 2 .  3 . 4 .  5 6 2 .  7 . 8 .")
```

`fill` (las primeras líneas son lo que necesita tener antes):

```lua
local amen = sample("amen.wav"):slice(8)
track("drums", amen):steps("1 . 2 .  3 . 4 .  5 6 2 .  7 . 8 .")
track("drums")
  :fill("roll", 4)
```

`ghost` (las primeras líneas son lo que necesita tener antes):

```lua
local amen = sample("amen.wav"):slice(8)
track("ghost", amen:slice_at(3))
  :steps(". x@0.2 . x@0.15  . x@0.2 x@0.1 .  . x@0.2 . x@0.15  . . x@0.2 x@0.1")
  :chance(0.4, "mute")
  :fx("lowpass", { cutoff = 4000 })
```

`intro` (las primeras líneas son lo que necesita tener antes):

```lua
local amen = sample("amen.wav"):slice(8)
track("intro", amen)
  :steps("1 . . .  . . . .  1 . . .  . . . .")
  :fx("lowpass", { cutoff = 800 })
  :gain(0.8)
```

`euclid` (las primeras líneas son lo que necesita tener antes):

```lua
local amen = sample("amen.wav"):slice(8)
track("hats", amen)
  :steps(euclid(5, 16, "2@0.6"))
```

`melodia` (las primeras líneas son lo que necesita tener antes):

```lua
local amen = sample("amen.wav"):slice(8)
local escala = scale("minor", "C")
track("melodia", amen)
  :steps(escala:melody("1 . 3 .  5 . 3 .  8 . 7 .  5 . 3 .", "3"))
  :gain(0.6)
```

`polirritmo` (las primeras líneas son lo que necesita tener antes):

```lua
local amen = sample("amen.wav"):slice(8)
track("tres", amen)
  :length(12)
  :steps("3 . . 3@0.5  . . 3 .  . 3@0.5 . .")
```

`barrido` (las primeras líneas son lo que necesita tener antes):

```lua
local amen = sample("amen.wav"):slice(8)
track("drums", amen):steps("1 . 2 .  3 . 4 .  5 6 2 .  7 . 8 .")
track("drums")
  :fx("lowpass", { cutoff = ramp(200, 8000, "4 compases") })
  :send("reverb", 0.2)
```

`groove` (las primeras líneas son lo que necesita tener antes):

```lua
local amen = sample("amen.wav"):slice(8)
track("drums", amen):steps("1 . 2 .  3 . 4 .  5 6 2 .  7 . 8 .")
track("drums")
  :swing(58)
  :humanize(6, 0.15)
```

`instrumento`:

```lua
local piano = sample("amen.wav"):instrument({
  root = "C3", loop = { 200, 600 }, voices = 4, adsr = { 5, 80, 0.7, 300 },
})
track("lead", piano)
  :melody("c _ e _  g _ e _  c+e+g _ _ _  . . . .")
```

<!-- api:fin -->

## 5. Cuantización, recarga atómica y errores

- **Todo cambio entra al empezar el siguiente compás.** Al guardar (o con
  Ctrl-L) el compás que suena termina tal cual y el nuevo empieza en su
  paso 0. Lo mismo con el tempo, el volumen y los efectos.
- **La recarga es atómica.** El script se evalúa entero en el hilo de
  control; si todo va bien, el tempo y los patrones de todas las pistas se
  envían juntos, como un lote, por una cola sin bloqueos. El motor los
  guarda aparte hasta tener el lote completo, así que nunca suena un compás
  con la mitad del cambio.
- **Si hay un error, sigue sonando lo anterior.** Sintaxis, una función mal
  llamada, notación inválida, un corte que no existe, un archivo que no
  está, un bucle infinito (se corta a los 2 s): no se envía nada al motor.
  El error aparece en el editor (borde rojo, mensaje al pie y la línea con
  fondo rojo); Ctrl-G lleva a ella. Al corregir y guardar, el cambio entra
  en el compás siguiente.
- **Las pistas conservan su sitio.** Una pista que sigue en el script
  sigue en la misma pista del motor; una que desaparece se calla en el
  compás siguiente.
- **`every` y `chance` siguen el compás de la canción**, no el de la
  recarga: `every(4)` cae en los compases 0, 4, 8... aunque recargues a
  mitad.
- **Tocar con el teclado no se cuantiza:** el modo tocar es interpretación,
  no un cambio de código.
- **Exportar no toca lo que suena:** renderiza en un motor aparte lo último
  cargado bien (si el texto tiene cambios sin cargar o con error, el
  mensaje lo avisa).

## 6. Recetas para amen breaks

Las recetas usan los cortes del break sintético (1 bombo, 2 charles, 3
caja, 4 charles, 5 bombo, 6 bombo, 7 caja, 8 charles). Cada una está en
`scripts/ejemplos/`; para oírla:

```sh
scripts/render-demo.sh   # una vez, para tener renders/synth_break.wav
cargo run -p tui -- --script scripts/ejemplos/01-ritmo-basico.lua --render renders/receta.wav --bars 8
```

En `scripts/ejemplos/`, `amen.wav` es un enlace al break sintético. Para
usar tu amen sin tocar los ejemplos, haz un proyecto con
`script = scripts/ejemplos/01-ritmo-basico.lua` y `samples = <tu carpeta>`.

### Ritmo básico

El break tal cual: cada golpe en su sitio, a tempo de jungle.

```lua
-- 01-ritmo-basico.lua
bpm(170)
local amen = sample("amen.wav"):slice(8)

track("drums", amen)
  :steps("1 . 2 .  3 . 4 .  5 6 2 .  7 . 8 .")
```

### Variación con every

Un compás de relleno cada 4 y un toque al revés cada 8.

```lua
-- 02-variacion-every.lua
bpm(170)
local amen = sample("amen.wav"):slice(8)

track("drums", amen)
  :steps("1 . 2 .  3 . 4 .  5 6 2 .  7 . 8 .")
  -- Compases 0, 4, 8...: otro patrón, más cargado al final.
  :every(4, function(c) c:steps("1 . 2 .  3 . 4 .  5 6 3 3  7 3 7 3") end)
  -- Compases 0, 8, 16...: además, todo al revés.
  :every(8, function(c) c:reverse() end)
```

### Ghost notes con chance

Una segunda pista con cajas muy suaves en los contratiempos, que aparecen y
desaparecen al azar.

```lua
-- 03-ghost-notes.lua
bpm(170)
local amen = sample("amen.wav"):slice(8)

track("drums", amen)
  :steps("1 . 2 .  3 . 4 .  5 6 2 .  7 . 8 .")
  :gain(0.8)

track("ghost", amen:slice_at(3))
  :steps(". x@0.2 . x@0.15  . x@0.2 x@0.1 .  . x@0.2 . x@0.15  . . x@0.2 x@0.1")
  :chance(0.4, "mute")
  :fx("lowpass", { cutoff = 4000 })
```

### Stutter para fills

El último tiempo de cada cuatro compases se rompe en repeticiones cada vez
más rápidas; además, algún golpe suelto tartamudea.

```lua
-- 04-stutter-fills.lua
bpm(170)
local amen = sample("amen.wav"):slice(8)

track("drums", amen)
  :steps("1 . 2 .  3 . 4 .  5 6 2 .  7 . 8 .")
  :every(4, function(c) c:steps("1 . 2 .  3 . 4 .  5 6 2 .  3r2 3r4 7r8 7r16") end)
  :chance(0.1, "stutter", 3)
```

### Pitch para melodías con cortes

Un solo golpe (la caja) afinado como un instrumento. Subir el pitch acorta
el golpe; bajarlo lo alarga.

```lua
-- 05-melodia-pitch.lua
bpm(170)
local amen = sample("amen.wav"):slice(8)

track("drums", amen)
  :steps("1 . . .  . . . .  5 . . .  . . . .")
  :gain(0.7)

track("melodia", amen:slice_at(3))
  :steps("x . x^3 .  x^7 . x^5 .  x^12 . x^10 .  x^7 . x^3 x^-5")
  :gain(0.5)
```

### Melodías con un instrumento

Con `sample:instrument()` el sample se toca como notas: cada golpe abre una
voz sin cortar las demás, con su envolvente ADSR, y dura su paso más sus
`_` (y luego el release). `melody` escribe las notas por su nombre desde
la raíz (`root`, la nota en la que suena el sample tal cual) y `+` hace
acordes. Con `loop`, una nota larga sigue en bucle sin clic. El tracker
enseña las notas (`C3+E3+G3`, y `│` en las filas que alargan una nota) y
el panel de efectos, las voces que suenan (`♪ C3 3/4`).

```lua
bpm(96)
local amen = sample("amen.wav"):slice(8)
track("drums", amen):steps("1 . 2 .  3 . 4 .  5 6 2 .  7 . 8 .")

local pad = sample("amen.wav"):instrument({
  root = "C3", loop = { 300, 700 }, voices = 6, adsr = { 40, 200, 0.6, 600 },
})
track("pad", pad)
  :melody("c+eb+g _ _ _  _ _ _ _  ab2+c+eb _ _ _  bb2+d+f _ _ _")
  :gain(0.4)
```

### Un bajo ácido

`synth("acid", {...})` es un bajo del 303 sin sample: un oscilador de
sierra (o cuadrada, `wave = "square"`) por un paso bajo de 4 polos con
mucha resonancia, monofónico. Se toca como un instrumento (`melody`,
`scale`, los pads, el MIDI) y su raíz es C2, así que `melody("c eb g")`
suena en la octava de un bajo. Lo que lo hace ácido está en cada paso:
`!` es el **accent** (la nota más fuerte y el filtro más abierto, con un
decay corto; varios seguidos se acumulan y el filtro se va abriendo) y
`/` el **slide** (la nota llega ligada desde la anterior: se desliza
hasta su altura en `glide` ms y las envolventes no vuelven a empezar, así
que el filtro sigue cerrándose por donde iba). La nota de antes de un
slide suena hasta que llega la ligada, aunque llegue tarde con swing; un
slide después de un silencio es una nota normal. Los parámetros se mueven
con `ramp()` y `lfo()` sin clics, y los efectos de la pista (`:fx`) van
detrás. En el tracker, las notas salen por su nombre con sus marcas
(`C2!`, `D#2/`); en lugar del sample, su panel enseña sus parámetros (con
`∿` en los que siguen una curva) y el de efectos dice `♪ acid C2 1/1`.
Los pads del modo tocar y el MIDI de entrada lo tocan en vivo, desde la
raíz, y `:midi(canal)` manda sus notas fuera como las de un instrumento.

```lua
-- 09-bajo-acido.lua
bpm(132)
local amen = sample("amen.wav"):slice(8)
track("drums", amen)
  :steps("1 . 2 .  3 . 4 .  5 6 2 .  7 . 8 .")
  :gain(0.6)

local bass = synth("acid", {
  cutoff = 320, res = 0.82, envmod = 0.65, decay = 220, accent = 0.8,
  dist = 0.25, glide = 70,
})
track("acid", bass)
  :melody("c c! c' eb/  c . c bb1/  c! g1 . c/  eb! f/ eb c")
  :every(4, function(c) c:melody("c c! g/ c  bb1! . c c/  f! eb/ c . c! c/ eb g/") end)
  :fx("lowpass", { cutoff = lfo("8 compases", 1500, 6000) })
  :gain(0.7)
```

Para que el filtro se abra poco a poco a lo largo de la canción, una
rampa en `cutoff` (o en `envmod`): `cutoff = ramp(200, 2000, "16
compases")`. Y para variar los accents sin escribirlos, `:chance(0.3,
"accent")` (con `"slide"`, igual).

### Un subgrave limpio

`synth("sub", {...})` (o `"subbass"`) da profundidad sin ocupar los
medios: un seno por un paso bajo suave (`tone`), monofónico, con la raíz
en C1 (32,7 Hz), así que `melody("c")` ya suena en la octava del sub y
`ab0`, `bb0` y `g0` bajan de ahí. Está hecho para no hacer ruido: cada
nota que empieza desde el silencio arranca en el cruce por cero (sin el
golpe sordo que haría un seno a media onda) y sube en `attack` ms; una
nota pegada a la anterior sigue su fase, sin volver a subir (sin clic al
cambiar de altura); `/` la desliza en `glide` ms y `!` la hace un 40 %
más fuerte. Un seno puro casi no se oye en los altavoces de un portátil:
`drive` lo satura un poco *antes* del filtro, y esos armónicos graves
(los que deja pasar `tone`) lo hacen oír sin ensuciarlo por arriba; la
triangular (`wave = "triangle"`) hace algo parecido. Con un bombo, que el
sub le deje sitio: un EQ dinámico con sidechain baja sus graves mientras
pega el bombo.

```lua
-- 10-subgrave.lua
bpm(170)
local amen = sample("amen.wav"):slice(8)

track("kick", amen:slice_at(1))
  :steps("x . . .  . . . .  x . x .  . . . .")
  :gain(0.4)

track("drums", amen)
  :steps(". . 2 .  3 . 4 .  . 6 2 .  7 . 8 .")
  :gain(0.4)

-- Un subgrave en C1: notas largas, una ligada (/) y un accent (!).
local sub = synth("sub", { tone = 200, drive = 0.2, release = 120 })
track("sub", sub)
  :melody("c _ _ _  _ _ _ _  ab0 _ _ _  bb0! _ g0/ _")
  -- Deja sitio al bombo: sus graves bajan mientras suena.
  :fx("dyneq", { freq = 60, q = 0.7, range = 8, threshold = -30,
                 attack = 2, release = 90, sidechain = "kick" })
  :gain(0.7)
```

Para oírlo bien hacen falta unos auriculares o un altavoz con graves: en
el render, mira el medidor de la pista. `tone` y `drive` admiten `ramp()`
y `lfo()` (un sub que se va abriendo); `attack` y `release` no (la
envolvente se fija al cargar).

### Un sub dentro del bajo (bloque sub)

Cualquier sintetizador de bajo (`acid`, `sub`) tiene debajo un **sub**:
una onda a la frecuencia de su oscilador dividida por `sub_div` (2, una
octava abajo; 3, una octava y una quinta; 4, dos octavas...). No es otro
oscilador: sale de la fase del suyo contando cuántas veces vuelve a
empezar, así que siempre está en fase con él, también al deslizarse con
`/`. `sub` es su nivel (0, apagado: el instrumento suena exactamente como
sin él), `sub_wave` su forma (`"square"`, la de siempre, con los saltos
suavizados para que no aliase; `"tri"` o `"sine"`, más redondas),
`sub_pan` su pan (de potencia constante: abierto a un lado no pierde ni
gana fuerza) y `sub_drive` satura la suma antes del filtro. En la tabla,
`sub`, `sub_pan` y `sub_drive` admiten `ramp()` y `lfo()`; `sub_div` y
`sub_wave` no (son un entero y un nombre). Y por paso, entre llaves, lo
que cambia en esa nota: `{sub=0}` la deja sin sub, `{sub_div=3}` la baja
una quinta más.

```lua
bpm(128)
local bass = synth("acid", {
  cutoff = 500, res = 0.75, envmod = 0.5,
  sub = 0.5, sub_div = 2, sub_wave = "sine", sub_pan = 0.3,
})
track("bass", bass)
  :melody("c c! eb{sub=0} c/  . c{sub_div=3} bb1 c  g1! . c c/  eb f/{sub=0.8} eb c")
```

El tracker marca las notas con su propio sub (`C2{sub}`) y el panel del
sintetizador enseña el bloque sub solo cuando suena.

### Una caja más grave que dura lo mismo

`%` transpone sin cambiar la duración: la caja una octava abajo sigue
ocupando su paso (con `^-12` duraría el doble y se comería el golpe
siguiente). En el tracker sale `03%-12`. Con `:formant(0)`, además, su
cuerpo no cambia: suena más grave pero sigue siendo la misma caja (el
panel de efectos dice `· formant 0`).

```lua
bpm(170)
local amen = sample("amen.wav"):slice(8)
track("drums", amen)
  :steps("1 . 3 .  . . 3%-12 .  1 1 3 .  . . 3%-5 3%7")
  :formant(0)
```

### Texturas con granos y congelar

`:grains()` hace que cada golpe de la pista se deshaga en granos durante su
paso y sus `_`; `:freeze(pos)` congela un instante del corte y lo sostiene.
El panel de efectos lo resume (`⁘ 60 ms · ∿`, `❄ 0.4 · 120 ms · 25/s`).
Todo lo que sea número admite `ramp()` y `lfo()`; el azar (`jitter`,
`spread`) sale de la semilla, así que cada render suena igual.

```lua
bpm(90)
local amen = sample("amen.wav"):slice(8)
track("drums", amen):steps("1 . 2 .  3 . 4 .  5 6 2 .  7 . 8 .")

-- La cola de la caja, congelada y una octava abajo, dos compases.
track("hielo", amen)
  :length(32)
  :steps("3 _ _ _  _ _ _ _  _ _ _ _  _ _ _ _  _ _ _ _  _ _ _ _  _ _ _ _  _ _ _ _")
  :freeze(lfo("2 compases", 0.2, 0.6), { size = 120, density = 25, jitter = 15, pitch = -12 })
  :gain(0.5)

-- Un bombo que se deshace: granos cortos cada vez más densos.
track("polvo", amen)
  :steps("1 _ _ _  _ _ _ _  . . . .  . . . .")
  :grains({ size = 30, density = ramp(5, 120, "1 compás"), jitter = 40, spread = 2, seed = 7 })
  :gain(0.4)
```

### Reverse en el último paso del compás

El golpe del último paso va al revés y "aspira" hacia el compás siguiente.

```lua
-- 06-reverse-final.lua
bpm(170)
local amen = sample("amen.wav"):slice(8)

track("drums", amen)
  :steps("1 . 2 .  3 . 4 .  5 6 2 .  7 . 8 7'")
```

### Break reordenado

Los mismos golpes en otro orden: la base del jungle. Cambia los números y
guarda para oír cada versión en el compás siguiente.

```lua
-- 07-break-reordenado.lua
bpm(172)
local amen = sample("amen.wav"):slice(8)

track("drums", amen)
  :steps("1 . 4 .  7 . 2 3  . 5 6 .  3 . 8 7")
  :every(2, function(c) c:steps("5 . 1 .  3 . 2 .  1 6 7 .  3 4 3 8") end)
```

### Varias pistas

Una pista para el break, otra que refuerza el bombo y otra con charles
agudos y sucios. Cada pista tiene su volumen y sus efectos; el limitador
del final evita que la suma pase de ±1, pero conviene dejar margen.

```lua
-- 08-varias-pistas.lua
bpm(170)
local amen = sample("amen.wav"):slice(8)

track("drums", amen)
  :steps("1 . 2 .  3 . 4 .  5 6 2 .  7 . 8 .")
  :fx("filter", { type = "lowpass", cutoff = 6000 })
  :every(4, function(c) c:reverse() end)
  :gain(0.7)

track("kick", amen:slice_at(1))
  :steps("x . . .  . . . .  x . x .  . . . .")
  :gain(0.4)

track("hats", amen:slice_at(2))
  :steps(". . x^12 .  . . x^12 .  . . x^12 .  . . x^12 x^12r2")
  :fx("highpass", { cutoff = 2000 })
  :fx("bitcrush", { bits = 6, downsample = 2 })
  :gain(0.3)
```

### Ensuciar solo la caja (efectos por corte)

`cortes:fx(cuál, efecto, { ... })` pone un efecto solo en un corte (o en
varios: número, etiqueta o lista). Los demás golpes suenan exactamente
igual; cada golpe de la caja pasa por su bitcrusher y su filtro y luego
por los efectos de la pista. Vale para todas las pistas que usan `amen`
y para sus pads. El panel de efectos dice `◇ 3 snare › crush 6b ÷1 · LP …`.

```lua
bpm(170)
local amen = sample("amen.wav"):slice(8)
amen:name(1, "kick"):name(3, "snare")
amen:fx("snare", "bitcrush", { bits = 6, downsample = 2 })
amen:fx("snare", "lowpass", { cutoff = lfo("2 compases", 1500, 9000) })
amen:fx({ "kick", 5 }, "gain", { level = 0.8 })
track("drums", amen)
  :steps("kick . snare .  3 . 4 .  5 6 snare .  7 . 8 .")
  :fx("highpass", { cutoff = 40 })
```

Los envíos (`send`) son de la pista entera: para una caja con reverb,
sácala a su pista con `amen:slice_at(3)`, que se lleva sus efectos.

### Que el break deje sitio al bombo (EQ dinámico y sidechain)

Un EQ fijo (`fx("eq")`) quita una zona siempre; el dinámico (`fx("dyneq")`)
solo mientras hace falta. Con `sidechain`, la banda de una pista baja solo
cuando suena fuerte en ella otra: aquí los graves del break se apartan
mientras pega el bombo y vuelven en cuanto acaba, y los medios del break
se calman solos cuando un golpe se pasa de fuerte. El análisis de choques
(«Choques de frecuencia») dice qué banda y qué pistas.

```lua
bpm(170)
local amen = sample("amen.wav"):slice(8)

track("kick", amen:slice_at(1))
  :steps("x . . .  . . . .  x . x .  . . . .")
  :gain(0.5)

track("drums", amen)
  :steps("1 . 2 .  3 . 4 .  5 6 2 .  7 . 8 .")
  -- Graves: −6 dB como mucho, solo mientras suena el bombo.
  :fx("dyneq", { freq = 80, q = 1, range = 6, threshold = -30,
                 attack = 2, release = 80, sidechain = "kick" })
  -- Medios: se escucha a sí misma y baja si un golpe pasa de −18 dB.
  :fx("dyneq", { freq = 1500, threshold = -18, range = 4 })
  :gain(0.7)
```

El bombo puede estar en mute y seguir mandando (un «bombo fantasma» que
solo empuja). Una pista no puede escucharse a sí misma por sidechain (sin
`sidechain` ya lo hace) ni en círculo (a escucha a b y b a a).

### Una canción entera con secciones

En vez de un loop plano, secciones con nombre encadenadas en una línea de
tiempo. Las pistas se definen fuera; cada `section()` cambia, solo para
ella, lo que haga falta (patrón, efectos, `mute`, `solo`) o añade pistas
suyas. `arrange()` dice el orden y los compases de cada una. Dentro de
una sección todo cuenta desde su primer compás: el `fill` cada 4 de abajo
cae al final de cada 4 compases del drop, empiece donde empiece.

```lua
bpm(170)
local amen = sample("amen.wav"):slice(8)

track("drums", amen):steps("1 . 2 .  3 . 4 .  5 6 2 .  7 . 8 .")
track("kick", amen:slice_at(1)):steps("x . . .  x . . .  x . . .  x . . .")

section("intro", function()
  track("drums"):mute()
  track("kick"):fx("lowpass", { cutoff = ramp(300, 6000, "4 compases") })
end)

section("drop", function()
  track("drums"):fill("stutter", 4)
end)

section("break", function()
  track("kick"):mute()
  track("drums"):steps("1 . . .  . . 3 .  . . . .  5 . . .")
end)

arrange({ {"intro", 4}, {"drop", 8}, {"break", 2}, {"drop", 8} })
```

Cada cambio de sección entra al empezar un compás y entero. Al acabar, la
línea de tiempo vuelve a empezar. En vivo, desde el modo tocar, `]` y `[`
saltan a la sección siguiente o anterior y `l` repite la que suena (otra
vez, vuelve a la línea de tiempo); la paleta tiene lo mismo y «Saltar a
una sección» (`seccion drop` en la línea de órdenes). Los saltos entran
en el compás siguiente. Encima de la vista del sample, una fila enseña la
línea de tiempo con la sección que suena resaltada y el compás por el que
va (`intro 4 │ ▶ drop 3/8 │ break 2`, con `⟳` si se repite); el tracker y
los paneles enseñan las pistas de esa sección. Recargar el script a mitad de canción
sigue por la misma sección y el mismo compás; si esa sección ya no está,
vuelve al principio y lo avisa. `exportar` (y `--render` sin `--bars`)
graba la línea de tiempo entera, hasta 256 compases. Sin `arrange()` suena
solo la primera sección, como una canción de siempre.

### Variantes A/B

Dos versiones de una parte para compararlas en vivo: `variant()` dice qué
cambia en la B (de una sección o de una pista suelta) y la A es lo
escrito. `v` en el modo tocar (o «Alternar variante A/B» en la paleta)
pasa de una a otra al empezar el compás siguiente; el panel del motor dice
`variante A` o `variante B`. Con secciones, la canción sigue por la misma
sección y el mismo compás. Recargar conserva la que suena, y exportar
graba esa.

```lua
bpm(170)
local amen = sample("amen.wav"):slice(8)
track("drums", amen):steps("1 . 2 .  3 . 4 .  5 6 2 .  7 . 8 .")

-- Una pista suelta: la B con otro patrón.
variant("drums", "B", function(t)
  t:steps("1 . . 1  . . 3 .  1 . 5 .  . 3 . 6")
end)

section("intro", function() track("drums"):mute() end)
section("drop", function() end)
-- Una sección: la B del drop añade un bombo.
variant("drop", "B", function()
  track("kick", amen:slice_at(1)):steps("x . . .  x . . .  x . . .  x . . .")
end)

arrange({ {"intro", 4}, {"drop", 8} })
```

## 7. Solución de problemas

### Cortes mal detectados

- **Mira dónde caen.** `--render` con `--pattern` imprime la lista de cortes
  con su posición en ms, y la interfaz los marca sobre la forma de onda.
- **Faltan golpes juntos** (flams, redobles): baja la separación mínima con
  `--min-gap`, por ejemplo `--min-gap 25`. Al revés, si salen cortes de
  más en un mismo golpe, súbela.
- **Cortes que caen en la cola de un bombo** o charles débiles que no
  aparecen: usa el detector por flujo espectral (el de por defecto,
  `--detector flux`). `--detector energy` es el antiguo y falla justo ahí.
- **Pedir menos cortes** (`slice(6)` en vez de `slice(8)`) se queda con los
  golpes más fuertes.
- En Lua, `:slice(n)` usa siempre el detector por flujo con 40 ms; los
  ajustes `--min-gap` y `--detector` son de la línea de comandos. Para
  probarlos, renderiza el WAV con `--pattern` y mira la lista de cortes.

### Al pegar, todo queda en una línea

La interfaz pide el pegado entre corchetes al terminal; tmux, kitty,
foot, WezTerm, Alacritty, GNOME Terminal y xterm lo admiten. Si aun así
se pierden los saltos de línea, el terminal (o algo entre medias, como un
multiplexor antiguo) manda lo pegado como teclas sueltas que no llegan
juntas: pega con el método del propio terminal en vez de escribirlo por
otro programa, o carga el texto como archivo.

### Saturación

La suma de pistas pasa por un limitador suave: por debajo de 0,9 no toca
nada y por encima comprime hasta ±1 sin recortar. El panel del motor tiene
un medidor del **master** (la suma antes del limitador, con su nivel en
dB) que avisa con `LIMITADOR` (amarillo) cuando el limitador actúa y con
`SATURA` (rojo) cuando la suma pasa de ±1; el panel de efectos marca `SAT`
en la pista que se pasa de ±1 ella sola. Los avisos se quedan un segundo a
la vista; «Reiniciar medidores» (paleta) los quita. Si se nota apretado:

- baja el volumen de las pistas con `:gain()`, sobre todo cuando dos pistas
  tocan el mismo golpe a la vez (break + refuerzo de bombo);
- baja golpes concretos con `@` (`5@0.6`), o un corte en todos sus golpes
  con `cortes:fx(5, "gain", { level = 0.6 })`;
- recuerda que `@` y `:gain()` llegan hasta 4: más de 1 sube el volumen;
- el delay y la reverb (`:send()`, `bus()`) se suman al master antes del
  limitador: con mucho feedback y envíos altos, baja su `level` o los
  envíos. El panel de efectos enseña los envíos de cada pista (`→ d0.30
  r0.15`: delay y reverb).

### Errores comunes de Lua

Los errores salen con archivo y línea, en el editor y en la línea de
comandos. Mientras tanto sigue sonando lo anterior.

```text
song.lua:2: steps: el patrón necesita 16 pasos, tiene 4
```

El patrón tiene que tener exactamente 16 pasos (o los de `:length(n)`, que
va antes de `:steps()`). Rellena con `.`.

```text
song.lua:2: steps: el corte 9 no existe ("drums" tiene 8)
```

Has pedido un corte mayor que los que hay (o el detector encontró menos
golpes de los que pediste en `slice(n)`).

```text
song.lua:2: steps: paso 3 ("5^"): falta el número después de ^
```

Los sufijos `^`, `@` y `r` necesitan su número pegado: `5^7`, `5@0.5`, `5r4`.

```text
song.lua:4: steps: paso 3 ("snre"): la etiqueta "snre" no existe (hay: kick, snare, hat)
```

Una etiqueta mal escrita o que esa pista no tiene. Con `slice_at` la pista
solo conoce la etiqueta de su corte. Si la etiqueta está en el proyecto
pero para otro número de cortes, el mensaje acaba con `; la etiqueta «kick»
del proyecto es de slice(8) y este script usa slice(6)`: vuelve a
`slice(8)` o etiqueta de nuevo los cortes.

```text
song.lua:3: name(1, "x"): "x" ya es el corte 1 en la notación
```

Un nombre de etiqueta que choca con la notación: ni números, ni `x`, ni un
final en `r` y un número.

```text
song.lua:2: la pista "hats" no existe; defínela con track("hats", sample(...):slice(n))
```

`track("nombre")` con un solo argumento solo recupera pistas ya definidas.

```text
song.lua:2: argumento 2 no válido: userdata is not expected type
```

Has pasado a `track()` un `sample(...)` sin cortar: añade `:slice(n)`, o
`:slice_grid(1)` para el sample entero como un solo corte. (`:slice(1)` no
sirve para eso: su único corte empieza en el golpe más fuerte, que puede
no ser el primero, y lo de antes queda fuera.)

```text
song.lua:2: argumento 2 no válido: se esperaba f32 y llegó string
```

Un número entre comillas (`gain("0.5")`). En los métodos (`pista:gain(...)`)
el argumento 1 es la propia pista, así que el primero tuyo es el 2.

```text
song.lua:3: fx("filter"): parámetro desconocido "cutof" (admite: type, cutoff, q)
```

Errata en un parámetro de efecto.

```text
song.lua:1: .../mi_amen.wav: error leyendo WAV: No such file or directory (os error 2)
```

El WAV no está donde se busca: junto al script o en la carpeta `samples`
del proyecto (orden `muestras` para ver cuál es).

```text
song.lua:3: no se puede llamar a track() dentro de every()
```

Dentro de `every` solo se modifica el compás que recibe la función.

```text
pista "d": every/chance dan un ciclo de 77 compases (máximo 64)
```

`every(7)` y `every(11)` necesitan 77 compases para repetirse. Usa números
que se lleven bien (2, 4, 8, 16...).

```text
song.lua:1: ')' expected near <eof>
```

Error de sintaxis de Lua: falta cerrar un paréntesis, unas comillas o un
`end`.

## 8. Tu primera canción, de principio a fin

Un tutorial para quien nunca ha usado el programa. Montas una canción
entera, paso a paso: un break troceado, una segunda percusión, efectos,
una melodía, un bajo y una estructura con intro, drop y break. Al final
la guardas y la exportas a WAV.

Cada paso es una acción con un resultado que puedes ver u oír. Si algo no
pasa como se dice, cada paso explica qué mirar. Lo que ya está explicado
en otra sección de la guía no se repite aquí: hay un enlace.

1. [Preparar la carpeta](#paso-1-preparar-la-carpeta)
2. [Abrir un proyecto nuevo](#paso-2-abrir-un-proyecto-nuevo)
3. [Cargar un break y trocearlo](#paso-3-cargar-un-break-y-trocearlo)
4. [El primer patrón de batería](#paso-4-el-primer-patrón-de-batería)
5. [Otro sample en otra pista](#paso-5-otro-sample-en-otra-pista)
6. [Efectos en un solo corte](#paso-6-efectos-en-un-solo-corte)
7. [Efectos de pista y automatización](#paso-7-efectos-de-pista-y-automatización)
8. [Abrir un instrumento](#paso-8-abrir-un-instrumento)
9. [Líneas melódicas](#paso-9-líneas-melódicas)
10. [Efectos sobre la melodía](#paso-10-efectos-sobre-la-melodía)
11. [Un bajo que no choque con el bombo](#paso-11-un-bajo-que-no-choque-con-el-bombo)
12. [Variación y estructura](#paso-12-variación-y-estructura)
13. [Guardar, versiones y exportar](#paso-13-guardar-versiones-y-exportar)
- [El script final](#el-script-final)
- [Si algo no suena, mira esto](#si-algo-no-suena-mira-esto)
- [Para seguir](#para-seguir)

**Cómo crece el script.** El script de la canción se escribe entero una
vez, en el paso 3. Después, cada bloque de código dice qué hacer con él:

- **«Añade al final»**: copia el bloque al final del script, tras una
  línea en blanco.
- **«Cambia el párrafo»**: busca el párrafo que empieza igual que el
  bloque (desde esa línea hasta la siguiente línea en blanco) y sustitúyelo
  entero por el bloque.

Algunos pasos terminan con **el script entero**, para que compares el
tuyo. Los tests del programa siguen estos mismos pasos y comprueban que
cada estado del script se carga y suena, y que el script entero de cada
paso es exactamente lo que llevas.

**Antes de oír nada, baja el volumen.** Sin `--allow-audio` el programa
no suena nunca (todo se mueve, pero el audio se tira). Antes de usar
`--allow-audio`, baja el volumen del sistema, por ejemplo al 30 %:

```sh
wpctl set-volume @DEFAULT_AUDIO_SINK@ 0.3
```

Luego súbelo poco a poco. Un error en el script puede dejar un sonido
muy fuerte (un `@4`, un `:gain(4)`).

### Paso 1. Preparar la carpeta

**Una carpeta por canción.** Crea una y entra en ella:

```sh
mkdir -p ~/musica/primera
cd ~/musica/primera
```

El script de la canción (`cancion.lua`) y sus samples van aquí.

**Los samples.** Este tutorial usa cuatro archivos WAV. Ponlos en la
carpeta con estos nombres:

| Archivo | Qué es |
| --- | --- |
| `amen.wav` | Un break de batería (el amen o cualquier otro) |
| `perc.wav` | Otro loop de percusión, de un compás, a otro tempo |
| `nota.wav` | Una nota sostenida de piano, synth o voz (mejor de 2 s o más) |
| `bajo.wav` | Una nota grave (un bajo, un synth grave) |

¿No tienes samples, o quieres probar sin derechos de autor? El
repositorio trae un break sintético (ver «Generar el break sintético» en
la [sección 1](#1-inicio-rápido)). Cópialo dos veces y fabrica las dos
notas con `ffmpeg` (un tono puro: suena simple, pero vale para aprender):

```sh
cp ~/sampler/renders/synth_break.wav amen.wav
cp ~/sampler/renders/synth_break.wav perc.wav
ffmpeg -f lavfi -i "sine=frequency=261.63:duration=3" nota.wav
ffmpeg -f lavfi -i "sine=frequency=65.41:duration=3" bajo.wav
```

(`~/sampler` es la carpeta del repositorio; cambia la ruta si está en
otro sitio.) La primera nota es un do central (C4) y la segunda, un do
dos octavas más abajo (C2). El break sintético va a 140 BPM; tus samples
irán a otro tempo, y está bien: el paso 5 explica cómo igualarlos.

**Formato.** Solo se leen archivos **WAV**. Valen mono o estéreo (el mono
suena igual por los dos lados; de un archivo con más canales se usan los
dos primeros), en enteros de 16 o 24 bits o en coma flotante, y a
cualquier frecuencia: al cargarlos se convierten a la del motor. Un MP3,
un FLAC o un OGG se convierte antes con `ffmpeg`:

```sh
ffmpeg -i amen.mp3 amen.wav
```

Para convertir todos los MP3 de la carpeta:

```sh
for f in *.mp3; do ffmpeg -i "$f" "${f%.mp3}.wav"; done
```

**Rutas.** En el script, `sample("amen.wav")` busca el archivo **en la
carpeta del script** (con un proyecto que diga `samples = ...`, en esa
carpeta; ver «Proyectos» en la [sección 1](#1-inicio-rápido)):

- `sample("amen.wav")`: junto al script.
- `sample("breaks/amen.wav")`: en la subcarpeta `breaks` de la carpeta
  del script.
- `sample("/home/yo/samples/amen.wav")`: una ruta absoluta (empieza por
  `/`) vale tal cual, esté donde esté el script. `~` no se entiende aquí:
  escribe la ruta entera.

En la línea de comandos, en cambio, las rutas son relativas a la carpeta
en la que estás (la del `cd`). Por eso este tutorial abre siempre el
programa desde la carpeta de la canción.

**Si no pasa:** si al cargar sale un error como este, el archivo no está
donde se busca (mira el nombre, las mayúsculas y la extensión):

```text
cancion.lua:3: amen.wav: error leyendo WAV: No such file or directory (os error 2)
```

La ruta del mensaje es la que se ha buscado (relativa a la carpeta del
script, salvo que sea absoluta). Un archivo que no es WAV de
verdad (un MP3 renombrado a `.wav`) también da error al leerlo:
conviértelo con `ffmpeg`.

### Paso 2. Abrir un proyecto nuevo

**Compila el programa una vez**, en la carpeta del repositorio, y crea un
atajo para llamarlo desde cualquier sitio:

```sh
cd ~/sampler
cargo build --release -p tui
alias tui=~/sampler/target/release/tui
```

(Añade la línea del `alias` a tu `~/.bashrc` para no repetirla.) Vuelve a
la carpeta de la canción: `cd ~/musica/primera`.

**Abre la canción con el editor dentro**, en una sola ventana:

```sh
tui --edit cancion.lua --new
```

`cancion.lua` todavía no existe: `--new` le dice al programa que lo
quieres empezar, y el editor abre una **plantilla** (un break de ejemplo
con `amen.wav`). El archivo no se crea hasta que guardas con Ctrl-S. Con
el archivo ya guardado, `--new` no cambia nada: lo abre tal cual. A la izquierda está el editor; a la derecha, el
estado del motor, la forma de onda del sample y el tracker; abajo, los
efectos de cada pista (ver «Abrir la interfaz» en la
[sección 1](#1-inicio-rápido)). Ctrl-Q sale.

Así no suena nada: el tracker se mueve, pero el audio se tira. Para oírlo
(¡con el volumen bajo!), sal y vuelve a abrir con `--allow-audio`:

```sh
tui --edit cancion.lua --new --allow-audio
```

**Si no pasa:** sin `--new`, si `cancion.lua` no existe, el programa no
se abre y dice `el script no existe (buscado en ...)`: mira la carpeta en
la que estás y el nombre, o añade `--new` si lo quieres empezar. Si el borde del editor se pone rojo y el pie dice
`error leyendo WAV`, la plantilla no encuentra `amen.wav`: vuelve al
paso 1. Si `tui` no se encuentra, el `alias` no está puesto en esta
terminal.

**El editor en su propia ventana (opcional).** Con dos ventanas, una
tiene el editor y la otra las vistas grandes (la forma de onda, el
tracker con todas las pistas, el mezclador) y el modo tocar. Hay tres
maneras:

1. **Que el programa abra la ventana del editor.** Dile qué terminal usar
   en `~/.config/sampler/config.toml`:

   ```toml
   [editor]
   terminal = "kitty --class sampler-editor -e"
   ```

   (con foot: `terminal = "foot --app-id sampler-editor"`). Luego:

   ```sh
   tui --edit cancion.lua --new --split --allow-audio
   ```

2. **Abrir el editor a mano**, sin esa línea o por SSH (en otro panel de
   `tmux`). En una terminal, la principal con `--split`; en la otra, desde
   la misma carpeta:

   ```sh
   tui --window-editor cancion.lua
   ```

   El editor busca la ventana principal de ese mismo script y se conecta.
   Mientras no la encuentra, lo dice y lo vuelve a intentar cada segundo.
   Con un archivo que todavía no existe (la principal abierta con
   `--new`), las dos ventanas parten de la misma plantilla.

3. **Sacarlo y meterlo en caliente**, sin que deje de sonar. F1 abre la
   paleta de comandos; escribe un trozo del nombre de la acción y Enter.
   F1 «Abrir el editor en su ventana» saca el editor a una terminal nueva
   (sin `[editor] terminal`, dice el comando para abrirlo a mano). F1
   «Volver a meter el editor en la principal» lo devuelve. El texto, lo
   que no has guardado y el deshacer van con él.

Qué hace cada ventana: en la del **editor** escribes, guardas (Ctrl-S) y
usas la paleta; en la **principal** suena todo, se ven las vistas y se
toca (está siempre en el modo tocar). Los detalles, en «El editor en su
propia ventana» ([sección 1](#1-inicio-rápido)). Para empezar, una sola
ventana basta.

### Paso 3. Cargar un break y trocearlo

**Borra la plantilla.** Ctrl-Inicio va al principio; Shift-Ctrl-Fin
selecciona hasta el final; Ctrl-X lo corta. (O escribe el script con tu
editor de siempre antes de abrir el programa.)

**Escribe el script entero** (o pégalo con Ctrl-Shift-V):

```lua entero
bpm(170)

local amen = sample("amen.wav"):slice(8)

track("drums", amen)
  :steps("1 . 2 .  3 . 4 .  5 . 6 .  7 . 8 .")
```

Qué dice cada línea:

- `bpm(170)`: el tempo de la canción, 170 pulsos por minuto.
- `sample("amen.wav"):slice(8)`: carga el break y lo corta en sus 8
  golpes más fuertes (los **cortes**). `local amen` les da un nombre.
- `track("drums", amen)`: una **pista** que se llama `drums` y toca esos
  cortes.
- `:steps(...)`: lo que toca. Cada número es un corte y cada `.` un
  silencio: aquí, los 8 cortes en orden, uno cada dos pasos.

**Guarda y carga con Ctrl-S.** El pie del editor dice `cargado: entra en
el siguiente compás` y el secuenciador arranca solo. Oyes los cortes uno
tras otro, y en la vista del sample la forma de onda tiene los cortes
numerados (el que suena, resaltado).

**Si no pasa:**

- No oyes nada pero el tracker se mueve: falta `--allow-audio`, o el
  volumen del sistema está a cero.
- Ni el tracker se mueve: Ctrl-T arranca y para el secuenciador.
- Borde rojo: el pie dice el error y Ctrl-G lleva a su línea. Mientras
  tanto sigue sonando lo anterior.
- Al pie sale `el corte 8 no existe ("drums" tiene 6)`: el detector ha
  encontrado menos golpes. Quita del patrón los números que sobran, o pide
  menos cortes.

**El tempo del break.** Tu break tiene su propio tempo. Desde la
carpeta de la canción:

```sh
tui amen.wav --detect-bpm
```

Con el break sintético dice:

```text
amen.wav: 140.00 BPM, 1 compás (encaje 100 %) → bpm(140)
```

Lo mismo desde la interfaz: F1 «Detectar el BPM del sample» deja escrita
la orden `bpm 140`; Enter cambia la línea `bpm()` del script y Esc la
deja como está. Aquí la dejamos en 170: cada corte se dispara en la
rejilla de la canción, así que el break troceado sigue el tempo que
elijas. Ver «Saber el tempo del break» en la [sección 1](#1-inicio-rápido).

**Oír cada corte.** Pulsa Esc: entras en el **modo tocar** (la línea de
ayuda cambia). Las teclas `1` a `8` tocan los cortes 1 a 8 de la pista
seleccionada, al instante. ← y → recorren los cortes y los preescuchan;
espacio vuelve a sonar el seleccionado. Apunta cuál es el bombo y cuál la
caja. Esc vuelve al editor. Todas las teclas están en «Modo tocar (Esc)»
([sección 2](#2-atajos-de-teclado)).

En el break sintético, el 1 es el bombo y el 3 la caja. En tu break
pueden ser otros números: usa los tuyos en lo que sigue.

**Ponles nombre.** Cambia el párrafo de `local amen` por este (con tus
números):

```lua cambia
local amen = sample("amen.wav"):slice(8)
amen:name(1, "kick"):name(3, "snare")
```

Ctrl-S. Ahora la vista del sample pone `kick` y `snare` junto a sus
números, y en los patrones puedes escribir `kick` en vez de `1`. Los
nombres llevan letras sin tilde, números y `_`.

**Ajustar un corte.** Si un corte empieza tarde (se come el ataque del
golpe) o pronto (suena un trozo del anterior), en el modo tocar
selecciónalo con ← → y muévelo: `,` lo adelanta 5 ms y `.` lo retrasa;
`<` y `>`, de 1 en 1 ms. Cada vez lo preescucha. La vista marca los
cortes ajustados con `*`. Los ajustes se guardan en el **proyecto**:
pulsa F2, escribe `guardar` y Enter. Se crea `cancion.sampler` junto al
script (el script, la carpeta de samples, el tempo y tus ajustes). La
próxima vez abre el proyecto en vez del script:

```sh
tui --edit cancion.sampler --allow-audio
```

F1 «Quitar el ajuste del corte» devuelve el corte seleccionado a donde
lo puso el detector.

**Otra forma de cortar.** `slice(8)` busca los golpes. `slice_grid(16)`
corta en 16 partes iguales, sin mirar los golpes: va bien con un break
bien cuadrado de un compás. Para probarlo, en un script aparte:

```lua
bpm(170)
local amen = sample("amen.wav"):slice_grid(16)
track("drums", amen):steps("1 2 3 4  5 6 7 8  9 10 11 12  13 14 15 16")
```

Si los cortes de `slice` no te convencen, mira «Cortes mal detectados» en
la [sección 7](#7-solución-de-problemas).

### Paso 4. El primer patrón de batería

Un patrón son 16 pasos: un compás de semicorcheas. Los espacios de más
no cuentan, así que se agrupan de cuatro en cuatro (un tiempo cada
grupo).

**Un ritmo con bombo y caja.** Cambia el párrafo de `track("drums",
amen)` por este:

```lua cambia
track("drums", amen)
  :steps("kick . 2 .  snare . 4 .  kick kick 2 .  snare . 8 .")
```

Ctrl-S: entra al empezar el compás siguiente. Bombo en el 1, caja en el
2 y el 4, como un break de verdad. (En el break sintético, 2, 4 y 8 son
charles.)

**Mira qué suena.** Con la canción en marcha, el paso que suena se
resalta **en el propio código**, dentro de `steps("...")`, con el color
de la pista. El tracker de la derecha lo enseña también, fila a fila. El
resaltado se para mientras escribes y vuelve cuando cargas (solo resalta
el texto que está sonando). F1 «Resaltado en vivo» lo apaga y lo
enciende; ver «Resaltado en vivo» en la [sección 2](#2-atajos-de-teclado).

**Si no pasa:** si un paso sale subrayado en rojo y el pie dice
`⚠ línea 7: ...`, hay un error en el patrón antes de guardar. El más
típico: `el patrón necesita 16 pasos, tiene 15`. Cuenta los grupos: cuatro
de cuatro. Si una etiqueta no existe (`snre`), el aviso dice cuáles hay.

**Dale vida con sufijos.** Cada paso admite sufijos pegados:

| Escribe | Qué hace | Ejemplo |
| --- | --- | --- |
| `@` y un número | Volumen del golpe (1 normal, 0.5 la mitad) | `2@0.4` |
| `^` y un número | Pitch en semitonos (negativo, más grave y más largo) | `kick^-2` |
| `'` | El golpe al revés | `snare'` |
| `r` y un número | Retrigger: el golpe se repite N veces en su paso | `snare'r2` |
| `.` | Silencio | `.` |

Cambia otra vez el párrafo de `track("drums", amen)`:

```lua cambia
track("drums", amen)
  :steps("kick . 2 2@0.4  snare . 4 .  kick kick^-2 2 .  snare . snare'r2 8@0.5")
```

Ctrl-S. Oyes un charles suave antes de la primera caja, un bombo más
grave, y al final una caja al revés repetida dos veces que «aspira» hacia
el compás siguiente. Todos los símbolos (también `%`, `_`, `~` y las
llaves) están en la [sección 3](#3-mini-notación).

**Si no pasa:** `falta el número después de ^`: los sufijos llevan el
número pegado (`kick^-2`, no `kick^ -2`). Si suena demasiado fuerte, mira
que ningún `@` pase de 1.

**El script entero**, hasta aquí:

```lua entero
bpm(170)

local amen = sample("amen.wav"):slice(8)
amen:name(1, "kick"):name(3, "snare")

track("drums", amen)
  :steps("kick . 2 2@0.4  snare . 4 .  kick kick^-2 2 .  snare . snare'r2 8@0.5")
```

### Paso 5. Otro sample en otra pista

**Una pista nueva con `perc.wav`.** Añade al final:

```lua añade
local perc = sample("perc.wav"):stretch(120):slice_grid(1)

track("perc", perc)
  :steps("1 . . .  . . . .  . . . .  . . . .")
  :gain(0.4)
```

Ctrl-S. Ahora suenan dos pistas: el tracker tiene dos columnas y el panel
de abajo, dos pistas. Ctrl-O cambia la pista seleccionada (la de la vista
del sample y la del modo tocar).

Qué hace cada parte:

- `slice_grid(1)`: el loop entero como un solo corte. (`slice(1)` no
  vale para esto: su corte empieza en el golpe más fuerte, que puede no
  ser el primero.)
- `steps("1 . . . ...")`: lo dispara al principio de cada compás.
- `stretch(120)`: el loop va a 120 BPM y la canción a 170. Sin estirarlo,
  un compás del loop dura más que uno de la canción y se corta antes de
  acabar. `stretch(120)` lo lleva de 120 al tempo de la canción **sin
  cambiar su tono**: cada golpe cae en su sitio. La vista del sample lo
  dice (`120→170 BPM`). Sin el número, `stretch()` usa el tempo que
  detecta (el de `--detect-bpm`, paso 3).
- `gain(0.4)`: el volumen de la pista (1 lo deja igual).

Pon el tempo de **tu** loop en `stretch(...)`. Va bien entre ×0,7 y
×1,4 (de 120 a 170 es casi el límite). Ver `sample:stretch` en la
[sección 4](#4-api-de-lua).

**Si no pasa:**

- `la pista "perc" no existe; defínela con track("perc", sample(...):slice(n))`:
  has escrito `track("perc")` sin los cortes. Con un solo argumento,
  `track` solo **recupera** una pista ya definida; para crearla hacen
  falta los dos: `track("perc", perc)`.
- `argumento 2 no válido`: has pasado a `track()` el sample sin cortar.
  Añade `:slice(n)` o `:slice_grid(n)`.
- Suena demasiado fuerte al sumar las dos pistas: baja los `:gain()`. El
  panel del motor avisa con `LIMITADOR` o `SATURA` (ver «Saturación» en
  la [sección 7](#7-solución-de-problemas)).

**Elegir samples sin salir.** F1 «Explorador de samples» lista los WAV
de la carpeta de samples (la del script, o la `samples` del proyecto) y
de sus subcarpetas. Escribe para filtrar por
nombre; ↑ ↓ eligen y preescuchan cada uno (sin tocar las pistas); Tab lo
vuelve a sonar; Enter escribe `sample("ruta")` donde está el cursor; Esc
cierra. Con el editor en su ventana, el explorador se abre en la
principal.

### Paso 6. Efectos en un solo corte

Un efecto puede sonar solo en un corte: ensuciar solo la caja, quitar
solo el clic del bombo. Hay dos maneras.

**En todos los golpes de un corte** (`cortes:fx`). Cambia el párrafo de
`local amen` por este:

```lua cambia
local amen = sample("amen.wav"):slice(8)
amen:name(1, "kick"):name(3, "snare")
amen:fx("snare", "bitcrush", { bits = 6, downsample = 2 })
amen:fx("kick", "lowpass", { cutoff = 2500 })
```

Ctrl-S. Cada caja suena sucia (`bitcrush`: menos bits, más grano) y cada
bombo, más redondo (`lowpass`: quita los agudos por encima de 2500 Hz). El
resto de golpes suena igual. El panel de efectos enseña cada corte con
efectos tras un `◇` (`◇ 3 snare › crush 6b ÷2`; si no caben, una línea:
`◇ efectos en 1 kick, 3 snare`). El primer argumento es el corte: su
etiqueta, su número o una lista (`{ "kick", 5 }`). Los efectos son los de
la pista (tabla en `pista:fx`, [sección 4](#4-api-de-lua)), hasta 4 por
corte.

**En un solo golpe** (llaves en el patrón). Cambia el párrafo de
`track("drums", amen)`:

```lua cambia
track("drums", amen)
  :steps("kick . 2 2@0.4  snare . 4{hp=4000} .  kick kick^-2 2 .  snare . snare'r2 2r4{lp=300>9000}")
```

Ctrl-S. Dos cosas nuevas:

- `4{hp=4000}`: ese golpe pasa por un filtro paso alto a 4000 Hz (solo
  quedan los agudos). Los demás golpes del corte 4, no.
- `2r4{lp=300>9000}`: el último paso se repite 4 veces (`r4`) con un
  filtro paso bajo que **se abre durante el paso**, de 300 a 9000 Hz. Es
  una rampa dentro del golpe: `{efecto=desde>hasta}`. Con `:4` detrás
  (`{lp=300>9000:4}`), la rampa dura 4 pasos.

Dentro de las llaves va el efecto (`lp`, `hp`, `crush`, `drive`...), `=`
y su valor principal; tras comas, los demás (`{lp=800,q=4}`). Varios
efectos, cada uno en sus llaves: `3{crush=6}{lp=900}`. Todo, en
«Efectos por paso» de la [sección 3](#3-mini-notación).

**Si no pasa:** un paso con un efecto mal escrito sale subrayado en rojo
antes de guardar. Entre el corte y el paso caben 4 efectos: si no, la
carga da error con la línea. Las llaves no valen en una pista de
instrumento (pasos 8-11): allí, efectos de pista.

### Paso 7. Efectos de pista y automatización

Los efectos de pista suenan sobre todo lo que toca la pista, después de
los efectos de cada corte. Van en orden, hasta 4 por pista.

**Los buses de delay y de reverb.** Son dos, compartidos por todas las
pistas: cada pista les **envía** una parte de su sonido con `:send()`.
Cambia el párrafo de `bpm(170)` por este:

```lua cambia
bpm(170)
bus("delay", { time = "3/16", feedback = 0.4, level = 0.6 })
bus("reverb", { size = 0.6, damp = 0.5 })
```

(Así solo cambia cómo suenan los buses; nada se oye distinto hasta que
una pista les envía algo.)

**La batería: limpia, comprimida y con algo de sala.** Cambia el párrafo
de `track("drums", amen)`:

```lua cambia
track("drums", amen)
  :steps("kick . 2 2@0.4  snare . 4{hp=4000} .  kick kick^-2 2 .  snare . snare'r2 2r4{lp=300>9000}")
  :fx("highpass", { cutoff = 40 })
  :fx("compressor", { threshold = -14, ratio = 3, attack = 10, makeup = 2 })
  :send("reverb", 0.1)
  :gain(0.6)
```

- `highpass` a 40 Hz quita los graves que no se oyen pero ocupan sitio.
- `compressor` iguala los golpes: por encima de -14 dB baja el volumen
  (`ratio = 3`); `attack = 10` deja pasar el ataque y `makeup = 2` sube
  2 dB al final.
- `send("reverb", 0.1)`: un 10 % de la batería va a la reverb.

**La percusión: un filtro que se mueve.** Cambia el párrafo de
`track("perc", perc)`:

```lua cambia
track("perc", perc)
  :steps("1 . . .  . . . .  . . . .  . . . .")
  :fx("lowpass", { cutoff = ramp(1500, 9000, "4 compases"), q = 2 })
  :fx("bitcrush", { bits = 8, downsample = 2 })
  :pan(lfo("2 compases", -0.5, 0.5))
  :send("delay", 0.25)
  :gain(0.4)
```

- `ramp(1500, 9000, "4 compases")`: en lugar de un número fijo, el corte
  del filtro sube de 1500 a 9000 Hz en 4 compases y vuelve a empezar.
- `lfo("2 compases", -0.5, 0.5)`: el pan va y viene de izquierda a
  derecha en un ciclo de 2 compases.
- `send("delay", 0.25)`: un eco en semicorchea con puntillo (`3/16`).

El panel de efectos marca lo que se mueve (`∿cutoff`) y los envíos
(`→ d0.25`). Qué parámetros admiten `ramp()` y `lfo()`, en «Qué se puede
automatizar» ([sección 4](#4-api-de-lua)). Más efectos (distorsión, EQ,
volumen) en la tabla de `pista:fx`.

**Mute y solo.** Para escuchar una pista sola o quitar otra mientras
pruebas: en el modo tocar (Esc), Ctrl-O elige la pista, `m` la silencia y
`s` la pone en solo, al instante. Otra vez, lo quita. F1 «Quitar mute y
solo» los quita todos. En el script, `:mute()` y `:solo()` hacen lo mismo
al empezar el compás (el paso 12 los usa).

**Si no pasa:**

- `fx("filter"): parámetro desconocido "cutof"`: una errata en un
  parámetro. El autocompletado (F4) y la ayuda del pie del editor dicen
  cuáles hay.
- La reverb o el delay no suenan: el `level` del bus o el envío están a
  0, o la pista está en mute (los envíos van después del mute).
- `fx: máximo 4 efectos por pista`: una pista admite 4 efectos; quita
  uno.

**El script entero**, hasta aquí:

```lua entero
bpm(170)
bus("delay", { time = "3/16", feedback = 0.4, level = 0.6 })
bus("reverb", { size = 0.6, damp = 0.5 })

local amen = sample("amen.wav"):slice(8)
amen:name(1, "kick"):name(3, "snare")
amen:fx("snare", "bitcrush", { bits = 6, downsample = 2 })
amen:fx("kick", "lowpass", { cutoff = 2500 })

track("drums", amen)
  :steps("kick . 2 2@0.4  snare . 4{hp=4000} .  kick kick^-2 2 .  snare . snare'r2 2r4{lp=300>9000}")
  :fx("highpass", { cutoff = 40 })
  :fx("compressor", { threshold = -14, ratio = 3, attack = 10, makeup = 2 })
  :send("reverb", 0.1)
  :gain(0.6)

local perc = sample("perc.wav"):stretch(120):slice_grid(1)

track("perc", perc)
  :steps("1 . . .  . . . .  . . . .  . . . .")
  :fx("lowpass", { cutoff = ramp(1500, 9000, "4 compases"), q = 2 })
  :fx("bitcrush", { bits = 8, downsample = 2 })
  :pan(lfo("2 compases", -0.5, 0.5))
  :send("delay", 0.25)
  :gain(0.4)
```

### Paso 8. Abrir un instrumento

Hasta aquí, cada golpe era un corte. Un **instrumento** toca un sample
como notas: cada nota suena a su altura, varias a la vez, con una
envolvente (cómo entra y cómo se apaga).

**Carga `nota.wav` como instrumento.** Añade al final:

```lua añade
local teclas = sample("nota.wav"):instrument({
  root = "C4", voices = 6, adsr = { 10, 300, 0.6, 400 }, loop = { 300, 1200 },
})

track("teclas", teclas)
  :melody("c _ _ _  . . . .  . . . .  . . . .")
  :gain(0.5)
```

Ctrl-S. Al principio de cada compás suena una nota: un do que dura 4
pasos y luego se apaga despacio. El panel de efectos dice `· ♪ C4 1/6`
(la raíz y cuántas voces suenan de las 6).

Qué es cada opción:

- `root = "C4"`: la nota en la que suena el sample tal cual (C4 es el do
  central). Si tu sample es un la, pon `"A3"` o la que sea: así las notas
  de `melody` suenan afinadas.
- `voices = 6`: cuántas notas pueden sonar a la vez. Con todas ocupadas,
  una nota nueva quita la más antigua.
- `adsr = { 10, 300, 0.6, 400 }`: la envolvente. Ataque de 10 ms (lo que
  tarda en entrar), decay de 300 ms (lo que tarda en bajar) hasta el
  sustain, 0.6 (el nivel mientras dura la nota), y release de 400 ms (lo
  que tarda en apagarse al acabar).
- `loop = { 300, 1200 }`: mientras dura la nota, el trozo entre los 300 y
  los 1200 ms se repite en bucle, con un fundido para que no haga clic.
  Así una nota puede durar más que el sample. Elige un trozo en el que la
  nota ya suena estable (tras el ataque).

**Si no pasa:**

- `loop = {300, 1200}: tiene que ir dentro del sample (0 a 900.0 ms)`: tu
  nota es más corta. Pon un bucle dentro de lo que dura, o quita `loop`
  (la nota acaba con el sample).
- Suena desafinada respecto a la batería o a otro instrumento: revisa
  `root`. Si no sabes qué nota es tu sample, compáralo de oído con las
  teclas del modo tocar (paso 9).
- Suena fuerte: un instrumento suma sus voces; baja el `:gain()`.

Todas las opciones (`keep`, `formant`, `crossfade`) están en
`sample:instrument`, [sección 4](#4-api-de-lua).

### Paso 9. Líneas melódicas

**Una melodía.** `melody` escribe notas por su nombre: `c d e f g a b`,
con `#` o `b` (`eb` es mi bemol), y la octava detrás si hace falta
(`c5`; sin ella, la de `root`). `_` alarga la nota anterior un paso y `.`
es silencio. Cambia el párrafo de `track("teclas", teclas)`:

```lua cambia
track("teclas", teclas)
  :melody("c . eb .  g . eb .  f _ _ .  g _ bb _")
  :gain(0.5)
```

Ctrl-S. Una frase en do menor. El tracker enseña las notas con sostenidos (el mi
bemol sale `D#4`) y `│` en las filas que alargan una nota. En el código se resalta la nota
que suena, como en el paso 4.

**Acordes y notas largas.** Con `+`, varias notas a la vez (las que no
llevan octava van justo encima de la anterior). Una pista de acordes con
el mismo instrumento; añade al final:

```lua añade
track("acordes", teclas)
  :melody("c+eb+g _ _ _  _ _ _ _  ab3+c+eb _ _ _  bb3+d+f _ _ _")
  :gain(0.3)
```

Ctrl-S. Tres acordes largos debajo de la melodía: do menor (8 pasos), la
bemol y si bemol. Un acorde lleva hasta 4 notas.

**Con una escala.** Si prefieres pensar en grados (1 la tónica, 3 la
tercera, 5 la quinta...), `scale()` los convierte en semitonos sobre la
raíz. Para probarlo, en un script aparte:

```lua
bpm(170)
local teclas = sample("nota.wav"):instrument({ root = "C4" })
local menor = scale("minor")
track("teclas", teclas):steps(menor:melody("1 . 3 .  5 . 3 .  4 . . .  5 . 7 .", "1"))
```

`menor:melody(...)` devuelve `"1^0 . 1^3 ..."`: en un instrumento, cada
`^` son semitonos sobre la raíz. Las escalas que hay, en `scale`
([sección 4](#4-api-de-lua)).

**Tocar en vivo.** Esc (modo tocar) y Ctrl-O hasta que la pista
seleccionada sea `teclas`. Cada tecla es una nota: `1` la raíz (C4) y
cada tecla siguiente, un semitono más. En do menor: `1` do, `4` mi
bemol, `6` fa, `8` sol, `9` la bemol, `q` si bemol, `e` el do de arriba.
La nota dura mientras mantienes la tecla, pero eso depende de la
terminal: en kitty, foot, WezTerm o Ghostty, hasta que la sueltas; en
tmux y la mayoría, mientras llegan las repeticiones de la tecla. Ver
«Modo tocar (Esc)», [sección 2](#2-atajos-de-teclado).

**Con un teclado MIDI.** Abre con `--allow-midi` y conecta el teclado al
puerto «sampler entrada»:

```sh
tui --edit cancion.sampler --allow-audio --allow-midi
aconnect -l                         # en otra terminal: lista los puertos
aconnect "Mi teclado" "sampler"     # con el nombre que salga en la lista
```

Cada nota suena en la pista seleccionada a su altura real (el C4 del
teclado es la raíz `C4`), con la velocidad como volumen. Ver «MIDI» en la
[sección 2](#2-atajos-de-teclado).

**Grabar lo tocado.** Para que lo que tocas quede escrito:

1. En el editor, escribe al final una línea nueva con
   `track("teclas"):` y deja el cursor detrás de los dos puntos.
2. Con `teclas` seleccionada, F1 «Grabar lo tocado». La línea de órdenes
   dice `grabar 1`: cambia el número por los compases que quieras y
   Enter.
3. La grabación empieza al principio del compás siguiente (el panel del
   motor dice `● REC teclas: al compás` y luego `● REC teclas 1/1`).
   Toca en el modo tocar o con el MIDI.
4. Al acabar se escribe en el cursor lo tocado, por ejemplo
   `steps("1^0 _ . 1^3  . . 1^7 _  ...")`: cada nota con su `^` (semitonos
   sobre la raíz) y sus `_`.
5. Ctrl-S lo carga: ese `steps` sustituye a la melodía de antes. Ctrl-Z
   lo quita de una vez.

Para seguir el tutorial con el mismo script que aquí, borra después esa
línea (o guárdala en otra pista).

**Si no pasa:** un aviso al pie como `⚠ línea N: ...` señala la nota que
no se entiende (una `h`, un acorde de más de 4 notas). Si las teclas del
modo tocar disparan cortes en vez de notas, la pista seleccionada no es
la del instrumento: Ctrl-O.

### Paso 10. Efectos sobre la melodía

Los efectos de un instrumento son los de su pista: las llaves por paso
no valen aquí (el paso 6 era para pistas de cortes). Cambia el párrafo
de `track("teclas", teclas)`:

```lua cambia
track("teclas", teclas)
  :melody("c . eb .  g . eb .  f _ _ .  g _ bb _")
  :fx("lowpass", { cutoff = lfo("1 compás", 900, 5000), q = 3 })
  :fx("distortion", { drive = 3, mix = 0.3 })
  :pan(-0.3)
  :send("delay", 0.3)
  :send("reverb", 0.25)
  :gain(0.5)
```

- `lowpass` con `lfo`: el filtro se abre y se cierra una vez por compás
  (`q = 3` le da un pico que se oye al moverse).
- `distortion` con `mix = 0.3`: un 30 % de la señal saturada, para darle
  cuerpo.
- `pan(-0.3)`: un poco a la izquierda.
- `send` al delay y a la reverb.

Y los acordes, más atrás, con más reverb y sin graves (dejan sitio al
bajo del paso 11). Cambia el párrafo de `track("acordes", teclas)`:

```lua cambia
track("acordes", teclas)
  :melody("c+eb+g _ _ _  _ _ _ _  ab3+c+eb _ _ _  bb3+d+f _ _ _")
  :fx("highpass", { cutoff = 200 })
  :pan(0.3)
  :send("reverb", 0.4)
  :gain(0.3)
```

Ctrl-S. Para cambiar el filtro **en una nota concreta**, `~` y la
frecuencia detrás de la nota (`eb~800`): mueve el primer filtro de la
pista desde esa nota hasta la siguiente. Ver «`~` (corte por paso)» en la
[sección 3](#3-mini-notación).

**Si no pasa:** `melody: paso 1 ("c{lp=800}"): las llaves ({...}) solo
valen en steps("...") de una pista de cortes, no en melody() ni en un
instrumento: usa los efectos de la pista (:fx)`. En `melody` y en los
instrumentos no hay llaves: usa `:fx()`. Si la
melodía tapa la batería, baja su `:gain()` o sus envíos.

### Paso 11. Un bajo que no choque con el bombo

**El bajo.** Otro instrumento, grave y de una sola voz (una nota nueva
corta la anterior, como un bajo de verdad). Añade al final:

```lua añade
local grave = sample("bajo.wav"):instrument({
  root = "C2", voices = 1, adsr = { 5, 200, 0.8, 150 },
})

track("bajo", grave)
  :melody("c2 _ _ _  . . c2 .  eb2 _ _ .  f2 _ g2 _")
  :fx("lowpass", { cutoff = 500 })
  :gain(0.7)
```

Ctrl-S. Notas largas (`_`) en las mismas notas que los acordes. El
`lowpass` a 500 Hz deja solo lo grave. Con `bajo.wav` a otra altura, pon
su nota en `root` (paso 8).

**Dónde chocan.** El bajo y el bombo suenan en la misma zona (60-100 Hz)
y se tapan. El programa lo mide sin escucharlo: F1 «Choques de
frecuencia». Renderiza aparte cada pista en solo (no toca lo que suena)
y abre un informe: un choque por fila (la pareja de pistas, la banda y
una puntuación de 0 a 1), y la tabla pareja × banda. Lo mismo desde la
línea de comandos:

```sh
tui --script cancion.lua --analyze-masking
```

**Quién manda.** En cada choque, una pista manda y la otra cede. Dile que
manda la batería; añade al final:

```lua añade
priority({ "drums", "bajo" })
```

Esto no cambia el sonido: solo dice al análisis a quién sugerir el
recorte.

**Recortar con un EQ.** En el informe, Enter sobre un choque propone un
EQ para la pista que cede, y Enter otra vez lo escribe en el script,
detrás de la pista (sin guardar: Ctrl-S lo carga, Ctrl-Z lo quita). Algo
como esto, con los números de tus samples:

```text
track("bajo"):fx("eq", { freq1 = 63, gain1 = -3.5, q1 = 1.4 })
```

Un EQ fijo quita esa zona siempre. Vuelve a analizar para ver cuánto ha
bajado el choque.

**O apartarse solo cuando pega el bombo.** Un EQ dinámico con
*sidechain* baja los graves del bajo solo mientras suena fuerte la
batería, y los devuelve en cuanto pasa. Cambia el párrafo de
`track("bajo", grave)`:

```lua cambia
track("bajo", grave)
  :melody("c2 _ _ _  . . c2 .  eb2 _ _ .  f2 _ g2 _")
  :fx("lowpass", { cutoff = 500 })
  :fx("dyneq", { freq = 80, range = 6, threshold = -30, attack = 2, release = 80, sidechain = "drums" })
  :gain(0.7)
```

`freq = 80` es la banda que escucha y que baja; `range = 6`, lo más que
baja (6 dB); `sidechain = "drums"`, que escucha la batería en vez del
propio bajo. Más, en «Que el break deje sitio al bombo» de la
[sección 6](#6-recetas-para-amen-breaks) y en la tabla de `pista:fx`.

**Si no pasa:** el informe no dice nada de `bajo` y `drums`: con tus
samples no chocan (o poco), y no hace falta recortar. (Con los samples de
prueba del paso 1, el choque que sale es `drums × perc`: las dos pistas
son el mismo break sintético, y la sugerencia va a `perc`.) Si el EQ sugerido
no cabe, la pista ya tiene 4 efectos: quita uno. Si `sidechain` da
error, mira que el nombre sea el de una pista que existe.

**El script entero**, hasta aquí:

```lua entero
bpm(170)
bus("delay", { time = "3/16", feedback = 0.4, level = 0.6 })
bus("reverb", { size = 0.6, damp = 0.5 })

local amen = sample("amen.wav"):slice(8)
amen:name(1, "kick"):name(3, "snare")
amen:fx("snare", "bitcrush", { bits = 6, downsample = 2 })
amen:fx("kick", "lowpass", { cutoff = 2500 })

track("drums", amen)
  :steps("kick . 2 2@0.4  snare . 4{hp=4000} .  kick kick^-2 2 .  snare . snare'r2 2r4{lp=300>9000}")
  :fx("highpass", { cutoff = 40 })
  :fx("compressor", { threshold = -14, ratio = 3, attack = 10, makeup = 2 })
  :send("reverb", 0.1)
  :gain(0.6)

local perc = sample("perc.wav"):stretch(120):slice_grid(1)

track("perc", perc)
  :steps("1 . . .  . . . .  . . . .  . . . .")
  :fx("lowpass", { cutoff = ramp(1500, 9000, "4 compases"), q = 2 })
  :fx("bitcrush", { bits = 8, downsample = 2 })
  :pan(lfo("2 compases", -0.5, 0.5))
  :send("delay", 0.25)
  :gain(0.4)

local teclas = sample("nota.wav"):instrument({
  root = "C4", voices = 6, adsr = { 10, 300, 0.6, 400 }, loop = { 300, 1200 },
})

track("teclas", teclas)
  :melody("c . eb .  g . eb .  f _ _ .  g _ bb _")
  :fx("lowpass", { cutoff = lfo("1 compás", 900, 5000), q = 3 })
  :fx("distortion", { drive = 3, mix = 0.3 })
  :pan(-0.3)
  :send("delay", 0.3)
  :send("reverb", 0.25)
  :gain(0.5)

track("acordes", teclas)
  :melody("c+eb+g _ _ _  _ _ _ _  ab3+c+eb _ _ _  bb3+d+f _ _ _")
  :fx("highpass", { cutoff = 200 })
  :pan(0.3)
  :send("reverb", 0.4)
  :gain(0.3)

local grave = sample("bajo.wav"):instrument({
  root = "C2", voices = 1, adsr = { 5, 200, 0.8, 150 },
})

track("bajo", grave)
  :melody("c2 _ _ _  . . c2 .  eb2 _ _ .  f2 _ g2 _")
  :fx("lowpass", { cutoff = 500 })
  :fx("dyneq", { freq = 80, range = 6, threshold = -30, attack = 2, release = 80, sidechain = "drums" })
  :gain(0.7)

priority({ "drums", "bajo" })
```

### Paso 12. Variación y estructura

Un loop que se repite igual cansa. Primero, pequeñas variaciones dentro
del loop; luego, partes distintas: intro, drop y break.

**Rellenos y azar en la batería.** Cambia el párrafo de `track("drums",
amen)`:

```lua cambia
track("drums", amen)
  :steps("kick . 2 2@0.4  snare . 4{hp=4000} .  kick kick^-2 2 .  snare . snare'r2 2r4{lp=300>9000}")
  :fill("roll", 4)
  :chance(0.1, "retrig", 3)
  :fx("highpass", { cutoff = 40 })
  :fx("compressor", { threshold = -14, ratio = 3, attack = 10, makeup = 2 })
  :send("reverb", 0.1)
  :gain(0.6)
```

- `fill("roll", 4)`: el último tiempo de cada 4 compases se rompe en
  repeticiones cada vez más rápidas (otros rellenos: `stutter`,
  `reverse`, `pitch`, `mute`; tabla en `pista:fill`).
- `chance(0.1, "retrig", 3)`: cada golpe tiene un 10 % de probabilidad de
  repetirse 3 veces. El azar se tira al cargar, para 16 compases que se
  repiten: cada Ctrl-S lo vuelve a tirar.

**Otra respuesta en la melodía.** `every(n, función)` cambia un compás de
cada `n`: los compases 1, 3, 5... (el primero de cada 2, contando desde
el principio). Cambia el párrafo de `track("teclas", teclas)`:

```lua cambia
track("teclas", teclas)
  :melody("c . eb .  g . eb .  f _ _ .  g _ bb _")
  :every(2, function(c) c:melody("c . eb .  g . eb .  bb _ g .  f _ eb _") end)
  :fx("lowpass", { cutoff = lfo("1 compás", 900, 5000), q = 3 })
  :fx("distortion", { drive = 3, mix = 0.3 })
  :pan(-0.3)
  :send("delay", 0.3)
  :send("reverb", 0.25)
  :gain(0.5)
```

Ctrl-S. La frase alterna dos finales. Dentro de la función, `c` es el
compás: `c:melody`, `c:steps`, `c:reverse()`, `c:pitch(n)`... (tabla en
`pista:every`).

**Secciones.** Una sección dice qué cambia mientras suena; las pistas
siguen definidas fuera. `arrange` pone las secciones en orden, cada una
con sus compases. Añade al final:

```lua añade
section("intro", function()
  track("drums"):mute()
  track("teclas"):mute()
  track("bajo"):mute()
  track("acordes"):fx("lowpass", { cutoff = ramp(300, 6000, "4 compases") })
end)

section("drop", function() end)

section("break", function()
  track("drums"):steps("kick . . .  . . . .  . . snare .  . . . .")
  track("perc"):mute()
  track("bajo"):mute()
end)

arrange({ {"intro", 4}, {"drop", 8}, {"break", 4}, {"drop", 8} })
```

Ctrl-S. La canción tiene ahora 24 compases y vuelve a empezar al acabar:

- **intro** (4 compases): solo la percusión y los acordes, con un filtro
  que se abre.
- **drop** (8): todo, como estaba.
- **break** (4): un bombo y una caja sueltos, los acordes y la melodía.
- **drop** (8) otra vez.

Encima de la vista del sample, una fila enseña la línea de tiempo y por
qué compás va (`▶ drop 3/8`). Dentro de una sección todo cuenta desde su
primer compás: el `fill` cada 4 cae al final de cada 4 compases del drop.

**Moverte en vivo.** En el modo tocar, `]` y `[` saltan a la sección
siguiente o anterior, y `l` repite la que suena (otra vez, sigue la línea
de tiempo). Los saltos entran al empezar el compás siguiente. F1 «Saltar a
una sección» pide el nombre (`seccion break`). Más, en «Una canción
entera con secciones» ([sección 6](#6-recetas-para-amen-breaks)).

**Variante A/B.** Para comparar dos versiones del drop sin tocar el
texto cada vez. Añade al final:

```lua añade
variant("drop", "B", function()
  track("teclas"):mute()
  track("drums"):steps("kick . 2 2  snare . 2 kick  . kick 2 .  snare . 2r4 2r8")
end)
```

Ctrl-S. Suena la A (lo escrito). En el modo tocar, `v` (o F1 «Alternar
variante A/B») pasa a la B en el compás siguiente: el drop sin melodía y
con otra batería. El panel del motor dice `variante A` o `variante B`.

**Si no pasa:**

- Una pista no suena en una parte: mira si esa sección la silencia
  (`:mute()`), o si estás en la variante B.
- `arrange: la sección "drp" no existe (hay: drop)`: el nombre tiene que
  ser igual que en `section(...)`.
- `every/chance dan un ciclo de 77 compases (máximo 64)`: usa números que
  se lleven bien (2, 4, 8, 16).
- `bpm() es de toda la canción: va fuera de section()`: el tempo y los
  buses (`bpm`, `bus`) van fuera de las secciones. Dentro de `every` no
  se puede llamar a `track`, `bpm` ni `sample`.

### Paso 13. Guardar, versiones y exportar

**Guardar.** Ctrl-S guarda `cancion.lua` (y lo carga). F2 `guardar`
guarda además el proyecto `cancion.sampler` (la carpeta de samples, el
tempo, las etiquetas y los ajustes de corte hechos desde la interfaz).
Ctrl-Q sale; si hay cambios sin guardar, pide un segundo Ctrl-Q.

**Versiones.** Cada carga buena se guarda sola como una versión, fuera de
la carpeta de la canción. F1 «Versiones» abre la lista, de la más nueva a
la más antigua, con la fecha y una vista previa. Enter pone esa versión
en el editor **sin guardar**: Ctrl-Z la quita y Ctrl-S la guarda. Así
puedes volver a lo que sonaba hace media hora. Ver «Historial de
versiones» en la [sección 2](#2-atajos-de-teclado).

**Exportar la mezcla a WAV.** F2, escribe `exportar` y Enter. Se crea
`cancion-mezcla.wav` en la carpeta de la canción, con la canción entera
(los 24 compases de `arrange`). Se renderiza aparte: lo que suena no se
corta. Graba lo último que se cargó bien y la variante que suena. Para
otro nombre o más compases: `exportar final.wav 48`.

Lo mismo sin abrir la interfaz, desde la carpeta de la canción:

```sh
tui --script cancion.lua --render cancion.wav
```

Con `arrange`, renderiza la línea de tiempo entera; sin ella, 4 compases
(`--bars 8` para otros). El metrónomo nunca entra, salvo con
`--metronome`. Ver «Renderizar a WAV» en la [sección 1](#1-inicio-rápido).

**Escucharla en otra máquina.** Si trabajas por SSH, el audio sale por
los altavoces de la máquina remota. Bájate el WAV:

```sh
scp laptop:~/musica/primera/cancion-mezcla.wav .
```

**Si no pasa:** si el mensaje dice que el texto tiene cambios sin cargar
o con errores, se ha exportado la última versión buena: Ctrl-S y exporta
otra vez. Si el WAV suena saturado, mira el medidor del master
(`LIMITADOR`, `SATURA`) y baja los `:gain()`.

Y recuerda: si publicas la canción, respeta los derechos de los samples
que uses.

### El script final

Lo que llevas si has seguido todos los pasos:

```lua entero
bpm(170)
bus("delay", { time = "3/16", feedback = 0.4, level = 0.6 })
bus("reverb", { size = 0.6, damp = 0.5 })

local amen = sample("amen.wav"):slice(8)
amen:name(1, "kick"):name(3, "snare")
amen:fx("snare", "bitcrush", { bits = 6, downsample = 2 })
amen:fx("kick", "lowpass", { cutoff = 2500 })

track("drums", amen)
  :steps("kick . 2 2@0.4  snare . 4{hp=4000} .  kick kick^-2 2 .  snare . snare'r2 2r4{lp=300>9000}")
  :fill("roll", 4)
  :chance(0.1, "retrig", 3)
  :fx("highpass", { cutoff = 40 })
  :fx("compressor", { threshold = -14, ratio = 3, attack = 10, makeup = 2 })
  :send("reverb", 0.1)
  :gain(0.6)

local perc = sample("perc.wav"):stretch(120):slice_grid(1)

track("perc", perc)
  :steps("1 . . .  . . . .  . . . .  . . . .")
  :fx("lowpass", { cutoff = ramp(1500, 9000, "4 compases"), q = 2 })
  :fx("bitcrush", { bits = 8, downsample = 2 })
  :pan(lfo("2 compases", -0.5, 0.5))
  :send("delay", 0.25)
  :gain(0.4)

local teclas = sample("nota.wav"):instrument({
  root = "C4", voices = 6, adsr = { 10, 300, 0.6, 400 }, loop = { 300, 1200 },
})

track("teclas", teclas)
  :melody("c . eb .  g . eb .  f _ _ .  g _ bb _")
  :every(2, function(c) c:melody("c . eb .  g . eb .  bb _ g .  f _ eb _") end)
  :fx("lowpass", { cutoff = lfo("1 compás", 900, 5000), q = 3 })
  :fx("distortion", { drive = 3, mix = 0.3 })
  :pan(-0.3)
  :send("delay", 0.3)
  :send("reverb", 0.25)
  :gain(0.5)

track("acordes", teclas)
  :melody("c+eb+g _ _ _  _ _ _ _  ab3+c+eb _ _ _  bb3+d+f _ _ _")
  :fx("highpass", { cutoff = 200 })
  :pan(0.3)
  :send("reverb", 0.4)
  :gain(0.3)

local grave = sample("bajo.wav"):instrument({
  root = "C2", voices = 1, adsr = { 5, 200, 0.8, 150 },
})

track("bajo", grave)
  :melody("c2 _ _ _  . . c2 .  eb2 _ _ .  f2 _ g2 _")
  :fx("lowpass", { cutoff = 500 })
  :fx("dyneq", { freq = 80, range = 6, threshold = -30, attack = 2, release = 80, sidechain = "drums" })
  :gain(0.7)

priority({ "drums", "bajo" })

section("intro", function()
  track("drums"):mute()
  track("teclas"):mute()
  track("bajo"):mute()
  track("acordes"):fx("lowpass", { cutoff = ramp(300, 6000, "4 compases") })
end)

section("drop", function() end)

section("break", function()
  track("drums"):steps("kick . . .  . . . .  . . snare .  . . . .")
  track("perc"):mute()
  track("bajo"):mute()
end)

arrange({ {"intro", 4}, {"drop", 8}, {"break", 4}, {"drop", 8} })

variant("drop", "B", function()
  track("teclas"):mute()
  track("drums"):steps("kick . 2 2  snare . 2 kick  . kick 2 .  snare . 2r4 2r8")
end)
```

### Si algo no suena, mira esto

| Lo que pasa | Mira esto |
| --- | --- |
| No se oye nada, pero el tracker se mueve | ¿Abriste con `--allow-audio`? ¿El volumen del sistema está a cero (`wpctl get-volume @DEFAULT_AUDIO_SINK@`)? ¿Hay alguna pista en solo (`S` en el tracker)? F1 «Quitar mute y solo» |
| El tracker no se mueve | Ctrl-T arranca. El secuenciador arranca solo con la primera carga buena: si el script tiene un error desde el principio, corrígelo y Ctrl-S |
| Borde rojo en el editor | El error está al pie; Ctrl-G va a su línea. Sigue sonando lo anterior |
| `el script no existe (buscado en ...)` al abrir | Estás en otra carpeta o el nombre tiene una errata. Para empezar un script nuevo, `--new` (paso 2) |
| `error leyendo WAV` | La ruta del sample (paso 1): nombre, mayúsculas, carpeta. ¿Es un WAV de verdad? Conviértelo con `ffmpeg` |
| `la pista "x" no existe` | Para crear una pista hacen falta los cortes: `track("x", cortes)`. `track("x")` solo recupera una ya creada (paso 5) |
| `el corte N no existe` | El detector encontró menos golpes que los que pediste: quita ese número del patrón o pide menos cortes |
| `el patrón necesita 16 pasos` | Cuenta los pasos: cuatro grupos de cuatro (o los de `:length(n)`) |
| He cambiado algo y no se oye | ¿Has cargado (Ctrl-S)? Los cambios entran al empezar el compás siguiente. Con secciones, ¿esa pista suena en esta sección? ¿Estás en la variante B? |
| Una pista no se oye | `:mute()` en el script o `m` en vivo (`M` en el tracker); otra pista en solo; `:gain(0)`; la sección la silencia |
| Suena saturado o distorsionado | El panel del motor dice `LIMITADOR` o `SATURA`: baja los `:gain()`, los `@` y los envíos (ver «Saturación», [sección 7](#7-solución-de-problemas)) |
| Muy fuerte de golpe | Un `@` o un `:gain()` mayor que 1 (llegan hasta 4). Baja el volumen del sistema antes de probar |
| El instrumento suena desafinado | `root` tiene que ser la nota en la que suena tu sample (paso 8) |
| Las notas se cortan | Más `voices`; un `release` más largo en `adsr`; o un `loop` para que la nota dure más que el sample |
| El bucle de la nota hace clic | Pon el `loop` en una zona estable de la nota, o sube el `crossfade` del instrumento |
| No se resalta el código | Se resalta solo lo que está cargado: carga (Ctrl-S). F1 «Resaltado en vivo» lo enciende. Lo que sale de una función (`euclid`...) no se resalta |
| Las teclas no tocan nada | Esc para entrar en el modo tocar. Con el editor en su ventana, se toca en la principal. Ctrl-O elige la pista |
| El editor en su ventana no conecta | La principal tiene que estar abierta con `--split` (o haberlo sacado con F1) y las dos con el mismo script |
| El WAV exportado está vacío o corto | Exporta lo último cargado bien. Sin `arrange`, son 4 compases (o el ciclo de `every`/`chance`) |

### Para seguir

- Todos los símbolos de los patrones: [sección 3](#3-mini-notación).
- Todas las funciones de Lua, con sus parámetros y rangos:
  [sección 4](#4-api-de-lua).
- Todas las teclas, la paleta y la línea de órdenes:
  [sección 2](#2-atajos-de-teclado).
- Cuándo entran los cambios y qué pasa con los errores:
  [sección 5](#5-cuantización-recarga-atómica-y-errores).
- Más ideas para el break (ghost notes, stutter, granos, secciones):
  [sección 6](#6-recetas-para-amen-breaks).
- Problemas con los cortes, el pegado y la saturación:
  [sección 7](#7-solución-de-problemas).
- Proyectos, render, choques de frecuencia y el editor en su ventana:
  [sección 1](#1-inicio-rápido).
