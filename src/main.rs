//! screenpeek: read the screen as numbered text, then click it by name.

mod caller;
mod capture;
mod daemon;
mod index;
mod mcp;
mod pointer;
#[cfg(target_os = "linux")]
mod portal;
mod read;

use std::path::PathBuf;

use anyhow::{Context, Result};
use clap::{Args, Parser, Subcommand};

use capture::Region;
use index::{Element, Snapshot};
use pointer::{Button, Pointer};

#[derive(Parser)]
#[command(version, about, long_about = None)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// List the text on screen, one element per line, as `id text @x,y`
    Scan {
        /// Only show elements containing this text
        #[arg(long, value_name = "TEXT")]
        grep: Option<String>,

        #[command(flatten)]
        area: Area,

        /// Print elements as JSON, including their size
        #[arg(long)]
        json: bool,
    },

    /// Click an element, by id from the last scan or by its text
    Click {
        target: String,

        #[arg(long, default_value = "left")]
        button: Button,

        /// Double click
        #[arg(long)]
        double: bool,

        /// Scan again instead of using the last scan
        #[arg(long)]
        fresh: bool,

        /// Warn when nothing near the target changes after the click
        #[arg(long)]
        check: bool,

        #[command(flatten)]
        area: Area,
    },

    /// Wait until an element appears, or disappears with --gone
    Wait {
        target: String,

        /// Give up after this many seconds
        #[arg(long, default_value_t = 10.0)]
        timeout: f64,

        /// Wait for the element to disappear instead
        #[arg(long)]
        gone: bool,

        #[command(flatten)]
        area: Area,
    },

    /// Scroll under the pointer, or over an element with --at
    Scroll {
        #[arg(value_parser = ["up", "down", "left", "right"])]
        direction: String,

        /// Number of wheel steps
        #[arg(default_value_t = 3)]
        amount: u32,

        /// Move the pointer over this element first
        #[arg(long, value_name = "TARGET")]
        at: Option<String>,

        /// Scan again instead of using the last scan
        #[arg(long)]
        fresh: bool,

        #[command(flatten)]
        area: Area,
    },

    /// Drag one element onto another
    Drag {
        from: String,
        to: String,

        /// Scan again instead of using the last scan
        #[arg(long)]
        fresh: bool,

        #[command(flatten)]
        area: Area,
    },

    /// Type text into whatever has focus
    Type {
        /// Leading dashes are part of the text, not flags
        #[arg(allow_hyphen_values = true)]
        text: String,
    },

    /// Press a key or a combination, such as ctrl+s, super+2 or slash
    Key {
        #[arg(allow_hyphen_values = true)]
        combination: String,
    },

    /// Run steps in order: click, fill, type, key, wait, scroll, drag
    Run {
        /// Steps such as "click Save", "wait Saved", "scroll down 3", "drag A to B"
        #[arg(required = true)]
        steps: Vec<String>,

        #[command(flatten)]
        area: Area,
    },

    /// Keep the models loaded in the background, so later commands are faster
    Serve,

    /// Serve the commands as MCP tools over stdio
    Mcp,

    /// List installed Tesseract text recognition languages
    Languages,

    /// Say whether a daemon is running
    Status,

    /// Capture once through the desktop portal, for checking GNOME and KDE
    #[cfg(target_os = "linux")]
    #[command(hide = true)]
    Portal,

    /// List what the accessibility tree reports, window by window
    #[cfg(target_os = "linux")]
    Tree,

    /// Read an image file instead of the screen, for testing and debugging
    Read {
        path: PathBuf,

        /// Magnify before reading (1–4); can help small text, costs more time
        #[arg(long, default_value_t = 1, value_name = "N", value_parser = clap::value_parser!(u32).range(1..=4))]
        scale: u32,

        /// Read this language with tesseract instead of the built-in model
        #[arg(long, value_name = "CODE", env = "SCREENPEEK_LANG")]
        lang: Option<String>,

        #[arg(long)]
        json: bool,
    },

    /// Click an element and type into it
    Fill {
        target: String,
        text: String,

        /// Scan again instead of using the last scan
        #[arg(long)]
        fresh: bool,

        #[command(flatten)]
        area: Area,
    },
}

