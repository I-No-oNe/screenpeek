# Performance notes

Measured on the machine described in [BENCHMARK.md](BENCHMARK.md). This is the
working record of where the time goes, what has been tried, and what is worth
trying next.

The daemon starts itself the first time a command needs it, so the fast path is
the default. `SCREENPEEK_NO_DAEMON=1` keeps everything in one process.

It is meant to be invisible while it is not working: recognition runs on half
the cores, bands are read in parallel within that budget, and a band whose
pixels have been read before is not read again, so a repeat look at an
unchanged screen costs 12 ms. It shuts itself down 10 minutes after the last
request, or a minute after the last session that used it has exited, whichever
comes first.

## Where a scan spends its time

| Stage | Cost |
| --- | --- |
| Capture, one shot | 354 ms |
| Capture, daemon with a persistent buffer | 9-26 ms |
| Accessibility tree walk, one window | 94 ms |
| Accessibility tree walk, nothing exposed | 3 ms |
| Recognition, 900x560 dialog, 20 labels | 336 ms |
| Recognition, 1920x1080 text-dense screen | 1,700-3,000 ms |
| Recognition, changed areas only | 145-210 ms |
| Repeat look, nothing changed | 13-14 ms |
| Reading one located window, no recognition at all | 0 ms |
| Accessibility tree walk, skipped for an unchanged window | 0 ms |
| One look in another language, through tesseract | 187 ms for a 900x560 dialog |

Recognition is now the whole cost. It scales with how much text is on screen,
not with area: half the screen took 1,679 ms against 1,739 ms for all of it,
because the half that was dropped was nearly empty, while a quarter with less
text took 927 ms.

## Tried and rejected

**Magnifying before recognition.** The usual fix for small text with classical
OCR. Recall fell from 95% to 90% at 2x and 80% at 3x, and it cost 40% more
time. ocrs normalizes line height itself; upscaling only adds interpolation
artefacts.

**`-C target-cpu=native`.** 392 ms against 336 ms on the same fixture, 17%
slower. rten dispatches SIMD at runtime and pinning the target defeats it.
Build it stock.

## Done since

**Bands read in parallel, and cached by their pixels.** A screen that flips
between two states, a menu opening and closing, is read once. An unchanged
screen costs 12 ms end to end.

**Windows located by the compositor, not by matching labels.** Hyprland's JSON
socket and Sway's i3 protocol report every window's position and size, and the
accessibility tree reports what each window contains. Joining the two on title and size gives exact
coordinates for exact text with nothing recognized. A change inside a located
window is not read at all, since its tree already describes it.

**Changed areas trimmed to the columns that changed.** A change is now a
rectangle rather than a full-width band, which is what lets a change inside one
window be recognized as belonging to that window.

**`scan --focused`.** The compositor names the focused window, so a scan can be
limited to it: 392 ms against 2.0 s for the whole screen, and the answer holds
only what the user is actually looking at.

**A per-line recognition cache.** Each detected line is hashed by its pixels
and remembered, so a line that has been read before is not read again wherever
it has moved to. Patched reads went from 320-430 ms to 145-210 ms.

**The tree walk skipped for windows that did not change.** Walking one window
costs 86-94 ms of D-Bus conversation. The daemon now keeps each window's items
and only walks again when that window's pixels moved, so a repeat look at a
settled screen costs 13 ms end to end, down from 151 ms.

**Windows walked at the same time.** Each window is a separate conversation
with a separate application, so they run together rather than one after
another. With one window open this changes nothing; with six it is the
difference between one wait and six.

**The capture handed to the reader without conversion.** ocrs takes RGBA as it
comes, so the frame is no longer copied into an RGB buffer on every read.

**X11 capture.** X11 sessions keep one connection open and read the root
window, the same shape as the Wayland path, so they get the same incremental
behaviour rather than falling back to a fresh session per frame.

## Worth trying next, most promising first

**Detection as well as recognition, skipped for located windows.** Detection
still runs over the whole capture even where every window is described by its
tree. Cropping the input to the uncovered rectangles would remove that too, and
it is the largest remaining item on a first look at a busy screen.

**A first look that is not a full read.** Everything above makes a repeat look
cheap; the first one still reads the whole screen at 1.8-2.0 s. Reading the
focused window first and the rest in the background would make the answer that
matters arrive in about 400 ms.

**Tesseract kept running.** `--lang` spawns a process per look. Tesseract can
be driven as a long-lived child, which would take the 187 ms dialog read down
by whatever the process start costs.

**river and Wayfire.** Neither reports window geometry, so they keep the
label-matching fallback. Hyprland, Sway and X11 are asked directly.

**A native recognition engine.** PaddleOCR's mobile models through ONNX Runtime
are typically several times faster than ocrs on CPU and are multilingual, which
would also remove the tesseract dependency for `--lang`. The cost is a native
runtime on both platforms, against the current single static binary.

**A different recognition engine.** PaddleOCR's mobile models through ONNX
Runtime are typically several times faster than ocrs on CPU, and they are
multilingual, which would remove the English limit where no accessible tree
exists. The cost is a native dependency and a larger install, against the
current single static binary. Worth a spike, not worth assuming.

**GPU.** rten has no stable GPU backend today. When one lands, recognition is
the part that would move.
