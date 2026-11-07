//! A patch: boxes and the cables between them, as data.

use alloc::collections::BTreeMap;
use alloc::string::String;
use alloc::vec::Vec;
use core::fmt;

/// Stable identifier of a node in a patch. Also breaks ties when ordering execution.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct NodeId(pub u32);

impl fmt::Display for NodeId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "node {}", self.0)
    }
}

/// A box placed in a patch.
#[derive(Debug, Clone, PartialEq)]
pub struct Node {
    /// Stable identifier.
    pub id: NodeId,
    /// Box type, as named in the registry (for example `osc~`).
    pub kind: String,
    /// Parameter values set by the patch; unset parameters use their default.
    pub params: BTreeMap<String, f32>,
}

/// A cable from an outlet to an inlet.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Connection {
    /// Source node.
    pub from: NodeId,
    /// Source outlet index.
    pub outlet: usize,
    /// Destination node.
    pub to: NodeId,
    /// Destination inlet index.
    pub inlet: usize,
}

/// A patch, as data. Validated when a [`crate::Plan`] is built from it.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Patch {
    /// Nodes, in any order.
    pub nodes: Vec<Node>,
    /// Cables, in any order.
    pub connections: Vec<Connection>,
}
