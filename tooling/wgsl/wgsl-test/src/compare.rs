//! What a kernel wrote compared with a reference: bit for bit by default, or within a
//! tolerance stated with its derivation, and the first difference reported with its index,
//! the inputs that produced it, and both values.
//!
//! Bit for bit means the bits, not the numbers: `-0.0` differs from `0.0`, and a
//! not-a-number differs from one of another payload. A tolerance applies to `f32` values
//! only, an integer being right or wrong; under one, `-0.0` and `0.0` are the same number,
//! any not-a-number matches any other and nothing else, and an infinity matches itself
//! only.

use core::fmt;

use crate::safe::safe_message;

#[cfg(test)]
#[path = "compare_tests.rs"]
mod tests;

/// The values of a buffer, read back or expected.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Values<'values> {
    /// `u32` values.
    U32(&'values [u32]),
    /// `i32` values.
    I32(&'values [i32]),
    /// `f32` values.
    F32(&'values [f32]),
    /// Raw bytes, of a binding of mixed members.
    Bytes(&'values [u8]),
}

impl Values<'_> {
    /// What the values are, as WGSL names them (`bytes` for raw bytes).
    pub(crate) const fn kind(self) -> &'static str {
        match self {
            Self::U32(_) => "u32",
            Self::I32(_) => "i32",
            Self::F32(_) => "f32",
            Self::Bytes(_) => "bytes",
        }
    }

    /// How many values they hold.
    const fn len(self) -> usize {
        match self {
            Self::U32(values) => values.len(),
            Self::I32(values) => values.len(),
            Self::F32(values) => values.len(),
            Self::Bytes(values) => values.len(),
        }
    }
}

/// How far a value read back may be from its reference.
#[derive(Debug, Clone, PartialEq)]
pub enum Tolerance {
    /// Bit for bit: the default, and the only one integers and bytes take.
    Exact,
    /// At most `max` units in the last place apart: the floats between the two values,
    /// counted along the number line, `-0.0` and `0.0` being one.
    Ulps {
        /// The most units in the last place allowed.
        max: u32,
        /// Why that many: the derivation of the bound, shown with every difference.
        derivation: String,
    },
    /// At most `max` apart, the distance computed in `f64` (exact when both values are
    /// within a factor 2^29 of each other, and otherwise within one rounding of `f64`).
    Absolute {
        /// The largest distance allowed: finite and not negative.
        max: f32,
        /// Why that much: the derivation of the bound, shown with every difference.
        derivation: String,
    },
}

impl Tolerance {
    /// At most `max` units in the last place apart, for the reason `derivation`.
    #[must_use]
    pub fn ulps(max: u32, derivation: &str) -> Self {
        Self::Ulps {
            max,
            derivation: derivation.to_owned(),
        }
    }

    /// At most `max` apart, for the reason `derivation`.
    #[must_use]
    pub fn absolute(max: f32, derivation: &str) -> Self {
        Self::Absolute {
            max,
            derivation: derivation.to_owned(),
        }
    }

    /// Why this tolerance cannot apply to values of `kind`, if it cannot.
    fn refuses(&self, kind: &str) -> Option<String> {
        let derivation = match *self {
            Self::Exact => return None,
            Self::Absolute { max, .. } if !(max.is_finite() && max >= 0.0_f32) => {
                return Some(format!(
                    "an absolute tolerance of {max:?}: give a finite bound, not negative"
                ));
            }
            Self::Ulps { ref derivation, .. } | Self::Absolute { ref derivation, .. } => derivation,
        };
        if kind != "f32" {
            Some(format!(
                "a tolerance on {kind}: only f32 values take one, the others are compared bit for bit"
            ))
        } else if derivation.trim().is_empty() {
            Some("a tolerance with no derivation: say why the bound holds".to_owned())
        } else {
            None
        }
    }

    /// Whether `actual` is close enough to `expected`.
    fn admits(&self, actual: f32, expected: f32) -> bool {
        if actual.to_bits() == expected.to_bits() {
            return true;
        }
        match *self {
            Self::Exact => false,
            Self::Ulps { .. } | Self::Absolute { .. } if actual.is_nan() || expected.is_nan() => {
                actual.is_nan() && expected.is_nan()
            }
            Self::Ulps { .. } | Self::Absolute { .. }
                if actual.is_infinite() || expected.is_infinite() =>
            {
                false
            }
            Self::Ulps { max, .. } => ulps_apart(actual, expected) <= u64::from(max),
            Self::Absolute { max, .. } => distance(actual, expected) <= f64::from(max),
        }
    }

    /// How far apart `actual` and `expected` are, in this tolerance's unit, and the most
    /// it allows; none when exact.
    fn apart(&self, actual: f32, expected: f32) -> Option<String> {
        match *self {
            Self::Exact => None,
            Self::Ulps {
                max,
                ref derivation,
            } => Some(format!(
                "{} ulps apart, at most {max} allowed: {}",
                ulps_apart(actual, expected),
                safe_message(derivation)
            )),
            Self::Absolute {
                max,
                ref derivation,
            } => Some(format!(
                "{:?} apart, at most {max:?} allowed: {}",
                distance(actual, expected),
                safe_message(derivation)
            )),
        }
    }
}

impl Tolerance {
    /// Whether the value `actual` is close enough to `expected`, a value of the same kind:
    /// integers and bytes equal, floats within this tolerance.
    fn admits_value(&self, actual: Value, expected: Value) -> bool {
        match (actual, expected) {
            (Value::F32(read), Value::F32(wanted)) => self.admits(read, wanted),
            _ => actual == expected,
        }
    }

    /// How far apart the values `actual` and `expected` are, for floats under a tolerance.
    fn apart_value(&self, actual: Value, expected: Value) -> Option<String> {
        match (actual, expected) {
            (Value::F32(read), Value::F32(wanted)) => self.apart(read, wanted),
            _ => None,
        }
    }
}

/// The place of `value` along the number line of finite floats, `-0.0` and `0.0` at 0.
fn place(value: f32) -> i64 {
    let bits = value.to_bits();
    let magnitude = i64::from(bits & 0x7fff_ffff_u32);
    if bits & 0x8000_0000_u32 == 0 {
        magnitude
    } else {
        magnitude.wrapping_neg()
    }
}

/// The units in the last place between two finite floats.
fn ulps_apart(actual: f32, expected: f32) -> u64 {
    place(actual).abs_diff(place(expected))
}

/// The distance between two finite floats, in `f64`.
fn distance(actual: f32, expected: f32) -> f64 {
    (f64::from(actual) - f64::from(expected)).abs()
}

/// One value, read back or expected, shown with its bits when they say more than its
/// number.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Value {
    /// A `u32`.
    U32(u32),
    /// An `i32`.
    I32(i32),
    /// An `f32`.
    F32(f32),
    /// A byte.
    Byte(u8),
}

