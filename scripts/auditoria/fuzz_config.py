#!/usr/bin/env python3
"""`config.toml` hostiles (segunda auditoría).

Las claves salen de las tablas de la guía (entre `config:inicio` y
`config:fin`, generadas desde `tui/src/config.rs`). Cada vuelta escribe un
`config.toml` con claves de verdad y valores límite (o bytes cambiados) y
lanza `tui --script x.lua --render`, que lee la configuración entera
(`main.rs::script_host`). Falla si el proceso muere por una señal, dice
«panicked» o tarda más de 10 s. La configuración nunca debe impedir cargar:
«si falta o está roto, por defecto y un aviso».

    scripts/auditoria/fuzz_config.py [--segundos 120] [--semilla 1]
"""
import argparse, os, random, re, shutil, subprocess, sys, tempfile, time

RAIZ = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))

VALORES = [
    '""', '"x"', '"ctrl-s"', '"ctrl-"', '"shift-ctrl-alt-f12"', '"f0"', '"f13"', '"espacio"', '"a"', '" "',
    '["ctrl-s", "ctrl-s"]', "[]", "[[]]", '["a", 1]', "0", "1", "-1", "255", "256", "65536", "4294967296",
    "9223372036854775807", "-9223372036854775808", "1e308", "nan", "inf", "-inf", "0.5", "true", "false",
    '"#ff8700"', '"#gggggg"', '"#fff"', '"rojo"', '"\\u0000"', '"' + "x" * 5000 + '"', "{}", "{ a = 1 }",
    '"vim"', '"clasico"', '"jk"', '"j"', '"jkl"', '"ññ"', '"kitty -e"', '["kitty", "-e"]', '"\'"', "1979-05-27",
    '"relleno"', '"linea"', '"barras"', '"espectrograma"', '"braille"', '"1/16"', "[21, 22]", "[128, -1]",
    "[" + ", ".join(["1"] * 3000) + "]",
]


def claves():
    texto = open(os.path.join(RAIZ, "docs/GUIA.md")).read()
    cuerpo = texto[texto.index("config:inicio") : texto.index("config:fin")]
    out, seccion = [], None
    for linea in cuerpo.splitlines():
        m = re.match(r"\| En `\[([a-z_.]+)\]`", linea)
        if m:
            seccion = m.group(1)
            continue
        m = re.match(r"\| `([a-z_0-9]+)` \|", linea)
        if m and seccion:
            out.append((seccion, m.group(1)))
    return out


def generar(rng, todas):
    secciones = {}
    for seccion, clave in rng.sample(todas, k=min(len(todas), rng.randint(1, 25))):
        secciones.setdefault(seccion, []).append(f"{clave} = {rng.choice(VALORES)}")
    if rng.random() < 0.2:
        secciones.setdefault(rng.choice(["teclas", "nada", "script", "interfaz", "voces", "midi.x"]), []).append(
            f"{rng.choice(['clave_rara', 'limite', 'memoria', 'tiempo', 'refresco'])} = {rng.choice(VALORES)}"
        )
    texto = ""
    for seccion, lineas in secciones.items():
        texto += f"[{seccion}]\n" + "\n".join(lineas) + "\n\n"
    b = bytearray(texto.encode())
    if rng.random() < 0.25 and b:
        for _ in range(rng.randint(1, 4)):
            b[rng.randrange(len(b))] = rng.choice(b'[]="\n#.,{}\x00\xff 0')
    if rng.random() < 0.05:
        b = b[: rng.randrange(len(b) + 1)]
    return bytes(b)


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--bin", default=os.path.join(RAIZ, "target/release/tui"))
    ap.add_argument("--segundos", type=float, default=120)
    ap.add_argument("--semilla", type=int, default=1)
    a = ap.parse_args()
    rng = random.Random(a.semilla)
    todas = claves()
    print(f"{len(todas)} claves en {len(set(s for s, _ in todas))} secciones (de la guía)")
    tmp = tempfile.mkdtemp(prefix="fuzz-config-")
    conf = os.path.join(tmp, "config", "sampler")
    os.makedirs(conf)
    shutil.copy(os.path.join(RAIZ, "scripts/ejemplos/amen.wav"), tmp)
    lua = os.path.join(tmp, "x.lua")
    open(lua, "w").write('bpm(120)\nlocal a = sample("amen.wav"):slice(8)\ntrack("d", a):steps("1 . . . 2 . . . 3 . . . 4 . . .")\n')
    env = dict(os.environ, XDG_CONFIG_HOME=os.path.join(tmp, "config"), HOME=tmp)
    n = malos = no_cargan = 0
    fin = time.monotonic() + a.segundos
    try:
        while time.monotonic() < fin:
            datos = generar(rng, todas)
            open(os.path.join(conf, "config.toml"), "wb").write(datos)
            n += 1
            try:
                p = subprocess.run([a.bin, "--script", lua, "--render", os.path.join(tmp, "o.wav"), "--bars", "1"],
                                   capture_output=True, timeout=10, env=env, cwd=tmp)
                salida = (p.stdout + p.stderr).decode("utf-8", "replace")
                motivo = None
                if p.returncode < 0:
                    motivo = f"señal {-p.returncode}"
                elif "panicked" in salida:
                    motivo = "pánico"
                elif p.returncode != 0:
                    no_cargan += 1
                    motivo = f"la configuración impide cargar (rc={p.returncode}): {' '.join(salida.split())[:160]}"
            except subprocess.TimeoutExpired:
                motivo = "más de 10 s"
            if motivo:
                malos += 1
                if malos <= 5:
                    ruta = os.path.join(RAIZ, "auditoria-fuzz", "hallazgos", f"config-{malos:03}.toml")
                    os.makedirs(os.path.dirname(ruta), exist_ok=True)
                    open(ruta, "wb").write(datos)
                    print(f"  {motivo} → {ruta}")
    finally:
        shutil.rmtree(tmp, ignore_errors=True)
    print(f"config: {n} configuraciones en {a.segundos:.0f} s, {malos} con problemas ({no_cargan} impiden cargar)")


if __name__ == "__main__":
    main()
