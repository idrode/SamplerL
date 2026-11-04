-- 21-lane-barrido.lua
bpm(170)
local amen = sample("amen.wav"):slice(8)
-- Un patrón de 3 compases y un barrido de 4 que se repite: cada vuelta
-- del barrido cae en otro sitio del patrón (polirritmo de automatización).
track("drums", amen)
  :length(48)
  :steps(string.rep("1 . 2 .  3 . 4 .  5 6 2 .  7 . 8 .  ", 3))
  :fx("lowpass", { cutoff = 3000, q = 2 })
  :auto("cutoff", { {0, 200, "exp"}, {3, 9000, "exp"}, {4, 200} }, { len = "4 compases" })
