//! The client side: reaching the daemon, starting it, and sending it work.

use std::io::{BufRead, BufReader, Write};
use std::net::{Ipv4Addr, TcpStream};
use std::process::{Command, Stdio};
use std::thread::sleep;
use std::time::{Duration, Instant};

use anyhow::{bail, Context, Result};

use super::endpoint::{log_path, read_endpoint};
use super::{Request, Response};
use crate::capture::Region;
use crate::index::Element;
use crate::pointer::Input;
use crate::read::Language;

/// How long a freshly started daemon is given to load its models.
pub(super) const STARTUP_TIMEOUT: Duration = Duration::from_secs(20);

/// How long a client waits for an answer before reading the screen itself.
pub(super) const ANSWER_TIMEOUT: Duration = Duration::from_secs(30);

/// Input may wait on the desktop's permission dialog the first time.
pub(super) const INPUT_TIMEOUT: Duration = Duration::from_secs(130);

/// Whether commands may go through the daemon.
#[cfg(target_os = "linux")]
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
pub(super) fn owning_session() -> u32 {
    std::os::unix::process::parent_id()
}

#[cfg(not(unix))]
pub(super) fn owning_session() -> u32 {
    0
}

pub(super) fn connect() -> Option<TcpStream> {
    let (port, _) = read_endpoint().ok()?;
    TcpStream::connect((Ipv4Addr::LOCALHOST, port)).ok()
}

pub(super) fn start() -> Option<()> {
    let binary = std::env::current_exe().ok()?;
    let log = log_path()
        .and_then(|path| {
            std::fs::create_dir_all(path.parent().context("no log directory")?)?;
            Ok(std::fs::File::create(path)?)
        })
        .map_or_else(|_| Stdio::null(), Stdio::from);
    Command::new(binary)
        .arg("serve")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(log)
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
