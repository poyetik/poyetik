//! The description a module gives of its box type (`spec/box-contract.md`, section 4), read
//! whole and checked before anything of the module runs.

#![cfg_attr(not(test), forbid(clippy::arithmetic_side_effects))]

use alloc::collections::BTreeSet;
use alloc::vec;
use alloc::vec::Vec;
use core::fmt;
use core::num::NonZeroU32;

use crate::abi::address;
use crate::object::{RangeFault, range_rule};
use crate::{BoxMode, Capability};

/// The longest description a module may give, in bytes: the size a patch document may have
/// (`spec/patch-format.md`). Provisional (`spec/box-contract.md`, section 4).
pub const DESCRIPTION_BYTES_MAX: u32 = 16_777_216;

/// What a port carries: its kind.
///
/// Its code fixes the fields after it in a description (`spec/box-contract.md`, section 4).
/// The one kind of this version is an audio signal, with no field; a later minor version
/// adds a kind, every other code being refused: [`RESERVED_PORT_KINDS`] as reserved, the
/// others as unknown.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum PortKind {
    /// One sample per frame.
    Signal,
}

/// The codes of a port kind reserved for a later minor version of the box contract.
///
/// 1 for audio sub-sample discontinuities (decision 31), whose fields, bound and buffers
/// #12 step 19 defines, and 2 for spectral frames (decision 27), defined by #12 step 42.
pub const RESERVED_PORT_KINDS: [u8; 2] = [1, 2];

/// How a box's outputs depend on its inputs of the same frame, for delay-free loops
/// (decision 3).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum Response {
    /// They do not, or the box does not say how.
    None,
    /// Linearly.
    Linear,
    /// Non-linearly, the box giving its derivative.
    NonLinear,
}

/// A port of a described box type.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DescribedPort<'bytes> {
    /// Its name.
    name: &'bytes str,
    /// What flows through it.
    description: &'bytes str,
    /// What it carries.
    kind: PortKind,
}

impl<'bytes> DescribedPort<'bytes> {
    /// Its name.
    #[must_use]
    pub const fn name(&self) -> &'bytes str {
        self.name
    }

    /// What flows through it.
    #[must_use]
    pub const fn description(&self) -> &'bytes str {
        self.description
    }

    /// What it carries.
    #[must_use]
    pub const fn kind(&self) -> PortKind {
        self.kind
    }
}

/// A parameter of a described box type, its values in binary64 (decision 10), its range one
/// its default is in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DescribedParam<'bytes> {
    /// Its name.
    name: &'bytes str,
    /// What it controls.
    description: &'bytes str,
    /// The binary64 bits of its default, its minimum and its maximum.
    bits: [u64; 3],
}

impl<'bytes> DescribedParam<'bytes> {
    /// Its name.
    #[must_use]
    pub const fn name(&self) -> &'bytes str {
        self.name
    }

    /// What it controls.
    #[must_use]
    pub const fn description(&self) -> &'bytes str {
        self.description
    }

    /// The binary64 bits of its default, its minimum and its maximum, as written.
    #[must_use]
    pub const fn bits(&self) -> [u64; 3] {
        self.bits
    }
}

/// The description of a box type, borrowed from a module's bytes. Built only by
/// [`Description::read`], so every rule of the format holds.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Description<'bytes> {
    /// The version of its algorithm (decision 29).
    algorithm_version: NonZeroU32,
    /// The execution modes it may run in.
    mode: BoxMode,
    /// Its instantaneous response.
    response: Response,
    /// Its name, used in patches.
    name: &'bytes str,
    /// Its description text: what it does.
    text: &'bytes str,
    /// Its inlets, in order.
    inlets: Vec<DescribedPort<'bytes>>,
    /// Its outlets, in order.
    outlets: Vec<DescribedPort<'bytes>>,
    /// Its parameters, in order.
    params: Vec<DescribedParam<'bytes>>,
    /// What it asks of the host.
    capabilities: Vec<Capability>,
}

