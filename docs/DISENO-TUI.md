# Rediseño de la TUI (propuesta)

Estado: **hecho** (ver «Rediseño de la interfaz» en [ESTADO.md](ESTADO.md)). Lo que cambió al implementarlo, en la sección 11.

La referencia es un estudio externo para la terminal, hecho para otro
lenguaje de patrones y con una licencia copyleft (AGPL). De él se toman
solo ideas: la disposición, la jerarquía visual y algunas formas de
interactuar. No se mira, copia ni adapta su código, ni sus textos ni sus
assets. Todo lo de abajo está descrito con palabras propias y se
implementa desde cero sobre la arquitectura de SamplerL.

## 1. Qué hay hoy

- **Con el editor** (`ui::draw`): el editor ocupa la mitad izquierda. A la
  derecha, de arriba abajo, van el panel «motor» (transporte, master y una
  línea por pista), la vista de ocupación si está abierta, la línea de
  tiempo, el sample y el tracker. Abajo van los efectos por pista a lo
  ancho y una línea de ayuda que junta el mensaje con los atajos. El piano
  roll ocupa el sitio del sample y del tracker. Las visualizaciones, el
  sidechain y las lanes se **dibujan encima** de ese hueco y se apuntan en
  `PanelAreas::covers` para que el ratón no llegue a lo que tapan.
- **Sin el editor** (`draw_main_window`, fase L): arriba va una línea de
  estado, luego la onda grande (con el sidechain y la tabla de choques a
  la derecha), el mezclador y el tracker completo, los efectos y la ayuda.
- **Foco**: no hay un foco explícito. Hay dos modos, `Mode::Edit` y el
  modo tocar (Esc pasa de uno a otro). Encima de ellos hay «cursores»
  que se quedan las teclas: el del tracker (`tracker_edit`), el de la
  ocupación, el punto de una lane y el cursor del piano roll. Se entra en
  cada uno desde la paleta o con un clic, y se sale con Esc, que vuelve al
  modo de antes. Los popups (paleta, explorador, bloques, mandos,
  dispositivos...) toman la entrada en `App::handle`, en este orden de
  prioridad.
- **Teclas**: las acciones con atajo salen de `config::BINDABLE`
  (configurables en `[teclas]`), y F1 abre siempre la paleta. La línea de
  ayuda se arma con `Keymap::help`, o con `key_help` cuando hay un cursor
  activo.
- **Lo que ya existe y sirve**: el refresco configurable (`[interfaz]
  refresco`, 30 Hz por defecto, y la entrada no espera al dibujo), el modo
  ASCII (`[interfaz] ascii`, según `LANG`), la degradación de colores sin
  truecolor (`theme.rs`), la geometría única con su `hit(x, y)`
  (`TrackerLayout`, `OccupancyLayout`, `MixerLayout`), las colas `rtrb`
  de las visualizaciones (`engine::viz`, encendidas con
  `Command::SetViz`), la orden `accion` del protocolo de ventanas (una
  acción de la paleta por su nombre) y los tests de pantalla con
  `TestBackend`.

## 2. Qué se toma de la referencia y qué no

| Idea de la referencia | ¿Aplica? | En SamplerL |
|---|---|---|
| Barra de menús arriba (Archivo, Edición, Escena, Transporte, Vista, Opciones, Ayuda), en la que cada entrada muestra su atajo | **Sí** | La barra se genera desde la paleta: cada `palette::Action` recibe un menú, así que la acción y su tecla se definen en un solo sitio. Se abre con **F10** porque F1 es la paleta y lo sigue siendo. Los menús quedan en Archivo, Editar, Pista, Transporte, Ver, Ventanas y Ayuda. |
| Cabecera con el estado del transporte, el número de avisos y si el sonido está listo, y bajo ella una línea fina que late con el tempo | **Sí** | La cabecera junta el transporte, el cue, la variante, la grabación, la ranura, los avisos del diagnóstico, el link y el master. La regla de 16 pasos va debajo, con la línea de tiempo de secciones. Sustituye a la cabecera del panel «motor». |
| Visualizaciones bajo el código que las produce (scope, piano roll, espectro) | **Ya existe** | `inline = true` en `pista:scope{}` y las demás. Se mantiene tal cual en la API. |
| Dos «docks» de visualización que se eligen desde el menú Ver | **Sí, adaptado** | Un panel de pestañas («dock») sustituye al apilado actual con `covers`: efectos, lanes, scope, espectro, sidechain, ocupación y choques. Se elige con el teclado, el ratón o el menú Ver, y ya no tapa a otros paneles. |
| Pie con el mensaje, la posición y un medidor del master grande con pico y sonoridad | **Sí, en parte** | El mensaje va en su propia línea y la referencia de teclas del foco en otra. El medidor del master se muestra con dB y pico (ya se publican). La sonoridad sería **aproximada**: se calcularía en la interfaz con la cola mono del master tras el limitador, sin tocar el motor, y por eso se rotula «≈ LUFS (aprox.)». Si no convence, se quita. |
| Columna de referencia junto al código (funciones, sonidos, samples) | **Sí, como referencia de teclas** | Columna «referencia» con las teclas del panel que tiene el foco (las mismas de `key_help` y `Keymap::help`, pero enteras). La referencia de la API sigue en F4 y en la línea ⓘ. |
| Columna de «knobs» que convierte en deslizadores los números de la llamada del cursor | **No ahora** | Es buena idea, pero es una función nueva (editar argumentos con deslizadores) y no forma parte del rediseño. Queda anotada como idea para después. |
| Tira de escenas con Alt+número | **No** | No tenemos escenas. Lo más cercano (secciones, variantes A/B y ranuras) ya sale en la cabecera y en la regla. Alt+número se usa para el foco. |
| Panel lateral de partituras, sesiones y cintas | **No** | Ya están el explorador de samples, los bloques y el historial de versiones como popups. Se dejan igual. |
| Pantalla de ajustes con páginas | **No** | La configuración vive en `config.toml`. El menú «Ayuda» muestra la ruta y si la terminal tiene truecolor, Unicode, el protocolo de kitty y ratón, que es lo útil de la página «Acerca de». |
| Editor partido en dos escenas | **No** | El editor en otra ventana (fase L) cubre esa necesidad. |
| Ctrl+Enter para evaluar y Ctrl+. para parar, con alternativas para terminales viejas | **No las teclas; sí la idea** | Seguimos con Ctrl-S/Ctrl-L y Ctrl-Espacio/F8. De la idea tomamos esto: si la terminal no distingue Shift-Ctrl-Espacio, los menús y la referencia muestran F9. |
| Evaluación automática tras una pausa al escribir | **No** | Con la recarga al guardar y la entrada al compás basta. Además, evaluar a medias va contra la regla 5. |
| Visuales externos, otros motores de síntesis, plugins, ciclos por segundo | **No** | Son de otro motor. Aquí hay compases de 16 pasos y BPM. |

