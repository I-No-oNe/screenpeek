//! Repeated capture on wlroots compositors, holding one Wayland connection
//! and one shared-memory buffer so a frame costs the copy and nothing else.
//! Anything that does not speak wlr-screencopy falls back to `screen::capture`.

use std::fs::File;
use std::os::fd::AsFd;

use anyhow::{anyhow, bail, Context, Result};
use image::RgbaImage;
use memmap2::MmapMut;
use wayland_client::protocol::{
    wl_buffer::WlBuffer,
    wl_output::{self, WlOutput},
    wl_registry::{self, WlRegistry},
    wl_shm::{self, WlShm},
    wl_shm_pool::WlShmPool,
};
use wayland_client::{Connection, Dispatch, EventQueue, Proxy, QueueHandle};
use wayland_protocols_wlr::screencopy::v1::client::{
    zwlr_screencopy_frame_v1::{self, ZwlrScreencopyFrameV1},
    zwlr_screencopy_manager_v1::ZwlrScreencopyManagerV1,
};

use crate::screen::{Capture, Region};

/// A persistent capture session for one output.
pub struct Screencopy {
    queue: EventQueue<State>,
    state: State,
    manager: ZwlrScreencopyManagerV1,
    shm: WlShm,
    buffer: Option<Buffer>,
}

/// A shared-memory buffer the compositor copies into, reused between frames.
struct Buffer {
    wl_buffer: WlBuffer,
    map: MmapMut,
    width: u32,
    height: u32,
    stride: u32,
    format: wl_shm::Format,
    _pool: WlShmPool,
    _file: File,
}

#[derive(Default)]
struct State {
    shm: Option<WlShm>,
    manager: Option<ZwlrScreencopyManagerV1>,
    outputs: Vec<Output>,
    frame: FrameState,
}

struct Output {
    output: WlOutput,
    x: i32,
    y: i32,
}

/// What the compositor has said about the frame in flight.
#[derive(Default)]
struct FrameState {
    format: Option<(wl_shm::Format, u32, u32, u32)>,
    damage: Vec<Region>,
    ready: bool,
    failed: bool,
}

impl Screencopy {
    /// Connects, or fails if the compositor does not speak screencopy.
    pub fn new() -> Result<Screencopy> {
        let connection = Connection::connect_to_env().context("no Wayland display")?;
        let mut queue = connection.new_event_queue();
        let handle = queue.handle();
        let display = connection.display();
        display.get_registry(&handle, ());

        let mut state = State::default();
        queue.roundtrip(&mut state)?;
        // Outputs announce their position in a second round of events.
        queue.roundtrip(&mut state)?;

        let manager = state
            .manager
            .clone()
            .ok_or_else(|| anyhow!("this compositor does not support wlr-screencopy"))?;
        let shm = state
            .shm
            .clone()
            .ok_or_else(|| anyhow!("this compositor does not offer shared memory buffers"))?;
        if state.outputs.is_empty() {
            bail!("no outputs");
        }

        Ok(Screencopy {
            queue,
            state,
            manager,
            shm,
            buffer: None,
        })
    }

    /// Captures an output, indexed as the compositor announced them.
    pub fn capture(&mut self, monitor: Option<usize>) -> Result<Capture> {
        let index = monitor.unwrap_or(0);
        let output = self
            .state
            .outputs
            .get(index)
            .ok_or_else(|| anyhow!("no output {index}; found {}", self.state.outputs.len()))?;
        let origin = (output.x, output.y);
        let output = output.output.clone();

        let handle = self.queue.handle();
        self.state.frame = FrameState::default();
        let frame = self.manager.capture_output(0, &output, &handle, ());

        while self.state.frame.format.is_none() && !self.state.frame.failed {
            self.queue.blocking_dispatch(&mut self.state)?;
        }
        if self.state.frame.failed {
            bail!("the compositor refused the capture");
        }
        let (format, width, height, stride) = self.state.frame.format.take().unwrap();

        self.ensure_buffer(format, width, height, stride)?;
        let buffer = self.buffer.as_ref().expect("buffer just created");
        frame.copy(&buffer.wl_buffer);

        while !self.state.frame.ready && !self.state.frame.failed {
            self.queue.blocking_dispatch(&mut self.state)?;
        }
        frame.destroy();
        if self.state.frame.failed {
            bail!("the copy failed");
        }

        let buffer = self.buffer.as_ref().expect("buffer still here");
        Ok(Capture {
            image: buffer.to_image()?,
            origin,
        })
    }

