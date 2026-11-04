-- 22-lane-pan-y-saltos.lua
bpm(170)
local amen = sample("amen.wav"):slice(8)
track("drums", amen)
  :steps("1 . 2 .  3 . 4 .  5 6 2 .  7 . 8 .")
  -- El pan va y viene, suave, en 2 compases.
  :auto("pan", { {0, -0.7, "smooth"}, {1, 0.7, "smooth"}, {2, -0.7} }, { len = 2 })
  -- El volumen salta cada negra: el suavizado de 5 ms evita los clics.
  :auto("gain", { {0, 0.3, "step"}, {0.25, 1, "step"}, {0.5, 0.5, "step"}, {0.75, 1, "step"} }, { len = 1 })
