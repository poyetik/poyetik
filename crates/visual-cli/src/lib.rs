//! `visual-cli`: the offline renderer of the visual library.
//!
//! Renders the visual conformance cases (`conformance/visual/cases/<name>.json`) with the
//! exact CPU reference renderer, hashes each sequence of images (`spec/image-hash.md`), and
//! checks the hashes against `conformance/visual/references.json`, or rewrites them, with the
//! display corpus's bytes (`conformance/visual/display.json`).
//!
//! Standalone, like the visual library: within the project it depends only on the visual
//! library's crates, which xtask's library gate checks (`CrateRole::VisualTool`).

#![forbid(unsafe_code)]
#![doc(test(attr(forbid(unsafe_code))))]

extern crate alloc;

mod case;
mod cli;
mod display;
mod files;
mod safe;
mod status;

pub use case::{
    CaseError, MAX_CASE_BYTES, MAX_IMAGES, MAX_RENDERED_PIXELS, PreparedCase, hash_case, parse_case,
};
pub use cli::{BestEffort, CliError, MAX_REFERENCES_BYTES, USAGE, arguments, run};
pub use display::{DISPLAY_FILE, display_corpus};
pub use files::{Disk, Files};
pub use safe::{MAX_MESSAGE_CHARS, safe_message};

#[cfg(test)]
#[path = "lib_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "host_modes_tests.rs"]
mod host_modes_tests;
