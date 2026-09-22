//! screenpeek: read the screen as numbered text, then click it by name.

mod capture;
mod daemon;
mod index;
mod pointer;
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

        #[command(flatten)]
        area: Area,
    },

    /// Type text into whatever has focus
    Type { text: String },

    /// Press a key or a combination, such as ctrl+s
    Key { combination: String },

    /// Run several steps against one scan: click, type, key, wait
    Run {
        /// Steps such as "click Save", "type report", "key enter", "wait Done"
        #[arg(required = true)]
        steps: Vec<String>,

        #[command(flatten)]
        area: Area,
    },

    /// Keep the models loaded in the background, so later commands are faster
    Serve,

    /// Say whether a daemon is running
    Status,

    /// List what the accessibility tree reports, window by window
    #[cfg(target_os = "linux")]
    Tree,

    /// Read an image file instead of the screen, for testing and debugging
    Read {
        path: PathBuf,

        /// Magnify before reading. Measured to lower accuracy, see BENCHMARK.md
        #[arg(long, default_value_t = 1, value_name = "N")]
        scale: u32,

        /// Read this language with tesseract instead of the built-in model
        #[arg(long, value_name = "CODE")]
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

    /// Read this language instead of English, for example deu, heb, chi_sim,
    /// or `auto` to work it out from the screen and the session's locale.
    /// Needs tesseract and that language's data installed
    #[arg(long, value_name = "CODE")]
    lang: Option<String>,
}

impl Area {
    /// The part of the desktop to read, with `--focused` resolved to the
    /// focused window's rectangle.
    fn region(&self) -> Result<Option<Region>> {
        if !self.focused {
            return Ok(self.region);
        }
        Ok(Some(focused_window()?))
    }
}

#[cfg(target_os = "linux")]
fn focused_window() -> Result<Region> {
    let placement = read::geometry::focused()?;
    Ok(Region {
        x: placement.x,
        y: placement.y,
        width: placement.width,
        height: placement.height,
    })
}

#[cfg(not(target_os = "linux"))]
fn focused_window() -> Result<Region> {
    anyhow::bail!("--focused needs a compositor that reports window positions")
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
            area,
        } => {
            let snapshot = resolve(&target, fresh, &area)?;
            let element = snapshot.find(&target)?;
            Pointer::new()?.click(element.x, element.y, button, if double { 2 } else { 1 })?;
            println!("{element}");
        }

        Command::Type { text } => Pointer::new()?.type_text(&text)?,

        Command::Key { combination } => Pointer::new()?.press(&combination)?,

        Command::Run { steps, area } => run(&steps, &area)?,

        Command::Serve => daemon::serve()?,

        #[cfg(target_os = "linux")]
        Command::Tree => {
            let started = std::time::Instant::now();
            let windows = read::atspi::windows()?;
            let elapsed = started.elapsed();
            for window in &windows {
                println!("{} ({}x{})", window.title, window.width, window.height);
                for item in &window.items {
                    println!("  {} @{},{}", item.text, item.x, item.y);
                }
            }
            eprintln!("{} window(s) in {}ms", windows.len(), elapsed.as_millis());
        }

        Command::Status => println!("{}", daemon::endpoint_summary()?),

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
                Some(language) => read::tesseract::read(&capture, &language)?,
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
            let snapshot = resolve(&target, fresh, &area)?;
            let element = snapshot.find(&target)?;
            let mut pointer = Pointer::new()?;
            pointer.click(element.x, element.y, Button::Left, 1)?;
            pointer.wait_for_focus();
            pointer.type_text(&text)?;
            println!("{element}");
        }
    }

    Ok(())
}

