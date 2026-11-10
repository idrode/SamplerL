#!/usr/bin/env python3
"""Scripts de Lua hostiles contra `tui --script x.lua --render` (segunda auditoría).

Cada caso se escribe en una carpeta temporal, se carga con el binario y se
anota lo que tarda, el pico de memoria del proceso y cómo acaba. No abre la
tarjeta de sonido: solo render offline.

    scripts/auditoria/hostil.py [--bin target/release/tui] [--solo nombre,...]
"""
import argparse
import os
import resource
import shutil
import subprocess
import sys
import tempfile
import time

RAIZ = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))

CASOS = {
    # --- qué hay en el entorno ---
    "globales": 'local n={} for k in pairs(_G) do n[#n+1]=k end table.sort(n) error(table.concat(n," "))',
    "bibliotecas": 'error("io="..tostring(io).." os="..tostring(os).." require="..tostring(require)'
    ' .." debug="..tostring(debug).." package="..tostring(package).." coroutine="..tostring(coroutine))',
    "dofile": 'local ok,e=pcall(dofile,"/etc/hostname") error("dofile: "..tostring(ok).." "..tostring(e))',
    "loadfile": 'local f,e=loadfile("/etc/passwd") error("loadfile: "..tostring(f).." "..tostring(e))',
    "load_bytecode": 'local f,e=load("\\27Lua") error("load: "..tostring(f).." "..tostring(e))',
    "print": 'print("HOLA DESDE LUA") bpm(120)',
    # --- límite de tiempo ---
    "bucle": "while true do end",
    "bucle_pcall": "while true do pcall(function() while true do end end) end",
    "patron_lento": 'local s=("a"):rep(5000) local r=s:find(("a-"):rep(25).."b") bpm(120)',
    "sort_malo": "local t={} for i=1,3e5 do t[i]=i end pcall(table.sort,t,function(a,b) return true end) bpm(120)",
    "gc_bucle": "setmetatable({}, {__gc=function() while true do end end}) bpm(120)",
    "error_tostring": "error(setmetatable({}, {__tostring=function() while true do end end}))",
    # --- límite de memoria ---
    "memoria_tabla": 'local t={} for i=1,1e9 do t[i]=("x"):rep(1000)..i end',
    "memoria_rep": 'local s=("x"):rep(2^31)',
    "gsub_grande": 'local s=("a"):rep(3e7) s:gsub(".", "%0%0%0") bpm(120)',
    # --- lo que guarda Rust (fuera del límite de memoria de Lua) ---
    "steps_enorme": 'local a=sample("amen.wav"):slice(8) track("d",a):steps(("1 "):rep(4e6))',
    "steps_llaves": 'local a=sample("amen.wav"):slice(8) track("d",a):steps("1"..("{lp=800}"):rep(300000).." . . . 1 . . . 1 . . . 1 . . .")',
    "steps_acorde": 'local a=sample("amen.wav"):slice(8) track("d",a):steps("1"..("^7"):rep(300000).." . . . 1 . . . 1 . . . 1 . . .")',
    "steps_memoria": 'local a=sample("amen.wav"):slice(8) track("d",a):steps(("1 "):rep(2e7))',
    "secciones_muchas": 'local a=sample("amen.wav"):slice(8) local arr={} for i=1,4000 do section("s"..i,function() track("d",a):steps("1 . . . 1 . . . 1 . . . 1 . . .") end) arr[i]={"s"..i,256} end arrange(arr)',
    "variaciones_ciclo": 'local a=sample("amen.wav"):slice(8) for i=1,8 do local t=track("d"..i,a):steps("1 . . . 1 . . . 1 . . . 1 . . .") for _,p in ipairs{7,11,13,17,19,23,29,31} do t:every(p,function(t) t:reverse() end) end end',
    "lane_puntos": 'local a=sample("amen.wav"):slice(8) local pts={} for i=1,1e6 do pts[i]={i/1e6,0.5} end track("d",a):steps("1 . . . 1 . . . 1 . . . 1 . . ."):fx("filter",{type="lowpass",cutoff=800}):auto("filter.cutoff",pts)',
    "steps_r": 'local a=sample("amen.wav"):slice(8) track("d",a):steps("1r999999999 . . . 1 . . . 1 . . . 1 . . .")',
    "length_enorme": 'local a=sample("amen.wav"):slice(8) track("d",a):length(1e9):steps("1 . . . 1 . . . 1 . . . 1 . . .")',
    "slice_enorme": 'local a=sample("amen.wav"):slice(1e9) track("d",a):steps("1")',
    "slice_grid_enorme": 'local a=sample("amen.wav"):slice_grid(1e9) track("d",a):steps("1")',
    "euclid_enorme": 'local a=sample("amen.wav"):slice(8) track("d",a):steps(euclid(1e9,2e9))',
    "arrange_enorme": 'local a=sample("amen.wav"):slice(8) section("a",function() track("d",a):steps("1") end)'
    ' arrange({{"a",1e15}})',
    "every_enorme": 'local a=sample("amen.wav"):slice(8) track("d",a):steps("1 . . . 1 . . . 1 . . . 1 . . ."):every(1e9,function(t) t:reverse() end)',
    "every_primos": 'local a=sample("amen.wav"):slice(8) local t=track("d",a):steps("1 . . . 1 . . . 1 . . . 1 . . .")'
    " for _,p in ipairs{7,11,13,17,19,23,29,31,37,41} do t:every(p,function(t) t:reverse() end) end",
    "stretch_extremo": 'local a=sample("amen.wav"):stretch(0.001):slice(8) bpm(300) track("d",a):steps("1")',
    "stretch_extremo2": 'local a=sample("amen.wav"):stretch(1e9):slice(8) bpm(20) track("d",a):steps("1")',
    "muchos_samples": 'for i=1,5000 do sample("amen.wav"):stretch(100+i/100) end bpm(120)',
    "bpm_nan": "bpm(0/0)",
    "bpm_inf": "bpm(1/0)",
    "gain_nan": 'local a=sample("amen.wav"):slice(8) track("d",a):steps("1 . . . 1 . . . 1 . . . 1 . . ."):gain(0/0)',
    "cutoff_nan": 'local a=sample("amen.wav"):slice(8) track("d",a):steps("1 . . . 1 . . . 1 . . . 1 . . ."):fx("filter",{type="lowpass",cutoff=0/0})',
    "cutoff_inf": 'local a=sample("amen.wav"):slice(8) track("d",a):steps("1 . . . 1 . . . 1 . . . 1 . . ."):fx("filter",{type="lowpass",cutoff=1/0,q=1/0})',
    "ruta_arriba": 'local a=sample("../../../../../../etc/passwd")',
    "ruta_dev": 'local a=sample("/dev/zero")',
    "ruta_larga": 'local a=sample(("a"):rep(100000)..".wav")',
    "nombre_nul": 'local a=sample("amen.wav\\0.txt")',
    "pista_nombre_raro": 'local a=sample("amen.wav"):slice(8) track(("x"):rep(1e6).."\\27[2J\\n",a):steps("1 . . . 1 . . . 1 . . . 1 . . .")',
}


