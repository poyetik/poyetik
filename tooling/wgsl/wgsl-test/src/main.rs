//! `wgsl-test`: validates WGSL files with naga (see the library's `cli` module).

#![forbid(unsafe_code)]
#![expect(
    clippy::multiple_crate_versions,
    reason = "naga's own tree holds two versions of syn (thiserror's derive and half's zerocopy \
              derive), which this crate cannot choose; the pair is a reviewed exception of \
              deny.toml"
)]

use std::process::ExitCode;

use wgsl_test::cli;

/// Runs the requested command on the files it names, relative to the current directory.
///
/// Exit codes: 0 on success, 1 when the work failed, 2 for a usage error; every error is
/// one `error:` line on stderr. `wgsl-test help` prints the usage on stdout. A closed
/// stdout is not an error.
fn main() -> ExitCode {
    let mut read =
        |path: &str| std::fs::File::open(path).and_then(|mut file| cli::read_source(&mut file));
    let ran = cli::arguments(std::env::args_os().skip(1).collect())
        .and_then(|args| cli::run(&args, &mut read, &mut std::io::stdout()));
    let Err(error) = ran else {
        return ExitCode::SUCCESS;
    };
    // Nothing more can be done if stderr itself is broken: the exit code still tells.
    drop(std::io::Write::write_all(
        &mut std::io::stderr(),
        format!("error: {}\n", error.line()).as_bytes(),
    ));
    ExitCode::from(error.code())
}
