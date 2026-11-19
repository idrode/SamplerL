-- 27-modulacion.lua
bpm(170)
local amen = sample("amen.wav"):slice(8)
amen:name(3, "snare")
-- El phaser también va en un corte: solo la caja barre.
amen:fx("snare", "phaser", { stages = 6, center = 1200, feedback = 0.5, mix = 0.5 })
track("drums", amen)
  :steps("1 . 2 .  snare . 4 .  5 6 2{phaser=0.7,stages=12} .  7 . snare .")
  -- Un barrido por compás, en fase con la canción.
  :fx("flanger", { sync = "1 compás", depth = 3, delay = 1, feedback = 0.6, mix = 0.5 })
  :gain(0.8)
track("pad", amen:slice_at(7))
  :steps("x _ _ _  _ _ _ _  x _ _ _  _ _ _ _")
  :fx("chorus", { voices = 3, depth = 4, rate = 0.5, mix = 0.5 })
  :gain(0.5)
