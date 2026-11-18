# Solapes de frecuencia y sidechain: lo que hay y lo que falta

Inventario de antes de empezar la continuación «solapes de frecuencia
entre pistas y sidechain» (pedida por el usuario; sin fase propia). Solo
lectura: qué existe en el código a fecha de 2026-11-18 (commit `35d705b`)
y qué falta de lo pedido. Las decisiones y el detalle de lo que se añade
van luego en «Solapes de frecuencia y sidechain» de `docs/ESTADO.md`.

## 1. `--analyze-masking` (análisis de choques)

- **Dónde**: la señal en `engine/src/masking.rs` (puro, sin motor ni Lua,
  con `rustfft`); el análisis de la canción en `script/src/masking.rs`
  (`Song::analyze_masking`); la orden en `tui/src/main.rs`
  (`masking_report`).
- **Fuente**: **render offline**. Cada pista se renderiza **en solo** en un
  motor aparte (`Song::render_track`: con sus efectos, envíos y el
  limitador) durante un ciclo entero de la canción (`length_bars(1)`, como
  mucho 256 compases) más un compás. Nada en vivo: el motor que suena no
  se toca.
- **Bandas**: 30 de **tercio de octava**, de 25 Hz a 20 kHz (centros
  `1000·10^(k/10)`, nombres nominales `BAND_NAMES`, bordes a ±1/6 de
  octava).
- **Resolución**: ventanas de Hann de 50 ms cada 25 ms, FFT rellenada a
  8192 puntos (bins de 5,9 Hz a 48 kHz; por debajo de ~60 Hz manda la
  ventana, ~20 Hz). Mono (media de L y R).
- **Métrica**: por pareja y banda, `BandClash { score, overlap, level_db }`.
  Una pista está *activa* en una banda y una ventana si no queda más de
  30 dB por debajo de su propio máximo; con las dos activas, la *cercanía*
  es `max(0, 1 − |La − Lb| / 9 dB)`; la puntuación es la media de la
  cercanía ponderada por la potencia de las dos (de 0 a 1). Bandas en las
  que una de las dos casi no suena (30 dB bajo su banda más fuerte):
  `None`. Se informa desde 0,25 (`strength`: leve, medio, fuerte).
- **Salida**: `MaskingReport` (pistas, parejas, `priority()`, sugerencias)
  con `text()` (lista de choques banda a banda, tabla pareja × banda y
  sugerencias), `clashes()` ordenados por puntuación, `summary`, `table`,
  `grid`, `winner`/`loser` (quién cede: `priority()` o la más fuerte en la
  banda) y **sugerencias de EQ** (`Suggestion`/`Cut`: recortes en campana
  de 2 a 4 dB en la pista que cede, bandas seguidas juntas, hasta 4) como
  línea de Lua (`lua_line`).
- **Lo que no tiene**: la energía de cada pista por banda (solo la de la
  pareja relativa a su banda más fuerte), los choques agrupados por rangos
  de bandas («60-120 Hz»), sugerencias de sidechain.

## 2. Los paneles

- **Choques** (pestaña del dock `Tab::Clash`, `ui::ClashPanel`): la tabla
  del **último análisis offline** pedido desde la paleta («Choques de
  frecuencia», `App::open_masking`): una fila por pareja, una columna por
  banda que choca, puntuación coloreada por gravedad; si no cabe, la lista
  de choques de más a menos. Avisa si es de una carga anterior. Sin
  teclado ni ratón propios. Además, el **popup** del informe
  (`MaskingPicker`): una fila por choque, filtrar, Enter dos veces inserta
  el EQ sugerido tras la definición de la pista (un paso de deshacer, sin
  cargar).
  **No hay matriz pista × banda ni nada en vivo.**
- **Ocupación** (`tui/src/occupancy.rs`): rejilla pista × paso de lo que
  tocará el secuenciador, huecos y sugerencias de relleno; usa la
  puntuación del análisis de choques (si lo hay) para elegir qué pista
  rellena. Nada de frecuencias propio.
- **Sidechain** (`Tab::Sidechain`, `tui/src/sidechain.rs`): **solo
  muestra**. Por cada EQ dinámico y compresor de la canción, la reducción
  en dB que publica el motor (`SharedStatus::add_reduction`, un atómico
  por pista y ranura) y el pico de la pista que lo dispara, en curvas de
  100 muestras. No procesa nada.

## 3. Sidechain en el motor

- **Procesa audio, sí, pero solo en el EQ dinámico**:
  `Fx::DynEq(DynEqParams { kind, freq_hz, q, threshold_db, ratio,
  range_db, attack_ms, release_ms, sidechain: Option<usize> })`. El
  detector es un paso de banda en la misma frecuencia sobre la señal que
  escucha (la propia o la de otra pista), envolvente de pico con ataque y
  release; la reducción mueve una campana (o un estante) en esa banda.
  Rodilla dura. **No hay ducking de la pista entera** ni disparo por
  patrón. El compresor (`Fx::Compressor`) se escucha solo a sí mismo.
