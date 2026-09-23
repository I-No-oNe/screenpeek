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
use crate::pointer::Input;
use crate::read::{Engine, Language, Placement};

mod diff;
use diff::{dirty_areas, merge_bands, worth_patching};

/// How long a freshly started daemon is given to load its models.
const STARTUP_TIMEOUT: Duration = Duration::from_secs(20);

/// How long a connected client may take to send its request.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(5);

/// How long a client waits for an answer before reading the screen itself.
const ANSWER_TIMEOUT: Duration = Duration::from_secs(30);

/// Input may wait on the desktop's permission dialog the first time.
const INPUT_TIMEOUT: Duration = Duration::from_secs(130);

/// How old input must be before a frame is trusted to show it.
const SETTLE: Duration = Duration::from_millis(120);

const IDLE_TIMEOUT: Duration = Duration::from_secs(10 * 60);

/// Allow this grace period after the last client session exits.
const ORPHAN_GRACE: Duration = Duration::from_secs(60);

/// Reserve half the cores for other applications.
fn worker_threads() -> usize {
    std::thread::available_parallelism()
        .map(|cores| (cores.get() / 2).max(2))
        .unwrap_or(2)
}

const BAND_CACHE_SIZE: usize = 32;

#[derive(Debug, Default, Serialize, Deserialize)]
pub struct Request {
    pub token: String,
    pub region: Option<Region>,
    pub monitor: Option<usize>,
    /// Parent process of the client; the daemon exits once all are gone.
    #[serde(default)]
    pub session: u32,
    #[serde(default)]
    pub lang: Option<Language>,
    #[serde(default)]
    pub excluded: Vec<Region>,
    /// Input to perform instead of reading the screen.
    #[serde(default)]
    pub input: Option<Input>,
}

#[derive(Debug, Serialize, Deserialize)]
pub enum Response {
    Elements { elements: Vec<Element> },
    Error(String),
}

