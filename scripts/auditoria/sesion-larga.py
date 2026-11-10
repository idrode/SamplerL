#!/usr/bin/env python3
"""Deriva en sesiones muy largas, sin esperar horas (segunda auditoría).

`--from N` arranca el render «como si la canción hubiera llegado» al compás
N (`Controller::start_at`). Una canción periódica tiene que sonar igual en
el compás 0 que en el N si N es múltiplo de su ciclo: se renderiza desde 0 y
desde compases cada vez más lejanos y se compara muestra a muestra.

    scripts/auditoria/sesion-larga.py cancion.lua [--bars 4] [--ciclo 64]
"""
import argparse, os, struct, subprocess, sys, tempfile, wave

RAIZ = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))


def render(binario, lua, desde, bars, tmp):
    out = os.path.join(tmp, f"desde-{desde}.wav")
    p = subprocess.run(
        [binario, "--script", lua, "--render", out, "--bars", str(bars), "--from", str(desde)], capture_output=True
    )
    if p.returncode != 0 or not os.path.exists(out):
        return None, (p.stdout + p.stderr).decode()[:200]
    w = wave.open(out)
    n = w.getnframes() * w.getnchannels()
    return struct.unpack("<%dh" % n, w.readframes(w.getnframes())), ""


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("lua")
    ap.add_argument("--bin", default=os.path.join(RAIZ, "target/release/tui"))
    ap.add_argument("--bars", type=int, default=4)
    ap.add_argument("--ciclo", type=int, default=64)
    a = ap.parse_args()
    tmp = tempfile.mkdtemp(prefix="larga-")
    base, err = render(a.bin, a.lua, 0, a.bars, tmp)
    if base is None:
        sys.exit(f"no renderiza: {err}")
    print(f"{os.path.basename(a.lua)}: {a.bars} compases, {len(base)} muestras; pico {max(abs(x) for x in base)}")
    for k in [1, 16, 256, 1000, 1023, 1024, 4096, 65536 // a.ciclo]:
        desde = k * a.ciclo
        otra, err = render(a.bin, a.lua, desde, a.bars, tmp)
        if otra is None:
            print(f"  desde el compás {desde:>8}: {err.strip()}")
            continue
        if len(otra) != len(base):
            print(f"  desde el compás {desde:>8}: LONGITUD DISTINTA ({len(otra)} frente a {len(base)})")
            continue
        dif = [abs(x - y) for x, y in zip(base, otra)]
        peor = max(dif)
        distintas = sum(1 for d in dif if d)
        print(f"  desde el compás {desde:>8}: diferencia máxima {peor:>5} de 32767, {100*distintas/len(dif):6.2f} % de las muestras distintas")


if __name__ == "__main__":
    main()
