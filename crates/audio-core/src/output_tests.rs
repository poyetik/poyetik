//! Tests of the output stage: non-finite samples replaced and reported, every other sample
//! kept bit for bit.

use super::{AsComputed, FirstReplaced, LimiterMode, OutputStage, Replaced, StageError};
use crate::host::{Delivered, Raw};
use crate::tests::limiter;
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

/// Applies `stage` to `blocks`, one per channel of the stage: written into its blocks, then
/// read back as applied.
fn apply<S: Sample, D: AsComputed<S>, const N: usize>(
    stage: &mut OutputStage<S, D, N>,
    blocks: &mut [Block<S, N>],
) {
    assert_eq!(blocks.len(), stage.channels(), "one block per channel");
    let applied = stage.apply(|into| into.copy_from_slice(blocks));
    blocks.copy_from_slice(applied);
}

/// Applies `stage` to `blocks`, one per channel of the stage, given as binary32 in `given`:
/// written into its blocks, then read back as applied.
fn apply_narrowing<const N: usize>(
    stage: &mut OutputStage<Binary64, f32, N>,
    blocks: &mut [Block<Binary64, N>],
    given: &mut [Block<Binary32, N>],
) {
    assert_eq!(blocks.len(), stage.channels(), "one block per channel");
    assert_eq!(given.len(), stage.channels(), "one block per channel");
    given.copy_from_slice(stage.apply_narrowing(|into| into.copy_from_slice(blocks)));
    blocks.copy_from_slice(stage.computed_blocks());
}

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
    let mut stage = OutputStage::<_, f64>::new(2);
    apply(&mut stage, &mut wide_blocks);
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
    let mut narrow_stage = OutputStage::<_, f32>::new(1);
    apply(&mut narrow_stage, &mut narrow_blocks);
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
        let mut stage = OutputStage::<_, f64>::new(1);
        apply(&mut stage, &mut blocks);
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
        let mut stage = OutputStage::<_, f32>::new(1);
        apply(&mut stage, &mut blocks);
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
    let mut stage = OutputStage::<_, f64>::new(3);
    // A later frame on channel 0, an earlier one on channel 2, and the same frame on
    // channel 1 and 2: channel-major order must not decide the first.
    let mut first = [
        block(&[one, one, one, one, one, nan]),
        block(&[one, one, one, nan]),
        block(&[one, one, one, nan, nan]),
    ];
    apply(&mut stage, &mut first);
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
    apply(&mut stage, &mut second);
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
    let mut later = OutputStage::<_, f64>::new(1);
    let mut silent = [Binary64::SILENCE];
    apply(&mut later, &mut silent);
    let mut next = [block(&[one, nan])];
    apply(&mut later, &mut next);
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
fn wraps_its_frame_past_any_stream() {
    // A stream of 2^64 frames at a billion frames a second lasts more than 500 years.
    let seconds = u64::MAX / 1_000_000_000;
    assert!(seconds / (365 * 24 * 3600) > 500, "{seconds} s");
    let mut stage = OutputStage::<_, f64>::new(1);
    stage.stream.frame = u64::MAX - 1;
    apply(&mut stage, &mut [Binary64::SILENCE]);
    assert_eq!(
        stage.stream.frame,
        u64::try_from(BLOCK_SIZE).expect("a frame count") - 2,
        "wrapped"
    );
}

#[test]
fn saturates_its_count_and_its_frames_within_a_block() {
    let nan = Binary64::new(f64::NAN);
    let mut stage = OutputStage::<_, f64>::new(1);
    stage.stream.replaced.count = u64::MAX - 1;
    stage.stream.frame = u64::MAX - 1;
    let mut blocks = [block(&[nan, nan, nan])];
    apply(&mut stage, &mut blocks);
    assert_eq!(stage.replaced().count, u64::MAX, "the count saturates");
    assert_eq!(
        stage.replaced().first,
        Some(FirstReplaced {
            frame: u64::MAX - 1,
            channel: 0
        }),
        "the first frame as it was"
    );
    let mut more = [block(&[nan])];
    apply(&mut stage, &mut more);
    assert_eq!(stage.replaced().count, u64::MAX, "still saturated");
}

