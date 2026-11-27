//! The description a module gives of its box type (`spec/box-contract.md`, section 4), read
//! whole and checked before anything of the module runs.

#![cfg_attr(not(test), forbid(clippy::arithmetic_side_effects))]

use alloc::collections::BTreeMap;
use alloc::vec;
use alloc::vec::Vec;
use core::fmt;
use core::num::NonZeroU32;

use crate::abi::address;
use crate::math::Shown;
use crate::object::range_rule;
use crate::{Bound, RangeRule};
use crate::{BoxMode, Capability};

/// The longest description a module may give, in bytes: the size a patch document may have
/// (`spec/patch-format.md`). Provisional (`spec/box-contract.md`, section 4).
pub const DESCRIPTION_BYTES_MAX: u32 = 16_777_216;

/// The most inlets, and the most outlets, a description may list.
///
/// The ports a patch can name, whose port indices are 16-bit (`spec/patch-format.md`), so
/// that no port is out of a cable's reach and the buffers a plan gives a box stay bounded
/// by them.
pub const PORTS_MAX: u32 = 65_536;

/// What a text of a description is: a name, or a description text.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum TextKind {
    /// The name of a box type, which a patch writes as one word.
    BoxName,
    /// The name of a port or a parameter: one a patch or a cable names it by.
    Name,
    /// What a box type, a port or a parameter does, in a sentence.
    DescriptionText,
}

/// Why a description refuses `text`, of kind `kind`, if it does (`spec/box-contract.md`,
/// section 4); the rule of a module's description and of a built-in box type's alike.
///
/// The first of these that holds:
/// - [`Fault::Character`], its first character that would drive a terminal, forge a line or
///   reorder what is shown: a control character (C0, DEL and C1), a bidirectional formatting
///   character, or the line or paragraph separator, the characters a message escapes
///   ([`needs_escape`]); and in a name, one that shows nothing, so that
///   `gain` and `gain` followed by U+200B would read as one name: a default-ignorable code
///   point (Unicode's `Default_Ignorable_Code_Point`, as its version 14.0 lists it), U+2800,
///   the braille pattern without a dot, or whitespace other than U+0020 (`White_Space`, as
///   Unicode 14.0 lists it), which reads as one, or a format character that shows nothing
///   outside those (U+FFF9 to U+FFFB, U+13430 to U+13438);
/// - [`Fault::Blank`], a text that shows nothing: empty, or whitespace, default-ignorable
///   code points, U+2800 and those format characters alone;
/// - [`Fault::Whitespace`], U+0020 in the name of a box type;
/// - [`Fault::Spaces`], U+0020 at the start or the end of the name of a port or a parameter,
///   or after another, where it shows nothing or little.
#[must_use]
pub fn text_fault(text: &str, kind: TextKind) -> Option<Fault> {
    let refused = |character: char| {
        needs_escape(character)
            || (kind != TextKind::DescriptionText && character != ' ' && shows_nothing(character))
    };
    text.chars()
        .find(|&character| refused(character))
        .map(Fault::Character)
        .or_else(|| text.chars().all(shows_nothing).then_some(Fault::Blank))
        .or_else(|| match kind {
            TextKind::BoxName => text.contains(' ').then_some(Fault::Whitespace),
            TextKind::Name => (text.starts_with(' ') || text.ends_with(' ') || text.contains("  "))
                .then_some(Fault::Spaces),
            TextKind::DescriptionText => None,
        })
}

/// Whether `character` shows nothing: whitespace, a default-ignorable code point, U+2800, the
/// braille pattern without a dot, or a format character outside them that shows nothing, as
/// Unicode 14.0 lists them: U+FFF9 to U+FFFB, the interlinear annotation characters, and
/// U+13430 to U+13438, the Egyptian hieroglyph format controls (the other format characters,
/// such as U+0600 to U+0605, show a sign).
const fn shows_nothing(character: char) -> bool {
    white_space(character)
        || default_ignorable(character)
        || matches!(
            character,
            '\u{2800}' | '\u{fff9}'..='\u{fffb}' | '\u{13430}'..='\u{13438}'
        )
}

