#!/usr/bin/env bash
# Download OCR models and optional Tesseract languages without root.
set -euo pipefail

models="${XDG_CACHE_HOME:-$HOME/.cache}/screenpeek"
tessdata="${TESSDATA_PREFIX:-${XDG_DATA_HOME:-$HOME/.local/share}/tessdata}"
config="${XDG_CONFIG_HOME:-$HOME/.config}/screenpeek/languages"
ocrs_url="https://ocrs-models.s3-accelerate.amazonaws.com"
tess_url="https://github.com/tesseract-ocr/tessdata_fast/raw/main"
# Offered at the prompt, and what --all installs. Tesseract has around a
# hundred more; any code from `tesseract --list-langs` works as an argument.
all_langs="ara chi_sim deu fra heb jpn rus spa por ita nld pol tur ukr kor vie"
describe() {
  case "$1" in
    ara) echo "Arabic";;      chi_sim) echo "Chinese, simplified";;
    deu) echo "German";;      fra) echo "French";;
    heb) echo "Hebrew";;      jpn) echo "Japanese";;
    rus) echo "Russian";;     spa) echo "Spanish";;
    por) echo "Portuguese";;  ita) echo "Italian";;
    nld) echo "Dutch";;       pol) echo "Polish";;
    tur) echo "Turkish";;     ukr) echo "Ukrainian";;
    kor) echo "Korean";;      vie) echo "Vietnamese";;
    *) echo "";;
  esac
}

# Asks which languages to add, when nothing was named and someone is there to
# answer. English is always read alongside them and is never in this list.
choose_languages() {
  local reply picked=() i=1
  echo "screenpeek reads English on its own." >&2
  echo "Pick extra languages to read (Hebrew, Arabic, Chinese...), or none." >&2
  echo "They are only used when such text is on screen, so English stays fast." >&2
  echo >&2
  for lang in $all_langs; do
    local mark="  "
    [ -s "$tessdata/$lang.traineddata" ] && mark="* "
    printf '%s%2d) %-8s %s\n' "$mark" "$i" "$lang" "$(describe "$lang")" >&2
    i=$((i + 1))
  done
  echo >&2
  echo "  * already installed. Enter numbers or codes, space separated." >&2
  echo "  Enter for none, 'all' for every one listed." >&2
  printf 'languages: ' >&2
  read -r reply || reply=""

  [ -z "$reply" ] && { echo "none added." >&2; return; }
  [ "$reply" = "all" ] && { printf '%s' "$all_langs"; return; }
  for word in $reply; do
    if [ "$word" -eq "$word" ] 2>/dev/null; then
      picked+=("$(printf '%s' "$all_langs" | tr ' ' '\n' | sed -n "${word}p")")
    else
      picked+=("$word")
    fi
  done
  printf '%s' "${picked[*]}"
}

fetch() { # url path
  [ -s "$2" ] && { echo "have    $(basename "$2")"; return; }
  echo "fetch   $(basename "$2")"
  curl -sfL --retry 3 -o "$2.partial" "$1"
  mv "$2.partial" "$2"      # rename, so an interrupted download leaves nothing
}

mkdir -p "$models"
for model in text-detection.rten text-recognition.rten; do
  fetch "$ocrs_url/$model" "$models/$model"
done

langs=("$@")
case "${1-}" in
  --all) read -ra langs <<<"$all_langs" ;;
  --none) langs=() ;;
  "") # Ask, but only when there is a terminal to ask at: a script or a CI
      # run gets the built-in models and nothing else.
     if [ -t 0 ] && [ -t 2 ]; then
       echo
       read -ra langs <<<"$(choose_languages)"
       asked=1
     fi ;;
esac
if [ ${#langs[@]} -eq 0 ]; then
  # Forget an earlier choice only when someone chose none just now.
  if [ "${1-}" = "--none" ] || [ -n "${asked-}" ]; then rm -f "$config"; fi
  echo "built-in models ready in $models"
  exit 0
fi

mkdir -p "$tessdata"
for lang in "${langs[@]}"; do
  [[ "$lang" =~ ^[a-zA-Z0-9_]+$ ]] || { echo "invalid language code: $lang" >&2; exit 2; }
  fetch "$tess_url/$lang.traineddata" "$tessdata/$lang.traineddata"
done

[ -e "$tessdata/eng.traineddata" ] || fetch "$tess_url/eng.traineddata" "$tessdata/eng.traineddata"

mkdir -p "$(dirname "$config")"
printf '%s\n' "${langs[*]}" > "$config"

echo
echo "screenpeek will read: English ${langs[*]}"
command -v tesseract >/dev/null 2>&1 || echo "install tesseract with your package manager to use them (e.g. sudo dnf install tesseract)"
if [ -n "${TESSDATA_PREFIX:-}" ] || [ "$tessdata" != "${XDG_DATA_HOME:-$HOME/.local/share}/tessdata" ]; then
  echo "language data in $tessdata"
fi
