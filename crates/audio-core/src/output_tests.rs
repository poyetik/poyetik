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
    let mut narrow_stage = OutputStage::new();
    narrow_stage.apply(&mut narrow_blocks);
    for (after, kept) in narrow_blocks
        .iter()
        .flatten()
        .zip(narrow_before.iter().flatten())
    {
        assert_eq!(after.bits(), kept.bits(), "binary32 {kept:?}");
    }
    assert_eq!(stage.replaced(), Replaced::default(), "nothing replaced");
    assert_eq!(
        narrow_stage.replaced(),
        Replaced::default(),
        "nothing replaced"
    );
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
        OutputStage::<Binary64>::default().frame,
        OutputStage::<Binary64>::new().frame,
        "the same start"
    );
}

#[test]
fn tells_a_block_whole_finite_or_not_at_every_position() {
    use crate::math::all_finite;
    // Finite values a sum of the samples themselves would overflow, of every sign: the
    // check must not add them.
    let mut huge = [Binary64::new(f64::MAX); BLOCK_SIZE];
    huge[1] = Binary64::new(-f64::MAX);
    huge[2] = Binary64::new(-0.0);
    huge[3] = Binary64::new(f64::from_bits(1));
    assert!(all_finite(&huge), "finite, however large");
    let narrow = [Binary32::from_raw(f32::MAX); BLOCK_SIZE];
    assert!(all_finite(&narrow), "finite in binary32 too");
    assert!(all_finite(&Binary64::SILENCE), "silence");
    for position in [0, 7, 8, BLOCK_SIZE - 1] {
        for value in NON_FINITE_64 {
            let mut block = [Binary64::new(1.0); BLOCK_SIZE];
            block[position] = Binary64::new(value);
            assert!(!all_finite(&block), "{value:?} at {position}");
        }
        for value in NON_FINITE_32 {
            let mut block = [Binary32::from_raw(1.0); BLOCK_SIZE];
            block[position] = Binary32::from_raw(value);
            assert!(!all_finite(&block), "{value:?} at {position}");
        }
    }
}

#[test]
fn counts_a_sample_its_host_replaced_as_given_in_its_block() {
    let mut stage = OutputStage::new();
    let mut first = [Binary64::SILENCE];
    stage.apply(&mut first);
    let mut second = [Binary64::SILENCE];
    stage.apply(&mut second);
    // The block applied last starts at frame BLOCK_SIZE.
    stage.note_given::<BLOCK_SIZE>(5, 1);
    stage.note_given::<BLOCK_SIZE>(2, 3);
    let start = u64::try_from(BLOCK_SIZE).expect("a frame count");
    assert_eq!(
        stage.replaced(),
        Replaced {
            count: 2,
            first: Some(FirstReplaced {
                frame: start + 2,
                channel: 3
            })
        },
        "counted from the first frame of the block applied last"
    );
    let mut saturated = OutputStage::<Binary64>::new();
    saturated.frame = u64::MAX;
    saturated.note_given::<BLOCK_SIZE>(usize::MAX, 0);
    assert_eq!(
        saturated.replaced().first,
        Some(FirstReplaced {
            frame: u64::MAX,
            channel: 0
        }),
        "the frame saturates"
    );
}

/// A protected limiter at 48 kHz for `channels` channels, its maximum lookahead 0.5 ms (24
/// frames, a latency of 30).
fn limiter<S: Sample>(channels: usize) -> crate::TruePeakLimiter<S> {
    crate::TruePeakLimiter::new(
        crate::SampleRate::new(48_000).expect("a rate"),
        u16::try_from(channels).expect("few channels"),
        Binary64::new(0.5_f64),
        crate::Detector::Standard,
        crate::LimiterSettings::new(
            Binary64::new(-1.0_f64),
            Binary64::new(0.5_f64),
            Binary64::new(100.0_f64),
            Binary64::new(0.5_f64),
        )
        .expect("settings"),
        true,
    )
    .expect("a limiter")
}

#[test]
fn a_raw_stage_has_no_latency_nor_limiter() {
    let mut stage = OutputStage::<Binary64>::new();
    assert_eq!(stage.latency(), 0, "no latency");
    assert!(stage.limiter().is_none(), "no limiter");
}

