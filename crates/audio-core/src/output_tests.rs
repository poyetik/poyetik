//! Tests of the output stage: non-finite samples replaced and reported, every other sample
//! kept bit for bit.

use super::{FirstReplaced, OutputStage, Replaced};
use crate::host::Raw;
use crate::{BLOCK_SIZE, Binary32, Binary64, Block, Sample};

/// Every class of finite binary64 value, signs included: zeros, the smallest subnormal, the
/// largest subnormal, the smallest and largest normals, and an ordinary value.
const FINITE_64: [f64; 12] = [
    0.0,
    -0.0,
    f64::from_bits(1),
    -f64::from_bits(1),
    f64::from_bits(0x000f_ffff_ffff_ffff),
    f64::MIN_POSITIVE,
    -f64::MIN_POSITIVE,
    f64::MAX,
    f64::MIN,
    0.5,
    -0.25,
    1.0,
];

/// Every non-finite binary64 value class: both infinities, quiet and signaling NaNs of both
/// signs, with payloads.
const NON_FINITE_64: [f64; 6] = [
    f64::INFINITY,
    f64::NEG_INFINITY,
    f64::NAN,
    -f64::NAN,
    f64::from_bits(0x7ff0_0000_0000_0001),
    f64::from_bits(0xfff4_0000_0000_beef),
];

/// The binary32 counterparts of [`FINITE_64`].
const FINITE_32: [f32; 12] = [
    0.0,
    -0.0,
    f32::from_bits(1),
    -f32::from_bits(1),
    f32::from_bits(0x007f_ffff),
    f32::MIN_POSITIVE,
    -f32::MIN_POSITIVE,
    f32::MAX,
    f32::MIN,
    0.5,
    -0.25,
    1.0,
];

/// The binary32 counterparts of [`NON_FINITE_64`].
const NON_FINITE_32: [f32; 6] = [
    f32::INFINITY,
    f32::NEG_INFINITY,
    f32::NAN,
    -f32::NAN,
    f32::from_bits(0x7f80_0001),
    f32::from_bits(0xffa0_beef),
];

/// A block holding `values` from its first frame, zeros after.
fn block<S: Sample>(values: &[S]) -> Block<S> {
    let mut block = S::SILENCE;
    for (sample, value) in block.iter_mut().zip(values) {
        *sample = *value;
    }
    block
}

#[test]
fn keeps_every_finite_sample_bit_for_bit() {
    let wide: alloc::vec::Vec<Binary64> = FINITE_64
        .iter()
        .map(|value| Binary64::new(*value))
        .collect();
    let mut wide_blocks = [block(&wide), block(&wide)];
    let wide_before = wide_blocks;
    let mut stage = OutputStage::new();
    stage.apply(&mut wide_blocks);
    for (after, kept) in wide_blocks
        .iter()
        .flatten()
        .zip(wide_before.iter().flatten())
    {
        assert_eq!(after.bits(), kept.bits(), "binary64 {kept:?}");
    }
    let narrow: alloc::vec::Vec<Binary32> = FINITE_32
        .iter()
        .map(|value| Binary32::from_raw(*value))
        .collect();
    let mut narrow_blocks = [block(&narrow)];
    let narrow_before = narrow_blocks;
    stage.apply(&mut narrow_blocks);
    for (after, kept) in narrow_blocks
        .iter()
        .flatten()
        .zip(narrow_before.iter().flatten())
    {
        assert_eq!(after.bits(), kept.bits(), "binary32 {kept:?}");
    }
    assert_eq!(stage.replaced(), Replaced::default(), "nothing replaced");
}

