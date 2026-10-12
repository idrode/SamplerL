-- 03-ghost-notes.lua
bpm(170)
local amen = sample("amen.wav"):slice(8)

track("drums", amen)
  :steps("1 . 2 .  3 . 4 .  5 6 2 .  7 . 8 .")
  :gain(0.8)

track("ghost", amen:slice_at(3))
  :steps(". x@0.2 . x@0.15  . x@0.2 x@0.1 .  . x@0.2 . x@0.15  . . x@0.2 x@0.1")
  :chance(0.4, "mute")
  :fx("lowpass", { cutoff = 4000 })
