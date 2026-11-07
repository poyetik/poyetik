//! Fixed-size processing blocks.

/// Number of frames processed per block.
///
/// Fixed and independent of the host buffer size (128 frames in a browser, variable in
/// a DAW), so that results never depend on how the host splits time.
pub const BLOCK_SIZE: usize = 64;

/// One block of mono samples.
pub type Block = [f32; BLOCK_SIZE];

/// A block of silence.
pub const SILENCE: Block = [0.0_f32; BLOCK_SIZE];
