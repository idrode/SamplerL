# Guía de uso

Sampler de breaks con live coding en la terminal: cortas un break (el amen,
por ejemplo) en sus golpes, los secuencias con una mini-notación y lo
controlas todo desde un script de Lua que se recarga en caliente.

Todos los bloques de código Lua de esta guía se comprueban en los tests
(`cargo test -p script --test guide`): se cargan contra el break sintético
y deben funcionar sin errores. Las recetas están también como archivos en
`scripts/ejemplos/`.

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

Recuerda respetar los derechos de los samples si publicas lo que hagas.

### Abrir la interfaz

```sh
cargo run -p tui -- --edit mi-cancion.lua
```

Si el archivo no existe, se abre una plantilla. A la izquierda está el
editor; a la derecha, el estado del motor, la forma de onda del sample y el
tracker; abajo, los efectos de cada pista. Ctrl-S guarda y carga.

**Sin `--allow-audio` no suena nada**: el motor avanza igual (el tracker se
mueve), pero el audio se tira. Para oírlo por la tarjeta:

```sh
cargo run -p tui -- --edit mi-cancion.lua --allow-audio
```

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
mínima entre cortes, 40) y `--detector flux|energy` (flux). Imprime los
cortes detectados con su posición en ms.

Desde la interfaz, la orden `exportar` (F2) hace lo mismo con lo que está
cargado.

Para escuchar en otra máquina: `scp laptop:~/sampler/renders/ritmo.wav .`

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
```

Las rutas relativas lo son a la carpeta del proyecto. Con `samples`,
`sample("amen.wav")` busca en esa carpeta. El `bpm` es el tempo por
defecto: si el script llama a `bpm()`, manda el script. Cada `etiqueta`
nombra un corte: sample (como se escribe en `sample("...")`), número de
cortes (la `n` de `slice(n)`: con otro número los cortes son otros y no se
aplica, y al cargar se avisa: `⚠ la etiqueta «kick» de amen.wav es de
slice(8) y este script usa slice(6): no se aplica`), corte y nombre. Así `steps("kick . snare .")` funciona sin
`:name()` en el script; si el script nombra el mismo corte, manda el
script. Se abre con
`tui --edit proyecto.sampler` o con la orden `abrir`.

## 2. Atajos de teclado

### Interfaz

| Tecla | Qué hace |
| --- | --- |
| Ctrl-S | Guardar el script y cargarlo (entra en el siguiente compás) |
| Ctrl-L o F5 | Cargar el texto del editor sin guardar |
| Ctrl-T o F8 | Marcha / paro del secuenciador |
| Ctrl-G | Ir a la línea del error |
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
elegido en `fx("...")` o `chance(p, "...")`); dentro de la tabla de
`fx("efecto", { ... })`, los parámetros del efecto y, tras `clave =`, el
rango y el valor por defecto de ese parámetro. Sale del mismo registro que
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
| Etiquetar el corte seleccionado | Abre «etiquetar N » para escribir la etiqueta (kick, snare...); se guarda en el proyecto | — |
| Corte siguiente | Selecciona el corte siguiente de la pista (sin tocarlo) | — |
| Corte anterior | Selecciona el corte anterior de la pista (sin tocarlo) | — |
| Salir | Sale (pide confirmación si hay cambios sin guardar) | Ctrl-Q |
| Snippet: `amen`, `fill`, `ghost`, `intro` | Inserta el snippet en el cursor (ver «Snippets» en la sección 4) | — |

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
| Inicio de línea | Funciones globales (`bpm`, `sample`, `track`), snippets (`amen`, `fill`, `ghost`, `intro`), variables locales y palabras clave de Lua |
| Tras `:` | Los métodos del objeto: de sample, de cortes, de pista o del compás de `every` (si no se sabe qué es, todos) |
| Dentro de `fx("...")` | Los efectos; dentro de la tabla, sus parámetros, y en `type = "..."` sus valores |
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

### Modo tocar (Esc)

| Teclas | Cortes de la pista seleccionada |
| --- | --- |
| 1 2 3 4 5 6 7 8 9 0 | cortes 1 a 10 |
| q w e r t y u i | cortes 11 a 18 |

El corte suena al instante (no espera al compás), con el volumen y los
efectos de la pista, y corta lo que sonaba en ella. El texto no se toca.

### Línea de órdenes (F2)

Enter ejecuta y Esc cancela. Las rutas son relativas a la carpeta del
proyecto o, si no hay, a la del script.

| Orden | Qué hace |
| --- | --- |
| `exportar [archivo.wav] [compases]` | Renderiza la mezcla a WAV. Por defecto `<script>-mezcla.wav` y la canción entera (el ciclo de `every`/`chance`, al menos 4 compases) |
| `guardar [proyecto.sampler]` | Guarda el script y el proyecto |
| `abrir proyecto.sampler` | Abre un proyecto (`abrir!` descarta los cambios sin guardar) |
| `muestras [carpeta]` | Cambia la carpeta de los samples (sin argumento, dice cuál es) |
| `etiquetar corte [nombre]` | Pone una etiqueta a un corte de la pista seleccionada (sin nombre, la quita). Se guarda en el proyecto abierto y se recarga |

### Editor

Los del widget de texto (`ratatui-textarea`): flechas para moverse, Tab
indentar (salvo con el autocompletado o un parámetro pendiente), Ctrl-Z o
Ctrl-U deshacer, Ctrl-R rehacer, Ctrl-C copiar, Ctrl-X cortar, Ctrl-Y pegar,
Ctrl-K borrar hasta el final de la línea, Ctrl-W borrar la palabra
anterior, Ctrl-A / Ctrl-E inicio y final de línea, Ctrl-V / Alt-V página
abajo y arriba.

## 3. Mini-notación

Un patrón son **16 pasos** (semicorcheas de un compás de 4/4) separados por
espacios. Los espacios de más no cuentan, así que se pueden agrupar por
tiempos.

| Símbolo | Significado | Ejemplo |
| --- | --- | --- |
| `3` | Corte 3 (se cuenta desde 1) | `3` |
| `.` | Silencio: no dispara nada; lo que sonaba sigue sonando | `.` |
| `x` | Igual que `1` (cómodo con `slice_at`) | `x` |
| `kick` | El corte con esa etiqueta (ver `cortes:name` en la sección 4) | `kick`, `snare` |
| `'` | Reverse: el corte suena del final al principio | `3'` |
| `^n` | Pitch en semitonos (signo y decimales, de -36 a 36). Cambia también la duración | `3^7`, `3^-5`, `3^0.5` |
| `@g` | Volumen del paso (lineal, de 0 a 4) | `3@0.5` |
| `rN` | Retrigger / stutter: el paso se divide en N partes y el corte vuelve a empezar en cada una (de 1 a 16) | `3r4` |

Los sufijos se combinan en cualquier orden, cada uno una vez:
`3'^7@0.5r2` es el corte 3, al revés, una quinta arriba, a media
intensidad y repetido dos veces. Con etiquetas, igual: `kick^7@0.5`,
`snare'r2`. Un `r` y un número pegados al final de la etiqueta son el
retrigger (`kickr4` es `kick` repetido 4 veces), por eso ninguna etiqueta
puede acabar así.

