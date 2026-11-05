-- Canción de la medición de consumo (scripts/medir-consumo.py): un break
-- y un bajo, sin visualizaciones pedidas en Lua (el scope y el espectro
-- del master salen solos en la interfaz si caben).
bpm(170)

local amen = sample("../../renders/synth_break.wav"):slice(8)

track("drums", amen)
  :steps("1 . . 1  . . 3 .  1 . 5 .  . 3' . 6")
  :gain(0.7)

local grave = sh101{
  saw = 0.7, pulse = 0.3, sub = 0.6, sub_div = 2,
  cutoff = 400, res = 0.5, env_amt = 0.4,
  a = 3, d = 250, s = 0.6, r = 120,
  glide = 60, glide_mode = "auto",
}

track("bajo", grave)
  :melody("c2 _ _ _  . . c2 .  eb2 _ _ .  f2 _ g2/ _")
  :gain(0.6)
