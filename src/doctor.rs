//! `screenpeek doctor`: what this desktop supports and what is missing.

use anyhow::{Context, Result};

#[cfg(any(target_os = "linux", windows))]
use crate::capture;
use crate::{daemon, read};

/// One line per part screenpeek needs: `ok` with what it uses, or `fix` with why.
pub(crate) fn doctor() {
    platform();
    if let Some(codes) = read::language::configured() {
        report(
            "languages",
            read::tesseract::installed()
                .map(|_| codes)
                .context(read::tesseract::INSTALL_HINT),
        );
    }
    report(
        "daemon",
        daemon::endpoint_summary().or_else(|_| Ok("starts on the first scan".into())),
    );
}

fn report(part: &str, result: Result<String>) {
    match result {
        Ok(detail) => out!("ok   {part}: {detail}"),
        Err(error) => out!("fix  {part}: {error:#}"),
    }
}

#[cfg(target_os = "linux")]
fn platform() {
    let desktop = std::env::var("XDG_CURRENT_DESKTOP").unwrap_or_else(|_| "unknown".into());
    let session = std::env::var("XDG_SESSION_TYPE").unwrap_or_else(|_| "unknown".into());
    out!("     desktop: {desktop} on {session}");
    report(
        "capture",
        capture::Backend::new().map(|backend| backend.name().to_owned()),
    );
    let portal = std::env::var("SCREENPEEK_INPUT").as_deref() == Ok("portal")
        || (capture::wayland::Screencopy::new().is_err()
            && capture::wayland::logical_desktop().is_some());
    report(
        "input",
        Ok(match portal {
            true => "portal, asks once per desktop".into(),
            false => "virtual keyboard and pointer".into(),
        }),
    );
    if matches!(capture::Backend::new(), Ok(capture::Backend::Portal)) {
        report("screen stream", frame_helper());
    }
    report(
        "windows",
        read::geometry::windows().map(|windows| format!("{} visible", windows.len())),
    );
    if read::geometry_helper::outdated_extension() {
        out!("fix  GNOME extension: an older copy is running; log out and back in to load the new one");
    }
    report(
        "accessibility",
        read::atspi::windows()
            .map(|windows| format!("{} windows describe themselves", windows.len())),
    );
}

#[cfg(windows)]
fn platform() {
    out!("     desktop: Windows");
    report(
        "capture",
        capture::screen(None, None).map(|shot| {
            let (width, height) = shot.image.dimensions();
            format!("{width}x{height} screen")
        }),
    );
    report(
        "input",
        crate::pointer::Pointer::new().map(|_| "SendInput".into()),
    );
    report(
        "windows",
        read::ui::windows().map(|windows| format!("{} visible", windows.len())),
    );
    report(
        "accessibility",
        read::ui::elements()
            .map(|elements| format!("UI Automation, {} named elements", elements.len())),
    );
}

#[cfg(not(any(target_os = "linux", windows)))]
fn platform() {
    out!("     desktop: not supported yet");
}

/// Whether the KDE and GNOME frame helper is installed and its libraries load.
#[cfg(target_os = "linux")]
fn frame_helper() -> Result<String> {
    let helper = std::env::current_exe()?.with_file_name("screenpeek-frames");
    let output = std::process::Command::new(&helper)
        .arg("--version")
        .output()
        .with_context(|| {
            format!(
                "{} is missing; scans use slower screenshots",
                helper.display()
            )
        })?;
    match output.status.success() {
        true => Ok("PipeWire".into()),
        false => anyhow::bail!(
            "{} does not start (install PipeWire's libraries); scans use slower screenshots",
            helper.display()
        ),
    }
}
