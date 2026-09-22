# Usage

## Build

Needs the Rust version in `Cargo.toml` (`rust-version`) or newer.

```sh
# Debian/Ubuntu
sudo apt-get install pkg-config libwayland-dev libxkbcommon-dev libxdo-dev
# Fedora
sudo dnf install gcc gcc-c++ pkgconf-pkg-config wayland-devel libxkbcommon-devel libXdo-devel

cargo install --path . --locked
```

`sh install.sh` installs the latest Linux x86-64 release into `~/.local/bin`
(`PREFIX` overrides). Windows binaries are on the Releases page.

## Commands

| Command | Purpose |
| --- | --- |
| `scan [--json] [--grep TEXT]` | List visible text and accessible controls |
| `click TARGET [--fresh] [--button right] [--double]` | Click by name or scan ID |
| `fill TARGET TEXT [--fresh]` | Click, then type (does not clear the field) |
| `type TEXT` / `key ctrl+s` | Input to the focused app |
| `run STEP...` | Run steps in order, stop on the first error |
| `read IMAGE [--scale N] [--lang CODE]` | Read a PNG instead of the screen |
| `serve` / `status` | Run / check the background daemon |
| `languages` / `tree` | List Tesseract languages / accessibility labels |

`scan`, `click`, `fill` and `run` take one of `--region X,Y,W,H`,
`--monitor N` or `--focused`, plus `--lang CODE`.

Matching is case-insensitive: exact, then prefix, then substring, then
OCR-tolerant (`Fi1e` matches `File`, accents ignored). Several matches fail
and list the candidates. IDs refer to the last scan. An empty `scan --grep` is
not an error, so check the output.

```sh
screenpeek run "click File" "click Save As" "type report.pdf" "key enter"
```

Steps: `click TARGET`, `fill TARGET with TEXT`, `type TEXT`, `key COMBO`,
`wait TARGET` (checks once, does not poll). Keys include modifiers, F1–F12,
arrows, characters and names like `slash` or `plus`.

## Daemon

The first scan starts a daemon that keeps models loaded, re-reads only changed
regions and exits after 10 idle minutes. `SCREENPEEK_NO_DAEMON=1` reads
directly. `SCREENPEEK_CAPTURE=portal` and `SCREENPEEK_INPUT=portal` force the
portal backends.

The terminal that launched screenpeek is left out of scans when its window can
be found through the process tree (Linux only).

## GNOME and KDE

Install the portal backend: `xdg-desktop-portal-gnome` or
`xdg-desktop-portal-kde`.

GNOME needs the geometry extension for `--focused`, terminal exclusion and
accurate accessibility positions:

```sh
sh helpers/gnome/install.sh
# log out and back in
gnome-extensions enable screenpeek@screenpeek
```

It only lists visible windows over the session bus. KDE Plasma uses a one-shot
KWin script loaded per query; nothing to install.

The first click asks you to allow keyboard/pointer control and pick monitors.
The grant is remembered (in `~/.cache/screenpeek/portal-token`); delete that
file to be asked again.

## Agents

```sh
sh scripts/install-skills.sh codex    # ~/.agents/skills
sh scripts/install-skills.sh claude   # ~/.claude/skills
```

`SCREENPEEK_SKILLS_DIR` overrides the destination. The agent needs
`screenpeek` on `PATH` and access to the display and session bus.
