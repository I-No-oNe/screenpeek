//! Screen frames from the desktop's screen-cast stream, through `screenpeek-frames`.
//! KDE and GNOME only: the helper is the one binary that needs libpipewire.

use std::io::{BufRead, BufReader, Read, Write};
use std::os::fd::{AsRawFd, OwnedFd};
use std::os::unix::process::CommandExt;
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::time::Duration;

use anyhow::{bail, Context, Result};
use image::RgbaImage;

use super::Capture;
use crate::pointer::Screen;

pub struct Frames {
    child: Child,
    requests: ChildStdin,
    replies: BufReader<ChildStdout>,
    screens: Vec<Screen>,
}

impl Frames {
    pub fn start(remote: OwnedFd, screens: Vec<Screen>) -> Result<Frames> {
        let helper = std::env::current_exe()?.with_file_name("screenpeek-frames");
        let fd = remote.as_raw_fd();
        let mut command = Command::new(&helper);
        command
            .args(screens.iter().map(|(node, _, _)| node.to_string()))
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null());
        // The helper reads the PipeWire connection from fd 3.
        unsafe {
            command.pre_exec(move || match dup2(fd, 3) {
                -1 => Err(std::io::Error::last_os_error()),
                _ => Ok(()),
            });
        }
        let mut child = command
            .spawn()
            .with_context(|| format!("cannot start {}", helper.display()))?;
        drop(remote);
        Ok(Frames {
            requests: child.stdin.take().context("helper has no stdin")?,
            replies: BufReader::new(child.stdout.take().context("helper has no stdout")?),
            child,
            screens,
        })
    }

    /// The whole desktop, after letting it settle for `settle`.
    pub fn capture(&mut self, settle: Duration) -> Result<Capture> {
        writeln!(self.requests, "{}", settle.as_millis())?;
        let left = self
            .screens
            .iter()
            .map(|(_, at, _)| at.0)
            .min()
            .unwrap_or(0);
        let top = self
            .screens
            .iter()
            .map(|(_, at, _)| at.1)
            .min()
            .unwrap_or(0);
        let right = self
            .screens
            .iter()
            .map(|(_, at, size)| at.0 + size.0 as i32)
            .max();
        let bottom = self
            .screens
            .iter()
            .map(|(_, at, size)| at.1 + size.1 as i32)
            .max();
        let (right, bottom) = (right.unwrap_or(0), bottom.unwrap_or(0));
        let mut desktop = RgbaImage::new((right - left) as u32, (bottom - top) as u32);
        for (_, at, size) in self.screens.clone() {
            let frame = self.read_frame()?;
            // Frames come in device pixels; everything else works in logical ones.
            let frame = match frame.dimensions() == size {
                true => frame,
                false => image::imageops::resize(
                    &frame,
                    size.0,
                    size.1,
                    image::imageops::FilterType::Triangle,
                ),
            };
            image::imageops::replace(
                &mut desktop,
                &frame,
                i64::from(at.0 - left),
                i64::from(at.1 - top),
            );
        }
        Ok(Capture {
            image: desktop,
            origin: (left, top),
        })
    }

    fn read_frame(&mut self) -> Result<RgbaImage> {
        let mut header = String::new();
        if self.replies.read_line(&mut header)? == 0 {
            bail!("the frame helper stopped");
        }
        let fields: Vec<&str> = header.split_whitespace().collect();
        let [width, height, stride, format] = fields.as_slice() else {
            bail!("the frame helper sent {header:?}");
        };
        let (width, height, stride): (u32, u32, usize) =
            (width.parse()?, height.parse()?, stride.parse()?);
        let mut bytes = vec![0; stride * height as usize];
        self.replies.read_exact(&mut bytes)?;
        if *format == "none" {
            bail!("the desktop sent no frame yet");
        }
        rgba(&bytes, width, height, stride, format)
    }
}

impl Drop for Frames {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// Rows of 4-byte pixels, `stride` bytes apart, as RGBA.
fn rgba(bytes: &[u8], width: u32, height: u32, stride: usize, format: &str) -> Result<RgbaImage> {
    let blue_first = match format {
        "BGRx" | "BGRA" => true,
        "RGBx" | "RGBA" => false,
        other => bail!("unexpected pixel format {other}"),
    };
    let row = width as usize * 4;
    if stride < row || bytes.len() < stride * height as usize {
        bail!("frame is smaller than {width}x{height}");
    }
    let mut pixels = Vec::with_capacity(row * height as usize);
    for line in bytes.chunks_exact(stride).take(height as usize) {
        for pixel in line[..row].as_chunks::<4>().0 {
            let (red, blue) = match blue_first {
                true => (pixel[2], pixel[0]),
                false => (pixel[0], pixel[2]),
            };
            pixels.extend_from_slice(&[red, pixel[1], blue, 255]);
        }
    }
    RgbaImage::from_raw(width, height, pixels).context("frame size mismatch")
}

unsafe extern "C" {
    fn dup2(old: i32, new: i32) -> i32;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn padded_blue_first_rows_become_rgba() {
        // Two pixels per row, padded to 12 bytes: blue, green, red, unused.
        let bytes = [
            1, 2, 3, 0, 4, 5, 6, 0, 9, 9, 9, 9, //
            7, 8, 9, 0, 10, 11, 12, 0, 9, 9, 9, 9,
        ];
        let image = rgba(&bytes, 2, 2, 12, "BGRx").unwrap();
        assert_eq!(image.get_pixel(0, 0).0, [3, 2, 1, 255]);
        assert_eq!(image.get_pixel(1, 1).0, [12, 11, 10, 255]);
        assert!(rgba(&bytes, 4, 2, 12, "BGRx").is_err());
    }
}
