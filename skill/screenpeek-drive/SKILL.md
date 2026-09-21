---
name: screenpeek-drive
description: Use when a running desktop application has to be operated rather than merely read - clicking a button, filling a field, pressing a key, confirming or dismissing a dialog, carrying out several UI steps in a row, or waiting for a screen before acting on it. Also use when a scan is already in hand and the next thing to do is act on it.
---

# Driving an application with screenpeek

One command does an interaction. `screenpeek run` takes the whole sequence,
scans only when a step needs something it does not already know, and reuses
that scan for every step that follows.

```bash
screenpeek run "click File" "click Save As" "type report.pdf" "key enter"
```

Steps:

| Step | What it does |
| --- | --- |
| `click <target>` | Clicks an element by text or by id |
| `fill <target> with <text>` | Clicks a field, waits for focus, types into it |
| `type <text>` | Types into whatever has focus |
| `key <combination>` | `enter`, `tab`, `esc`, `ctrl+s`, `alt+f4`, `shift+tab` |
| `wait <target>` | Resolves a target, rescanning until it is there |

Each acting step prints the element it used, so the output is the record of
what happened.

## Why it is one command and not four

A scan is the expensive part. `run` holds one scan across the whole sequence
and drops it only after a click, since a click is what changes the screen. A
`type` or a `key` after a click costs nothing extra.

Start `screenpeek serve` once at the beginning of a session with a lot of
interaction. The daemon keeps the models loaded and re-reads only the rows
that changed, which takes a repeat look from about 2.6 s to about 0.4 s.

## Rules

1. **Target by text, not by coordinate.** `click "Save"` is stable; a pixel
   position is not. Never compute a coordinate yourself and never pass one to
   another tool.
2. **Put the sequence in one `run`.** Reach for separate `click` and `type`
   commands only when something between them has to be decided from output.
3. **Ambiguity stops the run.** If a target matches several elements,
   screenpeek lists them and clicks nothing. Re-run with the id it printed, or
   with a longer piece of the label.
4. **A failed step exits non-zero** and nothing after it runs. Check the exit
   code rather than assuming the sequence completed.
5. **Wait by scanning, not by sleeping.**
   ```bash
   screenpeek run "wait Export complete" "click OK"
   ```
6. **Modifier keys are named, not typed.** `key ctrl+s`, not `type "^s"`.

## Worked examples

Save a document under a new name:

```bash
screenpeek run "key ctrl+shift+s" "fill Name with quarterly.odt" "click Save"
```

Fill a login form:

```bash
screenpeek run "fill Email with me@example.com" "fill Password with hunter2" "click Sign in" "wait Dashboard"
```

Dismiss whatever dialog is up:

```bash
screenpeek run "click Cancel"
```

Confirm something finished before moving on:

```bash
until screenpeek scan --grep "Done" | grep -q .; do sleep 1; done
```

## When a click has nowhere to land

If `scan` cannot see the control, it has no text: an icon, an image button, a
canvas. Take a screenshot for that one and work from the picture. Everything
with a label stays on screenpeek.
