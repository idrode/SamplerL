-- 10-subgrave.lua
bpm(170)
local amen = sample("amen.wav"):slice(8)

track("kick", amen:slice_at(1))
  :steps("x . . .  . . . .  x . x .  . . . .")
  :gain(0.4)

track("drums", amen)
  :steps(". . 2 .  3 . 4 .  . 6 2 .  7 . 8 .")
  :gain(0.4)

-- Un subgrave en C1: notas largas, una ligada (/) y un accent (!).
local sub = synth("sub", { tone = 200, drive = 0.2, release = 120 })
track("sub", sub)
  :melody("c _ _ _  _ _ _ _  ab0 _ _ _  bb0! _ g0/ _")
  -- Deja sitio al bombo: sus graves bajan mientras suena.
  :fx("dyneq", { freq = 60, q = 0.7, range = 8, threshold = -30,
                 attack = 2, release = 90, sidechain = "kick" })
  :gain(0.7)
