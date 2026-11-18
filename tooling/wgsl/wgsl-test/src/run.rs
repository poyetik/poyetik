//! Running a compute kernel from a test: a WGSL shader, buffers given as typed values, one
//! dispatch, and every buffer the kernel may write read back, bit for bit.
//!
//! Every failure is a [`RunError`], never a panic: wgpu's default handler panics on a
//! validation or out-of-memory error, so every call that can raise one runs inside error
//! scopes that catch it ([`Gpu::scoped`]).

use core::error::Error;
use core::fmt;
use core::pin::{Pin, pin};
use core::task::{Context, Waker};
use std::sync::{OnceLock, mpsc};
use std::thread;

use crate::layout::{self, Access, Binding, Scalar};
use crate::safe::safe_message;
use crate::validation::{Invalid, validated};

#[cfg(test)]
#[path = "run_tests.rs"]
mod tests;

/// Why a kernel could not be built or run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RunError {
    /// No GPU adapter answered: none, or none of the APIs asked for.
    NoAdapter(String),
    /// The adapter refused to open a device.
    NoDevice(String),
    /// The shader is not a valid WGSL module ([`validate`](crate::validate)'s refusal).
    InvalidShader(Invalid),
    /// The shader has no compute entry point of that name, or binds something other than a
    /// buffer.
    BadKernel(String),
    /// A buffer given does not fit the shader's binding, or a binding has no buffer.
    BadLayout {
        /// The bind group (`@group`).
        group: u32,
        /// The binding in the group (`@binding`).
        binding: u32,
        /// What does not fit.
        why: String,
    },
    /// More workgroups asked in one dimension than the device dispatches.
    TooManyWorkgroups {
        /// The workgroups asked, in x, y and z.
        asked: Workgroups,
        /// The most the device dispatches in one dimension.
        limit: u32,
    },
    /// A buffer given is larger than the device binds: its binding's limit, the least of the
    /// device's largest buffer and its largest binding of the buffer's kind (uniform or
    /// storage).
    TooLarge {
        /// The bind group (`@group`).
        group: u32,
        /// The binding in the group (`@binding`).
        binding: u32,
        /// The bytes given.
        bytes: u64,
        /// The most the device binds there.
        limit: u64,
    },
    /// The device refused a call: a validation or out-of-memory error, as wgpu says it.
    Device(String),
    /// The written buffers could not be read back: the device lost, or the mapping refused.
    ReadBack(String),
    /// A binding asked of a kernel's outputs that the kernel does not write: none, or one
    /// it only reads, which is never read back.
    NotWritten {
        /// The bind group (`@group`).
        group: u32,
        /// The binding in the group (`@binding`).
        binding: u32,
    },
}

impl fmt::Display for RunError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NoAdapter(why) => write!(formatter, "no GPU adapter: {}", safe_message(why)),
            Self::NoDevice(why) => write!(formatter, "no GPU device: {}", safe_message(why)),
            Self::InvalidShader(invalid) => write!(formatter, "invalid shader: {invalid}"),
            Self::BadKernel(why) => write!(formatter, "bad kernel: {}", safe_message(why)),
            Self::BadLayout {
                group,
                binding,
                why,
            } => write!(
                formatter,
                "bad layout: group {group} binding {binding}: {}",
                safe_message(why)
            ),
            Self::TooManyWorkgroups { asked, limit } => write!(
                formatter,
                "too many workgroups: {}x{}x{} asked, at most {limit} per dimension",
                asked.0, asked.1, asked.2
            ),
            Self::TooLarge {
                group,
                binding,
                bytes,
                limit,
            } => write!(
                formatter,
                "too large: group {group} binding {binding}: {bytes} bytes given, at most {limit} bound"
            ),
            Self::Device(why) => write!(formatter, "the device refused: {}", safe_message(why)),
            Self::ReadBack(why) => write!(formatter, "read-back failed: {}", safe_message(why)),
            Self::NotWritten { group, binding } => write!(
                formatter,
                "not written: group {group} binding {binding} is no read_write buffer of the kernel"
            ),
        }
    }
}

impl Error for RunError {}

/// The workgroups of one dispatch, in each dimension.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Workgroups(
    /// Along x.
    pub u32,
    /// Along y.
    pub u32,
    /// Along z.
    pub u32,
);

