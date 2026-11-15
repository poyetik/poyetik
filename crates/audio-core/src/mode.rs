//! Execution modes (`spec/engine-design.md`, decision 20): one engine, three modes with the
//! same semantics and the same bits, and box types that declare whether they may run live.

#![cfg_attr(not(test), forbid(clippy::arithmetic_side_effects))]

use core::fmt;

/// How a plan runs: never part of the result, which is the same bits in every mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ExecutionMode {
    /// A deadline every host buffer; what cannot keep up is reported, never hidden. A live
    /// plan holds live-capable boxes only ([`BoxMode::LiveCapable`]).
    Live,
    /// No deadline: the renderer waits for what it needs, so it is never late.
    Offline,
    /// The export: the patch is frozen and may be compiled and precomputed, and gives the
    /// bits of an offline render of the same patch.
    Build,
}

impl fmt::Display for ExecutionMode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Live => "live",
            Self::Offline => "offline",
            Self::Build => "build",
        })
    }
}

/// What a box type declares it may run in (decision 20), as part of its type: every box type
/// states it, a box type without one not compiling ([`crate::Create::MODE`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum BoxMode {
    /// Bounded work per frame and per message, no allocation or release of memory, no lock,
    /// no I/O while processing: it runs in every mode, the real-time rules proven on it by the
    /// box laws under the counting allocator.
    LiveCapable,
    /// It may allocate while processing, still deterministic, with no clock, file or network
    /// access: it runs offline and in the build only, and a live plan refuses it.
    OfflineOnly,
}

impl BoxMode {
    /// Whether a box type of this mode may run in a plan of `mode`.
    #[must_use]
    pub const fn runs_in(self, mode: ExecutionMode) -> bool {
        matches!(self, Self::LiveCapable) || !matches!(mode, ExecutionMode::Live)
    }
}

#[cfg(test)]
#[path = "mode_tests.rs"]
mod tests;