Mientras escribes, el editor revisa cada `steps("...")` sin esperar a
guardar: el paso con error sale subrayado en rojo y el aviso (`⚠ línea N:
...`) aparece al pie del editor. Comprueba la notación, las etiquetas, que
haya 16 pasos y los rangos de volumen, pitch y retrigger; los cortes que no
existen se ven al cargar. Las etiquetas que conoce son las de lo último
cargado, las del proyecto y las que el texto pone con `:name()`, de
cualquier pista: si una etiqueta no es de esa pista, lo dice la carga.

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

### `sample(ruta)`

Devuelve: sample.

Carga un WAV. La ruta es relativa a la carpeta del script o, en un proyecto, a su carpeta `samples`. Recargar el script no vuelve a leer el archivo si no ha cambiado.

- `ruta`: archivo WAV (cualquier frecuencia; se resamplea al cargar)

```lua
local amen = sample("amen.wav")
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

### `euclid(k, n [, golpe])`

Patrón euclídeo: k golpes repartidos por igual en n pasos, repetido hasta 16. Devuelve texto en mini-notación para `steps`. Empieza por un golpe (el paso `i` suena si `i·k mod n < k`); para desplazarlo, `rotate`. Si `n` no divide a 16, el ritmo se repite y se corta en el paso 16. El golpe puede ser cualquier paso que suene, con sufijos o una etiqueta.

- `k`: golpes (0 a n)
- `n`: pasos del ritmo (1 a 16)
- `golpe`: el paso que suena (por defecto "x", el corte 1)

```lua
local amen = sample("amen.wav"):slice(8)
-- "1 . . 1  . . 1 .  1 . . 1  . . 1 ."
track("kick", amen):steps(euclid(3, 8, "1"))
track("hats", amen):steps(euclid(5, 16, "2@0.6"))
```

### `rotate(patrón, n)`

Desplaza un patrón n pasos: positivo, más tarde; negativo, antes. Los pasos que se salen por un lado entran por el otro. Devuelve texto en mini-notación; se puede combinar con las demás funciones de patrones.

- `patrón`: 16 pasos en mini-notación
- `n`: pasos que se desplaza (negativo: hacia antes)

```lua
local amen = sample("amen.wav"):slice(8)
-- El break empezando en su segundo tiempo.
track("drums", amen):steps(rotate("1 . 2 .  3 . 4 .  5 6 2 .  7 . 8 .", -4))
-- Un euclídeo que empieza más tarde.
track("kick", amen):steps(rotate(euclid(3, 8, "1"), 2))
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

