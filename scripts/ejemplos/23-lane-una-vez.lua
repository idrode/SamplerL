-- 23-lane-una-vez.lua
bpm(170)
local amen = sample("amen.wav"):slice(8)
track("drums", amen)
  :steps("1 . 2 .  3 . 4 .  5 6 2 .  7 . 8 .")
  :fx("highpass", { cutoff = 40 })
  -- Una subida que pasa una vez a los 4 compases y vuelve a 40 Hz.
  :auto("cutoff", { {0, 0}, {2, 1, "exp"} }, { len = 2, offset = 4, mode = "once", range = {40, 2000}, scale = "log" })
  -- El bend de cada golpe baja de 0 a -12 a lo largo de 8 compases y se queda ahí.
  :auto("bend", { {0, 0}, {8, -12} }, { len = 8, mode = "hold" })
