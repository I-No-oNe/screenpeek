#!/usr/bin/env bash
# Checks input on a wlroots desktop at a fractional scale: headless Sway at
# 1.25, with wev recording where a click lands and which characters arrive.
set -euo pipefail

root=$(cd "$(dirname "$0")/.." && pwd)
screenpeek=${SCREENPEEK:-$root/target/release/screenpeek}
work=$(mktemp -d)
trap 'kill $(jobs -p) 2>/dev/null; rm -rf "$work"' EXIT
export XDG_RUNTIME_DIR=$work XDG_CACHE_HOME=$work/cache
unset DISPLAY WAYLAND_DISPLAY
log=$work/wev.log

fail() {
  echo "FAIL: $*" >&2
  tail -60 "$log" >&2 || true
  exit 1
}
wait_for() { # seconds, command...
  local deadline=$((SECONDS + $1)); shift
  until "$@" >/dev/null 2>&1; do
    [ $SECONDS -lt $deadline ] || return 1
    sleep 0.2
  done
}

cat >"$work/config" <<EOF
output HEADLESS-1 resolution 1920x1080 scale 1.25
default_border none
for_window [app_id="wev"] fullscreen enable
EOF
WLR_BACKENDS=headless WLR_RENDERER=pixman WLR_LIBINPUT_NO_DEVICES=1 \
  sway -c "$work/config" >"$work/sway.log" 2>&1 &
wait_for 30 sh -c "ls '$work' | grep -q '^wayland-[0-9]*\$'" || { cat "$work/sway.log"; fail "sway did not start"; }
export WAYLAND_DISPLAY=$(ls "$work" | grep '^wayland-[0-9]*$' | head -1)
export SWAYSOCK=$(ls "$work"/sway-ipc.*.sock)

# Line-buffered: a file gets block-buffered output, which hides the last events.
stdbuf -oL wev >"$log" 2>&1 &
# Fullscreen at 1.25 makes wev's surface the whole 1536x864 logical output,
# so the positions it reports are desktop positions.
wait_for 30 sh -c "swaymsg -t get_tree | grep -q '\"width\": 1536'" || fail "wev did not open fullscreen"
sleep 1

SCREENPEEK_DESKTOP_TEST=1 cargo test --release --locked --bin screenpeek \
  pointer::tests::click_reaches_the_desktop -- --ignored --exact
"$screenpeek" type 'about:config'
sleep 1

python3 - "$log" <<'PY' || fail "input did not arrive as sent"
import re, sys
position = clicked = None
typed, pressed = "", False
for line in open(sys.argv[1], errors="replace"):
    if "wl_pointer" in line and (m := re.search(r"x, y: ([-\d.]+), ([-\d.]+)", line)):
        position = (float(m[1]), float(m[2]))
    if "wl_pointer" in line and "button:" in line and "pressed" in line and clicked is None:
        clicked = position
    if "wl_keyboard" in line and "key:" in line:
        pressed = "state: 1" in line
    if pressed and (m := re.search(r"utf8: '(.*)'", line)):
        typed += m[1]
        pressed = False
print(f"click landed at {clicked}, typed {typed!r}")
assert clicked and abs(clicked[0] - 283) <= 1 and abs(clicked[1] - 649) <= 1, "click missed 283,649"
assert typed.endswith("about:config"), "typed text lost characters"
PY
echo "all Sway checks passed"
