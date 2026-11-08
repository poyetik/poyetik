//! Errors raised while validating a sample rate or a patch, building a plan, or
//! rendering offline.

use alloc::string::String;
use alloc::vec::Vec;
use core::fmt;

use crate::{Connection, NodeId, Patch, SampleRate};

/// Why a sample rate, a patch or a plan was rejected.
#[derive(Debug, Clone, PartialEq)]
pub enum CoreError {
    /// The sample rate is outside the supported range.
    InvalidSampleRate(u32),
    /// Two boxes share the same id.
    DuplicateNode(NodeId),
    /// The same cable is listed twice.
    DuplicateConnection(Connection),
    /// A patch has more boxes than [`Patch::MAX_NODES`].
    TooManyNodes(usize),
    /// A patch has more cables than [`Patch::MAX_CONNECTIONS`].
    TooManyConnections(usize),
    /// An offline render asks for more samples than memory can hold.
    RenderTooLong(usize),
    /// A box uses a box type that is not in the registry.
    UnknownObject {
        /// The box.
        node: NodeId,
        /// The unknown box type.
        kind: String,
    },
    /// A box sets a parameter its box type does not have.
    UnknownParam {
        /// The box.
        node: NodeId,
        /// The unknown parameter.
        name: String,
    },
    /// A parameter value is outside its range (or not a number).
    ParamOutOfRange {
        /// The box.
        node: NodeId,
        /// The parameter.
        name: String,
        /// The rejected value.
        value: f32,
    },
    /// A connection refers to a box that does not exist.
    UnknownNode(NodeId),
    /// A connection starts from an outlet the box does not have.
    InvalidOutlet {
        /// The box.
        node: NodeId,
        /// The outlet index.
        outlet: usize,
    },
    /// A connection ends on an inlet the box does not have.
    InvalidInlet {
        /// The box.
        node: NodeId,
        /// The inlet index.
        inlet: usize,
    },
    /// The patch contains a cycle without delay; these boxes are part of it or depend on it.
    Cycle(Vec<NodeId>),
}

impl fmt::Display for CoreError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidSampleRate(hz) => write!(
                f,
                "invalid sample rate: {hz} Hz (supported: {} to {} Hz)",
                SampleRate::MIN,
                SampleRate::MAX
            ),
            Self::DuplicateNode(node) => write!(f, "{node} is declared twice"),
            Self::DuplicateConnection(connection) => {
                write!(f, "the cable from {connection} is listed twice")
            }
            Self::TooManyNodes(count) => {
                write!(f, "too many boxes: {count} (at most {})", Patch::MAX_NODES)
            }
            Self::TooManyConnections(count) => write!(
                f,
                "too many cables: {count} (at most {})",
                Patch::MAX_CONNECTIONS
            ),
            Self::RenderTooLong(frames) => {
                write!(f, "cannot render {frames} frames: too long for memory")
            }
            Self::UnknownObject { node, kind } => write!(f, "{node}: unknown box type \"{kind}\""),
            Self::UnknownParam { node, name } => write!(f, "{node}: unknown parameter \"{name}\""),
            Self::ParamOutOfRange { node, name, value } => {
                write!(f, "{node}: parameter \"{name}\" out of range: {value}")
            }
            Self::UnknownNode(node) => {
                write!(f, "a cable refers to {node}, which does not exist")
            }
            Self::InvalidOutlet { node, outlet } => write!(f, "{node}: no outlet {outlet}"),
            Self::InvalidInlet { node, inlet } => write!(f, "{node}: no inlet {inlet}"),
            Self::Cycle(nodes) => {
                write!(f, "cycle without delay involving ")?;
                for (index, node) in nodes.iter().enumerate() {
                    if index > 0 {
                        write!(f, ", ")?;
                    }
                    write!(f, "{node}")?;
                }
                Ok(())
            }
        }
    }
}

impl core::error::Error for CoreError {}

#[cfg(test)]
mod tests {
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
                },
                "box 5: parameter \"gain\" out of range: 1.5",
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
                "cycle without delay involving box 1, box 9",
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
}
