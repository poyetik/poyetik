//! The visual library on the GPU, through `wgpu` (Metal, Vulkan or DirectX 12 natively): the
//! square rendered by its WGSL shader ([`SQUARE_WGSL`]), bit for bit the image of the CPU
//! reference renderer, and any image encoded for display by the library's display shader
//! ([`DISPLAY_WGSL`]), byte for byte the CPU's display encoding.
//!
//! Where the square is comes from the CPU ([`Square::placement`], exact, 32-bit edges); the
//! shader only fills the pixels, in integers, then converts each coverage to a float without
//! rounding, so that every GPU following WGSL's rules gives the CPU reference's bits. The
//! same shader runs in browsers through WebGPU. An image larger than the GPU's buffers is
//! rendered in bands of rows, each band the same pixels as the whole image would hold, and
//! encoded in bands of pixels.
//!
//! It creates its buffers and bind groups for every image, where a live host keeps them
//! from one image to the next: whether a GPU renderer keeps them is the resources and GPU
//! memory decision of roadmap #35 step 2, and its measured time includes that work.
//!
//! The crate is not published: a tool of the repository, which its binary runs as a step of
//! every check. [`bench`](mod@bench) is that command, public for its binary and documented as
//! the rest: it checks the GPU's time only on a GPU its baseline holds, on the system and
//! driver it records, and renders nothing on any other.

#![forbid(unsafe_code)]
#![doc(test(attr(forbid(unsafe_code))))]
// A public enum that may gain a variant is `#[non_exhaustive]`, so that adding one breaks no
// caller; one that is closed by its decision says why where it is declared.
#![deny(clippy::exhaustive_enums)]
#![deny(clippy::exhaustive_structs)]
#![expect(
    clippy::multiple_crate_versions,
    reason = "wgpu's own tree holds two versions of hashbrown (naga's indexmap, gpu-allocator) \
              and of syn (bytemuck's and zerocopy's derives), which this crate cannot choose; \
              each pair is a reviewed exception of deny.toml"
)]

extern crate alloc;

use alloc::boxed::Box;
use alloc::sync::Arc;
use core::fmt;
use core::future::Future;
use core::pin::{Pin, pin};
use core::sync::atomic::{AtomicBool, Ordering};
use core::task::{Context, Waker};
use std::sync::mpsc;

use visual_boxes::SQUARE_WGSL;
use visual_boxes::Square;
use visual_core::{
    DISPLAY_THRESHOLD_BITS, DISPLAY_WGSL, DisplayImage, Image, ImageIndex, ImageSize,
};

pub mod bench;
mod safe;

#[cfg(test)]
#[path = "lib_tests.rs"]
mod tests;

/// Pixels per side of a workgroup of the square's shader (`@workgroup_size(8, 8)`).
const SQUARE_WORKGROUP: u32 = 8;

/// Bytes of the square's shader's `Placement` uniform: seven 32-bit integers, padded to 32.
const PLACEMENT_BYTES: usize = 32;

/// Invocations of a workgroup of the display shader (`@workgroup_size(64)`).
const DISPLAY_WORKGROUP: u32 = 64;

/// Bytes of the display shader's `Params` uniform: two 32-bit integers, padded to 16.
const PARAMS_BYTES: usize = 16;

/// The bytes of the two timestamps of a pass, as the GPU resolves them.
const STAMP_BYTES: u64 = 16;

/// The display shader's thresholds as bytes: each of [`DISPLAY_THRESHOLD_BITS`], little-endian.
fn threshold_bytes() -> Vec<u8> {
    DISPLAY_THRESHOLD_BITS
        .iter()
        .flat_map(|bits| bits.to_le_bytes())
        .collect()
}

/// Why the GPU could not render.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum GpuError {
    /// No GPU adapter answered: none, none of the APIs asked for, or the request did not
    /// answer at once (wgpu's native futures always answer at once).
    NoAdapter(String),
    /// The adapter refused to open a device, or did not answer the request at once (wgpu's
    /// native futures always answer at once).
    NoDevice(String),
    /// The device was lost (destroyed, or by its driver): while opening, or before giving
    /// its work back, the rendered pixels or the encoded bytes. An error the scopes catch
    /// on a lost device is a loss, never a refusal ([`GpuError::Refused`]).
    Lost(String),
    /// The GPU refused the work: a limit below what it asks, memory it cannot hold, or a
    /// shader its back end cannot translate (wgpu's error scopes, [`ERROR_FILTERS`]); or its
    /// error scopes did not answer at once (wgpu's native futures always answer at once), the
    /// work then taken as refused: on an intact device, a lost one's being [`GpuError::Lost`].
    Refused(String),
    /// The rendered pixels or the encoded bytes could not be read back, on a device wgpu does
    /// not say is lost and with nothing caught: the wait, the mapping or its answer failed.
    NotRead(String),
    /// A display image was given an image of another size: the image's (as the CPU's
    /// [`visual_core::VisualError::DisplaySizeMismatch`] names it).
    DisplaySizeMismatch {
        /// The image's width, in pixels.
        width: u32,
        /// The image's height, in pixels.
        height: u32,
    },
}

