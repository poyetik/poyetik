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

/// The largest engine block a build may compile, in frames: 2^20 (decision 2).
///
/// Every function of the libraries that builds or runs a plan, a stage or a box instance
/// generic over the engine block `N` compiles for 1 to this size only, any other `N` not
/// compiling (each says so with its own examples), so that no block holds no frame and none
/// is larger; a box type of the layer above implementing `Object` itself is beyond that
/// list, but no plan can run it at another size. At both ends, every one compiles:
///
/// ```
/// use audio_core::{
///     Binary64, ExecutionMode, HostSettings, MAX_ENGINE_BLOCK, Object, OutputStage,
///     Patch, Plan, Renderer, SampleRate,
/// };
///
/// fn at<const N: usize>() {
///     let rate = SampleRate::new(48_000).expect("a valid rate");
///     let empty = Patch::new(vec![], vec![]).expect("an empty patch");
///     let plan = Plan::<Binary64, Box<dyn Object<Binary64, N>>, N>::build(
///         &empty,
///         &[],
///         rate,
///         ExecutionMode::Offline,
///     )
///     .expect("an empty plan");
///     let renderer =
///         Renderer::with_stage(plan, OutputStage::<_, f64, N>::new(0)).expect("a fresh raw stage");
///     assert_eq!(renderer.channels(), 0);
///     let mut stage = OutputStage::<Binary64, f32, N>::new(1);
///     assert_eq!(stage.apply(|_| ()).len(), 1);
///     assert_eq!(stage.apply_narrowing(|_| ()).len(), 1);
///     let block = u32::try_from(N).expect("an engine block fits 32 bits");
///     let host = HostSettings::new(48_000, block, 2, 0);
///     assert_eq!(host.apply::<N>(2).map(|applied| applied.block()), Ok(block));
/// }
///
/// at::<1>();
/// at::<MAX_ENGINE_BLOCK>();
/// ```
pub const MAX_ENGINE_BLOCK: usize = 1_048_576;

/// Refuses to compile for an engine block `N` outside 1 to [`MAX_ENGINE_BLOCK`] frames
/// (decision 2): the first statement of every function of the libraries that builds or runs
/// a plan, a stage or a box instance generic over it (or of a function it calls), so that a
/// block of no frame, which no processing can take a frame from, and one larger than the
/// largest, are compile errors where they are named, not panics where they run. Costs
/// nothing at run time: the compiler evaluates it once per size, as a constant.
pub(crate) const fn engine_block<const N: usize>() {
    const {
        assert!(
            matches!(N, 1..=MAX_ENGINE_BLOCK),
            "an engine block is 1 to audio_core::MAX_ENGINE_BLOCK frames"
        );
    }
}

/// One block of `N` mono samples (the engine block, [`BLOCK_SIZE`] by default), computed in
/// `S` ([`crate::Binary64`] or [`crate::Binary32`], [`crate::Sample`]); a block of silence is
/// `[S::ZERO; N]`.
pub type Block<S, const N: usize = BLOCK_SIZE> = [S; N];

/// The engine block a build names (`spec/engine-design.md`, decision 2), if any.
///
/// The size the variable `POYETIK_ENGINE_BLOCK` names when the build compiles, in decimal,
/// one of [`crate::ENGINE_BLOCKS`] (any other name does not compile); none when it names none.
///
/// A build naming one, such as each module of the web engine, holds the constructors of the
/// box types created through their virtual table for that size only, so that a module
/// carries no code of the other sizes; a build naming none (the tests, the desktop
/// patcher) holds them for every size of [`crate::ENGINE_BLOCKS`].
pub const NAMED_ENGINE_BLOCK: Option<usize> = match option_env!("POYETIK_ENGINE_BLOCK") {
    Some(name) => Some(named(name.as_bytes(), &NAMED)),
    None => None,
};

const _: () = assert!(
    !matches!(NAMED_ENGINE_BLOCK, Some(0)),
    "POYETIK_ENGINE_BLOCK must name one of audio_core::ENGINE_BLOCKS in decimal"
);

/// Whether a build naming `named` (none for every size) holds the code of the box types
/// created through their virtual table for the engine block of `frames` frames: every size
/// of [`crate::ENGINE_BLOCKS`] when it names none, else the one it names. A build asks it
/// of its own name, [`NAMED_ENGINE_BLOCK`].
pub(crate) const fn holds(named: Option<usize>, frames: usize) -> bool {
    match named {
        Some(named_frames) => named_frames == frames,
        None => true,
    }
}

/// `value` when `held`, else none: a constructor a build holds, or none it does not compile
/// ([`holds`]).
pub(crate) const fn held<T: Copy>(held: bool, value: T) -> Option<T> {
    if held { Some(value) } else { None }
}

/// Each engine block a build offers, by its decimal name: [`crate::ENGINE_BLOCKS`], held
/// equal by a test.
const NAMED: [(&str, usize); 7] = [
    ("16", 16),
    ("32", 32),
    ("64", 64),
    ("128", 128),
    ("256", 256),
    ("512", 512),
    ("1024", 1024),
];

/// The engine block of `named_sizes` called `name`, 0 when none is.
const fn named(name: &[u8], named_sizes: &[(&str, usize)]) -> usize {
    match named_sizes.split_first() {
        Some(((known, frames), rest)) => {
            if same(name, known.as_bytes()) {
                *frames
            } else {
                named(name, rest)
            }
        }
        None => 0,
    }
}

/// Whether `first` and `second` are the same bytes.
const fn same(first: &[u8], second: &[u8]) -> bool {
    match (first.split_first(), second.split_first()) {
        (Some((one, first_rest)), Some((other, second_rest))) => {
            *one == *other && same(first_rest, second_rest)
        }
        (None, None) => true,
        _ => false,
    }
}

#[cfg(test)]
#[path = "block_tests.rs"]
mod tests;
