-- 19-corte-bend.lua
bpm(170)
local amen = sample("amen.wav"):slice(8)
amen:set(1, { bend = -12, bend_time = 180 })
amen:set(5, { bend = ramp(0, -24, "4 compases"), bend_time = 120 })
amen:set(8, { rev = true })
track("drums", amen)
  :steps("1 . 2 .  3 . 4 .  5 6 2 .  7 . 8 1{bend=7,bend_time=40}")
