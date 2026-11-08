//! The `engine-cli` commands.

use alloc::collections::BTreeMap;
use std::io::Write;
use std::path::{Path, PathBuf};

use crate::{Files, prepare_case, render_case, wav_file};

/// Usage of the `engine-cli` command.
pub const USAGE: &str = "usage: engine-cli <command>
  render <case.json> <patch.json> [--wav <file.wav>]  render a case, print its hash, optionally write a WAV file
  check <dir>         compare every case of <dir>/cases (patches in <dir>/patches) with <dir>/references.json
  bless <dir>         rewrite <dir>/references.json from the current renders
  export <dir> <out>  write the WAV file of every case of <dir>/cases to <out>/<case>.wav";

/// Why a command did not succeed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CliError {
    /// The arguments do not form a command: shown with [`USAGE`], exit code 2.
    Usage,
    /// The command ran and failed: shown as one `error:` line, exit code 1.
    Failed(String),
}

impl core::fmt::Display for CliError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Usage => f.write_str(USAGE),
            Self::Failed(message) => f.write_str(message),
        }
    }
}

impl From<String> for CliError {
    fn from(message: String) -> Self {
        Self::Failed(message)
    }
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

/// Reads a file, naming it on failure.
fn read(files: &mut dyn Files, path: &Path) -> Result<String, String> {
    files
        .read(path)
        .map_err(|error| format!("cannot read {}: {error}", path.display()))
}

/// Writes a line of output.
fn say(out: &mut dyn Write, line: &str) -> Result<(), String> {
    writeln!(out, "{line}").map_err(|error| error.to_string())
}

/// A case of a conformance directory: its name, the text of its case file and the text
/// of its patch file.
type CaseFiles = (String, String, String);

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
    for (name, path) in names {
        let case = read(files, &path)?;
        let patch = read(files, &patches.join(format!("{name}.json")))?;
        read_cases.push((name, case, patch));
    }
    Ok(read_cases)
}

/// Renders every case of `<dir>/cases`, sorted by name: `(name, hash)`.
fn hashes(files: &mut dyn Files, dir: &Path) -> Result<BTreeMap<String, String>, String> {
    let mut hashes = BTreeMap::new();
    for (name, case, patch) in cases(files, dir)? {
        let rendered = render_case(&case, &patch).map_err(|error| format!("{name}: {error}"))?;
        // Case names are file names of one directory, hence unique.
        drop(hashes.insert(name, rendered.hash().to_string()));
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
        _ => return Err(CliError::Usage),
    };
    let case = read(files, Path::new(case))?;
    let prepared = prepare_case(&case, &read(files, Path::new(patch))?)?;
    let header = *prepared.wav_header();
    let rendered = prepared.render();
    if let Some(path) = wav {
        files
            .write(Path::new(path), &wav_file(&header, &rendered.samples))
            .map_err(|error| format!("cannot write {path}: {error}"))?;
    }
    Ok(say(out, &rendered.hash().to_string())?)
}

/// `check <dir>`.
fn check(files: &mut dyn Files, out: &mut dyn Write, dir: &Path) -> Result<(), String> {
    let actual = hashes(files, dir)?;
    let path = references_path(dir);
    let expected: BTreeMap<String, String> = serde_json::from_str(&read(files, &path)?)
        .map_err(|error| format!("invalid {}: {error}", path.display()))?;
    let mut failures = 0_usize;
    for (name, hash) in &actual {
        match expected.get(name) {
            Some(reference) if reference == hash => say(out, &format!("ok       {name} {hash}"))?,
            Some(reference) => {
                failures = failures.saturating_add(1);
                say(
                    out,
                    &format!("MISMATCH {name}: expected {reference}, got {hash}"),
                )?;
            }
            None => {
                failures = failures.saturating_add(1);
                say(
                    out,
                    &format!("MISSING  {name}: no reference hash (run bless)"),
                )?;
            }
        }
    }
    for name in expected.keys().filter(|name| !actual.contains_key(*name)) {
        failures = failures.saturating_add(1);
        say(out, &format!("STALE    {name}: reference without a case"))?;
    }
    if failures == 0 {
        Ok(())
    } else {
        Err(format!("conformance failed: {failures} problem(s)"))
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
    for (name, case, patch) in cases(files, dir)? {
        let prepared = prepare_case(&case, &patch).map_err(|error| format!("{name}: {error}"))?;
        let header = *prepared.wav_header();
        let rendered = prepared.render();
        let path = target.join(format!("{name}.wav"));
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
/// Returns [`CliError::Usage`] for an unknown command or invalid arguments, and
/// [`CliError::Failed`] for an unreadable or invalid case or a conformance failure.
pub fn run(args: &[String], files: &mut dyn Files, out: &mut dyn Write) -> Result<(), CliError> {
    match args {
        [command, rest @ ..] if command == "render" => render(files, out, rest),
        [command, dir] if command == "check" => Ok(check(files, out, Path::new(dir))?),
        [command, dir] if command == "bless" => Ok(bless(files, out, Path::new(dir))?),
        [command, dir, target] if command == "export" => {
            Ok(export(files, out, Path::new(dir), Path::new(target))?)
        }
        _ => Err(CliError::Usage),
    }
}