- **Parámetros**: freq, q, type, threshold, ratio, range, attack,
  release, sidechain. Automatizables (`ramp`, `lfo`, `macro`, lanes)
  freq, q, threshold, range, ratio, attack y release.
- **Enrutado**: la clave es la salida de la otra pista **tras sus efectos
  y antes de su volumen, pan y mute** (sigue disparando en mute o fuera del
  solo: el «bombo fantasma»). `Engine::mix` ordena las pistas
  (`mix_order`: cada una después de las que escucha), guarda la salida de
  las escuchadas en `Engine::side_buf` (8 trozos en el heap) y `run_fx` da
  a cada efecto su trozo (`FxState::process_keyed`; silencio si la otra
  calló). Sin retraso de un trozo. Los grupos también pueden llevar un
  EQ dinámico con sidechain. No hay toma post-fader.
- **Script**: `fx("dyneq", { ..., sidechain = "kick" })`, guardado en
  `TrackDef::sidechains`; `check_sidechains` da error con la pista que no
  existe y con los **círculos** (a escucha a b y b a a); escucharse a sí
  misma, error; en un corte o en un paso, error. Congelar: la pista
  congelada lleva su sidechain ya dentro del WAV (la huella lo cuenta).
- **Golpes del secuenciador**: el bucle de `Engine::process` parte los
  trozos en cada paso y subdisparo (`step`, `fire_retrig`), así que cada
  golpe (`play_hit`) cae al principio de un trozo. Nadie lo publica a
  otras pistas.

## 4. API de Lua

- `fx("dyneq", { freq, q, type, threshold, ratio, range, attack, release,
  sidechain })` y `fx("compressor", { threshold, ratio, attack, release,
  makeup })`, registrados en `script/src/api.rs` (autocompletado, ayuda y
  guía salen de ahí).
- `priority({ ... })`: quién manda en los choques.
- No hay `pista:sidechain(...)`.

## 5. Visualización por pista (para el análisis en vivo)

- `engine/src/viz.rs`: una cola `rtrb` por pista y otra del master
  (16 384 muestras mono), reservadas al crear el motor. La de una pista
  copia su salida **tras volumen, pan y mute** solo si su bandera está
  encendida; las banderas entran por el lote (`Batch::set_viz`,
  `Command::SetViz`) al compás. Apagadas, solo se mira un `bool`.
- `tui/src/app/viz.rs`: `sync_ui_viz` une lo que pide el script y lo que
  pide la interfaz (hoy, el master mientras se ve su scope o su espectro)
  y manda un lote solo si cambia; `tick_viz` vacía las colas al historial;
  `frame_viz` calcula los espectros una vez por cuadro, a `visuales_hz`.
  `viz::spectrum::Analyzer`: FFT propia de 2048 puntos (bins de 23 Hz a
  48 kHz: los tercios de octava por debajo de ~100 Hz comparten bin).

## Lo que falta, y qué se hace

| Pedido | Estado | Qué se hace |
|---|---|---|
| Matriz pista × banda con energía y choques | falta | Matriz de los mismos 30 tercios de octava; energía por celda; choques con el criterio del análisis offline (no otro) |
| Resumen «kick y bajo: 60-120 Hz» ordenado | a medias (choques banda a banda) | Choques agrupados en rangos de bandas seguidas, ordenados por gravedad, el mismo en vivo y en `--analyze-masking` |
| Análisis en vivo en la interfaz | falta | Las colas por pista ya existen: se encienden solo con el panel visible (mismo `sync_ui_viz`), una ventana por cuadro a `visuales_hz` |
| Panel en el dock | existe (tabla offline) | Se amplía `ClashPanel`: matriz en vivo, lista con cursor, teclado, ratón, ASCII; la tabla offline sigue |
| Sugerencias insertables | EQ, en el popup | Para cada choque, sidechain y recorte de EQ como líneas de Lua; una tecla inserta (un paso de deshacer, sin cargar) |
| Sidechain por audio | solo en una banda (`dyneq`) | Efecto nuevo de ducking (pista entera o graves) con detector RMS, filtro del detector, umbral, ratio, rodilla, toma pre o post |
| Sidechain por patrón | falta | El mismo efecto disparado por los golpes del secuenciador de la otra pista |
| Banda (cruce) | falta | Solo por debajo de un cruce, con un paso bajo Linkwitz-Riley de 24 dB/oct restado |
| Ciclos, pistas que no existen | hecho para `dyneq` | Se reutiliza `check_sidechains` |
| `pista:sidechain(...)` | falta | Método nuevo, registrado; sus parámetros son de efecto, así que `ramp`, `lfo`, `macro` y lanes valen sin código aparte |
