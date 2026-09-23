# Agent guide

screenpeek reads the screen as text and clicks things by name. Rust, one binary,
Linux (Hyprland, Sway, GNOME, KDE, X11) and Windows.

## Layout

- `src/capture/` screen pixels: wlr-screencopy, X11, portal, PipeWire frames, xcap on Windows
- `src/desktop/` window list and focus, one file per desktop
- `src/read/` OCR, Tesseract, accessibility trees (AT-SPI, UI Automation)
- `src/pointer.rs`, `src/pointer/` mouse and keyboard: enigo, RemoteDesktop portal
- `src/daemon/` background helper that keeps models loaded
- `src/act.rs`, `src/look.rs`, `src/main.rs` commands; `src/mcp.rs` MCP server

## Check before committing

```sh
cargo fmt --all --check
cargo clippy --all-targets -- -D warnings
cargo clippy --target x86_64-pc-windows-gnu --all-targets -- -D warnings
cargo test --locked
```

## Rules

- A fix for one platform must not change another. Gate code with `cfg`, and
  run clippy for Linux and Windows.
- Speed is the point. Anything that waits on another process (D-Bus, IPC,
  portals) needs a timeout. Never add a sleep without saying why it is needed.
- A change must keep accuracy, speed and performance the same or better.
  If it touches capture, reading or input, compare `bench/measure.py`
  before and after, and give the numbers in the PR.
- Remove dead code: unused functions, imports, cfg branches and stale
  comments. CLI commands and flags are exempt, since users and scripts call
  them even when nothing in the code does.
- Coordinates are logical desktop pixels on Linux and physical pixels on Windows.
- Keep diffs small. Don't refactor or reformat code you weren't asked to touch.
- Comments explain why, in one short line. Don't narrate what the code does.
- Add a test for new logic: the smallest one that fails if the logic breaks.

## Commits

- The subject is 50 characters or fewer, in the imperative: `Fix X on Y`.
- Add a body only when the reason isn't obvious. Keep it to a few lines.
- The author is the GitHub account's Gmail address, set with
  `git config user.email github-account-gmail`. Never use a noreply address.
- Never mention the AI tool or model in commits, PRs, comments or code: no
  `Co-Authored-By` trailers and no "Generated with" lines.
- This repo is public. Write patterns, not real data: no emails, tokens,
  webhooks or personal paths in code, docs or commits.