#[test]
fn a_limited_stage_replaces_then_limits_and_delays_by_its_latency() {
    let mut stage = OutputStage::limited(limiter::<Binary64>(1));
    assert_eq!(
        stage.latency(),
        30,
        "24 frames of lookahead and the detector's 6"
    );
    assert!(
        stage.limiter().is_some_and(|limiter| limiter.protected()),
        "protected"
    );
    // A NaN, then quiet samples: replaced before the limiter sees it, so that the limiter
    // passes zero and the quiet samples through unchanged, delayed.
    let mut first = [block(&[Binary64::new(f64::NAN), Binary64::new(0.25_f64)])];
    stage.apply(&mut first);
    let mut second = [Binary64::SILENCE];
    stage.apply(&mut second);
    assert_eq!(
        stage.replaced(),
        Replaced {
            count: 1,
            first: Some(FirstReplaced {
                frame: 0,
                channel: 0
            })
        },
        "the NaN, at the frame the plan computed it"
    );
    assert_eq!(first[0][29].bits(), 0, "silence while the delay fills");
    assert_eq!(first[0][30].bits(), 0, "the NaN, replaced by +0");
    assert_eq!(
        first[0][31].bits(),
        0.25_f64.to_bits(),
        "the quiet sample, bit for bit"
    );
    assert_eq!(second[0][0].bits(), 0, "then silence");
}

#[test]
fn counts_what_a_narrower_format_makes_infinite_at_the_frame_computed() {
    // Beyond the largest binary32: infinite once rounded, of its sign.
    let beyond = 1.0e300_f64;
    let mut stage = OutputStage::<Binary64>::new();
    {
        let mut given = [Binary32::SILENCE; 2];
        stage.apply_narrowing(&mut [Binary64::SILENCE; 2], &mut given);
        // The second block: two of its samples become infinite once rounded.
        let mut blocks = [Binary64::SILENCE; 2];
        blocks[1][40] = Binary64::new(beyond);
        blocks[1][45] = Binary64::new(-beyond);
        stage.apply_narrowing(&mut blocks, &mut given);
        assert_eq!(given[1][40].bits(), 0, "replaced by +0");
        assert_eq!(given[1][45].bits(), 0, "replaced by +0");
        assert_eq!(blocks[1][40].raw(), beyond, "the block as computed, kept");
        let block = u64::try_from(BLOCK_SIZE).expect("a frame count");
        assert_eq!(
            stage.replaced(),
            Replaced {
                count: 2,
                first: Some(FirstReplaced {
                    frame: block + 40,
                    channel: 1
                })
            },
            "counted at the frame the plan computed them"
        );
        let mut finite = [Binary64::SILENCE];
        stage.apply_narrowing(&mut finite, &mut [Binary32::SILENCE]);
        assert_eq!(stage.replaced().count, 2, "a finite block counts nothing");
    }
}

#[test]
fn counts_a_sample_given_infinite_at_the_frame_the_plan_computed_it() {
    // A frame given infinite by the host, `latency` frames after the plan computed it.
    let mut stage = OutputStage::limited(limiter::<Binary64>(1));
    stage.apply_narrowing(&mut [Binary64::SILENCE], &mut [Binary32::SILENCE]);
    stage.note_given::<BLOCK_SIZE>(40, 0);
    assert_eq!(
        stage.replaced().first,
        Some(FirstReplaced {
            frame: 40 - 30,
            channel: 0
        }),
        "the frame given, less the latency of 30 frames"
    );
}

#[test]
fn before_a_narrowing_a_raw_stage_counts_its_blocks_and_a_limited_one_applies_itself() {
    // Raw: the block is only counted, its non-finite samples replaced as given, at the
    // frames of the block applied last; the block as computed keeps them.
    let mut raw = OutputStage::<Binary64>::new();
    let mut given = [Binary32::SILENCE];
    raw.apply_narrowing(&mut [Binary64::SILENCE], &mut given);
    let mut nan = [block(&[
        Binary64::SILENCE[0],
        Binary64::SILENCE[0],
        Binary64::new(f64::NAN),
    ])];
    raw.apply_narrowing(&mut nan, &mut given);
    assert!(nan[0][2].raw().is_nan(), "left to the check as given");
    assert_eq!(given[0][2].bits(), 0, "replaced as given");
    let block_frames = u64::try_from(BLOCK_SIZE).expect("a frame count");
    assert_eq!(
        raw.replaced(),
        Replaced {
            count: 1,
            first: Some(FirstReplaced {
                frame: block_frames + 2,
                channel: 0
            })
        },
        "counted in the second block"
    );
    // Limited: replaced and limited before the rounding, so delayed by its latency.
    let mut limited = OutputStage::limited(limiter::<Binary64>(1));
    let mut quiet = [block(&[Binary64::new(0.25_f64)])];
    let mut given_limited = [block(&[Binary32::from_raw(0.5_f32)])];
    limited.apply_narrowing(&mut quiet, &mut given_limited);
    assert_eq!(quiet[0][0].bits(), 0, "delayed: silence first");
    assert_eq!(given_limited[0][0].bits(), 0, "and given_limited so");
    let mut nan_limited = [block(&[Binary64::new(f64::NAN)])];
    limited.apply_narrowing(&mut nan_limited, &mut given_limited);
    assert_eq!(
        nan_limited[0][0].bits(),
        0,
        "replaced before the limiter, as computed"
    );
    assert_eq!(limited.replaced().count, 1, "counted once");
}