impl fmt::Display for GpuError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NoAdapter(why) => write!(f, "no GPU adapter: {why}"),
            Self::NoDevice(why) => write!(f, "no GPU device: {why}"),
            Self::Lost(why) => write!(f, "the GPU device was lost: {why}"),
            Self::Refused(why) => write!(f, "the GPU refused the work: {why}"),
            Self::NotRead(why) => write!(f, "the GPU's work could not be read back: {why}"),
            Self::DisplaySizeMismatch { width, height } => write!(
                f,
                "the display image is not of the size of the image it encodes, {width}x{height}"
            ),
        }
    }
}

impl core::error::Error for GpuError {}

/// The driver of a GPU as wgpu gives it: its `driver` then its `driver_info` (its version),
/// each trimmed and joined by a space, and `-` when both are empty, as Metal leaves them
/// (its driver is the system's, which the GPU time's baseline records apart).
fn driver_text(driver: &str, info: &str) -> String {
    let joined = [driver.trim(), info.trim()]
        .into_iter()
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join(" ");
    if joined.is_empty() {
        "-".to_owned()
    } else {
        joined
    }
}

/// Polls `future` once, on this thread: wgpu's native futures are ready at their first poll.
///
/// A future still pending is left unfinished, never polled again with no waker to wait for:
/// its caller sets its answer to an error before, which only the finished future replaces
/// ([`NOT_ANSWERED`]). Not generic, so that its one instantiation is the one its tests
/// cover: a caller wraps its future in an `async` block that stores the output.
fn block_on(mut future: Pin<&mut dyn Future<Output = ()>>) {
    let mut context = Context::from_waker(Waker::noop());
    let _polled = future.as_mut().poll(&mut context);
}

/// The answer of a GPU request whose future was still pending at its one poll
/// ([`block_on`]): wgpu's native futures never are.
const NOT_ANSWERED: &str = "the GPU did not answer at once";

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
    // Replaced by the scopes' answer once every scope is popped: unanswered, a refusal.
    let mut caught = Some(NOT_ANSWERED.to_owned());
    block_on(pin!(async {
        let mut first = None;
        // Popped in the reverse order of their pushes, as wgpu requires.
        for scope in scopes.into_iter().rev() {
            if let Some(error) = scope.pop().await {
                let _first = first.get_or_insert_with(|| error.to_string());
            }
        }
        caught = first;
    }));
    caught
}

/// The ticks a pass took by its two timestamps, as the GPU resolved them (`stamps`, each a
/// little-endian 64-bit word): measured only when both are written, neither is the error
/// value, and the end is after the start. An unwritten timestamp reads 0: each pass writes
/// to a query set created for it alone and clears the buffer it resolves into first
/// ([`GpuSquare::run_pass`]), so that none holds another pass's, of the band before
/// included, whatever a back end leaves in a resolve's destination for a timestamp never
/// written (wgpu does not say). Metal leaves both unwritten for some passes (seen 2026-11-19
/// on an Apple M2 Pro, on renders of one band, into a fresh buffer; how many renders of each
/// size go unmeasured, `spec/gpu-time-runs.txt`), and writes all ones, its `MTLCounterErrorValue`, for a sample
/// it failed to take. A half-written, failed or reversed pair, or an end at its start
/// (0 ticks, which no pass takes: the shortest last microseconds, dozens of ticks even of
/// Metal's, whose times fall on multiples of about 41.7 ns, 24 MHz, though it reports a
/// period of 1 ns), is an unmeasured pass, never a time it did not take (a timestamp's own
/// count from its clock's origin for a half-written pair, about 1.8e19 ticks, the error
/// value less a timestamp, for a failed one, or 0).
fn pass_ticks(stamps: &[u8]) -> Option<u64> {
    let mut words = stamps
        .as_chunks::<8>()
        .0
        .iter()
        .map(|word| u64::from_le_bytes(*word))
        .filter(|&word| word != 0 && word != u64::MAX);
    let start = words.next()?;
    let end = words.next()?;
    end.checked_sub(start).filter(|&ticks| ticks != 0)
}

/// Writes `fields` into `uniform` as the shaders read a uniform of 32-bit integers: each
/// little-endian, in order (the one packing of both passes' uniforms).
fn pack(fields: &[u32], uniform: &mut [u8]) {
    for (slot, field) in uniform.as_chunks_mut::<4>().0.iter_mut().zip(fields) {
        *slot = field.to_le_bytes();
    }
}

