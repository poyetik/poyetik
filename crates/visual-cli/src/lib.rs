//! `visual-cli`: the offline renderer of the visual library.
//!
//! Renders the visual conformance cases (`conformance/visual/cases/<name>.json`) with the
//! exact CPU reference renderer, hashes each sequence of images (`spec/image-hash.md`), and
//! checks the hashes against `conformance/visual/references.json`, or rewrites them.
//!
//! Standalone, like the rest of the visual library: it depends on no audio crate.

#![forbid(unsafe_code)]

extern crate alloc;

mod case;
mod cli;
mod safe;

pub use case::{Case, MAX_CASE_BYTES, MAX_IMAGES, SquareCase, parse_case, render_case};
pub use cli::{CliError, USAGE, arguments, run};
pub use safe::{MAX_MESSAGE_CHARS, safe_message};
