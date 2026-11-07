//! Reads patches in the JSON patch format (see `spec/patch-format.md`).
//!
//! This is the boundary where untrusted text becomes a typed [`Patch`]: the structure
//! is validated here (format version, field names and types); box types, parameters
//! and ports are then validated against the registry by [`engine_core::Plan::build`].
//!
//! `no_std` and free of `unsafe`, like the core, so that every target (native, WASM,
//! embedded) reads patches with exactly the same code.

#![no_std]
#![forbid(unsafe_code)]

extern crate alloc;

use alloc::collections::BTreeMap;
use alloc::string::{String, ToString};
use alloc::vec::Vec;
use core::fmt;

use engine_core::{Connection, Node, NodeId, Patch};
use serde::Deserialize;

/// The only format version this crate reads.
pub const FORMAT: u32 = 1;

/// Why a patch document was rejected.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PatchError {
    /// The text is not valid JSON, or does not have the expected structure.
    Invalid(String),
    /// The document declares a format version this crate does not read.
    UnsupportedFormat(u32),
}

impl fmt::Display for PatchError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Invalid(reason) => write!(f, "invalid patch: {reason}"),
            Self::UnsupportedFormat(version) => {
                write!(f, "unsupported patch format {version} (expected {FORMAT})")
            }
        }
    }
}

impl core::error::Error for PatchError {}

/// A patch document, as written.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Document {
    /// Format version.
    format: u32,
    /// Boxes.
    nodes: Vec<NodeDocument>,
    /// Cables.
    connections: Vec<ConnectionDocument>,
}

/// A box, as written.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct NodeDocument {
    /// Identifier.
    id: u32,
    /// Box type.
    #[serde(rename = "box")]
    kind: String,
    /// Parameter values by name.
    #[serde(default)]
    params: BTreeMap<String, f32>,
}

/// A cable, as written: `[node id, port index]` at each end.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ConnectionDocument {
    /// Source node and outlet.
    from: (u32, usize),
    /// Destination node and inlet.
    to: (u32, usize),
}

/// Reads a patch document.
///
/// # Errors
///
/// Returns [`PatchError::Invalid`] when the text is not JSON or has unknown, missing or
/// mistyped fields, and [`PatchError::UnsupportedFormat`] for another format version.
pub fn parse_patch(json: &str) -> Result<Patch, PatchError> {
    let document: Document =
        serde_json::from_str(json).map_err(|error| PatchError::Invalid(error.to_string()))?;
    if document.format != FORMAT {
        return Err(PatchError::UnsupportedFormat(document.format));
    }
    Ok(Patch {
        nodes: document
            .nodes
            .into_iter()
            .map(|node| Node {
                id: NodeId(node.id),
                kind: node.kind,
                params: node.params,
            })
            .collect(),
        connections: document
            .connections
            .into_iter()
            .map(|c| Connection {
                from: NodeId(c.from.0),
                outlet: c.from.1,
                to: NodeId(c.to.0),
                inlet: c.to.1,
            })
            .collect(),
    })
}

#[cfg(test)]
#[path = "lib_tests.rs"]
mod tests;
