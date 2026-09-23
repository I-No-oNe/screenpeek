# Languages

screenpeek reads English (and other Latin-script text) by itself. For other
languages it uses [Tesseract](https://github.com/tesseract-ocr/tesseract).

## Add a language

1. Install Tesseract: `sudo dnf install tesseract` or
   `sudo apt-get install tesseract-ocr`. On Windows:
   `winget install UB-Mannheim.TesseractOCR`.
2. Pick languages:

   ```sh
   bash scripts/fetch-models.sh          # choose from a list
   bash scripts/fetch-models.sh heb jpn  # or name them
   bash scripts/fetch-models.sh --none   # back to English only
   ```

   On Windows, `scripts/fetch-models.ps1` does the same (`-None` for English
   only); `install.ps1` runs it for you.

That's it: screenpeek finds the data by itself. `screenpeek languages` lists
what is installed.

## How it picks a language

With languages chosen, screenpeek still reads the screen with its fast
built-in reader first. When enough lines come out garbled (a sign of another
script), it re-reads **all the lines** with Tesseract, because some text in
another script can look like ordinary English to the built-in reader. That
full-screen pass takes a few seconds, but lines already re-read are
remembered, so later scans only re-read what changed. A screen that reads
cleanly costs nothing extra.

You can also choose per command:

| Option | Meaning |
| --- | --- |
| `--lang heb` | Always read Hebrew too (plus English) |
| `--lang auto` | Use your chosen languages, only when needed |
| `--lang all` | Every installed language (slower) |
| `--lang none` | English only |

`SCREENPEEK_LANG` sets the same thing for every command.

## Accuracy

Twelve labels per language on test images, reading with `--lang`:

| Language | Labels read exactly |
| --- | ---: |
| English, French, German, Japanese, Hebrew, Russian | 12/12 |
| Arabic, Simplified Chinese | 11/12 |

These are test images, not a promise for every app. Check it yourself with
`python3 bench/measure.py --all`.
