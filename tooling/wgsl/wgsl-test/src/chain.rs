//! Kernels chained on buffers that stay on the GPU: buffers made once ([`Buffers`]), bound
//! to each kernel that uses them ([`Bound`]), then several steps ([`Step`]: passes and
//! copies) encoded into one submission, as a renderer chains its passes, with no copy back
//! to the processor between them; any buffer read back or rewritten between submissions.
//!
//! Every check of a one-dispatch run applies to each binding (its size, its scalar, one
//! buffer for it, no buffer where the shader binds none, no more bytes than the device
//! binds), and a buffer a pass writes is never bound twice in it, as WebGPU requires: all
//! before anything reaches the GPU.

use core::ptr;
use core::sync::atomic::{AtomicU64, Ordering};

use super::{Data, Gpu, Kernel, RunError, Workgroups, one_for};
use crate::compare::{Inputs, Mismatch, Tolerance, Values, comparing};
use crate::layout::{Access, Binding, Scalar};

#[cfg(test)]
#[path = "chain_tests.rs"]
mod tests;

/// The next owner given to a set of buffers, so that an id or a binding made by one set is
/// refused by another.
static OWNERS: AtomicU64 = AtomicU64::new(0);

/// The usages of every buffer: bound as storage or uniform, written, copied, read back.
const USAGES: wgpu::BufferUsages = wgpu::BufferUsages::STORAGE
    .union(wgpu::BufferUsages::UNIFORM)
    .union(wgpu::BufferUsages::COPY_SRC)
    .union(wgpu::BufferUsages::COPY_DST);

/// What the values of a buffer are made of, as far as the harness knows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Kind {
    /// Not known yet: bytes or zeros written, never bound to a binding that says.
    Unknown,
    /// Values of one scalar: written as such, or bound to a binding of that scalar.
    One(Scalar),
    /// Members of several kinds: bound to a structure of mixed members, read as bytes only.
    Mixed,
}

impl Kind {
    /// The scalar a buffer of this kind is given as to a binding, none for bytes.
    const fn given(self) -> Option<Scalar> {
        match self {
            Self::One(scalar) => Some(scalar),
            Self::Unknown | Self::Mixed => None,
        }
    }

    /// This kind once bound to a binding declaring `declared` (none for mixed members), the
    /// binding having accepted it.
    const fn bound(self, declared: Option<Scalar>) -> Self {
        match (self, declared) {
            (Self::Unknown, Some(scalar)) => Self::One(scalar),
            (Self::Unknown, None) => Self::Mixed,
            (kept @ (Self::One(_) | Self::Mixed), _) => kept,
        }
    }

    /// This kind once values of `written` (none for bytes or zeros) are written, if they
    /// may be.
    fn written(self, written: Option<Scalar>) -> Result<Self, String> {
        match (self, written) {
            (kept, None) => Ok(kept),
            (Self::Unknown, Some(scalar)) => Ok(Self::One(scalar)),
            (Self::One(held), Some(scalar)) if held == scalar => Ok(self),
            (Self::One(held), Some(scalar)) => Err(format!(
                "{scalar} values written to a buffer of {held} values"
            )),
            (Self::Mixed, Some(scalar)) => Err(format!(
                "{scalar} values written to a buffer of mixed members: write bytes"
            )),
        }
    }
}

/// One buffer of a set, as its id names it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BufferId {
    /// The set that made it.
    owner: u64,
    /// Its place in the set.
    index: usize,
}

/// Buffers on one device that several kernels share, each kept until the set is dropped.
#[derive(Debug)]
pub struct Buffers<'gpu> {
    /// The device they live on.
    gpu: &'gpu Gpu,
    /// This set, for its ids and bindings.
    owner: u64,
    /// Each buffer and what its values are made of, by index.
    held: Vec<(wgpu::Buffer, Kind)>,
}

/// A kernel bound to buffers of one set, ready to be a pass of [`Buffers::dispatch`].
#[derive(Debug)]
pub struct Bound {
    /// The set of the buffers.
    owner: u64,
    /// The kernel's pipeline.
    pipeline: wgpu::ComputePipeline,
    /// The bind group of each group from 0 to the last the kernel uses.
    groups: Vec<(u32, wgpu::BindGroup)>,
}

/// One step of a submission.
#[derive(Debug, Clone, Copy)]
pub enum Step<'bound> {
    /// A pass: a bound kernel dispatched over workgroups.
    Pass(&'bound Bound, Workgroups),
    /// A copy of every byte of a buffer onto another of the same size.
    Copy {
        /// The buffer copied.
        from: BufferId,
        /// The buffer written.
        to: BufferId,
    },
}

