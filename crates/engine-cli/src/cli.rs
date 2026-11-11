//! The `engine-cli` commands.

use crate::status::Status;
use alloc::collections::BTreeMap;
use std::ffi::OsString;
use std::io::Write;
use std::path::{Path, PathBuf};

use engine_core::RenderFormat;
use engine_fpenv::{DefaultMode, with_default_mode};
use engine_patch::MAX_PATCH_BYTES;

use engine_objects::REGISTRY;

use crate::record::{
    Before, Blessing, Record, References, Spent, Told, Versions, bless_problems, blessed,
    current_fingerprints, record_problems, same_rules, spent_problems, unknown_cases,
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
  check <dir> [--spent <record>]
                      compare every case of <dir>/cases (patches in <dir>/patches),
                      in every format, with <dir>/references.json, and
                      the algorithm versions of its box types, with
                      <dir>/algorithms.json, whose fingerprints must be those of
                      <record>, the record on main, at the versions both hold
  bless <dir> [--case-changed <name>]... [--spent <record>]
        [--battery-changed] [--from-nothing]
                      rewrite <dir>/references.json from the current renders,
                      and <dir>/algorithms.json from the current box types and
                      engine; refuses a reference changed under the same
                      versions but for each case named, a lower version, a
                      box type's outputs changed at a spent version (those of
                      <record>, the record on main, which `cargo xtask bless`
                      writes: a file holding null when main has none; else
                      those of <dir>/algorithms.json), another battery
                      but with --battery-changed (then no reference, version,
                      render rules or render hash may change), and a
                      directory missing either file but with --from-nothing
                      (neither file then)
  export <dir> <out>  write the WAV file of every case of <dir>/cases, in every
                      format, to <out>/<case>.<format>.wav
  help                print this help";

