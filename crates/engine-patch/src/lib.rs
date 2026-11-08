//! Reads patches in the JSON patch format (see `spec/patch-format.md`).
//!
//! This is the boundary where untrusted text becomes a typed [`Patch`]: the structure
//! is validated here (size, format version, field names and types, unique nodes, cables
//! and parameter names); box types, parameters
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

use engine_core::{Connection, CoreError, Node, NodeId, Patch};
use serde::Deserialize;
use serde::de::{self, Deserializer, MapAccess, Visitor};

/// The only format version this crate reads.
pub const FORMAT: u32 = 1;

/// Largest patch document read, in bytes (16 MiB).
///
/// Checked before parsing, so that an oversized document is refused without being
/// parsed. Readers of files bound what they load to this size too (`engine-cli` reads at
/// most one byte past it).
pub const MAX_PATCH_BYTES: usize = 16 * 1024 * 1024;

/// Why a patch document was rejected.
#[derive(Debug, Clone, PartialEq)]
pub enum PatchError {
    /// The document is longer than [`MAX_PATCH_BYTES`] (its length in bytes).
    TooLarge(usize),
    /// The text is not valid JSON, or does not have the expected structure.
    Invalid(String),
    /// The document declares a format version this crate does not read.
    UnsupportedFormat(u32),
    /// The document is well formed but is not a valid patch (duplicate box or cable,
    /// too many boxes or cables).
    Patch(CoreError),
}

impl fmt::Display for PatchError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::TooLarge(bytes) => write!(
                f,
                "patch too large: {bytes} bytes (at most {MAX_PATCH_BYTES})"
            ),
            Self::Invalid(reason) => write!(f, "invalid patch: {reason}"),
            Self::UnsupportedFormat(version) => {
                write!(f, "unsupported patch format {version} (expected {FORMAT})")
            }
            Self::Patch(error) => write!(f, "invalid patch: {error}"),
        }
    }
}

impl core::error::Error for PatchError {
    fn source(&self) -> Option<&(dyn core::error::Error + 'static)> {
        match self {
            Self::Patch(error) => Some(error),
            Self::TooLarge(_) | Self::Invalid(_) | Self::UnsupportedFormat(_) => None,
        }
    }
}

/// Parameter values by name, refusing a name given twice (a JSON object may repeat a
/// key; which value wins would be a parser detail, so the document is refused).
#[derive(Debug, Default)]
struct Params(BTreeMap<String, f32>);

impl<'de> Deserialize<'de> for Params {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        deserializer.deserialize_map(ParamsVisitor)
    }
}

/// Reads [`Params`], one entry at a time.
struct ParamsVisitor;

impl<'de> Visitor<'de> for ParamsVisitor {
    type Value = Params;

    fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "an object of parameter values")
    }

    fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Params, A::Error> {
        let mut params = BTreeMap::new();
        while let Some((name, value)) = map.next_entry::<String, f32>()? {
            if params.contains_key(&name) {
                return Err(de::Error::custom(alloc::format!(
                    "parameter `{name}` is given twice"
                )));
            }
            let _: Option<f32> = params.insert(name, value);
        }
        Ok(Params(params))
    }
}

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
    params: Params,
}

/// A cable, as written: `[node id, port index]` at each end.
///
/// Port indices are `u16` (0 to 65535, as the format states), not `usize`: a pointer-wide
/// type would accept on a 64-bit target what a 32-bit one (WebAssembly) refuses, so the
/// same document would fail differently on each.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ConnectionDocument {
    /// Source node and outlet.
    from: (u32, u16),
    /// Destination node and inlet.
    to: (u32, u16),
}

/// Reads a patch document.
///
/// # Errors
///
/// Returns [`PatchError::TooLarge`] beyond [`MAX_PATCH_BYTES`], [`PatchError::Invalid`]
/// when the text is not JSON or has unknown, missing, mistyped or repeated fields,
/// [`PatchError::UnsupportedFormat`] for another format version, and
/// [`PatchError::Patch`] for a duplicate node or cable or too many of them.
pub fn parse_patch(json: &str) -> Result<Patch, PatchError> {
    if json.len() > MAX_PATCH_BYTES {
        return Err(PatchError::TooLarge(json.len()));
    }
    let document: Document =
        serde_json::from_str(json).map_err(|error| PatchError::Invalid(error.to_string()))?;
    if document.format != FORMAT {
        return Err(PatchError::UnsupportedFormat(document.format));
    }
    let nodes = document
        .nodes
        .into_iter()
        .map(|node| Node {
            id: NodeId(node.id),
            kind: node.kind,
            params: node.params.0,
        })
        .collect();
    let connections = document
        .connections
        .into_iter()
        .map(|connection| Connection {
            from: NodeId(connection.from.0),
            outlet: usize::from(connection.from.1),
            to: NodeId(connection.to.0),
            inlet: usize::from(connection.to.1),
        })
        .collect();
    Patch::new(nodes, connections).map_err(PatchError::Patch)
}

#[cfg(test)]
#[path = "lib_tests.rs"]
mod tests;