/// The values of a buffer given to a kernel, transported as their bits, little-endian: a
/// float keeps its exact bits, the payload of a not-a-number included.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Data<'values> {
    /// `u32` values, of a binding made of `u32` only (scalars, vectors, arrays of them).
    U32(&'values [u32]),
    /// `i32` values, of a binding made of `i32` only.
    I32(&'values [i32]),
    /// `f32` values, of a binding made of `f32` only.
    F32(&'values [f32]),
    /// Raw bytes, for any binding: a structure of mixed members, a padded `vec3`.
    Bytes(&'values [u8]),
    /// This many zero bytes, for any binding: a buffer the kernel only writes.
    Zeroed(u64),
}

impl Data<'_> {
    /// The scalar of the values, none for raw or zero bytes.
    const fn scalar(&self) -> Option<Scalar> {
        match self {
            Self::U32(_) => Some(Scalar::U32),
            Self::I32(_) => Some(Scalar::I32),
            Self::F32(_) => Some(Scalar::F32),
            Self::Bytes(_) | Self::Zeroed(_) => None,
        }
    }

    /// The bytes of the values, little-endian, none for zero bytes: a new buffer holds
    /// zeros, so that a large output costs no copy.
    fn bytes(&self) -> Option<Vec<u8>> {
        Some(match *self {
            Self::U32(values) => values
                .iter()
                .flat_map(|value| value.to_le_bytes())
                .collect(),
            Self::I32(values) => values
                .iter()
                .flat_map(|value| value.to_le_bytes())
                .collect(),
            Self::F32(values) => values
                .iter()
                .flat_map(|value| value.to_bits().to_le_bytes())
                .collect(),
            Self::Bytes(bytes) => bytes.to_vec(),
            Self::Zeroed(_) => return None,
        })
    }

    /// How many bytes the values hold.
    fn len(&self) -> u64 {
        let bytes = match *self {
            Self::U32(values) => values.len().saturating_mul(4),
            Self::I32(values) => values.len().saturating_mul(4),
            Self::F32(values) => values.len().saturating_mul(4),
            Self::Bytes(bytes) => bytes.len(),
            Self::Zeroed(count) => return count,
        };
        u64::try_from(bytes).unwrap_or(u64::MAX)
    }
}

/// A buffer given to a kernel: its bind group, its binding, its values.
pub type Given<'values> = (u32, u32, Data<'values>);

/// Runs `future` to completion on this thread, yielding it while the future is pending.
///
/// wgpu's native futures are ready at their first poll, so the loop never spins in practice;
/// a future that is not is polled again after each yield, with no waker to wait for. Not
/// generic, so that its one instantiation is the one its tests cover: a caller wraps its
/// future in an `async` block that stores the output.
fn block_on(mut future: Pin<&mut dyn Future<Output = ()>>) {
    let mut context = Context::from_waker(Waker::noop());
    while future.as_mut().poll(&mut context).is_pending() {
        thread::yield_now();
    }
}

/// The limits a device is opened with.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Limits {
    /// WebGPU's default limits ([`wgpu::Limits::default`]), which every browser grants
    /// without asking: a kernel that runs under them runs on the web.
    WebGpu,
    /// The adapter's own limits: as much as this GPU allows, which a browser may not grant.
    Adapter,
}

/// A GPU device to run kernels on.
#[derive(Debug)]
pub struct Gpu {
    /// The device.
    device: wgpu::Device,
    /// Its queue.
    queue: wgpu::Queue,
    /// The limits it was opened with.
    limits: wgpu::Limits,
    /// The adapter's name and API, for the reports.
    adapter: String,
}

/// The device every test of a program shares, opened at the first call of
/// [`Gpu::shared`].
static SHARED: OnceLock<Result<Gpu, RunError>> = OnceLock::new();