/// The outcome of work `done` within error scopes that caught `caught`, on a device `lost`
/// or not: on a lost device a loss ([`GpuError::Lost`], made nowhere else), its work's own
/// failure first (it also fails the operations recorded after it, which the scopes catch),
/// then what the scopes caught, whatever the work's outcome, since the loss may be what they
/// caught; on an intact one a refusal of the work whatever step then failed (a readback
/// that cannot be mapped because its buffer was refused); then the work's own outcome
/// ([`GpuError::NotRead`], the only error the work has). The one classification of opening,
/// rendering and encoding.
fn classified<T>(
    done: Result<T, GpuError>,
    caught: Option<String>,
    lost: bool,
) -> Result<T, GpuError> {
    match (done, caught) {
        (Err(GpuError::NotRead(why)), _) | (_, Some(why)) if lost => Err(GpuError::Lost(why)),
        (_, Some(why)) => Err(GpuError::Refused(why)),
        (done, None) => done,
    }
}

/// The rows of each band of an image of `size` when a band holds at most `band_bytes`.
///
/// As many rows of pixels as fit, at least one (a row of the widest image, 256 KiB, is
/// below the smallest binding size WebGPU guarantees, 128 MiB), at most the image's.
fn band_rows(size: ImageSize, band_bytes: u64) -> u32 {
    let row_bytes = u64::from(size.width()).saturating_mul(Image::PIXEL_BYTES);
    u32::try_from(band_bytes.checked_div(row_bytes).unwrap_or(0))
        .unwrap_or(u32::MAX)
        .clamp(1, size.height())
}

/// The pixels of each band of encoding an image of `pixels` pixels for display, when a band
/// holds at most `band_bytes` bytes of values: as many as fit, at least one, at most the
/// image's.
fn display_band(pixels: u64, band_bytes: u64) -> u64 {
    (band_bytes / Image::PIXEL_BYTES).clamp(1, pixels.max(1))
}

/// What rendering one image costs the GPU, in exact counts.
///
/// The same on every GPU whose buffers allow the same bands: exact counts, which the benchmarks
/// hold to a baseline on every machine (`spec/benchmarks.md`, "Visual library"), where the
/// GPU's time depends on the GPU.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub struct RenderCost {
    /// The bands of rows the image is rendered in, each one submission to the GPU and one
    /// read back.
    pub bands: u32,
    /// The workgroups of 8 × 8 invocations dispatched, over every band.
    pub workgroups: u64,
    /// The bytes written to the GPU: the placement of the square, once per band.
    pub uploaded: u64,
    /// The bytes read back from the GPU: the image's values, and each band's two
    /// timestamps (16 bytes, read whether the GPU wrote them or not).
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
    let columns = u64::from(size.width().div_ceil(SQUARE_WORKGROUP));
    let groups =
        |band_rows: u32| columns.saturating_mul(u64::from(band_rows.div_ceil(SQUARE_WORKGROUP)));
    RenderCost {
        bands,
        workgroups: groups(rows)
            .saturating_mul(u64::from(full))
            .saturating_add(groups(last)),
        uploaded: u64::from(bands).saturating_mul(byte_count(PLACEMENT_BYTES)),
        read_back: size
            .pixels()
            .saturating_mul(Image::PIXEL_BYTES)
            .saturating_add(u64::from(bands).saturating_mul(STAMP_BYTES)),
    }
}

/// The display shader's workgroups for a band of `pixels` pixels when a dispatch holds at
/// most `max_groups` workgroups a dimension: columns, then rows. As many columns as the
/// limit allows, at least one, and as many rows as the groups need, the last padded (the
/// shader skips the invocations past the band). The one layout both the encoding and its
/// cost use.
///
/// The rows fit the limit too while the groups are at most its square: 65,535² on every
/// WebGPU device, where an image's largest band, 2²⁵ pixels, holds 2¹⁹ groups (9 rows).
fn display_grid(pixels: u32, max_groups: u32) -> (u32, u32) {
    let groups = pixels.div_ceil(DISPLAY_WORKGROUP);
    let columns = groups.min(max_groups).max(1);
    (columns, groups.div_ceil(columns))
}

/// What encoding one image for display costs the GPU, in exact counts: the same on every
/// GPU whose buffers and dispatch allow the same bands (`spec/benchmarks.md`, "Visual
/// library").
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub struct EncodeCost {
    /// The bands of pixels the image is encoded in, each one submission to the GPU and one
    /// read back.
    pub bands: u32,
    /// The workgroups of 64 invocations dispatched, over every band (a band's groups laid
    /// out in rows of at most the GPU's limit, the last row padded).
    pub workgroups: u64,
    /// The bytes written to the GPU: the image's values, and each band's parameters.
    pub uploaded: u64,
    /// The bytes read back from the GPU: four a pixel, and each band's two timestamps (16
    /// bytes, read whether the GPU wrote them or not).
    pub read_back: u64,
}