/// Which part of the desktop to read.
#[derive(Args, Clone)]
struct Area {
    /// Read only this part of the desktop, as x,y,width,height
    #[arg(long, value_name = "X,Y,W,H")]
    region: Option<Region>,

    /// Read this monitor instead of the primary one
    #[arg(long, value_name = "INDEX", conflicts_with = "region")]
    monitor: Option<usize>,

    /// Read only the window that has focus
    #[arg(long, conflicts_with_all = ["region", "monitor"])]
    focused: bool,

    /// Tesseract language code, CODE+CODE, auto, or all (also SCREENPEEK_LANG)
    #[arg(long, value_name = "CODE", env = "SCREENPEEK_LANG")]
    lang: Option<String>,
}

impl Area {
    /// The part of the desktop to read, with `--focused` resolved.
    fn region(&self, windows: &[read::Placement]) -> Result<Option<Region>> {
        if !self.focused {
            return Ok(self.region);
        }
        windows
            .iter()
            .find(|window| window.focused)
            .map(|window| Some(window.rect()))
            .context("--focused needs a compositor that reports the focused window")
    }
}

fn main() -> Result<()> {
    match Cli::parse().command {
        Command::Scan { grep, area, json } => {
            let elements = scan(&area)?;
            let shown: Vec<&Element> = match &grep {
                Some(needle) => {
                    let needle = needle.to_lowercase();
                    elements
                        .iter()
                        .filter(|element| element.text.to_lowercase().contains(&needle))
                        .collect()
                }
                None => elements.iter().collect(),
            };
            print(&shown, json)?;
        }

        Command::Click {
            target,
            button,
            double,
            fresh,
            check,
            area,
        } => {
            let mut pointer = Pointer::new()?;
            let snapshot = resolve(&target, fresh, &area)?;
            let element = snapshot.find(&target)?;
            let times = if double { 2 } else { 1 };
            let act = |pointer: &mut Pointer| pointer.click(element.x, element.y, button, times);
            if check {
                checked(&mut pointer, element, act)?;
            } else {
                act(&mut pointer)?;
            }
            println!("{element}");
        }

        Command::Wait {
            target,
            timeout,
            gone,
            area,
        } => {
            let timeout = std::time::Duration::try_from_secs_f64(timeout)
                .context("--timeout must be a positive number of seconds")?;
            let snapshot = wait(&target, timeout, gone, &area)?;
            match snapshot.matches(&target).first() {
                Some(element) => println!("{element}"),
                None => println!("{target} is gone"),
            }
        }

        Command::Scroll {
            direction,
            amount,
            at,
            fresh,
            area,
        } => {
            let mut pointer = Pointer::new()?;
            if let Some(at) = &at {
                let snapshot = resolve(at, fresh, &area)?;
                let element = snapshot.find(at)?;
                pointer.move_to(element.x, element.y)?;
            }
            scroll(&mut pointer, &direction, amount)?;
        }

        Command::Drag {
            from,
            to,
            fresh,
            area,
        } => {
            let mut pointer = Pointer::new()?;
            let snapshot = resolve(&from, fresh, &area)?;
            let snapshot = if snapshot.can_resolve(&to) {
                snapshot
            } else {
                Snapshot::new(scan(&area)?)
            };
            let (start, end) = (snapshot.find(&from)?, snapshot.find(&to)?);
            pointer.drag((start.x, start.y), (end.x, end.y))?;
            println!("{start}\n{end}");
        }

        Command::Type { text } => Pointer::new()?.type_text(&text)?,

        Command::Key { combination } => Pointer::new()?.press(&combination)?,

        Command::Run { steps, area } => run(&steps, &area)?,

        Command::Serve => daemon::serve()?,

        Command::Mcp => mcp::serve()?,

        #[cfg(target_os = "linux")]
        Command::Tree => {
            let started = std::time::Instant::now();
            let windows = read::atspi::windows()?;
            let elapsed = started.elapsed();
            for window in &windows {
                println!("{} ({}x{})", window.title, window.width, window.height);
                for item in &window.items {
                    let role = item.role.unwrap_or("-");
                    println!(
                        "  {role} {} @{},{} {:?}",
                        item.text, item.x, item.y, item.states
                    );
                }
            }
            eprintln!("{} window(s) in {}ms", windows.len(), elapsed.as_millis());
        }

        Command::Languages => println!("{}", read::tesseract::installed()?.join("\n")),

        Command::Status => println!("{}", daemon::endpoint_summary()?),

        #[cfg(target_os = "linux")]
        Command::Portal => {
            let started = std::time::Instant::now();
            let capture = capture::portal::Portal::new()?.capture()?;
            println!(
                "portal capture {}x{} in {}ms",
                capture.image.width(),
                capture.image.height(),
                started.elapsed().as_millis()
            );
        }

        Command::Read {
            path,
            scale,
            lang,
            json,
        } => {
            let image = image::open(&path)
                .with_context(|| format!("cannot open {}", path.display()))?
                .into_rgba8();
            let capture = capture::Capture::from_image(image);
            let elements = match resolve_language(lang.as_deref())? {
                Some(language) => read::tesseract::read_scaled(&capture, &language, scale)?,
                None => read::Engine::load()?.read_scaled(&capture, scale)?,
            };
            print(&elements.iter().collect::<Vec<_>>(), json)?;
        }

        Command::Fill {
            target,
            text,
            fresh,
            area,
        } => {
            let mut pointer = Pointer::new()?;
            let snapshot = resolve(&target, fresh, &area)?;
            let element = snapshot.find(&target)?;
            pointer.click(element.x, element.y, Button::Left, 1)?;
            pointer.wait_for_focus();
            pointer.type_text(&text)?;
            println!("{element}");
        }
    }

    Ok(())
}

