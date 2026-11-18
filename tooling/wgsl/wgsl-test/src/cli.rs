//! The command line of `wgsl-test`: `validate <file>...` and `help`, as a tested function
//! the binary only wires to the process.
//!
//! Exit codes: 0 on success, 1 when a file is not a valid WGSL module or cannot be read, 2
//! for a usage error. Each problem is one line on stdout, `<file>:<line>:<column>: <stage>:
//! <message>`, then one `error:` line sums them up on stderr; a closed stdout is not an
//! error and changes no exit code.

use std::ffi::OsString;
use std::io::{self, Read, Write};

use crate::{safe_message, validate};

#[cfg(test)]
#[path = "cli_tests.rs"]
mod tests;

/// The usage, which `wgsl-test help` prints on stdout.
pub const USAGE: &str = "usage: wgsl-test <command>

commands:
  validate <file>...  check that each file is a valid WGSL module, as wgpu's naga parses
                      and validates it under naga's default capabilities; each problem is
                      one line on stdout, <file>:<line>:<column>: <stage>: <message>
  help                print this usage
";

/// Why a command did not succeed: its exit code and the one line said on stderr after
/// `error: `.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CliError {
    /// 1 when the work failed, 2 for a usage error.
    code: u8,
    /// What went wrong, on one line.
    line: String,
}

impl CliError {
    /// A usage error: `reason`, then where the usage is.
    fn usage(reason: &str) -> Self {
        Self {
            code: 2,
            line: format!("{reason} (see: wgsl-test help)"),
        }
    }

    /// The exit code: 1 when the work failed, 2 for a usage error.
    #[must_use]
    pub const fn code(&self) -> u8 {
        self.code
    }

    /// What went wrong, on one line, without the `error: ` the binary writes before it.
    #[must_use]
    pub fn line(&self) -> &str {
        &self.line
    }
}

/// The most bytes of a source read: naga's positions are 32-bit offsets, so a longer source
/// is refused before it is held whole.
pub const MAX_SOURCE_BYTES: u64 = 4_294_967_295;

/// Turns the raw command-line arguments (without the program's name) into text.
///
/// # Errors
///
/// Returns a usage error naming the first argument that is not valid UTF-8, rather than
/// reading another file than the one named.
pub fn arguments(raw: Vec<OsString>) -> Result<Vec<String>, CliError> {
    raw.into_iter()
        .enumerate()
        .map(|(position, argument)| {
            argument.into_string().map_err(|_| {
                CliError::usage(&format!(
                    "argument {} is not valid UTF-8",
                    position.saturating_add(1)
                ))
            })
        })
        .collect()
}

/// Reads a source from `file`, at most [`MAX_SOURCE_BYTES`] bytes.
///
/// # Errors
///
/// Returns why `file` cannot be read, is not UTF-8, or holds more bytes than naga
/// addresses, refused once one byte past the bound is read, so that no longer source is
/// held whole.
pub fn read_source(file: &mut dyn Read) -> io::Result<String> {
    read_at_most(file, MAX_SOURCE_BYTES)
}

/// Reads `file` whole as UTF-8 when it holds at most `limit` bytes, reading at most one
/// byte past it otherwise.
fn read_at_most(file: &mut dyn Read, limit: u64) -> io::Result<String> {
    let mut text = String::new();
    let _: usize = file
        .take(limit.saturating_add(1))
        .read_to_string(&mut text)?;
    if u64::try_from(text.len()).map_or(true, |bytes| bytes > limit) {
        return Err(io::Error::new(
            io::ErrorKind::FileTooLarge,
            format!("larger than {limit} bytes, more than naga's 32-bit positions address"),
        ));
    }
    Ok(text)
}

/// Writes `line` and a newline to `out`, a closed stdout being no error: what is not
/// shown changes neither the work nor its exit code.
///
/// # Errors
///
/// Returns a failure of the work when `out` refuses the line for any other reason.
fn say(out: &mut dyn Write, line: &str) -> Result<(), CliError> {
    match writeln!(out, "{line}") {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == io::ErrorKind::BrokenPipe => Ok(()),
        Err(error) => Err(CliError {
            code: 1,
            line: format!("cannot write to stdout: {error}"),
        }),
    }
}

/// Runs the command `args` (the arguments after the program's name), reading each file
/// named through `read` and writing on `out`.
///
/// # Errors
///
/// Returns a usage error for an unknown command, a missing file list or an argument after
/// `help`; a failure of the work when a file cannot be read or is not a valid WGSL module
/// (each said on `out` first), or when `out` refuses a line for another reason than being
/// closed.
pub fn run(
    args: &[String],
    read: &mut dyn FnMut(&str) -> io::Result<String>,
    out: &mut dyn Write,
) -> Result<(), CliError> {
    match args {
        [command] if command == "help" => {
            for line in USAGE.lines() {
                say(out, line)?;
            }
            Ok(())
        }
        [command, files @ ..] if command == "validate" => {
            if files.is_empty() {
                return Err(CliError::usage("validate names no file"));
            }
            let mut refused: usize = 0;
            for file in files {
                let problem = match read(file) {
                    Ok(source) => validate(&source).err().map(|invalid| invalid.of_file(file)),
                    Err(error) => Some(format!(
                        "{}: cannot read: {}",
                        safe_message(file),
                        safe_message(&error.to_string())
                    )),
                };
                if let Some(problem) = problem {
                    refused = refused.saturating_add(1);
                    say(out, &problem)?;
                }
            }
            if refused == 0 {
                Ok(())
            } else {
                Err(CliError {
                    code: 1,
                    line: format!("{refused} of {} files refused, listed above", files.len()),
                })
            }
        }
        [] => Err(CliError::usage("no command")),
        [command, ..] if command == "help" => Err(CliError::usage("help takes no argument")),
        [command, ..] => Err(CliError::usage(&format!(
            "unknown command \"{}\"",
            safe_message(command)
        ))),
    }
}
