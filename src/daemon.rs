use std::collections::hash_map::DefaultHasher;
use std::fs;
use std::hash::{Hash, Hasher};
use std::io::{BufRead, BufReader, Write};
use std::net::{Ipv4Addr, SocketAddr, TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use anyhow::{anyhow, bail, Context, Result};
use image::RgbaImage;
use serde::{Deserialize, Serialize};

use crate::index::{self, Element};
use crate::ocr::Engine;
use crate::screen::{self, Capture, Region};

/// Past this much change, a full read beats stitching bands together.
const FULL_REDRAW_FRACTION: f64 = 0.55;

/// Changed bands grow by this much so glyphs are not cut in half.
const DIRTY_MARGIN: u32 = 16;

/// Bands closer than this are read as one.
const BAND_GAP: u32 = 48;

#[derive(Debug, Serialize, Deserialize)]
pub struct Request {
    pub token: String,
    pub region: Option<Region>,
    pub monitor: Option<usize>,
}

#[derive(Debug, Serialize, Deserialize)]
pub enum Response {
    Elements { elements: Vec<Element> },
    Error(String),
}

pub fn serve() -> Result<()> {
    let listener = TcpListener::bind(SocketAddr::from((Ipv4Addr::LOCALHOST, 0)))
        .context("cannot listen on the loopback interface")?;
    let port = listener.local_addr()?.port();
    let token = new_token();
    write_endpoint(port, &token)?;

    let mut session = Session::new()?;
    eprintln!(
        "screenpeek: listening on 127.0.0.1:{port} ({})",
        session.how
    );

    for stream in listener.incoming() {
        let mut stream = match stream {
            Ok(stream) => stream,
            Err(error) => {
                eprintln!("screenpeek: dropped connection: {error}");
                continue;
            }
        };

        let response = match read_request(&mut stream) {
            Ok(request) if request.token != token => Response::Error("bad token".into()),
            Ok(request) => match session.look(&request) {
                Ok(elements) => Response::Elements { elements },
                Err(error) => Response::Error(error.to_string()),
            },
            Err(error) => Response::Error(error.to_string()),
        };

        let mut line = serde_json::to_vec(&response)?;
        line.push(b'\n');
        let _ = stream.write_all(&line);
    }

    Ok(())
}

struct Session {
    engine: Engine,
    how: &'static str,
    #[cfg(target_os = "linux")]
    fast: Option<crate::wayland::Screencopy>,
    previous: Option<Frame>,
}

/// The last full-screen look and what was read out of it.
struct Frame {
    key: u64,
    image: RgbaImage,
    elements: Vec<Element>,
}

impl Session {
    fn new() -> Result<Session> {
        #[cfg(target_os = "linux")]
        {
            let fast = crate::wayland::Screencopy::new();
            let how = if fast.is_ok() {
                "wlr-screencopy"
            } else {
                "portable capture"
            };
            Ok(Session {
                engine: Engine::load()?,
                how,
                fast: fast.ok(),
                previous: None,
            })
        }

        #[cfg(not(target_os = "linux"))]
        Ok(Session {
            engine: Engine::load()?,
            how: "portable capture",
            previous: None,
        })
    }

    fn look(&mut self, request: &Request) -> Result<Vec<Element>> {
        let started = Instant::now();
        let capture = self.capture(request.monitor)?;
        let captured = started.elapsed();

        let key = context_key(request.monitor, capture.origin);
        let reusable = self
            .previous
            .as_ref()
            .filter(|frame| frame.key == key)
            .filter(|frame| frame.image.dimensions() == capture.image.dimensions());

        let recognition = Instant::now();
        let (elements, what) = match reusable {
            None => (self.engine.read(&capture)?, "full"),
            Some(frame) => {
                let bands = dirty_bands(&frame.image, &capture.image);
                if bands.is_empty() {
                    (frame.elements.clone(), "unchanged")
                } else if worth_patching(&bands, &capture.image) {
                    (patch(&self.engine, frame, &capture, &bands)?, "patched")
                } else {
                    (self.engine.read(&capture)?, "full")
                }
            }
        };

        eprintln!(
            "{what}: {} elements, capture {}ms, read {}ms",
            elements.len(),
            captured.as_millis(),
            recognition.elapsed().as_millis()
        );

        self.previous = Some(Frame {
            key,
            image: capture.image,
            elements: elements.clone(),
        });

        Ok(match request.region {
            Some(region) => elements
                .into_iter()
                .filter(|element| region.contains(element.x, element.y))
                .collect(),
            None => elements,
        })
    }

    fn capture(&mut self, monitor: Option<usize>) -> Result<Capture> {
        #[cfg(target_os = "linux")]
        if let Some(fast) = self.fast.as_mut() {
            match fast.capture(monitor) {
                Ok(capture) => return Ok(capture),
                Err(error) => {
                    eprintln!("screenpeek: screencopy failed, falling back: {error}");
                    self.fast = None;
                }
            }
        }
        screen::capture(monitor, None)
    }
}

/// Re-reads only the changed bands and keeps the elements outside them.
fn patch(
    engine: &Engine,
    frame: &Frame,
    capture: &Capture,
    bands: &[Region],
) -> Result<Vec<Element>> {
    let local = |element: &Element| (element.x - capture.origin.0, element.y - capture.origin.1);

    let mut elements: Vec<Element> = frame
        .elements
        .iter()
        .filter(|element| {
            let (x, y) = local(element);
            !bands.iter().any(|band| band.contains(x, y))
        })
        .cloned()
        .collect();

    for band in bands {
        let cropped = Capture {
            image: image::imageops::crop_imm(
                &capture.image,
                band.x as u32,
                band.y as u32,
                band.width,
                band.height,
            )
            .to_image(),
            origin: capture.to_desktop(band.x, band.y),
        };
        elements.extend(engine.read(&cropped)?);
    }

    index::number(&mut elements);
    Ok(elements)
}

/// The horizontal bands that differ between two frames, top to bottom.
fn dirty_bands(before: &RgbaImage, after: &RgbaImage) -> Vec<Region> {
    let width = before.width();
    let height = before.height();
    let stride = width as usize * 4;
    let (before, after) = (before.as_raw(), after.as_raw());

    let changed = |row: usize| before[row * stride..][..stride] != after[row * stride..][..stride];

    let mut bands: Vec<(u32, u32)> = Vec::new();
    for row in 0..height as usize {
        if !changed(row) {
            continue;
        }
        let row = row as u32;
        match bands.last_mut() {
            Some((_, end)) if row - *end <= BAND_GAP => *end = row,
            _ => bands.push((row, row)),
        }
    }

    bands
        .into_iter()
        .map(|(start, end)| {
            let top = start.saturating_sub(DIRTY_MARGIN);
            let bottom = (end + DIRTY_MARGIN + 1).min(height);
            Region {
                x: 0,
                y: top as i32,
                width,
                height: bottom - top,
            }
        })
        .collect()
}

fn worth_patching(bands: &[Region], image: &RgbaImage) -> bool {
    let changed: u32 = bands.iter().map(|band| band.height).sum();
    f64::from(changed) / f64::from(image.height()) < FULL_REDRAW_FRACTION
}

fn context_key(monitor: Option<usize>, origin: (i32, i32)) -> u64 {
    let mut hasher = DefaultHasher::new();
    monitor.hash(&mut hasher);
    origin.hash(&mut hasher);
    hasher.finish()
}

/// Asks a running daemon to look. `None` means no daemon, not an error.
pub fn ask(region: Option<Region>, monitor: Option<usize>) -> Option<Vec<Element>> {
    let (port, token) = read_endpoint().ok()?;
    let mut stream = TcpStream::connect((Ipv4Addr::LOCALHOST, port)).ok()?;

    let request = Request {
        token,
        region,
        monitor,
    };
    let mut line = serde_json::to_vec(&request).ok()?;
    line.push(b'\n');
    stream.write_all(&line).ok()?;

    let mut reply = String::new();
    BufReader::new(&stream).read_line(&mut reply).ok()?;
    match serde_json::from_str(&reply).ok()? {
        Response::Elements { elements } => Some(elements),
        Response::Error(error) => {
            eprintln!("screenpeek: the daemon refused: {error}");
            None
        }
    }
}

pub fn endpoint_summary() -> Result<String> {
    let (port, _) = read_endpoint().context("no daemon is running")?;
    if TcpStream::connect((Ipv4Addr::LOCALHOST, port)).is_err() {
        bail!("no daemon is running; the last one used port {port}");
    }
    Ok(format!("a daemon is listening on 127.0.0.1:{port}"))
}

fn read_request(stream: &mut TcpStream) -> Result<Request> {
    let mut line = String::new();
    BufReader::new(stream)
        .read_line(&mut line)
        .context("cannot read the request")?;
    serde_json::from_str(&line).context("cannot parse the request")
}

fn endpoint_path() -> Result<PathBuf> {
    Ok(dirs::cache_dir()
        .ok_or_else(|| anyhow!("no cache directory on this system"))?
        .join("screenpeek")
        .join("daemon"))
}

fn write_endpoint(port: u16, token: &str) -> Result<()> {
    let path = endpoint_path()?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(&path, format!("{port} {token}"))
        .with_context(|| format!("cannot write {}", path.display()))?;
    owner_only(&path)
}

#[cfg(unix)]
fn owner_only(path: &Path) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(path, fs::Permissions::from_mode(0o600))
        .with_context(|| format!("cannot restrict {}", path.display()))
}

