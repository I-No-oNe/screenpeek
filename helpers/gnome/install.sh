#!/usr/bin/env sh
# Install the GNOME geometry extension for the current user.
set -eu
uuid=screenpeek@screenpeek
source_dir=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
destination=${XDG_DATA_HOME:-$HOME/.local/share}/gnome-shell/extensions/$uuid
mkdir -p "$destination"
cp "$source_dir/metadata.json" "$source_dir/extension.js" "$destination/"
# GNOME loads new extensions only at login, so mark it enabled for then.
enabled=$(gsettings get org.gnome.shell enabled-extensions 2>/dev/null || echo "@as []")
case $enabled in
  *"'$uuid'"*) ;;
  "@as []") gsettings set org.gnome.shell enabled-extensions "['$uuid']" ;;
  *) gsettings set org.gnome.shell enabled-extensions "${enabled%]}, '$uuid']" ;;
esac
printf "Installed %s\nLog out and back in to load it.\n" "$destination"
