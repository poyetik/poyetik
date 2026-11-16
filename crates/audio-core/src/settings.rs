//! What a host asks of the engine and what the engine applies (`spec/engine-design.md`,
//! decision 1): a host states its settings, the engine answers with the ones it applies and
//! one reason for each setting it cannot honor as asked. A setting a host cannot honor is
//! reported, never faked.

#![cfg_attr(not(test), forbid(clippy::arithmetic_side_effects))]

use alloc::vec::Vec;
use core::fmt;

use crate::{CoreError, SampleRate};

/// The most output channels, and the most input channels, a host may ask for (decision 1):
/// the widest an installation asks for, the one maximum multichannel cables reuse
/// (decision 8).
pub const MAX_CHANNELS: u16 = 512;

/// A direction of the host's channels.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[expect(
    clippy::exhaustive_enums,
    reason = "channels go out of the engine or into it: there is no third direction"
)]
pub enum Direction {
    /// The channels the engine renders into.
    Outputs,
    /// The channels the host gives the engine.
    Inputs,
}

impl fmt::Display for Direction {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Outputs => "output",
            Self::Inputs => "input",
        })
    }
}

/// What a host asks for, as it states it: nothing checked yet ([`HostSettings::apply`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HostSettings {
    /// The sample rate, in hertz.
    pub rate: u32,
    /// The engine block, in frames.
    pub block: usize,
    /// The output channels.
    pub outputs: u16,
    /// The input channels.
    pub inputs: u16,
}

/// A setting the engine applies otherwise than its host asked, and why.
///
/// A reason may be added (`#[non_exhaustive]`; `InputsUnread` goes when an input box
/// exists): a match outside this crate needs a wildcard arm.
///
/// ```compile_fail,E0004
/// fn block(reason: audio_core::Unhonored) -> bool {
///     match reason {
///         audio_core::Unhonored::Block { .. } => true,
///         audio_core::Unhonored::Dropped { .. }
///         | audio_core::Unhonored::Silent { .. }
///         | audio_core::Unhonored::InputsUnread { .. } => false,
///     }
/// }
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum Unhonored {
    /// The engine block asked is not the one this engine is built for, which it applies.
    Block {
        /// The frames asked.
        asked: usize,
        /// The frames applied: the engine's own block.
        applied: usize,
    },
    /// The patch has more output channels than the host: the ones beyond the host's are
    /// dropped.
    Dropped {
        /// The patch's output channels.
        patch: usize,
        /// The host's.
        host: u16,
    },
    /// The host has more output channels than the patch: the ones beyond the patch's are
    /// silent.
    Silent {
        /// The patch's output channels.
        patch: usize,
        /// The host's.
        host: u16,
    },
    /// The host's input channels are accepted and unread: no box reads an input yet
    /// (#12 step 36).
    InputsUnread {
        /// The host's input channels.
        inputs: u16,
    },
}

impl fmt::Display for Unhonored {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Block { asked, applied } => write!(
                f,
                "engine block of {asked} frames asked, {applied} applied: the engine is built \
                 for {applied}"
            ),
            Self::Dropped { patch, host } => write!(
                f,
                "the patch has {patch} output channels and the host {host}: the ones beyond \
                 {host} are dropped"
            ),
            Self::Silent { patch, host } => write!(
                f,
                "the host has {host} output channels and the patch {patch}: the ones beyond \
                 {patch} are silent"
            ),
            Self::InputsUnread { inputs } => write!(
                f,
                "{inputs} input channels accepted and unread: no box reads an input yet"
            ),
        }
    }
}

/// The settings the engine applies for a host, with a reason for each setting applied
/// otherwise than asked ([`Unhonored`]), in the order of [`HostSettings`]'s fields.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppliedSettings {
    /// The sample rate.
    rate: SampleRate,
    /// The engine block, in frames.
    block: usize,
    /// The output channels.
    outputs: u16,
    /// The input channels.
    inputs: u16,
    /// Why each setting applied otherwise than asked is so.
    unhonored: Vec<Unhonored>,
}

impl AppliedSettings {
    /// The sample rate applied.
    #[must_use]
    pub const fn rate(&self) -> SampleRate {
        self.rate
    }

    /// The engine block applied, in frames.
    #[must_use]
    pub const fn block(&self) -> usize {
        self.block
    }

    /// The output channels applied: the host's.
    #[must_use]
    pub const fn outputs(&self) -> u16 {
        self.outputs
    }

    /// The input channels applied: the host's.
    #[must_use]
    pub const fn inputs(&self) -> u16 {
        self.inputs
    }

    /// Why each setting applied otherwise than asked is so; empty when every setting is
    /// honored as asked.
    #[must_use]
    pub fn unhonored(&self) -> &[Unhonored] {
        &self.unhonored
    }
}

impl HostSettings {
    /// The settings an engine built for blocks of `N` frames applies for this host, for a
    /// patch of `patch_channels` output channels.
    ///
    /// # Errors
    ///
    /// Returns [`CoreError::InvalidSampleRate`] for a rate the engine does not support, and
    /// [`CoreError::TooManyChannels`] for more than [`MAX_CHANNELS`] channels either way:
    /// settings no engine can apply.
    ///
    /// Compiles for an engine block `N` of 1 to [`crate::MAX_ENGINE_BLOCK`] frames only
    /// (decision 2), neither 0 nor one more:
    ///
    /// ```compile_fail,E0080
    /// use audio_core::HostSettings;
    /// let host = HostSettings { rate: 48_000, block: 64, outputs: 2, inputs: 0 };
    /// let _ = host.apply::<0>(2);
    /// ```
    ///
    /// ```compile_fail,E0080
    /// use audio_core::HostSettings;
    /// let host = HostSettings { rate: 48_000, block: 64, outputs: 2, inputs: 0 };
    /// let _ = host.apply::<1_048_577>(2);
    /// ```
    pub fn apply<const N: usize>(
        self,
        patch_channels: usize,
    ) -> Result<AppliedSettings, CoreError> {
        crate::block::engine_block::<N>();
        self.applied(N, patch_channels)
    }

    /// [`HostSettings::apply`] by an engine built for blocks of `engine_block` frames, once
    /// for every engine block.
    fn applied(
        self,
        engine_block: usize,
        patch_channels: usize,
    ) -> Result<AppliedSettings, CoreError> {
        let rate = SampleRate::new(self.rate)?;
        for (direction, count) in [
            (Direction::Outputs, self.outputs),
            (Direction::Inputs, self.inputs),
        ] {
            if count > MAX_CHANNELS {
                return Err(CoreError::TooManyChannels { direction, count });
            }
        }
        let mut unhonored = Vec::new();
        if self.block != engine_block {
            unhonored.push(Unhonored::Block {
                asked: self.block,
                applied: engine_block,
            });
        }
        let host = usize::from(self.outputs);
        if patch_channels > host {
            unhonored.push(Unhonored::Dropped {
                patch: patch_channels,
                host: self.outputs,
            });
        } else if patch_channels < host {
            unhonored.push(Unhonored::Silent {
                patch: patch_channels,
                host: self.outputs,
            });
        }
        if self.inputs > 0 {
            unhonored.push(Unhonored::InputsUnread {
                inputs: self.inputs,
            });
        }
        Ok(AppliedSettings {
            rate,
            block: engine_block,
            outputs: self.outputs,
            inputs: self.inputs,
            unhonored,
        })
    }
}

#[cfg(test)]
#[path = "settings_tests.rs"]
mod tests;
