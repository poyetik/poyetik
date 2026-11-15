//! The `visual-cli` commands.

use alloc::collections::BTreeMap;
use std::ffi::OsString;
use std::io::Write;
use std::path::{Path, PathBuf};

use crate::status::Status;
use crate::{Files, MAX_CASE_BYTES, hash_case, safe_message};

/// Usage of the `visual-cli` command, printed on stdout by `visual-cli help`.
pub const USAGE: &str = "\
usage: visual-cli <command>
  render <case.json>  render a visual case, print its image hash
  check <dir>         compare every case of <dir>/cases with <dir>/references.json
  bless <dir>         rewrite <dir>/references.json from the current renders
  help                print this help";

/// What follows the reason of a usage error: where the usage is.
const USAGE_HINT: &str = " (see: visual-cli help)";

/// The largest references file read, in bytes.
pub const MAX_REFERENCES_BYTES: usize = 1024 * 1024;

/// Why a command did not succeed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CliError {
    /// The arguments do not form a command: exit code 2.
    Usage(String),
    /// The command ran and failed: exit code 1.
    Failed(String),
}

impl CliError {
    /// The error as the one line the tool prints on stderr, without its newline: `error: `
    /// and the message made safe by [`safe_message`], then, for a usage error, its pointer
    /// to the usage, ` (see: visual-cli help)`, appended after the reason is bounded, so
    /// that no reason can cut it.
    #[must_use]
    pub fn line(&self) -> String {
        match self {
            Self::Usage(reason) => format!("error: {}{USAGE_HINT}", safe_message(reason)),
            Self::Failed(message) => format!("error: {}", safe_message(message)),
        }
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
            Self::Usage(reason) => write!(f, "{reason}{USAGE_HINT}"),
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

/// Reads a file of at most `limit` bytes, naming it on failure.
fn read(files: &mut dyn Files, path: &Path, limit: usize) -> Result<String, CliError> {
    files
        .read(path, limit)
        .map_err(|error| CliError::Failed(format!("cannot read {}: {error}", path.display())))
}

/// Renders the case file at `path` and returns its hash as text.
fn hash_of(files: &mut dyn Files, path: &Path) -> Result<String, CliError> {
    let text = read(files, path, MAX_CASE_BYTES)?;
    hash_case(&text)
        .map(|hash| hash.to_string())
        .map_err(|error| CliError::Failed(format!("{}: {error}", path.display())))
}

/// What a case name may hold, as an error says: so that it can be pasted into a command
/// unquoted, in any shell, and is the same file on every file system (the names
/// `audio-cli` takes).
const CASE_NAME: &str =
    "lowercase ASCII letters, digits, dots and hyphens, starting with a letter or a digit";

/// Whether `name` is a case name ([`CASE_NAME`]).
fn case_name(name: &str) -> bool {
    let mut bytes = name.bytes();
    let word = |byte: u8| byte.is_ascii_lowercase() || byte.is_ascii_digit();
    bytes.next().is_some_and(word) && bytes.all(|byte| word(byte) || byte == b'.' || byte == b'-')
}

/// The `.json` files of `<dir>/cases`, sorted, each with its case name ([`CASE_NAME`]);
/// any other file (a `.DS_Store` a file manager left) is not a case and is skipped.
fn case_files(files: &mut dyn Files, dir: &Path) -> Result<Vec<(String, PathBuf)>, CliError> {
    let cases = dir.join("cases");
    let mut entries = files
        .list(&cases)
        .map_err(|error| CliError::Failed(format!("cannot list {}: {error}", cases.display())))?;
    entries.sort();
    let named: Vec<(String, PathBuf)> = entries
        .into_iter()
        .filter(|path| {
            path.extension()
                .is_some_and(|extension| extension == "json")
        })
        .map(|path| {
            path.file_stem()
                .and_then(|stem| stem.to_str())
                .filter(|name| case_name(name))
                .map(|name| (name.to_owned(), path.clone()))
                .ok_or_else(|| {
                    CliError::Failed(format!("{}: not a case name ({CASE_NAME})", path.display()))
                })
        })
        .collect::<Result<_, _>>()?;
    if named.is_empty() {
        return Err(CliError::Failed(format!("no case in {}", cases.display())));
    }
    Ok(named)
}

/// The hash of every case of `<dir>/cases`, by case name (the file name without `.json`).
fn render_all(files: &mut dyn Files, dir: &Path) -> Result<BTreeMap<String, String>, CliError> {
    let mut hashes = BTreeMap::new();
    for (name, path) in case_files(files, dir)? {
        let hash = hash_of(files, &path)?;
        let _earlier = hashes.insert(name, hash);
    }
    Ok(hashes)
}

/// A writer to stdout for which a closed reader (a broken pipe, `| head`) is not an error:
/// what is written after it is dropped, and the command goes on.
#[derive(Debug)]
pub struct BestEffort<W>(pub W);

impl<W: Write> Write for BestEffort<W> {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        unless_closed(self.0.write(buf), buf.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        unless_closed(self.0.flush(), ())
    }
}

/// `result`, or `done` when it failed only because the reader is gone (a broken pipe).
///
/// Apart from [`BestEffort`], which is generic over its writer, so that the decision is
/// compiled once per result type whatever the writer, and every branch of it is reached
/// by the tests (the coverage gate counts each instantiation of a generic function).
fn unless_closed<T>(result: std::io::Result<T>, done: T) -> std::io::Result<T> {
    match result {
        Err(error) if error.kind() == std::io::ErrorKind::BrokenPipe => Ok(done),
        other => other,
    }
}

/// Writes one line, made safe by [`safe_message`], to `out`, which is stdout.
///
/// # Errors
///
/// Returns [`CliError::Failed`] when stdout cannot be written (a closed one is
/// [`BestEffort`]'s to forgive).
fn say(out: &mut dyn Write, line: &str) -> Result<(), CliError> {
    writeln!(out, "{}", safe_message(line))
        .map_err(|error| CliError::Failed(format!("cannot write to stdout: {error}")))
}

/// The references file of `hashes`, exactly as `visual-cli bless` writes it: a JSON object
/// of case names to hashes, in name order, indented, ending with a newline.
fn references_text(hashes: &BTreeMap<String, String>) -> String {
    let object: serde_json::Map<String, serde_json::Value> = hashes
        .iter()
        .map(|(name, hash)| (name.clone(), serde_json::Value::String(hash.clone())))
        .collect();
    format!("{:#}\n", serde_json::Value::Object(object))
}

/// Reads the references of `dir`, only exactly as `visual-cli bless` writes them.
///
/// A file that is not a JSON object of names to hashes is to be restored, not rewritten:
/// rewriting it from the current renders would also take in any change of the renders.
fn references(files: &mut dyn Files, dir: &Path) -> Result<BTreeMap<String, String>, CliError> {
    let file = dir.join("references.json");
    let text = read(files, &file, MAX_REFERENCES_BYTES)?;
    let references: BTreeMap<String, String> = serde_json::from_str(&text).map_err(|error| {
        CliError::Failed(format!(
            "{} is not a JSON object of case names to hashes ({error}): restore it from git (`visual-cli bless {}` rewrites it from the current renders, only when they are meant to change)",
            file.display(),
            dir.display()
        ))
    })?;
    if references_text(&references) == text {
        Ok(references)
    } else {
        Err(CliError::Failed(format!(
            "{} is not as `visual-cli bless {}` writes it: rewrite it with that command",
            file.display(),
            dir.display()
        )))
    }
}

/// Compares the renders of `dir` with its references.
fn check(files: &mut dyn Files, dir: &Path, out: &mut dyn Write) -> Result<(), CliError> {
    let hashes = render_all(files, dir)?;
    let references = references(files, dir)?;
    let mut problems = 0_usize;
    for (name, hash) in &hashes {
        let (status, line) = match references.get(name) {
            Some(expected) if expected == hash => (Status::Ok, format!("{name} {hash}")),
            Some(expected) => (
                Status::Mismatch,
                format!("{name}: {hash}, reference {expected}"),
            ),
            None => (Status::Missing, format!("{name}: no reference")),
        };
        if status != Status::Ok {
            problems = problems.saturating_add(1);
        }
        say(out, &format!("{status} {line}"))?;
    }
    for name in references.keys().filter(|name| !hashes.contains_key(*name)) {
        say(
            out,
            &format!("{} {name}: reference without a case", Status::Stale),
        )?;
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
fn bless(files: &mut dyn Files, dir: &Path, out: &mut dyn Write) -> Result<(), CliError> {
    let hashes = render_all(files, dir)?;
    let file = dir.join("references.json");
    files
        .write(&file, references_text(&hashes).as_bytes())
        .map_err(|error| CliError::Failed(format!("cannot write {}: {error}", file.display())))?;
    say(
        out,
        &format!(
            "{} {} references to {}",
            Status::Wrote,
            hashes.len(),
            file.display()
        ),
    )
}

/// Runs the command `args` (without the program name) on `files`, writing its output on
/// `out`, which is stdout.
///
/// # Errors
///
/// Returns [`CliError::Usage`] for arguments that form no command, and
/// [`CliError::Failed`] when the command ran and failed.
pub fn run(args: &[String], files: &mut dyn Files, out: &mut dyn Write) -> Result<(), CliError> {
    let Some((command, rest)) = args.split_first() else {
        return Err(CliError::Usage("no command".to_owned()));
    };
    let expects = |synopsis: &str| CliError::Usage(format!("{command} expects {synopsis}"));
    match (command.as_str(), rest) {
        ("render", [case]) => {
            let hash = hash_of(files, Path::new(case))?;
            say(out, &hash)
        }
        ("check", [dir]) => check(files, Path::new(dir), out),
        ("bless", [dir]) => bless(files, Path::new(dir), out),
        ("help", []) => writeln!(out, "{USAGE}")
            .map_err(|error| CliError::Failed(format!("cannot write to stdout: {error}"))),
        ("render", _) => Err(expects("<case.json>")),
        ("check" | "bless", _) => Err(expects("<dir>")),
        ("help", _) => Err(expects("no argument")),
        _ => Err(CliError::Usage(format!("unknown command \"{command}\""))),
    }
}
