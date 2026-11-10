#!/usr/bin/env python3
"""Mensajes hostiles por el socket de las ventanas aparte (protocolo 8).

Abre una principal de verdad (`tui --edit cancion.lua --split`, sin audio,
en una pseudo-terminal, con `[editor] terminal = ["true"]` para que no abra
nada), se conecta a su socket como si fuera el editor y le manda líneas
válidas con valores límite y líneas rotas. Falla si la principal muere o
deja de responder. No se mandan órdenes ni acciones de verdad (podrían
exportar o salir): solo nombres al azar.

    scripts/auditoria/fuzz_protocolo.py [--segundos 120] [--semilla 1]
"""
import argparse, fcntl, glob, os, pty, random, select, shutil, signal, socket, struct, subprocess, sys, tempfile, termios, time

RAIZ = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from sesion import SONG, Screen  # noqa: E402

NUMEROS = ["0", "1", "8", "7", "9", "-1", "4294967295", "4294967296", "18446744073709551615", "18446744073709551616",
           "999999999", "1e3", "", " ", "x", "0x10", "+1", "٣"]
TEXTOS = ["", "x", SONG, SONG * 40, "bpm(120)\n", "\\", "\\q", "\\t\\n\\r\\\\", "ñ🥁\u0000", "a\tb", "línea\\nlínea",
          "x" * 100000, "= = 1", "\x1b[2J\x1b[H"]
VERBOS = ["hola", "texto", "cargar", "accion", "orden", "vuelvo", "", "HOLA", "adios", "estado", "bienvenida", "raton",
          "tecla", "x" * 300]


def mensaje(rng):
    k = rng.random()
    if k < 0.15:
        return ("hola\t" + rng.choice(NUMEROS)).encode()
    if k < 0.40:
        return (rng.choice(["texto", "cargar"]) + "\t" + rng.choice(NUMEROS) + "\t" + esc(rng.choice(TEXTOS))).encode()
    if k < 0.50:
        return ("accion\t" + rng.choice(["nada-" + str(rng.randrange(99)), "", "x" * 5000, "ñ"])).encode()
    if k < 0.56:
        return ("orden\t" + rng.choice(["zzz", "", " ", "zzz " + "y" * 3000, "\\", "ñ ñ"])).encode()
    if k < 0.80:
        n = rng.choice([0, 0, 1, 2, 5, 50])
        historia = [esc(rng.choice(TEXTOS[:9])) for _ in range(n)]
        campos = [rng.choice(NUMEROS), rng.choice(NUMEROS), rng.choice(NUMEROS + [str(n), str(n + 1)]),
                  esc(rng.choice(TEXTOS))] + historia
        return ("vuelvo\t" + "\t".join(campos[: rng.choice([len(campos)] * 5 + [0, 1, 2, 3])])).encode()
    if k < 0.90:
        campos = [rng.choice(NUMEROS + TEXTOS[:8]) for _ in range(rng.randrange(8))]
        return (rng.choice(VERBOS) + "".join("\t" + c for c in campos)).encode()
    if k < 0.95:
        return bytes(rng.randrange(256) for _ in range(rng.randrange(200))).replace(b"\n", b" ")
    return b"texto\t1\t" + b"\xff\xfe" + b"x" * rng.randrange(2000)


def esc(t):
    return t.replace("\\", "\\\\").replace("\t", "\\t").replace("\n", "\\n").replace("\r", "\\r")


