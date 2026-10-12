-- Canción de ejemplo con la API de Lua. Usa el break sintético que genera
-- scripts/render-demo.sh (o `cargo run -p engine --example synth_break --
-- renders/synth_break.wav`). Renderizar:
--   cargo run -p tui -- --script scripts/demo.lua --render renders/demo-lua.wav --bars 8

bpm(170)

local amen = sample("../renders/synth_break.wav"):slice(8)

track("drums", amen)
  :steps("1 . . 1  . . 3 .  1 . 5 .  . 3' . 6")
  :fx("filter", { type = "lowpass", cutoff = 6000 })
  -- Cada 4 compases (0, 4, 8...), el compás va al revés.
  :every(4, function(t) t:reverse() end)
  -- Algún golpe suelto con stutter.
  :chance(0.15, "retrig", 4)
  :gain(0.7)

-- Charles agudos en otra pista, más bajos y con algo de bitcrusher.
track("hats", amen:slice_at(2))
  :steps(". . x^12 .  . . x^12 .  . . x^12 .  . . x^12 x^12r2")
  :fx("bitcrush", { bits = 6, downsample = 2 })
  :gain(0.3)

-- Bombo reforzado en los tiempos.
track("kick", amen:slice_at(1))
  :steps("x . . .  x . . .  x . . .  x . . .")
  :gain(0.3)
