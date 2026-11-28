//! The box contract across a module boundary (`spec/box-contract.md`, `spec/engine-design.md`
//! decision 25): what a box type compiled to its own module and the engine agree on. Public
//! through [`crate::host`] alone, which box code may not name: the bytes of a sample are a
//! raw float's, outside the math layer (decision 10).

#![cfg_attr(not(test), forbid(clippy::arithmetic_side_effects))]

use core::fmt;
use core::marker::PhantomData;

use crate::description::Description;
use crate::host::Raw as _;
use crate::{Binary32, Binary64, MAX_ENGINE_BLOCK, Sample, flush};

/// The alignment of every buffer handed to a box, in bytes: the widest vector considered,
/// 512 bits.
pub const BUFFER_ALIGNMENT: u32 = 64;

/// The version of the box contract this engine implements (`spec/box-contract.md`, section
/// 1), the one a module gives in its header.
pub const CONTRACT_VERSION: ContractVersion = ContractVersion::new(0, 0);

/// The versions this engine declares, as [`ContractVersion::check`] takes them: the last
/// minor version it knows of each major it supports, [`CONTRACT_VERSION`] alone before 1.0.
pub const SUPPORTED_VERSIONS: &[ContractVersion] = &[CONTRACT_VERSION];

/// A version of the box contract. From 1.0, a minor version only adds to the one before it;
/// before, nothing is promised between two versions.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ContractVersion {
    /// Versions of different majors do not understand each other.
    major: u16,
    /// Counts the additions within a major version.
    minor: u16,
}

impl ContractVersion {
    /// Version `major.minor`.
    #[must_use]
    pub const fn new(major: u16, minor: u16) -> Self {
        Self::made(major, minor)
    }

    /// [`ContractVersion::new`]'s body, a function of its own so that mutation testing reaches it:
    /// cargo-mutants never mutates a function named `new`.
    const fn made(major: u16, minor: u16) -> Self {
        Self { major, minor }
    }

    /// The version a module gives as one integer ([`ContractVersion::packed`]): every
    /// integer is one.
    #[must_use]
    pub const fn from_packed(packed: u32) -> Self {
        let [major_high, major_low, minor_high, minor_low] = packed.to_be_bytes();
        Self {
            major: u16::from_be_bytes([major_high, major_low]),
            minor: u16::from_be_bytes([minor_high, minor_low]),
        }
    }

    /// The version as it crosses the boundary: `major × 2^16 + minor`.
    #[must_use]
    pub const fn packed(self) -> u32 {
        let [major_high, major_low] = self.major.to_be_bytes();
        let [minor_high, minor_low] = self.minor.to_be_bytes();
        u32::from_be_bytes([major_high, major_low, minor_high, minor_low])
    }

    /// Its major number.
    #[must_use]
    pub const fn major(self) -> u16 {
        self.major
    }

    /// Its minor number.
    #[must_use]
    pub const fn minor(self) -> u16 {
        self.minor
    }

    /// Whether an engine declaring `declared`, the last minor version it knows of each major
    /// it supports, understands a module of this version.
    ///
    /// # Errors
    ///
    /// Returns the first of these that holds: [`VersionError::UnsupportedMajor`] when no
    /// declared version has this major, [`VersionError::Unfrozen`] when the major is 0 and
    /// this minor is not the last declared, [`VersionError::LaterMinor`] when this minor is
    /// later than it.
    pub fn check(self, declared: &[Self]) -> Result<(), VersionError> {
        let minors = declared
            .iter()
            .filter(|known| known.major == self.major)
            .map(|known| known.minor);
        let Some(last) = minors.max() else {
            return Err(VersionError::UnsupportedMajor { version: self });
        };
        if self.major == 0 && self.minor != last {
            return Err(VersionError::Unfrozen {
                version: self,
                known: last,
            });
        }
        if self.minor <= last {
            Ok(())
        } else {
            Err(VersionError::LaterMinor {
                version: self,
                last,
            })
        }
    }
}

