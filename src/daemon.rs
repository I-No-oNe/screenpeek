use std::collections::hash_map::DefaultHasher;
use std::collections::{HashMap, HashSet};
use std::fs;
use std::hash::{Hash, Hasher};
use std::io::{BufRead, BufReader, Write};
use std::net::{Ipv4Addr, SocketAddr, TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex};
use std::thread::sleep;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use anyhow::{anyhow, bail, Context, Result};
use image::RgbaImage;
use rayon::prelude::*;
use serde::{Deserialize, Serialize};

use crate::capture::{self, Capture, Region};
use crate::index::{self, Element};
use crate::read::Engine;

/// Past this much change, a full read beats stitching bands together.
const FULL_REDRAW_FRACTION: f64 = 0.55;

/// Changed bands grow by this much so glyphs are not cut in half.
const DIRTY_MARGIN: u32 = 16;

/// Bands closer than this are read as one.
const BAND_GAP: u32 = 48;

/// How long a freshly started daemon is given to load its models.
const STARTUP_TIMEOUT: Duration = Duration::from_secs(20);

/// A daemon nobody has asked anything for this long shuts itself down, so an
/// idle machine is not holding 12 MB of models and a capture buffer.
const IDLE_TIMEOUT: Duration = Duration::from_secs(10 * 60);

/// Once every session that used the daemon has exited, it waits this long for
/// a new one before shutting down.
const ORPHAN_GRACE: Duration = Duration::from_secs(60);

/// Recognition is given half the cores, rounded down, so a scan never takes
/// the machine away from whatever the user is actually doing.
fn worker_threads() -> usize {
    std::thread::available_parallelism()
        .map(|cores| (cores.get() / 2).max(2))
        .unwrap_or(2)
}

/// Bands that have been read before are remembered, so a screen that flips
/// between two states, a menu opening and closing, is read once.
const BAND_CACHE_SIZE: usize = 32;

#[derive(Debug, Serialize, Deserialize)]
pub struct Request {
    pub token: String,
    pub region: Option<Region>,
    pub monitor: Option<usize>,
    /// The process that owns the session issuing this request, so the daemon
    /// can tell when the last one has gone. Zero where it cannot be told.
    #[serde(default)]
    pub session: u32,
}

#[derive(Debug, Serialize, Deserialize)]
pub enum Response {
    Elements { elements: Vec<Element> },
    Error(String),
}

pub fn serve() -> Result<()> {
    rayon::ThreadPoolBuilder::new()
        .num_threads(worker_threads())
        .build_global()
        .ok();

    let listener = TcpListener::bind(SocketAddr::from((Ipv4Addr::LOCALHOST, 0)))
        .context("cannot listen on the loopback interface")?;
    let port = listener.local_addr()?.port();
    let token = new_token();
    write_endpoint(port, &token)?;

    let mut session = Session::new()?;
    eprintln!(
        "screenpeek: listening on 127.0.0.1:{port} ({}, {} threads)",
        session.how,
        worker_threads()
    );

    let activity = Arc::new(Mutex::new(Activity::new()));
    idle_shutdown(Arc::clone(&activity));

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
            Ok(request) => {
                activity
                    .lock()
                    .expect("activity not poisoned")
                    .record(request.session);
                match session.look(&request) {
                    Ok(elements) => Response::Elements { elements },
                    Err(error) => Response::Error(error.to_string()),
                }
            }
            Err(error) => Response::Error(error.to_string()),
        };

        let mut line = serde_json::to_vec(&response)?;
        line.push(b'\n');
        let _ = stream.write_all(&line);
    }

    Ok(())
}

/// What the daemon knows about who is still using it.
struct Activity {
    last_request: Instant,
    sessions: HashSet<u32>,
}

impl Activity {
    fn new() -> Activity {
        Activity {
            last_request: Instant::now(),
            sessions: HashSet::new(),
        }
    }

    fn record(&mut self, session: u32) {
        self.last_request = Instant::now();
        if session != 0 {
            self.sessions.insert(session);
        }
    }

    /// Why the daemon should stop, if it should.
    fn expired(&mut self) -> Option<&'static str> {
        self.sessions.retain(|session| alive(*session));

        if self.last_request.elapsed() >= IDLE_TIMEOUT {
            return Some("idle");
        }
        if self.sessions.is_empty() && self.last_request.elapsed() >= ORPHAN_GRACE {
            return Some("no session left using it");
        }
        None
    }
}

/// Ends the process once it is idle, or once every session that used it has
/// exited. An agent that stops working takes the daemon with it.
fn idle_shutdown(activity: Arc<Mutex<Activity>>) {
    std::thread::spawn(move || loop {
        sleep(Duration::from_secs(20));
        let reason = activity
            .lock()
            .ok()
            .and_then(|mut activity| activity.expired());
        if let Some(reason) = reason {
            eprintln!("screenpeek: {reason}, shutting down");
            std::process::exit(0);
        }
    });
}

#[cfg(unix)]
fn alive(pid: u32) -> bool {
    // Signal 0 asks about the process without touching it.
    unsafe extern "C" {
        fn kill(pid: i32, sig: i32) -> i32;
    }
    unsafe { kill(pid as i32, 0) == 0 }
}

