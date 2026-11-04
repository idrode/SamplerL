-- 24-viz-espectro.lua
bpm(170)
local amen = sample("amen.wav"):slice(8)
-- Un barrido del filtro de 4 compases, y el espectro del master para
-- verlo subir y bajar.
track("drums", amen)
  :steps("1 . 2 .  3 . 4 .  5 6 2 .  7 . 8 .")
  :fx("lowpass", { cutoff = 3000, q = 3 })
  :auto("cutoff", { {0, 200, "exp"}, {2, 12000, "exp"}, {4, 200} }, { len = 4 })
master():spectrum{ size = 8, bands = 48 }
