use std::error::Error;
use std::io::{BufRead, BufWriter, Write};
use std::os::fd::{FromRawFd, OwnedFd};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

use pipewire as pw;
use pw::spa;
use pw::spa::param::video::{VideoFormat, VideoInfoRaw};
use pw::spa::pod::Pod;

/// How long a request waits for the first frame of a stream that has none yet.
const FIRST_FRAME: Duration = Duration::from_secs(2);

#[derive(Default)]
struct Frame {
    width: u32,
    height: u32,
    stride: u32,
    format: &'static str,
    pixels: Vec<u8>,
    /// Frames received, so a request can wait for the first one.
    seen: u64,
}

struct Shared {
    frames: Mutex<Vec<Frame>>,
    arrived: Condvar,
}

pub fn run() -> Result<(), Box<dyn Error>> {
    let nodes: Vec<u32> = std::env::args()
        .skip(1)
        .map(|node| node.parse())
        .collect::<Result<_, _>>()?;
    // The daemon puts the portal's connection on fd 3 before starting us.
    let remote = unsafe { OwnedFd::from_raw_fd(3) };
    let shared = Arc::new(Shared {
        frames: Mutex::new(nodes.iter().map(|_| Frame::default()).collect()),
        arrived: Condvar::new(),
    });
    let answering = Arc::clone(&shared);
    std::thread::spawn(move || {
        answer(&answering);
        std::process::exit(0);
    });

    pw::init();
    let mainloop = pw::main_loop::MainLoopRc::new(None)?;
    let context = pw::context::ContextRc::new(&mainloop, None)?;
    let core = context.connect_fd_rc(remote, None)?;
    let format = format_pod()?;

    let mut streams = Vec::new();
    for (index, node) in nodes.iter().enumerate() {
        let stream = pw::stream::StreamRc::new(
            core.clone(),
            "screenpeek",
            pw::properties::properties! {
                *pw::keys::MEDIA_TYPE => "Video",
                *pw::keys::MEDIA_CATEGORY => "Capture",
                *pw::keys::MEDIA_ROLE => "Screen",
            },
        )?;
        let shared = Arc::clone(&shared);
        let listener = stream
            .add_local_listener_with_user_data(VideoInfoRaw::default())
            .param_changed(|_, info, id, param| {
                if let Some(param) = param.filter(|_| id == spa::param::ParamType::Format.as_raw())
                {
                    let _ = info.parse(param);
                }
            })
            .process(move |stream, info| {
                let Some(mut buffer) = stream.dequeue_buffer() else {
                    return;
                };
                let Some(data) = buffer.datas_mut().first_mut() else {
                    return;
                };
                let (offset, size, stride) = {
                    let chunk = data.chunk();
                    (
                        chunk.offset() as usize,
                        chunk.size() as usize,
                        chunk.stride(),
                    )
                };
                // Buffers we cannot map (DMA-BUF) are skipped; we only offer shared memory.
                let Some(bytes) = data.data() else {
                    return;
                };
                let Some(pixels) = bytes.get(offset..offset + size) else {
                    return;
                };
                let Some(format) = name(info.format()) else {
                    return;
                };
                let mut frames = shared.frames.lock().unwrap();
                let frame = &mut frames[index];
                frame.width = info.size().width;
                frame.height = info.size().height;
                frame.stride = stride.unsigned_abs();
                frame.format = format;
                frame.pixels.clear();
                frame.pixels.extend_from_slice(pixels);
                frame.seen += 1;
                shared.arrived.notify_all();
            })
            .register()?;
        stream.connect(
            spa::utils::Direction::Input,
            Some(*node),
            pw::stream::StreamFlags::AUTOCONNECT | pw::stream::StreamFlags::MAP_BUFFERS,
            &mut [Pod::from_bytes(&format).ok_or("bad format pod")?],
        )?;
        streams.push((stream, listener));
    }

    mainloop.run();
    Ok(())
}

/// Serve requests until the daemon closes stdin.
fn answer(shared: &Shared) {
    let stdout = std::io::stdout();
    for line in std::io::stdin().lock().lines() {
        let Ok(line) = line else { break };
        // Let the screen settle for this long, so input already sent shows in the frame.
        std::thread::sleep(Duration::from_millis(line.trim().parse().unwrap_or(0)));
        let started = Instant::now();
        let mut frames = shared.frames.lock().unwrap();
        while frames.iter().any(|frame| frame.seen == 0) && started.elapsed() < FIRST_FRAME {
            let left = FIRST_FRAME.saturating_sub(started.elapsed());
            frames = shared.arrived.wait_timeout(frames, left).unwrap().0;
        }
        let mut out = BufWriter::new(stdout.lock());
        let written = frames.iter().try_for_each(|frame| {
            writeln!(
                out,
                "{} {} {} {}",
                frame.width,
                frame.height,
                frame.stride,
                if frame.format.is_empty() {
                    "none"
                } else {
                    frame.format
                }
            )?;
            out.write_all(&frame.pixels)
        });
        if written.and_then(|()| out.flush()).is_err() {
            break;
        }
    }
}

fn name(format: VideoFormat) -> Option<&'static str> {
    Some(match format {
        VideoFormat::BGRx => "BGRx",
        VideoFormat::BGRA => "BGRA",
        VideoFormat::RGBx => "RGBx",
        VideoFormat::RGBA => "RGBA",
        _ => return None,
    })
}

/// Raw 4-byte formats at any size; without modifiers the desktop sends shared memory.
fn format_pod() -> Result<Vec<u8>, Box<dyn Error>> {
    use spa::param::format::{FormatProperties, MediaSubtype, MediaType};
    let object = spa::pod::object!(
        spa::utils::SpaTypes::ObjectParamFormat,
        spa::param::ParamType::EnumFormat,
        spa::pod::property!(FormatProperties::MediaType, Id, MediaType::Video),
        spa::pod::property!(FormatProperties::MediaSubtype, Id, MediaSubtype::Raw),
        spa::pod::property!(
            FormatProperties::VideoFormat,
            Choice,
            Enum,
            Id,
            VideoFormat::BGRx,
            VideoFormat::BGRx,
            VideoFormat::BGRA,
            VideoFormat::RGBx,
            VideoFormat::RGBA
        ),
        spa::pod::property!(
            FormatProperties::VideoSize,
            Choice,
            Range,
            Rectangle,
            spa::utils::Rectangle {
                width: 1920,
                height: 1080
            },
            spa::utils::Rectangle {
                width: 1,
                height: 1
            },
            spa::utils::Rectangle {
                width: 16384,
                height: 16384
            }
        ),
    );
    Ok(spa::pod::serialize::PodSerializer::serialize(
        std::io::Cursor::new(Vec::new()),
        &spa::pod::Value::Object(object),
    )?
    .0
    .into_inner())
}