impl<'bytes> Description<'bytes> {
    /// Reads the description `bytes` hold, all of them.
    ///
    /// # Errors
    ///
    /// Returns the first field refused, in the order of the format, and why ([`Fault`]).
    pub fn read(bytes: &'bytes [u8]) -> Result<Self, DescriptionError> {
        let mut reader = Reader {
            bytes,
            field: Field::Whole,
        };
        if !u32::try_from(bytes.len()).is_ok_and(|length| length <= DESCRIPTION_BYTES_MAX) {
            return Err(reader.refusal(Fault::TooLong));
        }
        reader.field = Field::AlgorithmVersion;
        let algorithm_version = NonZeroU32::new(u32::from_le_bytes(reader.take()?))
            .ok_or_else(|| reader.refusal(Fault::Zero))?;
        reader.field = Field::Mode;
        let mode = reader.code(|code| match code {
            0 => Some(BoxMode::LiveCapable),
            1 => Some(BoxMode::OfflineOnly),
            _ => None,
        })?;
        reader.field = Field::Response;
        let response = reader.code(|code| match code {
            0 => Some(Response::None),
            1 => Some(Response::Linear),
            2 => Some(Response::NonLinear),
            _ => None,
        })?;
        reader.field = Field::Name;
        let name = reader.text()?;
        if name.contains(char::is_whitespace) {
            return Err(reader.refusal(Fault::Whitespace));
        }
        reader.field = Field::DescriptionText;
        let text = reader.text()?;
        reader.field = Field::Inlets;
        let inlets = reader.ports()?;
        reader.field = Field::Outlets;
        let outlets = reader.ports()?;
        reader.field = Field::Params;
        let params = reader.list(Reader::param, |param| param.name)?;
        reader.field = Field::Capabilities;
        let capabilities = reader.list(
            |item| {
                item.code(|code| match code {
                    0 => Some(Capability::AudioOutput),
                    _ => None,
                })
            },
            |&capability| capability,
        )?;
        reader.field = Field::End;
        if reader.bytes.is_empty() {
            Ok(Self {
                algorithm_version,
                mode,
                response,
                name,
                text,
                inlets,
                outlets,
                params,
                capabilities,
            })
        } else {
            Err(reader.refusal(Fault::Trailing))
        }
    }

    /// The version of its algorithm (decision 29).
    #[must_use]
    pub const fn algorithm_version(&self) -> NonZeroU32 {
        self.algorithm_version
    }

    /// The execution modes it may run in.
    #[must_use]
    pub const fn mode(&self) -> BoxMode {
        self.mode
    }

    /// Its instantaneous response.
    #[must_use]
    pub const fn response(&self) -> Response {
        self.response
    }

    /// Its name, used in patches.
    #[must_use]
    pub const fn name(&self) -> &'bytes str {
        self.name
    }

    /// Its description text: what it does.
    #[must_use]
    pub const fn description(&self) -> &'bytes str {
        self.text
    }

    /// Its inlets, in order.
    #[must_use]
    pub fn inlets(&self) -> &[DescribedPort<'bytes>] {
        &self.inlets
    }

    /// Its outlets, in order.
    #[must_use]
    pub fn outlets(&self) -> &[DescribedPort<'bytes>] {
        &self.outlets
    }

    /// Its parameters, in order.
    #[must_use]
    pub fn params(&self) -> &[DescribedParam<'bytes>] {
        &self.params
    }

    /// What it asks of the host.
    #[must_use]
    pub fn capabilities(&self) -> &[Capability] {
        &self.capabilities
    }
}

/// The bytes of a description not read yet, and the field being read.
struct Reader<'bytes> {
    /// What is left.
    bytes: &'bytes [u8],
    /// The field a refusal names.
    field: Field,
}