pub fn serve() -> Result<()> {
    // Avoid competing daemons overwriting the per-user endpoint.
    if connect().is_some() {
        if let Ok(summary) = endpoint_summary() {
            eprintln!("screenpeek: {summary}, leaving it to serve");
        }
        return Ok(());
    }

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
    idle_shutdown(Arc::clone(&activity), port);

    for stream in listener.incoming() {
        let mut stream = match stream {
            Ok(stream) => stream,
            Err(error) => {
                eprintln!("screenpeek: dropped connection: {error}");
                continue;
            }
        };

        // A client that connects and says nothing must not stall the others.
        let _ = stream.set_read_timeout(Some(REQUEST_TIMEOUT));
        let response = match read_request(&mut stream) {
            Ok(request) if request.token != token => Response::Error("bad token".into()),
            Ok(mut request) => {
                activity
                    .lock()
                    .expect("activity not poisoned")
                    .record(request.session);
                let looked = match request.input.take() {
                    Some(input) => session.act(input).map(|()| Vec::new()),
                    None => session.look(&request),
                };
                match looked {
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

/// Remove the endpoint only if it still belongs to this daemon.
fn release_endpoint(port: u16) {
    if matches!(read_endpoint(), Ok((listed, _)) if listed == port) {
        let _ = endpoint_path().map(fs::remove_file);
    }
}

fn idle_shutdown(activity: Arc<Mutex<Activity>>, port: u16) {
    std::thread::spawn(move || loop {
        sleep(Duration::from_secs(20));
        let reason = activity
            .lock()
            .ok()
            .and_then(|mut activity| activity.expired());
        if let Some(reason) = reason {
            eprintln!("screenpeek: {reason}, shutting down");
            release_endpoint(port);
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
    capturer: capture::Backend,
    previous: Option<Frame>,
    bands: HashMap<u64, Vec<Element>>,
    tree: TreeCache,
    /// Whether each window's tree accounts for the text in its pixels.
    covered: HashMap<WindowKey, bool>,
    /// The portal session input and frames share, on KDE and GNOME.
    #[cfg(target_os = "linux")]
    desk: Option<crate::pointer::Desk>,
    /// The session being opened, which can wait on a permission dialog.
    #[cfg(target_os = "linux")]
    opening: Option<std::thread::JoinHandle<Result<crate::pointer::Desk>>>,
    #[cfg(target_os = "linux")]
    frames: Option<capture::frames::Frames>,
    /// Once the screen stream fails, screenshots are used instead.
    #[cfg(target_os = "linux")]
    frames_failed: bool,
    /// When input was last performed, so frames can wait for it to show.
    last_input: Option<Instant>,
}

/// A window as remembered between looks: title and size.
type WindowKey = (String, u32, u32);
#[cfg(target_os = "linux")]
type TreeCache = HashMap<WindowKey, Vec<crate::read::atspi::Item>>;
#[cfg(not(target_os = "linux"))]
type TreeCache = HashMap<WindowKey, ()>;

struct Frame {
    key: u64,
    image: RgbaImage,
    pixels: Vec<Element>,
    elements: Vec<Element>,
    lang: Option<Language>,
}

/// Minimum OCR text coverage required to trust a window’s accessibility tree.
const COVERAGE_THRESHOLD: f32 = 0.9;

struct Located {
    rect: Region,
    elements: Vec<Element>,
    key: Option<WindowKey>,
    /// Skip OCR only after the tree has passed the coverage check.
    verified: bool,
}

fn merge(pixels: Vec<Element>, located: Vec<Located>) -> Vec<Element> {
    index::merge_tree(
        pixels,
        located
            .into_iter()
            .flat_map(|window| window.elements)
            .collect(),
    )
}

/// Keep changed areas unless a verified tree fully covers them.
fn outside(changed: &[Region], located: &[Located], origin: (i32, i32)) -> Vec<Region> {
    changed
        .iter()
        .filter(|area| {
            let x = area.x + origin.0;
            let y = area.y + origin.1;
            !located
                .iter()
                .filter(|window| window.verified)
                .any(|window| {
                    x >= window.rect.x
                        && y >= window.rect.y
                        && x + area.width as i32 <= window.rect.x + window.rect.width as i32
                        && y + area.height as i32 <= window.rect.y + window.rect.height as i32
                })
        })
        .copied()
        .collect()
}

impl Session {
    fn new() -> Result<Session> {
        #[cfg(target_os = "linux")]
        {
            let capturer = capture::Backend::new()?;
            let how = capturer.name();

            Ok(Session {
                engine: Engine::load()?,
                how,
                capturer,
                previous: None,
                bands: HashMap::new(),
                tree: HashMap::new(),
                covered: HashMap::new(),
                desk: None,
                opening: None,
                frames: None,
                frames_failed: false,
                last_input: None,
            })
        }

        #[cfg(not(target_os = "linux"))]
        Ok(Session {
            engine: Engine::load()?,
            how: "portable capture",
            previous: None,
            bands: HashMap::new(),
            tree: HashMap::new(),
            covered: HashMap::new(),
            last_input: None,
        })
    }

    fn act(&mut self, input: Input) -> Result<()> {
        self.last_input = Some(Instant::now());
        #[cfg(target_os = "linux")]
        {
            self.open_desk(true)?;
            let desk = self.desk.take().expect("desk opened");
            let (done, desk) = desk.perform(input);
            self.desk = desk;
            if done.is_err() {
                // The session may have been revoked; the next command opens a new one.
                self.desk = None;
                self.frames = None;
            }
            done
        }
        #[cfg(not(target_os = "linux"))]
        crate::pointer::Pointer::new()?.perform(input)
    }

    fn look(&mut self, request: &Request) -> Result<Vec<Element>> {
        let started = Instant::now();
        let capture = self.capture(request.monitor, request.region)?;
        self.read_capture(request, capture, started.elapsed())
    }

    fn read_capture(
        &mut self,
        request: &Request,
        mut capture: Capture,
        captured: Duration,
    ) -> Result<Vec<Element>> {
        capture.exclude(&request.excluded);
        let key = context_key(request.monitor, capture.origin, &request.excluded);
        let comparable = self
            .previous
            .as_ref()
            .filter(|frame| frame.key == key)
            .filter(|frame| frame.lang == request.lang)
            .filter(|frame| frame.image.dimensions() == capture.image.dimensions())
            .is_some();

        let changed = match (comparable, self.previous.as_ref()) {
            (true, Some(frame)) => dirty_areas(&frame.image, &capture.image),
            _ => Vec::new(),
        };
        if comparable && changed.is_empty() {
            let elements = self.previous.as_ref().unwrap().elements.clone();
            eprintln!(
                "unchanged: {} elements, capture {}ms, read 0ms",
                elements.len(),
                captured.as_millis()
            );
            return Ok(elements);
        }
        if !comparable {
            self.tree.clear();
        }
        let placements = crate::read::placements();
        let edges = crate::read::edges(&placements);
        let recognition = Instant::now();

        let overlap = !comparable;
        let (located, read_ahead) = if overlap {
            let mut tree = std::mem::take(&mut self.tree);
            let (covered, engine) = (&self.covered, &self.engine);
            let (pixels, located) = std::thread::scope(|scope| {
                let walker = scope.spawn(|| {
                    located_windows(&mut tree, covered, &placements, &changed, capture.origin)
                });
                let pixels = engine.read_within(&capture, &edges, request.lang.as_ref());
                (pixels, walker.join().unwrap_or_default())
            });
            self.tree = tree;
            (located, Some(pixels?))
        } else {
            (
                located_windows(
                    &mut self.tree,
                    &self.covered,
                    &placements,
                    &changed,
                    capture.origin,
                ),
                None,
            )
        };

        // Patch Tesseract reads too, since its cost scales with image area.
        let language = request.lang.as_ref();
        let (pixels, what) = if let Some(pixels) = read_ahead {
            (pixels, "full")
        } else {
            match comparable.then_some(self.previous.as_ref()).flatten() {
                None => (self.read_all(&capture, &edges, language)?, "full"),
                Some(frame) => {
                    // Merge bands to avoid repeated detector startup costs.
                    let unread: Vec<Region> =
                        merge_bands(&outside(&changed, &located, capture.origin))
                            .into_iter()
                            .collect();

                    if unread.is_empty() {
                        (frame.pixels.clone(), "window only")
                    } else if worth_patching(&unread, &capture.image) {
                        let kept: Vec<Element> = frame
                            .pixels
                            .iter()
                            .filter(|element| {
                                let x = element.x - capture.origin.0;
                                let y = element.y - capture.origin.1;
                                !unread.iter().any(|area| area.contains(x, y))
                            })
                            .cloned()
                            .collect();
                        (
                            self.patch(kept, &capture, &unread, &edges, language)?,
                            "patched",
                        )
                    } else {
                        (self.read_all(&capture, &edges, language)?, "full")
                    }
                }
            }
        };

        // Compare tree coverage during full reads, when both results are available.
        if what == "full" {
            verify_coverage(&mut self.covered, &located, &pixels);
        }

        let (verified, windows) = (
            located.iter().filter(|window| window.verified).count(),
            located.len(),
        );

        let bounds = Region {
            x: capture.origin.0,
            y: capture.origin.1,
            width: capture.image.width(),
            height: capture.image.height(),
        };
        let mut elements: Vec<_> = merge(pixels.clone(), located)
            .into_iter()
            .filter(|element| bounds.contains(element.x, element.y))
            .collect();
        crate::caller::filter(&mut elements, &request.excluded);

        eprintln!(
            "{what}: {} elements, {verified}/{windows} window(s) verified, capture {}ms, read {}ms",
            elements.len(),
            captured.as_millis(),
            recognition.elapsed().as_millis()
        );

        self.previous = Some(Frame {
            key,
            image: capture.image,
            pixels,
            elements: elements.clone(),
            lang: request.lang.clone(),
        });

        Ok(elements)
    }
}

#[cfg(target_os = "linux")]
fn located_windows(
    tree: &mut TreeCache,
    covered: &HashMap<WindowKey, bool>,
    placements: &[Placement],
    changed: &[Region],
    origin: (i32, i32),
) -> Vec<Located> {
    if placements.is_empty() {
        return Vec::new();
    }

    let touched = |window: &crate::read::atspi::Window| {
        let Some(placement) =
            crate::read::fuse::placement_index(window, placements).map(|index| &placements[index])
        else {
            return true;
        };

        changed.iter().any(|area| {
            let left = area.x + origin.0;
            let top = area.y + origin.1;
            left < placement.x + placement.width as i32
                && left + area.width as i32 > placement.x
                && top < placement.y + placement.height as i32
                && top + area.height as i32 > placement.y
        })
    };

    let known = &*tree;
    let Ok(mut windows) = crate::read::atspi::windows_where(|window| {
        !known.contains_key(&(window.title.clone(), window.width, window.height)) || touched(window)
    }) else {
        return Vec::new();
    };

    for window in &mut windows {
        let key = (window.title.clone(), window.width, window.height);
        if window.items.is_empty() {
            if let Some(remembered) = tree.get(&key) {
                window.items = remembered.clone();
            }
        } else {
            tree.insert(key, window.items.clone());
        }
    }

    crate::read::fuse::place(&windows, placements)
        .into_iter()
        .map(|placed| {
            let key = placed.key;
            let verified = key
                .as_ref()
                .and_then(|key| covered.get(key))
                .copied()
                .unwrap_or(false);
            Located {
                rect: placed.rect,
                elements: placed.elements,
                key,
                verified,
            }
        })
        .collect()
}

#[cfg(not(target_os = "linux"))]
fn located_windows(
    _tree: &mut TreeCache,
    _covered: &HashMap<WindowKey, bool>,
    _placements: &[Placement],
    _changed: &[Region],
    _origin: (i32, i32),
) -> Vec<Located> {
    Vec::new()
}

fn verify_coverage(
    covered: &mut HashMap<WindowKey, bool>,
    located: &[Located],
    recognized: &[Element],
) {
    for window in located {
        let Some(key) = window.key.clone() else {
            continue;
        };
        let inside: Vec<&Element> = recognized
            .iter()
            .filter(|element| window.rect.contains(element.x, element.y))
            .collect();
        // A blank window proves nothing, so it stays unverified.
        if inside.is_empty() {
            covered.insert(key, false);
            continue;
        }
        let claimed: Vec<&str> = window
            .elements
            .iter()
            .map(|element| element.text.as_str())
            .collect();
        let accounted = inside
            .iter()
            .filter(|element| {
                claimed
                    .iter()
                    .any(|text| text.contains(element.text.as_str()))
            })
            .count();
        let coverage = accounted as f32 / inside.len() as f32;
        covered.insert(key, coverage >= COVERAGE_THRESHOLD);
    }
}

impl Session {
    fn capture(&mut self, monitor: Option<usize>, region: Option<Region>) -> Result<Capture> {
        #[cfg(target_os = "linux")]
        {
            let portal = matches!(self.capturer, capture::Backend::Portal);
            if portal && monitor.is_none() && !self.frames_failed {
                match self.frame() {
                    Ok(Some(full)) => {
                        return match region {
                            Some(region) => capture::crop(full, region),
                            None => Ok(full),
                        }
                    }
                    // Screenshots meanwhile, while the permission dialog is open.
                    Ok(None) => {}
                    Err(error) => {
                        eprintln!("screenpeek: no screen stream, using screenshots: {error:#}");
                        self.frames = None;
                        self.frames_failed = true;
                    }
                }
            }
            self.capturer.capture(monitor, region)
        }
        #[cfg(not(target_os = "linux"))]
        {
            capture::screen(monitor, region)
        }
    }
}

impl Session {
    fn read_all(
        &self,
        capture: &Capture,
        edges: &[i32],
        language: Option<&Language>,
    ) -> Result<Vec<Element>> {
        self.engine.read_within(capture, edges, language)
    }

    /// Re-read changed bands in parallel, skipping bands already cached.
    fn patch(
        &mut self,
        kept: Vec<Element>,
        capture: &Capture,
        bands: &[Region],
        edges: &[i32],
        language: Option<&Language>,
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
                    band_key(&image, language),
                    Capture {
                        image,
                        origin: capture.to_desktop(band.x, band.y),
                    },
                )
            })
            .collect();

        // Evict before reading, so this request never loses a band mid-assembly.
        if self.bands.len() + crops.len() > BAND_CACHE_SIZE {
            self.bands.clear();
        }
        let fresh: Vec<(u64, Vec<Element>)> = crops
            .par_iter()
            .filter(|(key, _)| !self.bands.contains_key(key))
            .map(|(key, crop)| {
                self.read_all(crop, edges, language)
                    .map(|read| (*key, offsets_from(&read, crop.origin)))
            })
            .collect::<Result<_>>()?;

        for (key, read) in fresh {
            self.bands.insert(key, read);
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

/// Band-local coordinates, so a band matches wherever it moves.
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

/// Key band text by pixels and language.
fn band_key(image: &RgbaImage, language: Option<&Language>) -> u64 {
    let mut hasher = DefaultHasher::new();
    hash_image(image).hash(&mut hasher);
    language
        .map(|language| (&language.codes, language.auto))
        .hash(&mut hasher);
    hasher.finish()
}

fn hash_image(image: &RgbaImage) -> u64 {
    let mut hasher = DefaultHasher::new();
    image.dimensions().hash(&mut hasher);
    image.as_raw().hash(&mut hasher);
    hasher.finish()
}

fn context_key(monitor: Option<usize>, origin: (i32, i32), excluded: &[Region]) -> u64 {
    let mut hasher = DefaultHasher::new();
    monitor.hash(&mut hasher);
    origin.hash(&mut hasher);
    excluded.hash(&mut hasher);
    hasher.finish()
}

#[cfg(target_os = "linux")]
impl Session {
    /// Open the portal session in the background; with `wait`, until it is open.
    fn open_desk(&mut self, wait: bool) -> Result<bool> {
        if self.desk.is_some() {
            return Ok(true);
        }
        let opening = self
            .opening
            .get_or_insert_with(|| std::thread::spawn(crate::pointer::Desk::open));
        if !wait && !opening.is_finished() {
            return Ok(false);
        }
        let opened = self.opening.take().expect("opening").join();
        self.desk = Some(opened.map_err(|_| anyhow!("opening the portal session panicked"))??);
        Ok(true)
    }

    /// The latest frame of the portal's screen stream, or none while it opens.
    fn frame(&mut self) -> Result<Option<Capture>> {
        if self.frames.is_none() {
            if !self.open_desk(false)? {
                return Ok(None);
            }
            let (remote, screens) = self.desk.as_ref().expect("desk opened").screens()?;
            self.frames = Some(capture::frames::Frames::start(remote, screens)?);
        }
        let settle = self
            .last_input
            .map_or(Duration::ZERO, |at| SETTLE.saturating_sub(at.elapsed()));
        self.frames
            .as_mut()
            .expect("frames started")
            .capture(settle)
            .map(Some)
    }
}

/// Whether commands may go through the daemon.
pub fn available() -> bool {
    std::env::var_os("SCREENPEEK_NO_DAEMON").is_none()
}

/// Have the daemon perform input with its portal session.
pub fn act(input: Input) -> Result<()> {
    let mut stream = match connect() {
        Some(stream) => stream,
        None => {
            start().context("cannot start the daemon for input")?;
            connect().context("cannot reach the daemon for input")?
        }
    };
    let (_, token) = read_endpoint()?;
    let request = Request {
        token,
        session: owning_session(),
        input: Some(input),
        ..Default::default()
    };
    let mut line = serde_json::to_vec(&request)?;
    line.push(b'\n');
    stream.set_read_timeout(Some(INPUT_TIMEOUT))?;
    stream.write_all(&line)?;
    let mut reply = String::new();
    BufReader::new(&stream).read_line(&mut reply)?;
    match serde_json::from_str(&reply).context("the daemon did not answer the input")? {
        Response::Elements { .. } => Ok(()),
        Response::Error(error) => bail!(error),
    }
}

/// Ask the daemon, starting it if needed; `None` means read directly.
pub fn ask(
    region: Option<Region>,
    monitor: Option<usize>,
    lang: Option<Language>,
    excluded: Vec<Region>,
) -> Option<Vec<Element>> {
    if std::env::var_os("SCREENPEEK_NO_DAEMON").is_some() {
        return None;
    }
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
        lang,
        excluded,
        input: None,
    };
    let mut line = serde_json::to_vec(&request).ok()?;
    line.push(b'\n');
    stream.set_read_timeout(Some(ANSWER_TIMEOUT)).ok()?;
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

fn start() -> Option<()> {
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
        .join("daemon-v3"))
}

fn write_endpoint(port: u16, token: &str) -> Result<()> {
    let path = endpoint_path()?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let partial = path.with_extension("partial");
    fs::write(&partial, format!("{port} {token}"))
        .with_context(|| format!("cannot write {}", partial.display()))?;
    owner_only(&partial)?;
    fs::rename(&partial, &path).with_context(|| format!("cannot write {}", path.display()))
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
mod tests;