Lo propio de SamplerL, que la referencia no tiene y que manda en la jerarquía:
**la onda del sample con los cortes** es el panel principal de la derecha,
y el tracker vertical es la secuencia.

## 3. Disposición

Hay tres anchos de referencia. La rejilla se decide por el tamaño del
terminal en cada cuadro (sin estado): un cambio de tamaño se aplica en el
siguiente dibujo. Los números de los títulos (`1 código`, `2 onda`...)
son las teclas de foco (Alt-1…8).

### 80×24 (mínimo cómodo)

Sin barra de menús visible: F10 la despliega encima. A la derecha, la
onda queda siempre fija con 2 filas y debajo van **pestañas** para lo
demás. El tracker se desplaza siguiendo el paso que suena, o el cursor si
tiene el foco.

```text
▶ 170.0 BPM  3.2.05  ⏮1  A  song.lua•  ⚠1        master ▇▇▇▇▇▇▆░░░  -3.1 dB
▰▰▰▰▰▱▱▱▱▱▱▱▱▱▱▱  intro 4 │ ▶ drop 3/8 ⟳ │ break 4
┌ 1 código ────────────────────────────┐┌ 2 onda · drums · 8 cortes · ▶3 ──────┐
│  1 bpm(170)                          ││1▂▅█▅▂ 2▁▃▆▃ 3▂▇█▆▃▁ 4▂▅▃ 5▃█▅▂ 6▂▄ 7▅│
│  2 local amen = sample("amen.wav")   ││▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀│
│  3   :slice(8)                       │├[3 tracker] 4 mezcla 5 fx 6 lan 7 vis─┤
│  4 track("drums", amen)              ││▲2 drums  kick   bass                 │
│  5   :steps("1 . . 1  . . 3 .  1 . 5 ││02 ---    ---    ---                  │
│  6 track("kick", amen:slice_at(1))   ││03 ---    ---    ---                  │
│  7   :steps("x . . .  x . . .")      ││04 1      x      c2                   │
│  8 track("bass", sh101{})            ││05▶---    ---    ---                  │
│  9   :melody("c2 . eb2 .")           ││06 3      ---    eb2                  │
│ 10                                   ││07 ---    ---    ---                  │
│                                      ││08 1      x      ---                  │
│                                      ││09 ---    ---    ---                  │
│                                      ││10 5      ---    c2                   │
│                                      ││11 ---    ---    ---                  │
│                                      ││12 1      x      ---                  │
│                                      ││13 ---    ---    ---                  │
│                                      ││14 ---    ---    ---                  │
│ ⓘ steps(patrón)  Pasos de la pista.  ││15 ---    ---    ---                  │
└──────────────────────────────────────┘└──────────────────────────────────────┘
cargado: entra en el siguiente compás
EDITOR  Ctrl-S cargar · Ctrl-Espacio marcha · Esc tocar · F1 paleta · F10 menú
```

### 120×40

Con barra de menús. A la derecha están la onda completa (números de
corte, onda, marcas y ms, y la línea del corte elegido), el tracker y el
mezclador lado a lado, y el dock con pestañas abajo.

