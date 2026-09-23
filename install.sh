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

# Shared libraries the binary cannot find, named by the package that has them.
missing=""
for lib in $(ldd "$prefix/screenpeek" 2>/dev/null | awk '/not found/ {print $1}'); do
  case $lib in
    libxkbcommon*) missing="$missing xkbcommon" ;;
    *) missing="$missing $lib" ;;
  esac
done
[ -n "$missing" ] || "$prefix/screenpeek" --version

# Offer extra languages when someone is at the terminal to answer.
if [ -t 0 ] && [ -t 1 ]; then
  here=$(CDPATH= cd -- "$(dirname -- "$0")" 2>/dev/null && pwd || echo .)
  if [ -f "$here/scripts/fetch-models.sh" ]; then
    bash "$here/scripts/fetch-models.sh"
  else
    curl -fsSL "https://raw.githubusercontent.com/$repo/main/scripts/fetch-models.sh" -o "$tmp/fetch-models.sh"
    bash "$tmp/fetch-models.sh"
  fi
fi

# Other languages are read with tesseract, which comes from the system.
languages=${XDG_CONFIG_HOME:-$HOME/.config}/screenpeek/languages
if [ -s "$languages" ] && ! command -v tesseract >/dev/null 2>&1; then
  missing="$missing tesseract"
fi

if [ -n "$missing" ]; then
  # Checked in this order so rpm-ostree wins on Silverblue, dnf over yum.
  if command -v rpm-ostree >/dev/null 2>&1; then
    install="sudo rpm-ostree install" xkb=libxkbcommon tess=tesseract
  elif command -v dnf >/dev/null 2>&1; then
    install="sudo dnf install" xkb=libxkbcommon tess=tesseract
  elif command -v yum >/dev/null 2>&1; then
    install="sudo yum install" xkb=libxkbcommon tess=tesseract
  elif command -v apt-get >/dev/null 2>&1; then
    install="sudo apt-get install" xkb=libxkbcommon0 tess=tesseract-ocr
  elif command -v pacman >/dev/null 2>&1; then
    install="sudo pacman -S" xkb=libxkbcommon tess=tesseract
  elif command -v zypper >/dev/null 2>&1; then
    install="sudo zypper install" xkb=libxkbcommon0 tess=tesseract-ocr
  elif command -v xbps-install >/dev/null 2>&1; then
    install="sudo xbps-install" xkb=libxkbcommon tess=tesseract-ocr
  elif command -v apk >/dev/null 2>&1; then
    install="sudo apk add" xkb=libxkbcommon tess=tesseract-ocr
  elif command -v emerge >/dev/null 2>&1; then
    install="sudo emerge" xkb=x11-libs/libxkbcommon tess=app-text/tesseract
  elif command -v eopkg >/dev/null 2>&1; then
    install="sudo eopkg install" xkb=libxkbcommon tess=tesseract
  else
    install="install with your package manager:" xkb=libxkbcommon tess=tesseract
  fi
  packages=""
  for name in $missing; do
    case $name in
      xkbcommon) packages="$packages $xkb" ;;
      tesseract) packages="$packages $tess" ;;
      *) packages="$packages $name" ;;
    esac
  done
  echo
  echo "screenpeek still needs some packages. Install them with:"
  echo "  $install$packages"
fi
