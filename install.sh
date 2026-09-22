#!/usr/bin/env sh
# Installs the latest screenpeek release into ~/.local/bin (override with PREFIX).
set -eu

repo=I-No-oNe/screenpeek
prefix=${PREFIX:-$HOME/.local/bin}

case "$(uname -s)-$(uname -m)" in
  Linux-x86_64) archive=screenpeek-x86_64-linux.tar.gz ;;
  *) echo "no prebuilt binary for $(uname -s)-$(uname -m); use: cargo install --git https://github.com/$repo" >&2; exit 1 ;;
esac

tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT

# gh is used when it is there, since it also works for a private repository.
if command -v gh >/dev/null 2>&1; then
  tag=$(gh release list --repo "$repo" --limit 1 --json tagName --jq '.[0].tagName')
  [ -n "$tag" ] || { echo "no release found" >&2; exit 1; }
  gh release download "$tag" --repo "$repo" --pattern "$archive" --dir "$tmp"
else
  tag=$(curl -fsSL "https://api.github.com/repos/$repo/releases?per_page=1" \
    | sed -n 's/.*"tag_name": *"\([^"]*\)".*/\1/p' | head -1)
  [ -n "$tag" ] || { echo "no release found" >&2; exit 1; }
  curl -fsSL -o "$tmp/$archive" "https://github.com/$repo/releases/download/$tag/$archive"
fi

tar -xzf "$tmp/$archive" -C "$tmp"
mkdir -p "$prefix"
install -m 755 "$tmp/screenpeek" "$prefix/screenpeek"
echo "installed $tag to $prefix/screenpeek"
"$prefix/screenpeek" --version
