//! Tests of what a host asks and what the engine applies.

#![forbid(unsafe_code)]

use alloc::string::ToString;
use alloc::vec;

use super::{Direction, HostSettings, MAX_CHANNELS, Unhonored};
use crate::{CoreError, SampleRate};

/// A host asking for 44.1 kHz, blocks of 64 frames, 2 outputs and no input.
const STEREO: HostSettings = HostSettings::new(44_100, 64, 2, 0);

#[test]
fn builds_settings_in_the_order_of_their_fields() {
    assert_eq!(
        HostSettings::new(48_000, 441, 3, 1),
        HostSettings {
            rate: 48_000,
            block: 441,
            outputs: 3,
            inputs: 1,
        }
    );
}

#[test]
fn honors_what_it_can_and_says_nothing() {
    let applied = STEREO.apply::<64>(2).expect("settings an engine applies");
    assert_eq!(
        applied.rate(),
        SampleRate::new(44_100).expect("a rate"),
        "rate"
    );
    assert_eq!(applied.block(), 64, "block");
    assert_eq!(applied.outputs(), 2, "outputs");
    assert_eq!(applied.inputs(), 0, "inputs");
    assert_eq!(applied.unhonored(), [], "nothing unhonored");
}

#[test]
fn applies_its_own_block_and_says_so() {
    let applied = STEREO.apply::<128>(2).expect("settings an engine applies");
    assert_eq!(applied.block(), 128, "the engine's block");
    assert_eq!(
        applied.unhonored(),
        [Unhonored::Block {
            asked: 64,
            applied: 128
        }],
        "the block said"
    );
}

#[test]
fn drops_or_silences_the_channels_the_other_side_lacks() {
    let fewer = STEREO.apply::<64>(3).expect("settings an engine applies");
    assert_eq!(fewer.outputs(), 2, "the host's outputs");
    assert_eq!(
        fewer.unhonored(),
        [Unhonored::Dropped { patch: 3, host: 2 }],
        "one patch channel too many"
    );
    let more = STEREO.apply::<64>(1).expect("settings an engine applies");
    assert_eq!(
        more.unhonored(),
        [Unhonored::Silent { patch: 1, host: 2 }],
        "one host channel too many"
    );
    let none = HostSettings {
        outputs: 0,
        ..STEREO
    }
    .apply::<64>(0)
    .expect("settings an engine applies");
    assert_eq!(none.unhonored(), [], "no output either side");
}

#[test]
fn accepts_inputs_unread_and_says_so() {
    let applied = HostSettings {
        inputs: 8,
        ..STEREO
    }
    .apply::<64>(2)
    .expect("settings an engine applies");
    assert_eq!(applied.inputs(), 8, "the host's inputs");
    assert_eq!(
        applied.unhonored(),
        [Unhonored::InputsUnread { inputs: 8 }],
        "unread inputs said"
    );
}

#[test]
fn says_every_unhonored_setting_in_order() {
    let applied = HostSettings {
        rate: 48_000,
        block: 441,
        outputs: 4,
        inputs: 1,
    }
    .apply::<32>(2)
    .expect("settings an engine applies");
    assert_eq!(
        applied.unhonored(),
        vec![
            Unhonored::Block {
                asked: 441,
                applied: 32
            },
            Unhonored::Silent { patch: 2, host: 4 },
            Unhonored::InputsUnread { inputs: 1 },
        ],
        "block, outputs, inputs"
    );
}

#[test]
fn refuses_what_no_engine_applies_at_the_limits_and_one_past() {
    assert_eq!(MAX_CHANNELS, 512, "decision 1's maximum");
    for direction in [Direction::Outputs, Direction::Inputs] {
        let with = |count: u16| match direction {
            Direction::Outputs => HostSettings {
                outputs: count,
                ..STEREO
            },
            Direction::Inputs => HostSettings {
                inputs: count,
                ..STEREO
            },
        };
        assert!(with(512).apply::<64>(2).is_ok(), "512 {direction} channels");
        assert_eq!(
            with(513).apply::<64>(2),
            Err(CoreError::TooManyChannels {
                direction,
                count: 513
            }),
            "513 {direction} channels"
        );
    }
    assert_eq!(
        HostSettings {
            rate: 7_999,
            ..STEREO
        }
        .apply::<64>(2),
        Err(CoreError::InvalidSampleRate(7_999)),
        "a rate below the range"
    );
}

#[test]
fn holds_each_count_at_the_limits_of_its_fixed_width() {
    // The largest block a host may ask, the largest engine block (2^20, beyond `u16`, written
    // apart) and the largest patch channel count, each kept whole.
    let applied = HostSettings {
        block: u32::MAX,
        outputs: 0,
        ..STEREO
    }
    .apply::<{ crate::MAX_ENGINE_BLOCK }>(u16::MAX)
    .expect("settings an engine applies");
    assert_eq!(applied.block(), 1_048_576, "the largest engine block");
    assert_eq!(
        applied.unhonored(),
        [
            Unhonored::Block {
                asked: u32::MAX,
                applied: 1_048_576
            },
            Unhonored::Dropped {
                patch: u16::MAX,
                host: 0
            }
        ],
        "every count whole"
    );
    let smallest = STEREO.apply::<1>(0).expect("settings an engine applies");
    assert_eq!(smallest.block(), 1, "the smallest engine block");
}

#[test]
fn explains_each_reason_in_one_line() {
    for (reason, line) in [
        (
            Unhonored::Block {
                asked: 441,
                applied: 64,
            },
            "engine block of 441 frames asked, 64 applied: the engine is built for 64",
        ),
        (
            Unhonored::Dropped { patch: 3, host: 2 },
            "the patch has 3 output channels and the host 2: the ones beyond 2 are dropped",
        ),
        (
            Unhonored::Silent { patch: 1, host: 2 },
            "the host has 2 output channels and the patch 1: the ones beyond 1 are silent",
        ),
        (
            Unhonored::InputsUnread { inputs: 8 },
            "8 input channels accepted and unread: no box reads an input yet",
        ),
    ] {
        assert_eq!(reason.to_string(), line, "{reason:?}");
    }
    assert_eq!(Direction::Outputs.to_string(), "output", "outputs named");
    assert_eq!(Direction::Inputs.to_string(), "input", "inputs named");
}
