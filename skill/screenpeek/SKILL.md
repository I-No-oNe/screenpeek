---
name: screenpeek
description: Read text and locate labelled controls in desktop applications with screenpeek. Use screenshots for visual appearance and unlabelled icons.
---

# Read the desktop

```sh
screenpeek scan --json
screenpeek scan --grep "Save"
screenpeek click "Save" --fresh
```

Output is `id text @x,y`; coordinates are text centres on the desktop.
Exact text matches beat prefixes, then substrings. Ambiguous targets fail;
choose an ID from the current scan or a more specific label.

The daemon starts automatically. `--grep` reduces returned text; `--region`
and `--focused` also crop the image before OCR. Prefer a known target or the
focused window to a full-screen listing when the task permits it.

On supported Linux desktops, the launching terminal is excluded automatically.
Cached IDs and positions can become stale. Use `--fresh` after a layout change.
An empty `scan --grep` succeeds: check its output, not just its exit code.
For OCR in another language, use `--lang CODE` with Tesseract and its language
data installed. Prefer the DOM for browser content when it is available.
