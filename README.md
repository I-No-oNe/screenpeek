# screenpeek

**See desktop apps as text. Click things by name.**

```sh
$ screenpeek scan --grep Save
7 Save @412,318
$ screenpeek click Save
7 Save @412,318
```

screenpeek lets AI agents and scripts drive any desktop app without
screenshots. It reads what is on screen (with local OCR and the app's own
accessibility labels) and gives back short lines of text. Then it clicks,
types, scrolls and switches windows for you.

It is built for **navigating and testing workflows quickly**. For questions
about how something *looks* (layout, colours, images), keep using screenshots.

## Why use it

- **Much faster.** A step takes about 0.1 s locally instead of a
  screenshot plus a round trip to a vision model.
- **Much cheaper.** `Save @412,318` is 7 tokens; a screenshot is around 1,000.
- **Works for any model.** Text-only models get "computer use" too.
- **Clicks the right thing.** It clicks the exact centre of a named element,
  and refuses to guess when a name matches several things.
- **Private.** Nothing leaves your machine; no API keys.

Compared with other tools: xdotool and ydotool need coordinates you don't
have; dogtail and pyatspi only see apps with accessibility support; image
matching breaks when the theme changes. screenpeek combines all three sources.

## Get started

**1. Install**

```sh
git clone https://github.com/I-No-oNe/screenpeek.git
cd screenpeek
sh install.sh
```

On Windows, in PowerShell:

```powershell
irm https://raw.githubusercontent.com/I-No-oNe/screenpeek/main/install.ps1 | iex
```

The installer asks which extra languages to read (Hebrew, Arabic, Chinese...).
Press Enter for none; you can change it later.

**2. Connect your agent**

```sh
sh scripts/install-skills.sh                  # Claude Code and/or Codex
claude mcp add screenpeek -- screenpeek mcp   # or use it as an MCP server
```

**3. Ask your agent to use it**, for example: *"Open Settings with screenpeek
and turn on dark mode."*

## What it can do

| You want to | Command |
| --- | --- |
| See what is on screen | `screenpeek scan --focused` |
| Find one thing | `screenpeek scan --grep Save` |
| Click it | `screenpeek click Save` |
| Type into a field | `screenpeek fill Search "cats"` |
| Press keys | `screenpeek key ctrl+s` |
| Wait for something | `screenpeek wait Saved` |
| Scroll or drag | `screenpeek scroll down 5` · `screenpeek drag A B` |
| Switch windows | `screenpeek windows` · `screenpeek focus Firefox` |
| Run several steps | `screenpeek run "click File" "click Save" "wait Saved"` |

Full guide: [docs/usage.md](docs/usage.md).

## Works on

Linux (Hyprland, Sway, GNOME, KDE Plasma, any X11 desktop) and Windows.
GNOME needs a small extension for window positions; see the guide.

[Guide](docs/usage.md) · [Languages](docs/languages.md) ·
[Speed and accuracy](docs/performance.md) · [MIT license](LICENSE)
