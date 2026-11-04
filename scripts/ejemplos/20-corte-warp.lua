-- 20-corte-warp.lua
bpm(170)
local amen = sample("amen.wav"):slice(8)
amen:set(3, { warp = 0.3 })
amen:set(7, { warp = { {0.25, 0.4}, {0.5, 0.45}, {0.75, 0.9} }, smear = 0.3 })
amen:set(2, { formant = 5 })
track("drums", amen)
  :steps("1 . 2 .  3 . 4 .  5 6 2 .  7 . 3{smear=0.8} .")
