//! What a host asks of the engine and what the engine applies (`spec/engine-design.md`,
//! decision 1): a host states its settings, and the answer gives the ones the engine applies
//! and one reason for each setting it cannot honor as asked ([`HostSettings::apply`]). A
//! setting a host cannot honor is reported, never faked. No rendering engine answers a
//! host's settings yet, `audio_host::Engine` or the WebAssembly engine
//! (roadmap #24 step 131): the browser's engine gives its own answer, in TypeScript.

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

/// A count of output channels, of a type of its own, apart from [`InputChannels`].
///
/// The two counts cannot be swapped, from what a host asks ([`HostSettings::new`] and its
/// fields) to what the engine applies ([`AppliedSettings::outputs`]). It is named apart from
/// the direction ([`Direction::Outputs`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[expect(
    clippy::exhaustive_structs,
    reason = "a count of output channels, nothing else"
)]
pub struct OutputChannels(pub u16);

/// A count of input channels, of a type of its own, apart from [`OutputChannels`].
///
/// The two counts cannot be swapped, from what a host asks ([`HostSettings::new`] and its
/// fields) to what the engine applies ([`AppliedSettings::inputs`]). It is named apart from
/// the direction ([`Direction::Inputs`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[expect(
    clippy::exhaustive_structs,
    reason = "a count of input channels, nothing else"
)]
pub struct InputChannels(pub u16);

/// What a host asks for, as it states it: nothing checked yet ([`HostSettings::apply`]).
///
/// It may gain a setting (decision 1: the execution mode and the host buffer are host
/// settings), so that it is built by [`HostSettings::new`], never by a literal outside this
/// crate, which a new field would break:
///
/// ```compile_fail,E0639
/// let host = audio_core::HostSettings { rate: 48_000, block: 64, outputs: 2, inputs: 0 };
/// ```
///
/// ```
/// use audio_core::{HostSettings, InputChannels, OutputChannels};
/// let host = HostSettings::new(48_000, 64, OutputChannels(2), InputChannels(0));
/// assert_eq!(
///     (host.rate, host.block, host.outputs, host.inputs),
///     (48_000, 64, OutputChannels(2), InputChannels(0))
/// );
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub struct HostSettings {
    /// The sample rate, in hertz.
    pub rate: u32,
    /// The engine block, in frames: a fixed width holding every engine block, up to
    /// [`crate::MAX_ENGINE_BLOCK`] (2^20, beyond `u16`).
    pub block: u32,
    /// The output channels.
    pub outputs: OutputChannels,
    /// The input channels.
    pub inputs: InputChannels,
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
        asked: u32,
        /// The frames applied: the engine's own block.
        applied: u32,
    },
    /// The patch has more output channels than the host: the ones beyond the host's are
    /// dropped.
    Dropped {
        /// The patch's output channels.
        patch: u16,
        /// The host's.
        host: u16,
    },
    /// The host has more output channels than the patch: the ones beyond the patch's are
    /// silent.
    Silent {
        /// The patch's output channels.
        patch: u16,
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
    block: u32,
    /// The output channels.
    outputs: OutputChannels,
    /// The input channels.
    inputs: InputChannels,
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
    pub const fn block(&self) -> u32 {
        self.block
    }

    /// The output channels applied: the host's.
    #[must_use]
    pub const fn outputs(&self) -> OutputChannels {
        self.outputs
    }

    /// The input channels applied: the host's.
    #[must_use]
    pub const fn inputs(&self) -> InputChannels {
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
    /// The settings of a host asking for `rate` hertz, blocks of `block` frames, `outputs`
    /// output channels and `inputs` input channels, checked by [`HostSettings::apply`].
    ///
    /// Its two counts of channels are of two types, so that they cannot be swapped:
    ///
    /// ```compile_fail,E0308
    /// use audio_core::{HostSettings, InputChannels, OutputChannels};
    /// let _ = HostSettings::new(48_000, 64, InputChannels(2), OutputChannels(0));
    /// ```
    ///
    /// nor once it is built:
    ///
    /// ```compile_fail,E0308
    /// use audio_core::{HostSettings, InputChannels, OutputChannels};
    /// let mut host = HostSettings::new(48_000, 64, OutputChannels(2), InputChannels(0));
    /// host.inputs = host.outputs;
    /// ```
    #[must_use]
    pub const fn new(
        rate: u32,
        block: u32,
        outputs: OutputChannels,
        inputs: InputChannels,
    ) -> Self {
        Self::made(rate, block, outputs, inputs)
    }

    /// [`HostSettings::new`]'s body, a function of its own so that mutation testing reaches it:
    /// cargo-mutants never mutates a function named `new`.
    const fn made(rate: u32, block: u32, outputs: OutputChannels, inputs: InputChannels) -> Self {
        Self {
            rate,
            block,
            outputs,
            inputs,
        }
    }

    /// The settings an engine built for blocks of `N` frames applies for this host, for a
    /// patch of `patch_channels` output channels: every count of fixed width, an engine's
    /// own (`N`, a `usize` of at most [`crate::MAX_ENGINE_BLOCK`] frames) converted here, at
    /// its edge.
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
    /// use audio_core::{HostSettings, InputChannels, OutputChannels};
    /// let host = HostSettings::new(48_000, 64, OutputChannels(2), InputChannels(0));
    /// let _ = host.apply::<0>(OutputChannels(2));
    /// ```
    ///
    /// ```compile_fail,E0080
    /// use audio_core::{HostSettings, InputChannels, OutputChannels};
    /// let host = HostSettings::new(48_000, 64, OutputChannels(2), InputChannels(0));
    /// let _ = host.apply::<1_048_577>(OutputChannels(2));
    /// ```
    pub fn apply<const N: usize>(
        self,
        patch_channels: OutputChannels,
    ) -> Result<AppliedSettings, CoreError> {
        crate::block::engine_block::<N>();
        // At most `MAX_ENGINE_BLOCK`, 2^20, by the assertion above: it fits.
        self.applied(u32::try_from(N).unwrap_or(u32::MAX), patch_channels)
    }

    /// [`HostSettings::apply`] by an engine built for blocks of `engine_block` frames, once
    /// for every engine block.
    fn applied(
        self,
        engine_block: u32,
        OutputChannels(patch_channels): OutputChannels,
    ) -> Result<AppliedSettings, CoreError> {
        let rate = SampleRate::new(self.rate)?;
        for (direction, count) in [
            (Direction::Outputs, self.outputs.0),
            (Direction::Inputs, self.inputs.0),
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
        let OutputChannels(host) = self.outputs;
        if patch_channels > host {
            unhonored.push(Unhonored::Dropped {
                patch: patch_channels,
                host,
            });
        } else if patch_channels < host {
            unhonored.push(Unhonored::Silent {
                patch: patch_channels,
                host,
            });
        }
        let InputChannels(inputs) = self.inputs;
        if inputs > 0 {
            unhonored.push(Unhonored::InputsUnread { inputs });
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
