//! Screen capture in virtual-desktop coordinates, the space the pointer uses.

use std::fmt;
use std::str::FromStr;

use anyhow::{bail, Context, Result};
use image::RgbaImage;
use serde::{Deserialize, Serialize};

#[cfg(not(target_os = "linux"))]
use anyhow::anyhow;
#[cfg(not(target_os = "linux"))]
use xcap::Monitor;

/// A rectangle on the virtual desktop.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Region {
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
}

impl Region {
    pub fn contains(&self, x: i32, y: i32) -> bool {
        x >= self.x
            && y >= self.y
            && x < self.x + self.width as i32
            && y < self.y + self.height as i32
    }
}

impl FromStr for Region {
    type Err = anyhow::Error;

    /// Parses `x,y,width,height`.
    fn from_str(s: &str) -> Result<Self> {
        let parts: Vec<&str> = s.split(',').map(str::trim).collect();
        let [x, y, width, height] = parts.as_slice() else {
            bail!("expected x,y,width,height, got {s:?}");
        };
        let region = Region {
            x: x.parse().context("bad x")?,
            y: y.parse().context("bad y")?,
            width: width.parse().context("bad width")?,
            height: height.parse().context("bad height")?,
        };
        if region.width == 0 || region.height == 0 {
            bail!("region has no area");
        }
        Ok(region)
    }
}

impl fmt::Display for Region {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{},{},{},{}", self.x, self.y, self.width, self.height)
    }
}

/// A captured image and the desktop position of its top-left pixel.
pub struct Capture {
    pub image: RgbaImage,
    pub origin: (i32, i32),
}

impl Capture {
    /// Wraps a loaded image, positioned at the origin.
    pub fn from_image(image: RgbaImage) -> Capture {
        Capture {
            image,
            origin: (0, 0),
        }
    }

    /// Translates a point inside the captured image to desktop coordinates.
    pub fn to_desktop(&self, x: i32, y: i32) -> (i32, i32) {
        (self.origin.0 + x, self.origin.1 + y)
    }
}

/// Captures a monitor, or the part of one covered by `region`. A region is
/// taken from the monitor it starts on, so scan coordinates can be fed back in.
pub fn capture(monitor: Option<usize>, region: Option<Region>) -> Result<Capture> {
    let full = full_screen(monitor, region)?;
    let Some(region) = region else {
        return Ok(full);
    };
    Ok(crop(full, region))
}

/// Narrows a capture to a region, clamped to what was actually captured.
pub fn crop(capture: Capture, region: Region) -> Capture {
    let x = (region.x - capture.origin.0).max(0) as u32;
    let y = (region.y - capture.origin.1).max(0) as u32;
    let width = region.width.min(capture.image.width().saturating_sub(x));
    let height = region.height.min(capture.image.height().saturating_sub(y));
    if width == 0 || height == 0 {
        return capture;
    }

    Capture {
        image: image::imageops::crop_imm(&capture.image, x, y, width, height).to_image(),
        origin: (capture.origin.0 + x as i32, capture.origin.1 + y as i32),
    }
}

#[cfg(target_os = "linux")]
fn full_screen(monitor: Option<usize>, region: Option<Region>) -> Result<Capture> {
    let mut screencopy = crate::wayland::Screencopy::new()?;
    match region {
        Some(region) => screencopy.capture_containing(region),
        None => screencopy.capture(monitor),
    }
}

#[cfg(not(target_os = "linux"))]
fn full_screen(monitor: Option<usize>, region: Option<Region>) -> Result<Capture> {
    let monitors = Monitor::all().context("cannot enumerate monitors")?;
    if monitors.is_empty() {
        bail!("no monitors found");
    }

    let monitor = match region {
        Some(region) => monitors
            .iter()
            .find(|monitor| bounds(monitor).is_ok_and(|bounds| bounds.contains(region.x, region.y)))
            .ok_or_else(|| anyhow!("no monitor contains {},{}", region.x, region.y))?,
        None => match monitor {
            Some(index) => monitors
                .get(index)
                .ok_or_else(|| anyhow!("no monitor {index}; found {}", monitors.len()))?,
            None => primary(&monitors)?,
        },
    };

    Ok(Capture {
        image: monitor.capture_image().context("screen capture failed")?,
        origin: (monitor.x()?, monitor.y()?),
    })
}

#[cfg(not(target_os = "linux"))]
fn primary(monitors: &[Monitor]) -> Result<&Monitor> {
    if let Some(monitor) = monitors
        .iter()
        .find(|monitor| monitor.is_primary().unwrap_or(false))
    {
        return Ok(monitor);
    }
    monitors.first().ok_or_else(|| anyhow!("no monitors found"))
}

#[cfg(not(target_os = "linux"))]
fn bounds(monitor: &Monitor) -> Result<Region> {
    Ok(Region {
        x: monitor.x()?,
        y: monitor.y()?,
        width: monitor.width()?,
        height: monitor.height()?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_a_region() {
        assert_eq!(
            "10,20,300,400".parse::<Region>().unwrap(),
            Region {
                x: 10,
                y: 20,
                width: 300,
                height: 400
            }
        );
        assert_eq!("-10, 0, 8, 8".parse::<Region>().unwrap().x, -10);
    }

    #[test]
    fn rejects_nonsense_regions() {
        for bad in ["", "1,2,3", "1,2,3,4,5", "a,b,c,d", "0,0,0,10"] {
            assert!(bad.parse::<Region>().is_err(), "{bad:?} should not parse");
        }
    }

    #[test]
    fn region_contains_its_own_corners_but_not_the_far_edge() {
        let region = Region {
            x: 0,
            y: 0,
            width: 10,
            height: 10,
        };
        assert!(region.contains(0, 0));
        assert!(region.contains(9, 9));
        assert!(!region.contains(10, 10));
        assert!(!region.contains(-1, 5));
    }
}