/// Whether `character` is whitespace (Unicode 14.0's `PropList.txt`, `White_Space`): the set
/// of the box contract, written here rather than taken from [`char::is_whitespace`], which
/// follows the Unicode version of the toolchain that builds it.
const fn white_space(character: char) -> bool {
    matches!(
        character,
        '\u{9}'..='\u{d}'
            | ' '
            | '\u{85}'
            | '\u{a0}'
            | '\u{1680}'
            | '\u{2000}'..='\u{200a}'
            | '\u{2028}'
            | '\u{2029}'
            | '\u{202f}'
            | '\u{205f}'
            | '\u{3000}'
    )
}

/// Whether `character` must not reach a terminal or a page as is.
///
/// A control character (C0, DEL and C1: a newline would forge a second line, ESC starts
/// terminal sequences), a bidirectional formatting character (it reorders what is shown), or
/// the line or paragraph separator. The characters a message escapes (`spec/safe-message-vectors.json`, its `unsafe` vector),
/// in one rule: `audio_patch::safe_message` escapes them by it, and [`text_fault`] refuses
/// them by it in every text of a description.
#[must_use]
pub const fn needs_escape(character: char) -> bool {
    character.is_control()
        || matches!(
            character,
            '\u{61c}'
                | '\u{200e}'
                | '\u{200f}'
                | '\u{202a}'..='\u{202e}'
                | '\u{2066}'..='\u{2069}'
                | '\u{2028}'
                | '\u{2029}'
        )
}

/// Whether `character` is a default-ignorable code point (Unicode 14.0's
/// `DerivedCoreProperties.txt`, `Default_Ignorable_Code_Point`).
const fn default_ignorable(character: char) -> bool {
    matches!(
        character,
        '\u{ad}'
            | '\u{34f}'
            | '\u{61c}'
            | '\u{115f}'..='\u{1160}'
            | '\u{17b4}'..='\u{17b5}'
            | '\u{180b}'..='\u{180f}'
            | '\u{200b}'..='\u{200f}'
            | '\u{202a}'..='\u{202e}'
            | '\u{2060}'..='\u{206f}'
            | '\u{3164}'
            | '\u{fe00}'..='\u{fe0f}'
            | '\u{feff}'
            | '\u{ffa0}'
            | '\u{fff0}'..='\u{fff8}'
            | '\u{1bca0}'..='\u{1bca3}'
            | '\u{1d173}'..='\u{1d17a}'
            | '\u{e0000}'..='\u{e0fff}'
    )
}

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
pub const RESERVED_PORT_KINDS: &[u8] = &[1, 2];

