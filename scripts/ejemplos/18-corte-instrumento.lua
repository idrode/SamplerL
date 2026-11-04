-- 18-corte-instrumento.lua
bpm(120)
local amen = sample("amen.wav"):slice(8)
-- Con una voz o un bajo, quita root: lo detecta.
local caja = amen:instrument(3, { root = "C3", voices = 4, adsr = { 2, 60, 0.7, 120 } })
local estirada = amen:instrument(3, { root = "C3", keep = true })
track("caja", caja):melody("c3 . e3 .  g3 . c3+e3+g3 _  . . a2 .  g2 . . .")
track("lenta", estirada):melody(". . . .  . . . .  c2 _ _ _  g2 _ _ _")
