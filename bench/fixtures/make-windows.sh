#!/usr/bin/env bash
# Regenerate adjacent Save/Cancel controls separated by a window edge at x=430.
set -euo pipefail
cd "$(dirname "$0")"

font=${FONT:-/usr/share/fonts/liberation/LiberationSans-Regular.ttf}
magick -size 700x200 xc:'#dcdcdc' \
  -fill '#ffffff' -stroke '#b0b0b0' \
  -draw "rectangle 20,20 429,180" \
  -draw "rectangle 431,20 680,180" \
  -stroke none -fill '#1a1a1a' -font "$font" -pointsize 13 \
  -draw "text 396,60 'Save'" \
  -draw "text 434,60 'Cancel'" \
  -draw "text 40,100 'Untitled'" \
  windows.png
