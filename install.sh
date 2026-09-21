#!/usr/bin/env sh
# Installs the latest screenpeek release into ~/.local/bin (override with PREFIX).
set -eu

repo=I-No-oNe/screenpeek
prefix=${PREFIX:-$HOME/.local/bin}

case "$(uname -s)" in
  Linux) archive=screenpeek-x86_64-linux.tar.gz ;;
  *) echo "no prebuilt binary for $(uname -s); use: cargo install --git https://github.com/$repo" >&2; exit 1 ;;
esac

tag=$(curl -fsSL "https://api.github.com/repos/$repo/releases/latest" | sed -n 's/.*"tag_name": *"\([^"]*\)".*/\1/p')
[ -n "$tag" ] || { echo "no release found" >&2; exit 1; }

tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT
curl -fsSL "https://github.com/$repo/releases/download/$tag/$archive" | tar -xz -C "$tmp"

mkdir -p "$prefix"
install -m 755 "$tmp/screenpeek" "$prefix/screenpeek"
echo "installed $tag to $prefix/screenpeek"
"$prefix/screenpeek" --version