```text
 Archivo  Editar  Pista  Transporte  Ver  Ventanas  Ayuda                              F10 menú · Alt-1…8 foco
 ▶ en marcha  170.0 BPM  compás 3 paso 05  ⏮ cue 1  variante A  ● rec   ⚠ 1      master ▇▇▇▇▇▇▇▆░░░░  -3.1 dB  pk -1.2
 ▰▰▰▰▰▱▱▱▱▱▱▱▱▱▱▱   intro 4 │ ▶ drop 3/8 ⟳ │ break 4 │ outro 8
┌ 1 código · song.lua• ──────────────────────────────┐┌ 2 onda · drums · amen.wav · 1.82 s · 8 cortes · ▶3 · zoom 1× ──┐
│  1 bpm(170)                                        ││1       2      3         4      5        6      7     8         │
│  2 local amen = sample("amen.wav"):slice(8)        ││▁▃▇█▆▄▂▁ ▂▅▆▃▂▁ ▃▆██▇▅▃▂▁ ▂▄▅▃▁ ▃▇█▆▄▂▁ ▂▄▆▄▂ ▁▃▅▃▁ ▂▃▂▁        │
│  3                                                 ││▔▀█▀▔▔▔▔ ▔▀▀▔▔▔ ▔▀██▀▔▔▔▔ ▔▀▀▔▔ ▔▀█▀▔▔▔ ▔▀▀▔▔ ▔▀▀▔▔ ▔▔▔▔        │
│  4 track("drums", amen)                            │││       │      ┃━━━━━━━━━│      │        │      │     │         │
│  5   :steps("1 . . 1  . . 3 .  1 . 5 .  . 3 . 6")  ││0 ms    228    456       684    912      1140   1368  1596      │
│  6   :fx("filter", { type = "lowpass",             ││▶3 «snare» · C#4 · dur 1/8 trim · rev · 228-456 ms              │
│  7         cutoff = lfo(800, 4000, 2) })           │└────────────────────────────────────────────────────────────────┘
│  8   :scope()                                      │┌ 3 tracker · compás 3 ────────────┐┌ 4 mezcla ──────────────────┐
│    ╶ scope drums ╴ ⡀⢀⣀⠤⠤⠒⠉⠉⠑⠒⠤⣀⡀⢀⣀⠤⠒⠉⠑⠒⠤⣀⡀⢀⣀⠤⠒⠉⠉⠑⠤⣀││   drums   kick    bass           ││ drums kick  bass  │ master │
│  9 track("kick", amen:slice_at(1))                 ││00 1       x       c2             ││  ░░    ░░    ░░   │   ░░   │
│ 10   :steps("x . . .  x . . .  x . . .  x . . .")  ││01 ---     ---     ---            ││  ░░    ░░    ░░   │   ██   │
│ 11 track("bass", sh101{ cutoff = 600 })            ││02 ---     ---     ---            ││  ░░    ░░    ░░   │   ██   │
│ 12   :melody("c2 . eb2 .  g1 . bb1 .")             ││03 1       ---     ---            ││  ██    ░░    ░░   │   ██   │
│ 13                                                 ││04 ---     x       eb2            ││  ██    ██    ░░   │   ██   │
│                                                    ││05▶---     ---     ---            ││  ██    ██    ░░   │   ██   │
│                                                    ││06 3       ---     ---            ││  ██    ██    ██   │   ██   │
│                                                    ││07 ---     ---     ---            ││  ██    ██    ██   │   ██   │
│                                                    ││08 1       x       g1             ││  ██    ██    ██   │   ██   │
│                                                    ││09 ---     ---     ---            ││ -6.0  -9.1   -14  │  -3.1  │
│                                                    ││10 5       ---     ---            ││ 1.00  0.90  0.80  │  lim   │
│                                                    ││11 ---     ---     ---            ││   C     C    L20  │        │
│                                                    ││12 ---     x       bb1            ││  M S   M S   M S  │        │
│                                                    ││13 3       ---     ---            ││              ▒M   │        │
│                                                    ││14 ---     ---     ---            ││                            │
│                                                    ││15 6       ---     ---            ││                            │
│                                                    │└──────────────────────────────────┘└────────────────────────────┘
│                                                    │┌ [5 fx] 6 lanes 7 scope 8 espectro · drums ─────────────────────┐
│                                                    ││ filter lowpass  cutoff ~ 2310 Hz ▕████▌     ▏ lfo 800→4000     │
│                                                    ││                 q       0.70                                   │
│                                                    ││ crush           bits   12      ▕██████▊   ▏                    │
│                                                    ││ envío delay 0.20  reverb 0.00   gain 1.00                      │
│ ⚠ l.6: cutoff fuera de rango (20-20000)            ││ ~ valor en vivo de la curva · ▕█▏ dentro de su rango           │
│ ⓘ fx(nombre, parámetros)  Efecto de la pista.      ││                                                                │
└────────────────────────────────────────────────────┘└────────────────────────────────────────────────────────────────┘
 cargado: entra en el siguiente compás
 EDITOR  Ctrl-S guardar y cargar · Ctrl-Espacio marcha/paro · Ctrl-G error · Esc tocar · F1 paleta · F10 menú
```

### Ancho (180×50 o más)

