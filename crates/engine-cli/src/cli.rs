//! The `engine-cli` commands.

use alloc::collections::BTreeMap;
use std::ffi::OsString;
use std::io::Write;
use std::path::{Path, PathBuf};

use engine_patch::MAX_PATCH_BYTES;

use crate::{Files, Prepared, prepare_case, wav_file};

/// Usage of the `engine-cli` command, printed on stdout by `engine-cli help`.
pub const USAGE: &str = "\
usage: engine-cli <command>
  render <case.json> <patch.json> [--wav <file.wav>]
                      render a case, print its hash, optionally write a WAV file
  check <dir>         compare every case of <dir>/cases (patches in <dir>/patches)
                      with <dir>/references.json
  bless <dir>         rewrite <dir>/references.json from the current renders
  export <dir> <out>  write the WAV file of every case of <dir>/cases to <out>/<case>.wav
  help                print this help";

/// Why a command did not succeed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CliError {
    /// The arguments do not form a command, and why: shown as one `error:` line that
    /// points to `engine-cli help`, exit code 2.
    Usage(String),
    /// The command ran and failed: shown as one `error:` line, exit code 1.
    Failed(String),
}

impl CliError {
    /// The error as the one line the tool prints on stderr, without its newline:
    /// `error: ` and the message made safe by [`engine_patch::safe_message`] (every
    /// character that could break the line or drive the terminal escaped, at most
    /// [`engine_patch::MAX_MESSAGE_CHARS`] characters).
    ///
    /// Messages quote untrusted text (box and field names from a patch): this is the one
    /// place they are made safe, whatever produced them.
    #[must_use]
    pub fn line(&self) -> String {
        format!("error: {}", engine_patch::safe_message(&self.to_string()))
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
            Self::Usage(reason) => write!(f, "{reason} (see: engine-cli help)"),
            Self::Failed(message) => f.write_str(message),
        }
    }
}

impl From<String> for CliError {
    fn from(message: String) -> Self {
        Self::Failed(message)
    }
}

/// Turns the raw command-line arguments (without the program name) into text.
///
/// The boundary where they are checked once. Paths that are not UTF-8 are refused, like
/// case names; the patch format is JSON, UTF-8 too.
///
/// # Errors
///
/// Returns [`CliError::Usage`] naming the first argument that is not valid UTF-8.
pub fn arguments(raw: Vec<OsString>) -> Result<Vec<String>, CliError> {
    raw.into_iter()
        .map(|argument| {
            argument.into_string().map_err(|invalid| {
                CliError::Usage(format!(
                    "argument is not valid UTF-8: {}",
                    invalid.to_string_lossy()
                ))
            })
        })
        .collect()
}

/// Writes to `W`, treating a closed reader (a broken pipe, as with `| head`) as success:
/// what nobody reads is not an error, and the exit code stays the outcome of the work.
#[derive(Debug)]
pub struct BestEffort<W>(pub W);

impl<W: Write> Write for BestEffort<W> {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        match self.0.write(buf) {
            Err(error) if error.kind() == std::io::ErrorKind::BrokenPipe => Ok(buf.len()),
            other => other,
        }
    }

    fn flush(&mut self) -> std::io::Result<()> {
        match self.0.flush() {
            Err(error) if error.kind() == std::io::ErrorKind::BrokenPipe => Ok(()),
            other => other,
        }
    }
}

/// Largest case file read, in bytes (64 KiB): a case holds two numbers.
pub const MAX_CASE_BYTES: usize = 64 * 1024;

/// Largest `references.json` read, in bytes (1 MiB): about 13,000 cases.
pub const MAX_REFERENCES_BYTES: usize = 1024 * 1024;

/// Reads a file of at most `limit` bytes, naming it on failure.
fn read(files: &mut dyn Files, path: &Path, limit: usize) -> Result<String, String> {
    files
        .read(path, limit)
        .map_err(|error| format!("cannot read {}: {error}", path.display()))
}

/// Writes a line of output.
fn say(out: &mut dyn Write, line: &str) -> Result<(), String> {
    writeln!(out, "{line}").map_err(|error| error.to_string())
}

/// A case of a conformance directory: its name, and the path and text of its case file
/// and of its patch file.
#[derive(Debug)]
struct CaseFiles {
    /// The case name: its file name without `.json`.
    name: String,
    /// Path of the case file.
    case_path: PathBuf,
    /// Text of the case file.
    case: String,
    /// Path of the patch file.
    patch_path: PathBuf,
    /// Text of the patch file.
    patch: String,
}

