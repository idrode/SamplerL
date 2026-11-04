-- 17-corte-duraciones.lua
bpm(170)
local amen = sample("amen.wav"):slice(8)
-- La caja corta, el bombo estirado a una negra y el charles sostenido.
amen:set(3, { dur = "1/16", release = 20 })
amen:set(1, { dur = "1/4", fit = "stretch" })
amen:set(2, { dur = 300, fit = "loop", loop_start = 20, loop_end = 60 })
track("drums", amen)
  -- El paso 15 alarga su caja: dos corcheas en bucle.
  :steps("1 . 2 .  3 . 4 .  5 6 2 .  7 . 3{dur=1/4,fit=loop} .")