def correr(binario, carpeta, nombre, lua, espera):
    ruta = os.path.join(carpeta, nombre + ".lua")
    with open(ruta, "w") as f:
        f.write(lua + "\n")
    antes = resource.getrusage(resource.RUSAGE_CHILDREN)
    t0 = time.monotonic()
    try:
        p = subprocess.run(
            [binario, "--script", ruta, "--render", os.path.join(carpeta, nombre + ".wav"), "--bars", "1"],
            capture_output=True,
            timeout=espera,
            cwd=carpeta,
        )
        rc, salida = p.returncode, (p.stdout + p.stderr).decode("utf-8", "replace")
    except subprocess.TimeoutExpired as e:
        rc, salida = "TIMEOUT", ((e.stdout or b"") + (e.stderr or b"")).decode("utf-8", "replace")
    dt = time.monotonic() - t0
    despues = resource.getrusage(resource.RUSAGE_CHILDREN)
    # ru_maxrss de los hijos es el máximo histórico: solo dice algo si sube.
    pico = despues.ru_maxrss // 1024
    subio = despues.ru_maxrss > antes.ru_maxrss
    linea = " ".join(salida.split())[:230]
    print(f"{nombre:20} rc={rc!s:8} {dt:6.2f}s  rss{'↑' if subio else '≤'}{pico:5} MB  {linea}")
    sys.stdout.flush()


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--bin", default=os.path.join(RAIZ, "target/release/tui"))
    ap.add_argument("--solo", default="")
    ap.add_argument("--espera", type=float, default=60.0)
    a = ap.parse_args()
    solo = [s for s in a.solo.split(",") if s]
    carpeta = tempfile.mkdtemp(prefix="hostil-")
    shutil.copy(os.path.join(RAIZ, "scripts/ejemplos/amen.wav"), carpeta)
    try:
        for nombre, lua in CASOS.items():
            if solo and nombre not in solo:
                continue
            correr(a.bin, carpeta, nombre, lua, a.espera)
    finally:
        shutil.rmtree(carpeta, ignore_errors=True)


if __name__ == "__main__":
    main()
