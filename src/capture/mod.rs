//! Screen capture in virtual-desktop coordinates, the space the pointer uses.

#[cfg(target_os = "linux")]
pub mod portal;
#[cfg(target_os = "linux")]
pub mod wayland;
#[cfg(target_os = "linux")]
pub mod x11;

use std::fmt;
use std::str::FromStr;

use anyhow::{anyhow, bail, Context, Result};
use image::RgbaImage;
use serde::{Deserialize, Serialize};

#[cfg(not(target_os = "linux"))]
use xcap::Monitor;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
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
    pub fn from_image(image: RgbaImage) -> Capture {
        Capture {
            image,
            origin: (0, 0),
        }
    }

    /// Black out excluded windows before comparing frames or recognizing text.
    pub fn exclude(&mut self, regions: &[Region]) {
        let (width, height) = self.image.dimensions();
        let clamp = |value: i64, limit: u32| value.clamp(0, i64::from(limit)) as usize;
        for region in regions {
            let left = i64::from(region.x) - i64::from(self.origin.0);
            let top = i64::from(region.y) - i64::from(self.origin.1);
            let (x, right) = (
                clamp(left, width),
                clamp(left + i64::from(region.width), width),
            );
            let (y, bottom) = (
                clamp(top, height),
                clamp(top + i64::from(region.height), height),
            );
            for row in self
                .image
                .chunks_mut(width as usize * 4)
                .take(bottom)
                .skip(y)
            {
                for pixel in row[x * 4..right * 4].chunks_mut(4) {
                    pixel.copy_from_slice(&[0, 0, 0, 255]);
                }
            }
        }
    }

    pub fn to_desktop(&self, x: i32, y: i32) -> (i32, i32) {
        (self.origin.0 + x, self.origin.1 + y)
    }
}

/// Captures a monitor, or the part of one covered by `region`. A region is
/// taken from the monitor it starts on, so scan coordinates can be fed back in.
pub fn screen(monitor: Option<usize>, region: Option<Region>) -> Result<Capture> {
    let full = full_screen(monitor, region)?;
    let Some(region) = region else {
        return Ok(full);
    };
    crop(full, region)
}

pub(crate) fn crop(capture: Capture, region: Region) -> Result<Capture> {
    let left = i64::from(region.x).max(i64::from(capture.origin.0));
    let top = i64::from(region.y).max(i64::from(capture.origin.1));
    let right = (i64::from(region.x) + i64::from(region.width))
        .min(i64::from(capture.origin.0) + i64::from(capture.image.width()));
    let bottom = (i64::from(region.y) + i64::from(region.height))
        .min(i64::from(capture.origin.1) + i64::from(capture.image.height()));
    if right <= left || bottom <= top {
        bail!("region does not intersect the captured monitor");
    }
    let x = (left - i64::from(capture.origin.0)) as u32;
    let y = (top - i64::from(capture.origin.1)) as u32;
    let width = (right - left) as u32;
    let height = (bottom - top) as u32;
    if x == 0 && y == 0 && (width, height) == capture.image.dimensions() {
        return Ok(capture);
    }
    Ok(Capture {
        image: image::imageops::crop_imm(&capture.image, x, y, width, height).to_image(),
        origin: (left as i32, top as i32),
    })
}

#[cfg(target_os = "linux")]
pub enum Backend {
    Wayland(wayland::Screencopy),
    X11(x11::Screen),
    Portal,
}

#[cfg(target_os = "linux")]
impl Backend {
    pub fn new() -> Result<Self> {
        if std::env::var("SCREENPEEK_CAPTURE").as_deref() == Ok("portal") {
            return Ok(Self::Portal);
        }
        match wayland::Screencopy::new() {
            Ok(screencopy) => Ok(Self::Wayland(screencopy)),
            Err(_) if wayland::logical_desktop().is_some() => Ok(Self::Portal),
            Err(wayland_error) => x11::Screen::new()
                .map(Self::X11)
                .map_err(|error| anyhow!("{wayland_error}; and {error}")),
        }
    }

    pub fn name(&self) -> &'static str {
        match self {
            Self::Wayland(_) => "wlr-screencopy",
            Self::X11(_) => "x11",
            Self::Portal => "portal",
        }
    }

    pub fn capture(&mut self, monitor: Option<usize>, region: Option<Region>) -> Result<Capture> {
        let full = match self {
            Self::Wayland(screencopy) => match region {
                Some(region) => screencopy.capture_containing(region),
                None => screencopy.capture(monitor),
            },
            Self::X11(screen) => match region {
                Some(region) => screen.capture_containing(region),
                None => screen.capture(monitor),
            },
            Self::Portal => {
                if monitor.is_some() {
                    bail!(
                        "portal capture uses the whole desktop; use --region instead of --monitor"
                    );
                }
                portal::Portal::new()?.capture()
            }
        }?;
        match region {
            Some(region) => crop(full, region),
            None => Ok(full),
        }
    }
}

#[cfg(target_os = "linux")]
fn full_screen(monitor: Option<usize>, region: Option<Region>) -> Result<Capture> {
    Backend::new()?.capture(monitor, region)
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
    #[test]
    fn crops_are_intersections_in_desktop_coordinates() {
        let capture = || Capture {
            image: RgbaImage::new(100, 80),
            origin: (-50, 20),
        };
        let clipped = crop(
            capture(),
            Region {
                x: -60,
                y: 10,
                width: 40,
                height: 40,
            },
        )
        .unwrap();
        assert_eq!(clipped.origin, (-50, 20));
        assert_eq!(clipped.image.dimensions(), (30, 30));
        let clipped = crop(
            capture(),
            Region {
                x: 30,
                y: 90,
                width: 50,
                height: 50,
            },
        )
        .unwrap();
        assert_eq!(clipped.origin, (30, 90));
        assert_eq!(clipped.image.dimensions(), (20, 10));
        assert!(crop(
            capture(),
            Region {
                x: 50,
                y: 20,
                width: 10,
                height: 10
            }
        )
        .is_err());
    }
    #[test]
    fn exclusion_masks_only_the_intersection_on_a_negative_origin_monitor() {
        let mut capture = Capture {
            image: RgbaImage::from_pixel(4, 4, image::Rgba([255; 4])),
            origin: (-10, -10),
        };
        capture.exclude(&[Region {
            x: -12,
            y: -12,
            width: 4,
            height: 4,
        }]);
        assert_eq!(capture.image.get_pixel(0, 0).0, [0, 0, 0, 255]);
        assert_eq!(capture.image.get_pixel(1, 1).0, [0, 0, 0, 255]);
        assert_eq!(capture.image.get_pixel(2, 2).0, [255; 4]);
    }
}
