//! Errors raised while validating a patch or building a plan.

use alloc::string::String;
use alloc::vec::Vec;
use core::fmt;

use crate::NodeId;

/// Why a sample rate, a patch or a plan was rejected.
#[derive(Debug, Clone, PartialEq)]
pub enum CoreError {
    /// The sample rate is outside the supported range.
    InvalidSampleRate(u32),
    /// Two nodes share the same id.
    DuplicateNode(NodeId),
    /// A node uses a box type that is not in the registry.
    UnknownObject {
        /// The node.
        node: NodeId,
        /// The unknown box type.
        kind: String,
    },
    /// A node sets a parameter its box type does not have.
    UnknownParam {
        /// The node.
        node: NodeId,
        /// The unknown parameter.
        name: String,
    },
    /// A parameter value is outside its range (or not a number).
    ParamOutOfRange {
        /// The node.
        node: NodeId,
        /// The parameter.
        name: String,
        /// The rejected value.
        value: f32,
    },
    /// A connection refers to a node that does not exist.
    UnknownNode(NodeId),
    /// A connection starts from an outlet the node does not have.
    InvalidOutlet {
        /// The node.
        node: NodeId,
        /// The outlet index.
        outlet: usize,
    },
    /// A connection ends on an inlet the node does not have.
    InvalidInlet {
        /// The node.
        node: NodeId,
        /// The inlet index.
        inlet: usize,
    },
    /// The patch contains a cycle without delay; these nodes are part of it or depend on it.
    Cycle(Vec<NodeId>),
}

impl fmt::Display for CoreError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidSampleRate(hz) => write!(f, "invalid sample rate: {hz} Hz"),
            Self::DuplicateNode(node) => write!(f, "duplicate node: {node}"),
            Self::UnknownObject { node, kind } => write!(f, "{node}: unknown box type \"{kind}\""),
            Self::UnknownParam { node, name } => write!(f, "{node}: unknown parameter \"{name}\""),
            Self::ParamOutOfRange { node, name, value } => {
                write!(f, "{node}: parameter \"{name}\" out of range: {value}")
            }
            Self::UnknownNode(node) => write!(f, "unknown node: {node}"),
            Self::InvalidOutlet { node, outlet } => write!(f, "{node}: no outlet {outlet}"),
            Self::InvalidInlet { node, inlet } => write!(f, "{node}: no inlet {inlet}"),
            Self::Cycle(nodes) => {
                write!(f, "cycle without delay involving")?;
                for node in nodes {
                    write!(f, " {node}")?;
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

    use super::CoreError;
    use crate::NodeId;

    #[test]
    fn describes_every_error() {
        let cases = [
            (CoreError::InvalidSampleRate(5), "invalid sample rate: 5 Hz"),
            (
                CoreError::DuplicateNode(NodeId(2)),
                "duplicate node: node 2",
            ),
            (
                CoreError::UnknownObject {
                    node: NodeId(3),
                    kind: String::from("x~"),
                },
                "node 3: unknown box type \"x~\"",
            ),
            (
                CoreError::UnknownParam {
                    node: NodeId(4),
                    name: String::from("gain"),
                },
                "node 4: unknown parameter \"gain\"",
            ),
            (
                CoreError::ParamOutOfRange {
                    node: NodeId(5),
                    name: String::from("gain"),
                    value: 1.5,
                },
                "node 5: parameter \"gain\" out of range: 1.5",
            ),
            (CoreError::UnknownNode(NodeId(6)), "unknown node: node 6"),
            (
                CoreError::InvalidOutlet {
                    node: NodeId(7),
                    outlet: 3,
                },
                "node 7: no outlet 3",
            ),
            (
                CoreError::InvalidInlet {
                    node: NodeId(8),
                    inlet: 4,
                },
                "node 8: no inlet 4",
            ),
            (
                CoreError::Cycle(vec![NodeId(1), NodeId(9)]),
                "cycle without delay involving node 1 node 9",
            ),
        ];
        for (error, message) in cases {
            assert_eq!(error.to_string(), message, "message of {error:?}");
        }
    }

    #[test]
    fn is_a_standard_error() {
        let error: &dyn core::error::Error = &CoreError::DuplicateNode(NodeId(1));
        assert!(error.source().is_none(), "no underlying cause");
    }
}