#[cfg(not(unix))]
fn alive(_pid: u32) -> bool {
    true
}

struct Session {
    engine: Engine,
    how: &'static str,
    #[cfg(target_os = "linux")]
    fast: Option<capture::wayland::Screencopy>,
    previous: Option<Frame>,
    bands: HashMap<u64, Vec<Element>>,
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
            let fast = capture::wayland::Screencopy::new();
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
                bands: HashMap::new(),
            })
        }

        #[cfg(not(target_os = "linux"))]
        Ok(Session {
            engine: Engine::load()?,
            how: "portable capture",
            previous: None,
            bands: HashMap::new(),
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
                    let kept: Vec<Element> = frame
                        .elements
                        .iter()
                        .filter(|element| {
                            let x = element.x - capture.origin.0;
                            let y = element.y - capture.origin.1;
                            !bands.iter().any(|band| band.contains(x, y))
                        })
                        .cloned()
                        .collect();
                    (self.patch(kept, &capture, &bands)?, "patched")
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
        capture::screen(monitor, None)
    }
}

impl Session {
    /// Re-reads the changed bands, in parallel and skipping any band whose
    /// pixels have been read before.
    fn patch(
        &mut self,
        kept: Vec<Element>,
        capture: &Capture,
        bands: &[Region],
    ) -> Result<Vec<Element>> {
        let crops: Vec<(u64, Capture)> = bands
            .iter()
            .map(|band| {
                let image = image::imageops::crop_imm(
                    &capture.image,
                    band.x as u32,
                    band.y as u32,
                    band.width,
                    band.height,
                )
                .to_image();
                (
                    hash_image(&image),
                    Capture {
                        image,
                        origin: capture.to_desktop(band.x, band.y),
                    },
                )
            })
            .collect();

        let fresh: Vec<(u64, Vec<Element>)> = crops
            .par_iter()
            .filter(|(key, _)| !self.bands.contains_key(key))
            .map(|(key, crop)| self.engine.read(crop).map(|read| (*key, read)))
            .collect::<Result<_>>()?;

        for (key, read) in fresh {
            if self.bands.len() >= BAND_CACHE_SIZE {
                self.bands.clear();
            }
            self.bands
                .insert(key, offsets_from(&read, crop_origin(&crops, key)));
        }

        let mut elements = kept;
        for (key, crop) in &crops {
            let cached = self.bands.get(key).cloned().unwrap_or_default();
            elements.extend(cached.into_iter().map(|element| Element {
                x: element.x + crop.origin.0,
                y: element.y + crop.origin.1,
                ..element
            }));
        }

        index::number(&mut elements);
        Ok(elements)
    }
}

/// Band text is cached in band-local coordinates, so the same band matches
/// wherever it appears on screen.
fn offsets_from(elements: &[Element], origin: (i32, i32)) -> Vec<Element> {
    elements
        .iter()
        .map(|element| Element {
            x: element.x - origin.0,
            y: element.y - origin.1,
            ..element.clone()
        })
        .collect()
}

fn crop_origin(crops: &[(u64, Capture)], key: u64) -> (i32, i32) {
    crops
        .iter()
        .find(|(candidate, _)| *candidate == key)
        .map(|(_, crop)| crop.origin)
        .unwrap_or((0, 0))
}

fn hash_image(image: &RgbaImage) -> u64 {
    let mut hasher = DefaultHasher::new();
    image.dimensions().hash(&mut hasher);
    image.as_raw().hash(&mut hasher);
    hasher.finish()
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

/// Asks a running daemon to look, starting one if there is none. `None` means
/// the daemon could not be reached and the caller should do the work itself.
pub fn ask(region: Option<Region>, monitor: Option<usize>) -> Option<Vec<Element>> {
    let mut stream = match connect() {
        Some(stream) => stream,
        None => {
            start()?;
            connect()?
        }
    };
    let (_, token) = read_endpoint().ok()?;

    let request = Request {
        token,
        region,
        monitor,
        session: owning_session(),
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

/// The process that owns this command: the shell or agent that ran it. The
/// daemon watches it so it can stop when that session is gone.
#[cfg(unix)]
fn owning_session() -> u32 {
    std::os::unix::process::parent_id()
}

#[cfg(not(unix))]
fn owning_session() -> u32 {
    0
}

fn connect() -> Option<TcpStream> {
    let (port, _) = read_endpoint().ok()?;
    TcpStream::connect((Ipv4Addr::LOCALHOST, port)).ok()
}

/// Starts a daemon in the background and waits for it to answer. The first
/// scan of a session pays for this once; every later one is on the fast path.
fn start() -> Option<()> {
    if std::env::var_os("SCREENPEEK_NO_DAEMON").is_some() {
        return None;
    }

    let binary = std::env::current_exe().ok()?;
    Command::new(binary)
        .arg("serve")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;

    let deadline = Instant::now() + STARTUP_TIMEOUT;
    while Instant::now() < deadline {
        if connect().is_some() {
            return Some(());
        }
        sleep(Duration::from_millis(100));
    }
    None
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
