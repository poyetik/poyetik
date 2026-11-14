//! `visual-cli`: see the `visual-cli` library.

#![forbid(unsafe_code)]

use std::process::ExitCode;

/// Runs the requested command. Exit codes: 0 on success, 1 when the command failed, 2 for
/// a usage error; every error is one `error:` line on stderr. A closed stdout is not an
/// error.
fn main() -> ExitCode {
    let outcome = visual_cli::arguments(std::env::args_os().skip(1).collect())
        .and_then(|args| visual_cli::run(&args, &mut std::io::stdout()));
    let error = match outcome {
        Ok(()) => return ExitCode::SUCCESS,
        Err(error) => error,
    };
    drop(std::io::Write::write_all(
        &mut std::io::stderr(),
        format!("{}\n", error.line()).as_bytes(),
    ));
    ExitCode::from(error.exit_code())
}
