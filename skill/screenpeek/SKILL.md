---
name: screenpeek
description: Navigate and test desktop apps fast through text. Use it to find, click, fill and type into labelled controls, switch windows and run UI workflows in any Linux or Windows app, instead of screenshot round trips. Keep screenshots or computer use for anything visual - design, layout, colours, images, charts - and for controls with no label.
---

# screenpeek: fast desktop navigation as text

screenpeek turns the screen into a short list of `id text @x,y [states]`
lines and clicks elements by name. It answers in about a tenth of a second,
costs a few tokens instead of an image, and works for models without vision.

Use it to **drive** apps: open menus, fill forms, switch windows, walk through
a workflow and check that the expected labels appear. It does not judge how
things look. When the question is visual (is it aligned, what colour, does
the design look right, what is in this image) take a screenshot or use
computer use.

## Look

```sh
screenpeek windows                # open windows, which one has focus
screenpeek scan --focused         # everything in the focused window
screenpeek scan --grep "Save"     # 7 Save @412,318
screenpeek scan --json            # adds size, source, role and states
```

Prefer `--grep` or `--focused` to a full listing. An empty `--grep` result
exits 0: read the output, not the exit code. States such as `[checked]` or
`[disabled]` come from the app. IDs stay the same across scans while an
element does not move.

## Act

```sh
screenpeek focus "Firefox"
screenpeek click "Save" --fresh
screenpeek fill "Search" "cats" --fresh      # clicks, then types; does not clear
screenpeek key ctrl+s
screenpeek wait "Saved" --timeout 10          # polls; --gone waits for it to vanish
screenpeek scroll down 5 --at "Results"
screenpeek drag "report.pdf" "Trash"
screenpeek run "click File" "click Save As" "type report.pdf" "key enter" "wait Saved"
```

- Use `--fresh` after anything that moved the layout.
- Use `wait` instead of sleeping while an app loads or a dialog opens.
- `click --check` warns when nothing near the target changed. Over MCP this is
  on by default where capture is fast; read the warning before moving on.
- Ambiguous names fail and list candidates: click by the listed ID instead.
- `run` steps: `focus WINDOW`, `click T`, `fill T with TEXT`, `type TEXT`,
  `key COMBO`, `wait T`, `scroll down 3`, `drag A to B`. It stops at the
  first failure. Start with `focus` so the terminal cannot take focus between steps.
- After acting, verify with `scan --grep` before the next decision.
- Never guess coordinates for something absent from the scan.

## Use something else when

- The answer depends on appearance: screenshot or computer use.
- A control has no text or accessible name (bare icons): screenshot.
- It is web page content and a DOM or browser tool is available: use that.
