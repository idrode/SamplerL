#!/usr/bin/env python3
"""Mide el consumo de la interfaz (`tui --edit`) sin terminal ni tarjeta.

Arranca `target/release/tui --edit` en un pseudoterminal de un tamaño fijo
(sin `--allow-audio`: el motor suena por la salida nula), contesta a las
preguntas de la terminal, y durante un tiempo fijo por escenario mide:

- la CPU de cada hilo (utime + stime de /proc/<pid>/task/*/stat), en % de
  un núcleo: `ui` es el hilo principal (entrada, lógica y dibujo), `audio`
  el de la salida nula (el callback del motor), y el resto, aparte;
- los bytes por segundo que la interfaz escribe a la terminal (lo que el
  emulador tendría que pintar) y los cuadros (cada sincronización o cada
  ráfaga de escritura separada por más de 2 ms).

No mide el emulador de terminal: para eso, mirar su proceso con `top`
mientras la interfaz corre de verdad.

Uso:
  scripts/medir-consumo.py [--segundos 10] [--tamanos 80x24,200x50]
                           [--escenarios parado,sonando,visual,raton]
                           [--bin target/release/tui] [--csv salida.csv]

Escenarios:
  parado   la canción cargada y parada con Ctrl-Espacio, sin tocar nada
  sonando  la canción tal como se carga (suena sola): el break y el bajo (scripts/consumo/cancion.lua)
  visual   sonando, con scope y espectro por pista además de los del master
           (scripts/consumo/visual.lua)
  raton    sonando, con el ratón moviéndose sin botón a 60 eventos por segundo
  raton-parado  parado, con el ratón moviéndose
"""

import argparse
import os
import pty
import re
import select
import shutil
import signal
import struct
import subprocess
import sys
import tempfile
import termios
import fcntl
import threading
import time

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
TICK = os.sysconf("SC_CLK_TCK")


def slow_cpus():
    """Las CPU de menos frecuencia máxima (los núcleos de eficiencia de un
    M1/M2; vacío si todas son iguales)."""
    freqs = {}
    base = "/sys/devices/system/cpu"
    for name in os.listdir(base):
        if re.fullmatch(r"cpu\d+", name):
            try:
                with open(f"{base}/{name}/cpufreq/cpuinfo_max_freq") as f:
                    freqs[int(name[3:])] = int(f.read())
            except OSError:
                pass
    if not freqs or len(set(freqs.values())) == 1:
        return set()
    top = max(freqs.values())
    return {cpu for cpu, freq in freqs.items() if freq < top}


def thread_cpu(pid, tid):
    """La CPU donde corrió el hilo la última vez."""
    try:
        with open(f"/proc/{pid}/task/{tid}/stat") as f:
            stat = f.read()
    except OSError:
        return None
    return int(stat[stat.rindex(")") + 2 :].split()[36])


def thread_times(pid):
    """{tid: (nombre, ticks)} de los hilos del proceso."""
    out = {}
    base = f"/proc/{pid}/task"
    for tid in os.listdir(base):
        try:
            with open(f"{base}/{tid}/stat") as f:
                stat = f.read()
        except OSError:
            continue
        name = stat[stat.index("(") + 1 : stat.rindex(")")]
        fields = stat[stat.rindex(")") + 2 :].split()
        out[int(tid)] = (name, int(fields[11]) + int(fields[12]))
    return out


