-- 14-mpe.lua
bpm(96)
local amen = sample("amen.wav"):slice(8)
track("drums", amen)
  :steps("1 . . .  3 . . .  . . 1 .  3 . . .")
  :gain(0.5)

-- Un pad de subarmónicos de seis voces para tocar con un controlador
-- MPE: cada dedo dobla su nota, la presión le da volumen y abre su
-- filtro, y deslizar hacia arriba le sube la resonancia.
local pad = subharm{
  voices = 6, root = "C3",
  o1 = { div = 1, level = 0.7 },
  s1 = { div = 2, level = 0.5 },
  cutoff = 350, res = 0.3, envmod = 0.2, attack = 40, decay = 900,
}
track("pad", pad)
  :mpe{
    bend = "pitch",
    pressure = { "cutoff", range = 3, curve = 1.5 },
    slide = { "res", range = 0.4 },
  }
  -- Un acorde largo del patrón, sin expresión, para tocar encima.
  :melody("c+eb+g _ _ _  _ _ _ _  _ _ _ _  _ _ _ _")
  :gain(0.7)
