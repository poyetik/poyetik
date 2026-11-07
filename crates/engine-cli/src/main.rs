//! `engine-cli`: see the `engine-cli` library.

#![forbid(unsafe_code)]

use std::process::ExitCode;

/// Runs the requested command on the real file system.
fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match engine_cli::run(&args, &mut engine_cli::Disk, &mut std::io::stdout()) {
        Ok(()) => ExitCode::SUCCESS,
        Err(message) => {
            // Nothing more can be done if stderr itself is broken: the exit code still fails.
            drop(std::io::Write::write_all(
                &mut std::io::stderr(),
                format!("{message}\n").as_bytes(),
            ));
            ExitCode::FAILURE
        }
    }
}
