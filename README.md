# screenpeek

Reads the screen as a numbered list of text and clicks it by name. Built for programs that drive a desktop without seeing one: agents, test harnesses, scripts. A scan is plain text with screen coordinates, so nothing has to send a screenshot anywhere or invent a pixel coordinate.

```console
$ screenpeek scan --grep save
7 Save File @412,318

$ screenpeek click "Save File"
7 Save File @412,318

$ screenpeek fill "Search" "hello world"
```

## Install and download

```bash
curl -fsSL https://raw.githubusercontent.com/I-No-oNe/screenpeek/main/install.sh | sh
```

That drops the latest release binary into `~/.local/bin`. Prebuilt Linux and Windows binaries are also attached to every tagged release on the [Releases page](https://github.com/I-No-oNe/screenpeek/releases).

To build it instead, with Rust 1.75 or newer:

```bash
cargo install --git https://github.com/I-No-oNe/screenpeek
```

One binary, no runtime dependencies. The OCR models (12 MB) download to the cache directory on first use.

Linux builds need `libwayland-dev` and `libxkbcommon-dev` (or your distribution's equivalents) at compile time.

Linux talks to wlroots compositors directly: `wlr-screencopy` for capture, `wlr-virtual-pointer` and `virtual-keyboard` for input. Hyprland, Sway, river and Wayfire have all three. GNOME, KDE and X11 are not supported on Linux; Windows is.

## Runtime design

```text
Windows ─── UI Automation ─── exact text + screen rectangles ─── elements

Linux ───┬─ AT-SPI ────────── exact text, no usable position ──┐
         │                                                     ├─ fused
         └─ wlr-screencopy ── OCR ── text with positions ──────┘

Any platform ─── enigo ─── pointer and keyboard
```

On Windows the control tree carries both the text and the rectangle, so recognition never runs.

On Linux it carries only the text: Wayland never tells a window where it sits, so a GTK4 window reports its contents from `0,0` wherever it really is. screenpeek reads the pixels as well, matches a couple of labels between the two, and that gives the window's offset. Every control in that window then gets a real position — including ones recognition cannot read at all, such as an icon whose only text is its accessible name — and the text is whatever the toolkit says it is, in any language. Windows with no accessible tree fall back to recognition alone.

A running daemon keeps the models loaded, keeps the last frame, and re-reads only the rows that changed.

## Commands

```
screenpeek scan   [--grep TEXT] [--region X,Y,W,H] [--monitor N] [--json]
screenpeek click  <id|text> [--button left|right|middle] [--double] [--fresh]
screenpeek type   <text>
screenpeek key    <combination>
screenpeek fill   <id|text> <text>
screenpeek run    <step>...
screenpeek serve
screenpeek status
screenpeek tree
screenpeek read   <image> [--scale N] [--json]
```

`scan` prints `id text @x,y`, where `x,y` is the centre of the text on the virtual desktop — the point a click lands on. `--json` adds each element's size.

`click` takes an id from the last scan or part of an element's text. Exact matches beat prefixes, prefixes beat substrings, so `click Save` picks `Save` over `Save As...`. Several matches is an error that lists them; nothing is clicked on a guess.

A scan is cached, and `click` scans again by itself when the cached one cannot answer, so acting on a screen is usually one command. `--fresh` forces a new scan when the text is unchanged but has moved.

`serve` keeps a daemon in the background; every other command uses it automatically when it is running and works without it when it is not. `status` says whether one is up.

`run` takes a whole interaction in one command and scans only when a step needs something it does not already know:

```bash
screenpeek run "click File" "click Save As" "type report.pdf" "key enter"
```

Its steps are `click <target>`, `fill <target> with <text>`, `type <text>`, `key <combination>` and `wait <target>`. Keys are named plainly: `enter`, `tab`, `esc`, `ctrl+s`, `alt+f4`.

`tree` prints what the accessibility tree reports, window by window, which is the quickest way to see whether an application exposes one. `read` runs recognition over an image file instead of the screen.

## Speed

Measured on 1920x1080, Hyprland, CPU only. Full method and numbers in [BENCHMARK.md](BENCHMARK.md).

| | screenpeek | screenshot to a vision model |
| --- | --- | --- |
| Bytes produced | 2,214 | 1,291,832 |
| Tokens for one look | ~554 | ~1,843 |
| Capture | 9 ms | 529 ms |
| One look, no daemon | 2,617 ms | 529 ms |
| One look, daemon, changed rows only | ~400 ms | 529 ms |

A look costs about a third of the tokens of a screenshot and needs no round trip to place a click. The daemon's persistent capture buffer took capture from 354 ms to 9 ms; incremental reads take a repeat look from 2.6 s to about 0.4 s when only part of the screen changed.

## Use it from an agent

`skill/screenpeek/` is an agent skill: it tells an agent to scan instead of taking a screenshot, and to click by name instead of by coordinate. Copy it into your agent's skills directory.

```bash
cp -r skill/screenpeek ~/.claude/skills/
```

## Limits

- Text only, where there is no control tree. An unlabelled icon is invisible to recognition.
- Languages other than English work through the accessibility tree; where a window exposes none, recognition is English.
- Linux accessibility (AT-SPI) is not used: under Wayland a client is not told where it sits on screen, so the extents it reports cannot be clicked. [BENCHMARK.md](BENCHMARK.md) has the measurement.
- The cache holds one scan per user.

## License

MIT
