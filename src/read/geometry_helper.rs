//! GNOME extension and one-shot KWin geometry queries.

use super::geometry::{placement_of, HyprlandClient, Placement};
use anyhow::{Context, Result};
use std::{io::Write, sync::mpsc, time::Duration};
use zbus::blocking::{connection::Builder, Connection, Proxy};

const INTERFACE: &str = "org.screenpeek.Windows";
const PATH: &str = "/org/screenpeek/Windows";

pub fn windows() -> Result<Vec<Placement>> {
    let connection = Connection::session()?;
    let listed: String = Proxy::new(&connection, INTERFACE, PATH, INTERFACE)?
        .call("List", &())
        .or_else(|_| kwin())
        .context("GNOME needs helpers/gnome installed; KDE needs KWin scripting enabled")?;
    parse(&listed)
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

fn kwin() -> Result<String, zbus::Error> {
    let (sender, receiver) = mpsc::sync_channel(1);
    let connection = Builder::session()?
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
        "const SCREENPEEK_CALLBACK = {:?};\n{}",
        connection.unique_name().unwrap().as_str(),
        include_str!("../../helpers/kwin/windows.js")
    )?;
    script.flush()?;
    let id: i32 = scripting.call("loadScript", &(&name, &name))?;
    if id < 0 {
        return Err(zbus::Error::Failure(
            "KWin refused the geometry script".into(),
        ));
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
            .map_err(|_| zbus::Error::Failure("KWin geometry query timed out".into()))
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

    #[test]
    fn helper_reply_travels_over_a_private_bus() -> Result<()> {
        let (sender, receiver) = mpsc::sync_channel(1);
        let (left, right) = std::os::unix::net::UnixStream::pair()?;
        let guid = zbus::Guid::generate();
        let server = std::thread::spawn(move || {
            Builder::async_io_unix_stream(left)
                .server(guid)
                .unwrap()
                .p2p()
                .serve_at(PATH, Receiver(sender))
                .unwrap()
                .build()
                .unwrap()
        });
        let client = Builder::async_io_unix_stream(right).p2p().build()?;
        let _server = server.join().unwrap();
        client.call_method(None::<&str>, PATH, Some(INTERFACE), "Reply", &("[]",))?;
        assert_eq!(receiver.recv_timeout(Duration::from_secs(1))?, "[]");
        Ok(())
    }
}