Tres columnas: código, sonido (onda, tracker y lanes) y control
(mezclador, scope y espectro del master, referencia de teclas). Los
efectos van a lo ancho abajo. Lo que en 120 son pestañas del dock aquí
se ve a la vez si cabe. Cuando no cabe, vuelve al dock.

```text
 Archivo  Editar  Pista  Transporte  Ver  Ventanas  Ayuda                                                                                               F10 menú · Alt-1…8 foco
 ▶ en marcha   170.0 BPM   compás 3 · paso 05   ⏮ cue 1   variante A   ● grabando drums   ranura 2   ⚠ 1   ⇄ link 3 pares        master ▇▇▇▇▇▇▇▆░░░░  -3.1 dB  pk -1.2
 ▰▰▰▰▰▱▱▱▱▱▱▱▱▱▱▱   intro 4 │ ▶ drop 3/8 ⟳ │ break 4 │ outro 8
┌ 1 código · song.lua• · vim NORMAL ───────────────────────┐┌ 2 onda · drums · amen.wav · 1.82 s · 8 cortes · ▶3 ────────────────┐┌ 4 mezcla · grupos ─────────────────────────────┐
│  1 bpm(170)                                              ││ 1        2        3          4        5         6       7      8   ││ drums kick  bass  hats   pad   bat  │ master   │
│  2 key("c", "minor")                                     ││ ▁▃▇█▆▄▂▁  ▂▅▆▃▂▁   ▃▆██▇▅▃▂▁  ▂▄▅▃▁    ▃▇█▆▄▂▁   ▂▄▆▄▂   ▁▃▅▃▁  ▂▃▂││  ░░    ░░    ░░    ░░    ░░    ░░   │   ░░     │
│  3 local amen = sample("amen.wav"):slice(8)              ││ ▃▆███▇▅▃  ▄▇██▅▃   ▅████▇▆▄▃  ▄▆▇▅▃    ▅███▇▅▃   ▄▆█▆▄   ▃▅▇▅▃  ▄▅▄││  ░░    ░░    ░░    ░░    ░░    ░░   │   ██     │
│  4                                                       ││ ▔▀█▀▀▔▔▔  ▔▀▀▀▔▔   ▔▀██▀▀▔▔▔  ▔▀▀▀▔    ▔▀█▀▀▔▔   ▔▀▀▀▔   ▔▀▀▀▔  ▔▀▔││  ░░    ░░    ░░    ░░    ░░    ██   │   ██     │
│  5 track("drums", amen)                                  ││ │        │        ┃━━━━━━━━━━│        │         │       │      │   ││  ██    ░░    ░░    ░░    ░░    ██   │   ██     │
│  6   :steps("1 . . 1  . . 3 .  1 . 5 .  . 3 . 6")        ││ 0 ms     228      456        684      912       1140    1368   1596││  ██    ██    ░░    ░░    ░░    ██   │   ██     │
│  7   :fx("filter", { type = "lowpass", cutoff = lfo(800, ││ ▶3 «snare» · C#4 · dur 1/8 trim · rev · 456-684 ms · zoom 1×       ││  ██    ██    ░░    ██    ░░    ██   │   ██     │
│  8   :every(4, function(t) t:reverse() end)              │└────────────────────────────────────────────────────────────────────┘│  ██    ██    ██    ██    ░░    ██   │   ██     │
│  9                                                       │┌ 3 tracker · compás 3 · drop 3/8 ───────────────────────────────────┐│  ██    ██    ██    ██    ██    ██   │   ██     │
│ 10 track("kick", amen:slice_at(1))                       ││   drums       kick        bass        hats        pad              ││  ██    ██    ██    ██    ██    ██   │   ██     │
│ 11   :steps("x . . .  x . . .  x . . .  x . . .")        ││00 1           x           c2          h           c4 eb4 g4        ││ -6.0  -9.1   -14   -11   -20  -4.2  │  -3.1    │
│ 12                                                       ││01 ---         ---         ---         ---         ---              ││   C     C    L20   R10    C     C   │          │
│ 13 track("bass", sh101{ cutoff = 600 })                  ││02 ---         ---         ---         ---         ---              ││  M S   M S   M S   M S   M S   M S  │  lim     │
│ 14   :melody("c2 . eb2 .  g1 . bb1 .")                   ││03 1           ---         ---         h           ---              ││                    ▒M               │          │
│ 15   :auto("cutoff", { 0, 0.2, 1, 0.4 }, { len = 4 })    ││04 ---         x           eb2         h           ---              │└────────────────────────────────────────────────┘
│ 16   :spectrum{ inline = true }                          ││05▶---         ---         ---         ---         ---              │┌ 7 scope · master ──────────────────────────────┐
│     ⢀⣀⣤⣶⣿⣿⣷⣦⣤⣀⡀⢀⣀⣤⣤⣀⡀  ⢀⣀⡀   ⢀⡀                          ││06 3{lp=800}   ---         ---         h           ---              ││ ⠀⢀⣀⡀⠀⠀⢀⣴⣿⣦⡀⠀⠀⢀⣀⡀⠀⠀⣠⣾⣷⣄⠀⠀⢀⣀⡀⠀⠀⢀⣴⣿⣦⡀⠀⠀⢀⣀⡀⠀⠀⣠⣾    │
│ 17                                                       ││07 ---         ---         ---         ---         ---              ││ ⠉⠉⠁⠈⠉⠉⠉⠁⠈⠛⠋⠁⠈⠉⠉⠉⠁⠈⠙⠛⠛⠋⠁⠈⠉⠉⠉⠁⠈⠛⠋⠁⠈⠉⠉⠉⠁⠈⠙⠛       │
│ 18 group("bat", { "drums", "kick" })                     ││08 1           x           g1          h           ---              ││                                                │
│                                                          ││09 ---         ---         ---         ---         ---              ││ pk -1.2 dB · ≈ -9.8 LUFS (aprox.)              │
│                                                          ││10 5           ---         ---         h           ---              │└────────────────────────────────────────────────┘
│                                                          ││11 ---         ---         ---         ---         ---              │┌ 8 espectro · master ───────────────────────────┐
│                                                          ││12 ---         x           bb1         h           ---              ││ ▂▄▆█▇▆▅▄▄▃▃▃▂▂▃▄▃▂▂▂▁▁▁▁▁▁▁                    │
│                                                          ││13 3           ---         ---         h           ---              ││ ██████████▇▇▆▆▆▇▆▆▅▅▄▃▃▂▂▁▁▁▁                  │
│                                                          ││14 ---         ---         ---         ---         ---              ││ 50   100    500   1k    5k   10k  20k          │
│                                                          ││15 6!          ---         ---         h           ---              │└────────────────────────────────────────────────┘
│                                                          │└────────────────────────────────────────────────────────────────────┘┌ referencia · EDITOR ───────────────────────────┐
│                                                          │┌ 6 lanes · bass · cutoff 0.27 → 1340 Hz ────────────────────────────┐│ Ctrl-S   guardar y cargar                      │
│                                                          ││ 1.0 ┤               ╭──────╮                                       ││ Ctrl-L   cargar sin guardar                    │
│                                                          ││     │          ╭────╯      ╰──╮                                    ││ Ctrl-Esp marcha/paro      F9 cue               │
│                                                          ││     │     ╭────╯   ▲           ╰────╮                              ││ Ctrl-G   ir al error  Alt-G línea              │
│                                                          ││ 0.0 ┤─────╯        ┊                ╰──                            ││ Ctrl-F   buscar  F6/F7 sig./ant.               │
│                                                          ││     └ compás 1 ──── 2 ──── 3 ──── 4 ┘  loop · smooth               ││ F4       completar                             │
│                                                          ││ ←→ paso · ↑↓ valor · c curva · Supr borra · Tab siguiente          ││ Esc      modo tocar                            │
│                                                          ││                                                                    ││ Alt-1…8  foco · F10 menú · F1 paleta           │
│ ⓘ auto(parámetro, puntos, opciones)  Lane.               ││                                                                    ││ F12      pánico                                │
└──────────────────────────────────────────────────────────┘└────────────────────────────────────────────────────────────────────┘└────────────────────────────────────────────────┘
┌ 5 efectos por pista ─────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────┐
│ drums                                    kick                                     bass                                     pad                                                   │
│ filter lp  cutoff ~2310 Hz ▕████▌   ▏    sin efectos                              synth sh101 · cutoff 600 (lane)        reverb envío 0.35                                       │
│ envío delay 0.20  reverb 0.00            gain 0.90                                eq 3 bandas  ▁▃▆▃▁                      gain 0.50                                              │
│ vol 1.00 · pan C                         vol 0.90 · pan C                         vol 0.80 · pan L20 · * congelada                                                               │
│                                                                                                                                                                                  │
└──────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────┘
 cargado: entra en el siguiente compás
 EDITOR  Ctrl-S guardar y cargar · Ctrl-L cargar · Ctrl-Espacio marcha/paro · Ctrl-G ir al error · Esc tocar · F1 paleta · F4 completar · F10 menú · Ctrl-Q salir
```

