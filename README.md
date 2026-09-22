# screenpeek

**Read desktop controls as text. Click them by name.**

```sh
screenpeek scan --grep "Save"
# 7 Save @412,318
screenpeek click "Save" --fresh
screenpeek fill "Search" "hello world" --fresh
```

Screenpeek gives agents and scripts a text interface to any desktop app. It
merges local OCR, accessibility labels and compositor window positions into a
numbered list of `id text @x,y`, then clicks by name or ID.

## Why screenpeek

**vs. vanilla computer use** (screenshot → vision model → guessed coordinates):

- **Tiny observations.** `scan --grep Save` returns ~7 tokens; a screenshot of
  the same dialog costs ~765. A full dialog listing is ~156.
- **Fast and local.** A warm scan takes tens to a few hundred ms on the CPU. No
  model round trip per step, no API key, no screenshots leave the machine.
- **Exact targets.** Clicks land on the centre of a named element. Ambiguous
  names fail with a list of candidates instead of clicking the wrong one.

**vs. other tools:**

| Tool | Gap screenpeek fills |
| --- | --- |
| xdotool / ydotool | Need coordinates; screenpeek finds them by label |
| dogtail / pyatspi | Tree only: miss apps with no accessibility (Chromium, many Electron apps) and report window-relative positions on Wayland |
| pyautogui / template matching | Break on themes, scaling and font changes |
| Cloud OCR / vision APIs | Upload your screen and add network latency |

Use screenshots for visual judgment, canvases and unlabelled icons. Use the DOM
or an app API when one is available.

## Install

```sh
git clone https://github.com/I-No-oNe/screenpeek.git
cd screenpeek
sh install.sh                     # latest release into ~/.local/bin
# or build it: cargo install --path . --locked
```

OCR models download on first use. For other languages, see
[languages](docs/languages.md).

## Replace computer use in your agent

```sh
sh scripts/install-skills.sh          # Codex and/or Claude Code, whichever is installed
sh scripts/install-skills.sh --link   # symlink, so `git pull` updates the skill
```

The `screenpeek` skill tells the agent to reach for screenpeek before taking a
screenshot: scan for the label, click or fill it by name, then verify with
another scan. Screenshots stay the fallback for visual questions. Start a new
agent session afterwards; it must run inside the logged-in desktop session.

Agents without skills can use the MCP server instead:

```sh
claude mcp add screenpeek -- screenpeek mcp
codex mcp add screenpeek -- screenpeek mcp
```

To make it explicit, add to your `AGENTS.md` or `CLAUDE.md`:

> For desktop apps, use `screenpeek` (scan, click, fill, key, run) instead of
> screenshots or computer use. Only take a screenshot when the answer depends
> on how something looks.

## Platforms

| Desktop | Capture and input |
| --- | --- |
| Hyprland, Sway | Native Wayland protocols |
| GNOME, KDE Plasma (Wayland) | Screenshot and RemoteDesktop portals, asked once |
| X11 (any window manager) | X11 |
| Windows | UI Automation, OCR fallback |

GNOME needs a small extension for window positions; KDE needs nothing.
See [setup](docs/usage.md#gnome-and-kde).

OCR can misread labels and cached positions go stale: use `--fresh` after the
layout changes and check each action's result.

[Usage](docs/usage.md) · [Measurements](docs/performance.md) · [MIT](LICENSE)
