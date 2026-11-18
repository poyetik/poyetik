//! Validation of a WGSL source with naga: parsed, then validated as a module, the first
//! problem reported as one line with its position.
//!
//! A thin layer: naga parses and validates; this module only chooses its settings (every
//! rule, naga's default capabilities), picks naga's narrowest position, and shapes its
//! message into one escaped, bounded line.

use core::error::Error;
use core::fmt;

use naga::valid::{Capabilities, ValidationFlags, Validator};

use crate::safe::safe_message;

#[cfg(test)]
#[path = "validation_tests.rs"]
mod tests;

/// Where a problem lies in a source.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Location {
    /// The line, from 1.
    line: u32,
    /// The column, from 1, counted in bytes of UTF-8 (naga's unit).
    column: u32,
}

impl Location {
    /// The line, from 1.
    #[must_use]
    pub const fn line(self) -> u32 {
        self.line
    }

    /// The column, from 1, counted in bytes of UTF-8 from the start of the line (naga's
    /// unit), so that a character beyond ASCII before it counts for each of its bytes.
    #[must_use]
    pub const fn column(self) -> u32 {
        self.column
    }
}

impl From<naga::SourceLocation> for Location {
    fn from(location: naga::SourceLocation) -> Self {
        Self {
            line: location.line_number,
            column: location.line_position,
        }
    }
}

/// What refused a source.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Stage {
    /// The source is longer than naga can address: its positions are 32-bit offsets.
    Size,
    /// The source is not WGSL: naga's parser refused it.
    Parse,
    /// The source parses, but the module breaks a rule of WGSL: naga's validator refused it.
    Validation,
}

impl Stage {
    /// The words that say it in a report.
    const fn words(self) -> &'static str {
        match self {
            Self::Size => "too large",
            Self::Parse => "parse error",
            Self::Validation => "invalid",
        }
    }
}

/// Why a source is not a valid WGSL module: the first problem naga found.
///
/// Shown on one line, `<line>:<column>: <stage>: <message>` (the position left out when
/// naga gives none), the message being naga's with the causes it chains, made safe to show
/// ([`safe_message`](crate::safe_message): escaped and bounded), so that a source can
/// neither drive the terminal nor forge a second line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Invalid {
    /// What refused the source.
    stage: Stage,
    /// Where, when naga says.
    location: Option<Location>,
    /// naga's message and its causes, escaped and bounded.
    message: String,
}

impl Invalid {
    /// What refused the source.
    #[must_use]
    pub const fn stage(&self) -> Stage {
        self.stage
    }

    /// Where the problem lies, when naga says.
    #[must_use]
    pub const fn location(&self) -> Option<Location> {
        self.location
    }

    /// naga's message and the causes it chains, joined by `: `, escaped and bounded.
    #[must_use]
    pub fn message(&self) -> &str {
        &self.message
    }

    /// The refusal of the file named `file`, on one line: `<file>:<line>:<column>:
    /// <stage>: <message>`, or `<file>: <stage>: <message>` without a position, the name
    /// made safe to show as the message is ([`safe_message`](crate::safe_message)).
    #[must_use]
    pub fn of_file(&self, file: &str) -> String {
        let gap = if self.location.is_some() { "" } else { " " };
        format!("{}:{gap}{self}", safe_message(file))
    }
}

impl fmt::Display for Invalid {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        if let Some(location) = self.location {
            write!(formatter, "{}:{}: ", location.line, location.column)?;
        }
        write!(formatter, "{}: {}", self.stage.words(), self.message)
    }
}

impl Error for Invalid {}

/// Checks that a source of `bytes` bytes can be addressed by naga, whose positions are
/// 32-bit offsets.
///
/// # Errors
///
/// Returns a [`Stage::Size`] refusal when it holds more bytes than a 32-bit offset counts.
fn addressable(bytes: usize) -> Result<(), Invalid> {
    match u32::try_from(bytes) {
        Ok(_) => Ok(()),
        Err(_) => Err(Invalid {
            stage: Stage::Size,
            location: None,
            message: format!("{bytes} bytes, more than naga's 32-bit positions address"),
        }),
    }
}

/// Checks that `source` is a valid WGSL module, as wgpu's naga parses and validates it.
///
/// Every rule of naga's validator is checked (`ValidationFlags::all`), under naga's default
/// capabilities (multisampled shading and cube array textures), so that a shader asking
/// for an optional feature of a device is refused. They are not all of WebGPU's core:
/// naga leaves out external textures (`texture_external`), which a shader of WebGPU may
/// use and this refuses.
///
/// # Errors
///
/// Returns the first problem: a source too large for naga's 32-bit positions
/// ([`Stage::Size`], refused before naga reads it), a parse error ([`Stage::Parse`]), or a
/// rule of the module broken ([`Stage::Validation`]).
pub fn validate(source: &str) -> Result<(), Invalid> {
    addressable(source.len())?;
    let module = naga::front::wgsl::parse_str(source).map_err(|error| Invalid {
        stage: Stage::Parse,
        location: error.location(source).map(Location::from),
        message: one_line(&error),
    })?;
    Validator::new(ValidationFlags::all(), Capabilities::default())
        .validate(&module)
        .map(drop)
        .map_err(|error| Invalid {
            stage: Stage::Validation,
            location: narrowest(error.spans().map(|&(span, _)| span))
                .map(|span| Location::from(span.location(source))),
            message: one_line(&error),
        })
}

/// The narrowest of `spans` that says where it lies, the last of the narrowest on a tie.
///
/// naga gives a validation error the spans of what holds the fault, most often from the
/// widest (the function) to the narrowest (the expression at fault), but not always (a
/// texture store gives its value's span before its statement's): the narrowest, measured,
/// is where to look.
fn narrowest(spans: impl Iterator<Item = naga::Span>) -> Option<naga::Span> {
    spans
        .filter_map(|span| span.to_range().map(|range| (range.len(), span)))
        .reduce(|kept, next| if next.0 <= kept.0 { next } else { kept })
        .map(|(_, span)| span)
}

/// `error` and the causes it chains, joined by `: `, made safe to show ([`safe_message`]).
fn one_line(error: &dyn Error) -> String {
    let mut text = error.to_string();
    let mut cause = error.source();
    while let Some(next) = cause {
        text.push_str(": ");
        text.push_str(&next.to_string());
        cause = next.source();
    }
    safe_message(&text)
}
