# Benchmark

Measured with the scripts in `bench/`:

```bash
cargo build --release
bench/bench.sh      # cost and latency of one look
bench/accuracy.sh   # how much of a known interface is read back correctly
```

Machine: Arch Linux 7.2.5, Hyprland on Wayland, 1920x1080, 8 cores, no GPU acceleration. Measured 2026-09-22 with screenpeek 0.0.1. Timings are medians of 3 runs.

## One look at the screen

| Measure | screenpeek | screenshot to a vision model |
| --- | --- | --- |
| Bytes produced | 2,214 | 1,291,832 (PNG) |
| Tokens for that look | ~554 | ~1,843 |
| Capture | 9 ms | 529 ms (`grim`) |
| One look, no daemon | 2,617 ms | 529 ms |
| One look, daemon running | 1,788 ms | 529 ms |
| Repeat look, only part of the screen changed | ~400 ms | 529 ms |

44 elements were on screen. Text is counted at 4 bytes per token; the screenshot uses Anthropic's documented `width × height / 750`, after the long edge is scaled to 1,568 px, giving 1,568 × 882 / 750 ≈ 1,843.

Where the time goes, from the daemon's own log:

```
full:    44 elements, capture 26ms, read 1968ms
full:    45 elements, capture 10ms, read 1773ms
patched: 47 elements, capture  9ms, read  427ms
```

Two things carry the speed:

**Persistent capture.** A one-shot screenshot opens a capture session, allocates a buffer and tears it all down: 354 ms per frame through a general-purpose capture crate, 529 ms through `grim`. The daemon holds one Wayland connection and one shared-memory buffer and asks the compositor to copy into it, which lands at **9-26 ms**.

**Reading only what changed.** The daemon keeps the last frame, compares rows, groups the differing ones into bands and re-reads only those, keeping the text outside them. A repeat look drops from ~2.6 s to **~400 ms**. Once more than 55% of the rows have changed it reads the whole screen instead, because stitching then costs more than it saves.

The benchmark's own screen is a scrolling terminal with an animated spinner, which is the worst case for this: most of its requests repaint most of the screen and show as `full`. A settled application window patches.

**Honest comparison.** Taking a screenshot is fast locally, since it is a copy. screenpeek is slower to produce its answer but produces a third of the tokens and a coordinate that is already correct. The screenshot pays afterwards, in inference over ~1,800 image tokens and in the round trip needed to turn "the button is around there" into a click.

## Reading a known interface

`bench/fixtures/dialog.png` is a 900x560 settings window drawn at real UI text sizes (13 px body, 15 px headings) by `make-dialog.sh`; `dialog.expected` lists its 20 labels. A label counts only on an exact match.

| Magnification | Labels read exactly | Recall | Time |
| --- | --- | --- | --- |
| 1x | 19/20 | 95% | 345 ms |
| 2x | 18/20 | 90% | 489 ms |
| 3x | 16/20 | 80% | 569 ms |

The miss at 1x is `4 spaces`, read as `spaces`.

Magnification is the usual fix for small text with classical OCR. It measurably hurts here, because ocrs normalizes line height itself and upscaling only adds interpolation artefacts. `scan` therefore has no scaling knob; `read --scale` keeps one so the experiment stays reproducible.

## Why not the accessibility APIs

This was tested, not assumed.

**Windows.** UI Automation exposes every control's name and its rectangle in screen coordinates. It is strictly better than recognition, so screenpeek uses it first on Windows and only falls back to reading pixels for windows that expose nothing. The Windows build is compiled and tested in CI; the numbers above were not measured on Windows.

**Linux.** The tree is used for its text and the compositor for its geometry. AT-SPI was queried directly on the benchmark machine:

```console
$ busctl --user call org.a11y.Bus /org/a11y/bus org.a11y.Bus GetAddress
s "unix:path=/run/user/1000/at-spi/bus_0"

$ busctl --address unix:path=/run/user/1000/at-spi/bus_0 \
    call org.a11y.atspi.Registry /org/a11y/atspi/accessible/root \
    org.a11y.atspi.Accessible GetChildren
a(so) 5 ":1.1" ... ":1.9" ...
```

The five registered applications are `xdg-desktop-portal-gtk`, `kdeconnectd`, `udiskie`, `quickshell` and `qs`, all background services. The only one with windows reported `a(so) 0`: no children, nothing to read. No terminal, editor or browser on the machine registers at all.

With a GTK4 application running, the tree is complete and exact, but its geometry is not: a window at 775,12 reports its contents from 0,0, because Wayland never tells a client where it sits. The compositor does know, so screenpeek asks the compositor for the window list, Hyprland over its JSON socket or Sway over the i3 protocol, and joins the two on title and size. Text then comes from the toolkit, in any language, and the position from the compositor, with nothing recognized at all. Applications that expose no tree, and compositors that answer no IPC, fall back to reading the pixels.

## Where each approach wins

**screenpeek is the better tool when:**

- The target has a text label, so clicking by name needs no coordinate from a model.
- The same screen is consulted repeatedly: the daemon re-reads only what changed.
- Only part of the screen matters: `--grep` and `--region` narrow the answer to a line.
- The interaction is scripted: `until screenpeek scan --grep Done | grep -q .`
- Budget or context is tight: ~554 tokens against ~1,843, per look.

**A screenshot and a vision model are the better tools when:**

- The target has no text: an icon, a colour swatch, a chart, a drawing.
- The question is about appearance or layout rather than about one control.
- The interface is not in English.
- The interface is unfamiliar and has to be judged as a whole.

A practical agent scans first, because most controls are labelled, and takes a screenshot when the scan comes back without the answer.
