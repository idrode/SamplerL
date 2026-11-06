#!/usr/bin/env python3
"""Un flujo real de la interfaz, sin tarjeta ni terminal (para la auditoría).

Uso: scripts/auditoria/sesion.py [--minutos 6] [--bin target/release/tui]

Prepara en una carpeta temporal un break sintético y una canción (cortes,
pasos, un filtro, un envío al delay), abre `tui --edit` en una
pseudo-terminal de 160x45 sin `--allow-audio` (salida nula a ritmo real) y:

1. la deja sonar y lee la cabecera (marcha y compás);
2. escribe en el editor una línea con un error de Lua y guarda (Ctrl-S):
   comprueba que el compás sigue avanzando, que se ve el error y qué quedó
   en el disco;
3. borra la línea y guarda otra vez: el error se va;
4. la deja sonar `--minutos`, mirando cada 30 s la memoria (VmRSS), la CPU
   de cada hilo y el compás.

Lee la pantalla con un emulador mínimo (mover el cursor y escribir; los
colores se ignoran), suficiente para lo que dibuja ratatui.
"""

import argparse
import fcntl
import os
import pty
import re
import select
import shutil
import signal
import struct
import subprocess
import tempfile
import termios
import time

ROOT = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
TICK = os.sysconf("SC_CLK_TCK")
SONG = """bpm(170)

local amen = sample("break.wav"):slice(8)

track("drums", amen)
  :steps("1 . . 1  . . 3 .  1 . 5 .  . 3 . 6")
  :fx("filter", { type = "lowpass", cutoff = 3000 })
  :send("delay", 0.2)

track("kick", amen:slice_at(1))
  :steps("x . . . x . . . x . . . x . . .")
  :gain(0.9)
"""


class Screen:
    """Lo justo de un VT100 para la salida de ratatui."""

    def __init__(self, cols, rows):
        self.cols, self.rows = cols, rows
        self.cells = [[" "] * cols for _ in range(rows)]
        self.x = self.y = 0
        self.buf = b""

    def feed(self, data):
        self.buf += data
        text = self.buf.decode("utf-8", errors="ignore")
        # Si se cortó a mitad de un carácter o de una secuencia, esperar.
        cut = len(text)
        tail = text.rfind("\x1b")
        if tail != -1 and not re.match(r"\x1b(\[[0-9;?<>=]*[ -/]*[@-~]|\][^\x07]*\x07|[@-Z\\-_])", text[tail:]):
            cut = tail
        self.buf = text[cut:].encode()
        i, text = 0, text[:cut]
        while i < len(text):
            c = text[i]
            if c == "\x1b":
                m = re.match(r"\x1b\[([0-9;?<>=]*)([ -/]*)([@-~])", text[i:])
                if m:
                    self.csi(m.group(1), m.group(3))
                    i += m.end()
                    continue
                m = re.match(r"\x1b\][^\x07]*\x07", text[i:])
                i += m.end() if m else 2
                continue
            if c == "\r":
                self.x = 0
            elif c == "\n":
                self.y = min(self.y + 1, self.rows - 1)
            elif c >= " ":
                if 0 <= self.y < self.rows and 0 <= self.x < self.cols:
                    self.cells[self.y][self.x] = c
                self.x += 1
            i += 1

    def csi(self, params, final):
        nums = [int(p) if p.isdigit() else 0 for p in params.lstrip("?").split(";")] if params else []
        if final == "H":
            self.y = (nums[0] if nums and nums[0] else 1) - 1
            self.x = (nums[1] if len(nums) > 1 and nums[1] else 1) - 1
        elif final == "J" and not params.startswith("?"):
            self.cells = [[" "] * self.cols for _ in range(self.rows)]
        elif final == "K":
            for x in range(self.x, self.cols):
                if 0 <= self.y < self.rows:
                    self.cells[self.y][x] = " "
        elif final == "C":
            self.x += nums[0] if nums and nums[0] else 1
        elif final == "D":
            self.x -= nums[0] if nums and nums[0] else 1
        elif final == "G":
            self.x = (nums[0] if nums else 1) - 1

    def text(self):
        return "\n".join("".join(r).rstrip() for r in self.cells)


