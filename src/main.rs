//! screenpeek: read the screen as numbered text, then click it by name.

mod act;
mod caller;
mod capture;
mod cli;
mod daemon;
#[cfg(target_os = "linux")]
mod doctor;
mod index;
mod look;
mod mcp;
mod pointer;
#[cfg(target_os = "linux")]
mod portal;
mod read;

use anyhow::{Context, Result};
use clap::Parser;

use index::Element;
use pointer::{Button, Pointer};

use act::{checked, describe, focus_window, pick_window, run, scroll};
use cli::{Cli, Command};
use look::{resolve, resolve_language, scan, scan_for, wait};

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
                scan_for(&[&from, &to], &area)?
            };
            let (start, end) = (snapshot.find(&from)?, snapshot.find(&to)?);
            pointer.drag((start.x, start.y), (end.x, end.y))?;
            println!("{start}\n{end}");
        }

        Command::Windows { json } => {
            let windows = read::placements();
            if json {
                let listed: Vec<_> = windows
                    .iter()
                    .map(|w| {
                        serde_json::json!({"title": w.title, "x": w.x, "y": w.y,
                        "width": w.width, "height": w.height, "focused": w.focused})
                    })
                    .collect();
                println!("{}", serde_json::to_string_pretty(&listed)?);
            } else {
                windows
                    .iter()
                    .for_each(|window| println!("{}", describe(window)));
            }
        }

        Command::Focus { title } => {
            let windows = read::placements();
            let window = pick_window(&windows, &title)?;
            focus_window(window)?;
            println!("{}", describe(window));
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
        Command::Doctor => doctor::doctor(),

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
            let language = resolve_language(lang.as_deref())?;
            let elements = read::Engine::load()?.read_scaled(&capture, scale, language.as_ref())?;
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
