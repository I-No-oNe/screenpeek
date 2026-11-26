//! Start an app with its accessibility tree switched on, so screenpeek reads
//! its controls by name instead of only through OCR.

use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use anyhow::{bail, Context, Result};

use crate::desktop::{self, Placement};

/// A browser's first window can take a few seconds on a cold start.
const WINDOW_TIMEOUT: Duration = Duration::from_secs(15);

/// Gap between looks at the window list, which has no event to wait on.
const WINDOW_POLL: Duration = Duration::from_millis(20);

/// Chromium and Electron apps hide their tree unless asked.
const CHROMIUM: &[&str] = &[
    "chrome", "chromium", "brave", "msedge", "edge", "vivaldi", "opera", "electron", "code",
    "codium", "slack", "discord", "obsidian",
];

pub(crate) fn launch(command: &[String]) -> Result<()> {
    let (program, given) = command.split_first().context("name the app to start")?;
    let name = Path::new(program)
        .file_stem()
        .map(|stem| stem.to_string_lossy().to_lowercase())
        .unwrap_or_default();
    let mut child = Command::new(program);
    child
        .args(given)
        .args(extra_args(&name, given))
        // GTK, Firefox and Qt read these.
        .env("GNOME_ACCESSIBILITY", "1")
        .env("ACCESSIBILITY_ENABLED", "1")
        .env("QT_LINUX_ACCESSIBILITY_ALWAYS_ON", "1")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    #[cfg(unix)]
    std::os::unix::process::CommandExt::process_group(&mut child, 0);
    let before = desktop::list();
    let mut child = child
        .spawn()
        .with_context(|| format!("cannot start {program}"))?;
    let Ok(before) = before else {
        out!(
            "started {program} (pid {}); this desktop cannot list windows to confirm it",
            child.id()
        );
        return Ok(());
    };
    let window = await_window(&mut child, &before, program)?;
    out!("{program} opened {}", crate::act::describe(&window));
    Ok(())
}

/// Confirm the app opened a window, failing as soon as it exits with an error.
fn await_window(child: &mut Child, before: &[Placement], program: &str) -> Result<Placement> {
    let started = Instant::now();
    let mut launcher_done = false;
    loop {
        let windows = desktop::placements();
        if let Some(window) = opened(&windows, child.id(), before, launcher_done) {
            return Ok(window.clone());
        }
        if !launcher_done {
            if let Some(status) = child.try_wait()? {
                if !status.success() {
                    bail!("{program} exited ({status}) before opening a window");
                }
                launcher_done = true;
                continue;
            }
        }
        if started.elapsed() > WINDOW_TIMEOUT {
            bail!(
                "{program} opened no window within {}s; it may have handed off to a copy \
                 already running",
                WINDOW_TIMEOUT.as_secs()
            );
        }
        std::thread::sleep(WINDOW_POLL);
    }
}

/// The app's window: its own process's, or, once a launcher that hands off
/// to another process has exited cleanly, one that was not there before.
fn opened<'a>(
    windows: &'a [Placement],
    pid: u32,
    before: &[Placement],
    launcher_done: bool,
) -> Option<&'a Placement> {
    let new = |window: &Placement| {
        window.handle.is_some()
            && !before
                .iter()
                .any(|old| old.handle == window.handle && old.pid == window.pid)
    };
    windows
        .iter()
        .find(|window| window.pid == Some(pid))
        .or_else(|| windows.iter().find(|window| launcher_done && new(window)))
}

/// Flags for the app `name`: its tree on, and on the private desktop, a
/// profile of its own, since a browser already open elsewhere would otherwise
/// take the window.
fn extra_args(name: &str, given: &[String]) -> Vec<String> {
    let chromium = CHROMIUM
        .iter()
        .any(|family| name.starts_with(family) || name.contains(&format!("-{family}")));
    let firefox = name.contains("firefox") || name.contains("librewolf");
    let mut extra = Vec::new();
    if chromium {
        extra.push("--force-renderer-accessibility".to_owned());
    }
    if !crate::headless::active() {
        return extra;
    }
    let profile_given = given.iter().any(|arg| {
        arg == "-P" || arg.starts_with("--profile") || arg.starts_with("--user-data-dir")
    });
    let profile = dirs::cache_dir().map(|cache| {
        cache
            .join("screenpeek")
            .join("headless-profiles")
            .join(name)
    });
    match (profile, profile_given) {
        (Some(profile), false) if chromium => {
            extra.push(format!("--user-data-dir={}", profile.display()));
            extra.push("--ozone-platform=wayland".to_owned());
        }
        (Some(profile), false) if firefox => {
            let _ = std::fs::create_dir_all(&profile);
            extra.extend(["--new-instance".to_owned(), "--profile".to_owned()]);
            extra.push(profile.display().to_string());
        }
        _ => {}
    }
    extra
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_the_apps_own_or_a_handed_off_window_counts() {
        let window = |handle: &str, pid| Placement {
            handle: Some(handle.into()),
            pid: Some(pid),
            ..Default::default()
        };
        let before = [window("1", 10)];
        let after = [window("1", 10), window("2", 20)];
        assert_eq!(opened(&after, 20, &before, false), Some(&after[1]));
        // Another app's window is not proof while the launcher still runs.
        assert_eq!(opened(&after, 30, &before, false), None);
        assert_eq!(opened(&after, 30, &before, true), Some(&after[1]));
        assert_eq!(opened(&before, 30, &before, true), None);
    }

    #[test]
    fn chromium_family_gets_its_tree_switched_on() {
        assert_eq!(
            extra_args("google-chrome-stable", &[]),
            ["--force-renderer-accessibility"]
        );
        assert!(extra_args("firefox", &[]).is_empty());
        assert!(extra_args("gedit", &[]).is_empty());
    }
}
