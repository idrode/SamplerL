-- 04-stutter-fills.lua
bpm(170)
local amen = sample("amen.wav"):slice(8)

track("drums", amen)
  :steps("1 . 2 .  3 . 4 .  5 6 2 .  7 . 8 .")
  :every(4, function(c) c:steps("1 . 2 .  3 . 4 .  5 6 2 .  3r2 3r4 7r8 7r16") end)
  :chance(0.1, "stutter", 3)
