#!/usr/bin/env sh
# Install the shared skills for a local coding agent.
set -eu
root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
case "${1:-codex}" in
  codex) destination=${SCREENPEEK_SKILLS_DIR:-$HOME/.agents/skills} ;;
  claude) destination=${SCREENPEEK_SKILLS_DIR:-$HOME/.claude/skills} ;;
  *) echo "usage: $0 [codex|claude]" >&2; exit 2 ;;
esac
for skill in screenpeek screenpeek-drive; do
  mkdir -p "$destination/$skill"
  cp "$root/skill/$skill/SKILL.md" "$destination/$skill/SKILL.md"
done
printf "Installed screenpeek skills in %s; start a new agent session.\n" "$destination"