class Principal:
    def __init__(self, binario, tmp):
        self.tmp = tmp
        home = os.path.join(tmp, "home")
        conf = os.path.join(home, "config", "sampler")
        os.makedirs(conf, exist_ok=True)
        open(os.path.join(conf, "config.toml"), "w").write('[editor]\nterminal = ["true"]\n')
        self.run = os.path.join(tmp, "run")
        shutil.rmtree(self.run, ignore_errors=True)
        os.makedirs(self.run, mode=0o700)
        env = dict(os.environ, XDG_CONFIG_HOME=os.path.join(home, "config"), XDG_STATE_HOME=os.path.join(home, "state"),
                   XDG_RUNTIME_DIR=self.run, HOME=home, TERM="xterm-256color", COLORTERM="truecolor", LANG="C.UTF-8")
        env.pop("TMUX", None)
        self.screen = Screen(160, 45)
        self.pid, self.fd = pty.fork()
        if self.pid == 0:
            os.chdir(tmp)
            os.execvpe(binario, [binario, "--edit", "cancion.lua", "--split"], env)
        fcntl.ioctl(self.fd, termios.TIOCSWINSZ, struct.pack("HHHH", 45, 160, 0, 0))
        os.kill(self.pid, signal.SIGWINCH)
        self.pump(3)

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

    def viva(self):
        try:
            r, status = os.waitpid(self.pid, os.WNOHANG)
        except ChildProcessError:
            return False
        return r == 0

    def conectar(self):
        socks = glob.glob(os.path.join(self.run, "sampler", "*.sock"))
        socks = [s for s in socks if not s.endswith(("-disp.sock", "-roll.sock"))]
        if not socks:
            return None
        c = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
        try:
            c.connect(socks[0])
        except OSError:
            return None
        c.setblocking(False)
        return c

    def cerrar(self):
        try:
            os.kill(self.pid, signal.SIGKILL)
            os.waitpid(self.pid, 0)
        except (ProcessLookupError, ChildProcessError):
            pass
        try:
            os.close(self.fd)
        except OSError:
            pass


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--bin", default=os.path.join(RAIZ, "target/release/tui"))
    ap.add_argument("--segundos", type=float, default=120)
    ap.add_argument("--semilla", type=int, default=1)
    a = ap.parse_args()
    rng = random.Random(a.semilla)
    tmp = tempfile.mkdtemp(prefix="fuzz-proto-")
    subprocess.run(["cargo", "run", "-q", "--release", "-p", "engine", "--example", "synth_break", "--",
                    os.path.join(tmp, "break.wav")], cwd=RAIZ, check=True, stdout=subprocess.DEVNULL)
    open(os.path.join(tmp, "cancion.lua"), "w").write(SONG)
    p = Principal(a.bin, tmp)
    enviados = conexiones = muertes = cuelgues = sin_socket = 0
    recientes = []
    fin = time.monotonic() + a.segundos
    try:
        while time.monotonic() < fin:
            c = p.conectar()
            if c is None:
                sin_socket += 1
                p.pump(0.3)
                if sin_socket % 10 == 0:
                    # La principal ya no escucha (el editor «volvió»): otra.
                    p.cerrar()
                    p = Principal(a.bin, tmp)
                continue
            conexiones += 1
            try:
                if rng.random() < 0.8:
                    c.send(b"hola\t8\n")
                for _ in range(rng.randrange(1, 12)):
                    m = mensaje(rng)
                    recientes = (recientes + [m[:300]])[-12:]
                    try:
                        c.sendall(m + b"\n")
                    except (BlockingIOError, BrokenPipeError, ConnectionResetError, OSError):
                        break
                    enviados += 1
                    p.pump(0.02)
            finally:
                p.pump(0.05)
                c.close()
            if not p.viva():
                muertes += 1
                ruta = os.path.join(RAIZ, "auditoria-fuzz", "hallazgos", f"protocolo-{muertes:03}.txt")
                os.makedirs(os.path.dirname(ruta), exist_ok=True)
                cola = "\n".join(l for l in p.screen.text().splitlines() if l.strip())[-1500:]
                open(ruta, "wb").write(b"### LA PRINCIPAL MURIO; ultimos mensajes:\n" + b"\n".join(recientes) + b"\n### pantalla:\n" + cola.encode())
                print(f"  la principal murió → {ruta}")
                if muertes >= 5:
                    break
                p = Principal(a.bin, tmp)
        # ¿Sigue respondiendo? F1 abre la paleta: la pantalla cambia.
        if p.viva():
            antes = p.screen.text()
            os.write(p.fd, b"\x1bOP")
            p.pump(1.5)
            if p.screen.text() == antes:
                cuelgues += 1
    finally:
        p.cerrar()
        shutil.rmtree(tmp, ignore_errors=True)
    print(f"protocolo: {enviados} mensajes en {conexiones} conexiones, {a.segundos:.0f} s; "
          f"{muertes} muertes de la principal, {cuelgues} cuelgues al final")


if __name__ == "__main__":
    main()
