-- 02-variacion-every.lua
bpm(170)
local amen = sample("amen.wav"):slice(8)

track("drums", amen)
  :steps("1 . 2 .  3 . 4 .  5 6 2 .  7 . 8 .")
  -- Compases 0, 4, 8...: otro patrón, más cargado al final.
  :every(4, function(c) c:steps("1 . 2 .  3 . 4 .  5 6 3 3  7 3 7 3") end)
  -- Compases 0, 8, 16...: además, todo al revés.
  :every(8, function(c) c:reverse() end)
