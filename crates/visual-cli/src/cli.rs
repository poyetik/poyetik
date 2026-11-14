//! The `visual-cli` commands.

use alloc::collections::BTreeMap;
use std::ffi::OsString;
use std::fs;
use std::io::{Read, Write};
use std::path::Path;

use crate::{MAX_CASE_BYTES, parse_case, render_case, safe_message};

/// Usage of the `visual-cli` command, printed on stdout by `visual-cli help`.
pub const USAGE: &str = "\
usage: visual-cli <command>
  render <case.json>  render a visual case, print its image hash
  check <dir>         compare every case of <dir>/cases with <dir>/references.json
  bless <dir>         rewrite <dir>/references.json from the current renders
  help                print this help";

/// The largest references file read, in bytes.
const MAX_REFERENCES_BYTES: u64 = 1024 * 1024;

/// Why a command did not succeed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CliError {
    /// The arguments do not form a command: exit code 2.
    Usage(String),
    /// The command ran and failed: exit code 1.
    Failed(String),
}

impl CliError {
    /// The error as the one line the tool prints on stderr, without its newline, made safe
    /// by [`safe_message`].
    #[must_use]
    pub fn line(&self) -> String {
        format!("error: {}", safe_message(&self.to_string()))
    }

    /// The exit code of the process for this error: 2 for a usage error, 1 for a failure.
    #[must_use]
    pub const fn exit_code(&self) -> u8 {
        match self {
            Self::Usage(_) => 2,
            Self::Failed(_) => 1,
        }
    }
}

impl core::fmt::Display for CliError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Usage(reason) => write!(f, "{reason} (see: visual-cli help)"),
            Self::Failed(message) => f.write_str(message),
        }
    }
}

/// Turns the raw command-line arguments (without the program name) into text.
///
/// # Errors
///
/// Returns [`CliError::Usage`] naming the first argument that is not valid UTF-8.
pub fn arguments(raw: Vec<OsString>) -> Result<Vec<String>, CliError> {
    raw.into_iter()
        .enumerate()
        .map(|(position, argument)| {
            argument.into_string().map_err(|_| {
                CliError::Usage(format!(
                    "argument {} is not valid UTF-8",
                    position.saturating_add(1)
                ))
            })
        })
        .collect()
}

/// Reads at most `limit` bytes of the file at `path` as UTF-8 text.
fn read_bounded(path: &Path, limit: u64) -> Result<String, CliError> {
    let name = path.display();
    let file = fs::File::open(path)
        .map_err(|error| CliError::Failed(format!("cannot read {name}: {error}")))?;
    let mut bytes = Vec::new();
    let _read = file
        .take(limit.saturating_add(1))
        .read_to_end(&mut bytes)
        .map_err(|error| CliError::Failed(format!("cannot read {name}: {error}")))?;
    if u64::try_from(bytes.len()).unwrap_or(u64::MAX) > limit {
        return Err(CliError::Failed(format!(
            "{name} is longer than {limit} bytes"
        )));
    }
    String::from_utf8(bytes).map_err(|_| CliError::Failed(format!("{name} is not UTF-8")))
}

/// Renders the case file at `path` and returns its hash as text.
fn hash_of(path: &Path) -> Result<String, CliError> {
    let limit = u64::try_from(MAX_CASE_BYTES).unwrap_or(u64::MAX);
    let text = read_bounded(path, limit)?;
    let case = parse_case(&text)
        .map_err(|error| CliError::Failed(format!("{}: {error}", path.display())))?;
    let hash = render_case(&case)
        .map_err(|error| CliError::Failed(format!("{}: {error}", path.display())))?;
    Ok(hash.to_string())
}

/// The hash of every case of `<dir>/cases`, by case name (the file name without `.json`).
fn render_all(dir: &Path) -> Result<BTreeMap<String, String>, CliError> {
    let cases = dir.join("cases");
    let entries = fs::read_dir(&cases)
        .map_err(|error| CliError::Failed(format!("cannot list {}: {error}", cases.display())))?;
    let mut hashes = BTreeMap::new();
    for entry in entries {
        let path = entry
            .map_err(|error| CliError::Failed(format!("cannot list {}: {error}", cases.display())))?
            .path();
        let name = path
            .file_name()
            .and_then(|file| file.to_str())
            .and_then(|file| file.strip_suffix(".json"))
            .ok_or_else(|| {
                CliError::Failed(format!(
                    "{} is not a case file (<name>.json)",
                    path.display()
                ))
            })?
            .to_owned();
        let _earlier = hashes.insert(name, hash_of(&path)?);
    }
    if hashes.is_empty() {
        return Err(CliError::Failed(format!("no case in {}", cases.display())));
    }
    Ok(hashes)
}

/// Writes one line on `out`; a closed output is not an error.
fn say(out: &mut dyn Write, line: &str) {
    drop(writeln!(out, "{}", safe_message(line)));
}

/// Compares the renders of `dir` with its references.
fn check(dir: &Path, out: &mut dyn Write) -> Result<(), CliError> {
    let hashes = render_all(dir)?;
    let file = dir.join("references.json");
    let text = read_bounded(&file, MAX_REFERENCES_BYTES)?;
    let references: BTreeMap<String, String> = serde_json::from_str(&text)
        .map_err(|error| CliError::Failed(format!("{}: {error}", file.display())))?;
    let mut problems = 0_usize;
    for (name, hash) in &hashes {
        match references.get(name) {
            Some(expected) if expected == hash => say(out, &format!("ok {name} {hash}")),
            Some(expected) => {
                say(
                    out,
                    &format!("changed {name}: {hash}, reference {expected}"),
                );
                problems = problems.saturating_add(1);
            }
            None => {
                say(out, &format!("no reference for {name}"));
                problems = problems.saturating_add(1);
            }
        }
    }
    for name in references.keys().filter(|name| !hashes.contains_key(*name)) {
        say(out, &format!("stale reference {name}: no such case"));
        problems = problems.saturating_add(1);
    }
    if problems == 0 {
        Ok(())
    } else {
        Err(CliError::Failed(format!(
            "{problems} of the visual cases do not match their references, listed above"
        )))
    }
}

/// Rewrites the references of `dir` from the current renders.
fn bless(dir: &Path, out: &mut dyn Write) -> Result<(), CliError> {
    let hashes = render_all(dir)?;
    let file = dir.join("references.json");
    let text = serde_json::to_string_pretty(&hashes)
        .map_err(|error| CliError::Failed(error.to_string()))?;
    fs::write(&file, format!("{text}\n"))
        .map_err(|error| CliError::Failed(format!("cannot write {}: {error}", file.display())))?;
    say(
        out,
        &format!("wrote {} references to {}", hashes.len(), file.display()),
    );
    Ok(())
}

/// Runs the command `args` (without the program name), writing its output on `out`.
///
/// # Errors
///
/// Returns [`CliError::Usage`] for arguments that form no command, and
/// [`CliError::Failed`] when the command ran and failed.
pub fn run(args: &[String], out: &mut dyn Write) -> Result<(), CliError> {
    match args {
        [command, case] if command == "render" => {
            let hash = hash_of(Path::new(case))?;
            say(out, &hash);
            Ok(())
        }
        [command, dir] if command == "check" => check(Path::new(dir), out),
        [command, dir] if command == "bless" => bless(Path::new(dir), out),
        [command] if command == "help" => {
            drop(writeln!(out, "{USAGE}"));
            Ok(())
        }
        [] => Err(CliError::Usage("no command".to_owned())),
        [command, ..] => Err(CliError::Usage(format!(
            "unknown command or arguments: {command}"
        ))),
    }
}
