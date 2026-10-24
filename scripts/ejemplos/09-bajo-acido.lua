-- 09-bajo-acido.lua
bpm(132)
local amen = sample("amen.wav"):slice(8)
track("drums", amen)
  :steps("1 . 2 .  3 . 4 .  5 6 2 .  7 . 8 .")
  :gain(0.8)

local bass = synth("acid", {
  cutoff = 320, res = 0.82, envmod = 0.65, decay = 220, accent = 0.8,
  dist = 0.25, glide = 70,
})
track("acid", bass)
  :melody("c c! c' eb/  c . c bb1/  c! g1 . c/  eb! f/ eb c")
  :every(4, function(c) c:melody("c c! g/ c  bb1! . c c/  f! eb/ c . c! c/ eb g/") end)
  :fx("lowpass", { cutoff = lfo("8 compases", 1500, 6000) })
  :gain(0.7)