fn run(steps: &[String], area: &Area) -> Result<()> {
    let mut pointer = Pointer::new()?;
    let mut snapshot: Option<Snapshot> = None;
    // A snapshot that can answer every target, scanning only when needed.
    let current = |snapshot: &mut Option<Snapshot>, targets: &[&str]| -> Result<Snapshot> {
        match snapshot.take() {
            Some(known) if targets.iter().all(|t| known.can_resolve(t)) => Ok(known),
            _ => Ok(Snapshot::new(scan(area)?)),
        }
    };

    for step in steps {
        let (verb, argument) = step.split_once(' ').unwrap_or((step.as_str(), ""));
        match verb {
            "click" | "fill" => {
                let (target, text) = match verb {
                    "fill" => argument.split_once(" with ").unwrap_or((argument, "")),
                    _ => (argument, ""),
                };
                let known = current(&mut snapshot, &[target])?;
                let element = known.find(target)?;
                pointer.click(element.x, element.y, Button::Left, 1)?;
                println!("{element}");
                if verb == "fill" {
                    pointer.wait_for_focus();
                    pointer.type_text(text)?;
                }
            }
            "wait" => {
                let known = wait(argument, WAIT_TIMEOUT, false, area)?;
                if let Some(element) = known.matches(argument).first() {
                    println!("{element}");
                }
                snapshot = Some(known);
                continue;
            }
            "scroll" => {
                let (direction, amount) = argument.split_once(' ').unwrap_or((argument, "3"));
                let amount = amount.parse().context("scroll amount must be a number")?;
                scroll(&mut pointer, direction, amount)?;
            }
            "drag" => {
                let (from, to) = argument
                    .split_once(" to ")
                    .context("drag steps look like \"drag A to B\"")?;
                let known = current(&mut snapshot, &[from, to])?;
                let (start, end) = (known.find(from)?, known.find(to)?);
                pointer.drag((start.x, start.y), (end.x, end.y))?;
                println!("{start}\n{end}");
            }
            "type" => pointer.type_text(argument)?,
            "key" => pointer.press(argument)?,
            other => anyhow::bail!("unknown step {other:?} in {step:?}"),
        }
        // Anything but a wait may have changed the screen.
        snapshot = None;
    }

    Ok(())
}

