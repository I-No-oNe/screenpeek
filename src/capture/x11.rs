//! Reuse an X11 connection to capture root-window pixels.

use anyhow::{anyhow, bail, Context, Result};
use image::RgbaImage;
use x11rb::connection::Connection;
use x11rb::protocol::randr::ConnectionExt as RandrExt;
use x11rb::protocol::xproto::{ConnectionExt, ImageFormat, Window};
use x11rb::rust_connection::RustConnection;

use super::{Capture, Region};

pub struct Screen {
    connection: RustConnection,
    root: Window,
    monitors: Vec<Region>,
}

impl Screen {
    pub fn new() -> Result<Screen> {
        let (connection, preferred) = x11rb::connect(None).context("no X display")?;
        let root = connection
            .setup()
            .roots
            .get(preferred)
            .ok_or_else(|| anyhow!("no screen {preferred}"))?
            .root;

        let monitors = monitors(&connection, root)?;
        Ok(Screen {
            connection,
            root,
            monitors,
        })
    }

    /// Captures a monitor, or the whole root window when there is only one.
    pub fn capture(&mut self, monitor: Option<usize>) -> Result<Capture> {
        let index = monitor.unwrap_or(0);
        let area = *self
            .monitors
            .get(index)
            .ok_or_else(|| anyhow!("no monitor {index}; found {}", self.monitors.len()))?;
        self.capture_area(area)
    }

    /// Captures the monitor a region starts on.
    pub fn capture_containing(&mut self, region: Region) -> Result<Capture> {
        let area = self
            .monitors
            .iter()
            .find(|monitor| monitor.contains(region.x, region.y))
            .copied()
            .ok_or_else(|| anyhow!("no monitor contains {},{}", region.x, region.y))?;
        self.capture_area(area)
    }

    fn capture_area(&self, area: Region) -> Result<Capture> {
        let image = self
            .connection
            .get_image(
                ImageFormat::Z_PIXMAP,
                self.root,
                area.x as i16,
                area.y as i16,
                area.width as u16,
                area.height as u16,
                !0,
            )
            .context("cannot ask X for the screen")?
            .reply()
            .context("X refused the capture")?;

        Ok(Capture {
            image: to_rgba(&image.data, area.width, area.height)?,
            origin: (area.x, area.y),
        })
    }
}

/// X sends 32 bits per pixel in this format, blue first, with the top byte
/// unused.
fn to_rgba(data: &[u8], width: u32, height: u32) -> Result<RgbaImage> {
    let expected = width as usize * height as usize * 4;
    if data.len() < expected {
        bail!("X returned {} bytes, expected {expected}", data.len());
    }

    let mut pixels = Vec::with_capacity(expected);
    for pixel in data[..expected].as_chunks::<4>().0 {
        pixels.extend_from_slice(&[pixel[2], pixel[1], pixel[0], 255]);
    }

    RgbaImage::from_raw(width, height, pixels).ok_or_else(|| anyhow!("the capture is malformed"))
}

/// The monitors RandR reports, or the whole root window when it reports none.
fn monitors(connection: &RustConnection, root: Window) -> Result<Vec<Region>> {
    let geometry = connection.get_geometry(root)?.reply()?;
    let whole = Region {
        x: 0,
        y: 0,
        width: geometry.width as u32,
        height: geometry.height as u32,
    };

    let Ok(reply) = connection.randr_get_monitors(root, true) else {
        return Ok(vec![whole]);
    };
    let Ok(reply) = reply.reply() else {
        return Ok(vec![whole]);
    };

    let mut monitors: Vec<Region> = reply
        .monitors
        .iter()
        .filter(|monitor| monitor.width > 0 && monitor.height > 0)
        .map(|monitor| Region {
            x: monitor.x as i32,
            y: monitor.y as i32,
            width: monitor.width as u32,
            height: monitor.height as u32,
        })
        .collect();

    // The primary monitor is reported first, so a bare scan reads it.
    if let Some(primary) = reply.monitors.iter().position(|monitor| monitor.primary) {
        monitors.swap(0, primary);
    }

    Ok(if monitors.is_empty() {
        vec![whole]
    } else {
        monitors
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pixels_arrive_blue_first() {
        let data = [10u8, 20, 30, 0, 40, 50, 60, 0];
        let image = to_rgba(&data, 2, 1).unwrap();
        assert_eq!(image.get_pixel(0, 0).0, [30, 20, 10, 255]);
        assert_eq!(image.get_pixel(1, 0).0, [60, 50, 40, 255]);
    }

    #[test]
    fn a_short_reply_is_an_error() {
        assert!(to_rgba(&[0, 0, 0, 0], 4, 4).is_err());
    }
}