/// What encoding an image of `size` for display costs when a band holds at most
/// `band_bytes` bytes of values and a dispatch at most `max_groups` workgroups a dimension
/// ([`EncodeCost`]).
#[must_use]
pub fn encode_cost(size: ImageSize, band_bytes: u64, max_groups: u32) -> EncodeCost {
    let pixels = size.pixels();
    let per_band = display_band(pixels, band_bytes);
    let bands = pixels.div_ceil(per_band);
    let last = pixels.saturating_sub(bands.saturating_sub(1).saturating_mul(per_band));
    // A band holds at most an image's pixels, 2²⁵: far below `u32::MAX`.
    let dispatched = |count: u64| {
        let (columns, rows) = display_grid(u32::try_from(count).unwrap_or(u32::MAX), max_groups);
        u64::from(columns).saturating_mul(u64::from(rows))
    };
    let band_extra = |per: u64| bands.saturating_mul(per);
    EncodeCost {
        bands: u32::try_from(bands).unwrap_or(u32::MAX),
        workgroups: dispatched(per_band)
            .saturating_mul(bands.saturating_sub(1))
            .saturating_add(dispatched(last)),
        uploaded: pixels
            .saturating_mul(Image::PIXEL_BYTES)
            .saturating_add(band_extra(byte_count(PARAMS_BYTES))),
        read_back: pixels
            .saturating_mul(DisplayImage::PIXEL_BYTES)
            .saturating_add(band_extra(STAMP_BYTES)),
    }
}

/// The buffers a band is encoded for display with ([`GpuSquare::encode_buffers`]).
struct EncodeBuffers {
    /// The pixels to encode and the dispatch's row, the shader's uniform.
    params: wgpu::Buffer,
    /// A band's values.
    values: wgpu::Buffer,
    /// A band's encoded bytes, written by the shader.
    pixels: wgpu::Buffer,
    /// The shader's binding of them, and of the thresholds.
    bind_group: wgpu::BindGroup,
}

/// The buffers a band is rendered with ([`GpuSquare::band_buffers`]).
struct BandBuffers {
    /// The square's placement, the shader's uniform.
    uniform: wgpu::Buffer,
    /// The band's values, as the shader writes them.
    storage: wgpu::Buffer,
    /// The binding of the uniform and the values to the shader.
    bind_group: wgpu::BindGroup,
}

/// The buffers every pass is read back through ([`GpuSquare::readback_buffers`]).
struct Readback {
    /// A band's bytes, then its pass's timestamps, read back.
    buffer: wgpu::Buffer,
    /// The pass's timestamps, as the GPU resolves them, cleared before each resolve.
    stamps: wgpu::Buffer,
}

/// One pass of a shader over a band, as both shaders run ([`GpuSquare::run_pass`]).
struct Pass<'pass> {
    /// Its name, in wgpu's reports.
    label: &'static str,
    /// The shader's pipeline.
    pipeline: &'pass wgpu::ComputePipeline,
    /// The shader's binding.
    bind_group: &'pass wgpu::BindGroup,
    /// The workgroups dispatched: columns, then rows.
    groups: (u32, u32),
    /// The buffer the shader writes, whose first [`Pass::bytes`] are read back.
    written: &'pass wgpu::Buffer,
    /// The bytes of the band read back.
    bytes: u64,
}

/// `count` bytes as a count of memory: every count of this crate is far below `u64::MAX`,
/// which a count beyond it (none on any target) would read as.
fn byte_count(count: usize) -> u64 {
    u64::try_from(count).unwrap_or(u64::MAX)
}

/// The ticks of a sum of passes, `sum`, with one pass more, `pass`: unmeasured once any
/// pass is.
fn summed(sum: Option<u64>, pass: Option<u64>) -> Option<u64> {
    sum.zip(pass).map(|(sum, pass)| sum.saturating_add(pass))
}

/// The bound a period must be above to time a pass, in nanoseconds a tick, 2⁻⁶⁵: the most
/// ticks, below 2⁶⁴, times 2⁻⁶⁵ is at most 0.5 ns, which rounds to 0.
const SHORTEST_PERIOD: f32 = f32::from_bits(0x1F00_0000);

/// The bound a period must be below to time a pass, in nanoseconds a tick, 2⁶⁴: one tick of
/// it is past what a `u64` holds.
const LONGEST_PERIOD: f32 = f32::from_bits(0x5F80_0000);

/// The period a driver reports, in nanoseconds a tick, kept only when it can time a pass:
/// above [`SHORTEST_PERIOD`], below [`LONGEST_PERIOD`]. A period of 0, negative, not a number
/// or infinite would make every pass 0 ns or no number, and one outside these bounds every
/// pass a time of 0 ns or past what a `u64` holds, so that no render is ever measured: such a
/// GPU has no clock its programs can read.
fn clock_period(reported: f32) -> Option<f32> {
    (SHORTEST_PERIOD < reported && reported < LONGEST_PERIOD).then_some(reported)
}

/// The nanoseconds of `ticks` of a clock whose tick lasts `period` nanoseconds. Ticks
/// below 2⁵³ convert exactly; a sum beyond is days of work.
fn nanoseconds(ticks: u64, period: f32) -> f64 {
    exact_f64(ticks) * f64::from(period)
}