impl<'gpu> Buffers<'gpu> {
    /// An empty set of buffers on `gpu`.
    #[must_use]
    pub fn new(gpu: &'gpu Gpu) -> Self {
        Self {
            gpu,
            owner: OWNERS.fetch_add(1, Ordering::Relaxed),
            held: Vec::new(),
        }
    }

    /// The buffer `id` names and its kind.
    ///
    /// # Errors
    ///
    /// Returns [`RunError::BadBuffer`] when `id` was made by another set.
    fn held(&self, id: BufferId) -> Result<&(wgpu::Buffer, Kind), RunError> {
        self.held
            .get(id.index)
            .filter(|_| id.owner == self.owner)
            .ok_or_else(foreign)
    }

    /// Adds a buffer holding `data`: its values, or zeros.
    ///
    /// # Errors
    ///
    /// Returns [`RunError::BadBuffer`] when its bytes are not whole 4-byte words or exceed
    /// the largest buffer the device makes, and [`RunError::Device`] when the device refuses
    /// it.
    pub fn add(&mut self, data: Data<'_>) -> Result<BufferId, RunError> {
        let bytes = data.len();
        let most = self.gpu.limits.max_buffer_size;
        if bytes.checked_rem(4) != Some(0) {
            return Err(RunError::BadBuffer(format!(
                "{bytes} bytes: a buffer holds whole 4-byte words"
            )));
        }
        if bytes > most {
            return Err(RunError::BadBuffer(format!(
                "{bytes} bytes: at most {most} in one buffer of this device"
            )));
        }
        let gpu = self.gpu;
        let mut created = None;
        let refused = gpu.scoped(&mut || {
            let buffer = gpu.device.create_buffer(&wgpu::BufferDescriptor {
                label: None,
                size: bytes,
                usage: USAGES,
                mapped_at_creation: false,
            });
            if let Some(values) = data.bytes() {
                gpu.queue.write_buffer(&buffer, 0, &values);
            }
            created = Some(buffer);
        });
        match (refused, created) {
            (None, Some(buffer)) => {
                let id = BufferId {
                    owner: self.owner,
                    index: self.held.len(),
                };
                self.held
                    .push((buffer, data.scalar().map_or(Kind::Unknown, Kind::One)));
                Ok(id)
            }
            (refused, _) => Err(gpu.refused(refused.unwrap_or_default())),
        }
    }

    /// Writes `data` over the whole buffer `id`, before the next submission: its values, or
    /// zeros.
    ///
    /// # Errors
    ///
    /// Returns [`RunError::BadBuffer`] when `id` was made by another set, when `data` does
    /// not hold the buffer's bytes exactly, or when its values are of another scalar than
    /// the buffer holds, and [`RunError::Device`] when the device refuses the write.
    pub fn write(&mut self, id: BufferId, data: Data<'_>) -> Result<(), RunError> {
        let gpu = self.gpu;
        let owner = self.owner;
        let (buffer, kind) = self
            .held
            .get_mut(id.index)
            .filter(|_| id.owner == owner)
            .ok_or_else(foreign)?;
        let size = buffer.size();
        if data.len() != size {
            return Err(RunError::BadBuffer(format!(
                "{} bytes written to a buffer of {size}",
                data.len()
            )));
        }
        let written = kind.written(data.scalar()).map_err(RunError::BadBuffer)?;
        let refused = gpu.scoped(&mut || {
            if let Some(values) = data.bytes() {
                gpu.queue.write_buffer(buffer, 0, &values);
            } else {
                let mut encoder = gpu
                    .device
                    .create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
                encoder.clear_buffer(buffer, 0, None);
                let _submitted = gpu.queue.submit([encoder.finish()]);
            }
        });
        if let Some(why) = refused {
            return Err(gpu.refused(why));
        }
        *kind = written;
        Ok(())
    }

