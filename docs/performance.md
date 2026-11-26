# Speed and accuracy

Measured on one Linux laptop (8 threads, Hyprland, 1920×1080), release build.
Your numbers will differ; the commands below reproduce them.

## Tokens per look

| What the agent sees | Tokens |
| --- | ---: |
| Screenshot of a 900×560 dialog | about 765 |
| Full text listing of that dialog | 156 |
| `scan --grep Save` | 7 |

## Time per step

| Step | Time |
| --- | --- |
| Scan with the background helper running | ~80 ms |
| Scan of a screen that did not change | a few ms after capture |
| Accessibility tree of a file manager window | ~65 ms |
| Typing 700 ordinary characters (Sway, every one checked) | ~0.2 s |
| First scan (starts the helper, loads models) | ~1 s |
| GNOME / KDE screen capture, screen stream (PipeWire) | ~10 ms |
| GNOME / KDE screen capture, screenshot portal fallback | 0.4–0.9 s |
| GNOME: `run` of five clicks by id | 0.15 s (was 3.9 s) |

With an extra language chosen, a scan of a changing Hebrew screen took
~220 ms, down from ~650 ms with the previous full-screen method.

## Time per action

Median on a headless Sway (4-core ARM VM, software rendering), with
`bench/actions.py`. Actions now return once the screen around the target
holds still, instead of after a fixed wait:

| Action | 0.0.5 | 0.0.6 |
| --- | ---: | ---: |
| `click --check`, the target reacts | 300 ms | 82 ms |
| `click --check`, nothing changes (warns) | 300 ms | 275 ms |
| `run "focus WINDOW"` | 160 ms | 28 ms |
| `click` | 37 ms | 5 ms |
| `fill` | 158 ms | 140 ms |

## Accuracy

On the test dialog, 19 of 20 labels are read exactly; on the dense toolbar,
8 of 8. See [languages](languages.md) for other scripts.

## Reproduce

```sh
cargo build --release --locked
python3 bench/measure.py --all                   # accuracy and time per image
python3 bench/compare_tools.py --runs 7          # built-in reader vs Tesseract
python3 bench/actions.py -- 'click Save --check' 'run "focus Editor"'  # p50/p95 per action
uv run --with tiktoken python bench/compare.py   # token counts
python3 bench/tasks.py bench/tasks.example.json  # scripted tasks vs screenshots
cargo test --release --bin screenpeek -- --ignored --nocapture --test-threads=1
```