/// How a box's outputs depend on its inputs of the same frame, for delay-free loops
/// (decision 3).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum Response {
    /// The box declares none: a loop through it needs a delay (decision 3).
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

    /// The binary64 bits of its default, as written: a host flushes it as a plan flushes a
    /// value (below 2^-126 in magnitude, a zero of its sign) before it gives it to a module
    /// (`spec/box-contract.md`, section 5, step 1).
    #[must_use]
    pub const fn default_bits(&self) -> u64 {
        self.bits[0]
    }

    /// The binary64 bits of its minimum, as written.
    #[must_use]
    pub const fn minimum_bits(&self) -> u64 {
        self.bits[1]
    }

    /// The binary64 bits of its maximum, as written.
    #[must_use]
    pub const fn maximum_bits(&self) -> u64 {
        self.bits[2]
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
    /// Reads the description `bytes` hold, all of them, as [`crate::host::CONTRACT_VERSION`] writes
    /// it: that of a module whose version ([`crate::host::Header::version`]) is accepted, by
    /// [`crate::host::ContractVersion::check`]. Reading by a module's own version comes with the
    /// first minor version that adds a kind of port (roadmap #12 step 19).
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
            let given = u64::try_from(bytes.len()).unwrap_or(u64::MAX);
            return Err(reader.refusal(Fault::TooLong { bytes: given }));
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
        let name = reader.text(TextKind::BoxName)?;
        reader.field = Field::DescriptionText;
        let text = reader.text(TextKind::DescriptionText)?;
        reader.field = Field::Inlets;
        let inlets = reader.ports(|index, part| Field::Inlet { index, part })?;
        reader.field = Field::Outlets;
        let outlets = reader.ports(|index, part| Field::Outlet { index, part })?;
        reader.field = Field::Params;
        let described = reader.count()?;
        let params = reader.list(
            described,
            |item, index| {
                item.field = Field::Param {
                    index,
                    part: Part::Name,
                };
                item.text(TextKind::Name)
            },
            Reader::param,
        )?;
        reader.field = Field::Capabilities;
        let asked = reader.count()?;
        let capabilities = reader.list(
            asked,
            |item, index| {
                item.field = Field::Capability { index };
                item.code(|code| match code {
                    0 => Some(Capability::AudioOutput),
                    _ => None,
                })
            },
            |_, _, capability| Ok(capability),
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
            let left = u32::try_from(reader.bytes.len()).unwrap_or(u32::MAX);
            Err(reader.refusal(Fault::Trailing { bytes: left }))
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

    /// The next text, of kind `kind`, refused as [`text_fault`] says: borrowed, so that the
    /// length announced reserves nothing.
    fn text(&mut self, kind: TextKind) -> Result<&'bytes str, DescriptionError> {
        let length = u64::from(u32::from_le_bytes(self.take()?));
        let (taken, rest) = self
            .bytes
            .split_at_checked(address(length))
            .ok_or_else(|| self.refusal(Fault::Truncated))?;
        self.bytes = rest;
        let text = core::str::from_utf8(taken).map_err(|_| self.refusal(Fault::NotUtf8))?;
        text_fault(text, kind).map_or(Ok(text), |fault| Err(self.refusal(fault)))
    }

    /// The next 32 bits: the count of the list that follows.
    fn count(&mut self) -> Result<u32, DescriptionError> {
        Ok(u32::from_le_bytes(self.take()?))
    }

    /// The next `count` items of a list, the one at `index` read by `key`, what no two of them
    /// share and its first part, then by `rest`, what follows its key: a key repeated is
    /// refused as soon as it is read, in the field `key` read. It grows with the items read,
    /// never by the count announced, and every item takes a byte at least, so the bytes bound
    /// it too.
    fn list<T, K: Ord + Copy>(
        &mut self,
        count: u32,
        key: impl Fn(&mut Self, u32) -> Result<K, DescriptionError>,
        rest: impl Fn(&mut Self, u32, K) -> Result<T, DescriptionError>,
    ) -> Result<Vec<T>, DescriptionError> {
        let mut keys = BTreeMap::new();
        let mut items = vec![];
        for index in 0..count {
            let read = key(self, index)?;
            if let Some(first) = keys.insert(read, index) {
                return Err(self.refusal(Fault::Repeated { first }));
            }
            items.push(rest(self, index, read)?);
        }
        Ok(items)
    }

    /// The next list of ports, at most [`PORTS_MAX`], the part of the port at an index being
    /// the field `at` gives.
    fn ports(
        &mut self,
        at: fn(u32, Part) -> Field,
    ) -> Result<Vec<DescribedPort<'bytes>>, DescriptionError> {
        let count = self.count()?;
        if count > PORTS_MAX {
            return Err(self.refusal(Fault::TooMany { count }));
        }
        self.list(
            count,
            |reader, index| {
                reader.field = at(index, Part::Name);
                reader.text(TextKind::Name)
            },
            |reader, index, name| {
                reader.field = at(index, Part::DescriptionText);
                let description = reader.text(TextKind::DescriptionText)?;
                reader.field = at(index, Part::Kind);
                let kind = reader.port_kind()?;
                Ok(DescribedPort {
                    name,
                    description,
                    kind,
                })
            },
        )
    }

    /// The parameter at `index`, called `name`, from its description text on.
    fn param(
        &mut self,
        index: u32,
        name: &'bytes str,
    ) -> Result<DescribedParam<'bytes>, DescriptionError> {
        let at = |part| Field::Param { index, part };
        self.field = at(Part::DescriptionText);
        let description = self.text(TextKind::DescriptionText)?;
        self.field = at(Part::Values);
        let bits = [
            u64::from_le_bytes(self.take()?),
            u64::from_le_bytes(self.take()?),
            u64::from_le_bytes(self.take()?),
        ];
        admissible(bits).map_err(|fault| self.refusal(fault))?;
        Ok(DescribedParam {
            name,
            description,
            bits,
        })
    }
}

