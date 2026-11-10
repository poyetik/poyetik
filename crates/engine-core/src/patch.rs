//! A patch: boxes and the cables between them, as data.

#![cfg_attr(not(test), forbid(clippy::arithmetic_side_effects))]

use alloc::collections::{BTreeMap, BTreeSet};
use alloc::string::String;
use alloc::vec::Vec;
use core::fmt;

use crate::CoreError;

/// Stable identifier of a box in a patch. Also breaks ties when ordering execution.
///
/// Shown as "box N": messages use the project's word for what users place in a patch
/// (`GLOSSARY.md`), whatever the code calls it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct NodeId(pub u32);

impl fmt::Display for NodeId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "box {}", self.0)
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
    /// Source box.
    pub from: NodeId,
    /// Source outlet index.
    pub outlet: usize,
    /// Destination box.
    pub to: NodeId,
    /// Destination inlet index.
    pub inlet: usize,
}

impl fmt::Display for Connection {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{} outlet {} to {} inlet {}",
            self.from, self.outlet, self.to, self.inlet
        )
    }
}

/// A patch, as data: boxes with unique ids and cables without duplicates, within the
/// size limits. Built only by [`Patch::new`], so these hold for every `Patch`.
///
/// Box types, parameters, ports and cycles are validated against a registry when a
/// [`crate::Plan`] is built from it.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Patch {
    /// Boxes, by id.
    nodes: BTreeMap<NodeId, Node>,
    /// Cables, in their canonical order (source, outlet, destination, inlet): the order
    /// in which the cables into an inlet are summed.
    connections: BTreeSet<Connection>,
}

impl Patch {
    /// Largest number of boxes a patch may have.
    pub const MAX_NODES: usize = 65_536;

    /// Largest number of cables a patch may have.
    pub const MAX_CONNECTIONS: usize = 262_144;

    /// Builds a patch from boxes and cables given in any order.
    ///
    /// # Errors
    ///
    /// Returns [`CoreError::TooManyNodes`] or [`CoreError::TooManyConnections`] beyond
    /// the limits, [`CoreError::DuplicateNode`] when two boxes share an id, and
    /// [`CoreError::DuplicateConnection`] when a cable is listed twice.
    pub fn new(nodes: Vec<Node>, connections: Vec<Connection>) -> Result<Self, CoreError> {
        if nodes.len() > Self::MAX_NODES {
            return Err(CoreError::TooManyNodes(nodes.len()));
        }
        if connections.len() > Self::MAX_CONNECTIONS {
            return Err(CoreError::TooManyConnections(connections.len()));
        }
        let mut by_id = BTreeMap::new();
        for node in nodes {
            let id = node.id;
            if by_id.insert(id, node).is_some() {
                return Err(CoreError::DuplicateNode(id));
            }
        }
        let mut cables = BTreeSet::new();
        for connection in connections {
            if !cables.insert(connection) {
                return Err(CoreError::DuplicateConnection(connection));
            }
        }
        Ok(Self {
            nodes: by_id,
            connections: cables,
        })
    }

    /// The boxes, by increasing id.
    pub fn nodes(&self) -> impl Iterator<Item = &Node> {
        self.nodes.values()
    }

    /// The cables, in their canonical order.
    pub fn connections(&self) -> impl Iterator<Item = &Connection> {
        self.connections.iter()
    }
}
