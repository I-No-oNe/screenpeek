#!/usr/bin/env bash
# Checks screenpeek's GNOME path in a headless GNOME Shell: the extension's
# window list and text input, the accessibility tree and screenshot scans.
# Input and the screen stream need a permission dialog, so they stay manual.
set -euo pipefail

root=$(cd "$(dirname "$0")/.." && pwd)
screenpeek=${SCREENPEEK:-$root/target/release/screenpeek}

if [ "${1-}" != "--inside" ]; then
  work=$(mktemp -d)
  trap 'fusermount -u "$work/run/doc" 2>/dev/null; rm -rf "$work"' EXIT
  mkdir -m 700 "$work/run"
  # Its own session bus and folders, so the desktop running this is untouched.
  XDG_CONFIG_HOME=$work/config XDG_DATA_HOME=$work/data XDG_CACHE_HOME=$work/cache \
    XDG_STATE_HOME=$work/state XDG_RUNTIME_DIR=$work/run \
    XDG_CURRENT_DESKTOP=GNOME XDG_SESSION_TYPE=wayland WAYLAND_DISPLAY=screenpeek-ci \
    GDK_BACKEND=wayland SCREENPEEK=$screenpeek \
    dbus-run-session -- env -u DISPLAY bash "$0" --inside
  exit
fi

fail() { echo "FAIL: $*" >&2; exit 1; }
# Stop what this session started, so nothing holds the terminal open afterwards.
trap 'kill $(jobs -p) 2>/dev/null' EXIT
wait_for() { # seconds, command...
  local deadline=$((SECONDS + $1)); shift
  until "$@" >/dev/null 2>&1; do
    [ $SECONDS -lt $deadline ] || return 1
    sleep 0.5
  done
}

dbus-update-activation-environment --all
/usr/libexec/at-spi-bus-launcher --launch-immediately &
sh "$root/helpers/gnome/install.sh" >/dev/null
gsettings set org.gnome.shell disable-user-extensions false
# A fresh profile greets with a tour dialog that takes keyboard focus.
gsettings set org.gnome.shell welcome-dialog-last-shown-version '9999' 
gnome-shell --headless --wayland --no-x11 --wayland-display screenpeek-ci \
  --virtual-monitor 1280x800 >"$XDG_RUNTIME_DIR/shell.log" 2>&1 &
wait_for 30 test -S "$XDG_RUNTIME_DIR/screenpeek-ci" || fail "gnome-shell did not start"
wait_for 30 gdbus introspect --session -d org.screenpeek.Windows -o /org/screenpeek/Windows \
  || fail "the screenpeek extension did not load"
/usr/libexec/at-spi2-registryd &

# The portal names a host program after its systemd scope, or "" without one.
scope=$(sed -n 's|.*/||p' /proc/self/cgroup | head -1)
app=${scope#app-}; app=${app%.scope}; app=${app%-*}
case $app in *-*) app=${app#*-} ;; esac
for id in "" "$app"; do
  busctl --user call org.freedesktop.impl.portal.PermissionStore \
    /org/freedesktop/impl/portal/PermissionStore org.freedesktop.impl.portal.PermissionStore \
    SetPermission sbssas screenshot true screenshot "$id" 1 yes
done

gnome-text-editor --standalone >/dev/null 2>&1 &
wait_for 30 sh -c "'$SCREENPEEK' windows | grep -q 'Text Editor'" || fail "windows does not list the editor"
echo "ok   windows"

"$SCREENPEEK" focus 'Text Editor' >/dev/null
wait_for 10 sh -c "'$SCREENPEEK' tree | grep -q 'Text Editor'" || fail "tree is empty"
echo "ok   tree"

commit() {
  [ "$(gdbus call --session -d org.screenpeek.Windows -o /org/screenpeek/Windows \
    -m org.screenpeek.Windows.Commit 'שלום')" = "(true,)" ]
}
# The editor's text field takes input focus a moment after the window does.
wait_for 10 commit || fail "the extension could not type"
wait_for 10 sh -c "'$SCREENPEEK' windows | grep -q 'שלום'" || fail "typed Hebrew did not arrive"
echo "ok   typing through the extension"

SCREENPEEK_NO_DAEMON=1 "$SCREENPEEK" scan --focused | grep -q . || fail "scan found nothing"
echo "ok   scan"

"$SCREENPEEK" doctor | tee "$XDG_RUNTIME_DIR/doctor.txt"
grep -q '^ok   windows' "$XDG_RUNTIME_DIR/doctor.txt" || fail "doctor reports a problem"
echo "all GNOME checks passed"
