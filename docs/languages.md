# Languages

The built-in OCR reads Latin text. For other scripts, install Tesseract with
your package manager, then its language data:

```sh
bash scripts/fetch-models.sh heb jpn      # or --all, or no argument to pick
export TESSDATA_PREFIX="$HOME/.local/share/tessdata"
screenpeek languages
screenpeek scan --lang heb
```

`--lang CODE` also reads installed English; `heb+eng` sets the order
explicitly. `auto` guesses from accessible text and the locale; `all` loads
every installed model and is slower. `SCREENPEEK_LANG` sets a default.

## Fixture results

Twelve labels per language, release build, local models. "Clickable" means the
query resolved to one element whose point is inside the control.

| Fixture | Exact | Clickable |
| --- | ---: | ---: |
| English, French, German, Japanese | 12/12 | 12/12 |
| Simplified Chinese | 11/12 | 11/12 |
| Hebrew | 11/12 | 11/12 |
| Russian | 10/12 | 12/12 |
| Arabic | 7/12 | 7/12 |

Arabic is the weakest script. These are synthetic fixtures, not a general
accuracy score. Reproduce with `python3 bench/measure.py --all`.
