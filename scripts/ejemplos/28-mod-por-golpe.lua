-- 28-mod-por-golpe.lua
bpm(170)
local amen = sample("amen.wav"):slice(8)
amen:name(3, "snare")
-- Cada golpe se cierra solo y el tercero de cada tres suena más.
mod("pluck", {
  vol = { { "ahdsr", a = 2, d = 180, s = 0.2, r = 60 },
          { "stepper", values = { 0.6, 0.6, 1 } } },
  cutoff = { input = 0.7, { "velocity", min = 0.7, max = 1 } },
})
-- La caja cae una quinta al sonar.
mod("drop", { pitch = { "fader", from = 0, to = -1, time = 120 }, range = 7 })
-- Un trino del filtro, solo en un paso.
mod("wobble", { cutoff = { input = 0.6, { "lfo", sync = "1/16", amount = 0.3, op = "add" } } })
amen:mod("snare", "drop")
track("drums", amen)
  :steps("1 . 2 .  snare . 4 .  5 6 2 .  7 . snare 8{mod=wobble}")
  :mod("pluck")
  :gain(0.8)
-- En un sintetizador, el mismo set abre y cierra su filtro con cada nota.
mod("bark", { cutoff = { { "ahdsr", a = 1, d = 150, s = 0.2 },
                         { "velocity", min = 0.6, max = 1 } } })
track("bajo", synth("acid"))
  :melody("c . c! .  eb . c .  g . c! .  bb . c .")
  :mod("bark")
  :gain(0.6)