    /// Binds `kernel` to buffers of this set: `given` holds one buffer for each of its
    /// bindings, by group and binding.
    ///
    /// # Errors
    ///
    /// Returns [`RunError::BadBuffer`] when the kernel was made on another device or a
    /// buffer by another set, [`RunError::BadLayout`] when a buffer names no binding of the
    /// shader, when a binding has none or several, when one does not fit its binding (its
    /// scalar or its size), or when a buffer the kernel writes is bound twice,
    /// [`RunError::TooLarge`] when one is larger than the device binds there, and
    /// [`RunError::Device`] when the device refuses the bind groups.
    pub fn bind(
        &mut self,
        kernel: &Kernel<'_>,
        given: &[(u32, u32, BufferId)],
    ) -> Result<Bound, RunError> {
        if !ptr::eq(kernel.gpu, self.gpu) {
            return Err(RunError::BadBuffer(
                "a kernel of another device than the buffers'".to_owned(),
            ));
        }
        kernel.unknown(&mut given.iter().map(|(group, binding, _)| (*group, *binding)))?;
        // Each binding with the buffer given for it and that buffer's kind as it is bound,
        // the kinds of earlier bindings of the same buffer applied.
        let mut placed: Vec<(&Binding, BufferId, &wgpu::Buffer, Kind)> = Vec::new();
        for binding in kernel.bindings() {
            let id = *one_for(binding, given)?;
            let (buffer, held) = self.held(id)?;
            let kind = placed
                .iter()
                .rev()
                .find(|(_, earlier, _, _)| *earlier == id)
                .map_or(*held, |(_, _, _, kind)| *kind);
            kernel.fits(binding, buffer.size(), kind.given())?;
            if let Some((earlier, _, _, _)) = placed.iter().find(|(earlier, other, _, _)| {
                *other == id
                    && (earlier.access() == Access::ReadWrite
                        || binding.access() == Access::ReadWrite)
            }) {
                return Err(RunError::BadLayout {
                    group: binding.group(),
                    binding: binding.binding(),
                    why: format!(
                        "its buffer is also bound at group {} binding {}, and the kernel writes one of them",
                        earlier.group(),
                        earlier.binding()
                    ),
                });
            }
            placed.push((binding, id, buffer, kind.bound(binding.scalar())));
        }
        let buffers: Vec<&wgpu::Buffer> = placed.iter().map(|(_, _, buffer, _)| *buffer).collect();
        let gpu = self.gpu;
        let mut groups = Vec::new();
        let refused = gpu.scoped(&mut || groups = kernel.groups(&buffers));
        if let Some(why) = refused {
            return Err(gpu.refused(why));
        }
        let kinds: Vec<(usize, Kind)> = placed
            .iter()
            .map(|(_, id, _, kind)| (id.index, *kind))
            .collect();
        drop(buffers);
        drop(placed);
        for (position, (_, kind)) in self.held.iter_mut().enumerate() {
            if let Some((_, bound)) = kinds.iter().rev().find(|(index, _)| *index == position) {
                *kind = *bound;
            }
        }
        Ok(Bound {
            owner: self.owner,
            pipeline: kernel.pipeline.clone(),
            groups,
        })
    }

