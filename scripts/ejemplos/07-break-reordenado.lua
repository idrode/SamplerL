-- 07-break-reordenado.lua
bpm(172)
local amen = sample("amen.wav"):slice(8)

track("drums", amen)
  :steps("1 . 4 .  7 . 2 3  . 5 6 .  3 . 8 7")
  :every(2, function(c) c:steps("5 . 1 .  3 . 2 .  1 6 7 .  3 4 3 8") end)
