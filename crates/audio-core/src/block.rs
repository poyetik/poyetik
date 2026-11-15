//! Processing blocks, of a size fixed when the engine is compiled (`spec/engine-design.md`,
//! decision 2): the processing is generic over it, so that every size runs the code a
//! fixed block compiles to.

#![cfg_attr(not(test), forbid(clippy::arithmetic_side_effects))]

/// Number of frames of a block by default: the engine block of a build that chooses none.
///
/// Independent of the host buffer size (128 frames in a browser, variable in a DAW), so
/// that results never depend on how the host splits time; and no render depends on the
/// engine block either (the engine-block law).
pub const BLOCK_SIZE: usize = 64;

/// The engine blocks a build of the patcher offers (decision 2), in frames, ascending.
///
/// The web engine ships one module per size, the desktop patcher holds them all. An
/// exported native target compiles the size it is exported with, any from 1 to
/// [`MAX_ENGINE_BLOCK`].
pub const ENGINE_BLOCKS: [usize; 6] = [32, 64, 128, 256, 512, 1024];

/// The largest engine block a build may compile, in frames: 2^20 (decision 2).
pub const MAX_ENGINE_BLOCK: usize = 1 << 20;

/// One block of `N` mono samples (the engine block, [`BLOCK_SIZE`] by default), computed in
/// `S` ([`crate::Binary64`] or [`crate::Binary32`], [`crate::Sample`]); a block of silence is
/// `[S::ZERO; N]`.
pub type Block<S, const N: usize = BLOCK_SIZE> = [S; N];