impl Gpu {
    /// The device every test of this program shares: opened once, at the first call, on the
    /// first adapter of the APIs the environment variable `WGPU_BACKEND` lists (wgpu's own
    /// variable, so that a CI runner picks its software GPU without a change of code), of
    /// all of them when it is not set; under WebGPU's default limits ([`Limits::WebGpu`]),
    /// so that a kernel the tests prove is one the web accepts.
    ///
    /// # Errors
    ///
    /// Returns the error of opening it, [`Gpu::open`]'s, at every call when it failed.
    pub fn shared() -> Result<&'static Self, RunError> {
        SHARED
            .get_or_init(|| {
                Self::open(
                    wgpu::Backends::from_env().unwrap_or_default(),
                    Limits::WebGpu,
                )
            })
            .as_ref()
            .map_err(Clone::clone)
    }

    /// Opens the first adapter of `backends` (all of them: [`wgpu::Backends::all`]) under
    /// `limits`.
    ///
    /// # Errors
    ///
    /// Returns [`RunError::NoAdapter`] when no adapter of `backends` answers, and
    /// [`RunError::NoDevice`] when it refuses a device (an adapter below WebGPU's default
    /// limits refuses [`Limits::WebGpu`]).
    pub fn open(backends: wgpu::Backends, limits: Limits) -> Result<Self, RunError> {
        Self::opening(
            backends,
            match limits {
                Limits::WebGpu => |_| wgpu::Limits::default(),
                Limits::Adapter => |adapter| adapter,
            },
        )
    }

    /// [`Gpu::open`], asking the device for `limits` applied to the adapter's own. A
    /// function pointer, not a generic, so that the one body the tests cover is the one
    /// shipped.
    fn opening(
        backends: wgpu::Backends,
        limits: fn(wgpu::Limits) -> wgpu::Limits,
    ) -> Result<Self, RunError> {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
            backends,
            ..wgpu::InstanceDescriptor::new_without_display_handle()
        });
        // Replaced by the answer: `block_on` returns once the request has completed.
        let mut adapter = Err(RunError::NoAdapter(String::new()));
        block_on(pin!(async {
            adapter = instance
                .request_adapter(&wgpu::RequestAdapterOptions::default())
                .await
                .map_err(|error| RunError::NoAdapter(error.to_string()));
        }));
        let adapter = adapter?;
        let required_limits = limits(adapter.limits());
        // Replaced by the answer, as the adapter's.
        let mut device = Err(RunError::NoDevice(String::new()));
        block_on(pin!(async {
            device = adapter
                .request_device(&wgpu::DeviceDescriptor {
                    label: Some("wgsl-test"),
                    required_limits: required_limits.clone(),
                    ..wgpu::DeviceDescriptor::default()
                })
                .await
                .map_err(|error| RunError::NoDevice(error.to_string()));
        }));
        let (device, queue) = device?;
        let info = adapter.get_info();
        Ok(Self {
            device,
            queue,
            limits: required_limits,
            adapter: format!("{} ({})", info.name, info.backend),
        })
    }

    /// The adapter's name and API, as the reports show it.
    #[must_use]
    pub fn adapter(&self) -> &str {
        &self.adapter
    }

    /// The limits the device was opened with.
    #[must_use]
    pub const fn limits(&self) -> &wgpu::Limits {
        &self.limits
    }

    /// Runs `work` inside a validation and an out-of-memory error scope, and returns the
    /// error the device raised, if any (the out-of-memory one first). Not generic, as
    /// [`block_on`]: `work` stores its own result.
    fn scoped(&self, work: &mut dyn FnMut()) -> Option<String> {
        let refused = self.device.push_error_scope(wgpu::ErrorFilter::Validation);
        let exhausted = self.device.push_error_scope(wgpu::ErrorFilter::OutOfMemory);
        work();
        let mut errors = (None, None);
        block_on(pin!(async {
            errors = (exhausted.pop().await, refused.pop().await);
        }));
        errors.0.or(errors.1).map(|error| error.to_string())
    }
}

/// A compute kernel: a shader's entry point compiled on a device, with the buffers its
/// shader binds.
#[derive(Debug)]
pub struct Kernel<'gpu> {
    /// The device it runs on.
    gpu: &'gpu Gpu,
    /// The compiled entry point.
    pipeline: wgpu::ComputePipeline,
    /// The buffers it binds, in group then binding order.
    bindings: Vec<Binding>,
}