### Pequeño y degradado (60×16, ASCII)

Por debajo de 80×24 se quitan cosas por orden: primero la barra de menús
(F10 la sigue abriendo), luego la regla, la línea ⓘ, la referencia y por
último las pestañas (el panel derecho muestra el que tenga el foco). Con
`ascii` (o sin UTF-8), los bordes, la onda y los medidores se dibujan con
`+-|`, `.:#` y `#-`.

```text
> 170.0 BPM 3.2.05 A song.lua* !1     master ######-- -3.1
+- 1 codigo ---------------------++- 2 onda drums >3 ------+
| 5   :steps("1 . . 1  . . 3 .   ||1  2  3  4  5  6  7  8  |
| 6 track("kick", amen:slice_at  ||.:#:. .:. :##:. .: :#:. |
| 7   :steps("x . . .  x . . ."  |+-[3 tracker] 4 5 6 7 ---+
| 8 track("bass", sh101{})       ||04 1     x     c2       |
| 9   :melody("c2 . eb2 .")      ||05>---   ---   ---      |
|10                              ||06 3     ---   eb2      |
|                                ||07 ---   ---   ---      |
|                                ||08 1     x     ---      |
|                                ||09 ---   ---   ---      |
|                                |+------------------------+
cargado: entra en el siguiente compas
EDITOR Ctrl-S cargar Ctrl-Espacio marcha Esc tocar F10 menu
```