impl<'bytes> Reader<'bytes> {
    /// A refusal of the field being read.
    const fn refusal(&self, fault: Fault) -> DescriptionError {
        DescriptionError {
            field: self.field,
            fault,
        }
    }

    /// The next `N` bytes.
    fn take<const N: usize>(&mut self) -> Result<[u8; N], DescriptionError> {
        let (taken, rest) = self
            .bytes
            .split_first_chunk()
            .ok_or_else(|| self.refusal(Fault::Truncated))?;
        self.bytes = rest;
        Ok(*taken)
    }

    /// The next 8 bits, as what `known` says they are.
    fn code<T>(&mut self, known: impl Fn(u8) -> Option<T>) -> Result<T, DescriptionError> {
        let [code] = self.take()?;
        known(code).ok_or_else(|| self.refusal(Fault::Code(code)))
    }

    /// The next port kind, a code [`RESERVED_PORT_KINDS`] holds refused as reserved.
    fn port_kind(&mut self) -> Result<PortKind, DescriptionError> {
        let [code] = self.take()?;
        if code == 0 {
            Ok(PortKind::Signal)
        } else if RESERVED_PORT_KINDS.contains(&code) {
            Err(self.refusal(Fault::Reserved(code)))
        } else {
            Err(self.refusal(Fault::Code(code)))
        }
    }

    /// The next text, not blank: borrowed, so that the length announced reserves nothing.
    fn text(&mut self) -> Result<&'bytes str, DescriptionError> {
        let length = u64::from(u32::from_le_bytes(self.take()?));
        let (taken, rest) = self
            .bytes
            .split_at_checked(address(length))
            .ok_or_else(|| self.refusal(Fault::Truncated))?;
        self.bytes = rest;
        let text = core::str::from_utf8(taken).map_err(|_| self.refusal(Fault::NotUtf8))?;
        if text.trim().is_empty() {
            Err(self.refusal(Fault::Blank))
        } else {
            Ok(text)
        }
    }

    /// The next list, each item read by `item`, none repeating another's `key`. It grows with
    /// the items read, never by the count announced, and every item takes a byte at least, so
    /// the bytes bound it.
    fn list<T, K: Ord>(
        &mut self,
        item: impl Fn(&mut Self) -> Result<T, DescriptionError>,
        key: impl Fn(&T) -> K,
    ) -> Result<Vec<T>, DescriptionError> {
        let count = u32::from_le_bytes(self.take()?);
        let mut keys = BTreeSet::new();
        let mut items = vec![];
        for _ in 0..count {
            let read = item(self)?;
            if !keys.insert(key(&read)) {
                return Err(self.refusal(Fault::Repeated));
            }
            items.push(read);
        }
        Ok(items)
    }

    /// The next list of ports.
    fn ports(&mut self) -> Result<Vec<DescribedPort<'bytes>>, DescriptionError> {
        self.list(
            |reader| {
                Ok(DescribedPort {
                    name: reader.text()?,
                    description: reader.text()?,
                    kind: reader.port_kind()?,
                })
            },
            |port| port.name,
        )
    }

    /// The next parameter.
    fn param(&mut self) -> Result<DescribedParam<'bytes>, DescriptionError> {
        let name = self.text()?;
        let description = self.text()?;
        let bits = [
            u64::from_le_bytes(self.take()?),
            u64::from_le_bytes(self.take()?),
            u64::from_le_bytes(self.take()?),
        ];
        admissible(bits.map(f64::from_bits)).map_err(|fault| self.refusal(fault))?;
        Ok(DescribedParam {
            name,
            description,
            bits,
        })
    }
}

