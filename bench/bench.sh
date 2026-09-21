#!/usr/bin/env bash
# What one look at the screen costs, with and without the daemon, against
# sending a screenshot to a vision model. Every figure is measured here; only
# the token counts are arithmetic, using the formulas in BENCHMARK.md.
#
# Usage: bench/bench.sh [runs]   (default 5)
set -euo pipefail

runs=${1:-5}
binary=${SCREENPEEK:-target/release/screenpeek}
work=$(mktemp -d)
trap 'rm -rf "$work"; pkill -x screenpeek 2>/dev/null || true' EXIT

command -v grim >/dev/null || { echo "bench needs grim (Wayland screenshots)" >&2; exit 1; }
[ -x "$binary" ] || { echo "build first: cargo build --release" >&2; exit 1; }

median_ms() {
  local times=()
  for _ in $(seq "$runs"); do
    local start end
    start=$(date +%s%N)
    "$@" >/dev/null 2>&1
    end=$(date +%s%N)
    times+=($(( (end - start) / 1000000 )))
  done
  printf '%s\n' "${times[@]}" | sort -n | awk '{ v[NR]=$1 } END { print v[int((NR+1)/2)] }'
}

# Anthropic counts an image as width*height/750 tokens, after the long edge is
# scaled down to 1568 px.
vision_tokens() {
  awk -v w="$1" -v h="$2" 'BEGIN {
    long = (w > h) ? w : h
    if (long > 1568) { s = 1568 / long; w *= s; h *= s }
    printf "%d\n", (w * h) / 750
  }'
}

grim "$work/screen.png"
size=$(identify -format "%w %h" "$work/screen.png")
width=${size% *}
height=${size#* }
shot_bytes=$(wc -c < "$work/screen.png")
shot_ms=$(median_ms grim "$work/screen.png")
shot_tokens=$(vision_tokens "$width" "$height")

pkill -x screenpeek 2>/dev/null || true
sleep 1
cold_ms=$(median_ms "$binary" scan)

setsid "$binary" serve >"$work/daemon.log" 2>&1 </dev/null &
sleep 5
"$binary" scan >"$work/scan.txt" 2>/dev/null
warm_ms=$(median_ms "$binary" scan)

elements=$(wc -l < "$work/scan.txt")
scan_bytes=$(wc -c < "$work/scan.txt")
scan_tokens=$(( (scan_bytes + 3) / 4 ))
capture_ms=$(grep -o 'capture [0-9]*ms' "$work/daemon.log" | grep -o '[0-9]*' \
  | sort -n | awk '{ v[NR]=$1 } END { print v[int((NR+1)/2)] }')

cat <<EOF
Screen:          ${width}x${height}
Elements:        ${elements}
Runs per figure: ${runs} (median)

| Measure                       | screenpeek | screenshot to a vision model |
| ----------------------------- | ---------- | ---------------------------- |
| Bytes produced                | ${scan_bytes} | ${shot_bytes} |
| Tokens for one look           | ~${scan_tokens} | ~${shot_tokens} |
| Capture                       | ${capture_ms} ms | ${shot_ms} ms |
| One look, no daemon           | ${cold_ms} ms | ${shot_ms} ms |
| One look, daemon running      | ${warm_ms} ms | ${shot_ms} ms |

Daemon log (what each request did):
$(grep -E '^(full|patched|unchanged)' "$work/daemon.log" | tail -6)
EOF