/// Checks the default, the minimum and the maximum of a parameter, given by their bits, by
/// the one rule of a parameter's range ([`crate::object::range_rule`]), which a built-in box
/// type's [`crate::ParamSpec::new`] follows too.
const fn admissible([value, minimum, maximum]: [u64; 3]) -> Result<(), Fault> {
    match range_rule(
        f64::from_bits(value),
        f64::from_bits(minimum),
        f64::from_bits(maximum),
    ) {
        Ok(()) => Ok(()),
        Err(rule) => Err(Fault::Range {
            rule,
            value,
            minimum,
            maximum,
        }),
    }
}

/// A field of a description, in the order of the format, or a part of an item of one of its
/// lists.
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
    /// The list of inlets, as a whole: its count.
    Inlets,
    /// A part of the inlet at `index`, counted from 0.
    Inlet {
        /// Its position in the list.
        index: u32,
        /// The part refused.
        part: Part,
    },
    /// The list of outlets, as a whole: its count.
    Outlets,
    /// A part of the outlet at `index`, counted from 0.
    Outlet {
        /// Its position in the list.
        index: u32,
        /// The part refused.
        part: Part,
    },
    /// The list of parameters, as a whole: its count.
    Params,
    /// A part of the parameter at `index`, counted from 0.
    Param {
        /// Its position in the list.
        index: u32,
        /// The part refused.
        part: Part,
    },
    /// The list of capabilities, as a whole: its count.
    Capabilities,
    /// The capability at `index`, counted from 0.
    Capability {
        /// Its position in the list.
        index: u32,
    },
    /// After the last field.
    End,
}

impl fmt::Display for Field {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {
            Self::Whole => f.write_str("description"),
            Self::AlgorithmVersion => f.write_str("algorithm version"),
            Self::Mode => f.write_str("execution mode"),
            Self::Response => f.write_str("instantaneous response"),
            Self::Name => f.write_str("name"),
            Self::DescriptionText => f.write_str("description text"),
            Self::Inlets => f.write_str("inlets"),
            Self::Inlet { index, part } => write!(f, "inlet {index}, its {part}"),
            Self::Outlets => f.write_str("outlets"),
            Self::Outlet { index, part } => write!(f, "outlet {index}, its {part}"),
            Self::Params => f.write_str("parameters"),
            Self::Param { index, part } => write!(f, "parameter {index}, its {part}"),
            Self::Capabilities => f.write_str("capabilities"),
            Self::Capability { index } => write!(f, "capability {index}"),
            Self::End => f.write_str("end"),
        }
    }
}

/// A part of a port or a parameter, in the order of the format.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum Part {
    /// Its name.
    Name,
    /// Its description text.
    DescriptionText,
    /// A port's kind.
    Kind,
    /// A parameter's default, minimum and maximum.
    Values,
}

impl fmt::Display for Part {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Name => "name",
            Self::DescriptionText => "description text",
            Self::Kind => "kind",
            Self::Values => "values",
        })
    }
}

