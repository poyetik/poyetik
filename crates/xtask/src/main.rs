//! `cargo xtask`: see the `xtask` library.

#![forbid(unsafe_code)]

use std::path::PathBuf;
use std::process::ExitCode;

/// Runs the requested task from the repository root.
///
/// Exit codes: 0 on success, 1 when a check or a command failed, 2 for a usage error.
fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let mut runner = xtask::SystemRunner { root };
    ExitCode::from(xtask::cli(
        &args,
        &mut runner,
        &mut xtask::BestEffort(std::io::stdout()),
        &mut std::io::stderr(),
    ))
}
