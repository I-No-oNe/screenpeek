//! Shared XDG portal requests.

use anyhow::{bail, Context, Result};
use serde::Serialize;
use std::collections::HashMap;
use zbus::blocking::{Connection, MessageIterator};
use zbus::zvariant::{DynamicType, OwnedObjectPath, OwnedValue};

pub const SERVICE: &str = "org.freedesktop.portal.Desktop";
pub const PATH: &str = "/org/freedesktop/portal/desktop";
pub type Values = HashMap<String, OwnedValue>;

pub fn request<B: Serialize + DynamicType>(
    connection: &Connection,
    interface: &str,
    method: &str,
    body: &B,
) -> Result<Values> {
    // Subscribe before calling: a portal may respond before the method returns.
    let signals = MessageIterator::for_match_rule(
        zbus::MatchRule::builder()
            .msg_type(zbus::message::Type::Signal)
            .sender(SERVICE)?
            .interface("org.freedesktop.portal.Request")?
            .member("Response")?
            .build(),
        connection,
        Some(16),
    )?;
    let path: OwnedObjectPath = connection
        .call_method(Some(SERVICE), PATH, Some(interface), method, body)?
        .body()
        .deserialize()?;
    let (sender, receiver) = std::sync::mpsc::sync_channel(1);
    let expected = path.clone();
    std::thread::spawn(move || {
        for message in signals {
            let result = match message {
                Ok(message) if message.header().path() == Some(&expected.as_ref()) => {
                    message.body().deserialize::<(u32, Values)>()
                }
                Ok(_) => continue,
                Err(error) => Err(error),
            };
            let _ = sender.send(result);
            break;
        }
    });
    let response = receiver.recv_timeout(std::time::Duration::from_secs(120));
    if response.is_err() {
        let _ = connection.call_method(
            Some(SERVICE),
            path.as_str(),
            Some("org.freedesktop.portal.Request"),
            "Close",
            &(),
        );
        let _ = connection.clone().close();
        bail!("portal request timed out after 120 seconds");
    }
    let (status, values) = response.context("portal closed without answering")??;
    if status != 0 {
        bail!("portal request was cancelled or denied (status {status})");
    }
    Ok(values)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{BufRead, BufReader};
    use std::process::{Command, Stdio};
    use zbus::blocking::connection::Builder;

    struct Bus(std::process::Child);
    impl Drop for Bus {
        fn drop(&mut self) {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }

    struct Screenshot;
    #[zbus::interface(name = "org.freedesktop.portal.Screenshot")]
    impl Screenshot {
        async fn screenshot(
            &self,
            status: u32,
            #[zbus(connection)] connection: &zbus::Connection,
        ) -> zbus::fdo::Result<OwnedObjectPath> {
            let path = "/org/freedesktop/portal/desktop/request/test/early";
            connection
                .emit_signal(
                    None::<&str>,
                    path,
                    "org.freedesktop.portal.Request",
                    "Response",
                    &(status, Values::new()),
                )
                .await?;
            Ok(path.try_into().unwrap())
        }
    }

    #[test]
    fn early_portal_response_is_not_lost_and_denial_is_an_error() -> Result<()> {
        let mut bus = Bus(Command::new("dbus-daemon")
            .args(["--session", "--nofork", "--print-address=1"])
            .stdout(Stdio::piped())
            .spawn()
            .context("tests need dbus-daemon")?);
        let mut address = String::new();
        BufReader::new(bus.0.stdout.take().unwrap()).read_line(&mut address)?;
        let _server = Builder::address(address.trim())?
            .name(SERVICE)?
            .serve_at(PATH, Screenshot)?
            .build()?;
        let client = Builder::address(address.trim())?.build()?;
        assert!(request(
            &client,
            "org.freedesktop.portal.Screenshot",
            "Screenshot",
            &(0u32,)
        )
        .is_ok());
        assert!(request(
            &client,
            "org.freedesktop.portal.Screenshot",
            "Screenshot",
            &(1u32,)
        )
        .is_err());
        Ok(())
    }
}
