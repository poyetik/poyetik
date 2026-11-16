//! The visual library on the GPU: the square rendered by one WGSL shader (`square.wgsl`)
//! through `wgpu` (Metal, Vulkan or DirectX 12 natively), bit for bit the image of the CPU
//! reference renderer.
//!
//! Where the square is comes from the CPU ([`Square::placement`], exact, 32-bit edges); the
//! shader only fills the pixels, in integers, then converts each coverage to a float without
//! rounding, so that every GPU following WGSL's rules gives the CPU reference's bits. The
//! same shader runs in browsers through WebGPU. An image larger than the GPU's buffers is
//! rendered in bands of rows, each band the same pixels as the whole image would hold.

#![forbid(unsafe_code)]
#![doc(test(attr(forbid(unsafe_code))))]
#![expect(
    clippy::multiple_crate_versions,
    reason = "wgpu's own tree holds two versions of hashbrown (naga's indexmap, gpu-allocator) \
              and of syn (bytemuck's and zerocopy's derives), which this crate cannot choose; \
              each pair is a reviewed exception of deny.toml"
)]

extern crate alloc;

use alloc::boxed::Box;
use core::fmt;
use core::future::Future;
use core::pin::{Pin, pin};
use core::task::{Context, Waker};
use std::sync::mpsc;
use std::thread;

use visual_boxes::Square;
use visual_core::{Image, ImageIndex};

#[cfg(test)]
#[path = "lib_tests.rs"]
mod tests;

/// The WGSL source of the square's shader, shared with the browser.
pub const SQUARE_WGSL: &str = include_str!("square.wgsl");

/// Pixels per side of a workgroup of the shader (`@workgroup_size(8, 8)`).
const WORKGROUP: u32 = 8;

/// Bytes per pixel: four 32-bit floats.
const PIXEL_BYTES: u64 = 16;

/// Bytes of the shader's `Placement` uniform: seven 32-bit integers, padded to 32.
const PLACEMENT_BYTES: usize = 32;

/// Why the GPU could not render.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GpuError {
    /// No GPU adapter answered: none, or none of the APIs asked for.
    NoAdapter(String),
    /// The adapter refused to open a device.
    NoDevice(String),
    /// The device did not give the rendered pixels back: lost, or destroyed.
    Lost(String),
}

impl fmt::Display for GpuError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NoAdapter(why) => write!(f, "no GPU adapter: {why}"),
            Self::NoDevice(why) => write!(f, "no GPU device: {why}"),
            Self::Lost(why) => write!(f, "the GPU did not give the image back: {why}"),
        }
    }
}

impl core::error::Error for GpuError {}

/// Runs `future` to completion on this thread, yielding it while the future is pending.
///
/// wgpu's native futures are ready at their first poll, so the loop never spins in practice;
/// a future that is not is polled again after each yield, with no waker to wait for.
/// Not generic, so that its one instantiation is the one its tests cover: a caller wraps
/// its future in an `async` block that stores the output.
fn block_on(mut future: Pin<&mut dyn Future<Output = ()>>) {
    let mut context = Context::from_waker(Waker::noop());
    while future.as_mut().poll(&mut context).is_pending() {
        thread::yield_now();
    }
}

/// The square's shader, compiled for one GPU.
#[derive(Debug)]
pub struct GpuSquare {
    /// The device.
    device: wgpu::Device,
    /// Its queue.
    queue: wgpu::Queue,
    /// The compute pipeline of `square.wgsl`.
    pipeline: wgpu::ComputePipeline,
    /// The most bytes of pixels one band may hold: what one storage buffer, bound whole, and
    /// one buffer to read back can hold on this device.
    band_bytes: u64,
    /// The GPU's name and API, for reports.
    adapter: String,
}