### `pista:steps(texto)`

Devuelve: pista.

Patrón del compás en mini-notación (16 pasos). La notación está en la sección 3. Los cortes que no existen dan error en esa línea.

- `texto`: 16 pasos separados por espacios

```lua
local amen = sample("amen.wav"):slice(8)
track("drums", amen):steps("1 . 2 .  3 . 4 .  5 6 2 .  7 . 8 .")
```

### `pista:gain(g)`

Devuelve: pista.

Volumen de la pista, lineal de 0 a 4. Se aplica después de los efectos.

- `g`: volumen lineal (0 a 4; 1 lo deja igual)

```lua
local amen = sample("amen.wav"):slice(8)
track("drums", amen):steps("1 . 2 .  3 . 4 .  5 6 2 .  7 . 8 ."):gain(0.7)
```

### `pista:fx(nombre, parámetros)`

Devuelve: pista.

Añade un efecto a la cadena de la pista (hasta 4, en orden). Un parámetro desconocido da error, para pillar las erratas. Los efectos y sus parámetros están en la tabla de abajo.

El filtro es un biquad de 12 dB/octava; `q = 0.707` es la respuesta más plana y valores altos resuenan en el corte. El bitcrusher reduce la resolución a `bits` y repite cada muestra `downsample` veces (reducción de la frecuencia de muestreo, sin filtro: suena sucio a propósito).

- `nombre`: el efecto (ver la tabla)
- `parámetros`: tabla con los parámetros del efecto

| Efecto | Parámetro | Qué es | Por defecto | Valores |
| --- | --- | --- | --- | --- |
| `"filter"` | `type` | tipo de filtro | `"lowpass"` | `"lowpass"` o `"highpass"` |
|  | `cutoff` | frecuencia de corte en Hz | obligatorio | 20–20000 |
|  | `q` | resonancia | `0.707` | 0.1–20 |
| `"lowpass"` | `cutoff` | frecuencia de corte en Hz | obligatorio | 20–20000 |
|  | `q` | resonancia | `0.707` | 0.1–20 |
| `"highpass"` | `cutoff` | frecuencia de corte en Hz | obligatorio | 20–20000 |
|  | `q` | resonancia | `0.707` | 0.1–20 |
| `"bitcrush"` (o `"bitcrusher"`) | `bits` | resolución | `8` | 1–16 |
|  | `downsample` | repite cada muestra n veces | `1` | 1–64 |

```lua
local amen = sample("amen.wav"):slice(8)
track("drums", amen)
  :steps("1 . 2 .  3 . 4 .  5 6 2 .  7 . 8 .")
  :fx("filter", { type = "lowpass", cutoff = 3000 })
  :fx("highpass", { cutoff = 80, q = 0.9 })
  :fx("bitcrush", { bits = 6, downsample = 2 })
```

### `pista:every(n, función)`

Devuelve: pista.

En los compases 0, n, 2n... llama a función(compás). Como en Tidal, cuenta los compases de la canción. La función puede cambiar ese compás con los métodos de la tabla de abajo, que se pueden encadenar. Dentro de la función no se puede llamar a `track`, `bpm` ni `sample`.

- `n`: cada cuántos compases (1 a 64)
- `función`: recibe el compás (`function(c) ... end`)

| Método | Qué hace |
| --- | --- |
| `c:steps(texto)` | Sustituye los pasos del compás. |
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
| `fill` | Relleno con stutter al final de cada N compases en una pista existente. |
| `ghost` | Pista de ghost notes: un golpe muy suave en contratiempos que aparece al azar. |
| `intro` | Pista de intro: pocos golpes, filtrada y más baja. |

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
  :every(4, function(c) c:steps("1 . 2 .  3 . 4 .  5 6 2 .  3r2 3r4 7r8 7r16") end)
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

### Saturación

La suma de pistas pasa por un limitador suave: por debajo de 0,9 no toca
nada y por encima comprime hasta ±1 sin recortar. Si se nota apretado:

- baja el volumen de las pistas con `:gain()`, sobre todo cuando dos pistas
  tocan el mismo golpe a la vez (break + refuerzo de bombo);
- baja golpes concretos con `@` (`5@0.6`);
- recuerda que `@` y `:gain()` llegan hasta 4: más de 1 sube el volumen.

### Errores comunes de Lua

Los errores salen con archivo y línea, en el editor y en la línea de
comandos. Mientras tanto sigue sonando lo anterior.

```text
song.lua:2: steps: el patrón necesita 16 pasos, tiene 4
```

El patrón tiene que tener exactamente 16 pasos. Rellena con `.`.

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

Has pasado a `track()` un `sample(...)` sin cortar: añade `:slice(n)` (o
`:slice(1)` para el sample entero).

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