Por debajo de **40×10** solo se dibuja una línea con «terminal demasiado
pequeña (N×M) · el audio sigue» y el transporte. La entrada sigue
funcionando (Ctrl-Espacio, F12, Ctrl-Q).

### Sin editor (editor en su ventana)

Es la misma rejilla sin la columna del código: la onda se agranda y el
tracker usa la vista completa (`FullTrackerPanel`). La **ventana del
editor** gana la cabecera, la regla y el pie de referencia, y puede abrir
la barra de menús. Sus entradas se mandan a la principal con la orden
`accion` que ya existe. La cabecera de esa ventana necesita los avisos, la
variante y el master, que `RemoteState` no lleva hoy (lleva marcha, bpm,
compás, paso, pista, corte y cue). Eso obliga a ampliar el protocolo a
la **versión 8**, con campos que se pueden omitir para que una ventana
vieja siga entendiéndose (como en las versiones anteriores).

## 4. Cómo se ven los elementos sonoros

- **Onda del sample con los cortes** (`2 onda`): fila de números de
  corte, onda en barras (o media celda en ASCII), una fila de marcas en la
  que el corte que suena va en `┃` con su tramo en `━`, la fila de los ms
  y una línea con el corte elegido: número, etiqueta, nota (`cortes:note`),
  variantes (`dur`, `fit`, `rev`, `warp`, `smear`, `formant`) y zoom. Todo
  sale de lo que ya calcula `SamplePanel` y `BigSamplePanel`. En una pista
  de synth sale el panel del synth (`synth_panel`), como hoy.
- **Cortes y sus variantes**: en la línea del corte elegido y en el
  tracker. Las llaves del paso (`3{lp=800}`, `6!`) salen tal cual en su
  celda, recortadas con `…` si no caben.
- **Tracker** (`3`): igual que hoy, más el número de filas ocultas (`▲2`
  o `▼3`) cuando se desplaza. Las columnas son pistas y, si no caben, se
  desplazan siguiendo a la pista elegida.
- **Pistas y grupos**: la línea por pista del panel «motor» desaparece.
  Su contenido se reparte entre el mezclador (nivel, mute y solo, congelada
  y ranura) y la cabecera del tracker (corte que suena, ciclo y longitud
  si no es 16). Los grupos de `group()` salen como tiras en el mezclador,
  después de las pistas y con su nombre.
- **Mezclador** (`4`): tiras verticales con su medidor (como hoy en la
  ventana sin editor), dB, volumen, pan, `M S` y el `▒` del mute en vivo.
  Al final va el master con el limitador. Con poco ancho cada tira se
  reduce a una columna de medidor y su inicial.
- **Lanes** (`6`): el panel actual, con su cursor, el valor en vivo y su
  unidad. En el dock o, si hay ancho, bajo el tracker.
- **Efectos** (`5`): el panel actual, con `~` y el valor en vivo de lo
  automatizado. A lo ancho en el diseño ancho y como pestaña en los demás.
- **Scope y espectro** (`7`, `8`): los widgets de `viz` que ya existen.
  En el dock muestran el master o la pista elegida. **Para eso la interfaz
  tiene que poder encender una fuente sin que el script la pida**, sumando
  su máscara a la del script y mandando `Command::SetViz`. Esa orden ya
  existe, no reserva memoria y solo hace que el motor copie a una cola que
  ya existe. No cambia el audio (golden intacto), pero es una orden nueva
  que manda la interfaz. Lo anoto por si se prefiere que todo pase por el
  lote.
- **Transporte**: cabecera (`▶`/`■`, BPM, `compás.paso` con el compás de
  la sección si la hay, cue, variante, grabación, ranura, avisos, link,
  pánico) y regla de 16 pasos (`▰▱`, con el paso que suena encendido). Todo
  sale de `TransportView` y lo que ya hay en `ViewModel`.
- **Jerarquía visual** (temas): el panel con el foco tiene el borde y el
  número en el color de acento y en negrita. Los demás llevan el borde
  `dim`. Los valores en vivo van en el color de los medidores y los avisos
  en el de aviso. Se añaden claves `borde`, `borde_foco` y `cabecera` a
  `[colores]` con valores por defecto, sin cambiar las que existen. Sin
  truecolor se degradan como ahora (`theme::downgrade`).

## 5. Navegación

- **Foco explícito**: un `Focus` en `App` con Código, Onda, Tracker,
  Mezcla, Efectos, Lanes, Scope/Espectro y Ocupación/Roll. El foco **es**
  lo que hoy son los modos y los cursores, con otro nombre:
  - Código es `Mode::Edit`.
  - Onda es el modo tocar (los atajos de cortes 1-0 y q-i, `n`, Supr,
    zoom).
  - Tracker es `tracker_edit` y Lanes es el punto de una lane.
  - Ocupación y Roll son sus cursores.
  - Mezcla y Efectos son nuevos, pero solo para navegar: ↑↓ y ←→ eligen
    pista o efecto, y m y s hacen mute y solo como en tocar. No escriben
    nada en el código. Editar en estos dos paneles queda fuera del
    rediseño.

  Así no se duplica el estado: `Focus` se deriva del modo y los cursores, y
  cambiar el foco abre o cierra lo mismo que hoy abren la paleta o el clic.
