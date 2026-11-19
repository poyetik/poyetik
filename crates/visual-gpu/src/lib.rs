//! The visual library on the GPU: the square rendered by its WGSL shader ([`SQUARE_WGSL`])
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

use visual_boxes::SQUARE_WGSL;
use visual_boxes::Square;
use visual_core::{
    DISPLAY_THRESHOLD_BITS, DISPLAY_WGSL, DisplayImage, Image, ImageIndex, ImageSize,
};

pub mod bench;

#[cfg(test)]
#[path = "lib_tests.rs"]
mod tests;

/// Invocations of a workgroup of the display shader (`@workgroup_size(64)`).
const DISPLAY_WORKGROUP: u32 = 64;

/// Bytes of the display shader's `Params` uniform: two 32-bit integers, padded to 16.
const PARAMS_BYTES: usize = 16;

/// The display shader's thresholds as bytes: each of [`DISPLAY_THRESHOLD_BITS`], little-endian.
fn threshold_bytes() -> Vec<u8> {
    DISPLAY_THRESHOLD_BITS
        .iter()
        .flat_map(|bits| bits.to_le_bytes())
        .collect()
}

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
    /// The GPU refused the work: a limit below what it asks, memory it cannot hold, or a
    /// shader its back end cannot translate (wgpu's error scopes, [`ERROR_FILTERS`]).
    Refused(String),
    /// A display image was given an image of another size: the image's, width then height.
    Size(u32, u32),
}

impl fmt::Display for GpuError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NoAdapter(why) => write!(f, "no GPU adapter: {why}"),
            Self::NoDevice(why) => write!(f, "no GPU device: {why}"),
            Self::Lost(why) => write!(f, "the GPU did not give the image back: {why}"),
            Self::Refused(why) => write!(f, "the GPU refused the work: {why}"),
            Self::Size(width, height) => write!(
                f,
                "the display image is not of the size of the image it encodes, {width}x{height}"
            ),
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

/// Every kind of error a wgpu error scope catches: each scope catches one.
pub const ERROR_FILTERS: [wgpu::ErrorFilter; 3] = [
    wgpu::ErrorFilter::Validation,
    wgpu::ErrorFilter::OutOfMemory,
    wgpu::ErrorFilter::Internal,
];

/// Runs `work` on `device` within a scope of every kind of error ([`ERROR_FILTERS`]), and
/// returns the first error caught, if any: without them, wgpu's default handler panics on
/// any refused operation. Not generic, so that its one body is the one its tests cover.
fn scoped(device: &wgpu::Device, work: &mut dyn FnMut()) -> Option<String> {
    let scopes = ERROR_FILTERS.map(|filter| device.push_error_scope(filter));
    work();
    let mut caught = None;
    block_on(pin!(async {
        // Popped in the reverse order of their pushes, as wgpu requires.
        for scope in scopes.into_iter().rev() {
            if let Some(error) = scope.pop().await {
                let _first = caught.get_or_insert_with(|| error.to_string());
            }
        }
    }));
    caught
}

/// The ticks a pass took by its two timestamps, as the GPU resolved them (`stamps`, each a
/// little-endian 64-bit word): none when the GPU wrote neither, both then 0. Metal leaves
/// both unwritten for about one render in twelve (measured 2026-11-19 on an Apple M2 Pro),
/// which is an unmeasured pass, never one of 0 ticks.
fn pass_ticks(stamps: &[u8]) -> Option<u64> {
    let mut words = stamps
        .as_chunks::<8>()
        .0
        .iter()
        .map(|word| u64::from_le_bytes(*word));
    let start = words.next().unwrap_or_default();
    let end = words.next().unwrap_or_default();
    (start != 0 || end != 0).then(|| end.saturating_sub(start))
}

/// The error of what [`scoped`] caught, if anything.
fn refusal(caught: Option<String>) -> Result<(), GpuError> {
    caught.map_or(Ok(()), |why| Err(GpuError::Refused(why)))
}

