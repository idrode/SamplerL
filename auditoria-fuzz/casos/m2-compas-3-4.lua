bpm(120)
meter(3, 4)
local amen = sample("amen.wav"):slice(8)
-- 12 pasos por compás (3/4). El volumen sube de 0 a 1 en «1 compás» y vuelve a empezar.
track("d", amen):length(12):steps("1 1 1 1  1 1 1 1  1 1 1 1"):gain(ramp(0, 1, "1 compás"))
