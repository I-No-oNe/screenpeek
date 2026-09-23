//! Keeps the latest frame of each screen-cast stream and hands them over on request.
//! The screenpeek daemon runs it on KDE and GNOME only, so other desktops need no PipeWire.
//!
//! Arguments: PipeWire node ids. Fd 3: the portal's PipeWire connection.
//! Each stdin line `SETTLE_MS` is answered after that long, per stream, with
//! `WIDTH HEIGHT STRIDE FORMAT` and then `STRIDE * HEIGHT` bytes of pixels.

#[cfg(target_os = "linux")]
mod linux;

#[cfg(target_os = "linux")]
fn main() {
    // Running at all proves libpipewire loads, which is what `screenpeek doctor` asks.
    if std::env::args().nth(1).as_deref() == Some("--version") {
        println!("screenpeek-frames {}", env!("CARGO_PKG_VERSION"));
        return;
    }
    if let Err(error) = linux::run() {
        eprintln!("screenpeek-frames: {error}");
        std::process::exit(1);
    }
}

#[cfg(not(target_os = "linux"))]
fn main() {
    eprintln!("screenpeek-frames: only needed on Linux");
    std::process::exit(1);
}
