//! A private desktop beside the user's: a nested headless Sway with its own
//! screen, cursor and keyboard focus, so an agent works there while the user
//! keeps working on their own desktop.

/// Whether this process reads and acts on the private desktop.
pub(crate) fn active() -> bool {
    std::env::var_os("SCREENPEEK_HEADLESS").is_some_and(|value| value == "1")
}

/// Added to per-desktop cache names, so ids and the daemon stay apart.
pub(crate) fn suffix() -> &'static str {
    match active() {
        true => "-headless",
        false => "",
    }
}

/// The processes with windows on the private desktop, when this process works
/// there. It shares the user's accessibility bus, and asking each of their
/// apps cost up to 400 ms a scan.
#[cfg(target_os = "linux")]
pub(crate) fn owners(placements: &[crate::desktop::Placement]) -> Option<Vec<u32>> {
    active().then(|| placements.iter().filter_map(|window| window.pid).collect())
}

#[cfg(target_os = "linux")]
pub(crate) use linux::*;

#[cfg(target_os = "linux")]
mod linux {
    use std::env;
    use std::fs;
    use std::os::unix::net::UnixStream;
    use std::os::unix::process::CommandExt;
    use std::path::{Path, PathBuf};
    use std::process::{Command, Stdio};
    use std::time::{Duration, Instant};

    use anyhow::{bail, Context, Result};

    /// Sway takes a fraction of a second to start or stop; give a slow machine plenty.
    const START_TIMEOUT: Duration = Duration::from_secs(10);

    /// A new window takes focus as it maps; a desktop that does not do it
    /// within this never will.
    const FOCUS_TAKEN: Duration = Duration::from_secs(2);

    /// Gap between checks on the session, which has no event to wait on.
    const START_POLL: Duration = Duration::from_millis(20);

    /// What a started private desktop needs to be reached.
    struct Session {
        display: String,
        socket: PathBuf,
    }

    impl Session {
        fn read() -> Option<Session> {
            let text = fs::read_to_string(state_path().ok()?).ok()?;
            let mut lines = text.lines();
            let session = Session {
                display: lines.next()?.to_owned(),
                socket: lines.next()?.into(),
            };
            UnixStream::connect(&session.socket).ok().map(|_| session)
        }
    }

    fn state_dir() -> Result<PathBuf> {
        let runtime = env::var_os("XDG_RUNTIME_DIR").context("no XDG_RUNTIME_DIR")?;
        Ok(PathBuf::from(runtime).join("screenpeek"))
    }

    fn state_path() -> Result<PathBuf> {
        Ok(state_dir()?.join("headless"))
    }

    /// What `enter` changes; `leave` puts the user's values back.
    const SWITCHED: &[&str] = &[
        "WAYLAND_DISPLAY",
        "SWAYSOCK",
        "XDG_CURRENT_DESKTOP",
        "HYPRLAND_INSTANCE_SIGNATURE",
        "I3SOCK",
        "DISPLAY",
    ];

    /// Set once the user's values are saved, so entering twice keeps the first.
    const SAVED: &str = "SCREENPEEK_USER_SAVED";

    fn saved(name: &str) -> String {
        format!("SCREENPEEK_USER_{name}")
    }

    /// Point this process, and what it starts, at the private desktop.
    pub(crate) fn enter() -> Result<()> {
        let session = Session::read()
            .context("no private desktop is running; start one with `screenpeek headless start`")?;
        if env::var_os(SAVED).is_none() {
            for name in SWITCHED {
                env::set_var(saved(name), env::var_os(name).unwrap_or_default());
            }
            env::set_var(SAVED, "1");
        }
        env::set_var("SCREENPEEK_HEADLESS", "1");
        env::set_var("WAYLAND_DISPLAY", &session.display);
        env::set_var("SWAYSOCK", &session.socket);
        env::set_var("XDG_CURRENT_DESKTOP", "sway");
        // enigo logs every keymap-less moment of a seat with no keyboard.
        env::set_var("XKB_LOG_LEVEL", "critical");
        // The user's own desktop must not answer for this one.
        for name in ["HYPRLAND_INSTANCE_SIGNATURE", "I3SOCK", "DISPLAY"] {
            env::remove_var(name);
        }
        Ok(())
    }