#[test]
fn a_stage_without_channels_still_advances() {
    let mut stage = OutputStage::<_, f64>::new(0);
    let mut none: [Block<Binary64>; 0] = [];
    apply(&mut stage, &mut none);
    assert_eq!(
        stage.stream.frame,
        u64::try_from(BLOCK_SIZE).expect("a frame count"),
        "one block of frames"
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
    let mut stage = OutputStage::<_, f64>::new(1);
    let mut first = [Binary64::SILENCE];
    apply(&mut stage, &mut first);
    let mut second = [Binary64::SILENCE];
    apply(&mut stage, &mut second);
    // The block applied last starts at frame BLOCK_SIZE.
    stage.note_given(5, 1);
    stage.note_given(2, 3);
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
    let mut saturated = OutputStage::<Binary64, f64>::new(1);
    saturated.stream.frame = u64::MAX;
    saturated.note_given(usize::MAX, 0);
    assert_eq!(
        saturated.replaced().first,
        Some(FirstReplaced {
            frame: u64::MAX,
            channel: 0
        }),
        "the frame saturates"
    );
}

#[test]
fn a_raw_stage_has_no_latency_nor_limiter_to_set() {
    let mut stage = OutputStage::<Binary64, f64>::new(1);
    assert_eq!(stage.latency(), 0, "no latency");
    assert!(stage.limiter().is_none(), "no limiter");
    assert_eq!(
        stage.set_limiter(crate::LimiterSettings::DEFAULT),
        Err(StageError::Raw),
        "no limiter to set"
    );
    assert_eq!(
        stage.set_protected(true),
        Err(StageError::Raw),
        "nor to protect"
    );
}

#[test]
fn sets_its_limiter_live_and_tells_a_refusal() {
    let mut stage = OutputStage::<Binary64, f64>::limited(limiter(1));
    let settings = crate::LimiterSettings::new(
        Binary64::new(-3.0_f64),
        Binary64::new(0.5_f64),
        Binary64::new(200.0_f64),
        Binary64::new(0.25_f64),
    )
    .expect("settings");
    assert_eq!(stage.set_limiter(settings), Ok(()), "within its maximum");
    assert_eq!(
        stage.limiter().map(crate::TruePeakLimiter::settings),
        Some(settings),
        "in force"
    );
    // 1 ms of lookahead, above the maximum of 0.5 ms `limiter` is built with.
    let above = settings
        .with_lookahead(Binary64::new(1.0_f64))
        .expect("a lookahead in range");
    let refused = stage.set_limiter(above);
    assert_eq!(
        refused,
        Err(StageError::Limiter(crate::LimiterError::AboveMaximum {
            lookahead: Binary64::new(1.0_f64),
            maximum: Binary64::new(0.5_f64),
        })),
        "above its maximum"
    );
    assert_eq!(
        stage.limiter().map(crate::TruePeakLimiter::settings),
        Some(settings),
        "unchanged"
    );
    // Its protection, too, through the stage alone: `limiter` is protected, so it stays
    // (`a_protected_stage_refuses_raw_and_keeps_protecting`).
    assert_eq!(
        stage.set_protected(false),
        Err(StageError::Locked),
        "locked"
    );
}

/// A protected stage, giving `D`, refuses to go raw and keeps protecting; a switchable one
/// goes both ways.
fn a_protected_stage_refuses_raw_and_keeps_protecting<S: Sample, D: Delivered<S>>() {
    let mut protected = OutputStage::<S, D>::limited(limiter::<S>(1));
    assert_eq!(
        protected.limiter().map(crate::TruePeakLimiter::mode),
        Some(LimiterMode::Protected),
        "built protected"
    );
    assert_eq!(
        protected.set_protected(false),
        Err(StageError::Locked),
        "refused raw"
    );
    assert_eq!(
        protected.limiter().map(crate::TruePeakLimiter::protected),
        Some(true),
        "still protecting"
    );
    assert_eq!(
        protected.set_protected(true),
        Ok(()),
        "told protected again"
    );
    let rate = crate::SampleRate::new(48_000).expect("a rate");
    let switchable = |protect| {
        let mut stage = OutputStage::<S, D>::limited(
            crate::TruePeakLimiter::new(
                rate,
                1,
                Binary64::new(1.5_f64),
                crate::Detector::Standard,
                crate::LimiterSettings::DEFAULT,
                LimiterMode::Switchable,
            )
            .expect("a limiter"),
        );
        (
            stage.limiter().map(crate::TruePeakLimiter::protected),
            stage.set_protected(protect),
            stage.limiter().map(crate::TruePeakLimiter::protected),
        )
    };
    assert_eq!(
        switchable(true),
        (Some(false), Ok(()), Some(true)),
        "switchable: raw first, then protected"
    );
    assert_eq!(
        switchable(false),
        (Some(false), Ok(()), Some(false)),
        "switchable: told raw while raw"
    );
}

#[test]
fn a_protected_stage_refuses_raw_and_keeps_protecting_in_every_format() {
    a_protected_stage_refuses_raw_and_keeps_protecting::<Binary64, f64>();
    a_protected_stage_refuses_raw_and_keeps_protecting::<Binary64, f32>();
    a_protected_stage_refuses_raw_and_keeps_protecting::<Binary32, f32>();
}

#[test]
fn tells_why_it_refused_a_setting_with_its_cause() {
    use core::error::Error as _;
    assert_eq!(
        alloc::format!("{}", StageError::Raw),
        "the output stage is raw: it has no limiter to set"
    );
    assert!(StageError::Raw.source().is_none(), "no cause");
    assert_eq!(
        alloc::format!("{}", StageError::Locked),
        "the output stage is protected: only a switchable stage is switched to raw"
    );
    assert!(StageError::Locked.source().is_none(), "no cause either");
    let cause = crate::LimiterError::AboveMaximum {
        lookahead: Binary64::new(1.0_f64),
        maximum: Binary64::new(0.5_f64),
    };
    let refused = StageError::Limiter(cause);
    assert_eq!(
        alloc::format!("{refused}"),
        "limiter lookahead 1 ms is above its maximum of 0.5 ms",
        "the limiter's words"
    );
    assert_eq!(
        refused.source().map(alloc::string::ToString::to_string),
        Some(alloc::string::ToString::to_string(&cause)),
        "its cause kept"
    );
}

#[test]
fn hands_its_produce_exactly_its_channels_and_limits_each() {
    for channels in [0, 1, 3] {
        let mut stage = OutputStage::<Binary64, f64>::new(channels);
        assert_eq!(stage.channels(), channels, "raw, {channels}");
        let handed = stage
            .apply(|blocks| assert_eq!(blocks.len(), channels))
            .len();
        assert_eq!(handed, channels, "raw, {channels} applied");
    }
    // 2.0 everywhere, above the ceiling of -1 dBTP (`limiter`): every channel the stage
    // hands is limited (a limiter of one channel applied to two blocks passed the second
    // unlimited, at 2.0).
    // 10^(-1/20), the nearest binary64.
    let ceiling = 0.891_250_938_133_745_6_f64;
    for channels in [1_usize, 2] {
        let mut stage = OutputStage::<Binary64, f64>::limited(limiter(channels));
        assert_eq!(stage.channels(), channels, "limited, {channels}");
        let mut peaks = alloc::vec![0.0_f64; channels];
        // 100 blocks, well past the latency of 30 frames.
        for _ in 0_u32..100 {
            let applied = stage.apply(|blocks| {
                assert_eq!(blocks.len(), channels, "limited, {channels} handed");
                for block in blocks.iter_mut() {
                    block.fill(Binary64::new(2.0_f64));
                }
            });
            for (peak, block) in peaks.iter_mut().zip(applied.iter()) {
                for sample in block {
                    let level = sample.raw().abs();
                    if level > *peak {
                        *peak = level;
                    }
                }
            }
        }
        assert!(
            peaks.iter().all(|peak| *peak > 0.8_f64 && *peak <= ceiling),
            "every channel limited, {channels}: {peaks:?}"
        );
    }
}

#[test]
fn hands_its_produce_the_blocks_it_returned_last() {
    // The blocks are never cleared between calls: `produce` receives them as the stage
    // returned them last (replaced, then limited and delayed), so a block it leaves
    // unwritten is applied again as it was given. Raw: a NaN at frame 3 among 0.5.
    let mut nan_among_halves = [Binary64::new(0.5_f64); BLOCK_SIZE];
    nan_among_halves[3] = Binary64::new(f64::NAN);
    let mut raw = OutputStage::<Binary64, f64>::new(1);
    let given = raw.apply(|blocks| blocks[0] = nan_among_halves)[0];
    assert_eq!(raw.blocks(), &[given], "read back as returned");
    let mut seen = None;
    let _ = raw.apply(|blocks| seen = Some(blocks[0]));
    let mut replaced = [Binary64::new(0.5_f64); BLOCK_SIZE];
    replaced[3] = Binary64::ZERO;
    assert_eq!(given, replaced, "the NaN replaced");
    assert_eq!(seen, Some(replaced), "handed back as returned");
    assert_eq!(
        raw.replaced().count(),
        1,
        "applied again, nothing more to replace"
    );
    // Limited: 4.0 everywhere, 30 frames late, reduced under the ceiling.
    let mut limited = OutputStage::<Binary64, f64>::limited(limiter(1));
    let mut last = [Binary64::ZERO; BLOCK_SIZE];
    for _ in 0..3_u8 {
        last = limited.apply(|blocks| blocks[0] = [Binary64::new(4.0_f64); BLOCK_SIZE])[0];
    }
    let mut seen_limited = None;
    let _ = limited.apply(|blocks| seen_limited = Some(blocks[0]));
    assert!(
        last.iter()
            .all(|sample| sample.to_binary64() < Binary64::new(1.0_f64)),
        "the last block limited"
    );
    assert_eq!(
        seen_limited,
        Some(last),
        "handed back as limited and delayed"
    );
}

#[test]
fn a_limited_stage_replaces_then_limits_and_delays_by_its_latency() {
    let mut stage = OutputStage::<_, f64>::limited(limiter::<Binary64>(1));
    assert_eq!(
        stage.latency(),
        30,
        "24 frames of lookahead and the detector's 6"
    );
    assert!(
        stage
            .limiter()
            .is_some_and(crate::TruePeakLimiter::protected),
        "protected"
    );
    // A NaN, then quiet samples: replaced before the limiter sees it, so that the limiter
    // passes zero and the quiet samples through unchanged, delayed.
    let mut first = [block(&[Binary64::new(f64::NAN), Binary64::new(0.25_f64)])];
    apply(&mut stage, &mut first);
    let mut second = [Binary64::SILENCE];
    apply(&mut stage, &mut second);
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
    let mut stage = OutputStage::<Binary64, f32>::new(2);
    {
        let mut given = [Binary32::SILENCE; 2];
        apply_narrowing(&mut stage, &mut [Binary64::SILENCE; 2], &mut given);
        // The second block: two of its samples become infinite once rounded.
        let mut blocks = [Binary64::SILENCE; 2];
        blocks[1][40] = Binary64::new(beyond);
        blocks[1][45] = Binary64::new(-beyond);
        apply_narrowing(&mut stage, &mut blocks, &mut given);
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
        let mut finite = [Binary64::SILENCE; 2];
        apply_narrowing(&mut stage, &mut finite, &mut given);
        assert_eq!(stage.replaced().count, 2, "a finite block counts nothing");
    }
}

#[test]
fn counts_a_sample_given_infinite_at_the_frame_the_plan_computed_it() {
    // A frame given infinite by the host, `latency` frames after the plan computed it.
    let mut stage = OutputStage::<_, f32>::limited(limiter::<Binary64>(1));
    apply_narrowing(
        &mut stage,
        &mut [Binary64::SILENCE],
        &mut [Binary32::SILENCE],
    );
    stage.note_given(40, 0);
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
    let mut raw = OutputStage::<Binary64, f32>::new(1);
    let mut given = [Binary32::SILENCE];
    apply_narrowing(&mut raw, &mut [Binary64::SILENCE], &mut given);
    let mut nan = [block(&[
        Binary64::SILENCE[0],
        Binary64::SILENCE[0],
        Binary64::new(f64::NAN),
    ])];
    apply_narrowing(&mut raw, &mut nan, &mut given);
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
    let mut limited = OutputStage::<_, f32>::limited(limiter::<Binary64>(1));
    let mut quiet = [block(&[Binary64::new(0.25_f64)])];
    let mut given_limited = [block(&[Binary32::from_raw(0.5_f32)])];
    apply_narrowing(&mut limited, &mut quiet, &mut given_limited);
    assert_eq!(quiet[0][0].bits(), 0, "delayed: silence first");
    assert_eq!(given_limited[0][0].bits(), 0, "and given_limited so");
    let mut nan_limited = [block(&[Binary64::new(f64::NAN)])];
    apply_narrowing(&mut limited, &mut nan_limited, &mut given_limited);
    assert_eq!(
        nan_limited[0][0].bits(),
        0,
        "replaced before the limiter, as computed"
    );
    assert_eq!(limited.replaced().count, 1, "counted once");
}

#[test]
fn builds_a_report_only_from_a_count_and_a_first_sample_that_agree() {
    let at = FirstReplaced {
        frame: 9,
        channel: 2,
    };
    assert_eq!(
        Replaced::new(0, None),
        Some(Replaced::default()),
        "nothing replaced"
    );
    let one = Replaced::new(1, Some(at)).expect("one sample, its first");
    assert_eq!((one.count(), one.first()), (1, Some(at)), "read back");
    let most = Replaced::new(u64::MAX, Some(at)).expect("a saturated count");
    assert_eq!(most.count(), u64::MAX, "the largest count");
    assert_eq!(
        Replaced::new(1, None),
        None,
        "a count without a first sample"
    );
    assert_eq!(
        Replaced::new(0, Some(at)),
        None,
        "a first sample without a count"
    );
}

/// What a stage counts of a NaN computed at `frame`, in the second block, when it reports on
/// the first `until` frames only.
fn counted_until<S: Sample + Raw, D: AsComputed<S>>(
    stage: OutputStage<S, D>,
    frame: usize,
    until: u64,
) -> u64 {
    let mut stage = stage.reporting_until(until);
    apply(&mut stage, &mut [S::SILENCE]);
    let offset = frame
        .checked_sub(BLOCK_SIZE)
        .expect("a frame of the second block");
    let mut nan = [S::SILENCE];
    nan[0][offset] = S::from_binary64(Binary64::new(f64::NAN));
    apply(&mut stage, &mut nan);
    assert_eq!(nan[0][offset].bits(), 0, "replaced all the same");
    stage.replaced().count()
}

fn counts_only_what_it_delivers_within_the_frames_it_reports_on<S: Sample + Raw>()
where
    S::Float: AsComputed<S>,
{
    // Raw: frame 99 is delivered by a render of 100 frames, frame 100 by one of 101 only.
    assert_eq!(
        counted_until(OutputStage::<S, S::Float>::new(1), 99, 100),
        1,
        "raw, 99 of 100"
    );
    assert_eq!(
        counted_until(OutputStage::<S, S::Float>::new(1), 100, 100),
        0,
        "raw, 100 of 100"
    );
    assert_eq!(
        counted_until(OutputStage::<S, S::Float>::new(1), 100, 101),
        1,
        "raw, 100 of 101"
    );
    // Limited: delivered 30 frames later (the latency of `limiter`), so frame 100 only by a
    // render of 131 frames.
    let limited = || OutputStage::<S, S::Float>::limited(limiter::<S>(1));
    assert_eq!(
        limited().latency(),
        30,
        "the latency the frames below assume"
    );
    assert_eq!(counted_until(limited(), 99, 130), 1, "limited, 99 of 130");
    assert_eq!(counted_until(limited(), 100, 130), 0, "limited, 100 of 130");
    assert_eq!(counted_until(limited(), 100, 131), 1, "limited, 100 of 131");
    // A stream without an end counts everything.
    let mut endless = OutputStage::<S, S::Float>::new(1);
    apply(&mut endless, &mut [S::SILENCE]);
    let mut nan = [S::SILENCE];
    nan[0][BLOCK_SIZE - 1] = S::from_binary64(Binary64::new(f64::NAN));
    apply(&mut endless, &mut nan);
    assert_eq!(endless.replaced().count(), 1, "no end");
}

#[test]
fn counts_only_what_it_delivers_within_the_frames_it_reports_on_in_each_precision() {
    counts_only_what_it_delivers_within_the_frames_it_reports_on::<Binary64>();
    counts_only_what_it_delivers_within_the_frames_it_reports_on::<Binary32>();
}

#[test]
fn counts_a_sample_given_infinite_only_within_the_frames_it_reports_on() {
    // A frame given by the host at offset 40 of the second block: frame 104 as given, frame
    // 74 as computed by the plan (the latency of `limiter`, 30 frames, earlier).
    for (until, counted) in [(104_u64, 0_u64), (105, 1)] {
        let mut stage =
            OutputStage::<_, f32>::limited(limiter::<Binary64>(1)).reporting_until(until);
        apply_narrowing(
            &mut stage,
            &mut [Binary64::SILENCE],
            &mut [Binary32::SILENCE],
        );
        apply_narrowing(
            &mut stage,
            &mut [Binary64::SILENCE],
            &mut [Binary32::SILENCE],
        );
        stage.note_given(40, 0);
        assert_eq!(
            stage.replaced().count(),
            counted,
            "a render of {until} frames"
        );
    }
}

/// What a raw stage of engine block `N` gives and counts of a block whose samples are 0.5
/// but at `position`, where it is `value`: the sample given at `position`, the count, and
/// whether every other sample is given as 0.5.
fn narrowed_at<const N: usize>(position: usize, value: f64) -> (u32, u64, bool) {
    let mut stage = OutputStage::<Binary64, f32, N>::new(1);
    let mut wide = [[Binary64::new(0.5_f64); N]];
    wide[0][position] = Binary64::new(value);
    let mut given = [[Binary32::SILENCE[0]; N]];
    apply_narrowing(&mut stage, &mut wide, &mut given);
    let others = given[0]
        .iter()
        .enumerate()
        .all(|(at, sample)| at == position || sample.bits() == u64::from(0.5_f32.to_bits()));
    (
        given[0][position].raw().to_bits(),
        stage.replaced().count(),
        others,
    )
}

/// [`narrowed_at`] at every position of an engine block of `N` frames: a sample beyond the
/// largest binary32 one is given as +0 and counted, a NaN too, and a finite one is given
/// rounded and not counted, at every position, those of a group of eight and those after
/// the last eight.
fn narrows_and_checks_every_position<const N: usize>() {
    for position in 0..N {
        assert_eq!(
            narrowed_at::<N>(position, 1.0e300_f64),
            (0, 1, true),
            "infinite once rounded, at {position} of {N}"
        );
        assert_eq!(
            narrowed_at::<N>(position, f64::NAN),
            (0, 1, true),
            "NaN, at {position} of {N}"
        );
        assert_eq!(
            narrowed_at::<N>(position, 0.25_f64),
            (0.25_f32.to_bits(), 0, true),
            "finite, at {position} of {N}"
        );
    }
}

#[test]
fn narrows_and_checks_every_position_at_every_engine_block() {
    // 1 and 7: no group of eight; 8 and 64: groups only; 13 and 441: both.
    narrows_and_checks_every_position::<1>();
    narrows_and_checks_every_position::<7>();
    narrows_and_checks_every_position::<8>();
    narrows_and_checks_every_position::<13>();
    narrows_and_checks_every_position::<64>();
    narrows_and_checks_every_position::<441>();
}
