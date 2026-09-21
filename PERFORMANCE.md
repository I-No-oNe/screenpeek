# Performance notes

Measured on the machine described in [BENCHMARK.md](BENCHMARK.md). This is the
working record of where the time goes, what has been tried, and what is worth
trying next.

## Where a scan spends its time

| Stage | Cost |
| --- | --- |
| Capture, one shot | 354 ms |
| Capture, daemon with a persistent buffer | 9–26 ms |
| Accessibility tree walk, one window | 94 ms |
| Accessibility tree walk, nothing exposed | 3 ms |
| Recognition, 900x560 dialog, 20 labels | 336 ms |
| Recognition, 1920x1080 text-dense screen | 1,700–3,000 ms |
| Recognition, changed bands only | 320–430 ms |

Recognition is now the whole cost. It scales with how much text is on screen,
not with area: half the screen took 1,679 ms against 1,739 ms for all of it,
because the half that was dropped was nearly empty, while a quarter with less
text took 927 ms.

## Tried and rejected

**Magnifying before recognition.** The usual fix for small text with classical
OCR. Recall fell from 95% to 90% at 2x and 80% at 3x, and it cost 40% more
time. ocrs normalizes line height itself; upscaling only adds interpolation
artefacts.

**`-C target-cpu=native`.** 392 ms against 336 ms on the same fixture — 17%
slower. rten dispatches SIMD at runtime and pinning the target defeats it.
Build it stock.

## Worth trying next, most promising first

**Skip recognition for windows the tree already covers.** A window whose
accessible tree is readable only needs pixels to work out where it sits. Once
the daemon has that offset, and the window has not moved, another look costs
the 94 ms tree walk and nothing else. Detecting movement is cheap: the daemon
already compares frames, and a window whose rows are unchanged has not moved.
This would take an accessible application from ~2 s to ~0.1 s per look.

**Recognize the focused window only.** Most interactions concern one window.
Asking the compositor which one has focus and reading that rectangle would cut
the work to the text in it. The numbers above suggest most of a saving comes
from excluding text-dense background windows, which is exactly what this does.

**Recognize bands in parallel.** Patched reads already split the screen into
independent bands. rten threads within one recognition pass, so the gain is
whatever is left idle between passes — worth measuring before building.

**Cache recognition per line.** Hash each detected line's pixels and keep the
text. A window that scrolls by one line currently re-reads every line in the
band; with a cache it would read one. This helps terminals and lists, which is
where the current worst case lives.

**A different recognition engine.** PaddleOCR's mobile models through ONNX
Runtime are typically several times faster than ocrs on CPU, and they are
multilingual, which would remove the English limit where no accessible tree
exists. The cost is a native dependency and a larger install, against the
current single static binary. Worth a spike, not worth assuming.

**GPU.** rten has no stable GPU backend today. When one lands, recognition is
the part that would move.