impl fmt::Display for ContractVersion {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}.{}", self.major, self.minor)
    }
}

/// Why a module's contract version is refused ([`ContractVersion::check`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum VersionError {
    /// The engine supports no version of this major.
    UnsupportedMajor {
        /// The module's version.
        version: ContractVersion,
    },
    /// Before 1.0, the engine accepts one minor version alone.
    Unfrozen {
        /// The module's version.
        version: ContractVersion,
        /// The minor version of major 0 the engine knows.
        known: u16,
    },
    /// The module may need an addition this engine does not have.
    LaterMinor {
        /// The module's version.
        version: ContractVersion,
        /// The last minor version the engine knows of that major.
        last: u16,
    },
}

impl fmt::Display for VersionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {
            Self::UnsupportedMajor { version } => write!(
                f,
                "contract version {version}: this engine does not support major version {}",
                version.major
            ),
            Self::Unfrozen { version, known } => write!(
                f,
                "contract version {version}: before 1.0 this engine accepts 0.{known} alone"
            ),
            Self::LaterMinor { version, last } => write!(
                f,
                "contract version {version} is later than {}.{last}, the last minor version \
                 this engine knows",
                version.major
            ),
        }
    }
}

/// An error of its own, with no cause: a host passes it on as any other.
impl core::error::Error for VersionError {}

/// What a module says of itself, found in its memory before any of its code runs
/// (`spec/box-contract.md`, section 5).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Header<'memory> {
    /// The contract version it implements.
    version: ContractVersion,
    /// The bytes of its description.
    description: &'memory [u8],
}

impl<'memory> Header<'memory> {
    /// The header at byte `offset` of `memory`: the version, the length of the description,
    /// then the description. Neither is checked here beyond its bounds ([`ContractVersion::check`],
    /// [`crate::host::Description::read`]).
    ///
    /// # Errors
    ///
    /// Returns the first of these that holds: [`HeaderError::Header`] when its two integers
    /// end beyond `memory`, [`HeaderError::Description`] when the description does.
    pub fn read(offset: u32, memory: &'memory [u8]) -> Result<Self, HeaderError> {
        let bytes = bytes_held(memory.len());
        let (version, length, rest) = memory
            .get(address(u64::from(offset))..)
            .and_then(|header| {
                let (version, rest) = header.split_first_chunk()?;
                let (length, rest) = rest.split_first_chunk()?;
                Some((
                    u32::from_le_bytes(*version),
                    u32::from_le_bytes(*length),
                    rest,
                ))
            })
            .ok_or(HeaderError::Header {
                offset,
                memory: bytes,
            })?;
        let description =
            rest.get(..address(u64::from(length)))
                .ok_or_else(|| HeaderError::Description {
                    offset: u64::from(offset).saturating_add(HEADER_BYTES),
                    length,
                    memory: bytes,
                })?;
        Ok(Self {
            version: ContractVersion::from_packed(version),
            description,
        })
    }

    /// The contract version the module implements.
    #[must_use]
    pub const fn version(self) -> ContractVersion {
        self.version
    }

    /// The bytes of its description.
    #[must_use]
    pub const fn description_bytes(self) -> &'memory [u8] {
        self.description
    }
}

/// The bytes of a header's two integers, the version and the description's length.
const HEADER_BYTES: u64 = 8;

/// Why a module's header is refused ([`Header::read`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum HeaderError {
    /// Its two integers end beyond the memory.
    Header {
        /// Where the module says it is.
        offset: u32,
        /// The bytes of the memory.
        memory: u64,
    },
    /// The description ends beyond the memory.
    Description {
        /// Its first byte, after the header's two integers.
        offset: u64,
        /// Its length, as the header gives it.
        length: u32,
        /// The bytes of the memory.
        memory: u64,
    },
}

