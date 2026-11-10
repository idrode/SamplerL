bpm(120)
local malo = sample("nan48.wav")
local amen = sample("amen.wav"):slice(8)
-- El sample con un NaN suena una vez en el compás 0 y nunca más.
track("malo", malo:slice(1)):steps("1 . . . . . . . . . . . . . . ."):every(64, function(t) end)
  :fx("filter", { type = "lowpass", cutoff = 3000 })
  :send("reverb", 0.3)
track("drums", amen):steps("1 . 2 . 3 . 4 . 5 . 6 . 7 . 8 ."):send("reverb", 0.3)
