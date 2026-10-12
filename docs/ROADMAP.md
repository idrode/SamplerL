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