/// `ticks` as a float, exactly up to 2⁵³, nearest beyond (no `as` cast: the bits are laid
/// out by halves, each exact).
pub(crate) fn exact_f64(ticks: u64) -> f64 {
    let high = u32::try_from(ticks >> 32_u32).unwrap_or(u32::MAX);
    let low = u32::try_from(ticks & 0xffff_ffff).unwrap_or(u32::MAX);
    f64::from(high) * 4_294_967_296.0 + f64::from(low)
}

/// The library's two shaders, the square's and the display encoding's, compiled for one
/// GPU (named after the square, its first: its name beyond it is roadmap #35 step 2's).
#[derive(Debug)]
pub struct GpuSquare {
    /// The device.
    device: wgpu::Device,
    /// Its queue.
    queue: wgpu::Queue,
    /// The compute pipeline of [`SQUARE_WGSL`].
    square_pipeline: wgpu::ComputePipeline,
    /// The compute pipeline of [`DISPLAY_WGSL`].
    display_pipeline: wgpu::ComputePipeline,
    /// The display shader's thresholds ([`threshold_bytes`]), written once.
    thresholds: wgpu::Buffer,
    /// The most workgroups of one dimension of a dispatch.
    max_groups: u32,
    /// The most bytes of pixels one band may hold: what one storage buffer, bound whole, and
    /// one buffer to read back, the band's two timestamps after its values, can hold on this
    /// device.
    band_bytes: u64,
    /// The GPU's name and API, for reports.
    adapter: String,
    /// The GPU's driver as wgpu names it ([`driver_text`]).
    driver: String,
    /// The nanoseconds of one tick of the GPU's clock, when it can time its own work (two
    /// timestamps a pass, in a query set of each pass's own, [`GpuSquare::run_pass`]):
    /// above [`SHORTEST_PERIOD`], below [`LONGEST_PERIOD`] ([`clock_period`]).
    period: Option<f32>,
    /// Whether the device was lost (destroyed, or by its driver), which wgpu says through
    /// its callback: what tells a lost device's failures from refusals of the work.
    lost: Arc<AtomicBool>,
}

