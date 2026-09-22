//! Reuse a Wayland connection and shared-memory buffer for wlr-screencopy.

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
use wayland_protocols::xdg::xdg_output::zv1::client::{
    zxdg_output_manager_v1::ZxdgOutputManagerV1,
    zxdg_output_v1::{self, ZxdgOutputV1},
};
use wayland_protocols_wlr::screencopy::v1::client::{
    zwlr_screencopy_frame_v1::{self, ZwlrScreencopyFrameV1},
    zwlr_screencopy_manager_v1::ZwlrScreencopyManagerV1,
};

use super::{Capture, Region};

/// A persistent capture session for one output.
pub struct Screencopy {
    queue: EventQueue<State>,
    state: State,
    manager: ZwlrScreencopyManagerV1,
    shm: WlShm,
    buffer: Option<Buffer>,
    _logical_outputs: Vec<ZxdgOutputV1>,
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
    logical_manager: Option<ZxdgOutputManagerV1>,
}

struct Output {
    output: WlOutput,
    x: i32,
    y: i32,
    width: i32,
    height: i32,
    logical_size: Option<(u32, u32)>,
    logical_position: Option<(i32, i32)>,
    scale: i32,
}

/// What the compositor has said about the frame in flight.
#[derive(Default)]
struct FrameState {
    format: Option<(wl_shm::Format, u32, u32, u32)>,
    ready: bool,
    failed: bool,
}

/// Return the logical desktop bounds reported by xdg-output.
pub fn logical_desktop() -> Option<(i32, i32, u32, u32)> {
    let connection = Connection::connect_to_env().ok()?;
    let mut queue = connection.new_event_queue();
    let handle = queue.handle();
    connection.display().get_registry(&handle, ());

    let mut state = State::default();
    queue.roundtrip(&mut state).ok()?;
    queue.roundtrip(&mut state).ok()?;

    let manager = state.logical_manager.clone()?;
    let _outputs: Vec<_> = state
        .outputs
        .iter()
        .map(|entry| manager.get_xdg_output(&entry.output, &handle, entry.output.clone()))
        .collect();
    queue.roundtrip(&mut state).ok()?;

    let placed: Vec<_> = state
        .outputs
        .iter()
        .filter_map(|output| {
            let (x, y) = output.logical_position.unwrap_or((output.x, output.y));
            let (width, height) = output.logical_size?;
            Some((x, y, width, height))
        })
        .collect();
    if placed.is_empty() {
        return None;
    }

    let left = placed.iter().map(|o| o.0).min()?;
    let top = placed.iter().map(|o| o.1).min()?;
    let right = placed.iter().map(|o| o.0 + o.2 as i32).max()?;
    let bottom = placed.iter().map(|o| o.1 + o.3 as i32).max()?;
    Some((left, top, (right - left) as u32, (bottom - top) as u32))
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

        let logical_outputs = if let Some(manager) = &state.logical_manager {
            state
                .outputs
                .iter()
                .map(|entry| manager.get_xdg_output(&entry.output, &handle, entry.output.clone()))
                .collect()
        } else {
            Vec::new()
        };
        queue.roundtrip(&mut state)?;

        Ok(Screencopy {
            queue,
            state,
            manager,
            shm,
            buffer: None,
            _logical_outputs: logical_outputs,
        })
    }

    /// Captures an output, indexed as the compositor announced them.
    pub fn capture(&mut self, monitor: Option<usize>) -> Result<Capture> {
        self.capture_part(monitor.unwrap_or(0), None)
    }

    /// Capture an output or a region in output logical coordinates.
    fn capture_part(&mut self, index: usize, part: Option<Region>) -> Result<Capture> {
        let output = self
            .state
            .outputs
            .get(index)
            .ok_or_else(|| anyhow!("no output {index}; found {}", self.state.outputs.len()))?;
        let output_origin = output.logical_position.unwrap_or((output.x, output.y));
        let origin = part.map_or(output_origin, |part| (part.x, part.y));
        let logical_size = match part {
            Some(part) => Some((part.width, part.height)),
            None => output.logical_size,
        };
        if output.scale != 1 && logical_size.is_none() {
            bail!("scaled capture needs xdg-output logical geometry");
        }
        let output = output.output.clone();

        let handle = self.queue.handle();
        self.state.frame = FrameState::default();
        let frame = match part {
            None => self.manager.capture_output(0, &output, &handle, ()),
            Some(part) => self.manager.capture_output_region(
                0,
                &output,
                part.x - output_origin.0,
                part.y - output_origin.1,
                part.width as i32,
                part.height as i32,
                &handle,
                (),
            ),
        };

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
            image: logical_image(buffer.to_image()?, logical_size),
            origin,
        })
    }

    /// Captures the output a region starts on.
    pub fn capture_containing(&mut self, region: Region) -> Result<Capture> {
        let index = self
            .state
            .outputs
            .iter()
            .position(|output| {
                let (x, y) = output.logical_position.unwrap_or((output.x, output.y));
                let (width, height) = output
                    .logical_size
                    .unwrap_or((output.width as u32, output.height as u32));
                Region {
                    x,
                    y,
                    width,
                    height,
                }
                .contains(region.x, region.y)
            })
            .ok_or_else(|| anyhow!("no output contains {},{}", region.x, region.y))?;

        // Older compositors may not implement the region request; a whole
        // output still answers the question, just with more pixels.
        match self.capture_part(index, Some(region)) {
            Ok(capture) => Ok(capture),
            Err(error) => {
                eprintln!("screenpeek: region capture failed, taking the output: {error}");
                self.capture_part(index, None)
            }
        }
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
            "zxdg_output_manager_v1" => {
                state.logical_manager = Some(registry.bind(name, version.min(3), handle, ()));
            }
            "wl_output" => {
                let output = registry.bind(name, version.min(4), handle, ());
                state.outputs.push(Output {
                    output,
                    x: 0,
                    y: 0,
                    width: 0,
                    height: 0,
                    logical_size: None,
                    logical_position: None,
                    scale: 1,
                });
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
        let Some(entry) = state
            .outputs
            .iter_mut()
            .find(|entry| entry.output.id() == output.id())
        else {
            return;
        };

        match event {
            wl_output::Event::Geometry { x, y, .. } => {
                entry.x = x;
                entry.y = y;
            }
            wl_output::Event::Scale { factor } => entry.scale = factor,
            wl_output::Event::Mode { width, height, .. } => {
                entry.width = width;
                entry.height = height;
            }
            _ => {}
        }
    }
}

