//! Tests of the engine core's errors and their descriptions.

use alloc::string::{String, ToString};
use alloc::vec;
use alloc::vec::Vec;

use super::CoreError;
use crate::{Connection, NodeId};

#[test]
fn describes_every_error() {
    let cases = [
        (
            CoreError::InvalidSampleRate(5),
            "invalid sample rate: 5 Hz (supported: 8000 to 384000 Hz)",
        ),
        (
            CoreError::DuplicateNode(NodeId(2)),
            "box 2 is declared twice",
        ),
        (
            CoreError::DuplicateConnection(Connection {
                from: NodeId(1),
                outlet: 0,
                to: NodeId(2),
                inlet: 1,
            }),
            "the cable from box 1 outlet 0 to box 2 inlet 1 is listed twice",
        ),
        (
            CoreError::TooManyNodes(70_000),
            "too many boxes: 70000 (at most 65536)",
        ),
        (
            CoreError::TooManyConnections(300_000),
            "too many cables: 300000 (at most 262144)",
        ),
        (
            CoreError::RenderTooLong(9),
            "cannot render 9 frames: too long for memory",
        ),
        (
            CoreError::UnknownObject {
                node: NodeId(3),
                kind: String::from("x~"),
            },
            "box 3: unknown box type \"x~\"",
        ),
        (
            CoreError::UnknownParam {
                node: NodeId(4),
                name: String::from("gain"),
            },
            "box 4: unknown parameter \"gain\"",
        ),
        (
            CoreError::ParamOutOfRange {
                node: NodeId(5),
                name: String::from("gain"),
                value: 1.5,
                minimum: -0.25_f64,
                maximum: 1.0_f64,
            },
            "box 5: parameter \"gain\": 1.5 is not within [-0.25, 1.0]",
        ),
        (
            CoreError::UnknownNode(NodeId(6)),
            "a cable refers to box 6, which does not exist",
        ),
        (
            CoreError::InvalidOutlet {
                node: NodeId(7),
                outlet: 3,
            },
            "box 7: no outlet 3",
        ),
        (
            CoreError::InvalidInlet {
                node: NodeId(8),
                inlet: 4,
            },
            "box 8: no inlet 4",
        ),
        (
            CoreError::Cycle(vec![NodeId(1), NodeId(9)]),
            "cycle without delay: boxes in it or after it: box 1, box 9",
        ),
    ];
    for (error, message) in cases {
        assert_eq!(error.to_string(), message, "message of {error:?}");
        assert!(!repeats_a_word(message), "{message} repeats a word");
    }
}

/// Whether a word appears twice in a row within a list item, ignoring numbers
/// between them ("box: box 2", "box 1 box 9"): such messages read badly. Items of
/// a comma-separated list ("box 1, box 9") may repeat their noun.
fn repeats_a_word(message: &str) -> bool {
    message.split(", ").any(|item| {
        let words: Vec<&str> = item
            .split(|character: char| !character.is_alphanumeric())
            .filter(|word| {
                !word.is_empty() && !word.chars().all(|character| character.is_ascii_digit())
            })
            .collect();
        words.windows(2).any(|pair| pair[0] == pair[1])
    })
}

#[test]
fn spots_repeated_words() {
    assert!(repeats_a_word("duplicate box: box 2"), "colon between");
    assert!(repeats_a_word("involving box 1 box 9"), "number between");
    assert!(!repeats_a_word("box 1, outlet 2"), "different words");
    assert!(!repeats_a_word("involving box 1, box 9"), "a list");
}

#[test]
fn is_a_standard_error() {
    let error: &dyn core::error::Error = &CoreError::DuplicateNode(NodeId(1));
    assert!(error.source().is_none(), "no underlying cause");
}

#[test]
fn describes_the_errors_of_hosts_and_modes() {
    let cases = [
        (
            CoreError::TooManyChannels {
                direction: crate::Direction::Inputs,
                count: 513,
            },
            "too many input channels: 513 (at most 512)",
        ),
        (
            CoreError::OfflineOnly {
                node: NodeId(6),
                kind: String::from("analyze~"),
            },
            "box 6: \"analyze~\" runs offline only, not in a live plan",
        ),
        (
            CoreError::Unbuilt {
                node: NodeId(4),
                kind: String::from("osc~"),
                frames: 128,
            },
            "box 4: \"osc~\" is not built for engine blocks of 128 frames",
        ),
        (
            CoreError::StageChannels { plan: 2, stage: 3 },
            "the output stage has 3 channels, the plan has 2",
        ),
        (
            CoreError::PastTheEnd { length: 4_800 },
            "cannot render past the end of a render of 4800 frames",
        ),
        (
            CoreError::UsedStage,
            "the output stage applied a block already: a render starts with a new one",
        ),
    ];
    for (error, message) in cases {
        assert_eq!(error.to_string(), message, "message of {error:?}");
        assert!(!repeats_a_word(message), "{message} repeats a word");
    }
}