- **Teclas de foco**: **Alt-1…8** van directas a cada panel (están libres:
  el editor usa Alt con letras, `<`, `>`, `[`, `]` y flechas, y nosotros
  Alt-G). En un panel, **Esc** vuelve a donde se estaba, como ahora (el
  cursor se cierra y queda el modo de antes). En Código, Esc sigue yendo al
  modo tocar. Tab **no** cambia el foco, porque lo usan el autocompletado,
  las plantillas, la ocupación y las lanes. Alt-1…8 serán configurables en
  `[teclas]` (`foco_codigo`, `foco_onda`...), porque algunas terminales
  (GNOME Terminal, Konsole) usan Alt-número para cambiar de pestaña. El
  menú Ver da la misma navegación sin atajos.
- **Menús** (F10): ←→ cambia de menú, ↑↓ elige, Enter ejecuta, Esc
  cierra, y la letra subrayada salta al menú o a la entrada. Cada entrada
  muestra su tecla, sacada de `Keymap::describe`. Los menús se construyen
  desde `palette::ACTIONS` con un campo de menú nuevo, y un test comprueba
  que **toda acción de la paleta está en algún menú**. Mientras haya un
  menú abierto, la entrada es suya, igual que con los popups (va en la
  misma cadena de `App::handle`, después del pánico).
- **Ratón**: un clic en el título o dentro de un panel le da el foco, y
  luego cada panel hace lo que hace hoy (tracker, ocupación y mezclador
  con `hit`, onda con sus marcas, roll, lanes y editor). Un clic en una
  pestaña la elige y un clic en la barra abre el menú. Al quitar los
  `covers`, el ratón llega a todo lo que se ve. Los `*Layout` siguen siendo
  la única geometría: lo que se ve y lo que se pulsa salen del mismo
  cálculo.
- **Referencia visible**: la última línea muestra siempre las teclas del
  foco (`EDITOR ...`, `TRACKER ...`). En el diseño ancho también va la
  columna «referencia» con todas. F1 (paleta) no cambia.

## 6. Qué cambia, qué se mantiene y qué se rompe

**Se mantiene**: todas las teclas de `BINDABLE` y sus valores por
defecto, F1 paleta, F2 órdenes, F4 completar, F12 pánico, Esc
editor↔tocar, Ctrl-O pista, las teclas de cada cursor y de cada popup, el
modo vim, la API de Lua (no cambia nada en `script/src/api.rs`), los
protocolos de ventana (el 8 solo añade campos), los popups y el
`TestBackend` de las pantallas.

**Cambia**:
- Desaparece el panel «motor». Su contenido pasa a la cabecera, el
  mezclador y el tracker.
- Las visualizaciones del panel, el sidechain y las lanes dejan de
  dibujarse encima y pasan a pestañas del dock o a su columna. Las
  `inline` siguen junto al código.
- La línea de ayuda se parte en dos: mensaje y referencia.
- Se añade la barra de menús (F10) y Alt-1…8.
- La tabla de choques y la ocupación pasan al dock en la ventana con
  editor (en la ventana sin editor ya tenían sitio).

**Se rompe** (con motivo):
- **Ningún atajo existente cambia de función.** F10 y Alt-1…8 están
  libres hoy.
- Los **tests del ratón con coordenadas fijas** (`tests/mouse.rs`,
  `tests/panels_mouse.rs`, `tests/lanes.rs`, `tests/roll.rs`,
  `tests/wave_zoom.rs` y los de `app/*.rs`) cambian de coordenadas porque
  los paneles se mueven. Se reescriben para pedir la posición al `*Layout`
  correspondiente en vez de usar números, así no vuelve a pasar.
- Quien abría el sidechain o las lanes «encima» del sample los verá en
  una pestaña.

## 7. Choques con la arquitectura (hallazgos)

1. **Si el dibujo falla, hoy el programa termina** y con él el audio:
   `terminal.draw(...)?` sale del bucle con el error, y un pánico en un
   widget (por ejemplo, un índice fuera de rango en un tamaño raro)
   desenrolla el hilo principal y suelta el stream. Para cumplir «la
   interfaz nunca para el audio», el bucle tiene que **saltarse el cuadro**
   si `draw` da error (y reintentar), y el dibujo tiene que ir dentro de
   `catch_unwind`. Si un cuadro entra en pánico, se apunta el mensaje y se
   dibuja la pantalla mínima. Esto entra en la primera parte.
2. **Scope y espectro en el dock sin pedirlos en Lua**: hace falta que la
   interfaz mande `SetViz` (ver la sección 4). No toca el hilo de audio.
3. **Sonoridad (LUFS)**: el motor no la calcula y la cola del master es
   mono. Solo cabe una aproximación en la interfaz o no mostrarla. No se
   añade publicación nueva al motor. Si más adelante se quisiera de verdad,
   sería una cola estéreo nueva y entraría en `no_alloc.rs`.
