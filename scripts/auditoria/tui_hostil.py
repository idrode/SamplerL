#!/usr/bin/env python3
"""Lo que hace la interfaz de verdad con un script hostil a mitad de sesión.

Abre `tui --edit` en una pseudo-terminal (sin `--allow-audio`: salida nula),
la deja sonar, escribe una línea al principio del script, guarda con Ctrl-S
y mira si el proceso sigue vivo, si el compás avanza y si responde.

    scripts/auditoria/tui_hostil.py [--bin target/release/tui] [--solo nombre,...]
"""
import argparse, math, os, shutil, signal, struct, subprocess, sys, tempfile, time

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from sesion import ROOT, SONG, Session  # noqa: E402

CASOS = {
    "control": "local nada = 1",
    "print": 'print("HOLA-DESDE-LUA-HOLA-DESDE-LUA")',
    "print_mucho": 'for i = 1, 200 do print("LINEA-DE-PRINT " .. i) end',
    "bucle": "while true do end",
    "bucle_pcall": "while true do pcall(function() while true do end end) end",
    "patron_lento": 'local s = ("a"):rep(5000) local r = s:find(("a-"):rep(25) .. "b")',
    "variante_sin_arrange": 'section("drop", function() end) variant("drop", "B", function() track("otra", sample("break.wav"):slice(2)):steps("x . . . x . . . x . . . x . . .") end)',
    "wav_nan_44k": 'local malo = sample("nan44.wav")',
    "wav_nan_48k": 'local malo = sample("nan48.wav") track("malo", malo:slice(1)):steps("1 . . . . . . . . . . . . . . ."):send("reverb", 0.3)',
}


def wav(path, rate, samples):
    data = b"".join(struct.pack("<f", s) for s in samples)
    hdr = (
        b"RIFF" + struct.pack("<I", 36 + len(data)) + b"WAVEfmt "
        + struct.pack("<IHHIIHH", 16, 3, 1, rate, rate * 4, 4, 32) + b"data" + struct.pack("<I", len(data))
    )
    open(path, "wb").write(hdr + data)


def vivo(pid):
    try:
        r, status = os.waitpid(pid, os.WNOHANG)
    except ChildProcessError:
        return False, "ya recogido"
    if r == 0:
        return True, ""
    if os.WIFSIGNALED(status):
        return False, f"señal {os.WTERMSIG(status)}"
    return False, f"salió con {os.WEXITSTATUS(status)}"


def caso(binario, nombre, linea, rate):
    work = tempfile.mkdtemp(prefix="tui-hostil-")
    try:
        subprocess.run(
            ["cargo", "run", "-q", "--release", "-p", "engine", "--example", "synth_break", "--", os.path.join(work, "break.wav")],
            cwd=ROOT, check=True, stdout=subprocess.DEVNULL,
        )
        seno = [math.sin(i * 0.05) for i in range(2000)]
        wav(os.path.join(work, "nan44.wav"), 44100 if rate != 44100 else 48000, seno + [float("nan")] + [0.1] * 2000)
        wav(os.path.join(work, "nan48.wav"), rate, seno + [float("nan")] + [0.1] * 2000)
        with open(os.path.join(work, "cancion.lua"), "w") as f:
            f.write(SONG)
        s = Session(binario, work)
        s.pump(4)
        antes = s.header()
        s.send(b"\x1b[1;5H")  # Ctrl-Inicio
        for ch in linea.encode():
            s.send(bytes([ch]), 0.004)
        s.send(b"\r", 0.2)
        escrito = s.screen.text()
        t0 = time.monotonic()
        s.send(b"\x13", 1.0)  # Ctrl-S
        s.pump(5)
        medio = s.header()
        cpu0 = s.cpu_ticks() if vivo(s.pid)[0] else {}
        s.pump(4)
        ok, como = vivo(s.pid)
        despues = s.header() if ok else (None, None)
        cpu1 = s.cpu_ticks() if ok else {}
        ui = (cpu1.get("ui", 0) - cpu0.get("ui", 0)) / os.sysconf("SC_CLK_TCK") / 4 * 100 if ok else 0
        texto = s.screen.text()
        rastro = [l.strip()[:110] for l in texto.splitlines() if "HOLA-DESDE" in l or "LINEA-DE-PRINT" in l or "panicked" in l]
        mensaje = [l.strip()[:150] for l in texto.splitlines() if "error" in l.lower() or "tarda" in l][:2]
        # ¿Responde? Abrir la paleta (F1) tiene que cambiar la pantalla.
        responde = None
        if ok:
            previa = s.screen.text()
            s.send(b"\x1bOP", 1.0)
            responde = s.screen.text() != previa
        print(f"== {nombre}: {linea[:70]}")
        print(f"   se escribió en el editor: {linea[:20] in escrito}")
        print(f"   proceso: {'vivo' if ok else 'MUERTO (' + como + ')'}; compás {antes[1]} → {medio[1]} → {despues[1]}; "
              f"CPU del hilo de la interfaz {ui:.0f} %; responde a F1: {responde}")
        if rastro:
            print(f"   en pantalla, {len(rastro)} líneas ajenas a la interfaz, p. ej.: {rastro[0]!r}")
        if mensaje:
            print(f"   mensaje: {mensaje[0]!r}")
        if not ok:
            cola = [l for l in texto.splitlines() if l.strip()][-4:]
            for l in cola:
                print("   |", l[:140])
        s.close()
    finally:
        shutil.rmtree(work, ignore_errors=True)


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--bin", default=os.path.join(ROOT, "target/release/tui"))
    ap.add_argument("--solo", default="")
    ap.add_argument("--rate", type=int, default=48000, help="la frecuencia de la salida nula")
    a = ap.parse_args()
    solo = [x for x in a.solo.split(",") if x]
    for nombre, linea in CASOS.items():
        if not solo or nombre in solo:
            caso(a.bin, nombre, linea, a.rate)


if __name__ == "__main__":
    main()