impl<'gpu> Kernel<'gpu> {
    /// Compiles the compute entry point `entry` of the WGSL `source` on `gpu`.
    ///
    /// The source is first validated as [`validate`](crate::validate) does, then the
    /// buffers it binds are read from it: the uniform and storage globals the entry point
    /// uses. The pipeline's layout is wgpu's, derived from the shader too.
    ///
    /// # Errors
    ///
    /// Returns [`RunError::InvalidShader`] when the source is not valid,
    /// [`RunError::BadKernel`] when it has no compute entry point named `entry` or binds
    /// something other than a buffer, and [`RunError::Device`] when the device refuses the
    /// shader or its pipeline (a limit of this device exceeded).
    pub fn new(gpu: &'gpu Gpu, source: &str, entry: &str) -> Result<Self, RunError> {
        let (module, info) = validated(source).map_err(RunError::InvalidShader)?;
        let bindings = layout::bindings(&module, &info, entry).map_err(RunError::BadKernel)?;
        let mut pipeline = None;
        let refused = gpu.scoped(&mut || {
            let shader = gpu
                .device
                .create_shader_module(wgpu::ShaderModuleDescriptor {
                    label: Some(entry),
                    source: wgpu::ShaderSource::Wgsl(source.into()),
                });
            pipeline = Some(
                gpu.device
                    .create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                        label: Some(entry),
                        layout: None,
                        module: &shader,
                        entry_point: Some(entry),
                        compilation_options: wgpu::PipelineCompilationOptions::default(),
                        cache: None,
                    }),
            );
        });
        match (refused, pipeline) {
            (None, Some(pipeline)) => Ok(Self {
                gpu,
                pipeline,
                bindings,
            }),
            (refused, _) => Err(RunError::Device(refused.unwrap_or_default())),
        }
    }

    /// The buffers the kernel binds, in group then binding order.
    #[must_use]
    pub fn bindings(&self) -> &[Binding] {
        &self.bindings
    }

    /// The buffer given for `binding`, checked against it.
    ///
    /// # Errors
    ///
    /// Returns [`RunError::BadLayout`] when none or several are given, or when the one given
    /// does not fit, and [`RunError::TooLarge`] when it is larger than the device binds.
    fn given_for<'values>(
        &self,
        binding: &Binding,
        given: &[Given<'values>],
    ) -> Result<Data<'values>, RunError> {
        let bad = |why: String| RunError::BadLayout {
            group: binding.group(),
            binding: binding.binding(),
            why,
        };
        let mut matching = given
            .iter()
            .filter(|(group, number, _)| (*group, *number) == (binding.group(), binding.binding()));
        let (_, _, data) = matching
            .next()
            .ok_or_else(|| bad("no buffer given for it".to_owned()))?;
        if matching.next().is_some() {
            return Err(bad("given more than once".to_owned()));
        }
        if let Some(why) = binding.refuses(data.len(), data.scalar()) {
            return Err(bad(why));
        }
        let limits = &self.gpu.limits;
        let limit = match binding.access() {
            Access::Uniform => limits.max_uniform_buffer_binding_size,
            Access::Read | Access::ReadWrite => limits.max_storage_buffer_binding_size,
        }
        .min(limits.max_buffer_size);
        if data.len() > limit {
            return Err(RunError::TooLarge {
                group: binding.group(),
                binding: binding.binding(),
                bytes: data.len(),
                limit,
            });
        }
        Ok(*data)
    }

    /// Runs the kernel once: `given` holds one buffer for each binding of the shader, then
    /// `workgroups` are dispatched, and every buffer the kernel may write
    /// ([`Access::ReadWrite`]) is read back.
    ///
    /// Every buffer is checked against its binding before anything reaches the GPU: exactly
    /// the bytes its type holds, for a binding of a fixed size, where WebGPU would also bind
    /// a larger buffer, so that a test says the size its shader declares; and no more bytes
    /// than the device binds there.
    ///
    /// # Errors
    ///
    /// Returns [`RunError::TooManyWorkgroups`] when a dimension exceeds the device's limit,
    /// [`RunError::BadLayout`] when a buffer given names no binding of the shader, when a
    /// binding has none or several, or when one does not fit its binding (its scalar or its
    /// size), [`RunError::TooLarge`] when one is larger than the device binds,
    /// [`RunError::Device`] when the device refuses a call, and
    /// [`RunError::ReadBack`] when the written buffers cannot be read back.
    pub fn run(&self, given: &[Given<'_>], workgroups: Workgroups) -> Result<Outputs, RunError> {
        let limit = self.gpu.limits.max_compute_workgroups_per_dimension;
        if [workgroups.0, workgroups.1, workgroups.2]
            .iter()
            .any(|count| *count > limit)
        {
            return Err(RunError::TooManyWorkgroups {
                asked: workgroups,
                limit,
            });
        }
        if let Some((group, binding, _)) = given.iter().find(|(group, number, _)| {
            !self
                .bindings
                .iter()
                .any(|declared| (declared.group(), declared.binding()) == (*group, *number))
        }) {
            return Err(RunError::BadLayout {
                group: *group,
                binding: *binding,
                why: "the shader binds no buffer there".to_owned(),
            });
        }
        let data = self
            .bindings
            .iter()
            .map(|binding| self.given_for(binding, given))
            .collect::<Result<Vec<_>, _>>()?;
        let mut outcome = Err(RunError::Device(String::new()));
        let refused = self
            .gpu
            .scoped(&mut || outcome = self.dispatch(&data, workgroups));
        refused.map_or(outcome, |why| Err(RunError::Device(why)))
    }

    /// [`Kernel::run`] once its buffers are checked, within its error scopes: `data` holds
    /// the buffer of each binding, in the order of [`Kernel::bindings`].
    fn dispatch(&self, data: &[Data<'_>], workgroups: Workgroups) -> Result<Outputs, RunError> {
        let device = &self.gpu.device;
        let buffers: Vec<wgpu::Buffer> = self
            .bindings
            .iter()
            .zip(data)
            .map(|(binding, values)| {
                let usage = match binding.access() {
                    Access::Uniform => wgpu::BufferUsages::UNIFORM,
                    Access::Read => wgpu::BufferUsages::STORAGE,
                    Access::ReadWrite => {
                        wgpu::BufferUsages::STORAGE.union(wgpu::BufferUsages::COPY_SRC)
                    }
                };
                let buffer = device.create_buffer(&wgpu::BufferDescriptor {
                    label: None,
                    size: values.len(),
                    usage: usage.union(wgpu::BufferUsages::COPY_DST),
                    mapped_at_creation: false,
                });
                if let Some(bytes) = values.bytes() {
                    self.gpu.queue.write_buffer(&buffer, 0, &bytes);
                }
                buffer
            })
            .collect();
        let last_group = self.bindings.iter().map(Binding::group).max();
        let groups: Vec<(u32, wgpu::BindGroup)> = last_group
            .map_or(0..0, |last| 0..last.saturating_add(1))
            .map(|group| {
                let entries: Vec<wgpu::BindGroupEntry<'_>> = self
                    .bindings
                    .iter()
                    .zip(&buffers)
                    .filter(|(binding, _)| binding.group() == group)
                    .map(|(binding, buffer)| wgpu::BindGroupEntry {
                        binding: binding.binding(),
                        resource: buffer.as_entire_binding(),
                    })
                    .collect();
                let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
                    label: None,
                    layout: &self.pipeline.get_bind_group_layout(group),
                    entries: &entries,
                });
                (group, bind_group)
            })
            .collect();
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
        {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor::default());
            pass.set_pipeline(&self.pipeline);
            for (group, bind_group) in &groups {
                pass.set_bind_group(*group, bind_group, &[]);
            }
            pass.dispatch_workgroups(workgroups.0, workgroups.1, workgroups.2);
        }
        let (written, readbacks): (Vec<&Binding>, Vec<wgpu::Buffer>) = self
            .bindings
            .iter()
            .zip(&buffers)
            .filter(|(binding, _)| binding.access() == Access::ReadWrite)
            .map(|(binding, buffer)| {
                let readback = device.create_buffer(&wgpu::BufferDescriptor {
                    label: None,
                    size: buffer.size(),
                    usage: wgpu::BufferUsages::MAP_READ.union(wgpu::BufferUsages::COPY_DST),
                    mapped_at_creation: false,
                });
                encoder.copy_buffer_to_buffer(buffer, 0, &readback, 0, buffer.size());
                (binding, readback)
            })
            .unzip();
        let _submitted = self.gpu.queue.submit([encoder.finish()]);
        let read = self.read_back(&readbacks)?;
        Ok(Outputs {
            buffers: written
                .into_iter()
                .zip(read)
                .map(|(binding, bytes)| (binding.group(), binding.binding(), bytes))
                .collect(),
        })
    }

    /// Waits for the GPU, then reads the whole of each of `readbacks`.
    ///
    /// # Errors
    ///
    /// Returns [`RunError::ReadBack`] when the device cannot be waited for or a buffer
    /// cannot be mapped.
    fn read_back(&self, readbacks: &[wgpu::Buffer]) -> Result<Vec<Vec<u8>>, RunError> {
        self.reading(readbacks)
            .map_err(|error| RunError::ReadBack(error.to_string()))
    }

    /// [`Kernel::read_back`], its error as the source gives it.
    ///
    /// # Errors
    ///
    /// Returns why the device could not be waited for or a buffer could not be mapped.
    fn reading(&self, readbacks: &[wgpu::Buffer]) -> Result<Vec<Vec<u8>>, Box<dyn Error>> {
        let receivers: Vec<_> = readbacks
            .iter()
            .map(|readback| {
                let (sender, receiver) = mpsc::channel();
                readback.map_async(wgpu::MapMode::Read, .., move |outcome| {
                    drop(sender.send(outcome));
                });
                receiver
            })
            .collect();
        let _polled = self.gpu.device.poll(wgpu::PollType::wait_indefinitely())?;
        let mut read = Vec::with_capacity(readbacks.len());
        for (readback, receiver) in readbacks.iter().zip(receivers) {
            receiver.try_recv()??;
            read.push(readback.get_mapped_range(..)?.to_vec());
            readback.unmap();
        }
        Ok(read)
    }
}