/// How long a `wait` step in `run` waits.
const WAIT_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(10);

/// Scan until the target appears (or, with `gone`, disappears).
fn wait(target: &str, timeout: std::time::Duration, gone: bool, area: &Area) -> Result<Snapshot> {
    let deadline = std::time::Instant::now() + timeout;
    loop {
        let snapshot = Snapshot::new(scan(area)?);
        if snapshot.matches(target).is_empty() == gone {
            return Ok(snapshot);
        }
        if std::time::Instant::now() >= deadline {
            match gone {
                true => anyhow::bail!("{target:?} is still on screen after {timeout:?}"),
                false => anyhow::bail!("nothing matched {target:?} within {timeout:?}"),
            }
        }
        std::thread::sleep(std::time::Duration::from_millis(150));
    }
}

fn scroll(pointer: &mut Pointer, direction: &str, amount: u32) -> Result<()> {
    let steps = i32::try_from(amount).context("scroll amount is too large")?;
    match direction {
        "down" => pointer.scroll(steps, false),
        "up" => pointer.scroll(-steps, false),
        "right" => pointer.scroll(steps, true),
        "left" => pointer.scroll(-steps, true),
        other => anyhow::bail!("unknown scroll direction {other:?}; use up, down, left or right"),
    }
}

/// Run an action and warn when the pixels around the target stay the same.
fn checked(
    pointer: &mut Pointer,
    element: &Element,
    act: impl FnOnce(&mut Pointer) -> Result<()>,
) -> Result<()> {
    let near = || {
        let around = Region {
            x: element.x - 200,
            y: element.y - 100,
            width: 400,
            height: 200,
        };
        capture::screen(None, Some(around))
            .or_else(|_| {
                capture::screen(
                    None,
                    Some(Region {
                        x: element.x,
                        y: element.y,
                        width: 200,
                        height: 100,
                    }),
                )
            })
            .ok()
    };
    let before = near();
    act(pointer)?;
    std::thread::sleep(std::time::Duration::from_millis(250));
    match (before, near()) {
        (Some(before), Some(after)) if before.image == after.image => {
            eprintln!(
                "screenpeek: nothing changed near {:?} after the action",
                element.text
            )
        }
        (None, _) | (_, None) => eprintln!("screenpeek: cannot capture to check the action"),
        _ => {}
    }
    Ok(())
}

fn scan(area: &Area) -> Result<Vec<Element>> {
    let windows = read::placements();
    let region = area.region(&windows)?;
    let language = resolve_language(area.lang.as_deref())?;
    let excluded = caller::regions(&windows);
    let mut elements = match controls(area) {
        Some(elements) => elements,
        None => match daemon::ask(region, area.monitor, language.clone(), excluded.clone()) {
            Some(elements) => elements,
            None => {
                // The tree walk overlaps capture and recognition.
                let (tree, recognized) = std::thread::scope(|scope| {
                    let tree = scope.spawn(tree);
                    let recognized = (|| {
                        let mut capture = capture::screen(area.monitor, region)?;
                        capture.exclude(&excluded);
                        match &language {
                            Some(language) => read::tesseract::read(&capture, language),
                            None => {
                                read::Engine::load()?.read_within(&capture, &read::edges(&windows))
                            }
                        }
                    })();
                    (tree.join().unwrap_or_default(), recognized)
                });
                with_tree_text(recognized?, tree, &windows)
            }
        },
    };
    if let Some(region) = region {
        elements.retain(|element| region.contains(element.x, element.y));
        index::number(&mut elements);
    }
    caller::filter(&mut elements, &excluded);
    if let Some(previous) = Snapshot::load() {
        index::keep_ids(&mut elements, &previous.elements);
    }
    Snapshot::new(elements.clone())
        .save()
        .context("cannot cache this scan")?;
    Ok(elements)
}

/// A snapshot that can answer `target`, scanning again only when needed.
fn resolve(target: &str, fresh: bool, area: &Area) -> Result<Snapshot> {
    if !fresh && area.region.is_none() && area.monitor.is_none() && !area.focused {
        if let Some(mut snapshot) = Snapshot::load() {
            caller::filter(
                &mut snapshot.elements,
                &caller::regions(&read::placements()),
            );
            if snapshot.can_resolve(target) {
                return Ok(snapshot);
            }
        }
    }

    Ok(Snapshot::new(scan(area)?))
}

