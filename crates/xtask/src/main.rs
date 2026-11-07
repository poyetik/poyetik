//! `cargo xtask`: see the `xtask` library.

#![forbid(unsafe_code)]

use std::path::PathBuf;
use std::process::ExitCode;

/// Runs the requested task from the repository root.
fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let mut runner = xtask::SystemRunner { root };
    if xtask::cli(
        &args,
        &mut runner,
        &mut std::io::stdout(),
        &mut std::io::stderr(),
    ) {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    }
}
