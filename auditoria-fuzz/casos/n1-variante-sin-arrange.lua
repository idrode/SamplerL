bpm(170)
local amen = sample("amen.wav"):slice(8)
track("drums", amen):steps("1 . 2 .  3 . 4 .  5 6 2 .  7 . 8 .")
section("drop", function() end)
variant("drop", "B", function()
  track("kick", amen:slice_at(1)):steps("x . . .  x . . .  x . . .  x . . .")
end)
-- arrange({ { "drop", 4 } })   <- comentado o todavía sin escribir
