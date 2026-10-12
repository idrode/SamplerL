-- 08-varias-pistas.lua
bpm(170)
local amen = sample("amen.wav"):slice(8)

track("drums", amen)
  :steps("1 . 2 .  3 . 4 .  5 6 2 .  7 . 8 .")
  :fx("filter", { type = "lowpass", cutoff = 6000 })
  :every(4, function(c) c:reverse() end)
  :gain(0.7)

track("kick", amen:slice_at(1))
  :steps("x . . .  . . . .  x . x .  . . . .")
  :gain(0.4)

track("hats", amen:slice_at(2))
  :steps(". . x^12 .  . . x^12 .  . . x^12 .  . . x^12 x^12r2")
  :fx("highpass", { cutoff = 2000 })
  :fx("bitcrush", { bits = 6, downsample = 2 })
  :gain(0.3)
