-- 11-subharm.lua
bpm(110)
local amen = sample("amen.wav"):slice(8)
track("kick", amen:slice_at(1))
  :steps("x . . .  x . . .  x . . .  x . . .")
  :gain(0.5)

-- Tres contra cuatro: o1 suena cuatro veces por compás y su sub s1 (una
-- octava y una quinta abajo), tres; s2, dos octavas abajo, una vez al
-- compás, larga.
local sh = subharm{
  o1 = { div = 1, rdiv = 4, pan = -0.5, level = 0.8 },
  s1 = { div = 3, rdiv = 3, pan = 0.5, level = 0.7 },
  s2 = { div = 4, rdiv = 1, level = 0.5 },
  cutoff = 1100, res = 0.45, envmod = 0.3, decay = 350,
}
track("sh", sh)
  :melody("c _ _ _  _ _ _ _  _ _ _ _  _ _ _ _")
  :every(2, function(c) c:melody("ab1 _ _ _  _ _ _ _  bb1 _ _ _  _ _ _ _") end)
  :gain(0.8)
