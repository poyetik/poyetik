//! `engine-cli`: see the `engine-cli` library.

#![forbid(unsafe_code)]

use std::process::ExitCode;

/// Runs the requested command on the real file system.
///
/// Exit codes: 0 on success, 1 when the command failed (one `error:` line on stderr),
/// 2 for a usage error (the usage on stderr). A closed stdout is not an error.
fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let outcome = engine_cli::run(
        &args,
        &mut engine_cli::Disk,
        &mut engine_cli::BestEffort(std::io::stdout()),
    );
    let (text, code) = match outcome {
        Ok(()) => return ExitCode::SUCCESS,
        Err(engine_cli::CliError::Usage) => (engine_cli::USAGE.to_owned(), 2_u8),
        Err(engine_cli::CliError::Failed(message)) => (format!("error: {message}"), 1_u8),
    };
    // Nothing more can be done if stderr itself is broken: the exit code still tells.
    drop(std::io::Write::write_all(
        &mut std::io::stderr(),
        format!("{text}\n").as_bytes(),
    ));
    ExitCode::from(code)
}