impl GpuSquare {
    /// Opens the first GPU of `backends` (all of them: [`wgpu::Backends::all`]) and compiles
    /// both shaders. `backends` is the pinned wgpu's own type: the crate is not published, and
    /// its one caller, its binary, builds on the same wgpu.
    ///
    /// The device is opened with the adapter's own limits, so that the bands are as large
    /// as this GPU allows.
    ///
    /// # Errors
    ///
    /// Returns [`GpuError::NoAdapter`] when no GPU of `backends` answers at once,
    /// [`GpuError::NoDevice`] when it refuses a device or does not answer the request at
    /// once, [`GpuError::Refused`] when the device refuses what opening it creates (a shader
    /// its back end cannot translate, a buffer beyond its limits, memory it cannot hold) or
    /// its error scopes do not answer at once, and [`GpuError::Lost`] when the
    /// device is lost while opening and an error was caught.
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
        // Replaced by the answer once the request has completed ([`block_on`]).
        let mut adapter = Err(GpuError::NoAdapter(NOT_ANSWERED.to_owned()));
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
        let mut device = Err(GpuError::NoDevice(NOT_ANSWERED.to_owned()));
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
        let lost = Arc::new(AtomicBool::new(false));
        let flag = Arc::clone(&lost);
        device.set_device_lost_callback(move |_, _| flag.store(true, Ordering::SeqCst));
        let info = adapter.get_info();
        // Every resource within error scopes: a refusal (a limit below a buffer, a shader
        // the back end cannot translate) is an error, never wgpu's default handler's panic.
        let mut built = None;
        let refused = scoped(&device, &mut || {
            let square_pipeline = Self::pipeline(&device, "square", SQUARE_WGSL);
            let display_pipeline = Self::pipeline(&device, "display", DISPLAY_WGSL);
            let table = threshold_bytes();
            let thresholds = device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("thresholds"),
                size: byte_count(table.len()),
                usage: wgpu::BufferUsages::STORAGE.union(wgpu::BufferUsages::COPY_DST),
                mapped_at_creation: false,
            });
            queue.write_buffer(&thresholds, 0, &table);
            let period = required_features
                .contains(wgpu::Features::TIMESTAMP_QUERY)
                .then(|| queue.get_timestamp_period())
                .and_then(clock_period);
            built = Some((square_pipeline, display_pipeline, thresholds, period));
        });
        // A loss while opening is a loss, a refusal on an intact device a refusal
        // ([`classified`]). Nothing built and nothing caught cannot happen: the work always
        // runs, so it is built unless a refusal was caught. It is an empty refusal, given
        // eagerly, rather than a branch of its own, a line no test can reach; nor are the
        // scopes split around each resource, whose guards must drop in the reverse order of
        // their pushes.
        let (square_pipeline, display_pipeline, thresholds, period) =
            classified(Ok(built), refused, lost.load(Ordering::SeqCst))?
                .ok_or(GpuError::Refused(String::new()))?;
        Ok(Self {
            device,
            queue,
            square_pipeline,
            display_pipeline,
            thresholds,
            max_groups: required_limits.max_compute_workgroups_per_dimension.max(1),
            // A band's readback holds its pass's timestamps after its values.
            band_bytes: required_limits
                .max_storage_buffer_binding_size
                .min(required_limits.max_buffer_size.saturating_sub(STAMP_BYTES)),
            adapter: format!("{} ({})", info.name, info.backend),
            driver: driver_text(&info.driver, &info.driver_info),
            period,
            lost,
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
    /// Returns [`GpuError::DisplaySizeMismatch`] when `display` is not of `image`'s size
    /// (`display` untouched); [`GpuError::Refused`] when the GPU refuses the work on an
    /// intact device (a buffer beyond its limits, memory it cannot hold) or its error
    /// scopes do not answer at once, and
    /// [`GpuError::Lost`] when the device is lost (destroyed, or by its driver) and does not
    /// give the bytes back, or an error was caught on it, and [`GpuError::NotRead`] when an
    /// intact one does not: `display` then holds the bands encoded before, possibly all.
    pub fn encode(&self, image: &Image, display: &mut DisplayImage) -> Result<(), GpuError> {
        self.timed_encode(image, display).map(drop)
    }

    /// [`GpuSquare::encode`], and the time the GPU's work took by its own clock, in
    /// nanoseconds: the sum of its passes', as [`GpuSquare::timed_render`] measures them.
    /// `None` on a GPU without a clock, and when the GPU wrote no timestamp of a pass.
    ///
    /// # Errors
    ///
    /// As [`GpuSquare::encode`].
    pub fn timed_encode(
        &self,
        image: &Image,
        display: &mut DisplayImage,
    ) -> Result<Option<f64>, GpuError> {
        let size = image.size();
        if display.size() != size {
            return Err(GpuError::DisplaySizeMismatch {
                width: size.width(),
                height: size.height(),
            });
        }
        let mut encoded = Ok(None);
        let refused = scoped(&self.device, &mut || {
            encoded = self.encode_bands(image, display);
        });
        let ticks = classified(encoded, refused, self.lost.load(Ordering::SeqCst))?;
        Ok(self.elapsed(ticks))
    }

    /// The nanoseconds of `ticks` of this GPU's clock: `None` without a clock, and for
    /// unmeasured ticks. The one conversion of both passes' times.
    fn elapsed(&self, ticks: Option<u64>) -> Option<f64> {
        self.period
            .zip(ticks)
            .map(|(period, ticks)| nanoseconds(ticks, period))
    }

    /// What encoding an image of `size` for display costs on this GPU ([`encode_cost`] at
    /// the bands its buffers and dispatch allow).
    #[must_use]
    pub fn encode_cost(&self, size: ImageSize) -> EncodeCost {
        encode_cost(size, self.band_bytes, self.max_groups)
    }

    /// [`GpuSquare::encode`], within its error scopes: bands of as many pixels as one
    /// storage buffer holds of their values.
    fn encode_bands(
        &self,
        image: &Image,
        display: &mut DisplayImage,
    ) -> Result<Option<u64>, GpuError> {
        let pixels_per_band = display_band(image.size().pixels(), self.band_bytes);
        let band_bytes = pixels_per_band.saturating_mul(Image::PIXEL_BYTES);
        let out_bytes = pixels_per_band.saturating_mul(DisplayImage::PIXEL_BYTES);
        let EncodeBuffers {
            params,
            values,
            pixels,
            bind_group,
        } = self.encode_buffers(band_bytes, out_bytes);
        let readback = self.readback_buffers(out_bytes);
        // Unmeasured once any pass is.
        let mut ticks = Some(0_u64);
        // Both sides split by pixels, each by its own channel count: a band is the same
        // pixels of the image and of its display image, by construction.
        let band_pixels = usize::try_from(pixels_per_band).unwrap_or(usize::MAX);
        for (input, output) in image
            .values()
            .as_chunks::<{ Image::CHANNELS }>()
            .0
            .chunks(band_pixels)
            .zip(
                display
                    .bytes_mut()
                    .as_chunks_mut::<{ DisplayImage::CHANNELS }>()
                    .0
                    .chunks_mut(band_pixels),
            )
        {
            let output = output.as_flattened_mut();
            let count = u32::try_from(input.len()).unwrap_or(u32::MAX);
            let (columns, rows) = display_grid(count, self.max_groups);
            let mut uniform_bytes = [0_u8; PARAMS_BYTES];
            pack(
                &[count, columns.saturating_mul(DISPLAY_WORKGROUP), 0, 0],
                &mut uniform_bytes,
            );
            self.queue.write_buffer(&params, 0, &uniform_bytes);
            let input_bytes: Vec<u8> = input
                .as_flattened()
                .iter()
                .flat_map(|value| value.to_bits().to_le_bytes())
                .collect();
            self.queue.write_buffer(&values, 0, &input_bytes);
            let pass = Pass {
                label: "display",
                pipeline: &self.display_pipeline,
                bind_group: &bind_group,
                groups: (columns, rows),
                written: &pixels,
                bytes: byte_count(output.len()),
            };
            let pass_ticks = self.run_pass(&pass, &readback, &mut |bytes| {
                output.copy_from_slice(bytes);
            })?;
            ticks = summed(ticks, pass_ticks);
        }
        Ok(ticks)
    }

    /// The buffers of encoding bands of `band_bytes` bytes of values into `out_bytes` bytes
    /// for display, and their binding.
    fn encode_buffers(&self, band_bytes: u64, out_bytes: u64) -> EncodeBuffers {
        let params = self.buffer(
            "params",
            byte_count(PARAMS_BYTES),
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
        let bind_group = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("display"),
            layout: &self.display_pipeline.get_bind_group_layout(0),
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
        EncodeBuffers {
            params,
            values,
            pixels,
            bind_group,
        }
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

    /// The GPU's driver and its version as wgpu gives them, each trimmed and joined by a
    /// space: `-` when it gives none, as on Metal, whose driver is the system's.
    #[must_use]
    pub fn driver(&self) -> &str {
        &self.driver
    }

    /// Renders image `index` of `square` into `output` on the GPU: exactly the values the
    /// CPU reference renders ([`visual_core::VisualBox::render`]), at `output`'s size.
    ///
    /// # Errors
    ///
    /// Returns [`GpuError::Refused`] when the GPU refuses the work on an intact device (a
    /// buffer beyond its limits, memory it cannot hold) or its error scopes do not answer
    /// at once, [`GpuError::Lost`] when the device is lost (destroyed, or by its driver) and
    /// does not give the pixels back, or an error was caught on it, and
    /// [`GpuError::NotRead`] when an intact one does not; `output` then holds the bands
    /// rendered before, possibly all.
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
    /// last, so that what the CPU does meanwhile does not count. `None` on a GPU
    /// without a clock its programs can read (no `wgpu::Features::TIMESTAMP_QUERY`, or a
    /// period no pass can be timed with), and when the GPU wrote no timestamp of a pass (an
    /// unmeasured pass, never one of 0 ticks).
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
        let ticks = classified(rendered, refused, self.lost.load(Ordering::SeqCst))?;
        Ok(self.elapsed(ticks))
    }

    /// [`GpuSquare::timed_render`], within its error scopes: the ticks its passes took.
    fn render_bands(
        &self,
        square: &Square,
        index: ImageIndex,
        output: &mut Image,
    ) -> Result<Option<u64>, GpuError> {
        let size = output.size();
        let placed = square.placement(index, size);
        let row_bytes = u64::from(size.width()).saturating_mul(Image::PIXEL_BYTES);
        let band_rows = band_rows(size, self.band_bytes);
        let band_bytes = row_bytes.saturating_mul(u64::from(band_rows));
        let BandBuffers {
            uniform,
            storage,
            bind_group,
        } = self.band_buffers(band_bytes);
        let readback = self.readback_buffers(band_bytes);
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
            pack(&fields, &mut uniform_bytes);
            self.queue.write_buffer(&uniform, 0, &uniform_bytes);
            let pass = Pass {
                label: "square",
                pipeline: &self.square_pipeline,
                bind_group: &bind_group,
                groups: (
                    size.width().div_ceil(SQUARE_WORKGROUP),
                    rows.div_ceil(SQUARE_WORKGROUP),
                ),
                written: &storage,
                bytes: row_bytes.saturating_mul(u64::from(rows)),
            };
            let pass_ticks = self.run_pass(&pass, &readback, &mut |bytes| {
                for (value, word) in band
                    .iter_mut()
                    .flat_map(|row| row.iter_mut())
                    .zip(bytes.as_chunks::<4>().0)
                {
                    *value = f32::from_le_bytes(*word);
                }
            })?;
            ticks = summed(ticks, pass_ticks);
            first_row = first_row.saturating_add(rows);
        }
        Ok(ticks)
    }

    /// The buffers of rendering bands of `band_bytes` bytes of pixels, and their binding.
    fn band_buffers(&self, band_bytes: u64) -> BandBuffers {
        let uniform = self.buffer(
            "placement",
            byte_count(PLACEMENT_BYTES),
            wgpu::BufferUsages::UNIFORM.union(wgpu::BufferUsages::COPY_DST),
        );
        let storage = self.buffer(
            "values",
            band_bytes,
            wgpu::BufferUsages::STORAGE.union(wgpu::BufferUsages::COPY_SRC),
        );
        let bind_group = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("square"),
            layout: &self.square_pipeline.get_bind_group_layout(0),
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
        BandBuffers {
            uniform,
            storage,
            bind_group,
        }
    }

    /// The buffers every pass of bands of at most `bytes` bytes is read back through: the
    /// band's bytes, then the timestamps of its pass when the GPU has a clock.
    fn readback_buffers(&self, bytes: u64) -> Readback {
        Readback {
            buffer: self.buffer(
                "readback",
                bytes.saturating_add(STAMP_BYTES),
                wgpu::BufferUsages::MAP_READ.union(wgpu::BufferUsages::COPY_DST),
            ),
            stamps: self.buffer(
                "timestamps",
                STAMP_BYTES,
                wgpu::BufferUsages::QUERY_RESOLVE
                    .union(wgpu::BufferUsages::COPY_SRC)
                    .union(wgpu::BufferUsages::COPY_DST),
            ),
        }
    }

    /// Runs `pass` on the GPU and reads its band back through `readback`: records it (its
    /// timestamps when the GPU has a clock, in a query set created for this pass alone,
    /// resolved into `readback`'s timestamps cleared first, so that no pass, of another band
    /// or another thread, reads timestamps another wrote),
    /// copies its bytes and timestamps to `readback`, submits, waits, gives the bytes to
    /// `read` and returns the ticks the pass took ([`pass_ticks`]). The one copy of that
    /// sequence both shaders run through; `read` is a trait object, so that its one body is
    /// the one the tests cover.
    ///
    /// # Errors
    ///
    /// Returns [`GpuError::NotRead`] when the bytes cannot be read back: [`classified`] makes
    /// it a loss on a lost device, and a refusal when the scopes of an intact one caught one.
    fn run_pass(
        &self,
        pass: &Pass<'_>,
        readback: &Readback,
        read: &mut dyn FnMut(&[u8]),
    ) -> Result<Option<u64>, GpuError> {
        // Created on the CPU, outside the interval its timestamps measure.
        let queries = self.period.map(|_| {
            self.device.create_query_set(&wgpu::QuerySetDescriptor {
                label: Some("timestamps"),
                ty: wgpu::QueryType::Timestamp,
                count: 2,
            })
        });
        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some(pass.label),
            });
        {
            let mut compute = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some(pass.label),
                timestamp_writes: queries
                    .as_ref()
                    .map(|set| wgpu::ComputePassTimestampWrites {
                        query_set: set,
                        beginning_of_pass_write_index: Some(0),
                        end_of_pass_write_index: Some(1),
                    }),
            });
            compute.set_pipeline(pass.pipeline);
            compute.set_bind_group(0, pass.bind_group, &[]);
            compute.dispatch_workgroups(pass.groups.0, pass.groups.1, 1);
        }
        encoder.copy_buffer_to_buffer(pass.written, 0, &readback.buffer, 0, pass.bytes);
        if let Some(set) = &queries {
            // Cleared first: a timestamp the GPU leaves unwritten then resolves to 0 (an
            // unmeasured pass), never to the previous band's, which this buffer held.
            encoder.clear_buffer(&readback.stamps, 0, None);
            encoder.resolve_query_set(set, 0..2, &readback.stamps, 0);
            encoder.copy_buffer_to_buffer(
                &readback.stamps,
                0,
                &readback.buffer,
                pass.bytes,
                STAMP_BYTES,
            );
        }
        let _submitted = self.queue.submit([encoder.finish()]);
        self.map_read(&readback.buffer, pass.bytes, read)
            .map_err(|error| GpuError::NotRead(error.to_string()))
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

    /// Waits for the GPU, maps the first `bytes` of `readback` and the pass's two timestamps
    /// that follow them, gives the `bytes` to `read`, and returns the ticks the pass took
    /// ([`pass_ticks`]). The map-and-read step of [`GpuSquare::run_pass`]; `read` is a
    /// trait object, so that its one body is the one the tests cover.
    ///
    /// # Errors
    ///
    /// Returns why the bytes could not be read: one error for the caller, whatever the step
    /// that failed (the device's wait, the mapping, or its answer).
    fn map_read(
        &self,
        readback: &wgpu::Buffer,
        bytes: u64,
        read: &mut dyn FnMut(&[u8]),
    ) -> Result<Option<u64>, Box<dyn core::error::Error>> {
        let mapped_bytes = bytes.saturating_add(STAMP_BYTES);
        let (sender, receiver) = mpsc::channel();
        readback
            .slice(..mapped_bytes)
            .map_async(wgpu::MapMode::Read, move |outcome| {
                drop(sender.send(outcome));
            });
        let _polled = self.device.poll(wgpu::PollType::wait_indefinitely())?;
        // The mapping is done once the wait returns, but its callback may still be running on
        // another thread sharing the device, whose own wait fired it: waited for, never assumed.
        // An error only if the callback is dropped uncalled.
        receiver.recv()??;
        let mapped = readback.slice(..mapped_bytes).get_mapped_range()?;
        let (values, stamps) = mapped.split_at(usize::try_from(bytes)?);
        read(values);
        let ticks = pass_ticks(stamps);
        drop(mapped);
        readback.unmap();
        Ok(ticks)
    }
}