impl fmt::Display for HeaderError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {
            Self::Header { offset, memory } => write!(
                f,
                "the module's header, {HEADER_BYTES} bytes at byte {offset}, ends before byte \
                 {}, beyond a memory of {memory} byte(s)",
                u64::from(offset).saturating_add(HEADER_BYTES)
            ),
            Self::Description {
                offset,
                length,
                memory,
            } => write!(
                f,
                "the module's description, {length} byte(s) at byte {offset}, ends before byte \
                 {}, beyond a memory of {memory} byte(s)",
                offset.saturating_add(u64::from(length))
            ),
        }
    }
}

/// An error of its own, with no cause: a host passes it on as any other.
impl core::error::Error for HeaderError {}

/// Where a run of buffers lies in a module's memory: one buffer per audio port, each holding a
/// span of samples `S`, starting on [`BUFFER_ALIGNMENT`] and padded to a multiple of it.
///
/// Inside its memory by construction: its one constructor checks it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BufferRun<S> {
    /// First byte of the run.
    offset: u32,
    /// Samples of each buffer.
    frames: u32,
    /// How many buffers.
    buffers: u32,
    /// The samples it holds.
    sample: PhantomData<S>,
}

impl<S: Sample> BufferRun<S> {
    /// A run of `buffers` buffers of `frames` samples from byte `offset` of a memory of
    /// `memory` bytes.
    ///
    /// # Errors
    ///
    /// Returns the first that holds: [`RunError::Misaligned`] unless `offset` is a multiple
    /// of [`BUFFER_ALIGNMENT`], [`RunError::Span`] unless `frames` is 1 to
    /// [`MAX_ENGINE_BLOCK`], [`RunError::OutsideMemory`] when the run ends beyond `memory`.
    pub fn new(offset: u32, frames: u32, buffers: u32, memory: u64) -> Result<Self, RunError> {
        Self::checked(offset, frames, buffers, memory)
    }

    /// [`BufferRun::new`]'s body, a function of its own so that mutation testing reaches it:
    /// cargo-mutants never mutates a function named `new`.
    ///
    /// # Errors
    ///
    /// As [`BufferRun::new`].
    fn checked(offset: u32, frames: u32, buffers: u32, memory: u64) -> Result<Self, RunError> {
        if !offset.is_multiple_of(BUFFER_ALIGNMENT) {
            return Err(RunError::Misaligned { offset });
        }
        if !usize::try_from(frames).is_ok_and(|span| (1..=MAX_ENGINE_BLOCK).contains(&span)) {
            return Err(RunError::Span { frames });
        }
        let run = Self {
            offset,
            frames,
            buffers,
            sample: PhantomData,
        };
        let end = run.end();
        if end <= memory {
            Ok(run)
        } else {
            Err(RunError::OutsideMemory { end, memory })
        }
    }

    /// Samples of each buffer.
    #[must_use]
    pub const fn frames(self) -> u32 {
        self.frames
    }

    /// How many buffers.
    #[must_use]
    pub const fn buffers(self) -> u32 {
        self.buffers
    }

    /// The bytes from one buffer to the next: those of a span, rounded up to a multiple of
    /// [`BUFFER_ALIGNMENT`].
    #[must_use]
    pub fn stride(self) -> u64 {
        self.span().next_multiple_of(u64::from(BUFFER_ALIGNMENT))
    }

    /// The first byte of buffer `index`, if the run has one.
    #[must_use]
    pub fn start(self, index: u32) -> Option<u64> {
        (index < self.buffers).then(|| self.after(index))
    }

    /// The byte after the run.
    #[must_use]
    pub fn end(self) -> u64 {
        self.after(self.buffers)
    }

    /// Its first byte and the byte after it.
    fn bytes(self) -> [u64; 2] {
        [u64::from(self.offset), self.end()]
    }

    /// The bytes of a span, without its padding.
    fn span(self) -> u64 {
        Self::bytes_of(self.frames)
    }

    /// The bytes of `frames` samples: at most 2^32 samples of 8 bytes, so never saturating.
    fn bytes_of(frames: u32) -> u64 {
        u64::from(frames).saturating_mul(u64::from(S::PRECISION.bits().div_euclid(8)))
    }

