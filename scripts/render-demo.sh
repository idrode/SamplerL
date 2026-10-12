#!/usr/bin/env bash
# Renderiza offline un patrón del secuenciador a renders/demo.wav, sin usar
# la salida de audio. Sin argumento usa un break sintético (renders/synth_break.wav).
#
# Uso:
#   scripts/render-demo.sh [entrada.wav]
# Variables opcionales (con su valor por defecto):
#   PATTERN="1 . . 1  . . 3 .  1 . 5 .  . 3 . 6"  SLICES=8  BPM=170  BARS=4  MIN_GAP=40
#   DETECTOR=flux (o energy)
# PATTERN usa la mini-notación: 3 corte, . silencio, 3' reverse, 3^7 pitch,
# 3@0.5 volumen, 3r4 retrigger, combinables (3'^7@0.5r2).
set -euo pipefail
cd "$(dirname "$0")/.."

OUT=renders
mkdir -p "$OUT"

INPUT="${1:-}"
if [ -z "$INPUT" ]; then
    INPUT="$OUT/synth_break.wav"
    cargo run -q --release -p engine --example synth_break -- "$INPUT"
fi

cargo run -q --release -p tui -- "$INPUT" \
    --render "$OUT/demo.wav" \
    --pattern "${PATTERN:-1 . . 1  . . 3 .  1 . 5 .  . 3 . 6}" \
    --slices "${SLICES:-8}" \
    --bpm "${BPM:-170}" \
    --bars "${BARS:-4}" \
    --min-gap "${MIN_GAP:-40}" \
    --detector "${DETECTOR:-flux}"

echo
echo "Para escucharlo en otra máquina:"
echo "  scp $(hostname):$(pwd)/$OUT/demo.wav ."