/// Why a field is refused. A parameter's values are given by their binary64 bits, as the
/// description gives them, so that a refusal compares equal to itself, NaN included.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum Fault {
    /// The bytes end inside the field.
    Truncated,
    /// Bytes follow the last field.
    Trailing {
        /// How many.
        bytes: u32,
    },
    /// A number that is none of the field's codes in this version of the box contract.
    Code(u8),
    /// A port kind's code reserved for a later minor version ([`RESERVED_PORT_KINDS`]).
    Reserved(u8),
    /// An algorithm version of 0.
    Zero,
    /// More than [`DESCRIPTION_BYTES_MAX`] bytes.
    TooLong {
        /// How many, at most `u64::MAX` (on no target are there more).
        bytes: u64,
    },
    /// A list of more than [`PORTS_MAX`] ports.
    TooMany {
        /// The count it announces.
        count: u32,
    },
    /// A text that is not UTF-8.
    NotUtf8,
    /// A text that shows nothing ([`text_fault`]).
    Blank,
    /// A text holding a character it may not ([`text_fault`]).
    Character(char),
    /// Whitespace in the name of the box type.
    Whitespace,
    /// U+0020 at the start or the end of the name of a port or a parameter, or after another.
    Spaces,
    /// The name of an item, or a capability, that an earlier item of its list has.
    Repeated {
        /// The position of that earlier item.
        first: u32,
    },
    /// A parameter's range that breaks a rule of every box type's ranges (the one rule of a
    /// built-in box type's [`crate::ParamSpec::new`] too), with the bits of the values it
    /// was read with.
    Range {
        /// The rule the range breaks.
        rule: RangeRule,
        /// The bits of the default, as given.
        value: u64,
        /// The bits of the minimum.
        minimum: u64,
        /// The bits of the maximum.
        maximum: u64,
    },
}

impl fmt::Display for Fault {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let number = Shown;
        match *self {
            Self::Truncated => f.write_str("the bytes end inside it"),
            Self::Trailing { bytes } => write!(f, "{bytes} byte(s) follow the last field"),
            Self::Code(code) => write!(
                f,
                "{code} is none of its codes in this version of the box contract"
            ),
            Self::Reserved(code) => write!(
                f,
                "{code} is reserved for a later version of the box contract"
            ),
            Self::Zero => f.write_str("is 0, versions count from 1"),
            Self::TooLong { bytes } => {
                write!(f, "holds {bytes} bytes, more than {DESCRIPTION_BYTES_MAX}")
            }
            Self::TooMany { count } => write!(f, "holds {count} ports, more than {PORTS_MAX}"),
            Self::NotUtf8 => f.write_str("is not UTF-8"),
            Self::Blank => f.write_str("is blank"),
            Self::Character(character) => {
                write!(f, "holds U+{:04X}", u32::from(character))
            }
            Self::Whitespace => f.write_str("holds whitespace"),
            Self::Spaces => f.write_str("starts or ends with a space, or holds two in a row"),
            Self::Repeated { first } => write!(f, "repeats item {first} of its list"),
            Self::Range {
                rule,
                value,
                minimum,
                maximum,
            } => {
                let (minimum, maximum) = (number(minimum), number(maximum));
                let bound = |bound| match bound {
                    Bound::Minimum => ("minimum", minimum),
                    Bound::Maximum => ("maximum", maximum),
                };
                match rule {
                    RangeRule::NotFinite(which) => {
                        let (word, bound) = bound(which);
                        write!(f, "the {word} {bound} is not finite")
                    }
                    RangeRule::Unreachable(which) => {
                        let (word, bound) = bound(which);
                        write!(
                            f,
                            "the {word} {bound} is below 2^-126 in magnitude but not zero: no \
                             value reaches it"
                        )
                    }
                    RangeRule::Reversed => {
                        write!(f, "the minimum {minimum} is above the maximum {maximum}")
                    }
                    RangeRule::Width => write!(
                        f,
                        "the width, {maximum} less {minimum}, is not finite in binary64"
                    ),
                    RangeRule::Default => write!(
                        f,
                        "the default {}, flushed, is not within [{minimum}, {maximum}]",
                        number(value)
                    ),
                }
            }
        }
    }
}

/// Why a description is refused ([`Description::read`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
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