    /// Reuses the buffer unless the output was reconfigured.
    fn ensure_buffer(
        &mut self,
        format: wl_shm::Format,
        width: u32,
        height: u32,
        stride: u32,
    ) -> Result<()> {
        if let Some(buffer) = &self.buffer {
            if buffer.format == format
                && buffer.width == width
                && buffer.height == height
                && buffer.stride == stride
            {
                return Ok(());
            }
        }

        let size = (stride as u64) * (height as u64);
        let file = tempfile::tempfile().context("cannot create a shared memory file")?;
        file.set_len(size)?;
        let map = unsafe { MmapMut::map_mut(&file) }.context("cannot map the capture buffer")?;

        let handle = self.queue.handle();
        let pool = self.shm.create_pool(file.as_fd(), size as i32, &handle, ());
        let wl_buffer = pool.create_buffer(
            0,
            width as i32,
            height as i32,
            stride as i32,
            format,
            &handle,
            (),
        );

        self.buffer = Some(Buffer {
            wl_buffer,
            map,
            width,
            height,
            stride,
            format,
            _pool: pool,
            _file: file,
        });
        Ok(())
    }
}

impl Buffer {
    fn to_image(&self) -> Result<RgbaImage> {
        let mut image = RgbaImage::new(self.width, self.height);
        let swap_red_and_blue = matches!(
            self.format,
            wl_shm::Format::Xrgb8888 | wl_shm::Format::Argb8888
        );

        for y in 0..self.height as usize {
            let row = &self.map[y * self.stride as usize..][..self.width as usize * 4];
            for x in 0..self.width as usize {
                let pixel = &row[x * 4..x * 4 + 4];
                let (r, g, b) = if swap_red_and_blue {
                    (pixel[2], pixel[1], pixel[0])
                } else {
                    (pixel[0], pixel[1], pixel[2])
                };
                image.put_pixel(x as u32, y as u32, image::Rgba([r, g, b, 255]));
            }
        }
        Ok(image)
    }
}

impl Dispatch<WlRegistry, ()> for State {
    fn event(
        state: &mut State,
        registry: &WlRegistry,
        event: wl_registry::Event,
        _: &(),
        _: &Connection,
        handle: &QueueHandle<State>,
    ) {
        let wl_registry::Event::Global {
            name,
            interface,
            version,
        } = event
        else {
            return;
        };

        match interface.as_str() {
            "wl_shm" => state.shm = Some(registry.bind(name, version.min(1), handle, ())),
            "zwlr_screencopy_manager_v1" => {
                state.manager = Some(registry.bind(name, version.min(3), handle, ()))
            }
            "wl_output" => {
                let output = registry.bind(name, version.min(4), handle, ());
                state.outputs.push(Output { output, x: 0, y: 0 });
            }
            _ => {}
        }
    }
}

impl Dispatch<WlOutput, ()> for State {
    fn event(
        state: &mut State,
        output: &WlOutput,
        event: wl_output::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<State>,
    ) {
        if let wl_output::Event::Geometry { x, y, .. } = event {
            if let Some(entry) = state
                .outputs
                .iter_mut()
                .find(|entry| entry.output.id() == output.id())
            {
                entry.x = x;
                entry.y = y;
            }
        }
    }
}

impl Dispatch<ZwlrScreencopyFrameV1, ()> for State {
    fn event(
        state: &mut State,
        _: &ZwlrScreencopyFrameV1,
        event: zwlr_screencopy_frame_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<State>,
    ) {
        match event {
            zwlr_screencopy_frame_v1::Event::Buffer {
                format,
                width,
                height,
                stride,
            } => {
                if let Ok(format) = format.into_result() {
                    state.frame.format = Some((format, width, height, stride));
                }
            }
            zwlr_screencopy_frame_v1::Event::Damage {
                x,
                y,
                width,
                height,
            } => state.frame.damage.push(Region {
                x: x as i32,
                y: y as i32,
                width,
                height,
            }),
            zwlr_screencopy_frame_v1::Event::Ready { .. } => state.frame.ready = true,
            zwlr_screencopy_frame_v1::Event::Failed => state.frame.failed = true,
            _ => {}
        }
    }
}

macro_rules! ignore_events {
    ($($interface:ty),* $(,)?) => {$(
        impl Dispatch<$interface, ()> for State {
            fn event(
                _: &mut State,
                _: &$interface,
                _: <$interface as Proxy>::Event,
                _: &(),
                _: &Connection,
                _: &QueueHandle<State>,
            ) {
            }
        }
    )*};
}

ignore_events!(WlShm, WlShmPool, WlBuffer, ZwlrScreencopyManagerV1);
