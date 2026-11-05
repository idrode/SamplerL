-- Como cancion.lua, con el scope y el espectro de cada pista pedidos en
-- Lua además de los del master (scripts/medir-consumo.py).
bpm(170)

local amen = sample("../../renders/synth_break.wav"):slice(8)

track("drums", amen)
  :steps("1 . . 1  . . 3 .  1 . 5 .  . 3' . 6")
  :gain(0.7)
  :scope()
  :spectrum()

local grave = sh101{
  saw = 0.7, pulse = 0.3, sub = 0.6, sub_div = 2,
  cutoff = 400, res = 0.5, env_amt = 0.4,
  a = 3, d = 250, s = 0.6, r = 120,
  glide = 60, glide_mode = "auto",
}

track("bajo", grave)
  :melody("c2 _ _ _  . . c2 .  eb2 _ _ .  f2 _ g2/ _")
  :gain(0.6)
  :scope()
  :spectrum()
