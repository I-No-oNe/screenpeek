//! screenpeek: read the screen as numbered text, then click it by name.

mod daemon;
mod index;
mod ocr;
mod pointer;
mod screen;
#[cfg(windows)]
mod ui;
#[cfg(target_os = "linux")]
mod wayland;

use std::path::PathBuf;

use anyhow::{Context, Result};
use clap::{Args, Parser, Subcommand};

use index::{Element, Snapshot};
use pointer::{Button, Pointer};
use screen::Region;

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

    /// Keep the models loaded in the background, so later commands are faster
    Serve,

    /// Say whether a daemon is running
    Status,

    /// Read an image file instead of the screen, for testing and debugging
    Read {
        path: PathBuf,

        /// Magnify before reading. Measured to lower accuracy, see BENCHMARK.md
        #[arg(long, default_value_t = 1, value_name = "N")]
        scale: u32,

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

        Command::Serve => daemon::serve()?,

        Command::Status => println!("{}", daemon::endpoint_summary()?),

        Command::Read { path, scale, json } => {
            let image = image::open(&path)
                .with_context(|| format!("cannot open {}", path.display()))?
                .into_rgba8();
            let capture = screen::Capture::from_image(image);
            let elements = ocr::Engine::load()?.read_scaled(&capture, scale)?;
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
fn scan(area: &Area) -> Result<Vec<Element>> {
    let elements = match controls(area) {
        Some(elements) => elements,
        None => match daemon::ask(area.region, area.monitor) {
            Some(elements) => elements,
            None => {
                let capture = screen::capture(area.monitor, area.region)?;
                ocr::Engine::load()?.read(&capture)?
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

#[cfg(windows)]
fn controls(area: &Area) -> Option<Vec<Element>> {
    let elements = match ui::elements() {
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

fn print(elements: &[&Element], json: bool) -> Result<()> {
    if json {
        println!("{}", serde_json::to_string_pretty(elements)?);
        return Ok(());
    }
    for element in elements {
        println!("{element}");
    }
    Ok(())
}
