#!/usr/bin/env bash
# How much of a known interface screenpeek reads back correctly, at each
# magnification. The fixture is bench/fixtures/dialog.png, drawn at real UI
# text sizes by make-dialog.sh, and dialog.expected lists every label in it.
set -euo pipefail

binary=${SCREENPEEK:-target/release/screenpeek}
fixture=bench/fixtures/dialog.png
expected=bench/fixtures/dialog.expected
[ -x "$binary" ] || { echo "build first: cargo build --release" >&2; exit 1; }

total=$(wc -l < "$expected")
echo "| Magnification | Labels read exactly | Recall | Time (ms) |"
echo "| ------------- | ------------------- | ------ | --------- |"

for scale in 1 2 3; do
  start=$(date +%s%N)
  got=$("$binary" read "$fixture" --scale "$scale" | sed 's/^[0-9]* //; s/ @[-0-9]*,[-0-9]*$//')
  elapsed=$(( ($(date +%s%N) - start) / 1000000 ))

  hits=0
  while IFS= read -r label; do
    grep -qxF "$label" <<<"$got" && hits=$((hits + 1))
  done < "$expected"

  awk -v s="$scale" -v h="$hits" -v t="$total" -v ms="$elapsed" \
    'BEGIN { printf "| %dx | %d/%d | %.0f%% | %d |\n", s, h, t, 100 * h / t, ms }'
done