    /// Where buffer `index` lies in a memory, for a copy of `given` samples: its first byte,
    /// the byte after its padding, and the bytes the samples take from its start.
    fn slot(self, index: u32, given: usize) -> Result<(usize, usize, usize), CopyError> {
        let Some(start) = self.start(index) else {
            return Err(CopyError::NoBuffer {
                index,
                buffers: self.buffers,
            });
        };
        let Some(copied) = u32::try_from(given)
            .ok()
            .filter(|copied| (1..=self.frames).contains(copied))
        else {
            return Err(CopyError::Frames {
                given,
                span: self.frames,
            });
        };
        Ok((
            address(start),
            address(start.saturating_add(self.stride())),
            address(Self::bytes_of(copied)),
        ))
    }

    /// The byte after `buffers` buffers: below 2^32 + 2^32 × 2^23, so never saturating.
    fn after(self, buffers: u32) -> u64 {
        u64::from(self.offset).saturating_add(u64::from(buffers).saturating_mul(self.stride()))
    }
}

/// The two runs of buffers of a module's box, its inputs and its outputs.
///
/// As the module gives them after `poyetik_prepare` (`spec/box-contract.md`, section 5, step
/// 3): one buffer per audio port of its description, of the span prepared, the two not
/// overlapping.
///
/// Inside its memory and apart by construction: its one constructor checks them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Runs<S> {
    /// One buffer per inlet.
    inputs: BufferRun<S>,
    /// One buffer per outlet.
    outputs: BufferRun<S>,
}

impl<S: Sample> Runs<S> {
    /// The runs at bytes `inputs` and `outputs` of a memory of `memory` bytes, of `frames`
    /// frames each, as many buffers as `description` has inlets and outlets.
    ///
    /// # Errors
    ///
    /// Returns the first of these that holds: [`RunsError::Inputs`] when the inputs are not
    /// a run ([`BufferRun::new`]), [`RunsError::Outputs`] when the outputs are not,
    /// [`RunsError::Overlap`] when the two share a byte.
    pub fn new(
        description: &Description<'_>,
        [inputs, outputs]: [u32; 2],
        frames: u32,
        memory: u64,
    ) -> Result<Self, RunsError> {
        Self::checked(description, [inputs, outputs], frames, memory)
    }

    /// [`Runs::new`]'s body, a function of its own so that mutation testing reaches it:
    /// cargo-mutants never mutates a function named `new`.
    ///
    /// # Errors
    ///
    /// As [`Runs::new`].
    fn checked(
        description: &Description<'_>,
        [inputs, outputs]: [u32; 2],
        frames: u32,
        memory: u64,
    ) -> Result<Self, RunsError> {
        let inputs = BufferRun::new(inputs, frames, count(description.inlets().len()), memory)
            .map_err(RunsError::Inputs)?;
        let outputs = BufferRun::new(outputs, frames, count(description.outlets().len()), memory)
            .map_err(RunsError::Outputs)?;
        let [taken, given] = [inputs, outputs].map(BufferRun::bytes);
        // Two runs of bytes share one when the later start is before the earlier end: never
        // when either is empty, even inside the other.
        if taken[0].max(given[0]) < taken[1].min(given[1]) {
            Err(RunsError::Overlap {
                inputs: taken,
                outputs: given,
            })
        } else {
            Ok(Self { inputs, outputs })
        }
    }

    /// The run of its input buffers.
    #[must_use]
    pub const fn inputs(self) -> BufferRun<S> {
        self.inputs
    }

    /// The run of its output buffers.
    #[must_use]
    pub const fn outputs(self) -> BufferRun<S> {
        self.outputs
    }
}

/// The count of a list of ports: at most [`crate::host::PORTS_MAX`], so never saturating.
fn count(ports: usize) -> u32 {
    u32::try_from(ports).unwrap_or(u32::MAX)
}

