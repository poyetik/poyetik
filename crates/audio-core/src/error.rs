//! Errors raised while validating a sample rate or a patch, building a plan, or
//! rendering offline.

#![cfg_attr(not(test), forbid(clippy::arithmetic_side_effects))]

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
        /// The parameter's lowest accepted value.
        minimum: f32,
        /// The parameter's highest accepted value.
        maximum: f32,
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
    /// More channels one way than any engine applies ([`crate::MAX_CHANNELS`]).
    TooManyChannels {
        /// Which way.
        direction: crate::Direction,
        /// The channels asked.
        count: u16,
    },
    /// A box whose box type the plan's instance type cannot create in its engine block in
    /// this build: a box type through its virtual table, in a build naming another engine
    /// block (`crate::built_for`).
    Unbuilt {
        /// The box.
        node: NodeId,
        /// Its box type.
        kind: String,
        /// The engine block, in frames.
        frames: usize,
    },
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
            Self::ParamOutOfRange {
                node,
                name,
                value,
                minimum,
                maximum,
            } => write!(
                f,
                "{node}: parameter \"{name}\": {value} is outside [{minimum}, {maximum}]"
            ),
            Self::UnknownNode(node) => {
                write!(f, "a cable refers to {node}, which does not exist")
            }
            Self::InvalidOutlet { node, outlet } => write!(f, "{node}: no outlet {outlet}"),
            Self::InvalidInlet { node, inlet } => write!(f, "{node}: no inlet {inlet}"),
            Self::TooManyChannels { direction, count } => write!(
                f,
                "too many {direction} channels: {count} (at most {})",
                crate::MAX_CHANNELS
            ),
            Self::Unbuilt { node, kind, frames } => write!(
                f,
                "{node}: \"{kind}\" is not built for engine blocks of {frames} frames"
            ),
            Self::Cycle(nodes) => {
                write!(f, "cycle without delay: boxes in it or after it: ")?;
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
#[path = "error_tests.rs"]
mod tests;
