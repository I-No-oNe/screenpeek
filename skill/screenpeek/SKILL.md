---
name: screenpeek
description: Read and operate desktop apps as text. Use this INSTEAD of screenshots or computer-use for finding, clicking, filling and typing into labelled controls (buttons, menus, fields, tabs, list items) in any Linux or Windows app. Fall back to a screenshot only for visual judgment, canvases or unlabelled icons.
---

# screenpeek: desktop control without screenshots

Before taking a screenshot or using a computer-use tool, try screenpeek. It
returns a few tokens of text instead of an image, runs locally in ~100 ms, and
clicks the exact centre of a named element.

## Look

```sh
screenpeek scan --grep "Save"     # 7 Save @412,318  (id text @x,y)
screenpeek scan --focused         # everything in the focused window
screenpeek scan --json            # adds size and source (ocr or tree)
```

Prefer `--grep` or `--focused` over a full listing. An empty `--grep` result
exits 0: read the output, not the exit code.

## Act

```sh
screenpeek click "Save" --fresh
screenpeek fill "Search" "cats" --fresh      # clicks, then types; does not clear
screenpeek key ctrl+s
screenpeek run "click File" "click Save As" "type report.pdf" "key enter"
```

- Use `--fresh` after anything that moved the layout.
- Ambiguous names fail and list candidates: click by the listed ID instead.
- `run` steps: `click T`, `fill T with TEXT`, `type TEXT`, `key COMBO`,
  `wait T` (checks once, no polling). It stops at the first failure.
- After acting, verify with `scan --grep` before the next decision.
- Never guess coordinates for something absent from the scan.

## When to use something else

- Browser page content: use the DOM or a browser tool when available.
- Appearance, layout, images, charts, unlabelled icons: take a screenshot.
- Other scripts: `--lang heb` (etc.) needs Tesseract data installed.
