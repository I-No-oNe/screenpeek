#!/usr/bin/env sh
# Install the GNOME geometry extension for the current user.
set -eu
source_dir=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
destination=${XDG_DATA_HOME:-$HOME/.local/share}/gnome-shell/extensions/screenpeek@screenpeek
mkdir -p "$destination"
cp "$source_dir/metadata.json" "$source_dir/extension.js" "$destination/"
printf "Installed %s\nLog out and back in, then run:\n  gnome-extensions enable screenpeek@screenpeek\n" "$destination"
