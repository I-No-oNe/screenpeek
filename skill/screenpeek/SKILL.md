---
name: screenpeek
description: Use when a task needs to know what is currently on the screen of a running desktop application - reading a window, finding a control, checking whether a dialog, error or result is showing, confirming a screen has settled - or when a screenshot was about to be taken to locate something textual. Not for questions about images, colours or visual layout.
---

# Reading and driving the screen with screenpeek

`screenpeek` turns the screen into a numbered list of text elements and clicks
them by name. It replaces the screenshot-then-look loop: no image enters the
conversation, and a click needs no pixel coordinates.

Check it is installed with `screenpeek --version`. If it is missing:
`cargo install --git https://github.com/I-No-oNe/screenpeek`.

Start `screenpeek serve` in the background before a session with many looks: it
keeps the models loaded and re-reads only the rows that changed, which takes a
repeat scan from about 2.6 s to about 0.4 s.

## The loop

```sh
screenpeek scan                  # id text @x,y, one element per line
screenpeek click "Save"          # by text, or by id from the last scan
screenpeek fill "Search" "cats"  # click a field, then type into it
screenpeek type "hello"          # type into whatever has focus
```

For a sequence of actions rather than a single look, use the `screenpeek-drive`
skill: `screenpeek run` does a whole interaction against one scan.

`click` reuses the last scan and rescans by itself when the text is not in it,
so a single `screenpeek click "Save"` is usually the whole interaction. Do not
run a scan before every click out of habit.

## Rules

1. **Do not take a screenshot to find text.** Scan instead. Reach for a
   screenshot only when the thing you need is genuinely visual: an image, a
   colour, a layout problem, an unlabelled icon.
2. **Narrow the read.** `--grep` filters the output, `--region X,Y,W,H` limits
   what is read at all and is several times faster. Coordinates from a scan can
   be passed straight back as a region.
3. **Prefer text over ids.** `click "Save"` survives a changed screen; `click 7`
   is only valid for the scan that produced it. Use ids to disambiguate.
4. **Let it rescan.** After an action changes the screen, just issue the next
   `click` with text, which refreshes when it has to. Use `--fresh` when the
   element's text is unchanged but its position moved.
5. **Check the result.** Every acting command prints the element it used. If
   nothing matched, screenpeek exits non-zero and clicks nothing, so never
   fall back to guessing pixel coordinates.
6. **Ambiguity is a stop, not a guess.** When several elements match,
   screenpeek lists them; pick one by id rather than rephrasing blindly.

## Verifying state

To wait for or confirm something, scan for it rather than describing a
screenshot:

```sh
screenpeek scan --grep "Export complete"     # empty output means not yet
until screenpeek scan --grep "Done" | grep -q .; do sleep 1; done
```

## When to use something else

- Unlabelled icons, images, colours, visual layout: take a screenshot.
- Text inside a browser page you control: the DOM is exact, use it.
- Non-English interfaces: text comes from the accessibility tree where a window
  exposes one, in any language. Where none is exposed, recognition is English
  only, so take a screenshot instead.

## Reference

```
screenpeek scan  [--grep TEXT] [--region X,Y,W,H] [--monitor N] [--json]
screenpeek click <id|text> [--button left|right|middle] [--double] [--fresh]
screenpeek type  <text>
screenpeek fill  <id|text> <text>
```

`x,y` is the centre of the text on the virtual desktop, the point a click
lands on. `--json` adds each element's width and height.
