//! The status that starts every line `check`, `bless` and `export` print: one word, padded
//! to one column, so that the names after it line up.

/// What a printed line reports.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Status {
    /// A case renders as its reference.
    Ok,
    /// A case renders otherwise than its reference.
    Mismatch,
    /// Something that should be recorded is not.
    Missing,
    /// A case cannot be rendered.
    Failed,
    /// Something recorded that nothing uses.
    Stale,
    /// A box type at another algorithm version than its references were blessed with.
    Version,
    /// References hashed with another scheme.
    Hash,
    /// References rendered under other render rules.
    Rules,
    /// Fingerprints computed with another battery.
    Battery,
    /// A box type's outputs changed at the same algorithm version.
    Bits,
    /// A name `bless` was given that is no case.
    Unknown,
    /// A version below one already blessed.
    Lower,
    /// A reference changed under the same versions.
    Changed,
    /// A file written.
    Wrote,
}

impl Status {
    /// The word of the status.
    const fn word(self) -> &'static str {
        match self {
            Self::Ok => "ok",
            Self::Mismatch => "MISMATCH",
            Self::Missing => "MISSING",
            Self::Failed => "FAILED",
            Self::Stale => "STALE",
            Self::Version => "VERSION",
            Self::Hash => "HASH",
            Self::Rules => "RULES",
            Self::Battery => "BATTERY",
            Self::Bits => "BITS",
            Self::Unknown => "UNKNOWN",
            Self::Lower => "LOWER",
            Self::Changed => "CHANGED",
            Self::Wrote => "wrote",
        }
    }
}

impl core::fmt::Display for Status {
    /// The word, padded to the width of the longest (eight characters).
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "{:<8}", self.word())
    }
}