    /// Point this process back at the user's desktop, if `enter` moved it.
    pub(crate) fn leave() {
        if env::var_os(SAVED).is_none() {
            return;
        }
        for name in SWITCHED {
            match env::var_os(saved(name)).filter(|value| !value.is_empty()) {
                Some(value) => env::set_var(name, value),
                None => env::remove_var(name),
            }
            env::remove_var(saved(name));
        }
        env::remove_var(SAVED);
        env::remove_var("SCREENPEEK_HEADLESS");
    }

    /// Open the app behind a window of the user's desktop, and optionally a
    /// page, on the private desktop. The user's copy stays where it is.
    pub(crate) fn bring(title: &str, url: Option<&str>) -> Result<()> {
        leave();
        let windows = crate::desktop::placements();
        let window = crate::act::pick_window(&windows, title)?;
        let pid = window
            .pid
            .context("this desktop does not say which program owns the window")?;
        let raw = fs::read(format!("/proc/{pid}/cmdline"))
            .with_context(|| format!("cannot read how process {pid} was started"))?;
        let mut command = without_profile(
            raw.split(|byte| *byte == 0)
                .filter(|part| !part.is_empty())
                .map(|part| String::from_utf8_lossy(part).into_owned())
                .collect(),
        );
        command.extend(url.map(str::to_owned));
        if Session::read().is_none() {
            start(DEFAULT_SIZE, false)?;
        }
        enter()?;
        crate::launch::launch(&command)
    }

    /// The user's profile is in use by their own copy, so drop it.
    fn without_profile(command: Vec<String>) -> Vec<String> {
        let mut kept = Vec::new();
        let mut parts = command.into_iter();
        while let Some(part) = parts.next() {
            match part.as_str() {
                "-P" | "-profile" | "--profile" | "--user-data-dir" => {
                    parts.next();
                }
                _ if part.starts_with("--user-data-dir=") || part.starts_with("--profile=") => {}
                _ => kept.push(part),
            }
        }
        kept
    }

    pub(crate) const DEFAULT_SIZE: &str = "1920x1080";

    pub(crate) fn start(size: &str, watch: bool) -> Result<()> {
        leave();
        if let Some(session) = Session::read() {
            out!("already running on {}", session.display);
            return Ok(());
        }
        let (width, height) = size
            .split_once('x')
            .and_then(|(w, h)| Some((w.parse::<u32>().ok()?, h.parse::<u32>().ok()?)))
            .context("--size looks like 1920x1080")?;
        let exe = env::current_exe()?;
        let exe = exe
            .to_str()
            .filter(|path| !path.contains(char::is_whitespace));
        let exe = exe.context("screenpeek's path has a space, which Sway's exec cannot run")?;

        let dir = state_dir()?;
        fs::create_dir_all(&dir)?;
        let _ = fs::remove_file(state_path()?);
        let config = dir.join("headless.conf");
        // Sway runs `ready` once its socket is up, and that writes the state file.
        fs::write(
            &config,
            format!(
                // Scale 1: captures need no resizing, the fastest to read.
                "output * resolution {width}x{height} scale 1\n\
                 default_border none\n\
                 exec {exe} headless ready\n"
            ),
        )?;

        let log = fs::File::create(dir.join("headless.log"))?;
        let mut sway = Command::new("sway");
        sway.arg("--config")
            .arg(&config)
            .env("XDG_CURRENT_DESKTOP", "sway")
            .env_remove("HYPRLAND_INSTANCE_SIGNATURE")
            .env_remove("SWAYSOCK")
            .env_remove("I3SOCK")
            .env_remove("SCREENPEEK_HEADLESS")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(log)
            // Its own group, so Ctrl-C in this terminal does not end it.
            .process_group(0);
        if watch {
            sway.env("WLR_BACKENDS", "wayland");
        } else {
            sway.env("WLR_BACKENDS", "headless")
                .env("WLR_LIBINPUT_NO_DEVICES", "1")
                .env_remove("WAYLAND_DISPLAY")
                .env_remove("DISPLAY");
            if !has_render_node() {
                sway.env("WLR_RENDERER", "pixman");
            }
        }
        // A new window takes focus on most desktops; the user's goes back after.
        let focused = match watch {
            true => crate::desktop::placements().into_iter().find(|w| w.focused),
            false => None,
        };
        sway.spawn()
            .context("cannot start sway; install it to use headless mode")?;

        let started = Instant::now();
        while started.elapsed() < START_TIMEOUT {
            if let Some(session) = Session::read() {
                if let Some(window) = &focused {
                    give_back_focus(window);
                }
                out!(
                    "private desktop on {} ({width}x{height}); use `screenpeek --headless ...` \
                     or SCREENPEEK_HEADLESS=1",
                    session.display
                );
                return Ok(());
            }
            std::thread::sleep(START_POLL);
        }
        bail!(
            "the private desktop did not start; see {}",
            dir.join("headless.log").display()
        )
    }