4. **Protocolo de la ventana del editor**: la versión 8, con campos
   opcionales (sección 3).
5. **Alt-número** en algunas terminales es de la terminal. Se resuelve con
   las teclas configurables y el menú Ver.

No hace falta tocar el motor en ningún punto: no hay publicación nueva, y
`no_alloc.rs` y `golden.rs` quedan como están.

## 8. Plan de commits

Cada parte en un commit, con sus tests de pantalla (`TestBackend` a
80×24, 120×40, 180×50, 60×16 ASCII y 39×9) y la guía al día en el mismo
commit cuando cambie algo visible.

1. **Rejilla y temas**: `ui::layout` puro (tamaño a rectángulos por
   panel, sin dibujar) con tests por tamaño, `Focus` derivado y bordes
   según el foco, más las claves `borde`, `borde_foco` y `cabecera`.
   También el **dibujo a prueba de fallos** (punto 7.1). Las funciones no
   cambian todavía.
2. **Cabecera, regla, pie y referencia de teclas**: fuera el panel
   «motor» y la línea de ayuda partida en dos.
3. **Menús (F10)** desde la paleta, con el test de que toda acción está
   en un menú.
4. **Paneles de sonido en la nueva disposición**: onda, tracker con `▲▼`,
   mezclador con grupos, el dock con efectos, lanes, scope, espectro,
   sidechain, ocupación y choques, y `SetViz` desde la interfaz. Se quitan
   los `covers`.
5. **Foco, navegación y ratón**: Alt-1…8 configurables, clic para el foco,
   pestañas y menús con el ratón, y los tests del ratón pasados a
   coordenadas del `*Layout`. Tests de foco: cada Alt-n, Esc desde cada
   panel y que el foco de Tracker, Lanes o Roll es su cursor.
6. **Responsive y degradación**: el orden de lo que se quita, la pantalla
   mínima, ASCII y 256 colores.
7. **Ventana del editor** con cabecera, pie y menús (protocolo 8).
8. **Docs**: GUIA (pantallas en texto de los tres tamaños, menús, foco),
   ESTADO, ROADMAP, la línea en CLAUDE.md, las tablas de la paleta y de la
   configuración (`ACTUALIZAR_GUIA=1`), y el tutorial si sus capturas o
   sus pasos nombran el panel «motor».

## 9. Riesgos

- **Tamaño del cambio en `ui.rs`** (3141 líneas) y `app.rs`: para
  limitarlo, cada panel conserva su `Widget` y solo cambia dónde se
  coloca. La rejilla nueva es una función pura y testeable aparte.
- **Coste del dibujo** con tres columnas y dos visualizaciones a 30 Hz en
  la laptop vieja. Se mide con `pantalla_*` y un test de tiempo por cuadro.
  Si hace falta, las visualizaciones del dock bajan a la mitad del
  refresco.
- **Alt-número capturado por la terminal o por tmux** (ver la sección 5).
- **La guía y el tutorial** describen dónde está cada cosa y el test de la
  guía comprueba enlaces y bloques, no capturas. Hay que revisar a mano
  las frases sobre la posición de los paneles.
- **Hábito**: quien usa el sidechain o las lanes encima de la onda cambia
  de sitio. Se mitiga con el menú Ver y con que la paleta siga abriéndolos
  igual.

## 10. Qué tendrá que mirar el usuario (tras implementar)

- Cambiar el tamaño del terminal en vivo de ancho a 120×40, 80×24 y
  menos, con el audio sonando. No debe cortarse ni oírse nada raro.
- Navegar solo con el teclado: F10 por todos los menús, Alt-1…8 y Esc
  desde cada panel. Comprobar dentro de tmux y en su terminal si Alt-número
  llega.
- El ratón en cada panel y en las pestañas y la barra.
- Editar Lua con el audio sonando y guardar con errores: sigue lo
  anterior y el aviso sale en la cabecera y en el editor.
- `LANG=C` y una terminal sin truecolor.
- El editor en su ventana (`--split`) con los menús.

## 11. Lo que cambió al implementarlo

- **Nueve teclas de foco** (Alt-1…9): la ocupación tiene la suya (9). El
  panel de la onda se llama «sample», como su título de siempre.
- **Las pestañas van en una fila propia** encima de su panel, no dentro de
  su borde: así cada panel conserva su título.
- **El dock crece** hasta lo que pide la pestaña que se ve, apretando la
  onda y luego el tracker.
- **RMS en vez de LUFS** en el scope del master: la cola es mono y sin
  ponderar, y llamarlo LUFS engañaría.
- **El editor ocupa la mitad** en el tamaño mediano (no nueve vigésimos):
  con menos, las líneas largas y la ayuda se cortaban.
- **Los grupos no tienen tira** en el mezclador: siguen en el panel de
  efectos.
- **El primer clic en el editor** sin el foco solo se lo devuelve, sin
  mover el cursor (desde el modo tocar no se edita sin querer).
- **Las pestañas que no caben** se acortan y la fila se desplaza hasta la
  elegida.

