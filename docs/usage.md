# Guide

- [Install](#install)
- [Look at the screen](#look-at-the-screen)
- [Click, type and move](#click-type-and-move)
- [Windows](#windows)
- [Several steps at once](#several-steps-at-once)
- [Agents and MCP](#agents-and-mcp)
- [GNOME and KDE](#gnome-and-kde)
- [Browsers](#browsers)
- [Settings](#settings)
- [Troubleshooting](#troubleshooting)

## Install

**Linux (x86-64):**
`curl -fsSL https://raw.githubusercontent.com/I-No-oNe/screenpeek/main/install.sh | sh`
installs to `~/.local/bin` (set `PREFIX` to change it) and offers the agent
skill and extra languages. No clone needed.

**Windows (x64 and ARM64):** in PowerShell,
`irm https://raw.githubusercontent.com/I-No-oNe/screenpeek/main/install.ps1 | iex`
installs to `%LOCALAPPDATA%\screenpeek\bin` (set `SCREENPEEK_PREFIX` to change
it), adds it to your PATH, and offers the agent skill and extra languages. With
[Scoop](https://scoop.sh):

```powershell
scoop bucket add screenpeek https://github.com/I-No-oNe/screenpeek
scoop install screenpeek
```

Both installers check each download against the `.sha256` file published next
to it.

**From source:** needs a recent Rust (see `rust-version` in `Cargo.toml`).

```sh
# Fedora
sudo dnf install gcc gcc-c++ pkgconf-pkg-config wayland-devel libxkbcommon-devel libxdo-devel
# Debian / Ubuntu
sudo apt-get install pkg-config libwayland-dev libxkbcommon-dev libxdo-dev

cargo install --path . --locked
bash scripts/fetch-models.sh     # optional: pick extra languages
```

On KDE and GNOME, also build the frame helper, which makes scans much faster.
It is the only part that needs PipeWire:

```sh
sudo dnf install pipewire-devel clang              # Fedora
sudo apt-get install libpipewire-0.3-dev clang     # Debian / Ubuntu
cargo install --path frames --locked
```

The OCR models (about 12 MB) download on first use.

## Look at the screen

```sh
screenpeek scan                  # everything on the current monitor
screenpeek scan --focused        # only the focused window
screenpeek scan --grep save      # only lines containing "save"
screenpeek scan --json           # adds size, role and states
```

Each line is `id text @x,y`, where `x,y` is where a click lands. When the app
reports it, states follow: `12 Dark mode @300,200 [checked]`.

Narrow a scan with `--focused`, `--region X,Y,W,H` or `--monitor N`.

## Click, type and move

```sh
screenpeek click Save            # by name
screenpeek click 7               # by id from the last scan
screenpeek click Save --fresh    # scan again first (after the layout changed)
screenpeek click Save --check    # warn if nothing changed after clicking
screenpeek click File --button right
screenpeek fill Search "cats"    # click a field, then type (does not clear it)
screenpeek fill Amount -5        # text may start with a dash
screenpeek type "hello"
screenpeek key ctrl+s            # also alt+F4, enter, slash, F5...
screenpeek wait Saved            # wait up to 10 s for "Saved" to appear
screenpeek wait Loading --gone   # wait for it to disappear
screenpeek scroll down 5 --at Results
screenpeek drag report.pdf Trash
```

Names are looked for in the focused window first, which is several times
faster, then on the whole screen. They match ignoring case: exact first, then
"starts with", then "contains".
Small OCR mistakes and accents are tolerated (`Fi1e` finds `File`). When a
name matches several things, nothing is clicked and the choices are listed:
use the id instead.

## Windows

```sh
screenpeek windows               # title, position, size, which has focus
screenpeek focus Firefox         # bring it to the front
```

Supported on Hyprland (scratchpads included), Sway, X11, GNOME (with the
extension), KDE and Windows.

## Several steps at once

```sh
screenpeek run "click File" "click Save As" "type report.pdf" "key enter" "wait Saved"
```

Steps: `focus WINDOW`, `click NAME`, `fill NAME with TEXT`, `type TEXT`,
`key KEYS`, `wait NAME`, `scroll down 3`, `drag A to B`. The run stops at the
first step that fails.

## Agents and MCP

```sh
curl -fsSL https://raw.githubusercontent.com/I-No-oNe/screenpeek/main/scripts/install-skills.sh | sh
curl -fsSL .../scripts/install-skills.sh | sh -s -- claude   # or codex, or all
sh scripts/install-skills.sh --link      # from a clone: stay updated with git pull
```

Skills go to `~/.claude/skills` (Claude Code) and `~/.agents/skills` (Codex).
Start a new agent session afterwards. The agent must run inside your desktop
session.

For any MCP client, run `screenpeek mcp`:

```sh
claude mcp add screenpeek -- screenpeek mcp
codex mcp add screenpeek -- screenpeek mcp
```

Over MCP, `click` checks that something changed near the target, so a miss
shows at once. It is on by default where screen capture is fast (Hyprland,
Sway, X11, Windows), and off on GNOME and KDE, where each check would add most
of a second. Pass `check: false` or `check: true` to choose.

To make your agent prefer it, add to `CLAUDE.md` or `AGENTS.md`:

> To navigate desktop apps, use `screenpeek` (scan, click, fill, key, wait,
> windows, focus). Take a screenshot only when the answer depends on how
> something looks.

## GNOME and KDE

Install the desktop portal: `xdg-desktop-portal-gnome` or
`xdg-desktop-portal-kde`.

**GNOME** needs a small extension so screenpeek knows where windows are.
`install.sh` adds it on GNOME; log out and back in once to load it. To add it
by hand: `sh helpers/gnome/install.sh`.

**KDE** needs nothing extra.

The first scan or click asks you to allow screen and input access. Your answer
is remembered for each desktop; to be asked again, delete
`~/.cache/screenpeek/portal-token-*`. After that, the background helper keeps
the screen stream open, so scans read the screen in a few milliseconds.

Run `screenpeek doctor` to see what this desktop supports and what is missing.

## Browsers

Browsers hide their accessibility info unless asked, so screenpeek reads them
with OCR. For exact names and states, start them with it on:

- Firefox: `GNOME_ACCESSIBILITY=1 firefox`
- Chromium, Chrome and Electron apps: add `--force-renderer-accessibility`

## Settings

| Variable | Effect |
| --- | --- |
| `SCREENPEEK_LANG` | Languages to read, like `heb` or `auto` (see [languages](languages.md)) |
| `SCREENPEEK_NO_DAEMON=1` | Don't use the background helper |
| `SCREENPEEK_CAPTURE=portal` | Force screenshot-portal capture |
| `SCREENPEEK_INPUT=portal` | Force portal keyboard and mouse |

A small background helper starts on the first scan and keeps the models
loaded. It stops after 10 idle minutes, or a minute after the program that
started it exits. It logs to `~/.cache/screenpeek/daemon.log`
(`%LOCALAPPDATA%\screenpeek\daemon.log` on Windows). The terminal you run screenpeek from is left
out of scans.

## Troubleshooting

- **Something does not work:** run `screenpeek doctor`. It also shows where
  the background helper's log is.
- **A command stops with "did not answer":** a desktop service is stuck,
  often behind an open dialog such as a keyring prompt. Answer the dialog, or
  run `systemctl --user restart xdg-desktop-portal`. screenpeek waits a few
  seconds at most, never forever.
- **Nothing found:** try `--fresh`, check `screenpeek windows`, or narrow with
  `--focused`.
- **Clicks land in the wrong place after the window moved:** use `--fresh`.
- **A label is misread:** click by id, or add its language
  (`bash scripts/fetch-models.sh`).
- **`screenpeek tree` shows nothing for an app:** it has no accessibility
  info; OCR still works.
