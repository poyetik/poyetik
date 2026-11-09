//! Fixed-size processing blocks.

/// Number of frames processed per block.
///
/// Fixed and independent of the host buffer size (128 frames in a browser, variable in
/// a DAW), so that results never depend on how the host splits time.
pub const BLOCK_SIZE: usize = 64;

/// One block of mono samples, computed in `S` (`f64` or `f32`, [`crate::Sample`]); a block
/// of silence is [`crate::Sample::SILENCE`].
pub type Block<S> = [S; BLOCK_SIZE];
