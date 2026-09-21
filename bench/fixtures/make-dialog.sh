#!/usr/bin/env bash
# Regenerates dialog.png, a synthetic settings window drawn at the sizes a real
# one uses: 13 px body text, 15 px headings, on a 900x560 window. Deterministic,
# so the accuracy numbers in BENCHMARK.md can be reproduced.
set -euo pipefail
cd "$(dirname "$0")"

font=${FONT:-/usr/share/fonts/liberation/LiberationSans-Regular.ttf}
magick -size 900x560 xc:'#f6f6f6' \
  -fill '#1a1a1a' -font "$font" -pointsize 15 \
  -draw "text 24,40 'Preferences'" \
  -fill '#333333' -pointsize 13 \
  -draw "text 24,80 'Appearance'" \
  -draw "text 24,110 'Theme'" \
  -draw "text 200,110 'Automatic'" \
  -draw "text 24,140 'Accent colour'" \
  -draw "text 200,140 'Blue'" \
  -draw "text 24,180 'Editor'" \
  -draw "text 24,210 'Font family'" \
  -draw "text 200,210 'JetBrains Mono'" \
  -draw "text 24,240 'Font size'" \
  -draw "text 200,240 '13'" \
  -draw "text 24,270 'Tab width'" \
  -draw "text 200,270 '4 spaces'" \
  -draw "text 24,310 'Privacy'" \
  -draw "text 24,340 'Send usage statistics'" \
  -draw "text 24,370 'Check for updates automatically'" \
  -draw "text 24,400 'Restore previous session on launch'" \
  -fill '#ffffff' -stroke '#c8c8c8' \
  -draw "roundrectangle 560,480 660,514 4,4" \
  -draw "roundrectangle 680,480 780,514 4,4" \
  -draw "roundrectangle 800,480 876,514 4,4" \
  -stroke none -fill '#1a1a1a' \
  -draw "text 588,503 'Cancel'" \
  -draw "text 716,503 'Apply'" \
  -draw "text 826,503 'Save'" \
  dialog.png
