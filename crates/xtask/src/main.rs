//! `cargo xtask`: see the `xtask` library.

#![forbid(unsafe_code)]

use std::path::PathBuf;
use std::process::ExitCode;

/// Runs the requested task from the repository root.
///
/// The arguments are read as they are (`args_os`): one that is not UTF-8 is a usage error,
/// never a panic. Exit codes: 0 on success, 1 when a check or a command failed, 2 for a
/// usage error, each failure told in one `error:` line on stderr.
fn main() -> ExitCode {
    let args: Vec<std::ffi::OsString> = std::env::args_os().skip(1).collect();
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let mut runner = xtask::SystemRunner { root, env: &[] };
    ExitCode::from(xtask::cli(
        &args,
        &mut runner,
        &mut xtask::BestEffort(std::io::stdout()),
        &mut std::io::stderr(),
    ))
}
