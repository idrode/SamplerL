-- Canción de ejemplo con la API de Lua. Usa el break sintético que genera
-- scripts/render-demo.sh (o `cargo run -p engine --example synth_break --
-- renders/synth_break.wav`). Renderizar:
--   cargo run -p tui -- --script scripts/demo.lua --render renders/demo-lua.wav --bars 4

bpm(170)

local amen = sample("../renders/synth_break.wav"):slice(8)

track("drums", amen)
  :steps("1 . . 1  . . 3 .  1 . 5 .  . 3' . 6")

-- Segunda pista con el mismo sample: charles agudos y más bajos.
track("hats", amen)
  :steps(". . 2^12@0.25 .  . . 2^12@0.25 .  . . 2^12@0.25 .  . . 8@0.3 2^12@0.25")
