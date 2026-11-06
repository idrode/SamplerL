#!/usr/bin/env python3
"""Mide la CPU por hilo de `tui --edit` y los bytes que escribe a la terminal.

Uso: scripts/medir-consumo.py <binario> <carpeta con cancion.lua> [segundos] [COLxFIL]

Abre la interfaz en una pseudo-terminal (200x50 si no se dice otra), sin --allow-audio (el
motor avanza con la salida nula, a ritmo real, sin abrir la tarjeta), con
HOME y XDG_CONFIG_HOME en una carpeta vacía para no leer tu configuración.
La canción arranca sola al abrirse: mide «al abrir» (en marcha), pulsa F8
(marcha/paro) y mide «tras F8» (parado). El estado entre corchetes es el
último que la interfaz escribió en pantalla. El hilo «(principal)» es el de
la interfaz; el otro con CPU es la salida nula del motor.
"""
import os, pty, sys, time, fcntl, termios, struct, select, signal, tempfile

binp, song = os.path.abspath(sys.argv[1]), os.path.abspath(sys.argv[2])
secs = float(sys.argv[3]) if len(sys.argv) > 3 else 20
cols, rows = (int(n) for n in (sys.argv[4] if len(sys.argv) > 4 else "200x50").split("x"))
home = tempfile.mkdtemp(prefix="medir-consumo-")
TCK = os.sysconf("SC_CLK_TCK")
pid, fd = pty.fork()
if pid == 0:
    os.chdir(song)
    env = dict(os.environ, HOME=home, XDG_CONFIG_HOME=home + "/.config",
               TERM="xterm-256color", COLORTERM="truecolor")
    env.pop("TMUX", None)
    os.execve(binp, [binp, "--edit", "cancion.lua"], env)
fcntl.ioctl(fd, termios.TIOCSWINSZ, struct.pack("HHHH", rows, cols, 0, 0))
os.kill(pid, signal.SIGWINCH)
nbytes, state, buf = 0, "?", b""


def pump(t):
    global nbytes, state, buf
    end = time.time() + t
    while (r := end - time.time()) > 0:
        if select.select([fd], [], [], r)[0]:
            try:
                d = os.read(fd, 65536)
            except OSError:
                return
            nbytes += len(d)
            buf = (buf + d)[-200000:]
            im, ip = buf.rfind("en marcha".encode()), buf.rfind("■ parado".encode())
            if im >= 0 or ip >= 0:
                state = "en marcha" if im > ip else "parado"


def snap():
    out = {}
    for t in os.listdir(f"/proc/{pid}/task"):
        try:
            st = open(f"/proc/{pid}/task/{t}/stat").read()
            comm = open(f"/proc/{pid}/task/{t}/comm").read().strip()
        except OSError:
            continue
        f = st[st.rindex(")") + 2:].split()
        out[t] = (comm, int(f[11]) + int(f[12]))
    return out


def measure(name):
    a, b0, t0 = snap(), nbytes, time.time()
    pump(secs)
    dt, b = time.time() - t0, snap()
    rows = sorted(((100.0 * (v - a[t][1]) / TCK / dt, c, t)
                   for t, (c, v) in b.items() if t in a), reverse=True)
    tot = sum(r[0] for r in rows)
    print(f"## {name} [{state}] ({dt:.0f} s): total {tot:.1f} %, "
          f"terminal {(nbytes - b0) / dt / 1024:.1f} KiB/s")
    for p, c, t in rows:
        if p >= 0.05 or t == str(pid):
            print(f"   {c:<20} {'(principal)' if t == str(pid) else '':<12} {p:6.1f} %")


pump(6)
print(f"# {os.path.basename(binp)} {cols}x{rows}")
measure("al abrir")
os.write(fd, b"\x1b[19~")  # F8
pump(3)
measure("tras F8")
os.kill(pid, signal.SIGTERM)
time.sleep(0.5)
try:
    os.kill(pid, signal.SIGKILL)
except OSError:
    pass
