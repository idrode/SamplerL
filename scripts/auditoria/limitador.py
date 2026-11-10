#!/usr/bin/env python3
"""Cuánto trabaja el limitador en las recetas (segunda auditoría).

Renderiza offline cada `scripts/ejemplos/*.lua` y cuenta las muestras de la
salida por encima del umbral del limitador (0,9: `engine/src/limiter.rs`).
El limitador es una curva sin memoria y creciente, lineal hasta 0,9: una
muestra sale por encima de 0,9 si y solo si entró por encima, así que la
cuenta sobre el WAV es exacta (salvo el redondeo a 16 bits). Deshaciendo la
curva (atanh) se saca también el pico que entró.

    scripts/auditoria/limitador.py [--bin target/release/tui] [--bars 8] [archivo.lua ...]
"""
import argparse, glob, math, os, struct, subprocess, sys, tempfile, wave

RAIZ = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
UMBRAL = 0.9


def entrada(y):
    """Lo que entró al limitador para que saliera `y` (0 a 1)."""
    if y <= UMBRAL:
        return y
    z = min((y - UMBRAL) / (1 - UMBRAL), 1 - 1e-7)
    return UMBRAL + (1 - UMBRAL) * math.atanh(z)


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--bin", default=os.path.join(RAIZ, "target/release/tui"))
    ap.add_argument("--bars", default="8")
    ap.add_argument("archivos", nargs="*")
    a = ap.parse_args()
    archivos = a.archivos or sorted(glob.glob(os.path.join(RAIZ, "scripts/ejemplos/*.lua")))
    tmp = tempfile.mkdtemp(prefix="limitador-")
    print(f"{'canción':28} {'pico sal.':>9} {'pico ent.':>9} {'dBFS ent.':>9} {'>0,9 %':>8} {'>0,99 %':>8}  racha más larga >0,9 (ms)")
    total = [0, 0]
    for lua in archivos:
        out = os.path.join(tmp, os.path.basename(lua) + ".wav")
        p = subprocess.run([a.bin, "--script", lua, "--render", out, "--bars", a.bars], capture_output=True)
        if p.returncode != 0 or not os.path.exists(out):
            print(f"{os.path.basename(lua):28} no renderiza: {p.stderr.decode()[:80]}")
            continue
        w = wave.open(out)
        n, ch, rate = w.getnframes(), w.getnchannels(), w.getframerate()
        s = struct.unpack("<%dh" % (n * ch), w.readframes(n))
        pico = max(abs(x) for x in s) / 32767
        sobre = casi = racha = peor = 0
        for i in range(0, len(s), ch):
            nivel = max(abs(x) for x in s[i : i + ch]) / 32767
            if nivel > UMBRAL:
                sobre += 1
                racha += 1
                peor = max(peor, racha)
            else:
                racha = 0
            if nivel > 0.99:
                casi += 1
        total[0] += sobre
        total[1] += n
        ent = entrada(pico)
        print(
            f"{os.path.basename(lua):28} {pico:9.4f} {ent:9.3f} {20*math.log10(max(ent,1e-9)):+9.2f} "
            f"{100*sobre/n:8.3f} {100*casi/n:8.3f}  {1000*peor/rate:6.1f}"
        )
    if total[1]:
        print(f"\nen total: {100*total[0]/total[1]:.3f} % de los frames por encima de {UMBRAL}")


if __name__ == "__main__":
    main()
