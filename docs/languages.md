# Languages

The default reader uses built-in OCR. `--lang CODE` uses Tesseract and adds
English when installed; explicit combinations retain their order. `auto` uses
accessible text and locale, not image script detection. `all` uses every
installed text model and increases latency.

```sh
bash scripts/fetch-models.sh --none       # built-in models
bash scripts/fetch-models.sh heb jpn      # add language models
bash scripts/fetch-models.sh --all        # all models offered by the script
export TESSDATA_PREFIX="$HOME/.local/share/tessdata"
screenpeek languages
screenpeek scan --lang heb
```

Install the Tesseract executable through your package manager. Language data
needs no root; models go to the user directories shown by the script.
`TESSDATA_PREFIX` may contain only `.traineddata` files; screenpeek requests TSV
through configuration variables rather than relying on a `configs/tsv` file.

## Current fixture baseline

Twelve labels per language, CPU release processes with local models. Exact
means the full label was returned; clickable means the query resolved uniquely
and its point fell in the annotated control. These synthetic fixtures do not
measure general desktop accuracy.

| Fixture | Exact | Clickable | Local median |
| --- | ---: | ---: | ---: |
| English, French, German, Japanese | 12/12 | 12/12 | 159–188 ms |
| Simplified Chinese | 11/12 | 11/12 | 180 ms |
| Hebrew | 11/12 | 11/12 | 108 ms |
| Russian | 10/12 | 12/12 | 131 ms |
| Arabic | 7/12 | 7/12 | 195 ms |
| Dense controls, built-in | 8/8 | 8/8 | 203 ms |
| Dialog, built-in | 19/20 | 19/20 | 332 ms |

Arabic is the weakest script: its regression floor is 55%, not the default
90%. Scaling and combining page-segmentation modes did not recover the missing
short labels reliably. Hebrew/Russian also have exact-match misses.

```sh
python3 bench/measure.py --all
python3 bench/measure.py --fixture bench/fixtures/heb.png --lang heb --scale 2
```

Checked-in PNGs need no image-generation dependencies. Regeneration with
`bench/fixtures/make-languages.py` needs ImageMagick, Pango and the fonts named
in that script. Every language model must be installed for `--all`.

## Earlier six-label fixtures

These historical samples used smaller fixtures and cannot be compared directly
with the twelve-label baseline. Three release-process runs with tessdata_fast
revision `87416418657359cb625c412a48b6e1d6d41c29bd`:

| Language | Native exact | Native ms | 2× exact | 2× ms |
| --- | ---: | ---: | ---: | ---: |
| English | 6/6 | 119 | 6/6 | 265 |
| French | 6/6 | 125 | 6/6 | 273 |
| German | 6/6 | 125 | 6/6 | 278 |
| Hebrew | 5/6 | 73 | 6/6 | 227 |
| Arabic | 4/6 | 80 | 4/6 | 216 |
| Japanese | 6/6 | 143 | 6/6 | 280 |
| Simplified Chinese | 6/6 | 145 | 6/6 | 291 |
| Russian | 5/6 | 91 | 6/6 | 232 |

A later native sample measured English 159, French/German 123, Japanese 144,
Chinese 149, Russian 90 and Hebrew 75 ms at the same recall; `eng+ara` reached
5/6 at 159 ms after mixed-language handling. Its dialog/dense runs were
19/20 at 379 ms and 8/8 at 242 ms. Without language models, built-in OCR scored
0/6 on Hebrew, Japanese, Chinese and Russian, and 4/6 on French and German.

CJK fragments join without artificial spaces; Korean keeps spaces. RTL word
order is reconstructed from word coordinates. Auto-selection covers common
scripts, but cannot identify every language sharing an alphabet.