class Session:
    def __init__(self, binary, workdir, cols=160, rows=45):
        env = dict(os.environ)
        home = os.path.join(workdir, "home")
        env.update(
            {
                "XDG_CONFIG_HOME": os.path.join(home, "config"),
                "XDG_STATE_HOME": os.path.join(home, "state"),
                "HOME": home,
                "TERM": "xterm-256color",
                "COLORTERM": "truecolor",
                "LANG": "C.UTF-8",
            }
        )
        env.pop("TMUX", None)
        self.screen = Screen(cols, rows)
        self.pid, self.fd = pty.fork()
        if self.pid == 0:
            os.chdir(workdir)
            os.execvpe(binary, [binary, "--edit", "cancion.lua"], env)
        fcntl.ioctl(self.fd, termios.TIOCSWINSZ, struct.pack("HHHH", rows, cols, 0, 0))
        os.kill(self.pid, signal.SIGWINCH)

    def pump(self, seconds):
        end = time.monotonic() + seconds
        while (left := end - time.monotonic()) > 0:
            r, _, _ = select.select([self.fd], [], [], left)
            if not r:
                continue
            try:
                data = os.read(self.fd, 1 << 16)
            except OSError:
                return
            if b"\x1b[c" in data:
                os.write(self.fd, b"\x1b[?62;22c")
            if b"\x1b[6n" in data:
                os.write(self.fd, b"\x1b[1;1R")
            self.screen.feed(data)

    def send(self, data, wait=0.15):
        os.write(self.fd, data)
        self.pump(wait)

    def header(self):
        text = self.screen.text()
        m = re.search(r"(▶ en marcha|■ parado).*?compás (\d+)", text)
        return (m.group(1), int(m.group(2))) if m else (None, None)

    def rss_kb(self):
        with open(f"/proc/{self.pid}/status") as f:
            for line in f:
                if line.startswith("VmRSS:"):
                    return int(line.split()[1])
        return 0

    def cpu_ticks(self):
        total = {}
        for tid in os.listdir(f"/proc/{self.pid}/task"):
            try:
                stat = open(f"/proc/{self.pid}/task/{tid}/stat").read()
            except OSError:
                continue
            f = stat[stat.rindex(")") + 2 :].split()
            total["ui" if int(tid) == self.pid else "otros"] = total.get(
                "ui" if int(tid) == self.pid else "otros", 0
            ) + int(f[11]) + int(f[12])
        return total

    def close(self):
        try:
            os.kill(self.pid, signal.SIGTERM)
            time.sleep(0.3)
            os.kill(self.pid, signal.SIGKILL)
        except ProcessLookupError:
            pass


def find(text, pattern):
    return [l.strip() for l in text.splitlines() if re.search(pattern, l)]


def main():
    p = argparse.ArgumentParser()
    p.add_argument("--minutos", type=float, default=6)
    p.add_argument("--bin", default=os.path.join(ROOT, "target/release/tui"))
    args = p.parse_args()
    work = tempfile.mkdtemp(prefix="auditoria-sesion-")
    try:
        subprocess.run(
            ["cargo", "run", "-q", "--release", "-p", "engine", "--example", "synth_break", "--", os.path.join(work, "break.wav")],
            cwd=ROOT,
            check=True,
            stdout=subprocess.DEVNULL,
        )
        with open(os.path.join(work, "cancion.lua"), "w") as f:
            f.write(SONG)
        s = Session(args.bin, work)
        s.pump(5)
        print("1. al abrir:", s.header(), "RSS", s.rss_kb() // 1024, "MB")
        s.pump(3)
        print("   3 s después:", s.header())

        # 2. Un error de Lua al final, y guardar.
        s.send(b"\x1b[1;5F")  # Ctrl-Fin
        s.send(b"\r")
        for ch in b"= = 1":
            s.send(bytes([ch]), 0.05)
        before = s.header()
        s.send(b"\x13", 1.0)  # Ctrl-S
        s.pump(3)
        after = s.header()
        text = s.screen.text()
        disk = open(os.path.join(work, "cancion.lua")).read()
        print("2. con el error guardado:", before, "->", after)
        print("   en el disco, la última línea:", repr(disk.rstrip().splitlines()[-1]))
        print("   en pantalla:", find(text, r"error|esperaba|unexpected|cerca|near")[:3])

        # 3. Quitar la línea y guardar otra vez.
        for _ in range(6):
            s.send(b"\x7f", 0.05)
        s.send(b"\x13", 1.0)
        s.pump(3)
        text = s.screen.text()
        print("3. arreglado:", s.header(), "errores en pantalla:", find(text, r"error")[:2])

        # 4. La sesión larga.
        print(f"4. {args.minutos} minutos sonando (cada 30 s: compás, RSS, CPU de la interfaz y del resto):")
        start = time.monotonic()
        last_t, last_c = start, s.cpu_ticks()
        first_rss = s.rss_kb()
        while time.monotonic() - start < args.minutos * 60:
            s.pump(30)
            now, cpu = time.monotonic(), s.cpu_ticks()
            dt = now - last_t
            ui = 100 * (cpu.get("ui", 0) - last_c.get("ui", 0)) / TICK / dt
            other = 100 * (cpu.get("otros", 0) - last_c.get("otros", 0)) / TICK / dt
            print(
                f"   {now - start:5.0f} s  {s.header()}  RSS {s.rss_kb() / 1024:6.1f} MB  ui {ui:4.1f} %  otros {other:4.1f} %",
                flush=True,
            )
            last_t, last_c = now, cpu
        print(f"   RSS: de {first_rss / 1024:.1f} a {s.rss_kb() / 1024:.1f} MB")
        print("   pantalla final (cabecera y mensaje):")
        lines = s.screen.text().splitlines()
        for l in lines[:3] + lines[-3:]:
            print("   |", l[:150])
        s.close()
    finally:
        shutil.rmtree(work, ignore_errors=True)


if __name__ == "__main__":
    main()
