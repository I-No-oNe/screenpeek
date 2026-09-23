//! GNOME extension and one-shot KWin geometry queries.

use super::geometry::{placement_of, HyprlandClient};
use super::Placement;
use anyhow::{Context, Result};
use std::{io::Write, sync::mpsc, time::Duration};
use zbus::blocking::{connection::Builder, Proxy};

const INTERFACE: &str = "org.screenpeek.Windows";
const PATH: &str = "/org/screenpeek/Windows";

pub fn windows() -> Result<Vec<Placement>> {
    let connection = crate::portal::session()?;
    let listed: String = Proxy::new(&connection, INTERFACE, PATH, INTERFACE)?
        .call("List", &())
        .or_else(|_| kwin(include_str!("../../helpers/kwin/windows.js"), ""))
        .map_err(|_| anyhow::anyhow!(missing_helper()))?;
    parse(&listed)
}

/// What to do when neither the GNOME extension nor KWin answers.
fn missing_helper() -> &'static str {
    match std::env::var("XDG_CURRENT_DESKTOP") {
        Ok(desktop) if desktop.contains("GNOME") => {
            "run `sh helpers/gnome/install.sh`, log in again, then `gnome-extensions enable screenpeek@screenpeek`"
        }
        _ => "this desktop does not report window positions (KDE needs KWin scripting enabled)",
    }
}

pub fn focus(handle: &str) -> Result<()> {
    let connection = crate::portal::session()?;
    let focused =
        match Proxy::new(&connection, INTERFACE, PATH, INTERFACE)?.call("Focus", &(handle,)) {
            Ok(focused) => focused,
            Err(_) => {
                let target = format!("const SCREENPEEK_TARGET = {handle:?};\n");
                kwin(include_str!("../../helpers/kwin/focus.js"), &target)
                    .map_err(|_| anyhow::anyhow!(missing_helper()))?
                    == "ok"
            }
        };
    anyhow::ensure!(focused, "the window is gone");
    Ok(())
}

/// Whether GNOME runs an older copy of the extension, from before the last update.
pub fn outdated_extension() -> bool {
    let Ok(connection) = crate::portal::session() else {
        return false;
    };
    let described: zbus::Result<String> = Proxy::new(
        &connection,
        INTERFACE,
        PATH,
        "org.freedesktop.DBus.Introspectable",
    )
    .and_then(|proxy| proxy.call("Introspect", &()));
    // Commit is the newest method; GNOME loads new extension code only at login.
    described.is_ok_and(|xml| !xml.contains("Commit"))
}

/// Type text through the GNOME extension's input method; false when it cannot.
pub fn commit(text: &str) -> bool {
    crate::portal::session()
        .and_then(|connection| {
            Proxy::new(&connection, INTERFACE, PATH, INTERFACE)?.call("Commit", &(text,))
        })
        .unwrap_or(false)
}

fn parse(listed: &str) -> Result<Vec<Placement>> {
    let clients: Vec<HyprlandClient> =
        serde_json::from_str(listed).context("window helper returned invalid geometry")?;
    Ok(clients.into_iter().filter_map(placement_of).collect())
}

struct Receiver(mpsc::SyncSender<String>);

#[zbus::interface(name = "org.screenpeek.Windows")]
impl Receiver {
    fn reply(&self, windows: String) {
        let _ = self.0.try_send(windows);
    }
}

/// Run a one-shot KWin script that answers through `callDBus`.
fn kwin(source: &str, preamble: &str) -> Result<String, zbus::Error> {
    let (sender, receiver) = mpsc::sync_channel(1);
    let connection = Builder::session()?
        .method_timeout(Duration::from_secs(5))
        .serve_at(PATH, Receiver(sender))?
        .build()?;
    let scripting = Proxy::new(
        &connection,
        "org.kde.KWin",
        "/Scripting",
        "org.kde.kwin.Scripting",
    )?;
    let mut script = tempfile::NamedTempFile::new()?;
    let name = script.path().to_string_lossy().into_owned();
    writeln!(
        script,
        "const SCREENPEEK_CALLBACK = {:?};\n{preamble}{source}",
        connection.unique_name().unwrap().as_str(),
    )?;
    script.flush()?;
    let id: i32 = scripting.call("loadScript", &(&name, &name))?;
    if id < 0 {
        return Err(zbus::Error::Failure("KWin refused the script".into()));
    }
    let result = (|| {
        let path = format!("/Scripting/Script{id}");
        Proxy::new(
            &connection,
            "org.kde.KWin",
            path.as_str(),
            "org.kde.kwin.Script",
        )?
        .call::<_, _, ()>("run", &())?;
        receiver
            .recv_timeout(Duration::from_secs(3))
            .map_err(|_| zbus::Error::Failure("KWin script timed out".into()))
    })();
    let _ = scripting.call::<_, _, bool>("unloadScript", &(&name,));
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn helper_geometry_preserves_offsets_and_rejects_bad_shapes() {
        let windows = parse(r#"[
            {"title":"Editor","at":[-1200,32],"size":[800,600],"mapped":true,"focusHistoryID":0,"pid":42},
            {"title":"Hidden","at":[0,0],"size":[0,0],"mapped":false,"focusHistoryID":1}
        ]"#).unwrap();
        assert_eq!(windows.len(), 1);
        assert_eq!(
            (windows[0].x, windows[0].y, windows[0].pid),
            (-1200, 32, Some(42))
        );
        assert!(windows[0].focused);
        assert!(parse("not JSON").is_err());
        assert!(parse(r#"[{"title":"bad"}]"#).is_err());
    }
}