    /// Once the private desktop's window has taken focus, return it to `window`.
    fn give_back_focus(window: &crate::desktop::Placement) {
        let started = Instant::now();
        while started.elapsed() < FOCUS_TAKEN {
            let windows = crate::desktop::placements();
            if windows
                .iter()
                .any(|w| w.focused && w.handle != window.handle)
            {
                let _ = crate::desktop::focus(window);
                return;
            }
            std::thread::sleep(START_POLL);
        }
    }

    /// Called by Sway inside the new session: record how to reach it.
    pub(crate) fn ready() -> Result<()> {
        let display = env::var("WAYLAND_DISPLAY").context("no WAYLAND_DISPLAY")?;
        let socket = env::var("SWAYSOCK").context("no SWAYSOCK")?;
        let path = state_path()?;
        let partial = path.with_extension("partial");
        fs::write(&partial, format!("{display}\n{socket}\n"))?;
        fs::rename(&partial, &path)?;
        Ok(())
    }

    pub(crate) fn stop() -> Result<()> {
        let Some(session) = Session::read() else {
            out!("no private desktop is running");
            return Ok(());
        };
        // Sway quits before it answers, so its reply cannot tell; its socket can.
        let _ = crate::desktop::sway_command(&session.socket, "exit");
        let started = Instant::now();
        while Session::read().is_some() {
            if started.elapsed() > START_TIMEOUT {
                bail!("the private desktop on {} did not stop", session.display);
            }
            std::thread::sleep(START_POLL);
        }
        let _ = fs::remove_file(state_path()?);
        out!("stopped the private desktop on {}", session.display);
        Ok(())
    }

    pub(crate) fn status() {
        match Session::read() {
            Some(session) => out!("private desktop running on {}", session.display),
            None => out!("no private desktop is running"),
        }
    }

    /// Without a GPU render node it may open, wlroots needs its software renderer.
    fn has_render_node() -> bool {
        fs::read_dir(Path::new("/dev/dri")).is_ok_and(|entries| {
            entries.flatten().any(|entry| {
                entry.file_name().to_string_lossy().starts_with("renderD")
                    && fs::File::open(entry.path()).is_ok()
            })
        })
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn profiles_in_use_are_dropped() {
            let parts = |text: &str| text.split(' ').map(str::to_owned).collect::<Vec<_>>();
            assert_eq!(
                without_profile(parts("firefox -P work --new-window")),
                parts("firefox --new-window")
            );
            assert_eq!(
                without_profile(parts("chrome --user-data-dir=/x --incognito")),
                parts("chrome --incognito")
            );
        }
    }
}
