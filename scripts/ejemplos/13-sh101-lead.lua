-- 13-sh101-lead.lua
bpm(124)
local amen = sample("amen.wav"):slice(8)
track("drums", amen)
  :steps("1 . . .  3 . . .  1 . 1 .  3 . . .")
  :gain(0.6)

-- Un lead en C4: sierra y pulso que se ensancha y se estrecha con el LFO,
-- sustain alto, el filtro que sigue al teclado y un vibrato suave. Las
-- notas ligadas (/) deslizan sin volver a atacar.
local lead = sh101{
  root = "C4",
  saw = 0.7, pulse = 0.5, pw = 0.5, pwm_src = "lfo", pwm_amt = 0.5,
  cutoff = 1800, res = 0.35, env_amt = 0.3, kbd = 0.6,
  a = 5, d = 400, s = 0.7, r = 250,
  lfo_rate = 5.5, lfo_pitch = 0.15,
  glide = 120, glide_mode = "auto",
}
track("lead", lead)
  :melody("g _ _ bb/  c5 _ _ _  bb _ g/ f  g _ _ _")
  :gain(1.1)
