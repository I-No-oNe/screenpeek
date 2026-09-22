# Measurements

These measurements came from local development on Linux with eight logical
CPUs, CPU OCR and release builds. Desktop samples used Hyprland at 1920×1080,
including 1.25 scaling. Models were already downloaded. Historical samples
below were taken at different revisions and are not one controlled trial.

## Reproduce

```sh
cargo fmt --check
cargo clippy --all-targets --locked -- -D warnings
cargo test --locked
cargo build --release --locked
python3 bench/measure.py
python3 bench/measure.py --all
python3 bench/compare_tools.py --runs 7
cargo test --release --bin screenpeek -- --ignored --nocapture --test-threads=1
uv run --with tiktoken==0.14.0 python bench/compare.py
```

Normal tests need no desktop or OCR models. Ignored OCR tests need downloaded
models; the ignored compositor test needs a live desktop. Run desktop tests
separately from headless CI. `--all` requires every fixture language model.

`measure.py` warms once, then measures fresh CLI processes using `perf_counter`.
It checks exact labels, name resolution and click rectangles. The default
clickable-label floor is 90%; Arabic has a disclosed 55% regression floor.
Passing that floor does **not** mean Arabic is 90% accurate. Use `--max-ms`
for a local latency budget. CI uses generous limits to detect stalls.

## Observation size

`compare.py` counts fixture text using GPT-4.1's `o200k_base` tokenizer and
estimates image tokens using the documented image formula.

| Observation | Tokens | Reduction vs full high-detail image |
| --- | ---: | ---: |
| Full 900×560 dialog, high detail | 765 estimated | — |
| Full image, low detail | 85 estimated | — |
| Button-row crop, high detail | 255 estimated | — |
| Full text listing | 156 measured | 4.9× |
| Filtered `Save` result | 7 measured | 109× |

