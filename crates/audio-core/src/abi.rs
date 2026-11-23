//! The box contract across a module boundary (`spec/box-contract.md`, `spec/engine-design.md`
//! decision 25): what a box type compiled to its own module and the engine agree on. Public
//! through [`crate::host`] alone, which box code may not name: the bytes of a sample are a
//! raw float's, outside the math layer (decision 10).

#![cfg_attr(not(test), forbid(clippy::arithmetic_side_effects))]

use core::fmt;
use core::marker::PhantomData;

use crate::host::Raw as _;
use crate::{Binary32, Binary64, MAX_ENGINE_BLOCK, Sample, flush};

/// The alignment of every buffer handed to a box, in bytes: the widest vector considered,
/// 512 bits.
pub const BUFFER_ALIGNMENT: u32 = 64;

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
    /// Returns [`VersionRefusal::UnsupportedMajor`] when no declared version has this major,
    /// [`VersionRefusal::Unfrozen`] when the major is 0 and this minor is not the one
    /// declared, and [`VersionRefusal::LaterMinor`] when this minor is later than it.
    pub fn check(self, declared: &[Self]) -> Result<(), VersionRefusal> {
        let Some(known) = declared.iter().find(|known| known.major == self.major) else {
            return Err(VersionRefusal::UnsupportedMajor { version: self });
        };
        if self.major == 0 && self.minor != known.minor {
            return Err(VersionRefusal::Unfrozen {
                version: self,
                known: known.minor,
            });
        }
        if self.minor <= known.minor {
            Ok(())
        } else {
            Err(VersionRefusal::LaterMinor {
                version: self,
                last: known.minor,
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
pub enum VersionRefusal {
    /// The engine supports no version of this major.
    UnsupportedMajor {
        /// The module's version.
        version: ContractVersion,
    },
    /// Before 1.0, the engine knows one minor version alone.
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

impl fmt::Display for VersionRefusal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {
            Self::UnsupportedMajor { version } => write!(
                f,
                "contract version {version}: this engine does not support major version {}",
                version.major
            ),
            Self::Unfrozen { version, known } => write!(
                f,
                "contract version {version}: before 1.0 this engine knows 0.{known} alone"
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
impl core::error::Error for VersionRefusal {}

/// What a module says of itself, found in its memory before any of its code runs
/// (`spec/box-contract.md`, section 5).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Header<'memory> {
    /// The contract version it implements.
    version: ContractVersion,
    /// The bytes of its metadata.
    metadata: &'memory [u8],
}

impl<'memory> Header<'memory> {
    /// The header at byte `offset` of `memory`: the version, the length of the metadata, then
    /// the metadata. Neither is checked here beyond its bounds ([`ContractVersion::check`],
    /// [`crate::host::Metadata::read`]).
    ///
    /// # Errors
    ///
    /// Returns [`HeaderRefusal::Header`] when its two integers end beyond `memory`, then
    /// [`HeaderRefusal::Metadata`] when the metadata do.
    pub fn read(offset: u32, memory: &'memory [u8]) -> Result<Self, HeaderRefusal> {
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
            .ok_or(HeaderRefusal::Header { offset })?;
        let metadata = rest
            .get(..address(u64::from(length)))
            .ok_or(HeaderRefusal::Metadata { length })?;
        Ok(Self {
            version: ContractVersion::from_packed(version),
            metadata,
        })
    }

    /// The contract version the module implements.
    #[must_use]
    pub const fn version(self) -> ContractVersion {
        self.version
    }

    /// The bytes of its metadata.
    #[must_use]
    pub const fn metadata(self) -> &'memory [u8] {
        self.metadata
    }
}

/// Why a module's header is refused ([`Header::read`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum HeaderRefusal {
    /// Its two integers end beyond the memory.
    Header {
        /// Where the module says it is.
        offset: u32,
    },
    /// The metadata end beyond the memory.
    Metadata {
        /// Their length, as the header gives it.
        length: u32,
    },
}

impl fmt::Display for HeaderRefusal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {
            Self::Header { offset } => write!(
                f,
                "the module's header, at byte {offset}, ends beyond its memory"
            ),
            Self::Metadata { length } => write!(
                f,
                "the module's metadata, of {length} bytes, end beyond its memory"
            ),
        }
    }
}

/// An error of its own, with no cause: a host passes it on as any other.
impl core::error::Error for HeaderRefusal {}

/// Where a run of buffers lies in a module's memory: one buffer per port, each holding a
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
    /// Returns the first that holds: [`RunRefusal::Misaligned`] unless `offset` is a multiple
    /// of [`BUFFER_ALIGNMENT`], [`RunRefusal::Span`] unless `frames` is 1 to
    /// [`MAX_ENGINE_BLOCK`], [`RunRefusal::OutsideMemory`] when the run ends beyond `memory`.
    pub fn new(offset: u32, frames: u32, buffers: u32, memory: u64) -> Result<Self, RunRefusal> {
        if !offset.is_multiple_of(BUFFER_ALIGNMENT) {
            return Err(RunRefusal::Misaligned { offset });
        }
        if !usize::try_from(frames).is_ok_and(|span| (1..=MAX_ENGINE_BLOCK).contains(&span)) {
            return Err(RunRefusal::Span { frames });
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
            Err(RunRefusal::OutsideMemory { end, memory })
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

    /// The bytes of a span, without its padding: at most 2^20 samples of 8 bytes, so never
    /// saturating.
    fn span(self) -> u64 {
        u64::from(self.frames).saturating_mul(u64::from(S::PRECISION.bits().div_euclid(8)))
    }

    /// Where buffer `index` lies in a memory, for a copy of `given` samples: its first byte
    /// and the byte after its padding.
    fn slot(self, index: u32, given: usize) -> Result<(usize, usize), CopyRefusal> {
        let Some(start) = self.start(index) else {
            return Err(CopyRefusal::NoBuffer { index });
        };
        if address(u64::from(self.frames)) != given {
            return Err(CopyRefusal::Frames { given });
        }
        Ok((address(start), address(start.saturating_add(self.stride()))))
    }

    /// The byte after `buffers` buffers: below 2^32 + 2^32 × 2^23, so never saturating.
    fn after(self, buffers: u32) -> u64 {
        u64::from(self.offset).saturating_add(u64::from(buffers).saturating_mul(self.stride()))
    }
}

/// A sample that crosses the boundary: its bytes in a module's memory, little-endian.
pub trait Crossing: Sample {
    /// Writes `samples` from the start of `bytes`, as many as fit, with their bits.
    fn put(samples: &[Self], bytes: &mut [u8]);

    /// Reads `samples` from the start of `bytes`, as many as it holds, every NaN canonical
    /// (`spec/box-contract.md`, section 3).
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
            *sample = Self::from_raw(f64::from_bits(bits));
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
            *sample = Self::from_raw(f32::from_bits(bits));
        }
    }
}

impl<S: Crossing> BufferRun<S> {
    /// Writes `samples` into buffer `index` of the run in `memory`, with their bits, then
    /// zeros to the end of its stride.
    ///
    /// # Errors
    ///
    /// Returns the first that holds, `memory` left as it is: [`CopyRefusal::NoBuffer`]
    /// unless the run has that buffer, [`CopyRefusal::Frames`] unless `samples` is one span
    /// of it, [`CopyRefusal::ShorterMemory`] when `memory` does not hold the buffer.
    pub fn write(self, index: u32, samples: &[S], memory: &mut [u8]) -> Result<(), CopyRefusal> {
        let (first, end) = self.slot(index, samples.len())?;
        let bytes = memory.len();
        let buffer = memory
            .get_mut(first..end)
            .ok_or(CopyRefusal::ShorterMemory { bytes })?;
        S::put(samples, buffer);
        for padding in buffer.iter_mut().skip(address(self.span())) {
            *padding = 0;
        }
        Ok(())
    }

    /// Reads buffer `index` of the run in `memory` into `samples`, every NaN made canonical,
    /// never its padding.
    ///
    /// # Errors
    ///
    /// Those of [`BufferRun::write`], `samples` left as they are.
    pub fn read(self, index: u32, memory: &[u8], samples: &mut [S]) -> Result<(), CopyRefusal> {
        let (first, end) = self.slot(index, samples.len())?;
        let bytes = memory.len();
        let buffer = memory
            .get(first..end)
            .ok_or(CopyRefusal::ShorterMemory { bytes })?;
        S::take(buffer, samples);
        Ok(())
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
pub enum CopyRefusal {
    /// The run has no such buffer.
    NoBuffer {
        /// The buffer asked for.
        index: u32,
    },
    /// The samples given are not one span of the run.
    Frames {
        /// How many were given.
        given: usize,
    },
    /// The memory given does not hold the buffer: another memory than the run's, or a part
    /// of it.
    ShorterMemory {
        /// Its bytes.
        bytes: usize,
    },
}

impl fmt::Display for CopyRefusal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {
            Self::NoBuffer { index } => write!(f, "a run of buffers has no buffer {index}"),
            Self::Frames { given } => write!(
                f,
                "a copy of {given} samples is not the span of its run of buffers"
            ),
            Self::ShorterMemory { bytes } => {
                write!(f, "a memory of {bytes} bytes does not hold the buffer")
            }
        }
    }
}

/// An error of its own, with no cause: a host passes it on as any other.
impl core::error::Error for CopyRefusal {}

/// Why a run of buffers is refused ([`BufferRun::new`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum RunRefusal {
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

impl fmt::Display for RunRefusal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {
            Self::Misaligned { offset } => write!(
                f,
                "a run of buffers starts at byte {offset}, not a multiple of {BUFFER_ALIGNMENT}"
            ),
            Self::Span { frames } => write!(
                f,
                "a span of {frames} frames, outside 1 to {MAX_ENGINE_BLOCK}"
            ),
            Self::OutsideMemory { end, memory } => write!(
                f,
                "a run of buffers ends at byte {end}, beyond a memory of {memory}"
            ),
        }
    }
}

/// An error of its own, with no cause: a host passes it on as any other.
impl core::error::Error for RunRefusal {}

#[cfg(test)]
#[path = "abi_tests.rs"]
mod tests;
