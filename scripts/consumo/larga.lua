-- Canción larga para la medición de consumo (tests::consumo): el texto
-- de una canción de verdad (unas 120 líneas, ocho pistas), para lo que
-- la interfaz hace con el texto en cada cuadro.
bpm(170)

local amen = sample("../../renders/synth_break.wav"):slice(8)

-- Pista 1: un patrón de break, con su filtro y su volumen.
-- (Cada pista lleva su comentario, como en una canción de verdad.)
track("p1", amen)
  :steps("1 . . 1  . . 3 .  1 . 5 .  . 3' . 6")
  :fx("lowpass", { cutoff = 3000 })
  :every(2, function(c) c:steps("x . . .  x . . .  x . . .  x . . .") end)
  :gain(0.30)

-- Pista 2: un patrón de golpes, con su filtro y su volumen.
-- (Cada pista lleva su comentario, como en una canción de verdad.)
track("p2", amen:slice_at(2))
  :steps("x . . .  x . . .  x . . .  x . . .")
  :fx("lowpass", { cutoff = 3500 })
  :every(3, function(c) c:steps(". . x .  . . x .  . . x .  . . x x") end)
  :gain(0.35)

-- Pista 3: un patrón de golpes, con su filtro y su volumen.
-- (Cada pista lleva su comentario, como en una canción de verdad.)
track("p3", amen:slice_at(3))
  :steps(". . x .  . . x .  . . x .  . . x x")
  :fx("lowpass", { cutoff = 4000 })
  :every(4, function(c) c:steps("x . x .  . x . x  x . . x  . x . .") end)
  :gain(0.40)

-- Pista 4: un patrón de golpes, con su filtro y su volumen.
-- (Cada pista lleva su comentario, como en una canción de verdad.)
track("p4", amen)
  :steps("1 2 3 4  5 6 7 8  1 . 3 .  5 . 7 .")
  :fx("lowpass", { cutoff = 4500 })
  :every(2, function(c) c:steps("x . x .  . x . x  x . . x  . x . .") end)
  :gain(0.45)

-- Pista 5: un patrón de golpes, con su filtro y su volumen.
-- (Cada pista lleva su comentario, como en una canción de verdad.)
track("p5", amen:slice_at(5))
  :steps("x . x .  . x . x  x . . x  . x . .")
  :fx("lowpass", { cutoff = 5000 })
  :every(3, function(c) c:steps(". x . x  . x . x  . x . x  . x x x") end)
  :gain(0.50)

-- Pista 6: un patrón de golpes, con su filtro y su volumen.
-- (Cada pista lleva su comentario, como en una canción de verdad.)
track("p6", amen:slice_at(6))
  :steps(". x . x  . x . x  . x . x  . x x x")
  :fx("lowpass", { cutoff = 5500 })
  :every(4, function(c) c:steps("x . . x  . . x .  x . x .  . x . x") end)
  :gain(0.55)

local grave = sh101{
  saw = 0.7, pulse = 0.3, sub = 0.6, sub_div = 2,
  cutoff = 400, res = 0.5, env_amt = 0.4,
  a = 3, d = 250, s = 0.6, r = 120,
  glide = 60, glide_mode = "auto",
}

track("bajo", grave)
  :melody("c2 _ _ _  . . c2 .  eb2 _ _ .  f2 _ g2/ _")
  :every(4, function(c) c:melody("c2 _ c2 .  eb2 . f2 _  g2 _ _ .  bb1/ _ c2 _") end)
  :gain(0.6)

-- Nota 1: ideas para la parte 1 (variaciones, cortes, un fill al final).
-- La guía explica cada función; aquí solo hay texto que no suena.

-- Nota 2: ideas para la parte 2 (variaciones, cortes, un fill al final).
-- La guía explica cada función; aquí solo hay texto que no suena.

-- Nota 3: ideas para la parte 3 (variaciones, cortes, un fill al final).
-- La guía explica cada función; aquí solo hay texto que no suena.

-- Nota 4: ideas para la parte 4 (variaciones, cortes, un fill al final).
-- La guía explica cada función; aquí solo hay texto que no suena.

-- Nota 5: ideas para la parte 5 (variaciones, cortes, un fill al final).
-- La guía explica cada función; aquí solo hay texto que no suena.

-- Nota 6: ideas para la parte 6 (variaciones, cortes, un fill al final).
-- La guía explica cada función; aquí solo hay texto que no suena.

-- Nota 7: ideas para la parte 7 (variaciones, cortes, un fill al final).
-- La guía explica cada función; aquí solo hay texto que no suena.

-- Nota 8: ideas para la parte 8 (variaciones, cortes, un fill al final).
-- La guía explica cada función; aquí solo hay texto que no suena.

-- Nota 9: ideas para la parte 9 (variaciones, cortes, un fill al final).
-- La guía explica cada función; aquí solo hay texto que no suena.

-- Nota 10: ideas para la parte 10 (variaciones, cortes, un fill al final).
-- La guía explica cada función; aquí solo hay texto que no suena.

-- Nota 11: ideas para la parte 11 (variaciones, cortes, un fill al final).
-- La guía explica cada función; aquí solo hay texto que no suena.

-- Nota 12: ideas para la parte 12 (variaciones, cortes, un fill al final).
-- La guía explica cada función; aquí solo hay texto que no suena.

-- Nota 13: ideas para la parte 13 (variaciones, cortes, un fill al final).
-- La guía explica cada función; aquí solo hay texto que no suena.

-- Nota 14: ideas para la parte 14 (variaciones, cortes, un fill al final).
-- La guía explica cada función; aquí solo hay texto que no suena.

-- Nota 15: ideas para la parte 15 (variaciones, cortes, un fill al final).
-- La guía explica cada función; aquí solo hay texto que no suena.

-- Nota 16: ideas para la parte 16 (variaciones, cortes, un fill al final).
-- La guía explica cada función; aquí solo hay texto que no suena.

-- Nota 17: ideas para la parte 17 (variaciones, cortes, un fill al final).
-- La guía explica cada función; aquí solo hay texto que no suena.

