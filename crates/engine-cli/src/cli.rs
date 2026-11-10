//! The `engine-cli` commands.

use alloc::collections::BTreeMap;
use std::ffi::OsString;
use std::io::Write;
use std::path::{Path, PathBuf};

use engine_core::RenderFormat;
use engine_fpenv::{DefaultMode, with_default_mode};
use engine_patch::MAX_PATCH_BYTES;

use engine_objects::REGISTRY;

use crate::record::{
    Record, References, Versions, bless_problems, blessed, current_fingerprints, record_problems,
    same_rules,
};
use crate::{Files, Prepared, Rendered, prepare_case};
use engine_hash::RenderHash;
use serde::Serialize;
use serde::de::DeserializeOwned;

/// Usage of the `engine-cli` command, printed on stdout by `engine-cli help`.
pub const USAGE: &str = "\
usage: engine-cli <command>
  render <case.json> <patch.json> [--format <format>] [--wav <file.wav>]
                      render a case in <format> (binary64, the default,
                      binary64-as-binary32 or binary32), print its hash,
                      optionally write a WAV file
  check <dir>         compare every case of <dir>/cases (patches in <dir>/patches),
                      in every format, with <dir>/references.json, and
                      the algorithm versions of its box types, with
                      <dir>/algorithms.json
  bless <dir> [--case-changed <name>]...
                      rewrite <dir>/references.json from the current renders,
                      and <dir>/algorithms.json from the current box types and
                      engine; refuses a reference changed under the same
                      versions but for each case named, a lower version, and
                      a box type's outputs changed at the same version
  export <dir> <out>  write the WAV file of every case of <dir>/cases, in every
                      format, to <out>/<case>.<format>.wav
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
    /// Messages quote untrusted text (box and field names from a patch): this is where
    /// the error line is made safe, whatever produced it; every line on stdout is made
    /// safe the same way, as a `OneLine`.
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

/// One line of stdout, made safe: its only constructor passes the whole text through
/// [`engine_patch::safe_message`], so that untrusted parts (case names from file names,
/// box types and field names quoted by an error, the keys and values of
/// `references.json`, paths) can neither drive the terminal nor forge a second line, and
/// the line holds at most [`engine_patch::MAX_MESSAGE_CHARS`] characters.
mod one_line {
    /// A line of text with no character that could break it or drive a terminal.
    #[derive(Debug)]
    pub(super) struct OneLine(String);

    impl OneLine {
        /// `text` made safe to print as one line.
        pub(super) fn new(text: &str) -> Self {
            Self(engine_patch::safe_message(text))
        }

        /// The safe text.
        pub(super) fn as_str(&self) -> &str {
            &self.0
        }
    }
}

use one_line::OneLine;

/// Writes `line` to `out`, which is stdout: every line but the usage goes through here.
fn say(out: &mut dyn Write, line: &OneLine) -> Result<(), String> {
    writeln!(out, "{}", line.as_str()).map_err(|error| stdout_error(&error))
}

/// Writes the usage to `out`, which is stdout: the one output of several lines, and it
/// holds no untrusted text.
fn say_usage(out: &mut dyn Write) -> Result<(), String> {
    writeln!(out, "{USAGE}").map_err(|error| stdout_error(&error))
}

/// A failed write to stdout, naming it.
fn stdout_error(error: &std::io::Error) -> String {
    format!("cannot write to stdout: {error}")
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
    /// Prepares the case in `format`, in the default floating-point mode that `mode`
    /// proves, naming the file at fault on failure.
    fn prepare(&self, format: RenderFormat, mode: DefaultMode<'_>) -> Result<Prepared, String> {
        prepare_case(&self.case, &self.patch, format, mode)
            .map_err(|error| error.in_files(&self.case_path, &self.patch_path))
    }
}

/// Where the algorithm versions of a conformance directory live, beside its references.
fn algorithms_path(dir: &Path) -> PathBuf {
    dir.join("algorithms.json")
}