impl fmt::Display for Value {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {
            Self::U32(value) => write!(formatter, "{value} (0x{value:08x})"),
            Self::I32(value) => write!(formatter, "{value}"),
            Self::F32(value) => write!(formatter, "{value:?} (0x{:08x})", value.to_bits()),
            Self::Byte(value) => write!(formatter, "0x{value:02x}"),
        }
    }
}

/// Why what a kernel wrote does not match its reference.
#[derive(Debug, Clone, PartialEq)]
pub enum Mismatch {
    /// The kernel does not write that binding: none, or one it only reads.
    NotWritten {
        /// The bind group (`@group`).
        group: u32,
        /// The binding in the group (`@binding`).
        binding: u32,
    },
    /// The reference is not of the values compared with it.
    Kinds {
        /// What the values compared are: `u32`, `i32`, `f32`, `bytes`, or `mixed` for a
        /// binding of members of several kinds.
        actual: &'static str,
        /// What the reference is.
        expected: &'static str,
    },
    /// The tolerance cannot apply: on values other than `f32`, with no derivation, or with a
    /// bound that is not a finite number, not negative.
    BadTolerance(String),
    /// Not as many values as the reference.
    Lengths {
        /// The values compared.
        actual: usize,
        /// The values of the reference.
        expected: usize,
    },
    /// The first value outside the tolerance.
    Differs {
        /// Its index, in values (in bytes for raw bytes).
        index: usize,
        /// The reference's value.
        expected: Value,
        /// The value compared.
        actual: Value,
        /// How far apart they are and the most allowed, with the tolerance's derivation;
        /// none when compared bit for bit.
        apart: Option<String>,
        /// The inputs that produced it, as the test describes them, if it does.
        inputs: Option<String>,
    },
}

