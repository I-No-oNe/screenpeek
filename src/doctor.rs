//! : what this desktop supports and what is missing.

use anyhow::{Context, Result};

use crate::{capture, daemon, read};

/// One line per part screenpeek needs: `ok` with what it uses, or `fix` with why.
#[cfg(target_os = "linux")]
pub(crate) fn doctor() {
    let report = |part: &str, result: Result<String>| match result {
        Ok(detail) => println!("ok   {part}: {detail}"),
        Err(error) => println!("fix  {part}: {error:#}"),
    };
    let desktop = std::env::var("XDG_CURRENT_DESKTOP").unwrap_or_else(|_| "unknown".into());
    let session = std::env::var("XDG_SESSION_TYPE").unwrap_or_else(|_| "unknown".into());
    println!("     desktop: {desktop} on {session}");
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
        println!("fix  GNOME extension: an older copy is running; log out and back in to load the new one");
    }
    report(
        "accessibility",
        read::atspi::windows()
            .map(|windows| format!("{} windows describe themselves", windows.len())),
    );
    if let Some(codes) = read::language::configured() {
        report(
            "languages",
            read::tesseract::installed()
                .map(|_| codes)
                .context("install tesseract with your package manager"),
        );
    }
    report(
        "daemon",
        daemon::endpoint_summary().or_else(|_| Ok("starts on the first scan".into())),
    );
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