impl Dispatch<ZxdgOutputV1, WlOutput> for State {
    fn event(
        state: &mut State,
        _: &ZxdgOutputV1,
        event: zxdg_output_v1::Event,
        output: &WlOutput,
        _: &Connection,
        _: &QueueHandle<State>,
    ) {
        let Some(entry) = state
            .outputs
            .iter_mut()
            .find(|entry| entry.output.id() == output.id())
        else {
            return;
        };
        match event {
            zxdg_output_v1::Event::LogicalSize { width, height } if width > 0 && height > 0 => {
                entry.logical_size = Some((width as u32, height as u32));
            }
            zxdg_output_v1::Event::LogicalPosition { x, y } => {
                entry.logical_position = Some((x, y))
            }
            _ => {}
        }
    }
}

/// OCR, compositor geometry and input all use logical desktop pixels.
fn logical_image(image: RgbaImage, size: Option<(u32, u32)>) -> RgbaImage {
    match size {
        Some((width, height)) if image.dimensions() != (width, height) => {
            image::imageops::resize(&image, width, height, image::imageops::FilterType::Triangle)
        }
        _ => image,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn fractional_scaling_uses_logical_click_coordinates() {
        let image = RgbaImage::from_pixel(125, 100, image::Rgba([255; 4]));
        let image = logical_image(image, Some((100, 80)));
        let capture = Capture {
            image,
            origin: (-100, 20),
        };
        assert_eq!(capture.image.dimensions(), (100, 80));
        assert_eq!(capture.to_desktop(80, 40), (-20, 60));
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

ignore_events!(
    WlShm,
    WlShmPool,
    WlBuffer,
    ZwlrScreencopyManagerV1,
    ZxdgOutputManagerV1
);