impl fmt::Display for Mismatch {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotWritten { group, binding } => write!(
                formatter,
                "not written: group {group} binding {binding} is no read_write buffer of the kernel"
            ),
            Self::Kinds { actual, expected } => write!(
                formatter,
                "different kinds: {actual} values compared with a reference of {expected}"
            ),
            Self::BadTolerance(why) => write!(formatter, "bad tolerance: {}", safe_message(why)),
            Self::Lengths { actual, expected } => write!(
                formatter,
                "different lengths: {actual} values compared with a reference of {expected}"
            ),
            Self::Differs {
                index,
                expected,
                actual,
                apart,
                inputs,
            } => {
                let apart = apart.as_ref().map_or_else(
                    || ", bit for bit expected".to_owned(),
                    |apart| format!(", {apart}"),
                );
                let inputs = inputs.as_ref().map_or_else(String::new, |inputs| {
                    format!("; inputs: {}", safe_message(inputs))
                });
                write!(
                    formatter,
                    "first difference at index {index}: expected {expected}, actual {actual}\
                     {apart}{inputs}"
                )
            }
        }
    }
}

impl core::error::Error for Mismatch {}

/// What describes the inputs that produced the value at an index, for the report of a
/// difference: called once, at the first difference only.
pub type Inputs<'describe> = &'describe dyn Fn(usize) -> String;

/// Each value of `actual` beside the value of `expected` at its index, as far as both go.
///
/// # Errors
///
/// Returns [`Mismatch::Kinds`] when they are not values of one kind.
fn pairs(actual: Values<'_>, expected: Values<'_>) -> Result<Vec<(Value, Value)>, Mismatch> {
    Ok(match (actual, expected) {
        (Values::U32(read), Values::U32(wanted)) => read
            .iter()
            .zip(wanted)
            .map(|(read, wanted)| (Value::U32(*read), Value::U32(*wanted)))
            .collect(),
        (Values::I32(read), Values::I32(wanted)) => read
            .iter()
            .zip(wanted)
            .map(|(read, wanted)| (Value::I32(*read), Value::I32(*wanted)))
            .collect(),
        (Values::F32(read), Values::F32(wanted)) => read
            .iter()
            .zip(wanted)
            .map(|(read, wanted)| (Value::F32(*read), Value::F32(*wanted)))
            .collect(),
        (Values::Bytes(read), Values::Bytes(wanted)) => read
            .iter()
            .zip(wanted)
            .map(|(read, wanted)| (Value::Byte(*read), Value::Byte(*wanted)))
            .collect(),
        _ => {
            return Err(Mismatch::Kinds {
                actual: actual.kind(),
                expected: expected.kind(),
            });
        }
    })
}

/// Compares `actual` with the reference `expected`, value by value, within `tolerance`.
///
/// # Errors
///
/// Returns why they do not match, the first that holds of: values of different kinds, a
/// tolerance that cannot apply to them, different lengths, and the first value outside the
/// tolerance, with the inputs `inputs` describes at its index.
pub fn compare(
    actual: Values<'_>,
    expected: Values<'_>,
    tolerance: &Tolerance,
    inputs: Option<Inputs<'_>>,
) -> Result<(), Mismatch> {
    let paired = pairs(actual, expected)?;
    if let Some(why) = tolerance.refuses(expected.kind()) {
        return Err(Mismatch::BadTolerance(why));
    }
    if actual.len() != expected.len() {
        return Err(Mismatch::Lengths {
            actual: actual.len(),
            expected: expected.len(),
        });
    }
    let first = paired
        .into_iter()
        .enumerate()
        .find(|(_, (read, wanted))| !tolerance.admits_value(*read, *wanted));
    first.map_or(Ok(()), |(index, (read, wanted))| {
        Err(Mismatch::Differs {
            index,
            expected: wanted,
            actual: read,
            apart: tolerance.apart_value(read, wanted),
            inputs: inputs.map(|describe| describe(index)),
        })
    })
}
