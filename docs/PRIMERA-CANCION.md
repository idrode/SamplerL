# Tu primera canción, de principio a fin

Un tutorial para quien nunca ha usado el programa. Montas una canción
entera, paso a paso: un break troceado, una segunda percusión, efectos,
una melodía, un bajo y una estructura con intro, drop y break. Al final
la guardas y la exportas a WAV.

Cada paso es una acción con un resultado que puedes ver u oír. Si algo no
pasa como se dice, cada paso explica qué mirar. Lo que ya está explicado
en otra sección de la guía no se repite aquí: hay un enlace. Cada paso
empieza con una línea de **Referencia** que enlaza con la
[guía de referencia](GUIA.md), y al final de esas secciones de la guía
hay un enlace de vuelta al paso.

**Qué debes saber antes de empezar.** Nada del programa, pero sí lo
básico de una terminal de Linux (moverte con `cd`, crear carpetas con
`mkdir`, copiar con `cp`), tener la toolchain de Rust para compilarlo
una vez (`cargo`) y cuatro archivos WAV a mano (el paso 1 dice cuáles, y
cómo fabricarlos si no los tienes). `ffmpeg` solo hace falta para
convertir formatos. No hace falta saber Lua: el tutorial dice qué escribir en cada
paso. Lo que no verás aquí, como todas las funciones de Lua con sus
parámetros, está en la [guía de referencia](GUIA.md#4-api-de-lua).

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

## Paso 1. Preparar la carpeta

> **Referencia:** [Generar el break sintético](GUIA.md#generar-el-break-sintético) · [Usar un sample propio](GUIA.md#usar-un-sample-propio).

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
la [sección 1](GUIA.md#1-inicio-rápido)). Cópialo dos veces y fabrica las dos
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
carpeta; ver «Proyectos» en la [sección 1](GUIA.md#1-inicio-rápido)):

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

## Paso 2. Abrir un proyecto nuevo

> **Referencia:** [Abrir la interfaz](GUIA.md#abrir-la-interfaz) · [Proyectos](GUIA.md#proyectos) · [El editor en su propia ventana](GUIA.md#el-editor-en-su-propia-ventana) · [Interfaz](GUIA.md#interfaz) · [Modo tocar (Esc)](GUIA.md#modo-tocar-esc).

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
[sección 1](GUIA.md#1-inicio-rápido)). Ctrl-Q sale.

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
propia ventana» ([sección 1](GUIA.md#1-inicio-rápido)). Para empezar, una sola
ventana basta.

## Paso 3. Cargar un break y trocearlo

> **Referencia:** [sample(ruta [, opciones])](GUIA.md#sampleruta--opciones) · [sample:slice(n [, opciones])](GUIA.md#sampleslicen--opciones) · [Saber el tempo del break](GUIA.md#saber-el-tempo-del-break) · [sample:detect_bpm()](GUIA.md#sampledetect_bpm) · [bpm(n)](GUIA.md#bpmn) · [Etiquetar cortes desde la interfaz](GUIA.md#etiquetar-cortes-desde-la-interfaz).

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
- Ni el tracker se mueve: Ctrl-Espacio (o Ctrl-T) arranca y para el secuenciador.
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
elijas. Ver «Saber el tempo del break» en la [sección 1](GUIA.md#1-inicio-rápido).

**Oír cada corte.** Pulsa Esc: entras en el **modo tocar** (la línea de
ayuda cambia). Las teclas `1` a `8` tocan los cortes 1 a 8 de la pista
seleccionada, al instante. ← y → recorren los cortes y los preescuchan;
espacio vuelve a sonar el seleccionado. Apunta cuál es el bombo y cuál la
caja. Esc vuelve al editor. Todas las teclas están en «Modo tocar (Esc)»
([sección 2](GUIA.md#2-atajos-de-teclado)).

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
la [sección 7](GUIA.md#7-solución-de-problemas).

> **¿Un clic al empezar un corte?** Pasa sobre todo en rejilla, porque
> la división cae en cualquier punto de la onda. `slice_grid(16, {
> zero_cross = 2 })` (o `slice(8, { ... })`) lleva cada inicio al cruce
> por cero más cercano, a no más de 2 ms. Vale también para tus ajustes
> con `,` y `.`. Ver
> [sample:slice(n [, opciones])](GUIA.md#sampleslicen--opciones).

## Paso 4. El primer patrón de batería

> **Referencia:** [mini-notación](GUIA.md#3-mini-notación) · [pista:steps(texto)](GUIA.md#pistastepstexto) · [track(nombre, cortes) / track(nombre)](GUIA.md#tracknombre-cortes--tracknombre) · [recarga y cuantización](GUIA.md#5-cuantización-recarga-atómica-y-errores).

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
enciende; ver «Resaltado en vivo» en la [sección 2](GUIA.md#2-atajos-de-teclado).

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
llaves) están en la [sección 3](GUIA.md#3-mini-notación).

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

## Paso 5. Otro sample en otra pista

> **Referencia:** [sample:stretch([bpm_origen])](GUIA.md#samplestretchbpm_origen) · [pista:gain(g)](GUIA.md#pistagaing) · [pista:pan(p)](GUIA.md#pistapanp) · [pista:length(n)](GUIA.md#pistalengthn).

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
[sección 4](GUIA.md#4-api-de-lua).

**Si no pasa:**

- `la pista "perc" no existe; defínela con track("perc", sample(...):slice(n))`:
  has escrito `track("perc")` sin los cortes. Con un solo argumento,
  `track` solo **recupera** una pista ya definida; para crearla hacen
  falta los dos: `track("perc", perc)`.
- `argumento 2 no válido`: has pasado a `track()` el sample sin cortar.
  Añade `:slice(n)` o `:slice_grid(n)`.
- Suena demasiado fuerte al sumar las dos pistas: baja los `:gain()`. El
  panel del motor avisa con `LIMITADOR` o `SATURA` (ver «Saturación» en
  la [sección 7](GUIA.md#7-solución-de-problemas)).

**Elegir samples sin salir.** F1 «Explorador de samples» lista los WAV
de la carpeta de samples (la del script, o la `samples` del proyecto) y
de sus subcarpetas. Escribe para filtrar por
nombre; ↑ ↓ eligen y preescuchan cada uno (sin tocar las pistas); Tab lo
vuelve a sonar; Enter escribe `sample("ruta")` donde está el cursor; Esc
cierra. Con el editor en su ventana, el explorador se abre en la
principal.

## Paso 6. Efectos en un solo corte

> **Referencia:** [cortes:fx(cuál, nombre, parámetros)](GUIA.md#cortesfxcuál-nombre-parámetros) · [cortes:set([i,] parámetros)](GUIA.md#cortesseti-parámetros) · [Resaltado en vivo](GUIA.md#resaltado-en-vivo) · [Ensuciar solo la caja (efectos por corte)](GUIA.md#ensuciar-solo-la-caja-efectos-por-corte).

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
la pista (tabla en `pista:fx`, [sección 4](GUIA.md#4-api-de-lua)), hasta 4 por
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
«Efectos por paso» de la [sección 3](GUIA.md#3-mini-notación).

**Si no pasa:** un paso con un efecto mal escrito sale subrayado en rojo
antes de guardar. Entre el corte y el paso caben 4 efectos: si no, la
carga da error con la línea. Las llaves no valen en una pista de
instrumento (pasos 8-11): allí, efectos de pista.

## Paso 7. Efectos de pista y automatización

> **Referencia:** [pista:fx(nombre, parámetros)](GUIA.md#pistafxnombre-parámetros) · [ramp(desde, hasta, duración)](GUIA.md#rampdesde-hasta-duración) · [lfo(frecuencia, mín, máx)](GUIA.md#lfofrecuencia-mín-máx) · [Qué se puede automatizar](GUIA.md#qué-se-puede-automatizar) · [bus(nombre, parámetros)](GUIA.md#busnombre-parámetros) · [pista:send(bus, nivel)](GUIA.md#pistasendbus-nivel).

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
automatizar» ([sección 4](GUIA.md#4-api-de-lua)). Más efectos (distorsión, EQ,
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

## Paso 8. Abrir un instrumento

> **Referencia:** [sample:instrument([opciones])](GUIA.md#sampleinstrumentopciones) · [Modo tocar (Esc)](GUIA.md#modo-tocar-esc) · [Melodías con un instrumento](GUIA.md#melodías-con-un-instrumento).

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
`sample:instrument`, [sección 4](GUIA.md#4-api-de-lua).

## Paso 9. Líneas melódicas

> **Referencia:** [pista:melody(notas)](GUIA.md#pistamelodynotas) · [scale([nombre [, tónica]])](GUIA.md#scalenombre--tónica) · [escala:melody(grados [, golpe])](GUIA.md#escalamelodygrados--golpe) · [Modo tocar (Esc)](GUIA.md#modo-tocar-esc) · [MIDI](GUIA.md#midi) · [Dispositivos: audio y MIDI](GUIA.md#dispositivos-audio-y-midi).

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
([sección 4](GUIA.md#4-api-de-lua)).

> **¿En qué tonalidad está tu sample?** F1 «Tonalidad del sample» (o
> Ctrl-K en el explorador) la analiza; `r` la pone de raíz del
> instrumento y `k` propone `key()` para la canción. Ver
> [Saber la tonalidad de un sample](GUIA.md#saber-la-tonalidad-de-un-sample).

> **La tonalidad de la canción.** Con `key("C", "minor")` al principio,
> `melody("1 . 3 .  5 . 3 .")` entiende grados, `scale()` toma su escala y
> los pads del modo tocar tocan los grados de do menor en vez de
> semitonos. `transpose(2)` sube todas las notas de los instrumentos (no
> la batería). Ver [key(tónica [, modo])](GUIA.md#keytónica--modo) y
> [transpose(n)](GUIA.md#transposen).

**Tocar en vivo.** Esc (modo tocar) y Ctrl-O hasta que la pista
seleccionada sea `teclas`. Cada tecla es una nota: `1` la raíz (C4) y
cada tecla siguiente, un semitono más. En do menor: `1` do, `4` mi
bemol, `6` fa, `8` sol, `9` la bemol, `q` si bemol, `e` el do de arriba.
La nota dura mientras mantienes la tecla, pero eso depende de la
terminal: en kitty, foot, WezTerm o Ghostty, hasta que la sueltas; en
tmux y la mayoría, mientras llegan las repeticiones de la tecla. Ver
«Modo tocar (Esc)», [sección 2](GUIA.md#2-atajos-de-teclado).

**Con un teclado MIDI.** Abre con `--allow-midi`, F1 «Dispositivos», →
hasta *MIDI de entrada* y Enter sobre tu teclado. Al tocar, `○ nota` se
enciende. Queda guardado: la próxima vez se conecta solo. Si no sale en
la lista, `r` (después de enchufarlo). También se puede pedir al
arrancar, o conectar a mano al puerto «sampler entrada»:

```sh
tui --edit cancion.sampler --allow-audio --midi-in "Mi teclado"
aconnect -l                         # o, en otra terminal: lista los puertos
aconnect "Mi teclado" "sampler"     # con el nombre que salga en la lista
```

Cada nota suena en la pista seleccionada a su altura real (el C4 del
teclado es la raíz `C4`), con la velocidad como volumen. Ver «MIDI» en la
[sección 2](GUIA.md#2-atajos-de-teclado).

> **¿Tu teclado tiene mandos?** F1 «MIDI learn», elige un parámetro
> (`teclas · gain`, el corte de un filtro...) y gira un mando: queda
> asignado y se guarda en el proyecto. F1 «Mandos MIDI» los lista. Ver
> [MIDI learn](GUIA.md#midi-learn-un-mando-para-cada-parámetro).

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

## Paso 10. Efectos sobre la melodía

> **Referencia:** [pista:fx(nombre, parámetros)](GUIA.md#pistafxnombre-parámetros) · [bus(nombre, parámetros)](GUIA.md#busnombre-parámetros) · [pista:send(bus, nivel)](GUIA.md#pistasendbus-nivel).

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
[sección 3](GUIA.md#3-mini-notación).

**Si no pasa:** `melody: paso 1 ("c{lp=800}"): las llaves ({...}) solo
valen en steps("...") de una pista de cortes, no en melody() ni en un
instrumento: usa los efectos de la pista (:fx)`. En `melody` y en los
instrumentos no hay llaves: usa `:fx()`. Si la
melodía tapa la batería, baja su `:gain()` o sus envíos.

> **¿Va justo de CPU?** Una pista terminada se puede congelar: F1
> «Congelar la pista seleccionada» la renderiza con sus efectos a
> `congelado/<pista>.wav` y añade al final del script una línea
> `frozen(...)` que la cambia por ese WAV. Tu código no se toca y
> «Descongelar» quita la línea. No vale con secciones (el paso 12): hazlo
> antes de ese paso o descongela antes de añadirlas. Ver
> [Congelar una pista](GUIA.md#congelar-una-pista).

## Paso 11. Un bajo que no choque con el bombo

> **Referencia:** [priority(pistas)](GUIA.md#prioritypistas) · [Choques de frecuencia entre pistas](GUIA.md#choques-de-frecuencia-entre-pistas) · [Que el break deje sitio al bombo (EQ dinámico y sidechain)](GUIA.md#que-el-break-deje-sitio-al-bombo-eq-dinámico-y-sidechain) · [synth(tipo, [opciones])](GUIA.md#synthtipo-opciones) · [Un bajo ácido](GUIA.md#un-bajo-ácido).

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
[sección 6](GUIA.md#6-recetas-para-amen-breaks) y en la tabla de `pista:fx`.

> **¿Baja de verdad?** F1 «Vista del sidechain» enseña, mientras suena,
> quién dispara a quién (`bajo · eq dinámico 80 Hz ← drums`), cuántos dB
> baja ahora y las curvas del disparo y de la reducción. Ver
> [Vista del sidechain](GUIA.md#vista-del-sidechain).

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

## Paso 12. Variación y estructura

> **Referencia:** [section(nombre, función)](GUIA.md#sectionnombre-función) · [arrange({ {sección, compases}, ... })](GUIA.md#arrange-sección-compases--) · [variant(nombre, letra, función)](GUIA.md#variantnombre-letra-función) · [Una canción entera con secciones](GUIA.md#una-canción-entera-con-secciones) · [Variantes A/B](GUIA.md#variantes-ab) · [Vista de ocupación](GUIA.md#vista-de-ocupación).

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
entera con secciones» ([sección 6](GUIA.md#6-recetas-para-amen-breaks)).

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

## Paso 13. Guardar, versiones y exportar

> **Referencia:** [Proyectos](GUIA.md#proyectos) · [Historial de versiones](GUIA.md#historial-de-versiones) · [Renderizar a WAV](GUIA.md#renderizar-a-wav) · [Arrancar desde un compás: el cue](GUIA.md#arrancar-desde-un-compás-el-cue).

**Guardar.** Ctrl-S guarda `cancion.lua` (y lo carga). F2 `guardar`
guarda además el proyecto `cancion.sampler` (la carpeta de samples, el
tempo, las etiquetas y los ajustes de corte hechos desde la interfaz).
Ctrl-Q sale; si hay cambios sin guardar, pide un segundo Ctrl-Q.

**Versiones.** Cada carga buena se guarda sola como una versión, fuera de
la carpeta de la canción. F1 «Versiones» abre la lista, de la más nueva a
la más antigua, con la fecha y una vista previa. Enter pone esa versión
en el editor **sin guardar**: Ctrl-Z la quita y Ctrl-S la guarda. Así
puedes volver a lo que sonaba hace media hora. Ver «Historial de
versiones» en la [sección 2](GUIA.md#2-atajos-de-teclado).

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
`--metronome`. Ver «Renderizar a WAV» en la [sección 1](GUIA.md#1-inicio-rápido).

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

## El script final

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

## Si algo no suena, mira esto

| Lo que pasa | Mira esto |
| --- | --- |
| No se oye nada, pero el tracker se mueve | ¿Abriste con `--allow-audio`? F1 «Dispositivos»: ¿por dónde dice que suena? ¿El volumen del sistema está a cero (`wpctl get-volume @DEFAULT_AUDIO_SINK@`)? ¿Hay alguna pista en solo (`S` en el tracker)? F1 «Quitar mute y solo». Más en «Si no suena nada», [sección 2](GUIA.md#2-atajos-de-teclado) |
| El teclado MIDI no toca | F1 «Dispositivos»: ¿se enciende `● nota` al tocar? Si no, elige el teclado en *MIDI de entrada* (`r` si no sale). Ver «Si no llega el MIDI», [sección 2](GUIA.md#2-atajos-de-teclado) |
| El tracker no se mueve | Ctrl-Espacio (o Ctrl-T) arranca. El secuenciador arranca solo con la primera carga buena: si el script tiene un error desde el principio, corrígelo y Ctrl-S |
| Borde rojo en el editor | El error está al pie; Ctrl-G va a su línea. Sigue sonando lo anterior |
| `el script no existe (buscado en ...)` al abrir | Estás en otra carpeta o el nombre tiene una errata. Para empezar un script nuevo, `--new` (paso 2) |
| `error leyendo WAV` | La ruta del sample (paso 1): nombre, mayúsculas, carpeta. ¿Es un WAV de verdad? Conviértelo con `ffmpeg` |
| `la pista "x" no existe` | Para crear una pista hacen falta los cortes: `track("x", cortes)`. `track("x")` solo recupera una ya creada (paso 5) |
| `el corte N no existe` | El detector encontró menos golpes que los que pediste: quita ese número del patrón o pide menos cortes |
| `el patrón necesita 16 pasos` | Cuenta los pasos: cuatro grupos de cuatro (o los de `:length(n)`) |
| He cambiado algo y no se oye | ¿Has cargado (Ctrl-S)? Los cambios entran al empezar el compás siguiente. Con secciones, ¿esa pista suena en esta sección? ¿Estás en la variante B? |
| Una pista no se oye | `:mute()` en el script o `m` en vivo (`M` en el tracker); otra pista en solo; `:gain(0)`; la sección la silencia |
| Suena saturado o distorsionado | El panel del motor dice `LIMITADOR` o `SATURA`: baja los `:gain()`, los `@` y los envíos (ver «Saturación», [sección 7](GUIA.md#7-solución-de-problemas)) |
| Muy fuerte de golpe | Un `@` o un `:gain()` mayor que 1 (llegan hasta 4). Baja el volumen del sistema antes de probar |
| El instrumento suena desafinado | `root` tiene que ser la nota en la que suena tu sample (paso 8) |
| Las notas se cortan | Más `voices`; un `release` más largo en `adsr`; o un `loop` para que la nota dure más que el sample |
| El bucle de la nota hace clic | Pon el `loop` en una zona estable de la nota, o sube el `crossfade` del instrumento |
| No se resalta el código | Se resalta solo lo que está cargado: carga (Ctrl-S). F1 «Resaltado en vivo» lo enciende. Lo que sale de una función (`euclid`...) no se resalta |
| Las teclas no tocan nada | Esc para entrar en el modo tocar. Con el editor en su ventana, se toca en la principal. Ctrl-O elige la pista |
| El editor en su ventana no conecta | La principal tiene que estar abierta con `--split` (o haberlo sacado con F1) y las dos con el mismo script |
| El WAV exportado está vacío o corto | Exporta lo último cargado bien. Sin `arrange`, son 4 compases (o el ciclo de `every`/`chance`) |

## Para seguir

- Todos los símbolos de los patrones: [sección 3](GUIA.md#3-mini-notación).
- Todas las funciones de Lua, con sus parámetros y rangos:
  [sección 4](GUIA.md#4-api-de-lua).
- Todas las teclas, la paleta y la línea de órdenes:
  [sección 2](GUIA.md#2-atajos-de-teclado).
- Cuándo entran los cambios y qué pasa con los errores:
  [sección 5](GUIA.md#5-cuantización-recarga-atómica-y-errores).
- Más ideas para el break (ghost notes, stutter, granos, secciones):
  [sección 6](GUIA.md#6-recetas-para-amen-breaks).
- Problemas con los cortes, el pegado y la saturación:
  [sección 7](GUIA.md#7-solución-de-problemas).
- Proyectos, render, choques de frecuencia y el editor en su ventana:
  [sección 1](GUIA.md#1-inicio-rápido).