/// The rows of each band of an image of `size` when a band holds at most `band_bytes`.
///
/// As many rows of pixels as fit, at least one (a row of the widest image, 256 KiB, is
/// below the smallest binding size WebGPU guarantees, 128 MiB), at most the image's.
fn band_rows(size: ImageSize, band_bytes: u64) -> u32 {
    let row_bytes = u64::from(size.width()).saturating_mul(PIXEL_BYTES);
    u32::try_from(band_bytes.checked_div(row_bytes).unwrap_or(0))
        .unwrap_or(u32::MAX)
        .clamp(1, size.height())
}

/// The pixels of each band of encoding an image of `pixels` pixels for display, when a band
/// holds at most `band_bytes` bytes of values: as many as fit, at least one, at most the
/// image's.
fn display_band(pixels: u64, band_bytes: u64) -> u64 {
    (band_bytes / PIXEL_BYTES).clamp(1, pixels.max(1))
}

/// What rendering one image costs the GPU, in exact counts.
///
/// The same on every GPU whose buffers allow the same bands: exact counts, which the benchmarks hold to a baseline on every machine
/// (`spec/benchmarks.md`, "Visual library"), where the GPU's time depends on the GPU.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RenderCost {
    /// The bands of rows the image is rendered in, each one submission to the GPU and one
    /// read back.
    pub bands: u32,
    /// The workgroups of 8 × 8 invocations dispatched, over every band.
    pub workgroups: u64,
    /// The bytes written to the GPU: the placement of the square, once per band.
    pub uploaded: u64,
    /// The bytes read back from the GPU: the image's values.
    pub read_back: u64,
}

/// What rendering an image of `size` costs when a band holds at most `band_bytes` bytes
/// of pixels ([`RenderCost`]).
#[must_use]
pub fn render_cost(size: ImageSize, band_bytes: u64) -> RenderCost {
    let rows = band_rows(size, band_bytes);
    let bands = size.height().div_ceil(rows);
    let full = bands.saturating_sub(1);
    let last = size.height().saturating_sub(full.saturating_mul(rows));
    let columns = u64::from(size.width().div_ceil(WORKGROUP));
    let groups = |band_rows: u32| columns.saturating_mul(u64::from(band_rows.div_ceil(WORKGROUP)));
    RenderCost {
        bands,
        workgroups: groups(rows)
            .saturating_mul(u64::from(full))
            .saturating_add(groups(last)),
        uploaded: u64::from(bands)
            .saturating_mul(u64::try_from(PLACEMENT_BYTES).unwrap_or(u64::MAX)),
        read_back: size.pixels().saturating_mul(PIXEL_BYTES),
    }
}

/// The buffers a band is rendered with ([`GpuSquare::band_buffers`]).
struct BandBuffers {
    /// The square's placement, the shader's uniform.
    uniform: wgpu::Buffer,
    /// The band's values, as the shader writes them.
    storage: wgpu::Buffer,
    /// The band's values then its pass's timestamps, read back.
    readback: wgpu::Buffer,
    /// The pass's timestamps, as the GPU resolves them.
    stamps: wgpu::Buffer,
    /// The binding of the uniform and the values to the shader.
    bind_group: wgpu::BindGroup,
}

/// The nanoseconds of `ticks` of a clock whose tick lasts `period` nanoseconds. Ticks
/// below 2⁵³ convert exactly; a sum beyond is days of work.
fn nanoseconds(ticks: u64, period: f32) -> f64 {
    exact_f64(ticks) * f64::from(period)
}

/// `ticks` as a float, exactly up to 2⁵³, nearest beyond (no `as` cast: the bits are laid
/// out by halves, each exact).
fn exact_f64(ticks: u64) -> f64 {
    let high = u32::try_from(ticks >> 32_u32).unwrap_or(u32::MAX);
    let low = u32::try_from(ticks & 0xffff_ffff).unwrap_or(u32::MAX);
    f64::from(high) * 4_294_967_296.0 + f64::from(low)
}

