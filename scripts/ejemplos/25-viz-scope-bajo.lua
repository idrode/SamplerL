-- 25-viz-scope-bajo.lua
bpm(120)
-- La forma de onda del bajo: el disparo la deja quieta en cada nota.
track("bajo", sh101{ saw = 1, cutoff = 900, res = 0.4 })
  :melody("c2 _ . c2  eb2 _ g1 .  c2 _ . bb1  . g1 c2 .")
  :scope{ size = 6 }
  :pitchwheel{ size = 6 }
