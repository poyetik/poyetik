//! A body moved from `new` into a function of its own, as a refactor leaves it.

/// A side of at most ten units.
pub struct Side(u32);

impl Side {
    /// The side `units` long, if it is at most ten.
    #[must_use]
    pub const fn new(units: u32) -> Option<Self> {
        Self::checked(units)
    }

    /// The body of `new`, which cargo-mutants never mutates.
    const fn checked(units: u32) -> Option<Self> {
        if units > 10 { None } else { Some(Self(units)) }
    }

    /// How long it is.
    #[must_use]
    pub const fn units(&self) -> u32 {
        self.0
    }
}
