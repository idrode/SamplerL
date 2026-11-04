-- 26-viz-punchcard.lua
bpm(170)
local amen = sample("amen.wav"):slice(8)
track("drums", amen)
  :steps("1 . 2 .  3 . 4 .  5 6 2 .  7 . 8 .")
  :every(4, function(c) c:reverse() end)
  -- Los golpes de la batería, con el cursor (y lo que cambia cada 4
  -- compases); y un scope pequeño junto a esta línea.
  :punchcard{ size = 8 }
  :scope{ size = 2, inline = true }
