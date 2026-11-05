#!/usr/bin/env bash
# Comprueba que dos binarios de `tui` renderizan lo mismo, bit a bit: cada
# receta de scripts/ejemplos/ y las canciones de scripts/consumo/, a WAV
# con --render, y `cmp`. Sirve en ARM64, donde golden.rs no corre (sus
# huellas son de x86_64): una optimización del motor no debe cambiar
# ningún byte.
#
# Uso:
#   scripts/comparar-audio.sh BINARIO_DE_ANTES [BINARIO_DE_AHORA] [COMPASES]
# (por defecto, target/release/tui y 8 compases). Para el de antes, por
# ejemplo: git worktree add /tmp/base HEAD && (cd /tmp/base && cargo build --release -p tui)
set -euo pipefail
cd "$(dirname "$0")/.."

BEFORE="${1:?uso: $0 BINARIO_DE_ANTES [BINARIO_DE_AHORA] [COMPASES]}"
AFTER="${2:-target/release/tui}"
BARS="${3:-8}"

if [ ! -e renders/synth_break.wav ]; then
    mkdir -p renders
    cargo run -q --release -p engine --example synth_break -- renders/synth_break.wav
fi

OUT="$(mktemp -d)"
trap 'rm -rf "$OUT"' EXIT
fail=0
count=0
for song in scripts/ejemplos/*.lua scripts/consumo/*.lua; do
    name="$(basename "$song" .lua)"
    "$BEFORE" --script "$song" --render "$OUT/$name-antes.wav" --bars "$BARS" >/dev/null 2>&1 || {
        echo "sin render (antes): $song"
        continue
    }
    # Las que usan azar sin semilla (`chance`...) cambian de un render a
    # otro con el mismo binario: no se pueden comparar.
    "$BEFORE" --script "$song" --render "$OUT/$name-otra.wav" --bars "$BARS" >/dev/null 2>&1
    if ! cmp -s "$OUT/$name-antes.wav" "$OUT/$name-otra.wav"; then
        echo "no determinista, sin comparar: $song"
        continue
    fi
    "$AFTER" --script "$song" --render "$OUT/$name-ahora.wav" --bars "$BARS" >/dev/null 2>&1
    count=$((count + 1))
    if ! cmp -s "$OUT/$name-antes.wav" "$OUT/$name-ahora.wav"; then
        echo "DISTINTO: $song"
        fail=1
    fi
done
if [ "$fail" = 0 ]; then
    echo "iguales bit a bit: $count canciones, $BARS compases"
fi
exit "$fail"