/// Control tree where the platform has one, daemon when it runs, else OCR.
/// Runs a sequence of steps against one scan, rescanning only when a step
/// cannot be answered from what is already known.
fn run(steps: &[String], area: &Area) -> Result<()> {
    let mut pointer = Pointer::new()?;
    let mut snapshot: Option<Snapshot> = None;

    for step in steps {
        let (verb, argument) = step.split_once(' ').unwrap_or((step.as_str(), ""));
        match verb {
            "click" | "wait" | "fill" => {
                let (target, text) = match verb {
                    "fill" => argument.split_once(" with ").unwrap_or((argument, "")),
                    _ => (argument, ""),
                };

                let known = snapshot
                    .as_ref()
                    .is_some_and(|snapshot| snapshot.can_resolve(target));
                if !known {
                    snapshot = Some(Snapshot::new(scan(area)?));
                }
                let current = snapshot.as_ref().expect("just scanned");
                let element = current.find(target)?;

                if verb == "wait" {
                    println!("{element}");
                    continue;
                }

                pointer.click(element.x, element.y, Button::Left, 1)?;
                println!("{element}");
                if verb == "fill" {
                    pointer.wait_for_focus();
                    pointer.type_text(text)?;
                }
                snapshot = None;
            }
            "type" => pointer.type_text(argument)?,
            "key" => pointer.press(argument)?,
            other => anyhow::bail!("unknown step {other:?} in {step:?}"),
        }
    }

    Ok(())
}

fn scan(area: &Area) -> Result<Vec<Element>> {
    let region = area.region()?;
    let language = resolve_language(area.lang.as_deref())?;
    let elements = match controls(area) {
        Some(elements) => elements,
        None => match daemon::ask(region, area.monitor, language.clone()) {
            Some(elements) => elements,
            None => {
                let capture = capture::screen(area.monitor, region)?;
                let recognized = match &language {
                    Some(language) => read::tesseract::read(&capture, language)?,
                    None => read::Engine::load()?.read(&capture)?,
                };
                with_tree_text(recognized)
            }
        },
    };
    Snapshot::new(elements.clone())
        .save()
        .context("cannot cache this scan")?;
    Ok(elements)
}

/// A snapshot that can answer `target`, scanning again only when needed.
fn resolve(target: &str, fresh: bool, area: &Area) -> Result<Snapshot> {
    if !fresh {
        if let Some(snapshot) = Snapshot::load() {
            if snapshot.can_resolve(target) {
                return Ok(snapshot);
            }
        }
    }
    Ok(Snapshot::new(scan(area)?))
}

/// Turns `--lang` into the languages to read with, checking that tesseract has
/// them. `auto` looks at the text the accessibility tree already gives and at
/// the session's locale, and comes back empty when English is all that is
/// needed, since the built-in model reads that faster.
fn resolve_language(requested: Option<&str>) -> Result<Option<String>> {
    let Some(requested) = requested else {
        return Ok(None);
    };

    if requested != "auto" {
        read::tesseract::supports(requested)?;
        return Ok(Some(requested.to_owned()));
    }

    let installed = read::tesseract::installed()?;
    Ok(read::language::detect(&known_text(), &installed))
}

/// Text the platform hands over without recognition, which is what the
/// language guess is made from.
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

/// Replaces what was recognized with what the accessibility tree says, using
/// the compositor's window positions where it answers and matched labels where
/// it does not.
#[cfg(target_os = "linux")]
fn with_tree_text(elements: Vec<Element>) -> Vec<Element> {
    let Ok(windows) = read::atspi::windows() else {
        return elements;
    };
    if windows.is_empty() {
        return elements;
    }

    let Ok(placements) = read::geometry::windows() else {
        return read::fuse::fuse(elements, &windows);
    };

    let located = read::fuse::place(&windows, &placements);
    if located.is_empty() {
        return read::fuse::fuse(elements, &windows);
    }

    let mut merged: Vec<Element> = elements
        .into_iter()
        .filter(|element| {
            !located
                .iter()
                .any(|window| window.rect.contains(element.x, element.y))
        })
        .collect();
    for window in located {
        merged.extend(window.elements);
    }
    index::number(&mut merged);
    merged
}

#[cfg(not(target_os = "linux"))]
fn with_tree_text(elements: Vec<Element>) -> Vec<Element> {
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

/// Writes the listing, treating a closed pipe as the end of the work rather
/// than as a failure, so `screenpeek scan | head` is quiet.
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