impl CaseFiles {
    /// Prepares the case, naming the file at fault on failure.
    fn prepare(&self) -> Result<Prepared, String> {
        prepare_case(&self.case, &self.patch)
            .map_err(|error| error.in_files(&self.case_path, &self.patch_path))
    }
}

/// The names of the `.json` files of `dir`, sorted, with their paths.
fn json_files(files: &mut dyn Files, dir: &Path) -> Result<Vec<(String, PathBuf)>, String> {
    let mut entries = files
        .list(dir)
        .map_err(|error| format!("cannot list {}: {error}", dir.display()))?;
    entries.sort();
    entries
        .into_iter()
        .filter(|path| {
            path.extension()
                .is_some_and(|extension| extension == "json")
        })
        .map(|path| {
            path.file_stem()
                .and_then(|stem| stem.to_str())
                .map(str::to_owned)
                .map(|name| (name, path.clone()))
                .ok_or_else(|| format!("invalid case file name: {}", path.display()))
        })
        .collect()
}

/// Reads every case of `<dir>/cases` with its patch `<dir>/patches/<name>.json`, sorted
/// by name. Every case needs its patch, and every patch its case.
fn cases(files: &mut dyn Files, dir: &Path) -> Result<Vec<CaseFiles>, String> {
    let patches = dir.join("patches");
    let names = json_files(files, &dir.join("cases"))?;
    let patch_names = json_files(files, &patches)?;
    if let Some((orphan, path)) = patch_names
        .iter()
        .find(|(patch, _)| !names.iter().any(|(name, _)| name == patch))
    {
        return Err(format!(
            "{orphan}: patch without a case: {}",
            path.display()
        ));
    }
    if names.is_empty() {
        return Err(format!(
            "no conformance case in {}",
            dir.join("cases").display()
        ));
    }
    let mut read_cases = Vec::new();
    for (name, case_path) in names {
        let patch_path = patches.join(format!("{name}.json"));
        read_cases.push(CaseFiles {
            case: read(files, &case_path, MAX_CASE_BYTES)?,
            patch: read(files, &patch_path, MAX_PATCH_BYTES)?,
            name,
            case_path,
            patch_path,
        });
    }
    Ok(read_cases)
}

/// Renders every case of `<dir>/cases`, sorted by name: `(name, hash)`.
fn hashes(files: &mut dyn Files, dir: &Path) -> Result<BTreeMap<String, String>, String> {
    let mut hashes = BTreeMap::new();
    for case in cases(files, dir)? {
        let rendered = case.prepare()?.render();
        // Case names are file names of one directory, hence unique.
        drop(hashes.insert(case.name, rendered.hash().to_string()));
    }
    Ok(hashes)
}

/// Where the reference hashes of a conformance directory live.
fn references_path(dir: &Path) -> PathBuf {
    dir.join("references.json")
}

/// `render <case> <patch> [--wav <file>]`.
fn render(files: &mut dyn Files, out: &mut dyn Write, args: &[String]) -> Result<(), CliError> {
    let (case, patch, wav) = match args {
        [case, patch] => (case, patch, None),
        [case, patch, flag, wav] if flag == "--wav" => (case, patch, Some(wav)),
        [_, _, option, ..] if option.starts_with('-') && option != "--wav" => {
            return Err(CliError::Usage(format!("unknown option \"{option}\"")));
        }
        _ => {
            return Err(CliError::Usage(
                "render expects <case.json> <patch.json> [--wav <file.wav>]".to_owned(),
            ));
        }
    };
    let (case_path, patch_path) = (Path::new(case), Path::new(patch));
    let prepared = prepare_case(
        &read(files, case_path, MAX_CASE_BYTES)?,
        &read(files, patch_path, MAX_PATCH_BYTES)?,
    )
    .map_err(|error| error.in_files(case_path, patch_path))?;
    let header = *prepared.wav_header();
    let rendered = prepared.render();
    if let Some(path) = wav {
        files
            .write(Path::new(path), &wav_file(&header, &rendered.samples))
            .map_err(|error| format!("cannot write {path}: {error}"))?;
    }
    Ok(say(out, &rendered.hash().to_string())?)
}