/// The current algorithm version of every box type the patch of `case` uses, by name.
///
/// # Errors
///
/// Returns why the patch cannot be read, or the box type no registered box type is: the
/// render of such a case fails too.
fn algorithms(case: &CaseFiles) -> Result<Versions, String> {
    let patch = engine_patch::parse_patch(&case.patch)
        .map_err(|error| format!("{}: {error}", case.patch_path.display()))?;
    patch
        .nodes()
        .map(|node| {
            REGISTRY
                .iter()
                .find(|spec| spec.name == node.kind)
                .map(|spec| (node.kind.clone(), spec.algorithm_version.get()))
                .ok_or_else(|| {
                    format!(
                        "{}: unknown box type \"{}\"",
                        case.patch_path.display(),
                        node.kind
                    )
                })
        })
        .collect()
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

/// The hash of `rendered`, or why it is not a render, in words.
pub(crate) fn hash_of(rendered: &Rendered) -> Result<RenderHash, String> {
    rendered
        .hash()
        .map_err(|error| format!("cannot hash the render: {error}"))
}

/// Renders every case of `<dir>/cases` in every render format, sorted by name: for each
/// case, its hash in each format.
fn hashes(files: &mut dyn Files, dir: &Path, mode: DefaultMode<'_>) -> Result<References, String> {
    let mut hashes = BTreeMap::new();
    for case in cases(files, dir)? {
        let mut formats = BTreeMap::new();
        for format in RenderFormat::ALL {
            let rendered = case.prepare(format, mode)?.render(mode);
            drop(formats.insert(format.name().to_owned(), hash_of(&rendered)?.to_string()));
        }
        // Case names are file names of one directory, hence unique.
        drop(hashes.insert(case.name, formats));
    }
    Ok(hashes)
}

/// The render format named `name`.
fn format_named(name: &str) -> Result<RenderFormat, CliError> {
    RenderFormat::ALL
        .into_iter()
        .find(|format| format.name() == name)
        .ok_or_else(|| {
            CliError::Usage(format!(
                "unknown render format \"{name}\" (binary64, binary64-as-binary32 or binary32)"
            ))
        })
}

/// Where the reference hashes of a conformance directory live.
fn references_path(dir: &Path) -> PathBuf {
    dir.join("references.json")
}

/// The options of `render`, each taking a value.
const RENDER_OPTIONS: [&str; 2] = ["--format", "--wav"];

/// `render <case> <patch> [--format <format>] [--wav <file>]`: each option at most once, with
/// a value that is not an option.
fn render(
    files: &mut dyn Files,
    out: &mut dyn Write,
    args: &[String],
    mode: DefaultMode<'_>,
) -> Result<(), CliError> {
    let expects = || {
        CliError::Usage(
            "render expects <case.json> <patch.json> [--format <format>] [--wav <file.wav>]"
                .to_owned(),
        )
    };
    let [case, patch, options @ ..] = args else {
        return Err(expects());
    };
    let (mut format, mut wav): (Option<&String>, Option<&String>) = (None, None);
    let mut words = options.iter();
    while let Some(option) = words.next() {
        let slot = match option.as_str() {
            "--format" => &mut format,
            "--wav" => &mut wav,
            unknown if unknown.starts_with('-') => {
                return Err(CliError::Usage(format!("unknown option \"{unknown}\"")));
            }
            _ => return Err(expects()),
        };
        // A value is never one of the options: `--wav --format` names no file.
        let value = words
            .next()
            .filter(|value| !RENDER_OPTIONS.contains(&value.as_str()))
            .ok_or_else(|| CliError::Usage(format!("option \"{option}\" needs a value")))?;
        if slot.replace(value).is_some() {
            return Err(CliError::Usage(format!("option \"{option}\" given twice")));
        }
    }
    let format = format.map_or(Ok(RenderFormat::Binary64), |name| format_named(name))?;
    let (case_path, patch_path) = (Path::new(case), Path::new(patch));
    let prepared = prepare_case(
        &read(files, case_path, MAX_CASE_BYTES)?,
        &read(files, patch_path, MAX_PATCH_BYTES)?,
        format,
        mode,
    )
    .map_err(|error| error.in_files(case_path, patch_path))?;
    let rendered = prepared.render(mode);
    if let Some(path) = wav {
        files
            .write(Path::new(path), &rendered.wav())
            .map_err(|error| format!("cannot write {path}: {error}"))?;
    }
    Ok(say(out, &OneLine::new(&hash_of(&rendered)?.to_string()))?)
}

/// The problems of the algorithm versions of case `name`: `current`, those of its box types
/// now (none when its patch cannot be read), against `recorded`, those its references were
/// blessed with. A box type at another version than its references were blessed with is
/// refused (a new version needs new references), as are a missing record and the record of
/// a box type the patch does not use.
fn version_problems(
    name: &str,
    current: Option<&Versions>,
    recorded: Option<&Versions>,
) -> Vec<String> {
    let Some(current) = current else {
        return Vec::new();
    };
    let Some(recorded) = recorded else {
        return vec![format!(
            "MISSING  {name}: no algorithm versions (run bless)"
        )];
    };
    let changed = current
        .iter()
        .filter_map(|(kind, version)| match recorded.get(kind) {
            Some(blessed) if blessed == version => None,
            Some(blessed) => Some(format!(
                "VERSION  {name} {kind}: at algorithm version {version}, the references were \
             blessed with version {blessed} (bless again)"
            )),
            None => Some(format!(
                "MISSING  {name} {kind}: no algorithm version (run bless)"
            )),
        });
    let stale = recorded
        .keys()
        .filter(|kind| !current.contains_key(*kind))
        .map(|kind| {
            format!("STALE    {name} {kind}: algorithm version of a box type the case does not use")
        });
    changed.chain(stale).collect()
}

/// `check <dir>`: renders every case and reports each one on `out`, a case that fails
/// to render included, so that one run lists every problem.
///
/// `bless` and `export`, by contrast, stop at the first failing case: a partial set of
/// references or renders would be wrong.
fn check(
    files: &mut dyn Files,
    out: &mut dyn Write,
    dir: &Path,
    mode: DefaultMode<'_>,
) -> Result<(), String> {
    let cases = cases(files, dir)?;
    let expected: References = read_blessed(files, &references_path(dir))?;
    let record: Record = read_blessed(files, &algorithms_path(dir))?;
    let blessed = &record.cases;
    let rules = same_rules(&record);
    let mut failures = 0_usize;
    let mut report = |stdout: &mut dyn Write, passed: bool, line: String| {
        if !passed {
            failures = failures.saturating_add(1);
        }
        say(stdout, &OneLine::new(&line))
    };
    for case in &cases {
        let name = &case.name;
        let references = expected.get(name);
        // The versions now and when blessed; a case whose patch cannot be read has none
        // now, and its render reports why.
        let current = algorithms(case).ok();
        let recorded = blessed.get(name);
        let same_versions = rules && current.is_some() && current.as_ref() == recorded;
        for format in RenderFormat::ALL {
            let (passed, line) = match case
                .prepare(format, mode)
                .and_then(|prepared| hash_of(&prepared.render(mode)))
                .map(|hash| hash.to_string())
            {
                Ok(hash) => match references.and_then(|hashes| hashes.get(format.name())) {
                    Some(reference) if *reference == hash => {
                        (true, format!("ok       {name} {format} {hash}"))
                    }
                    Some(reference) => (
                        false,
                        format!(
                            "MISMATCH {name} {format}: expected {reference}, got {hash}{}",
                            if same_versions {
                                " (the same algorithm versions: its bits changed without a \
                                 new version)"
                            } else {
                                ""
                            }
                        ),
                    ),
                    None => (
                        false,
                        format!("MISSING  {name} {format}: no reference hash (run bless)"),
                    ),
                },
                Err(reason) => (false, format!("FAILED   {name} {format}: {reason}")),
            };
            report(out, passed, line)?;
        }
        for unknown in references
            .into_iter()
            .flat_map(BTreeMap::keys)
            .filter(|key| !RenderFormat::ALL.iter().any(|format| format.name() == *key))
        {
            report(
                out,
                false,
                format!("STALE    {name} {unknown}: reference of no render format"),
            )?;
        }
        for line in version_problems(name, current.as_ref(), recorded) {
            report(out, false, line)?;
        }
    }
    for line in record_problems(&record, &current_fingerprints()) {
        report(out, false, line)?;
    }
    for name in expected
        .keys()
        .filter(|name| !cases.iter().any(|case| case.name == **name))
    {
        report(
            out,
            false,
            format!("STALE    {name}: reference without a case"),
        )?;
    }
    for name in blessed
        .keys()
        .filter(|name| !cases.iter().any(|case| case.name == **name))
    {
        report(
            out,
            false,
            format!("STALE    {name}: algorithm versions without a case"),
        )?;
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

/// `bless <dir> [--case-changed <name>]...`: the reference hash of every case in every
/// render format, cases and the names of the formats sorted, and the record of what they
/// were blessed under (`spec/engine-design.md`, decision 29).
///
/// Refuses, writing nothing and naming each problem on `out`, what [`bless_problems`]
/// refuses against the files it replaces; a directory without them is blessed whole.
fn bless(
    files: &mut dyn Files,
    out: &mut dyn Write,
    dir: &Path,
    changed: &[String],
    mode: DefaultMode<'_>,
) -> Result<(), String> {
    // The versions first: a patch that cannot be read stops the bless before any render.
    let versions: BTreeMap<String, Versions> = cases(files, dir)?
        .iter()
        .map(|case| algorithms(case).map(|current| (case.name.clone(), current)))
        .collect::<Result<_, _>>()?;
    let actual = hashes(files, dir, mode)?;
    let path = references_path(dir);
    let versions_path = algorithms_path(dir);
    let earlier: Option<References> = read_earlier(files, &path)?;
    let old: Option<Record> = read_earlier(files, &versions_path)?;
    let now = current_fingerprints();
    let problems = old.as_ref().map_or_else(Vec::new, |old| {
        bless_problems(
            old,
            &earlier.unwrap_or_default(),
            &now,
            &versions,
            &actual,
            changed,
        )
    });
    if !problems.is_empty() {
        for problem in &problems {
            say(out, &OneLine::new(problem))?;
        }
        return Err(format!(
            "bless refused in {}: {} problem(s), nothing written",
            dir.display(),
            problems.len()
        ));
    }
    let record = blessed(versions, &now, old.as_ref());
    files
        .write(&path, blessed_text(&actual).as_bytes())
        .map_err(|error| format!("cannot write {}: {error}", path.display()))?;
    // References first: if the record cannot be written after them, `check` names the
    // versions to bless again, never a reference changed without a new version.
    files
        .write(&versions_path, blessed_text(&record).as_bytes())
        .map_err(|error| {
            format!(
                "cannot write {}: {error} ({} was written: bless again)",
                versions_path.display(),
                path.display()
            )
        })?;
    say(
        out,
        &OneLine::new(&format!(
            "blessed {} case(s) in {}",
            actual.len(),
            path.display()
        )),
    )
}

/// The text `bless` writes for `value`: JSON indented by two spaces, keys sorted, and a
/// final newline. The files it writes are read back only in exactly this form.
fn blessed_text<T: Serialize>(value: &T) -> String {
    format!("{:#}\n", serde_json::json!(value))
}

/// Reads `path`, a file only `bless` writes, refusing it unless it is exactly as `bless`
/// writes it (a repeated key or a hand edit included): its contents decide what `check`
/// accepts.
///
/// # Errors
///
/// Returns why it cannot be read or is not as `bless` writes it, saying to run bless when
/// that rewrites it: when it is missing, invalid or not in its written form.
fn read_blessed<T: Serialize + DeserializeOwned>(
    files: &mut dyn Files,
    path: &Path,
) -> Result<T, String> {
    let text = files.read(path, MAX_REFERENCES_BYTES).map_err(|error| {
        let hint = if error.kind() == std::io::ErrorKind::NotFound {
            " (run bless)"
        } else {
            ""
        };
        format!("cannot read {}: {error}{hint}", path.display())
    })?;
    let value: T = serde_json::from_str(&text)
        .map_err(|error| format!("invalid {}: {error} (run bless)", path.display()))?;
    if blessed_text(&value) == text {
        Ok(value)
    } else {
        Err(format!(
            "{} is not as bless writes it (run bless)",
            path.display()
        ))
    }
}

/// Reads `path`, a file `bless` is about to replace: none when it is missing (a first
/// bless), else its contents, in any JSON form.
///
/// # Errors
///
/// Returns why it cannot be read, or is not valid: what it holds could not be compared.
fn read_earlier<T: DeserializeOwned>(
    files: &mut dyn Files,
    path: &Path,
) -> Result<Option<T>, String> {
    match files.read(path, MAX_REFERENCES_BYTES) {
        Ok(text) => serde_json::from_str(&text).map(Some).map_err(|error| {
            format!(
                "invalid {}: {error} (bless compares with it: fix it, or remove it to bless \
                 from nothing)",
                path.display()
            )
        }),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(format!("cannot read {}: {error}", path.display())),
    }
}

/// `export <dir> <out>`: the WAV file of every case, for tools that check renders
/// independently of this engine.
fn export(
    files: &mut dyn Files,
    out: &mut dyn Write,
    dir: &Path,
    target: &Path,
    mode: DefaultMode<'_>,
) -> Result<(), String> {
    files
        .create_dir_all(target)
        .map_err(|error| format!("cannot create {}: {error}", target.display()))?;
    for case in cases(files, dir)? {
        for format in RenderFormat::ALL {
            let rendered = case.prepare(format, mode)?.render(mode);
            let path = target.join(format!("{}.{format}.wav", case.name));
            files
                .write(&path, &rendered.wav())
                .map_err(|error| format!("cannot write {}: {error}", path.display()))?;
            let hash = hash_of(&rendered)?;
            say(
                out,
                &OneLine::new(&format!("wrote    {} {hash}", path.display())),
            )?;
        }
    }
    Ok(())
}

/// Runs the command named by `args`, in the default floating-point mode of the processor.
///
/// The mode is set by `engine_fpenv::with_default_mode`, whatever mode the calling thread
/// had: renders are the same bits as WebAssembly's even under a host that flushes
/// subnormals.
///
/// # Errors
///
/// Returns [`CliError::Usage`] for a missing or unknown command or invalid arguments,
/// and [`CliError::Failed`] for an unreadable or invalid case, a conformance failure or
/// an output that cannot be written.
pub fn run(args: &[String], files: &mut dyn Files, out: &mut dyn Write) -> Result<(), CliError> {
    with_default_mode(|mode| dispatch(args, files, out, mode))
}

/// Runs the command named by `args`, rendering in the default floating-point mode `mode`
/// proves ([`run`]).
fn dispatch(
    args: &[String],
    files: &mut dyn Files,
    out: &mut dyn Write,
    mode: DefaultMode<'_>,
) -> Result<(), CliError> {
    let Some((command, rest)) = args.split_first() else {
        return Err(CliError::Usage("missing command".to_owned()));
    };
    let expects = |synopsis: &str| CliError::Usage(format!("{command} expects {synopsis}"));
    match (command.as_str(), rest) {
        ("render", _) => render(files, out, rest, mode),
        ("check", [dir]) => Ok(check(files, out, Path::new(dir), mode)?),
        ("bless", [dir, options @ ..]) => {
            let mut changed = Vec::new();
            for pair in options.chunks(2) {
                match pair {
                    [flag, name] if flag == "--case-changed" => changed.push(name.clone()),
                    [flag, _] => {
                        return Err(CliError::Usage(format!("unknown option \"{flag}\"")));
                    }
                    _ => return Err(expects("<dir> [--case-changed <name>]...")),
                }
            }
            Ok(bless(files, out, Path::new(dir), &changed, mode)?)
        }
        ("export", [dir, target]) => {
            Ok(export(files, out, Path::new(dir), Path::new(target), mode)?)
        }
        ("help", []) => Ok(say_usage(out)?),
        ("check", _) => Err(expects("<dir>")),
        ("bless", _) => Err(expects("<dir> [--case-changed <name>]...")),
        ("export", _) => Err(expects("<dir> <out>")),
        ("help", _) => Err(expects("no argument")),
        _ => Err(CliError::Usage(format!("unknown command \"{command}\""))),
    }
}