class Pty:
    """La interfaz en un pseudoterminal: cuenta lo que escribe y contesta
    a las preguntas que hace al arrancar."""

    def __init__(self, argv, cols, rows, env):
        self.pid, self.fd = pty.fork()
        if self.pid == 0:
            os.chdir(ROOT)
            os.execvpe(argv[0], argv, env)
        fcntl.ioctl(self.fd, termios.TIOCSWINSZ, struct.pack("HHHH", rows, cols, 0, 0))
        os.kill(self.pid, signal.SIGWINCH)
        self.lock = threading.Lock()
        self.bytes = 0
        self.writes = 0
        self.frames = 0
        self.last_write = 0.0
        self.alive = True
        self.reader = threading.Thread(target=self.read, daemon=True)
        self.reader.start()

    def read(self):
        while self.alive:
            try:
                r, _, _ = select.select([self.fd], [], [], 0.1)
                if not r:
                    continue
                data = os.read(self.fd, 1 << 16)
            except OSError:
                return
            if not data:
                return
            now = time.monotonic()
            with self.lock:
                self.bytes += len(data)
                self.writes += 1
                # Un cuadro: o la sincronización de ratatui, o una ráfaga
                # separada de la anterior.
                syncs = data.count(b"\x1b[?2026h")
                if syncs:
                    self.frames += syncs
                elif now - self.last_write > 0.002:
                    self.frames += 1
                self.last_write = now
            # Preguntas de la terminal: atributos (DA1) y posición del
            # cursor. El protocolo de teclado de kitty no se anuncia (como
            # en tmux): Ctrl-Espacio llega como NUL.
            if b"\x1b[c" in data:
                self.send(b"\x1b[?62;22c")
            if b"\x1b[6n" in data:
                self.send(b"\x1b[1;1R")

    def send(self, data):
        try:
            os.write(self.fd, data)
        except OSError:
            pass

    def counters(self):
        with self.lock:
            return self.bytes, self.writes, self.frames

    def close(self):
        self.send(b"\x11")  # Ctrl-Q, por si sale sola
        time.sleep(0.2)
        try:
            os.kill(self.pid, signal.SIGTERM)
        except ProcessLookupError:
            pass
        try:
            os.waitpid(self.pid, 0)
        except ChildProcessError:
            pass
        self.alive = False


def mouse_mover(term, cols, rows, stop):
    """Movimientos sin botón (SGR 1006, como kitty), 60 por segundo, que
    recorren la pantalla."""
    i = 0
    while not stop.is_set():
        x = 1 + (i * 7) % cols
        y = 1 + (i * 3) % rows
        term.send(f"\x1b[<35;{x};{y}M".encode())
        i += 1
        time.sleep(1 / 60)


def measure(args, scenario, cols, rows, env):
    script = "scripts/consumo/visual.lua" if scenario == "visual" else "scripts/consumo/cancion.lua"
    argv = [args.bin, "--edit", script] + (["--allow-midi"] if args.midi else [])
    term = Pty(argv, cols, rows, env)
    time.sleep(args.arranque)
    # Al cargar, la canción ya suena: parado es Ctrl-Espacio (paro).
    if scenario in ("parado", "raton-parado"):
        term.send(b"\x00")
    time.sleep(1.0)
    stop = threading.Event()
    mover = None
    if scenario.startswith("raton"):
        mover = threading.Thread(target=mouse_mover, args=(term, cols, rows, stop), daemon=True)
        mover.start()
        time.sleep(0.5)
    t0 = thread_times(term.pid)
    b0, w0, f0 = term.counters()
    start = time.monotonic()
    # Dónde corre cada hilo, mirado a menudo (en un M1/M2, un núcleo de
    # eficiencia a poca frecuencia da más % por el mismo trabajo).
    slow = slow_cpus()
    seen = {}
    while time.monotonic() - start < args.segundos:
        for tid in t0:
            cpu = thread_cpu(term.pid, tid)
            if cpu is not None:
                hits = seen.setdefault(tid, [0, 0])
                hits[0] += cpu in slow
                hits[1] += 1
        time.sleep(0.05)
    elapsed = time.monotonic() - start
    t1 = thread_times(term.pid)
    b1, w1, f1 = term.counters()
    stop.set()
    term.close()

    pid = term.pid
    threads = {}
    for tid, (name, ticks) in t1.items():
        before = t0.get(tid, (name, ticks))[1]
        pct = 100.0 * (ticks - before) / TICK / elapsed
        if tid == pid:
            label = "ui"
        else:
            label = f"{name}:{tid - pid}"
        threads[label] = pct
    # El hilo del audio: el que más gasta de los otros sonando; parado, el
    # que gasta algo con regularidad. Se marca el de más CPU que no es ui.
    others = sorted(((v, k) for k, v in threads.items() if k != "ui"), reverse=True)
    audio = others[0][1] if others else None
    def slow_share(label):
        tid = term.pid if label == "ui" else next((t for t in t1 if f"{t1[t][0]}:{t - term.pid}" == label), None)
        hits = seen.get(tid)
        return 100.0 * hits[0] / hits[1] if hits and hits[1] and slow else None

    return {
        "lentos_ui": slow_share("ui"),
        "lentos_audio": slow_share(audio) if audio else None,
        "escenario": scenario,
        "tamano": f"{cols}x{rows}",
        "ui": threads.get("ui", 0.0),
        "audio": threads.get(audio, 0.0) if audio else 0.0,
        "otros": sum(v for k, v in threads.items() if k not in ("ui", audio)),
        "total": sum(threads.values()),
        "kB/s": (b1 - b0) / elapsed / 1024,
        "cuadros/s": (f1 - f0) / elapsed,
        "escrituras/s": (w1 - w0) / elapsed,
        "hilos": threads,
    }