/// Why a module's runs of buffers are refused ([`Runs::new`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum RunsError {
    /// The inputs are not a run.
    Inputs(RunError),
    /// The outputs are not a run.
    Outputs(RunError),
    /// The two share a byte.
    Overlap {
        /// The first byte of the inputs and the byte after them.
        inputs: [u64; 2],
        /// The first byte of the outputs and the byte after them.
        outputs: [u64; 2],
    },
}

impl fmt::Display for RunsError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {
            Self::Inputs(run) => write!(f, "the inputs: {run}"),
            Self::Outputs(run) => write!(f, "the outputs: {run}"),
            Self::Overlap {
                inputs: [first, end],
                outputs: [start, after],
            } => write!(
                f,
                "the inputs, bytes {first} to before {end}, overlap the outputs, bytes {start} \
                 to before {after}"
            ),
        }
    }
}

/// An error of its own, with no cause: its message holds a refused run's.
impl core::error::Error for RunsError {}

/// A sample that crosses the boundary: its bytes in a module's memory, little-endian.
pub trait Crossing: Sample {
    /// Writes `samples` from the start of `bytes`, as many as fit, with their bits.
    fn put(samples: &[Self], bytes: &mut [u8]);

    /// Reads `samples` from the start of `bytes`, as many as it holds, every NaN canonical
    /// and every subnormal number a zero of its sign (`spec/box-contract.md`, section 3): the
    /// flush every box output has (`spec/engine-design.md`, decision 11), which the engine
    /// does not trust a module to do.
    fn take(bytes: &[u8], samples: &mut [Self]);
}

impl Crossing for Binary64 {
    fn put(samples: &[Self], bytes: &mut [u8]) {
        for (held, sample) in bytes.as_chunks_mut().0.iter_mut().zip(samples) {
            *held = sample.raw().to_le_bytes();
        }
    }

    fn take(bytes: &[u8], samples: &mut [Self]) {
        for (sample, held) in samples.iter_mut().zip(bytes.as_chunks().0) {
            let bits = flush::canonical_bits_64(u64::from_le_bytes(*held));
            *sample = Self::from_raw(f64::from_bits(bits)).flush();
        }
    }
}

impl Crossing for Binary32 {
    fn put(samples: &[Self], bytes: &mut [u8]) {
        for (held, sample) in bytes.as_chunks_mut().0.iter_mut().zip(samples) {
            *held = sample.raw().to_le_bytes();
        }
    }

    fn take(bytes: &[u8], samples: &mut [Self]) {
        for (sample, held) in samples.iter_mut().zip(bytes.as_chunks().0) {
            let bits = flush::canonical_bits_32(u32::from_le_bytes(*held));
            *sample = Self::from_raw(f32::from_bits(bits)).flush();
        }
    }
}

impl<S: Crossing> BufferRun<S> {
    /// Writes `samples`, the first frames of a span, into buffer `index` of the run in
    /// `memory`, with their bits, then zeros to the end of its stride (`spec/box-contract.md`,
    /// section 5: a call processes 1 frame to a span).
    ///
    /// # Errors
    ///
    /// Returns the first that holds, `memory` left as it is: [`CopyError::NoBuffer`]
    /// unless the run has that buffer, [`CopyError::Frames`] unless `samples` holds 1 frame
    /// to the run's span, [`CopyError::ShorterMemory`] when `memory` does not hold the
    /// buffer.
    pub fn write(self, index: u32, samples: &[S], memory: &mut [u8]) -> Result<(), CopyError> {
        let (first, end, written) = self.slot(index, samples.len())?;
        let bytes = bytes_held(memory.len());
        let buffer = memory
            .get_mut(first..end)
            .ok_or_else(|| shorter(bytes, end))?;
        S::put(samples, buffer);
        for rest in buffer.iter_mut().skip(written) {
            *rest = 0;
        }
        Ok(())
    }

