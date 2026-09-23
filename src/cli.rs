//! Command-line arguments.

use std::path::PathBuf;

use anyhow::{Context, Result};
use clap::{Args, Parser, Subcommand};

use crate::capture::Region;
use crate::pointer::Button;
use crate::read;

#[derive(Parser)]
#[command(version, about, long_about = None)]
pub(crate) struct Cli {
    #[command(subcommand)]
    pub(crate) command: Command,
}

#[derive(Subcommand)]
pub(crate) enum Command {
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

    /// List the visible windows, as `title @x,y WIDTHxHEIGHT`
    Windows {
        #[arg(long)]
        json: bool,
    },

    /// Bring a window to the front by (part of) its title
    Focus { title: String },

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

    /// Run steps in order: focus, click, fill, type, key, wait, scroll, drag
    Run {
        /// Steps such as "focus Firefox", "click Save", "wait Saved", "scroll down 3", "drag A to B"
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

    /// Check what this desktop supports and what is missing
    #[cfg(target_os = "linux")]
    Doctor,

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
pub(crate) struct Area {
    /// Read only this part of the desktop, as x,y,width,height
    #[arg(long, value_name = "X,Y,W,H")]
    pub(crate) region: Option<Region>,

    /// Read this monitor instead of the primary one
    #[arg(long, value_name = "INDEX", conflicts_with = "region")]
    pub(crate) monitor: Option<usize>,

    /// Read only the window that has focus
    #[arg(long, conflicts_with_all = ["region", "monitor"])]
    pub(crate) focused: bool,

    /// Tesseract language code, CODE+CODE, auto, or all (also SCREENPEEK_LANG)
    #[arg(long, value_name = "CODE", env = "SCREENPEEK_LANG")]
    pub(crate) lang: Option<String>,
}

impl Area {
    /// The part of the desktop to read, with `--focused` resolved.
    pub(crate) fn region(&self, windows: &[read::Placement]) -> Result<Option<Region>> {
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
