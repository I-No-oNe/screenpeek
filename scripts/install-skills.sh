#!/usr/bin/env sh
# Install the screenpeek skill for Codex and/or Claude Code.
# usage: install-skills.sh [codex|claude|all] [--link]
set -eu
root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
targets= link=
for arg; do
  case $arg in
    codex|claude) targets="$targets $arg" ;;
    all) targets="codex claude" ;;
    --link) link=1 ;;
    *) echo "usage: $0 [codex|claude|all] [--link]" >&2; exit 2 ;;
  esac
done
if [ -z "$targets" ]; then
  command -v codex >/dev/null 2>&1 && targets=codex
  command -v claude >/dev/null 2>&1 && targets="$targets claude"
  [ -n "$targets" ] || targets="codex claude"
fi
for agent in $targets; do
  case $agent in
    codex) dir=${SCREENPEEK_SKILLS_DIR:-$HOME/.agents/skills} ;;
    claude) dir=${SCREENPEEK_SKILLS_DIR:-$HOME/.claude/skills} ;;
  esac
  mkdir -p "$dir"
  rm -rf "$dir/screenpeek" "$dir/screenpeek-drive"
  if [ -n "$link" ]; then
    ln -s "$root/skill/screenpeek" "$dir/screenpeek"
  else
    cp -R "$root/skill/screenpeek" "$dir/screenpeek"
  fi
  echo "$agent: $dir/screenpeek"
done
command -v screenpeek >/dev/null 2>&1 \
  || echo "warning: screenpeek is not on PATH; run sh install.sh first" >&2
echo "Start a new agent session to load the skill."