/// Checks the default, the minimum and the maximum of a parameter by the one rule of a
/// parameter's range ([`crate::object::range_rule`]), which a built-in box type's
/// [`crate::ParamSpec::new`] follows too.
const fn admissible([default_value, minimum, maximum]: [f64; 3]) -> Result<(), Fault> {
    match range_rule(default_value, minimum, maximum) {
        Ok(()) => Ok(()),
        Err(RangeFault::Range) => Err(Fault::Range),
        Err(RangeFault::Width) => Err(Fault::Width),
        Err(RangeFault::Default) => Err(Fault::Default),
    }
}

/// A field of a description, in the order of the format.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum Field {
    /// The whole description, before its first field.
    Whole,
    /// The version of the algorithm.
    AlgorithmVersion,
    /// The execution mode.
    Mode,
    /// The instantaneous response.
    Response,
    /// The name of the box type.
    Name,
    /// The description text of the box type: what it does, in a sentence.
    DescriptionText,
    /// The inlets.
    Inlets,
    /// The outlets.
    Outlets,
    /// The parameters.
    Params,
    /// The capabilities.
    Capabilities,
    /// After the last field.
    End,
}

impl fmt::Display for Field {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Whole => "description",
            Self::AlgorithmVersion => "algorithm version",
            Self::Mode => "execution mode",
            Self::Response => "instantaneous response",
            Self::Name => "name",
            Self::DescriptionText => "description text",
            Self::Inlets => "inlets",
            Self::Outlets => "outlets",
            Self::Params => "parameters",
            Self::Capabilities => "capabilities",
            Self::End => "end",
        })
    }
}

/// Why a field is refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum Fault {
    /// The bytes end before the field does.
    Truncated,
    /// Bytes follow the last field.
    Trailing,
    /// A number that is none of the field's codes.
    Code(u8),
    /// A port kind's code reserved for a later minor version ([`RESERVED_PORT_KINDS`]).
    Reserved(u8),
    /// An algorithm version of 0.
    Zero,
    /// More than [`DESCRIPTION_BYTES_MAX`] bytes.
    TooLong,
    /// A text that is not UTF-8.
    NotUtf8,
    /// A text that is empty or whitespace alone.
    Blank,
    /// Whitespace in the name of the box type.
    Whitespace,
    /// A name or a capability listed twice.
    Repeated,
    /// A parameter's range that is not finite, not in order, or with a bound no value reaches.
    Range,
    /// A parameter's range whose width, its maximum less its minimum, is beyond binary64.
    Width,
    /// A parameter's default outside its range, once flushed, or not a number.
    Default,
}

impl fmt::Display for Fault {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {
            Self::Truncated => f.write_str("the description ends before it does"),
            Self::Trailing => f.write_str("bytes follow the last field"),
            Self::Code(code) => write!(f, "{code} is not a code of the box contract"),
            Self::Reserved(code) => {
                write!(
                    f,
                    "{code} is reserved for a later version of the box contract"
                )
            }
            Self::Zero => f.write_str("is 0, versions count from 1"),
            Self::TooLong => write!(f, "longer than {DESCRIPTION_BYTES_MAX} bytes"),
            Self::NotUtf8 => f.write_str("a text is not UTF-8"),
            Self::Blank => f.write_str("is blank"),
            Self::Whitespace => f.write_str("holds whitespace"),
            Self::Repeated => f.write_str("an item repeats"),
            Self::Range => f.write_str("a range is not one a parameter may have"),
            Self::Width => f.write_str("a range is wider than the largest binary64 number"),
            Self::Default => f.write_str("a default is outside its range"),
        }
    }
}

/// Why a description is refused ([`Description::read`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[expect(
    clippy::exhaustive_structs,
    reason = "a refusal is the field and the fault, nothing else"
)]
pub struct DescriptionError {
    /// The first field refused.
    pub field: Field,
    /// Why.
    pub fault: Fault,
}

impl fmt::Display for DescriptionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.field, self.fault)
    }
}

/// An error of its own, with no cause: a host passes it on as any other.
impl core::error::Error for DescriptionError {}

#[cfg(test)]
#[path = "description_tests.rs"]
mod tests;