#[test]
fn replaces_every_non_finite_sample_by_positive_zero() {
    for value in NON_FINITE_64 {
        let mut blocks = [block(&[Binary64::new(1.0), Binary64::new(value)])];
        let mut stage = OutputStage::new();
        stage.apply(&mut blocks);
        assert_eq!(
            blocks[0][1].bits(),
            0.0_f64.to_bits(),
            "{value:?} replaced by +0"
        );
        assert_eq!(
            blocks[0][0].raw().to_bits(),
            1.0_f64.to_bits(),
            "its neighbor kept"
        );
        assert_eq!(
            stage.replaced(),
            Replaced {
                count: 1,
                first: Some(FirstReplaced {
                    frame: 1,
                    channel: 0
                })
            },
            "{value:?} reported"
        );
    }
    for value in NON_FINITE_32 {
        let mut blocks = [block(&[Binary32::from_raw(value)])];
        let mut stage = OutputStage::new();
        stage.apply(&mut blocks);
        assert_eq!(
            blocks[0][0].raw().to_bits(),
            0.0_f32.to_bits(),
            "{value:?} replaced by +0"
        );
        assert_eq!(stage.replaced().count, 1, "{value:?} counted");
    }
}

#[test]
fn reports_the_count_and_the_earliest_frame_then_the_lowest_channel() {
    let nan = Binary64::new(f64::NAN);
    let one = Binary64::new(1.0);
    let mut stage = OutputStage::new();
    // A later frame on channel 0, an earlier one on channel 2, and the same frame on
    // channel 1 and 2: channel-major order must not decide the first.
    let mut first = [
        block(&[one, one, one, one, one, nan]),
        block(&[one, one, one, nan]),
        block(&[one, one, one, nan, nan]),
    ];
    stage.apply(&mut first);
    assert_eq!(
        stage.replaced(),
        Replaced {
            count: 4,
            first: Some(FirstReplaced {
                frame: 3,
                channel: 1
            })
        }
    );
    // A later block counts from its own first frame, and never moves the first.
    let mut second = [block(&[nan]), block(&[one]), block(&[one])];
    stage.apply(&mut second);
    assert_eq!(stage.replaced().count, 5, "counted across blocks");
    assert_eq!(
        stage.replaced().first,
        Some(FirstReplaced {
            frame: 3,
            channel: 1
        }),
        "the first stays the first"
    );
    // Its frame: the second block's first frame is BLOCK_SIZE.
    let mut later = OutputStage::new();
    let mut silent = [Binary64::SILENCE];
    later.apply(&mut silent);
    let mut next = [block(&[one, nan])];
    later.apply(&mut next);
    assert_eq!(
        later.replaced().first,
        Some(FirstReplaced {
            frame: u64::try_from(BLOCK_SIZE).expect("a frame count") + 1,
            channel: 0
        }),
        "frames count from the stage's first block"
    );
}

#[test]
fn saturates_its_count_and_its_frame() {
    let nan = Binary64::new(f64::NAN);
    let mut stage = OutputStage::new();
    stage.replaced.count = u64::MAX - 1;
    stage.frame = u64::MAX - 1;
    let mut blocks = [block(&[nan, nan, nan])];
    stage.apply(&mut blocks);
    assert_eq!(stage.replaced().count, u64::MAX, "the count saturates");
    assert_eq!(
        stage.replaced().first,
        Some(FirstReplaced {
            frame: u64::MAX - 1,
            channel: 0
        }),
        "the first frame as it was"
    );
    assert_eq!(stage.frame, u64::MAX, "the frame saturates");
    let mut more = [block(&[nan])];
    stage.apply(&mut more);
    assert_eq!(stage.replaced().count, u64::MAX, "still saturated");
}

#[test]
fn a_stage_without_channels_still_advances() {
    let mut stage = OutputStage::new();
    let mut none: [Block<Binary64>; 0] = [];
    stage.apply(&mut none);
    assert_eq!(
        stage.frame,
        u64::try_from(BLOCK_SIZE).expect("a frame count"),
        "one block of frames"
    );
    assert_eq!(
        OutputStage::default().frame,
        OutputStage::new().frame,
        "the same start"
    );
}
