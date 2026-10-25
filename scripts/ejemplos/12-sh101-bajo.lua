-- 12-sh101-bajo.lua
bpm(124)
local amen = sample("amen.wav"):slice(8)
track("drums", amen)
  :steps("1 . . .  3 . . .  1 . 1 .  3 . . .")
  :gain(0.6)

-- Sierra y algo de pulso, el sub una octava abajo, resonancia alta y la
-- envolvente sin sustain: cada nota es un golpe que se cierra. Las notas
-- con / se ligan y deslizan (glide_mode = "auto").
local bass = sh101{
  saw = 0.8, pulse = 0.3, pw = 0.4, sub = 0.6, sub_div = 2,
  cutoff = 500, res = 0.7, env_amt = 0.5,
  a = 2, d = 250, s = 0, r = 100,
  glide = 80, glide_mode = "auto",
}
track("bass", bass)
  :melody("c . c c/  eb . c . g1 . bb1 c/  . c eb f")
  :gain(1.1)
