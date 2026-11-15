//! The status that starts every line `check` and `bless` print: one word, padded to one
//! column, so that the names after it line up; the words `audio-cli` prints.

/// What a printed line reports.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Status {
    /// A case renders as its reference.
    Ok,
    /// A case renders otherwise than its reference.
    Mismatch,
    /// A case has no reference.
    Missing,
    /// A reference has no case.
    Stale,
    /// A case cannot be rendered.
    Failed,
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
            Self::Stale => "STALE",
            Self::Failed => "FAILED",
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