Image counts are model-specific estimates; low-detail label accuracy was not
tested. Counts exclude prompts, tool wrappers, history and generated output.
See [OpenAI's image token calculation](https://developers.openai.com/api/docs/guides/images-vision#tile-based-image-tokenization).

A fixture pipeline sample measured 391 ms full OCR, 229 ms button-region OCR
and 0.18 ms unchanged-frame lookup. Adding an **assumed** 30 ms overhead and
comparing with an **assumed** extra 3-second vision turn gives 7.2×, 11.7× and
100.4× step speedups. These are scenarios, not measured model latencies or
whole-task speedups. Capture, IPC, input, application settling and verification
are excluded from the fixture pipeline. `compare.py` also reports 1- and
5-second scenarios and accepts `--overhead-ms` and `--vision-ms`.

## Fixture and desktop timings

| Measurement | Result | Scope |
| --- | --- | --- |
| Original / first updated dialog CLI, 7 runs | 504 / 514 ms; 19/20 labels | No demonstrated end-to-end speedup; build activity during updated run |
| Loaded-model dialog OCR, 7 runs | 210 ms; 19/20 | Excludes process and model startup |
| Updated dialog CLI before / after control splitting | 369.0 / 369.2 ms; 19/20 | Same recall and latency |
| Dense controls before / after splitting | 4/8 → 8/8; 232.8 ms after | Before latency not recorded |
| Changing desktop, direct / daemon, 3 runs | 2,230 / 980 ms | Smoke sample with mixed patched/full reads |
| Steady desktop capture | 59–67 ms | A fresh capture is still required for cache reuse |
| Unchanged frame recognition | 0 ms | Excludes capture |
| Warm full desktop recognition | ~200–236 ms | Content-dependent |
| First daemon recognition | ~870 ms | Graph warmup |
| Tree traversal alone | 127 ms | One GTK application |
| Sequential / overlapped tree and OCR | 281 / 194 ms | Warm full read; ~31% reduction |
| 400×300 region capture before / after native cropping | 60 / 8 ms | Whole-output capture remained 60 ms |
| Portal screenshot capture | 430–930 ms | Includes PNG write/read/decode |
| Portal vs native capture | 99 elements each; 52 shared unique labels; max delta 0 px | Hyprland, OCR only; not a GNOME/KDE test |
| Region geometry check | 76 elements, none outside; shared labels within 0–1 px | OCR only |
| Region filtering bug before fix | 137 of 162 elements outside requested area | Direct path with an accessibility tree |
| Contiguous-row line hashing | 40.6 → 8.5 µs | 300×20 pixels; 7 × 10,000 hashes; hashing only |

The daemon checks tree coverage before skipping OCR. Nautilus coverage was
0.52 in an earlier comparison and 0.61 with bidirectional substring matching,
below the 0.90 threshold; file sizes, dates and counts were missing from its
tree. Those figures describe that experiment, not a promise of present coverage.

## Multilingual updates

| Measurement | Before | After / alternative |
| --- | --- | --- |
| Tesseract `eng+heb`, full frame / band | 1,333 ms at 1920×1080 | 289 ms at 1920×200 |
| Live Hebrew changed-frame read | 1,083–1,160 ms | 264–323 ms with band patching |
| Repeated Hebrew band | 1,083–1,160 ms | 1–2 ms cached recognition |
| English full reads in that session | 150–171 ms | Unchanged |
| German model alone / with English | 104 ms | 179 ms |
| German and English as concurrent processes | — | 160 ms; separate processes still contend for CPU |

Cached-band figures exclude capture. Explicit single-language requests add
installed English; use explicit combinations to control order, not to disable
that behavior. [Current language fixture results](languages.md).

## Comparison with dogtail

`bench/compare_methods.py` runs the real dogtail runner and screenpeek in the
same session. This compares observations and coordinate outputs, not complete
automation success rates. Distinct coordinates do not prove a correct click.

| Tool | Elements | Unique names | Distinct points for unique names | Median, 3 runs |
| --- | ---: | ---: | ---: | ---: |
| dogtail | 205 | 38 | 38 | 568 ms |
| screenpeek | 175 | 63 | 63 | 70 ms |

Every shared unique label had an offset of (775, 12), the window position:
dogtail returned window-relative coordinates; screenpeek applied compositor
geometry. Of screenpeek's elements, 169 came from the tree and six from OCR.
Fifteen accessible icon-only controls were found, including Back and Main Menu.
Within the compared window's label set, neither tool lacked a shared name.

In the browser sample, Chromium exposed no AT-SPI application; screenpeek read
70 visible elements, 26 containing Hebrew. With another workspace displayed,
dogtail still returned the hidden file-manager tree; screenpeek used compositor
visibility. These are session-specific observations, not universal browser or
dogtail behavior. Dogtail also exposes roles, states and actions that screenpeek
does not model.

An earlier **stand-in AT-SPI reader** returned 196 elements, 37 unique names,
9 distinct points and 248 ms, against screenpeek's 168 / 60 / 60 / 64 ms.
Its coordinate collapse did **not** reproduce with real dogtail and must not
be used as a dogtail claim. The reference script remains available for diagnosis.

## OCR engine comparisons

Seven fresh-process dialog runs with annotated targets:

| Reader | Median | Correct targets |
| --- | ---: | ---: |
| Built-in screenpeek | 461 ms | 95% |
| Screenpeek with Tesseract | 230 ms | 80% |
| Direct Tesseract | 184 ms | 95% |

The confidence filter dropped some correct Tesseract words. Screenpeek is
not the fastest cold OCR reader. A separate PNG / PPM comparison measured
219 / 230 ms, so removing PNG encoding did not demonstrate a speedup.

An external PP-OCRv6-small experiment used `ocr-rs` 2.4.1 and MNN. Its code
and models are not shipped here, so these historical results are not fully
reproducible from this repository. Seven warm, decoded-image runs, four threads,
`OCR_BORDER=2`:

| Fixture | Exact labels / correct points | Median |
| --- | --- | ---: |
| dialog | 20/20 | 296 ms |
| dense | 8/8 | 94 ms |
| English | 6/6 | 97 ms |
| French | 6/6 | 127 ms |
| German | 6/6 | 123 ms |
| Japanese | 6/6 | 91 ms |
| Simplified Chinese | 6/6 | 97 ms |
| Hebrew | 0/6 | 106 ms |
| Arabic | 0/6 | 87 ms |
| Russian | 0/6 | 122 ms |

That run reported 6 ms model load and 256 MB peak RSS. A separate cold engine
comparison reported 116 ms load and 306 MB peak RSS versus 160 MB for ocrs:

| Image | ocrs | PP-OCRv6 small |
| --- | ---: | ---: |
| 4×4 blank | ~170 ms | 0.7 ms |
| 300×40 crop | 171 ms | 14 ms |
| 640×240 dense | 202 ms | 94 ms |
| 900×560 dialog | 338 ms | 304 ms |
| 1920×1080 desktop | 1,380 ms | 1,395 ms |

The desktop figures include different startup costs; 1,230 ms was also reported
for ocrs with the process floor subtracted. They do not establish a clean 1.13×
engine comparison. A projected patched-read comparison was ~150 versus ~40 ms
recognition plus ~60 ms capture; unchanged frames would not improve.

The candidate added a C++ runtime, raised model downloads from 11.7 to ~26 MB
(~15 MB for the small pair alone), and lacked a Hebrew recognizer. A separate
Arabic head reached 12/12 after reading-order correction versus Tesseract's
7/12; the general head failed Arabic. The engine was not adopted.

## Experiments that were not retained

| Experiment | Observation | Decision |
| --- | --- | --- |
| PP-OCR border 0, 1, 2, 3 / 4, 6 | Dense 8/8 / 7/8; dialog 20/20 throughout | Adjacent glyphs leaked into wider crops |
| Early 2× scaling run | Dialog 19/20 at 369 → 442 ms; dense 8/8 at 233 → 267 ms | No recall gain |
| Later 2× scaling run | Dense 8/8 → 7/8; dialog 19/20 → 17/20; browser search label lost | Keep scaling explicit |
| 2× on selected crops | 400×40 row recovered `4 Spaces`; 700×200 split `SaveCancel` | Crop-specific gains do not justify global scaling |
| Nearest vs Triangle resize | 62–67 vs 57–67 ms capture | No convincing speed gain; retain Triangle |
| Arabic PSM 11 + PSM 6 union | No gain over PSM 11 | Keep one pass |

Window-edge splitting, Unicode folding, mapped models, bounded line/band
caches, region clipping and concurrent reads have regression tests. A future
engine change needs broad language and desktop validation, not just one dialog.

## Alpha correctness fixes

The alpha merges tree labels with OCR by matching text and position; a partial
tree no longer removes every OCR label inside its window. Direct reads, cached
reads and inferred window placement share that merge. Window placement now
rejects indistinguishable candidates and different-title windows with identical
sizes, preventing hidden trees from claiming a visible window.

The historical desktop counts above predate these fixes; their accuracy and
latency need remeasurement on the same application layout.

## What the evidence supports

Screenpeek can replace the screenshot-to-coordinate step for labelled controls,
with compact text observations and local caching. No task corpus here measures
what fraction of all computer use that covers, and no benchmark establishes
that it is universally best. Visual reasoning, inaccessible icons, stale trees,
OCR errors and app-specific automation remain reasons to use other tools.
