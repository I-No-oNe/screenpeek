# screenpeek

**Read desktop controls as text. Click them by name.** `0.0.1-alpha`

```sh
screenpeek scan --grep "Save"
# 7 Save @412,318
screenpeek click "Save" --fresh
screenpeek fill "Search" "hello world" --fresh
```

Screenpeek gives coding agents and scripts a text interface to desktop apps.
It combines local OCR, accessibility labels and window positions, then returns
IDs and coordinates. No vision API, API key or screenshot upload is needed.

Use it for labelled buttons, menus, fields and accessible icons. Keep screenshots
for visual judgment, canvas content and icons without accessibility names;
use a browser DOM or an application API when available.

## Why use it

| Task | What screenpeek changes | Evidence |
| --- | --- | --- |
| Find a known label | Return a short text result instead of an image | `Save`: 7 text tokens vs an estimated 765 high-detail image tokens |
| Re-read a still desktop | Reuse recognized text after capture | About 60–70 ms on the measured Hyprland session |
| Read a changed multilingual screen | Recognize only the changed region | Hebrew desktop read: 1,083–1,160 ms → 264–323 ms |
| Work across toolkits | Combine accessible controls with OCR | Read a browser with no exposed AT-SPI tree; name icon-only GTK buttons |
| Click on Wayland | Add compositor window positions to accessibility coordinates | Corrected the shared-label offset in the dogtail comparison |

These are local measurements, not a universal speed or accuracy ranking.
The full dialog listing is 156 tokens, so its reduction is about **4.9×**;
the **109×** figure applies only to the filtered `Save` result.
[Benchmarks, comparison limits and reproduction commands](docs/performance.md).

## Install

Download a binary from [Releases](https://github.com/I-No-oNe/screenpeek/releases),
or clone and run the installer:

```sh
git clone https://github.com/I-No-oNe/screenpeek.git
cd screenpeek
sh install.sh
```

For a source build, use Rust 1.94+ and the [build dependencies](docs/usage.md):

```sh
cargo install --path . --locked
```

OCR models download on first use. Additional languages need Tesseract:

```sh
bash scripts/fetch-models.sh heb jpn
export TESSDATA_PREFIX="$HOME/.local/share/tessdata"
screenpeek scan --lang heb
```

## Codex and Claude Code

Install the CLI, then install its two skills from this checkout:

```sh
sh scripts/install-skills.sh codex
# Or: sh scripts/install-skills.sh claude
```

In a new local Codex session, ask `$screenpeek` to read the desktop or
`$screenpeek-drive` to operate an app. The skills use shell commands; no MCP
server is required. Codex must run in the logged-in desktop session with access
to its display and session bus. A cloud agent cannot see your local desktop.
[Codex skill locations](https://learn.chatgpt.com/docs/build-skills).

## Platforms

| Desktop | Capture and input | Status |
| --- | --- | --- |
| Linux Hyprland / Sway | Native Wayland protocols | Local Hyprland measurements; Sway code paths and parser tests |
| Linux X11 | X11 | Implemented; desktop validation still needed |
| GNOME / KDE Wayland | Screenshot and RemoteDesktop portals | Experimental; input requires desktop consent |
| Windows | UI Automation, OCR fallback, native input | CI build/tests; interactive validation still needed |

GNOME window geometry needs the included extension; KDE uses an included
one-shot KWin script. [Fedora setup and validation](docs/usage.md#fedora-gnome-and-kde).
The portal capture path was tested on Hyprland; GNOME/KDE end-to-end operation
still needs a real session. Mixed-scale multi-monitor portal captures remain
unverified.

This alpha can misread labels or use stale positions. Ambiguous targets fail;
use `--fresh` after a layout change and check the result of each action.
Arabic is the weakest tested script. [Language results](docs/languages.md).

[Commands and setup](docs/usage.md) · [Measurements](docs/performance.md) · [MIT](LICENSE)
