# Usage

## Build and install

Rust 1.94+ is required. Debian/Ubuntu dependencies:

```sh
sudo apt-get install pkg-config libwayland-dev libxkbcommon-dev libxdo-dev
cargo install --path . --locked
```

Fedora dependencies:

```sh
sudo dnf install gcc gcc-c++ pkgconf-pkg-config wayland-devel libxkbcommon-devel libXdo-devel
cargo install --path . --locked
```

`sh install.sh` installs the Linux x86-64 release into `~/.local/bin`;
`PREFIX` changes the destination. Windows binaries are available in Releases;
source builds use the MSVC Rust toolchain.

## Commands

| Command | Purpose |
| --- | --- |
| `scan [--json] [--grep TEXT]` | Read visible text and accessible controls |
| `click TARGET [--fresh] [--button right] [--double]` | Click a name or scan ID |
| `fill TARGET TEXT [--fresh]` | Click and type without clearing the field |
| `type TEXT` / `key ctrl+s` | Send input to the focused application |
| `run STEP...` | Execute steps in order and stop on error |
| `serve` / `status` | Run the daemon / inspect its endpoint |
| `read IMAGE [--scale N] [--lang CODE] [--json]` | Read a PNG without a desktop |
| `languages` | List installed Tesseract languages |
| `tree` | Inspect Linux accessibility labels |

Scan, click, fill and run accept `--region X,Y,W,H`, `--monitor N`,
`--focused` and `--lang CODE`. Region, monitor and focused are mutually
exclusive. Portal capture supports `--region`, not monitor selection.
`--focused` requires window geometry.

Matches are case-insensitive: exact, prefix, substring, then OCR-tolerant
folding. Folding handles accents and common substitutions such as `Fi1e` for
`File`. Multiple matches fail. IDs and cached coordinates refer to the last
scan; use `--fresh` after a layout change. Empty `scan --grep` output is not
an error, so inspect the output before acting.

```sh
screenpeek run "click File" "click Save As" "type report.pdf" "key enter"
```

Steps are `click TARGET`, `fill TARGET with TEXT`, `type TEXT`,
`key COMBINATION`, and `wait TARGET`. `wait` checks once; it does not poll.
Split a sequence when the app needs time to settle, then scan to verify.
A batch reuses one input session, including portal consent.

Keys include modifiers, navigation keys, F1–F12, characters and punctuation
names: `ctrl+s`, `alt+F4`, `/`, `slash`, `plus`. `type` treats leading dashes
as text. `read --scale 2` may help small glyphs; coordinates stay in the
original image, and supported scale values are 1–4.

## Capture and caching

The daemon starts automatically, keeps models loaded, recognizes changed
regions and exits when idle. An unchanged-frame result still requires a new
capture to check the pixels. `SCREENPEEK_NO_DAEMON=1` forces direct reads.
`SCREENPEEK_CAPTURE=portal` forces portal capture, including in a new daemon.

Linux combines AT-SPI names with compositor window positions and OCR.
Accessible icon buttons can be clicked by their application-provided names;
JSON marks them with `"source": "tree"`. Icons without accessible names need
another tool. Windows uses a nonempty UI Automation tree before OCR, so
inaccessible controls in an otherwise accessible desktop can be missed.

Hyprland, Sway, X11 and the GNOME/KWin helpers provide window positions.
The launching terminal is excluded using the nearest ancestor with a visible
window; detached tmux ancestry and missing PID data can prevent exclusion.
Caller exclusion is not implemented on Windows/macOS.

Wayland captures use logical desktop coordinates. The `daemon-v2` endpoint
and `last-scan-v2.json` cache separate them from the old physical-pixel format.
Mixed-scale monitor layouts need validation before relying on portal clicks.

## Fedora GNOME and KDE

These paths are experimental: they have parser, private-bus and input-coordinate
tests, but have not been validated on a live GNOME or KDE session.

Install the desktop portal backend: `xdg-desktop-portal-gnome` for GNOME or
`xdg-desktop-portal-kde` for KDE. Run from a terminal in the logged-in desktop.

On GNOME, install the geometry extension:

```sh
sh helpers/gnome/install.sh
# Log out and back in, then:
gnome-extensions enable screenpeek@screenpeek
```

Disable with `gnome-extensions disable screenpeek@screenpeek`.
The extension exposes visible window titles, PIDs and logical rectangles over
the session bus; it does not inject input or capture images.
GNOME 45–51 is declared in metadata, pending desktop validation.

KDE Plasma 6 geometry uses a bundled one-shot KWin script, loaded and unloaded
per query; no persistent installation is needed.

Capture uses the Screenshot portal; input uses the
[RemoteDesktop portal](https://flatpak.github.io/xdg-desktop-portal/docs/doc-org.freedesktop.portal.RemoteDesktop.html).
Approve keyboard/pointer access and select the monitors to control when prompted.
`SCREENPEEK_INPUT=portal` forces portal input. Each CLI invocation starts a new
session; `run` shares one across its steps. Denied requests and missing monitor
geometry fail instead of guessing coordinates. Requests time out after two minutes.

Validate on Fedora using a disposable text editor:

1. Run `screenpeek scan --json` and check the visible labels and positions.
2. Click a harmless editor control with `screenpeek click "LABEL" --fresh` and approve access.
3. With the editor focused, run `screenpeek type "screenpeek test"`, then `screenpeek key ctrl+a`.
4. Move the editor and change workspace; scans should follow the visible window.
5. Repeat at your usual display scale and on each monitor.

Geometry uses [Mutter client rectangles](https://gnome.pages.gitlab.gnome.org/mutter/meta/method.Window.get_client_content_rect.html)
and [KWin scripting](https://develop.kde.org/docs/plasma/kwin/api/).
Older Mutter versions use a client-rectangle fallback whose decoration offsets
need particular attention during validation.

## Agent setup

```sh
sh scripts/install-skills.sh codex    # ~/.agents/skills
sh scripts/install-skills.sh claude   # ~/.claude/skills
```

`SCREENPEEK_SKILLS_DIR` overrides either destination.
Start a new local session after installation; the agent needs the CLI on PATH
and access to the desktop display and session bus under its sandbox policy.
Installation does not alter agent permissions.

## Languages

The built-in reader is the default. `--lang CODE` selects Tesseract and adds
installed English; explicit combinations such as `heb+eng` retain their order.
`auto` uses accessible text and locale; `all` loads every installed text model.

```sh
export SCREENPEEK_LANG=eng+heb
# fish: set -gx SCREENPEEK_LANG eng+heb
```

See [language setup and measured quality](languages.md).