impl GpuSquare {
    /// Opens the first GPU of `backends` (all of them: [`wgpu::Backends::all`]) and compiles
    /// the shader.
    ///
    /// The device is opened with the adapter's own limits, so that the bands are as large
    /// as this GPU allows.
    ///
    /// # Errors
    ///
    /// Returns [`GpuError::NoAdapter`] when no GPU of `backends` answers, and
    /// [`GpuError::NoDevice`] when it refuses a device.
    pub fn new(backends: wgpu::Backends) -> Result<Self, GpuError> {
        Self::open(backends, |limits| limits)
    }

    /// [`GpuSquare::new`], asking the device for `limits` applied to the adapter's own.
    fn open(
        backends: wgpu::Backends,
        limits: impl FnOnce(wgpu::Limits) -> wgpu::Limits,
    ) -> Result<Self, GpuError> {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
            backends,
            ..wgpu::InstanceDescriptor::new_without_display_handle()
        });
        // Replaced by the answer: `block_on` returns once the request has completed.
        let mut adapter = Err(GpuError::NoAdapter(String::new()));
        block_on(pin!(async {
            adapter = instance
                .request_adapter(&wgpu::RequestAdapterOptions::default())
                .await
                .map_err(|error| GpuError::NoAdapter(error.to_string()));
        }));
        let adapter = adapter?;
        let required_limits = limits(adapter.limits());
        // Replaced by the answer, as the adapter's.
        let mut device = Err(GpuError::NoDevice(String::new()));
        block_on(pin!(async {
            device = adapter
                .request_device(&wgpu::DeviceDescriptor {
                    label: Some("square"),
                    required_limits: required_limits.clone(),
                    ..wgpu::DeviceDescriptor::default()
                })
                .await
                .map_err(|error| GpuError::NoDevice(error.to_string()));
        }));
        let (device, queue) = device?;
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("square"),
            source: wgpu::ShaderSource::Wgsl(SQUARE_WGSL.into()),
        });
        let pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("square"),
            layout: None,
            module: &module,
            entry_point: Some("main"),
            compilation_options: wgpu::PipelineCompilationOptions::default(),
            cache: None,
        });
        let info = adapter.get_info();
        Ok(Self {
            device,
            queue,
            pipeline,
            band_bytes: required_limits
                .max_storage_buffer_binding_size
                .min(required_limits.max_buffer_size),
            adapter: format!("{} ({})", info.name, info.backend),
        })
    }

    /// The GPU in use, and its API.
    #[must_use]
    pub fn adapter(&self) -> &str {
        &self.adapter
    }

    /// Renders image `index` of `square` into `output` on the GPU: exactly the values the
    /// CPU reference renders ([`visual_core::VisualBox::render`]), at `output`'s size.
    ///
    /// # Errors
    ///
    /// Returns [`GpuError::Lost`] when the device does not give the pixels back (lost,
    /// destroyed, out of memory, or refusing the work); `output` then holds the bands
    /// rendered before.
    pub fn render(
        &self,
        square: &Square,
        index: ImageIndex,
        output: &mut Image,
    ) -> Result<(), GpuError> {
        // Without its own scopes, a refused operation would reach wgpu's default handler,
        // which panics.
        let refused = self.device.push_error_scope(wgpu::ErrorFilter::Validation);
        let exhausted = self.device.push_error_scope(wgpu::ErrorFilter::OutOfMemory);
        let rendered = self.render_bands(square, index, output);
        let mut errors = (None, None);
        block_on(pin!(async {
            errors = (exhausted.pop().await, refused.pop().await);
        }));
        errors
            .0
            .or(errors.1)
            .map_or(rendered, |error| Err(GpuError::Lost(error.to_string())))
    }

    /// [`GpuSquare::render`], within its error scopes.
    fn render_bands(
        &self,
        square: &Square,
        index: ImageIndex,
        output: &mut Image,
    ) -> Result<(), GpuError> {
        let size = output.size();
        let placed = square.placement(index, size);
        let row_bytes = u64::from(size.width()).saturating_mul(PIXEL_BYTES);
        // At least one row: a row of the widest image, 256 KiB, is below the smallest
        // binding size WebGPU guarantees (128 MiB).
        let band_rows = u32::try_from(self.band_bytes.checked_div(row_bytes).unwrap_or(0))
            .unwrap_or(u32::MAX)
            .clamp(1, size.height());
        let band_bytes = row_bytes.saturating_mul(u64::from(band_rows));
        let uniform = self.buffer(
            "placement",
            32,
            wgpu::BufferUsages::UNIFORM.union(wgpu::BufferUsages::COPY_DST),
        );
        let storage = self.buffer(
            "values",
            band_bytes,
            wgpu::BufferUsages::STORAGE.union(wgpu::BufferUsages::COPY_SRC),
        );
        let readback = self.buffer(
            "readback",
            band_bytes,
            wgpu::BufferUsages::MAP_READ.union(wgpu::BufferUsages::COPY_DST),
        );
        let bind_group = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("square"),
            layout: &self.pipeline.get_bind_group_layout(0),
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: uniform.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: storage.as_entire_binding(),
                },
            ],
        });
        let rows_per_band = usize::try_from(band_rows).unwrap_or(usize::MAX);
        let mut first_row = 0_u32;
        for band in output
            .rows_mut()
            .collect::<Vec<_>>()
            .chunks_mut(rows_per_band)
        {
            let rows = u32::try_from(band.len()).unwrap_or(band_rows);
            let fields = [
                placed.left(),
                placed.right(),
                placed.top(),
                placed.bottom(),
                size.width(),
                first_row,
                rows,
                0,
            ];
            let mut uniform_bytes = [0_u8; PLACEMENT_BYTES];
            for (slot, field) in uniform_bytes.as_chunks_mut::<4>().0.iter_mut().zip(fields) {
                *slot = field.to_le_bytes();
            }
            self.queue.write_buffer(&uniform, 0, &uniform_bytes);
            let mut encoder = self
                .device
                .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                    label: Some("square"),
                });
            {
                let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                    label: Some("square"),
                    timestamp_writes: None,
                });
                pass.set_pipeline(&self.pipeline);
                pass.set_bind_group(0, &bind_group, &[]);
                pass.dispatch_workgroups(
                    size.width().div_ceil(WORKGROUP),
                    rows.div_ceil(WORKGROUP),
                    1,
                );
            }
            let bytes = row_bytes.saturating_mul(u64::from(rows));
            encoder.copy_buffer_to_buffer(&storage, 0, &readback, 0, bytes);
            let _submitted = self.queue.submit([encoder.finish()]);
            self.read_back(&readback, bytes, band)
                .map_err(|error| GpuError::Lost(error.to_string()))?;
            first_row = first_row.saturating_add(rows);
        }
        Ok(())
    }

    /// A buffer of this device.
    fn buffer(&self, label: &str, size: u64, usage: wgpu::BufferUsages) -> wgpu::Buffer {
        self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some(label),
            size,
            usage,
            mapped_at_creation: false,
        })
    }

    /// Waits for the GPU, then reads the first `bytes` of `readback` into the rows of `band`.
    ///
    /// # Errors
    ///
    /// Returns why the pixels could not be read: one error for the caller, whatever the
    /// step that failed (the device's wait, the mapping, or its answer).
    fn read_back(
        &self,
        readback: &wgpu::Buffer,
        bytes: u64,
        band: &mut [&mut [f32]],
    ) -> Result<(), Box<dyn core::error::Error>> {
        let (sender, receiver) = mpsc::channel();
        readback
            .slice(..bytes)
            .map_async(wgpu::MapMode::Read, move |outcome| {
                drop(sender.send(outcome));
            });
        let _polled = self.device.poll(wgpu::PollType::wait_indefinitely())?;
        receiver.try_recv()??;
        let mapped = readback.slice(..bytes).get_mapped_range()?;
        let words = mapped.as_chunks::<4>().0;
        for (value, word) in band.iter_mut().flat_map(|row| row.iter_mut()).zip(words) {
            *value = f32::from_le_bytes(*word);
        }
        drop(mapped);
        readback.unmap();
        Ok(())
    }
}