/// What a usage error ends with: where the usage is.
const USAGE_HINT: &str = " (see: engine-cli help)";

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
    /// [`engine_patch::MAX_MESSAGE_CHARS`] characters), then, for a usage error, its
    /// pointer to the usage, ` (see: engine-cli help)`, appended after the reason is
    /// bounded, so that no reason can cut it.
    ///
    /// Messages quote untrusted text (box and field names from a patch): this is where
    /// the error line is made safe, whatever produced it; every line on stdout is made
    /// safe the same way, as a `OneLine`.
    #[must_use]
    pub fn line(&self) -> String {
        match self {
            Self::Usage(reason) => {
                format!("error: {}{USAGE_HINT}", engine_patch::safe_message(reason))
            }
            Self::Failed(message) => format!("error: {}", engine_patch::safe_message(message)),
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

/// Largest case file read, in bytes (64 KiB): a case holds two numbers.
pub const MAX_CASE_BYTES: usize = 64 * 1024;

/// Largest `references.json` or `algorithms.json` read, in bytes (1 MiB).
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

/// Renders every case of `cases` in every render format: for each case, by name, its hash
/// in each format.
fn hashes(cases: &[CaseFiles], mode: DefaultMode<'_>) -> Result<References, String> {
    let mut hashes = BTreeMap::new();
    for case in cases {
        let mut formats = BTreeMap::new();
        for format in RenderFormat::ALL {
            let rendered = case.prepare(format, mode)?.render(mode);
            drop(formats.insert(format.name().to_owned(), hash_of(&rendered)?.to_string()));
        }
        // Case names are file names of one directory, hence unique.
        drop(hashes.insert(case.name.clone(), formats));
    }
    Ok(hashes)
}

/// The render format named `name`.
fn format_named(name: &str) -> Result<RenderFormat, CliError> {
    RenderFormat::ALL
        .into_iter()
        .find(|format| format.name() == name)
        .ok_or_else(|| {
            // From `RenderFormat::ALL`: a fourth format does not compile until it is named here.
            let [first, second, third] = RenderFormat::ALL.map(RenderFormat::name);
            CliError::Usage(format!(
                "unknown render format \"{name}\" ({first}, {second} or {third})"
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
/// a box type the patch does not use; `referenced` says whether the case has references,
/// which a bless replaces only when the case is named.
fn version_problems(
    name: &str,
    current: Option<&Versions>,
    recorded: Option<&Versions>,
    referenced: bool,
) -> Vec<String> {
    let Some(current) = current else {
        return Vec::new();
    };
    let Some(recorded) = recorded else {
        // A case with references but no versions is one bless refuses unless it is named
        // (`bless_problems`): the hint names it.
        let hint = if referenced {
            format!("`cargo xtask bless --case-changed {name}`, or restore the record from git")
        } else {
            "run `cargo xtask bless`".to_owned()
        };
        return vec![format!(
            "{status} {name}: no algorithm versions ({hint})",
            status = Status::Missing
        )];
    };
    let changed = current
        .iter()
        .filter_map(|(kind, version)| match recorded.get(kind) {
            Some(blessed) if blessed == version => None,
            Some(blessed) => Some(format!(
                "{status} {name} {kind}: at algorithm version {version}, the references were \
             blessed with version {blessed} (check the renders of the new version, then \
             run `cargo xtask bless`)",
                status = Status::Version
            )),
            None => Some(format!(
                "{status} {name} {kind}: no algorithm version (run `cargo xtask bless`)",
                status = Status::Missing
            )),
        });
    let stale = recorded
        .keys()
        .filter(|kind| !current.contains_key(*kind))
        .map(|kind| {
            format!(
                "{status} {name} {kind}: algorithm version of a box type the case does not use",
                status = Status::Stale
            )
        });
    changed.chain(stale).collect()
}

/// The line `check` prints for `case` in `format`, and whether it passed: its render
/// against its reference among `references`, `same_versions` saying whether the case's
/// algorithm versions and render rules are those its references were blessed with.
fn format_line(
    case: &CaseFiles,
    format: RenderFormat,
    references: Option<&BTreeMap<String, String>>,
    same_versions: bool,
    mode: DefaultMode<'_>,
) -> (bool, String) {
    let name = &case.name;
    match case
        .prepare(format, mode)
        .and_then(|prepared| hash_of(&prepared.render(mode)))
        .map(|hash| hash.to_string())
    {
        Ok(hash) => match references.and_then(|hashes| hashes.get(format.name())) {
            Some(reference) if *reference == hash => (
                true,
                format!("{status} {name} {format} {hash}", status = Status::Ok),
            ),
            Some(reference) => (
                false,
                format!(
                    "{status} {name} {format}: expected {reference}, got {hash}{}",
                    if same_versions {
                        " (the same algorithm versions: its bits changed without a \
                         new version)"
                    } else {
                        ""
                    },
                    status = Status::Mismatch
                ),
            ),
            None => (
                false,
                format!(
                    "{status} {name} {format}: no reference hash (run `cargo xtask bless`)",
                    status = Status::Missing
                ),
            ),
        },
        Err(reason) => (
            false,
            format!(
                "{status} {name} {format}: {reason}",
                status = Status::Failed
            ),
        ),
    }
}

/// `check <dir> [--spent <record>]`: renders every case and reports each one on `out`, a
/// case that fails to render included, so that one run lists every problem; with `spent`,
/// the record on `main`, also each version spent there given other bits.
///
/// `bless` and `export`, by contrast, stop at the first failing case: a partial set of
/// references or renders would be wrong.
fn check(
    files: &mut dyn Files,
    out: &mut dyn Write,
    dir: &Path,
    spent: Option<&Path>,
    mode: DefaultMode<'_>,
) -> Result<(), String> {
    let cases = cases(files, dir)?;
    let expected: References = read_blessed(files, &references_path(dir))?;
    let record: Record = read_blessed(files, &algorithms_path(dir))?;
    let base = spent
        .map(|path| read_spent(files, path, "cargo xtask check"))
        .transpose()?;
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
            let (passed, line) = format_line(case, format, references, same_versions, mode);
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
                format!(
                    "{status} {name} {unknown}: reference of no render format",
                    status = Status::Stale
                ),
            )?;
        }
        for line in version_problems(name, current.as_ref(), recorded, references.is_some()) {
            report(out, false, line)?;
        }
    }
    for line in record_problems(&record, &current_fingerprints(mode))
        .into_iter()
        .chain(spent_problems(&record, base.flatten().as_ref()))
    {
        report(out, false, line)?;
    }
    for name in expected
        .keys()
        .filter(|name| !cases.iter().any(|case| case.name == **name))
    {
        report(
            out,
            false,
            format!(
                "{status} {name}: reference without a case",
                status = Status::Stale
            ),
        )?;
    }
    for name in blessed
        .keys()
        .filter(|name| !cases.iter().any(|case| case.name == **name))
    {
        report(
            out,
            false,
            format!(
                "{status} {name}: algorithm versions without a case",
                status = Status::Stale
            ),
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

/// The arguments `bless` expects after its command.
const BLESS: &str =
    "<dir> [--case-changed <name>]... [--spent <record>] [--battery-changed] [--from-nothing]";

/// The options of `bless`, as given on its command line.
#[derive(Debug, Default, PartialEq, Eq)]
struct BlessOptions {
    /// The cases whose references changed on purpose (`--case-changed`, repeatable).
    changed: Vec<String>,
    /// The record at the merge base (`--spent`, at most once): the versions spent.
    spent: Option<PathBuf>,
    /// `--battery-changed`: the fingerprint battery changed, nothing else.
    battery_changed: bool,
    /// `--from-nothing`: a first bless, with neither file to compare with.
    from_nothing: bool,
}

impl BlessOptions {
    /// Reads `options`, every one at most once but `--case-changed`.
    ///
    /// # Errors
    ///
    /// Returns the option that is unknown or repeated, or `None` when one lacks its value.
    fn parse(options: &[String]) -> Result<Self, Option<String>> {
        let mut parsed = Self::default();
        let mut rest = options.iter();
        while let Some(option) = rest.next() {
            let once = |given: bool| {
                if given {
                    Err(Some(format!("option \"{option}\" given twice")))
                } else {
                    Ok(true)
                }
            };
            match option.as_str() {
                "--case-changed" => parsed.changed.push(rest.next().ok_or(None)?.clone()),
                "--spent" => {
                    let _once = once(parsed.spent.is_some())?;
                    parsed.spent = Some(PathBuf::from(rest.next().ok_or(None)?));
                }
                "--battery-changed" => parsed.battery_changed = once(parsed.battery_changed)?,
                "--from-nothing" => parsed.from_nothing = once(parsed.from_nothing)?,
                _ => return Err(Some(format!("unknown option \"{option}\""))),
            }
        }
        Ok(parsed)
    }
}

/// `bless <dir> [options]`: the reference hash of every case in every render format, cases
/// and the names of the formats sorted, and the record of what they were blessed under
/// (`spec/engine-design.md`, decision 29).
///
/// Refuses, writing nothing and naming each problem on `out`, what [`bless_problems`]
/// refuses against the files it replaces, which must both exist; with `--from-nothing`,
/// neither may, and the directory is blessed whole.
fn bless(
    files: &mut dyn Files,
    out: &mut dyn Write,
    dir: &Path,
    options: &BlessOptions,
    mode: DefaultMode<'_>,
) -> Result<(), String> {
    // One reading of the cases gives both the versions and the hashes. The versions first:
    // a patch that cannot be read stops the bless before any render.
    let cases = cases(files, dir)?;
    let versions: BTreeMap<String, Versions> = cases
        .iter()
        .map(|case| algorithms(case).map(|current| (case.name.clone(), current)))
        .collect::<Result<_, _>>()?;
    let actual = hashes(&cases, mode)?;
    let path = references_path(dir);
    let versions_path = algorithms_path(dir);
    let earlier: Option<References> = read_earlier(files, &path)?;
    let old: Option<Record> = read_earlier(files, &versions_path)?;
    let base: Option<Option<Record>> = options
        .spent
        .as_ref()
        .map(|spent| read_spent(files, spent, "cargo xtask bless"))
        .transpose()?;
    let now = current_fingerprints(mode);
    let told = Told {
        changed: &options.changed,
        battery_changed: options.battery_changed,
    };
    let blessing = Blessing {
        fingerprints: &now,
        cases: &versions,
        references: &actual,
    };
    let missing = |what: &Path, other: &Path| {
        format!(
            "{status} {}: missing beside {} (restore {} from git: bless compares with both)",
            what.display(),
            other.display(),
            what.display(),
            status = Status::Missing
        )
    };
    let problems = match (&old, &earlier, options.from_nothing) {
        (Some(record), Some(references), false) => bless_problems(
            &Before {
                record,
                references,
                spent: base
                    .as_ref()
                    .map_or(Spent::Replaced, |base| Spent::Base(base.as_ref())),
            },
            &blessing,
            &told,
        ),
        (None, None, true) => unknown_cases(&told, &actual),
        (None, None, false) => vec![format!(
            "{status} {} and {}: none to compare with (a first bless: `cargo xtask bless \
             --from-nothing`)",
            path.display(),
            versions_path.display(),
            status = Status::Missing
        )],
        (Some(_), Some(_), true) => vec![format!(
            "{status} --from-nothing: {} and {} exist, to compare with (run `cargo xtask \
             bless` without it)",
            path.display(),
            versions_path.display(),
            status = Status::Option
        )],
        (Some(_), None, _) => vec![missing(&path, &versions_path)],
        (None, Some(_), _) => vec![missing(&versions_path, &path)],
    };
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
            let (written, unwritten) = (path.display(), versions_path.display());
            // A first bless that wrote only the references leaves a directory that neither
            // bless accepts: the way out is to remove what it wrote. Otherwise the record
            // it replaces is compared with again.
            let hint = if options.from_nothing {
                format!("remove {written}, then run `cargo xtask bless --from-nothing`")
            } else {
                "run `cargo xtask bless` again".to_owned()
            };
            format!("cannot write {unwritten}: {error} ({written} was written: {hint})")
        })?;
    say(
        out,
        &OneLine::new(&format!(
            "blessed {} case(s) in {} and {}",
            actual.len(),
            path.display(),
            versions_path.display()
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
/// Returns why it cannot be read or is not as `bless` writes it, with what succeeds in
/// that state: restoring it from git when it is missing or invalid (bless compares with
/// it), or for a new directory a first bless; rewriting it by bless when it is valid but
/// not in its written form.
fn read_blessed<T: Serialize + DeserializeOwned>(
    files: &mut dyn Files,
    path: &Path,
) -> Result<T, String> {
    let shown = path.display();
    let text = files.read(path, MAX_REFERENCES_BYTES).map_err(|error| {
        let hint = if error.kind() == std::io::ErrorKind::NotFound {
            format!(
                " (restore {shown} from git; a new directory: `cargo xtask bless \
                 --from-nothing`)"
            )
        } else {
            String::new()
        };
        format!("cannot read {shown}: {error}{hint}")
    })?;
    let value: T = serde_json::from_str(&text)
        .map_err(|error| format!("invalid {shown}: {error} (restore {shown} from git)"))?;
    if blessed_text(&value) == text {
        Ok(value)
    } else {
        Err(format!(
            "{shown} is not as bless writes it (run `cargo xtask bless`)"
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
                "invalid {}: {error} (bless compares with it: restore {} from git)",
                path.display(),
                path.display()
            )
        }),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(format!("cannot read {}: {error}", path.display())),
    }
}

/// Reads `path`, the record spent (`--spent`): the record on `main`, or `null` when `main`
/// has none, as `writer`, the command of `cargo xtask` that runs this one, writes it first.
///
/// # Errors
///
/// Returns why it cannot be read or is not a record or `null`, naming `writer`.
fn read_spent(files: &mut dyn Files, path: &Path, writer: &str) -> Result<Option<Record>, String> {
    let shown = path.display();
    let hint = format!("(`{writer}` writes it: run that)");
    let text = files
        .read(path, MAX_REFERENCES_BYTES)
        .map_err(|error| format!("cannot read the spent record {shown}: {error} {hint}"))?;
    serde_json::from_str(&text)
        .map_err(|error| format!("invalid spent record {shown}: {error} {hint}"))
}

/// `export <dir> <out>`: the WAV file of every case, for tools that check renders
/// independently of this engine. The corpus is listed and read whole before `out` is
/// created, so that a corpus missing, empty, unpaired or unreadable leaves nothing behind;
/// a case refused when it is prepared (an invalid case file or patch) leaves `out` and the
/// WAV files of the cases before it.
fn export(
    files: &mut dyn Files,
    out: &mut dyn Write,
    dir: &Path,
    target: &Path,
    mode: DefaultMode<'_>,
) -> Result<(), String> {
    let cases = cases(files, dir)?;
    files
        .create_dir_all(target)
        .map_err(|error| format!("cannot create {}: {error}", target.display()))?;
    for case in cases {
        for format in RenderFormat::ALL {
            let rendered = case.prepare(format, mode)?.render(mode);
            let path = target.join(format!("{}.{format}.wav", case.name));
            files
                .write(&path, &rendered.wav())
                .map_err(|error| format!("cannot write {}: {error}", path.display()))?;
            let hash = hash_of(&rendered)?;
            say(
                out,
                &OneLine::new(&format!(
                    "{status} {} {hash}",
                    path.display(),
                    status = Status::Wrote
                )),
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
        ("check", [dir]) => Ok(check(files, out, Path::new(dir), None, mode)?),
        ("check", [dir, option, spent]) if option == "--spent" => Ok(check(
            files,
            out,
            Path::new(dir),
            Some(Path::new(spent)),
            mode,
        )?),
        ("bless", [dir, options @ ..]) => {
            let options = BlessOptions::parse(options).map_err(|problem| {
                CliError::Usage(problem.unwrap_or_else(|| format!("{command} expects {BLESS}")))
            })?;
            Ok(bless(files, out, Path::new(dir), &options, mode)?)
        }
        ("export", [dir, target]) => {
            Ok(export(files, out, Path::new(dir), Path::new(target), mode)?)
        }
        ("help", []) => Ok(say_usage(out)?),
        ("check", _) => Err(expects("<dir> [--spent <record>]")),
        ("bless", _) => Err(expects(BLESS)),
        ("export", _) => Err(expects("<dir> <out>")),
        ("help", _) => Err(expects("no argument")),
        _ => Err(CliError::Usage(format!("unknown command \"{command}\""))),
    }
}