    /// Encodes `steps` in order into one submission, and submits it.
    ///
    /// # Errors
    ///
    /// Returns [`RunError::BadBuffer`] when a pass was bound to another set, a buffer copied
    /// was made by another set, a buffer is copied onto itself or onto one of another size,
    /// [`RunError::TooManyWorkgroups`] when a pass asks more workgroups in a dimension than
    /// the device dispatches, and [`RunError::Device`] when the device refuses the
    /// submission: all checked before anything is encoded.
    pub fn dispatch(&self, steps: &[Step<'_>]) -> Result<(), RunError> {
        let mut ready = Vec::with_capacity(steps.len());
        for step in steps {
            ready.push(match *step {
                Step::Pass(bound, workgroups) => {
                    if bound.owner != self.owner {
                        return Err(RunError::BadBuffer(
                            "a kernel bound to other buffers".to_owned(),
                        ));
                    }
                    self.gpu.dispatchable(workgroups)?;
                    Encoded::Pass(bound, workgroups)
                }
                Step::Copy { from, to } => {
                    let (source, _) = self.held(from)?;
                    let (target, _) = self.held(to)?;
                    if from == to {
                        return Err(RunError::BadBuffer(
                            "a buffer copied onto itself".to_owned(),
                        ));
                    }
                    if source.size() != target.size() {
                        return Err(RunError::BadBuffer(format!(
                            "a buffer of {} bytes copied onto one of {}",
                            source.size(),
                            target.size()
                        )));
                    }
                    Encoded::Copy(source, target)
                }
            });
        }
        let gpu = self.gpu;
        let refused = gpu.scoped(&mut || {
            let mut encoder = gpu
                .device
                .create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
            for step in &ready {
                match *step {
                    Encoded::Pass(bound, workgroups) => {
                        let mut pass =
                            encoder.begin_compute_pass(&wgpu::ComputePassDescriptor::default());
                        pass.set_pipeline(&bound.pipeline);
                        for (group, bind_group) in &bound.groups {
                            pass.set_bind_group(*group, bind_group, &[]);
                        }
                        pass.dispatch_workgroups(workgroups.0, workgroups.1, workgroups.2);
                    }
                    Encoded::Copy(source, target) => {
                        encoder.copy_buffer_to_buffer(source, 0, target, 0, source.size());
                    }
                }
            }
            let _submitted = gpu.queue.submit([encoder.finish()]);
        });
        refused.map_or(Ok(()), |why| Err(gpu.refused(why)))
    }

    /// The bytes of each buffer of `ids`, read back after every submission so far.
    ///
    /// # Errors
    ///
    /// Returns [`RunError::BadBuffer`] when an id was made by another set, and what
    /// [`Gpu::read`] returns.
    pub(crate) fn read(&self, ids: &[BufferId]) -> Result<Vec<Vec<u8>>, RunError> {
        let sources = ids
            .iter()
            .map(|id| self.held(*id).map(|(buffer, _)| buffer))
            .collect::<Result<Vec<_>, _>>()?;
        self.gpu.read(&sources)
    }

    /// The bytes of the buffer `id`, read back after every submission so far.
    ///
    /// # Errors
    ///
    /// Returns [`RunError::BadBuffer`] when `id` was made by another set,
    /// [`RunError::Device`] when the device refuses the copy, and [`RunError::ReadBack`]
    /// when it cannot be read back.
    pub fn bytes(&self, id: BufferId) -> Result<Vec<u8>, RunError> {
        self.read(&[id])
            .map(|mut read| read.pop().unwrap_or_default())
    }

    /// Compares the buffer `id`, read back after every submission so far, with the
    /// reference `expected`, within `tolerance`, as [`Outputs::check`](crate::Outputs::check)
    /// does: read as the scalar its writes and bindings made it, raw bytes comparing with any
    /// buffer.
    ///
    /// # Errors
    ///
    /// Returns [`Mismatch::Unread`] when the buffer cannot be read back (or was made by
    /// another set), [`Mismatch::Kinds`] when the reference is not of the buffer's scalar,
    /// and otherwise what [`compare`](crate::compare()) returns, a difference naming the
    /// adapter.
    pub fn check(
        &self,
        id: BufferId,
        expected: Values<'_>,
        tolerance: &Tolerance,
        inputs: Option<Inputs<'_>>,
    ) -> Result<(), Mismatch> {
        let (_, kind) = self.held(id).map_err(Mismatch::Unread)?;
        let bytes = self.bytes(id).map_err(Mismatch::Unread)?;
        checked(
            &bytes,
            *kind,
            expected,
            tolerance,
            inputs,
            &self.gpu.adapter,
        )
    }
}

/// A step checked, its buffers found.
#[derive(Debug, Clone, Copy)]
enum Encoded<'step> {
    /// A pass.
    Pass(&'step Bound, Workgroups),
    /// A copy of the first buffer onto the second.
    Copy(&'step wgpu::Buffer, &'step wgpu::Buffer),
}

/// The refusal of a buffer made by another set.
fn foreign() -> RunError {
    RunError::BadBuffer("a buffer of another set of buffers".to_owned())
}

/// Compares `bytes`, values of `kind`, with the reference `expected` within `tolerance`,
/// read as the reference's kind where the bytes' own is not known, a difference naming
/// `adapter`.
///
/// # Errors
///
/// Returns [`Mismatch::Kinds`] when the reference is not of the bytes' scalar (raw bytes
/// compare with any, and are the only reference of mixed members), and otherwise what
/// [`compare`](crate::compare()) returns.
pub(super) fn checked(
    bytes: &[u8],
    kind: Kind,
    expected: Values<'_>,
    tolerance: &Tolerance,
    inputs: Option<Inputs<'_>>,
    adapter: &str,
) -> Result<(), Mismatch> {
    let words = bytes.as_chunks::<4>().0;
    let u32s: Vec<u32>;
    let i32s: Vec<i32>;
    let f32s: Vec<f32>;
    let actual = match (expected, kind) {
        (Values::Bytes(_), _) => Values::Bytes(bytes),
        (Values::U32(_) | Values::I32(_) | Values::F32(_), Kind::Mixed) => {
            return Err(Mismatch::Kinds {
                actual: "mixed",
                expected: expected.kind(),
            });
        }
        (Values::U32(_), Kind::Unknown) | (_, Kind::One(Scalar::U32)) => {
            u32s = words.iter().copied().map(u32::from_le_bytes).collect();
            Values::U32(&u32s)
        }
        (Values::I32(_), Kind::Unknown) | (_, Kind::One(Scalar::I32)) => {
            i32s = words.iter().copied().map(i32::from_le_bytes).collect();
            Values::I32(&i32s)
        }
        (Values::F32(_), Kind::Unknown) | (_, Kind::One(Scalar::F32)) => {
            f32s = words
                .iter()
                .map(|word| f32::from_bits(u32::from_le_bytes(*word)))
                .collect();
            Values::F32(&f32s)
        }
    };
    comparing(actual, expected, tolerance, inputs, Some(adapter))
}