/// `check <dir>`: renders every case and reports each one on `out`, a case that fails
/// to render included, so that one run lists every problem.
///
/// `bless` and `export`, by contrast, stop at the first failing case: a partial set of
/// references or renders would be wrong.
fn check(files: &mut dyn Files, out: &mut dyn Write, dir: &Path) -> Result<(), String> {
    let cases = cases(files, dir)?;
    let path = references_path(dir);
    let expected: BTreeMap<String, String> =
        serde_json::from_str(&read(files, &path, MAX_REFERENCES_BYTES)?)
            .map_err(|error| format!("invalid {}: {error}", path.display()))?;
    let mut failures = 0_usize;
    for case in &cases {
        let name = &case.name;
        let (passed, line) = match case
            .prepare()
            .map(|prepared| prepared.render().hash().to_string())
        {
            Ok(hash) => match expected.get(name) {
                Some(reference) if *reference == hash => (true, format!("ok       {name} {hash}")),
                Some(reference) => (
                    false,
                    format!("MISMATCH {name}: expected {reference}, got {hash}"),
                ),
                None => (
                    false,
                    format!("MISSING  {name}: no reference hash (run bless)"),
                ),
            },
            Err(reason) => (false, format!("FAILED   {name}: {reason}")),
        };
        if !passed {
            failures = failures.saturating_add(1);
        }
        say(out, &line)?;
    }
    for name in expected
        .keys()
        .filter(|name| !cases.iter().any(|case| case.name == **name))
    {
        failures = failures.saturating_add(1);
        say(out, &format!("STALE    {name}: reference without a case"))?;
    }
    if failures == 0 {
        Ok(())
    } else {
        Err(format!(
            "conformance failed in {}: {failures} problem(s)",
            dir.display()
        ))
    }
}

/// `bless <dir>`.
fn bless(files: &mut dyn Files, out: &mut dyn Write, dir: &Path) -> Result<(), String> {
    let actual = hashes(files, dir)?;
    let path = references_path(dir);
    let entries = actual
        .iter()
        .map(|(name, hash)| (name.clone(), serde_json::Value::from(hash.as_str())));
    let json = format!("{:#}\n", serde_json::Value::Object(entries.collect()));
    files
        .write(&path, json.as_bytes())
        .map_err(|error| format!("cannot write {}: {error}", path.display()))?;
    say(
        out,
        &format!("blessed {} case(s) in {}", actual.len(), path.display()),
    )
}

/// `export <dir> <out>`: the WAV file of every case, for tools that check renders
/// independently of this engine.
fn export(
    files: &mut dyn Files,
    out: &mut dyn Write,
    dir: &Path,
    target: &Path,
) -> Result<(), String> {
    files
        .create_dir_all(target)
        .map_err(|error| format!("cannot create {}: {error}", target.display()))?;
    for case in cases(files, dir)? {
        let prepared = case.prepare()?;
        let header = *prepared.wav_header();
        let rendered = prepared.render();
        let path = target.join(format!("{}.wav", case.name));
        files
            .write(&path, &wav_file(&header, &rendered.samples))
            .map_err(|error| format!("cannot write {}: {error}", path.display()))?;
        say(
            out,
            &format!("wrote    {} {}", path.display(), rendered.hash()),
        )?;
    }
    Ok(())
}

/// Runs the command named by `args`.
///
/// # Errors
///
/// Returns [`CliError::Usage`] for a missing or unknown command or invalid arguments,
/// and [`CliError::Failed`] for an unreadable or invalid case, a conformance failure or
/// an output that cannot be written.
pub fn run(args: &[String], files: &mut dyn Files, out: &mut dyn Write) -> Result<(), CliError> {
    let Some((command, rest)) = args.split_first() else {
        return Err(CliError::Usage("missing command".to_owned()));
    };
    let expects = |synopsis: &str| CliError::Usage(format!("{command} expects {synopsis}"));
    match (command.as_str(), rest) {
        ("render", _) => render(files, out, rest),
        ("check", [dir]) => Ok(check(files, out, Path::new(dir))?),
        ("bless", [dir]) => Ok(bless(files, out, Path::new(dir))?),
        ("export", [dir, target]) => Ok(export(files, out, Path::new(dir), Path::new(target))?),
        ("help", []) => Ok(say(out, USAGE)?),
        ("check" | "bless", _) => Err(expects("<dir>")),
        ("export", _) => Err(expects("<dir> <out>")),
        ("help", _) => Err(expects("no argument")),
        _ => Err(CliError::Usage(format!("unknown command \"{command}\""))),
    }
}