/// The square's shader, compiled for one GPU.
#[derive(Debug)]
pub struct GpuSquare {
    /// The device.
    device: wgpu::Device,
    /// Its queue.
    queue: wgpu::Queue,
    /// The compute pipeline of [`SQUARE_WGSL`].
    pipeline: wgpu::ComputePipeline,
    /// The compute pipeline of [`DISPLAY_WGSL`].
    display: wgpu::ComputePipeline,
    /// The display shader's thresholds ([`threshold_bytes`]), written once.
    thresholds: wgpu::Buffer,
    /// The most workgroups of one dimension of a dispatch.
    max_groups: u32,
    /// The most bytes of pixels one band may hold: what one storage buffer, bound whole, and
    /// one buffer to read back can hold on this device.
    band_bytes: u64,
    /// The GPU's name and API, for reports.
    adapter: String,
    /// The GPU's clock, when it can time its own work: two timestamps a pass, and the
    /// nanoseconds of one of its ticks.
    timer: Option<(wgpu::QuerySet, f32)>,
}

/// The bytes of the two timestamps of a pass, as the GPU resolves them.
const STAMP_BYTES: u64 = 16;

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
        Self::open(backends, |limits| limits, true)
    }

    /// [`GpuSquare::new`], asking the device for `limits` applied to the adapter's own, and
    /// for its clock if `with_clock` and the adapter has one. A function pointer, not a
    /// generic, so that the one body the tests cover is the one shipped.
    fn open(
        backends: wgpu::Backends,
        limits: fn(wgpu::Limits) -> wgpu::Limits,
        with_clock: bool,
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
        let required_features = if with_clock {
            adapter
                .features()
                .intersection(wgpu::Features::TIMESTAMP_QUERY)
        } else {
            wgpu::Features::empty()
        };
        // Replaced by the answer, as the adapter's.
        let mut device = Err(GpuError::NoDevice(String::new()));
        block_on(pin!(async {
            device = adapter
                .request_device(&wgpu::DeviceDescriptor {
                    label: Some("square"),
                    required_features,
                    required_limits: required_limits.clone(),
                    ..wgpu::DeviceDescriptor::default()
                })
                .await
                .map_err(|error| GpuError::NoDevice(error.to_string()));
        }));
        let (device, queue) = device?;
        let info = adapter.get_info();
        // Every resource within error scopes: a refusal (a limit below a buffer, a shader
        // the back end cannot translate) is an error, never wgpu's default handler's panic.
        let mut built = None;
        let refused = scoped(&device, &mut || {
            let pipeline = Self::pipeline(&device, "square", SQUARE_WGSL);
            let display = Self::pipeline(&device, "display", DISPLAY_WGSL);
            let table = threshold_bytes();
            let thresholds = device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("thresholds"),
                size: u64::try_from(table.len()).unwrap_or(u64::MAX),
                usage: wgpu::BufferUsages::STORAGE.union(wgpu::BufferUsages::COPY_DST),
                mapped_at_creation: false,
            });
            queue.write_buffer(&thresholds, 0, &table);
            let timer = required_features
                .contains(wgpu::Features::TIMESTAMP_QUERY)
                .then(|| {
                    let set = device.create_query_set(&wgpu::QuerySetDescriptor {
                        label: Some("square"),
                        ty: wgpu::QueryType::Timestamp,
                        count: 2,
                    });
                    (set, queue.get_timestamp_period())
                });
            built = Some((pipeline, display, thresholds, timer));
        });
        let (pipeline, display, thresholds, timer) = match (refused, built) {
            (None, Some(built)) => built,
            (refused, _) => return Err(GpuError::Refused(refused.unwrap_or_default())),
        };
        Ok(Self {
            device,
            queue,
            pipeline,
            display,
            thresholds,
            max_groups: required_limits.max_compute_workgroups_per_dimension.max(1),
            band_bytes: required_limits
                .max_storage_buffer_binding_size
                .min(required_limits.max_buffer_size),
            adapter: format!("{} ({})", info.name, info.backend),
            timer,
        })
    }

    /// The compute pipeline of the WGSL `source`'s `main`, compiled on `device`.
    fn pipeline(device: &wgpu::Device, label: &str, source: &str) -> wgpu::ComputePipeline {
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some(label),
            source: wgpu::ShaderSource::Wgsl(source.into()),
        });
        device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some(label),
            layout: None,
            module: &module,
            entry_point: Some("main"),
            compilation_options: wgpu::PipelineCompilationOptions::default(),
            cache: None,
        })
    }

    /// Encodes `image` for display into `display` on the GPU: exactly the bytes the CPU
    /// encoding gives ([`DisplayImage::encode`], every pixel by [`visual_core::pixel8`]),
    /// whatever the values (not a number, infinite, subnormal or negative included).
    ///
    /// # Errors
    ///
    /// Returns [`GpuError::Size`] when `display` is not of `image`'s size, and
    /// [`GpuError::Lost`] when the device does not give the bytes back; `display` then holds
    /// the bands encoded before.
    pub fn encode(&self, image: &Image, display: &mut DisplayImage) -> Result<(), GpuError> {
        let size = image.size();
        if display.size() != size {
            return Err(GpuError::Size(size.width(), size.height()));
        }
        let mut encoded = Ok(());
        let refused = scoped(&self.device, &mut || {
            encoded = self.encode_bands(image, display);
        });
        // A lost device also fails the operations recorded after it: its own failure, the
        // bytes not given back, says why first.
        encoded?;
        refusal(refused)
    }

    /// [`GpuSquare::encode`], within its error scopes: bands of as many pixels as one
    /// storage buffer holds of their values.
    fn encode_bands(&self, image: &Image, display: &mut DisplayImage) -> Result<(), GpuError> {
        let pixels_per_band = display_band(image.size().pixels(), self.band_bytes);
        let band_bytes = pixels_per_band.saturating_mul(PIXEL_BYTES);
        let out_bytes = pixels_per_band.saturating_mul(DisplayImage::PIXEL_BYTES);
        let params = self.buffer(
            "params",
            u64::try_from(PARAMS_BYTES).unwrap_or(u64::MAX),
            wgpu::BufferUsages::UNIFORM.union(wgpu::BufferUsages::COPY_DST),
        );
        let values = self.buffer(
            "values",
            band_bytes,
            wgpu::BufferUsages::STORAGE.union(wgpu::BufferUsages::COPY_DST),
        );
        let pixels = self.buffer(
            "pixels",
            out_bytes,
            wgpu::BufferUsages::STORAGE.union(wgpu::BufferUsages::COPY_SRC),
        );
        let readback = self.buffer(
            "readback",
            out_bytes,
            wgpu::BufferUsages::MAP_READ.union(wgpu::BufferUsages::COPY_DST),
        );
        let bind_group = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("display"),
            layout: &self.display.get_bind_group_layout(0),
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: params.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: values.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: self.thresholds.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: pixels.as_entire_binding(),
                },
            ],
        });
        let channels = usize::try_from(pixels_per_band)
            .unwrap_or(usize::MAX)
            .saturating_mul(Image::CHANNELS);
        for (input, output) in image
            .values()
            .chunks(channels)
            .zip(display.bytes_mut().chunks_mut(channels))
        {
            let count =
                u32::try_from(input.as_chunks::<{ Image::CHANNELS }>().0.len()).unwrap_or(u32::MAX);
            let groups = count.div_ceil(DISPLAY_WORKGROUP);
            let columns = groups.min(self.max_groups);
            let rows = groups.div_ceil(columns.max(1));
            let mut uniform_bytes = [0_u8; PARAMS_BYTES];
            let fields = [count, columns.saturating_mul(DISPLAY_WORKGROUP), 0, 0];
            for (slot, field) in uniform_bytes.as_chunks_mut::<4>().0.iter_mut().zip(fields) {
                *slot = field.to_le_bytes();
            }
            self.queue.write_buffer(&params, 0, &uniform_bytes);
            let input_bytes: Vec<u8> = input
                .iter()
                .flat_map(|value| value.to_bits().to_le_bytes())
                .collect();
            self.queue.write_buffer(&values, 0, &input_bytes);
            let mut encoder = self
                .device
                .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                    label: Some("display"),
                });
            {
                let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                    label: Some("display"),
                    timestamp_writes: None,
                });
                pass.set_pipeline(&self.display);
                pass.set_bind_group(0, &bind_group, &[]);
                pass.dispatch_workgroups(columns, rows, 1);
            }
            let bytes = u64::try_from(output.len()).unwrap_or(u64::MAX);
            encoder.copy_buffer_to_buffer(&pixels, 0, &readback, 0, bytes);
            let _submitted = self.queue.submit([encoder.finish()]);
            self.read_bytes(&readback, output)
                .map_err(|error| GpuError::Lost(error.to_string()))?;
        }
        Ok(())
    }

    /// Waits for the GPU, then reads the first bytes of `readback` into `output`.
    ///
    /// # Errors
    ///
    /// Returns why the bytes could not be read, as [`GpuSquare::read_back`].
    fn read_bytes(
        &self,
        readback: &wgpu::Buffer,
        output: &mut [u8],
    ) -> Result<(), Box<dyn core::error::Error>> {
        let bytes = u64::try_from(output.len())?;
        let (sender, receiver) = mpsc::channel();
        readback
            .slice(..bytes)
            .map_async(wgpu::MapMode::Read, move |outcome| {
                drop(sender.send(outcome));
            });
        let _polled = self.device.poll(wgpu::PollType::wait_indefinitely())?;
        receiver.try_recv()??;
        let mapped = readback.slice(..bytes).get_mapped_range()?;
        output.copy_from_slice(&mapped);
        drop(mapped);
        readback.unmap();
        Ok(())
    }

    /// What rendering an image of `size` costs on this GPU, in transfers and work
    /// ([`render_cost`] at the bands its buffers allow).
    #[must_use]
    pub fn cost(&self, size: ImageSize) -> RenderCost {
        render_cost(size, self.band_bytes)
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
        self.timed_render(square, index, output).map(drop)
    }

    /// [`GpuSquare::render`], and the time the GPU's work took by its own clock, in
    /// nanoseconds: the sum of its passes', from the first instruction of each to its
    /// last, so that what the processor does meanwhile does not count. `None` on a GPU
    /// without a clock its programs can read (`wgpu::Features::TIMESTAMP_QUERY`), and when
    /// the GPU wrote no timestamp of a pass (an unmeasured pass, never one of 0 ticks).
    ///
    /// # Errors
    ///
    /// As [`GpuSquare::render`].
    pub fn timed_render(
        &self,
        square: &Square,
        index: ImageIndex,
        output: &mut Image,
    ) -> Result<Option<f64>, GpuError> {
        let mut rendered = Ok(None);
        let refused = scoped(&self.device, &mut || {
            rendered = self.render_bands(square, index, output);
        });
        // As in `encode`: a lost device's own failure first.
        let took = rendered?;
        refusal(refused)?;
        Ok(took)
    }

    /// [`GpuSquare::timed_render`], within its error scopes.
    fn render_bands(
        &self,
        square: &Square,
        index: ImageIndex,
        output: &mut Image,
    ) -> Result<Option<f64>, GpuError> {
        let size = output.size();
        let placed = square.placement(index, size);
        let row_bytes = u64::from(size.width()).saturating_mul(PIXEL_BYTES);
        let band_rows = band_rows(size, self.band_bytes);
        let band_bytes = row_bytes.saturating_mul(u64::from(band_rows));
        let BandBuffers {
            uniform,
            storage,
            readback,
            stamps,
            bind_group,
        } = self.band_buffers(band_bytes);
        let rows_per_band = usize::try_from(band_rows).unwrap_or(usize::MAX);
        let mut first_row = 0_u32;
        // Unmeasured once any pass is.
        let mut ticks = Some(0_u64);
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
                    timestamp_writes: self.timer.as_ref().map(|(set, _)| {
                        wgpu::ComputePassTimestampWrites {
                            query_set: set,
                            beginning_of_pass_write_index: Some(0),
                            end_of_pass_write_index: Some(1),
                        }
                    }),
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
            if let Some((set, _)) = &self.timer {
                encoder.resolve_query_set(set, 0..2, &stamps, 0);
                encoder.copy_buffer_to_buffer(&stamps, 0, &readback, bytes, STAMP_BYTES);
            }
            let _submitted = self.queue.submit([encoder.finish()]);
            let pass_ticks = self
                .read_back(&readback, bytes, band)
                .map_err(|error| GpuError::Lost(error.to_string()))?;
            ticks = ticks
                .zip(pass_ticks)
                .map(|(sum, pass)| sum.saturating_add(pass));
            first_row = first_row.saturating_add(rows);
        }
        Ok(self
            .timer
            .as_ref()
            .zip(ticks)
            .map(|(&(_, period), ticks)| nanoseconds(ticks, period)))
    }

    /// The buffers of rendering bands of `band_bytes` bytes of pixels, and their binding.
    fn band_buffers(&self, band_bytes: u64) -> BandBuffers {
        let uniform = self.buffer(
            "placement",
            u64::try_from(PLACEMENT_BYTES).unwrap_or(u64::MAX),
            wgpu::BufferUsages::UNIFORM.union(wgpu::BufferUsages::COPY_DST),
        );
        let storage = self.buffer(
            "values",
            band_bytes,
            wgpu::BufferUsages::STORAGE.union(wgpu::BufferUsages::COPY_SRC),
        );
        // A band's values, then the timestamps of its pass when the GPU has a clock.
        let readback = self.buffer(
            "readback",
            band_bytes.saturating_add(STAMP_BYTES),
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
        let stamps = self.buffer(
            "timestamps",
            STAMP_BYTES,
            wgpu::BufferUsages::QUERY_RESOLVE.union(wgpu::BufferUsages::COPY_SRC),
        );
        BandBuffers {
            uniform,
            storage,
            readback,
            stamps,
            bind_group,
        }
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

    /// Waits for the GPU, then reads the first `bytes` of `readback` into the rows of `band`,
    /// and returns the ticks its pass took by the timestamps that follow them
    /// ([`pass_ticks`]: none on a GPU without a clock, which wrote none).
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
    ) -> Result<Option<u64>, Box<dyn core::error::Error>> {
        let mapped_bytes = bytes.saturating_add(STAMP_BYTES);
        let (sender, receiver) = mpsc::channel();
        readback
            .slice(..mapped_bytes)
            .map_async(wgpu::MapMode::Read, move |outcome| {
                drop(sender.send(outcome));
            });
        let _polled = self.device.poll(wgpu::PollType::wait_indefinitely())?;
        receiver.try_recv()??;
        let mapped = readback.slice(..mapped_bytes).get_mapped_range()?;
        let (values, stamps) = mapped.split_at(usize::try_from(bytes)?);
        for (value, word) in band
            .iter_mut()
            .flat_map(|row| row.iter_mut())
            .zip(values.as_chunks::<4>().0)
        {
            *value = f32::from_le_bytes(*word);
        }
        let ticks = pass_ticks(stamps);
        drop(mapped);
        readback.unmap();
        Ok(ticks)
    }
}
