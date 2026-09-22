//! Capture through the XDG Screenshot portal.

use anyhow::{anyhow, bail, Context, Result};
use std::collections::HashMap;
use std::fs;
use zbus::blocking::Connection;
use zbus::zvariant::{OwnedValue, Value};

use super::Capture;

pub struct Portal {
    connection: Connection,
}

impl Portal {
    pub fn new() -> Result<Portal> {
        let connection = Connection::session().context("no session bus")?;
        // Ask for the interface's version, which fails when no portal is
        // running rather than when the first capture is wanted.
        let _: u32 = connection
            .call_method(
                Some("org.freedesktop.portal.Desktop"),
                "/org/freedesktop/portal/desktop",
                Some("org.freedesktop.DBus.Properties"),
                "Get",
                &("org.freedesktop.portal.Screenshot", "version"),
            )
            .context("no screenshot portal on this session")?
            .body()
            .deserialize::<OwnedValue>()
            .ok()
            .and_then(|value| u32::try_from(value).ok())
            .ok_or_else(|| anyhow!("the screenshot portal did not answer"))?;

        Ok(Portal { connection })
    }

    /// One screenshot of the whole desktop, in its own pixels.
    pub fn capture(&mut self) -> Result<Capture> {
        let token = format!("screenpeek{}", std::process::id());
        let mut options: HashMap<&str, Value> = HashMap::new();
        options.insert("handle_token", Value::from(token.as_str()));
        // Neither a dialog nor a modal parent: a scan is not a moment for
        // either, and a desktop that insists will remember the answer.
        options.insert("interactive", Value::from(false));
        options.insert("modal", Value::from(false));

        let results = crate::portal::request(
            &self.connection,
            "org.freedesktop.portal.Screenshot",
            "Screenshot",
            &("", options),
        )?;
        let uri = results
            .get("uri")
            .and_then(|value| String::try_from(value.try_clone().ok()?).ok())
            .context("portal answered without a screenshot URI")?;
        let path = file_path(&uri)?;
        let image = image::open(&path)
            .with_context(|| format!("cannot read the portal's screenshot at {path:?}"))?
            .into_rgba8();
        // The portal hands the file over; nobody else is going to remove it.
        let _ = fs::remove_file(&path);

        // Normalize screenshot pixels to logical desktop coordinates before OCR.
        let (image, origin) = match super::wayland::logical_desktop() {
            Some((x, y, width, height)) if (width, height) != image.dimensions() => (
                image::imageops::resize(
                    &image,
                    width,
                    height,
                    image::imageops::FilterType::Triangle,
                ),
                (x, y),
            ),
            Some((x, y, _, _)) => (image, (x, y)),
            None => {
                bail!("portal capture needs logical display geometry; no xdg-output data available")
            }
        };

        Ok(Capture { image, origin })
    }
}

fn file_path(uri: &str) -> Result<std::path::PathBuf> {
    use std::os::unix::ffi::OsStringExt;
    let encoded = uri
        .strip_prefix("file:///")
        .context("portal returned a non-local file URI")?;
    let bytes = encoded.as_bytes();
    let mut decoded = vec![b'/'];
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' {
            let hex = encoded.get(i + 1..i + 3).context("invalid URI escape")?;
            decoded.push(u8::from_str_radix(hex, 16).context("invalid URI escape")?);
            i += 3;
        } else {
            decoded.push(bytes[i]);
            i += 1;
        }
    }
    if decoded.contains(&0) {
        bail!("NUL in screenshot path");
    }
    Ok(std::ffi::OsString::from_vec(decoded).into())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn portal_file_uris_are_decoded_and_must_be_local() {
        assert_eq!(
            file_path("file:///tmp/a%20b%23c%25.png").unwrap(),
            std::path::Path::new("/tmp/a b#c%.png")
        );
        for bad in [
            "https://host/file",
            "file://host/file",
            "file:///tmp/%",
            "file:///tmp/%00",
        ] {
            assert!(file_path(bad).is_err());
        }
    }
}
