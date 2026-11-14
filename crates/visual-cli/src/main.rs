//! `visual-cli`: see the `visual-cli` library.

#![forbid(unsafe_code)]

use std::process::ExitCode;

/// Runs the requested command on the real file system.
///
/// Exit codes: 0 on success, 1 when the command failed, 2 for a usage error; every error
/// is one `error:` line on stderr. `visual-cli help` prints the usage on stdout. A closed
/// stdout is not an error.
fn main() -> ExitCode {
    let outcome = visual_cli::arguments(std::env::args_os().skip(1).collect()).and_then(|args| {
        visual_cli::run(
            &args,
            &mut visual_cli::Disk,
            &mut visual_cli::BestEffort(std::io::stdout()),
        )
    });
    let error = match outcome {
        Ok(()) => return ExitCode::SUCCESS,
        Err(error) => error,
    };
    // Nothing more can be done if stderr itself is broken: the exit code still tells.
    drop(std::io::Write::write_all(
        &mut std::io::stderr(),
        format!("{}\n", error.line()).as_bytes(),
    ));
    ExitCode::from(error.exit_code())
}
