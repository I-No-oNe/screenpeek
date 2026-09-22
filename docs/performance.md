# Measurements

Local runs on an 8-thread Linux CPU, Hyprland at 1920×1080, release build,
models already downloaded. Treat them as indications, not guarantees.

## Observation size

Tokens counted with `o200k_base`; image cost uses OpenAI's high-detail formula.

| Observation | Tokens |
| --- | ---: |
| 900×560 dialog screenshot | ~765 |
| Full text listing of that dialog | 156 |
| `scan --grep Save` | 7 |

## Latency

| Step | Time |
| --- | --- |
| Native capture, full output / 400×300 region | ~60 ms / ~8 ms |
| Portal capture (GNOME/KDE) | 430–930 ms |
| Warm full-desktop OCR | ~200–240 ms |
| Unchanged frame (daemon cache) | 0 ms after capture |
| Hebrew changed band vs full frame | ~290 ms vs ~1,100 ms |
| First daemon read (model warm-up) | ~870 ms |

In one side-by-side run against dogtail on the same GTK window, screenpeek
returned 63 uniquely clickable names in 70 ms against dogtail's 38 in 568 ms.
dogtail's coordinates were window-relative on Wayland.

## Reproduce

```sh
cargo test --locked
cargo build --release --locked
python3 bench/measure.py --all            # accuracy and CLI latency
python3 bench/compare_tools.py --runs 7   # built-in OCR vs Tesseract
uv run --with tiktoken python bench/compare.py   # token counts
cargo test --release --bin screenpeek -- --ignored --nocapture --test-threads=1
```

Ignored tests need the OCR models.