def main():
    p = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    p.add_argument("--segundos", type=float, default=10.0)
    p.add_argument("--arranque", type=float, default=3.0)
    p.add_argument("--tamanos", default="80x24,200x50")
    p.add_argument("--escenarios", default="parado,sonando,visual,raton,raton-parado")
    p.add_argument("--bin", default=os.path.join(ROOT, "target/release/tui"))
    p.add_argument("--csv")
    p.add_argument("--hilos", action="store_true", help="enseña cada hilo")
    p.add_argument("--midi", action="store_true", help="con --allow-midi (la espera del bucle baja a 2 ms)")
    args = p.parse_args()

    if not os.path.exists(os.path.join(ROOT, "renders/synth_break.wav")):
        os.makedirs(os.path.join(ROOT, "renders"), exist_ok=True)
        subprocess.run(
            ["cargo", "run", "-q", "--release", "-p", "engine", "--example", "synth_break", "--", "renders/synth_break.wav"],
            cwd=ROOT,
            check=True,
        )
    # Configuración, bloques e historial aparte: la medición no depende
    # de la configuración del usuario ni le escribe nada.
    home = tempfile.mkdtemp(prefix="medir-consumo-")
    env = dict(os.environ)
    env.update(
        {
            "XDG_CONFIG_HOME": os.path.join(home, "config"),
            "XDG_STATE_HOME": os.path.join(home, "state"),
            "TERM": "xterm-256color",
            "COLORTERM": "truecolor",
            "LANG": env.get("LANG", "es_ES.UTF-8") if "UTF-8" in env.get("LANG", "") else "C.UTF-8",
        }
    )
    env.pop("TMUX", None)
    rows_out = []
    header = (
        f"{'escenario':<13} {'tamaño':>7} {'ui %':>6} {'audio %':>7} {'otros %':>7} "
        f"{'kB/s':>8} {'cuadros/s':>9}  núcleo lento ui/audio"
    )
    print(header)
    try:
        for size in args.tamanos.split(","):
            cols, rows = (int(v) for v in size.split("x"))
            for scenario in args.escenarios.split(","):
                r = measure(args, scenario, cols, rows, env)
                rows_out.append(r)
                share = lambda v: "-" if v is None else f"{v:.0f} %"
                print(
                    f"{r['escenario']:<13} {r['tamano']:>7} {r['ui']:6.1f} {r['audio']:7.1f} {r['otros']:7.1f} "
                    f"{r['kB/s']:8.1f} {r['cuadros/s']:9.1f}  {share(r['lentos_ui'])} / {share(r['lentos_audio'])}",
                    flush=True,
                )
                if args.hilos:
                    for k, v in sorted(r["hilos"].items()):
                        print(f"    {k:<24} {v:6.2f} %")
    finally:
        shutil.rmtree(home, ignore_errors=True)
    if args.csv:
        with open(args.csv, "w") as f:
            f.write("escenario,tamano,midi,ui,audio,otros,kBps,cuadros\n")
            for r in rows_out:
                f.write(
                    f"{r['escenario']},{r['tamano']},{int(args.midi)},{r['ui']:.2f},{r['audio']:.2f},{r['otros']:.2f},{r['kB/s']:.2f},{r['cuadros/s']:.2f}\n"
                )


if __name__ == "__main__":
    main()