/// Validate languages and use locale or accessible text for auto-selection.
fn resolve_language(requested: Option<&str>) -> Result<Option<String>> {
    let Some(requested) = requested else {
        return Ok(None);
    };

    let installed = read::tesseract::installed()?;

    // Include English for mixed-language labels when its model is installed.
    if requested == "all" {
        return Ok(Some(read::language::every(&installed)));
    }
    if requested != "auto" {
        read::tesseract::supports(requested)?;
        return Ok(Some(read::language::with_english(requested, &installed)));
    }

    Ok(read::language::detect(&known_text(), &installed))
}

/// Accessible text, which is what `--lang auto` guesses from.
#[cfg(target_os = "linux")]
fn known_text() -> Vec<Element> {
    let Ok(windows) = read::atspi::windows() else {
        return Vec::new();
    };
    windows
        .iter()
        .flat_map(|window| window.items.iter())
        .map(|item| Element {
            id: 0,
            text: item.text.clone(),
            x: item.x,
            y: item.y,
            width: item.width,
            height: item.height,
            source: index::Source::Tree,
            role: item.role.map(String::from),
            states: item.states.iter().map(|state| state.to_string()).collect(),
        })
        .collect()
}

#[cfg(windows)]
fn known_text() -> Vec<Element> {
    read::ui::elements().unwrap_or_default()
}

#[cfg(not(any(target_os = "linux", windows)))]
fn known_text() -> Vec<Element> {
    Vec::new()
}

#[cfg(target_os = "linux")]
type Tree = Vec<read::atspi::Window>;
#[cfg(not(target_os = "linux"))]
type Tree = ();

#[cfg(target_os = "linux")]
fn tree() -> Tree {
    read::atspi::windows().unwrap_or_default()
}

#[cfg(not(target_os = "linux"))]
fn tree() -> Tree {}

/// Place tree labels using compositor geometry or matching OCR text.
#[cfg(target_os = "linux")]
fn with_tree_text(
    elements: Vec<Element>,
    tree: Tree,
    placements: &[read::Placement],
) -> Vec<Element> {
    if tree.is_empty() {
        return elements;
    }
    if placements.is_empty() {
        return read::fuse::fuse(elements, &tree);
    }
    let located = read::fuse::place(&tree, placements)
        .into_iter()
        .flat_map(|window| window.elements)
        .collect();
    index::merge_tree(elements, located)
}

#[cfg(not(target_os = "linux"))]
fn with_tree_text(
    elements: Vec<Element>,
    _tree: Tree,
    _placements: &[read::Placement],
) -> Vec<Element> {
    elements
}

#[cfg(windows)]
fn controls(area: &Area) -> Option<Vec<Element>> {
    let elements = match read::ui::elements() {
        Ok(elements) if !elements.is_empty() => elements,
        Ok(_) => return None,
        Err(error) => {
            eprintln!("screenpeek: UI Automation unavailable, reading the pixels instead: {error}");
            return None;
        }
    };
    Some(match area.region {
        Some(region) => elements
            .into_iter()
            .filter(|element| region.contains(element.x, element.y))
            .collect(),
        None => elements,
    })
}

#[cfg(not(windows))]
fn controls(_area: &Area) -> Option<Vec<Element>> {
    None
}

/// Print the listing; a closed pipe (`scan | head`) is not an error.
fn print(elements: &[&Element], json: bool) -> Result<()> {
    use std::io::Write;

    let stdout = std::io::stdout();
    let mut out = stdout.lock();

    let written = if json {
        writeln!(out, "{}", serde_json::to_string_pretty(elements)?)
    } else {
        elements
            .iter()
            .try_for_each(|element| writeln!(out, "{element}"))
    };

    match written {
        Err(error) if error.kind() == std::io::ErrorKind::BrokenPipe => Ok(()),
        other => Ok(other?),
    }
}
