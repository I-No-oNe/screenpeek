---
name: screenpeek-drive
description: Click labelled desktop controls, enter text, and run sequences of UI actions with screenpeek.
---

# Operate the desktop

```sh
screenpeek run "click File" "click Save As" "type report.pdf" "key enter"
screenpeek fill "Search" "cats" --fresh
```

Steps are `click TARGET`, `fill TARGET with TEXT`, `type TEXT`,
`key COMBINATION`, and `wait TARGET`. A failed step stops the sequence.
`fill` types into the clicked field; it does not clear existing text.
`wait` checks once and fails if absent; it does not poll or wait for animation.

Use a batch for a known sequence. Split it when the next action depends on
output, then verify the resulting state with `scan`. The daemon starts itself.
Use `--fresh` with standalone `click` or `fill` after controls move.
Resolve ambiguity using an ID from a current scan; do not guess coordinates.
Use screenshots or another suitable tool for controls absent from the scan.