#[cfg(not(unix))]
fn owner_only(_path: &Path) -> Result<()> {
    Ok(())
}

fn read_endpoint() -> Result<(u16, String)> {
    let raw = fs::read_to_string(endpoint_path()?)?;
    let (port, token) = raw
        .split_once(' ')
        .ok_or_else(|| anyhow!("the daemon file is malformed"))?;
    Ok((port.parse()?, token.to_owned()))
}

fn new_token() -> String {
    let mut hasher = DefaultHasher::new();
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|age| age.as_nanos())
        .unwrap_or(0)
        .hash(&mut hasher);
    std::process::id().hash(&mut hasher);
    format!("{:016x}", hasher.finish())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame(width: u32, height: u32, fill: u8) -> RgbaImage {
        RgbaImage::from_pixel(width, height, image::Rgba([fill, fill, fill, 255]))
    }

    fn paint_row(image: &mut RgbaImage, y: u32) {
        for x in 0..image.width() {
            image.put_pixel(x, y, image::Rgba([255, 255, 255, 255]));
        }
    }

    #[test]
    fn identical_frames_have_no_bands() {
        assert!(dirty_bands(&frame(64, 64, 0), &frame(64, 64, 0)).is_empty());
    }

    #[test]
    fn a_changed_row_becomes_one_band_with_a_margin() {
        let before = frame(64, 400, 0);
        let mut after = before.clone();
        paint_row(&mut after, 200);

        let bands = dirty_bands(&before, &after);
        assert_eq!(bands.len(), 1);
        assert_eq!(bands[0].y, 200 - DIRTY_MARGIN as i32);
        assert_eq!(bands[0].height, 2 * DIRTY_MARGIN + 1);
        assert_eq!(bands[0].width, 64);
    }

    #[test]
    fn distant_changes_stay_in_separate_bands() {
        let before = frame(64, 600, 0);
        let mut after = before.clone();
        paint_row(&mut after, 20);
        paint_row(&mut after, 500);

        let bands = dirty_bands(&before, &after);
        assert_eq!(bands.len(), 2, "{bands:?}");
        assert!(bands[0].y < bands[1].y);
    }

    #[test]
    fn nearby_changes_merge_into_one_band() {
        let before = frame(64, 600, 0);
        let mut after = before.clone();
        paint_row(&mut after, 100);
        paint_row(&mut after, 100 + BAND_GAP - 1);

        assert_eq!(dirty_bands(&before, &after).len(), 1);
    }

    #[test]
    fn patching_is_abandoned_once_most_of_the_screen_changed() {
        let image = frame(100, 100, 0);
        let band = |height| Region {
            x: 0,
            y: 0,
            width: 100,
            height,
        };
        assert!(worth_patching(&[band(10)], &image));
        assert!(!worth_patching(&[band(30), band(30), band(30)], &image));
    }
}