    /// Reads the first frames of buffer `index` of the run in `memory`, as many as `samples`
    /// holds, into `samples`, every NaN made canonical and every subnormal number a zero of its
    /// sign ([`Crossing::take`]), never the rest of the buffer.
    ///
    /// # Errors
    ///
    /// Those of [`BufferRun::write`], `samples` left as they are.
    pub fn read(self, index: u32, memory: &[u8], samples: &mut [S]) -> Result<(), CopyError> {
        let (first, end, _) = self.slot(index, samples.len())?;
        let buffer = memory
            .get(first..end)
            .ok_or_else(|| shorter(bytes_held(memory.len()), end))?;
        S::take(buffer, samples);
        Ok(())
    }
}

/// The bytes of a memory given as a slice: at most 2^64 on every target.
fn bytes_held(length: usize) -> u64 {
    u64::try_from(length).unwrap_or(u64::MAX)
}

/// The refusal of a memory of `memory` bytes for a buffer that ends before byte `end`.
fn shorter(memory: u64, end: usize) -> CopyError {
    CopyError::ShorterMemory {
        memory,
        end: bytes_held(end),
    }
}

/// A byte of a module's memory as an address of this machine: the largest address for one
/// beyond its width, which no slice reaches.
pub(crate) fn address(byte: u64) -> usize {
    usize::try_from(byte).unwrap_or(usize::MAX)
}

/// Why a copy into or out of a run of buffers is refused ([`BufferRun::write`],
/// [`BufferRun::read`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum CopyError {
    /// The run has no such buffer.
    NoBuffer {
        /// The buffer asked for.
        index: u32,
        /// How many buffers the run has.
        buffers: u32,
    },
    /// The samples given are none or more than the run's span.
    Frames {
        /// How many were given.
        given: usize,
        /// The frames of the run's span.
        span: u32,
    },
    /// The memory given does not hold the buffer: another memory than the run's, or a part
    /// of it.
    ShorterMemory {
        /// The bytes of the memory.
        memory: u64,
        /// The byte the buffer, its padding included, ends before.
        end: u64,
    },
}

impl fmt::Display for CopyError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {
            Self::NoBuffer { index, buffers } => {
                write!(f, "a run of {buffers} buffer(s) has no buffer {index}")
            }
            Self::Frames { given, span } => write!(
                f,
                "a copy of {given} frames, outside 1 to the {span} frame(s) of its run of buffers"
            ),
            Self::ShorterMemory { memory, end } => write!(
                f,
                "a buffer ends before byte {end}, beyond a memory of {memory} byte(s)"
            ),
        }
    }
}

/// An error of its own, with no cause: a host passes it on as any other.
impl core::error::Error for CopyError {}

/// Why a run of buffers is refused ([`BufferRun::new`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum RunError {
    /// It does not start on [`BUFFER_ALIGNMENT`].
    Misaligned {
        /// Where it starts.
        offset: u32,
    },
    /// Its span is no engine block's: outside 1 to [`MAX_ENGINE_BLOCK`] frames.
    Span {
        /// The frames asked for.
        frames: u32,
    },
    /// It ends beyond its memory.
    OutsideMemory {
        /// The byte after it.
        end: u64,
        /// The bytes of the memory.
        memory: u64,
    },
}

impl fmt::Display for RunError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {
            Self::Misaligned { offset } => write!(
                f,
                "a run of buffers starts at byte {offset}, not a multiple of {BUFFER_ALIGNMENT}"
            ),
            Self::Span { frames } => write!(
                f,
                "a run of buffers has a span of {frames} frames, outside 1 to {MAX_ENGINE_BLOCK}"
            ),
            Self::OutsideMemory { end, memory } => write!(
                f,
                "a run of buffers ends before byte {end}, beyond a memory of {memory} byte(s)"
            ),
        }
    }
}

/// An error of its own, with no cause: a host passes it on as any other.
impl core::error::Error for RunError {}

#[cfg(test)]
#[path = "abi_tests.rs"]
mod tests;