/// The buffers a kernel wrote, read back after its dispatch: one for each binding it may
/// write, in group then binding order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Outputs {
    /// Each written buffer: its bind group, its binding, its bytes.
    buffers: Vec<(u32, u32, Vec<u8>)>,
}

impl Outputs {
    /// The bytes of the buffer of `binding` in `group`.
    ///
    /// # Errors
    ///
    /// Returns [`RunError::NotWritten`] when the kernel does not write that binding.
    pub fn bytes(&self, group: u32, binding: u32) -> Result<&[u8], RunError> {
        self.buffers
            .iter()
            .find(|(at_group, at_binding, _)| (*at_group, *at_binding) == (group, binding))
            .map(|(_, _, bytes)| bytes.as_slice())
            .ok_or(RunError::NotWritten { group, binding })
    }

    /// The buffer of `binding` in `group` as 32-bit words, little-endian, a trailing part of
    /// a word left out.
    ///
    /// # Errors
    ///
    /// Returns [`RunError::NotWritten`] when the kernel does not write that binding.
    fn words(&self, group: u32, binding: u32) -> Result<Vec<[u8; 4]>, RunError> {
        self.bytes(group, binding)
            .map(|bytes| bytes.as_chunks::<4>().0.to_vec())
    }

    /// The buffer of `binding` in `group` as `u32` values.
    ///
    /// # Errors
    ///
    /// Returns [`RunError::NotWritten`] when the kernel does not write that binding.
    pub fn u32s(&self, group: u32, binding: u32) -> Result<Vec<u32>, RunError> {
        self.words(group, binding)
            .map(|words| words.into_iter().map(u32::from_le_bytes).collect())
    }

    /// The buffer of `binding` in `group` as `i32` values.
    ///
    /// # Errors
    ///
    /// Returns [`RunError::NotWritten`] when the kernel does not write that binding.
    pub fn i32s(&self, group: u32, binding: u32) -> Result<Vec<i32>, RunError> {
        self.words(group, binding)
            .map(|words| words.into_iter().map(i32::from_le_bytes).collect())
    }

    /// The buffer of `binding` in `group` as `f32` values, their bits exactly as the kernel
    /// wrote them (the payload of a not-a-number included).
    ///
    /// # Errors
    ///
    /// Returns [`RunError::NotWritten`] when the kernel does not write that binding.
    pub fn f32s(&self, group: u32, binding: u32) -> Result<Vec<f32>, RunError> {
        self.words(group, binding).map(|words| {
            words
                .into_iter()
                .map(|word| f32::from_bits(u32::from_le_bytes(word)))
                .collect()
        })
    }
}
