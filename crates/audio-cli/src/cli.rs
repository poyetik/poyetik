//! The `audio-cli` commands.

use crate::MAX_FRAMES;
use crate::hint::{Hint, said};
use crate::status::Status;
use alloc::collections::BTreeMap;
use std::ffi::OsString;
use std::io::Write;
use std::path::{Path, PathBuf};

use audio_battery::FINGERPRINT_BATTERY;
use audio_core::{RenderFormat, SampleRate};
use audio_patch::MAX_PATCH_BYTES;
use fpenv::{DefaultMode, with_default_mode};

use audio_boxes::REGISTRY;

use crate::case::{Registry, prepare_in};
use crate::record::{
    Before, Blessing, LIVE_VECTOR, LiveHashes, Others, Record, References, Spent, Told, Versions,
    bless_problems, blessed, current_fingerprints, first_bless_problems, record_problems,
    same_rules, spent_problems, spent_skipped,
};
use crate::{Files, Prepared, Rendered};
use audio_hash::RenderHash;
use serde::Serialize;
use serde::de::DeserializeOwned;

/// Usage of the `audio-cli` command, printed on stdout by `audio-cli help`.
pub const USAGE: &str = "\
usage: audio-cli <command>
  render <case.json> <patch.json> [--format <format>] [--wav <file.wav>]
                      render a case in <format> (binary64, the default,
                      binary64-as-binary32 or binary32), print its hash,
                      optionally write a WAV file; warns on stderr of
                      non-finite samples (NaN or infinite), each rendered as
                      zero, the exit code unchanged
  check <dir> [--spent <record>]
                      compare every case of <dir>/cases (patches in <dir>/patches),
                      in every format, with <dir>/references.json, and
                      the algorithm versions of its box types, with
                      <dir>/algorithms.json, whose fingerprints must be those of
                      <record>, the spent record, at the versions both hold;
                      fails a case whose render replaced a non-finite sample,
                      even when its hash matches its reference; and, when
                      <dir>/live-limiter.json is present (the live vector of
                      the limiter's settings), its render in every format with
                      <dir>/live-references.json, which fails alone
  bless <dir> [--case-changed <name>]... [--live-changed]
        [--spent <record>] [--battery-changed] [--from-nothing]
                      rewrite <dir>/references.json from the current renders,
                      and <dir>/algorithms.json from the current box types and
                      engine; refuses a reference changed under the same
                      versions but for each case named, a lower version, a
                      box type's outputs changed at a spent version (those of
                      <record>, the spent record, which `cargo xtask bless`
                      writes: a file holding null when main has none; else
                      those of <dir>/algorithms.json), another battery
                      but with --battery-changed (then no reference, version,
                      render rules or render hash may change), and a
                      directory missing either file but with --from-nothing
                      (neither file then), and a case whose render replaced a
                      non-finite sample; and rewrites <dir>/live-references.json
                      from the live vector's renders, when it is present,
                      refusing a hash changed but with --live-changed, and
                      it alone
  export <dir> <out>  write the WAV file of every case of <dir>/cases, in every
                      format, to <out>/<case>.<format>.wav; refuses each render
                      that replaced a non-finite sample, naming it on its line,
                      writes every other one, then fails counting them; a case
                      that cannot be prepared stops it, leaving the files
                      written before it; files already in <out> stay, an
                      earlier file of a render it refuses among them
  help                print this help";

/// What a usage error ends with: where the usage is.
const USAGE_HINT: &str = " (see: audio-cli help)";

/// Why a command did not succeed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CliError {
    /// The arguments do not form a command, and why: shown as one `error:` line that
    /// points to `audio-cli help`, exit code 2.
    Usage(String),
    /// The command ran and failed: shown as one `error:` line, exit code 1.
    Failed(String),
}

impl CliError {
    /// The error as the one line the tool prints on stderr, without its newline:
    /// `error: ` and the message made safe by [`audio_patch::safe_message`] (every
    /// character that could break the line or drive the terminal escaped, at most
    /// [`audio_patch::MAX_MESSAGE_CHARS`] characters), then, for a usage error, its
    /// pointer to the usage, ` (see: audio-cli help)`, appended after the reason is
    /// bounded, so that no reason can cut it.
    ///
    /// Messages quote untrusted text (box and field names from a patch): this is where
    /// the error line is made safe, whatever produced it; every line on stdout is made
    /// safe the same way, as a `OneLine`.
    #[must_use]
    pub fn line(&self) -> String {
        match self {
            Self::Usage(reason) => {
                format!("error: {}{USAGE_HINT}", audio_patch::safe_message(reason))
            }
            Self::Failed(message) => format!("error: {}", audio_patch::safe_message(message)),
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

/// Largest `references.json` read, in bytes (1 MiB).
pub const MAX_REFERENCES_BYTES: usize = 1024 * 1024;

/// Largest record read, in bytes (1 MiB): `algorithms.json`, and the spent record, an
/// `algorithms.json` read from git.
const MAX_RECORD_BYTES: usize = 1024 * 1024;

/// Reads a file of at most `limit` bytes, naming it on failure.
fn read(files: &mut dyn Files, path: &Path, limit: usize) -> Result<String, String> {
    files
        .read(path, limit)
        .map_err(|error| format!("cannot read {}: {error}", path.display()))
}

/// One line of stdout, made safe: its only constructor passes the whole text through
/// [`audio_patch::safe_message`], so that untrusted parts (case names from file names,
/// box types and field names quoted by an error, the keys and values of
/// `references.json`, paths) can neither drive the terminal nor forge a second line, and
/// the line holds at most [`audio_patch::MAX_MESSAGE_CHARS`] characters.
mod one_line {
    /// A line of text with no character that could break it or drive a terminal.
    #[derive(Debug)]
    pub(super) struct OneLine(String);

    impl OneLine {
        /// `text` made safe to print as one line.
        pub(super) fn new(text: &str) -> Self {
            Self(audio_patch::safe_message(text))
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
    /// The box types its patch may use.
    registry: Registry,
}

impl CaseFiles {
    /// Prepares the case in `format`, in the default floating-point mode that `mode`
    /// proves, naming the file at fault on failure.
    fn prepare(&self, format: RenderFormat, mode: DefaultMode<'_>) -> Result<Prepared, String> {
        prepare_in(&self.case, &self.patch, format, self.registry, mode)
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
    let patch = audio_patch::parse_patch(&case.patch)
        .map_err(|error| format!("{}: {error}", case.patch_path.display()))?;
    patch
        .nodes()
        .map(|node| {
            case.registry
                .iter()
                .find(|spec| spec.name() == node.kind)
                .map(|spec| (node.kind.clone(), spec.algorithm_version().get()))
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

/// What a case name may hold, as an error says: so that a hint can paste it into a command
/// unquoted, in any shell (`conformance/README.md`).
const CASE_NAME: &str =
    "lowercase ASCII letters, digits, dots and hyphens, starting with a letter or a digit";

/// Whether `name` is a case name ([`CASE_NAME`]).
fn case_name(name: &str) -> bool {
    let mut bytes = name.bytes();
    let word = |byte: u8| byte.is_ascii_lowercase() || byte.is_ascii_digit();
    bytes.next().is_some_and(word) && bytes.all(|byte| word(byte) || byte == b'.' || byte == b'-')
}

/// The names of the `.json` files of `dir`, sorted, with their paths: each a case name
/// ([`CASE_NAME`]), refused where it is read.
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
                .filter(|name| case_name(name))
                .map(|name| (name.to_owned(), path.clone()))
                .ok_or_else(|| format!("{}: not a case name ({CASE_NAME})", path.display()))
        })
        .collect()
}

/// Reads every case of `<dir>/cases` with its patch `<dir>/patches/<name>.json`, sorted
/// by name, each to be built from the box types of `registry`. Every case needs its patch,
/// and every patch its case.
fn cases(files: &mut dyn Files, dir: &Path, registry: Registry) -> Result<Vec<CaseFiles>, String> {
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
            registry,
        });
    }
    Ok(read_cases)
}

/// What the output stage replaced in `rendered`, in words, or `None` when it replaced
/// nothing (`spec/engine-design.md`, decision 12).
pub(crate) fn replaced_words(rendered: &Rendered) -> Option<String> {
    let replaced = rendered.replaced();
    let first = replaced.first()?;
    // One choice for every word the count agrees with: the noun, and "the first" of more
    // than one.
    let (samples, at) = if replaced.count() == 1 {
        ("1 non-finite sample".to_owned(), "at")
    } else {
        (
            format!("{} non-finite samples", replaced.count()),
            "the first at",
        )
    };
    Some(format!(
        "{samples} (NaN or infinite) rendered as zero, {at} frame {} of channel {} (both \
         counted from 0)",
        first.frame, first.channel
    ))
}

/// Writes one `warning:` line to `warnings` when the output stage replaced samples in
/// `rendered`, a render of the case file `case` with the patch file `patch`, both named, with
/// its cause, a box of that patch (the refusals of [`conformant`] name it their own way): a
/// warning, never a failure, the render going on (`spec/engine-design.md`, decision 12); a
/// closed stderr changes nothing.
pub(crate) fn warn_replaced(
    warnings: &mut dyn Write,
    (case, patch): (&str, &str),
    rendered: &Rendered,
) {
    if let Some(words) = replaced_words(rendered) {
        drop(writeln!(
            warnings,
            "warning: {} with {}: {words}: from a box of this patch",
            OneLine::new(case).as_str(),
            OneLine::new(patch).as_str()
        ));
    }
}

/// `rendered`, a render of the conformance case `name` in `format`, when its output stage
/// replaced nothing: a non-finite sample is a defect of a box, which no reference may hold.
///
/// # Errors
///
/// Returns what was replaced as the line `bless` and `export` print for it, naming the case
/// and the format as `check` does: `FAILED <case> <format>: <what was replaced>`.
pub(crate) fn conformant<'rendered>(
    rendered: &'rendered Rendered,
    name: &str,
    format: RenderFormat,
) -> Result<&'rendered Rendered, String> {
    defect(rendered).map(|()| rendered).map_err(|defect| {
        format!(
            "{status} {} {format}: {defect}",
            OneLine::new(name).as_str(),
            status = Status::Failed
        )
    })
}

/// Nothing when the output stage replaced nothing in `rendered`, else what it replaced, in
/// words: the defect that fails a conformance case, in `bless`, `export` and `check` alike.
fn defect(rendered: &Rendered) -> Result<(), String> {
    replaced_words(rendered).map_or(Ok(()), |words| {
        Err(format!(
            "{words}: a defect of a box, which no conformance case may hold"
        ))
    })
}

/// The hash `check` compares for `rendered`, a render of a conformance case, in words; or
/// the defect that fails it, as [`conformant`] fails `bless` and `export`
/// (`spec/engine-design.md`, decision 12), the line of `check` naming the case and format.
///
/// # Errors
///
/// Returns what the output stage replaced, or why the render has no hash.
pub(crate) fn conformant_hash(rendered: &Rendered) -> Result<String, String> {
    defect(rendered)?;
    hash_of(rendered).map(|hash| hash.to_string())
}

/// The hash of `rendered`, or why it is not a render, in words.
pub(crate) fn hash_of(rendered: &Rendered) -> Result<RenderHash, String> {
    hashed(rendered.hash())
}

/// A render's hash, or why its samples are not a render, in words: the one wording of
/// [`hash_of`] and of the live vector's renders.
fn hashed(hash: Result<RenderHash, audio_hash::NotWholeFrames>) -> Result<RenderHash, String> {
    hash.map_err(|error| format!("cannot hash the render: {error}"))
}

/// Renders every case of `cases` in every render format: for each case, by name, its hash
/// in each format; or, when the output stage replaced samples in any render, the line of
/// every such render ([`conformant`]), every case rendered first.
///
/// # Errors
///
/// Returns why a case cannot be prepared or a render hashed, at the first.
fn hashes(
    cases: &[CaseFiles],
    mode: DefaultMode<'_>,
) -> Result<Result<References, Vec<String>>, String> {
    let mut hashes = BTreeMap::new();
    let mut defects = Vec::new();
    for case in cases {
        let mut formats = BTreeMap::new();
        for format in RenderFormat::ALL {
            let rendered = case.prepare(format, mode)?.render(mode);
            match conformant(&rendered, &case.name, format) {
                Ok(rendered) => {
                    drop(formats.insert(format.name().to_owned(), hash_of(rendered)?.to_string()));
                }
                Err(defect) => defects.push(defect),
            }
        }
        // Case names are file names of one directory, hence unique.
        drop(hashes.insert(case.name.clone(), formats));
    }
    Ok(if defects.is_empty() {
        Ok(hashes)
    } else {
        Err(defects)
    })
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
    (out, warnings): (&mut dyn Write, &mut dyn Write),
    args: &[String],
    (registry, mode): (Registry, DefaultMode<'_>),
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
    let prepared = prepare_in(
        &read(files, case_path, MAX_CASE_BYTES)?,
        &read(files, patch_path, MAX_PATCH_BYTES)?,
        format,
        registry,
        mode,
    )
    .map_err(|error| error.in_files(case_path, patch_path))?;
    let rendered = prepared.render(mode);
    warn_replaced(warnings, (case, patch), &rendered);
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
/// which a bless replaces only when the case is named. The lines of the box types the
/// patch does not use come apart, second: stale entries, which a bless changing the
/// battery drops. Each line gives its hint when `hints` ([`said`]).
fn version_problems(
    name: &str,
    current: Option<&Versions>,
    recorded: Option<&Versions>,
    referenced: bool,
    record: &Path,
    hints: bool,
) -> (Vec<String>, Vec<String>) {
    let Some(current) = current else {
        return (Vec::new(), Vec::new());
    };
    let Some(recorded) = recorded else {
        // A case with references but no versions is one bless refuses unless it is named
        // (`bless_problems`): the hint names it.
        let hint = if referenced {
            Hint::CaseChangedOrRestore { case: name, record }
        } else {
            Hint::NewCase
        };
        return (
            vec![format!(
                "{status} {name}: no algorithm versions{hint}",
                status = Status::Missing,
                hint = said(hint, hints)
            )],
            Vec::new(),
        );
    };
    let changed = current
        .iter()
        .filter_map(|(kind, version)| match recorded.get(kind) {
            Some(blessed) if blessed == version => None,
            Some(blessed) => Some(format!(
                "{status} {name} {kind}: at algorithm version {version}, the references were \
             blessed with version {blessed}{hint}",
                status = Status::Version,
                hint = said(Hint::NewVersion, hints)
            )),
            None => Some(format!(
                "{status} {name} {kind}: no algorithm version{hint}",
                status = Status::Missing,
                hint = said(Hint::UnversionedBox, hints)
            )),
        });
    let stale = recorded
        .keys()
        .filter(|kind| !current.contains_key(*kind))
        .map(|kind| {
            format!(
                "{status} {name} {kind}: algorithm version of a box type the case does not \
                 use{hint}",
                status = Status::Stale,
                hint = said(Hint::Stale, hints)
            )
        });
    (changed.collect(), stale.collect())
}

/// How the line of `check` for one case in one render format ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Verdict {
    /// The render is its reference.
    Passed,
    /// The render differs from its reference under the algorithm versions and render rules
    /// it was blessed with: a bless names the case, or a new version comes first.
    Changed,
    /// Any other failure.
    Failed,
}

/// The line `check` prints for `case` in `format`, and how it ended: its render against
/// its reference among `references`, `same_versions` saying whether the case's algorithm
/// versions and render rules are those its references were blessed with.
fn format_line(
    case: &CaseFiles,
    format: RenderFormat,
    references: Option<&BTreeMap<String, String>>,
    same_versions: bool,
    hints: bool,
    mode: DefaultMode<'_>,
) -> (Verdict, String) {
    let name = &case.name;
    match case
        .prepare(format, mode)
        .and_then(|prepared| conformant_hash(&prepared.render(mode)))
    {
        Ok(hash) => match references.and_then(|hashes| hashes.get(format.name())) {
            Some(reference) if *reference == hash => (
                Verdict::Passed,
                format!("{status} {name} {format} {hash}", status = Status::Ok),
            ),
            Some(reference) => (
                if same_versions {
                    Verdict::Changed
                } else {
                    Verdict::Failed
                },
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
                Verdict::Failed,
                format!(
                    "{status} {name} {format}: no reference hash{hint}",
                    status = Status::Missing,
                    hint = said(Hint::NoReference, hints)
                ),
            ),
        },
        Err(reason) => (
            Verdict::Failed,
            format!(
                "{status} {name} {format}: {reason}",
                status = Status::Failed
            ),
        ),
    }
}

/// The hashes of the live vector's render in each render format, beside it, which only
/// `bless` writes, a changed one only when told (`--live-changed`).
const LIVE_REFERENCES: &str = "live-references.json";

/// Largest live vector read, in bytes (64 KiB, as a case).
pub(crate) const MAX_LIVE_BYTES: usize = 65_536;

/// A limiter's settings in the live vector.
#[derive(Debug, Clone, Copy, serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct LiveSettings {
    /// The ceiling, in dBTP.
    ceiling: f64,
    /// The lookahead, in milliseconds.
    lookahead: f64,
    /// The release, in milliseconds.
    release: f64,
    /// The adaptivity of the release.
    adaptivity: f64,
}

impl LiveSettings {
    /// The limiter's settings, checked against their ranges.
    fn settings(self) -> Result<audio_core::LimiterSettings, String> {
        let value = <audio_core::Binary64 as audio_core::host::Raw>::from_raw;
        audio_core::LimiterSettings::new(
            value(self.ceiling),
            value(self.lookahead),
            value(self.release),
            value(self.adaptivity),
        )
        .map_err(|error| error.to_string())
    }
}

/// The output stage of the live vector: protected, its maximum lookahead, its detector by
/// name and its first settings.
#[derive(Debug, serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct LiveOutput {
    /// Its mode: `protected`, the one mode of the vector.
    mode: String,
    /// The maximum lookahead, in milliseconds.
    maximum_lookahead: f64,
    /// The detector, by name.
    detector: String,
    /// The limiter's first settings.
    limiter: LiveSettings,
}

/// A change of the live vector: the settings set before the block `block` of the default
/// engine block, counted from 0.
#[derive(Debug, serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct LiveChange {
    /// The block.
    block: u64,
    /// The settings set.
    limiter: LiveSettings,
}

/// The live vector, as its file holds it.
#[derive(Debug, serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct LiveVector {
    /// The name of its patch, in the directory's `patches/`.
    patch: String,
    /// Its sample rate, in hertz.
    sample_rate: u32,
    /// The frames rendered, whole blocks of the default engine block, at most
    /// [`MAX_FRAMES`] ([`live_blocks`]).
    frames: u64,
    /// Its output stage.
    output: LiveOutput,
    /// Its changes, in the order of their blocks, each before the end ([`live_blocks`]).
    changes: Vec<LiveChange>,
}

/// The blocks of the default engine block a live vector of `frames` frames renders, whose
/// changes are at the blocks `at`, in their order.
///
/// # Errors
///
/// Returns why not: frames that are not a positive multiple of the block or are above
/// [`MAX_FRAMES`] (the longest case), a change at or past the end, or after one at a later
/// block.
pub(crate) fn live_blocks(frames: u64, at: impl IntoIterator<Item = u64>) -> Result<u64, String> {
    let block = u64::try_from(audio_core::BLOCK_SIZE).unwrap_or(u64::MAX);
    let longest = u64::try_from(MAX_FRAMES).unwrap_or(u64::MAX);
    let blocks = frames.checked_div(block).unwrap_or(0);
    if blocks == 0 || !frames.is_multiple_of(block) {
        return Err(format!(
            "frames must be a positive multiple of the engine block, {block}"
        ));
    }
    if frames > longest {
        return Err(format!("frames must be at most {longest}, got {frames}"));
    }
    let mut earlier = 0;
    for change in at {
        if change >= blocks {
            return Err(format!(
                "a change at block {change}, past the end of its {blocks} blocks"
            ));
        }
        if change < earlier {
            return Err(format!(
                "a change at block {change} after one at block {earlier}, out of order"
            ));
        }
        earlier = change;
    }
    Ok(blocks)
}

/// The hash of the live vector of `dir` in each render format, rendered natively through the
/// safe engine (`audio_host::Engine`), as `@poyetik/audio-wasm` renders it through the
/// WebAssembly module: none when the directory holds no live vector.
///
/// # Errors
///
/// Returns why the vector or its patch cannot be read, is invalid, or does not render.
fn live_hashes(
    files: &mut dyn Files,
    dir: &Path,
    _mode: DefaultMode<'_>,
) -> Result<Option<LiveHashes>, String> {
    let path = dir.join(LIVE_VECTOR);
    let shown = path.display();
    let text = match files.read(&path, MAX_LIVE_BYTES) {
        Ok(text) => text,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(format!("cannot read {shown}: {error}")),
    };
    let invalid = |why: &dyn core::fmt::Display| format!("invalid {shown}: {why}");
    let vector: LiveVector = serde_json::from_str(&text).map_err(|error| invalid(&error))?;
    if !case_name(&vector.patch) {
        return Err(invalid(&format!(
            "the patch's name holds other than {CASE_NAME}"
        )));
    }
    let blocks = live_blocks(
        vector.frames,
        vector.changes.iter().map(|change| change.block),
    )
    .map_err(|why| invalid(&why))?;
    let detector = audio_core::Detector::ALL
        .into_iter()
        .find(|detector| detector.name() == vector.output.detector)
        .ok_or_else(|| invalid(&"a detector of no name the engine knows"))?;
    if vector.output.mode != "protected" {
        return Err(invalid(&"an output stage that is not protected"));
    }
    let stage = audio_host::Output::Protected(audio_host::LimiterStage::new(
        <audio_core::Binary64 as audio_core::host::Raw>::from_raw(vector.output.maximum_lookahead),
        detector,
        vector
            .output
            .limiter
            .settings()
            .map_err(|why| invalid(&why))?,
    ));
    let changes = vector
        .changes
        .iter()
        .map(|change| Ok((change.block, change.limiter.settings()?)))
        .collect::<Result<Vec<_>, String>>()
        .map_err(|why| invalid(&why))?;
    let patch_path = dir.join("patches").join(format!("{}.json", vector.patch));
    let patch_text = read(files, &patch_path, MAX_PATCH_BYTES)?;
    let rate = SampleRate::new(vector.sample_rate).map_err(|error| invalid(&error))?;
    let mut hashes = BTreeMap::new();
    for format in RenderFormat::ALL {
        let mut engine = audio_host::Engine::<{ audio_core::BLOCK_SIZE }>::new(
            patch_text.as_bytes(),
            vector.sample_rate,
            format,
            stage,
        )
        .map_err(|why| invalid(&why))?;
        let (mut wide, mut narrow) = (Vec::new(), Vec::new());
        for at in 0..blocks {
            for (_, settings) in changes.iter().filter(|(change, _)| *change == at) {
                engine
                    .set_limiter(*settings)
                    .map_err(|why| invalid(&format!("block {at}: {why}")))?;
            }
            match engine.process() {
                audio_host::Blocks::Binary64(channels) => {
                    for frame in 0..audio_core::BLOCK_SIZE {
                        wide.extend(channels.iter().filter_map(|channel| {
                            channel
                                .get(frame)
                                .map(|sample| audio_core::host::Raw::raw(*sample))
                        }));
                    }
                }
                audio_host::Blocks::Binary32(channels) => {
                    for frame in 0..audio_core::BLOCK_SIZE {
                        narrow.extend(channels.iter().filter_map(|channel| {
                            channel
                                .get(frame)
                                .map(|sample| audio_core::host::Raw::raw(*sample))
                        }));
                    }
                }
            }
        }
        let samples = match format {
            RenderFormat::Binary64 => audio_hash::Samples::Binary64(&wide),
            RenderFormat::Binary64AsBinary32 => audio_hash::Samples::Binary64AsBinary32(&narrow),
            RenderFormat::Binary32 => audio_hash::Samples::Binary32(&narrow),
        };
        let hash = hashed(audio_hash::render_hash(
            rate,
            engine.channels(),
            samples,
            audio_core::MODE,
        ))?;
        drop(hashes.insert(format.name().to_owned(), hash.to_string()));
    }
    Ok(Some(hashes))
}

/// The lines of `check` for the live vector's `hashes` against those `bless` wrote beside it
/// (`blessed`): each format's, then each blessed hash of no render format; and whether all
/// passed.
fn live_lines(hashes: &LiveHashes, blessed: &LiveHashes) -> Vec<(bool, String)> {
    let name = LIVE_VECTOR;
    let mut lines: Vec<(bool, String)> = hashes
        .iter()
        .map(|(format, hash)| match blessed.get(format) {
            Some(reference) if reference == hash => (
                true,
                format!("{status} {name} {format} {hash}", status = Status::Ok),
            ),
            Some(reference) => (
                false,
                format!(
                    "{status} {name} {format}: expected {reference}, got {hash}",
                    status = Status::Mismatch
                ),
            ),
            None => (
                false,
                format!(
                    "{status} {name} {format}: no reference hash",
                    status = Status::Missing
                ),
            ),
        })
        .collect();
    lines.extend(
        blessed
            .keys()
            .filter(|key| !hashes.contains_key(*key))
            .map(|key| {
                (
                    false,
                    format!(
                        "{status} {name} {key}: reference of no render format",
                        status = Status::Stale
                    ),
                )
            }),
    );
    lines
}

/// The line of `check` and `bless` for the live vector's hashes in `dir` beside no live
/// vector, which no command reads: stale, as a reference of no case is; none when `dir`
/// holds no such file.
///
/// # Errors
///
/// Returns why the file cannot be read, but that it does not exist.
fn orphan_live(files: &mut dyn Files, dir: &Path) -> Result<Option<String>, String> {
    let path = dir.join(LIVE_REFERENCES);
    match files.read(&path, MAX_REFERENCES_BYTES) {
        Ok(_) => Ok(Some(format!(
            "{status} {LIVE_REFERENCES}: the hashes of no live vector, {LIVE_VECTOR} ({hint})",
            status = Status::Stale,
            hint = Hint::RemoveLiveReferences { file: &path }
        ))),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(format!("cannot read {}: {error}", path.display())),
    }
}

/// The lines of `check` for the live vector of `dir` ([`live_lines`]), none when it holds
/// none, and whether a hash of it changed from the one `bless` wrote.
///
/// # Errors
///
/// Returns why the vector cannot be rendered ([`live_hashes`]), or why its hashes cannot be
/// read ([`read_blessed`]).
fn live_check(
    files: &mut dyn Files,
    dir: &Path,
    mode: DefaultMode<'_>,
) -> Result<(Vec<(bool, String)>, bool), String> {
    let Some(hashes) = live_hashes(files, dir, mode)? else {
        let orphan = orphan_live(files, dir)?;
        return Ok((
            orphan.into_iter().map(|line| (false, line)).collect(),
            false,
        ));
    };
    let blessed: LiveHashes =
        read_blessed(files, &dir.join(LIVE_REFERENCES), MAX_REFERENCES_BYTES)?;
    let changed = hashes
        .iter()
        .any(|(format, hash)| blessed.get(format).is_some_and(|earlier| earlier != hash));
    Ok((live_lines(&hashes, &blessed), changed))
}

/// `check <dir> [--spent <record>]`: renders every case and reports each one on `out`, a
/// case that fails to render included, so that one run lists every problem; with `spent`,
/// the spent record, also each version spent there given other bits.
///
/// `bless` and `export`, by contrast, stop at the first case that cannot be prepared:
/// `bless` writing nothing, since a partial set of references would be wrong, and `export`
/// because that case has no render, keeping the files it wrote before it. A render whose
/// output stage replaced a non-finite sample does not stop them: each such render is named
/// on its line, every other one rendered, then the command fails, `bless` writing nothing.
fn check(
    files: &mut dyn Files,
    out: &mut dyn Write,
    dir: &Path,
    spent: Option<&Path>,
    (registry, mode): (Registry, DefaultMode<'_>),
) -> Result<(), String> {
    let cases = cases(files, dir, registry)?;
    let expected: References = read_blessed(files, &references_path(dir), MAX_REFERENCES_BYTES)?;
    let record: Record = read_blessed(files, &algorithms_path(dir), MAX_RECORD_BYTES)?;
    let base = spent
        .map(|path| read_spent(files, path, "cargo xtask check"))
        .transpose()?;
    let blessed = &record.cases;
    let rules = same_rules(&record);
    // While the battery differs, every bless but the battery's is refused: no line but the
    // battery's gives a hint.
    let hints = record.fingerprint_battery == FINGERPRINT_BATTERY;
    let mut failures = 0_usize;
    // The lines of a case's stale entries, which a bless changing the battery drops.
    let mut stale = 0_usize;
    let mut changed = Vec::new();
    let report = |stdout: &mut dyn Write, count: &mut usize, passed: bool, line: String| {
        if !passed {
            *count = count.saturating_add(1);
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
            let (verdict, line) = format_line(case, format, references, same_versions, hints, mode);
            if verdict == Verdict::Changed && !changed.contains(name) {
                changed.push(name.clone());
            }
            report(out, &mut failures, verdict == Verdict::Passed, line)?;
        }
        for unknown in references
            .into_iter()
            .flat_map(BTreeMap::keys)
            .filter(|key| !RenderFormat::ALL.iter().any(|format| format.name() == *key))
        {
            report(
                out,
                &mut failures,
                false,
                format!(
                    "{status} {name} {unknown}: reference of no render format{hint}",
                    status = Status::Stale,
                    hint = said(Hint::Stale, hints)
                ),
            )?;
            stale = stale.saturating_add(1);
        }
        let (problems, unused) = version_problems(
            name,
            current.as_ref(),
            recorded,
            references.is_some(),
            &algorithms_path(dir),
            hints,
        );
        for line in problems {
            report(out, &mut failures, false, line)?;
        }
        for line in unused {
            report(out, &mut failures, false, line)?;
            stale = stale.saturating_add(1);
        }
    }
    // The live vector, when the directory holds one, against the hashes bless wrote; a
    // changed hash is one the last bless of a battery change blessed alone names.
    let (lines, live_changed) = live_check(files, dir, mode)?;
    for (passed, line) in lines {
        report(out, &mut failures, passed, line)?;
    }
    let base = base.flatten();
    // A problem of a case but a stale entry: a battery change cannot be blessed with it.
    let others = Others {
        failed: failures > stale,
        changed: &changed,
        live_changed,
    };
    for line in record_problems(&record, &current_fingerprints(mode), others)
        .into_iter()
        .chain(spent_problems(&record, base.as_ref()))
    {
        report(out, &mut failures, false, line)?;
    }
    if let Some(line) = spent_skipped(&record, base.as_ref()) {
        report(out, &mut failures, true, line)?;
    }
    for line in without_a_case(&expected, blessed, &cases, hints) {
        report(out, &mut failures, false, line)?;
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

/// The lines of `check` for what `expected`, the references, and `blessed`, the versions of
/// the record, hold of no case of `cases`, which a bless drops, each with its hint when
/// `hints` ([`said`]).
fn without_a_case(
    expected: &References,
    blessed: &BTreeMap<String, Versions>,
    cases: &[CaseFiles],
    hints: bool,
) -> Vec<String> {
    let stale = |name: &&String| !cases.iter().any(|case| case.name == **name);
    let references = expected.keys().filter(stale).map(|name| {
        format!(
            "{status} {name}: reference without a case{hint}",
            status = Status::Stale,
            hint = said(Hint::Stale, hints)
        )
    });
    let versions = blessed.keys().filter(stale).map(|name| {
        format!(
            "{status} {name}: algorithm versions without a case{hint}",
            status = Status::Stale,
            hint = said(Hint::Stale, hints)
        )
    });
    references.chain(versions).collect()
}

/// The arguments `bless` expects after its command.
const BLESS: &str = "<dir> [--case-changed <name>]... [--live-changed] [--spent <record>] \
                     [--battery-changed] [--from-nothing]";

/// The options of `bless`, as given on its command line.
#[derive(Debug, Default, PartialEq, Eq)]
struct BlessOptions {
    /// The cases whose references changed on purpose (`--case-changed`, repeatable).
    changed: Vec<String>,
    /// `--live-changed`: the live vector's hashes changed on purpose.
    live_changed: bool,
    /// The record at the merge base (`--spent`, at most once): the versions spent.
    spent: Option<PathBuf>,
    /// `--battery-changed`: the fingerprint battery changed, nothing else.
    battery_changed: bool,
    /// `--from-nothing`: a first bless, with neither file to compare with.
    from_nothing: bool,
}

/// The options of `bless`.
const BLESS_OPTIONS: [&str; 5] = [
    "--case-changed",
    "--live-changed",
    "--spent",
    "--battery-changed",
    "--from-nothing",
];

impl BlessOptions {
    /// Reads `options`, every one at most once but `--case-changed`, each value one that
    /// is not an option (`--case-changed --battery-changed` names no case).
    ///
    /// # Errors
    ///
    /// Returns the option that is unknown, repeated, or lacks its value.
    fn parse(options: &[String]) -> Result<Self, String> {
        let mut parsed = Self::default();
        let mut rest = options.iter();
        while let Some(option) = rest.next() {
            let once = |given: bool| {
                if given {
                    Err(format!("option \"{option}\" given twice"))
                } else {
                    Ok(true)
                }
            };
            let mut value = || {
                rest.next()
                    .filter(|value| !BLESS_OPTIONS.contains(&value.as_str()))
                    .ok_or_else(|| format!("option \"{option}\" needs a value"))
            };
            match option.as_str() {
                "--case-changed" => parsed.changed.push(value()?.clone()),
                "--spent" => {
                    let _once = once(parsed.spent.is_some())?;
                    parsed.spent = Some(PathBuf::from(value()?));
                }
                "--live-changed" => parsed.live_changed = once(parsed.live_changed)?,
                "--battery-changed" => parsed.battery_changed = once(parsed.battery_changed)?,
                "--from-nothing" => parsed.from_nothing = once(parsed.from_nothing)?,
                _ => return Err(format!("unknown option \"{option}\"")),
            }
        }
        Ok(parsed)
    }
}

/// Refuses a bless of `dir`: each of `problems` on its line of `out`, then the error naming
/// how many, nothing written.
///
/// # Errors
///
/// Always: the refusal, or why `out` cannot be written.
fn refused(out: &mut dyn Write, dir: &Path, problems: &[String]) -> Result<(), String> {
    for problem in problems {
        say(out, &OneLine::new(problem))?;
    }
    Err(format!(
        "bless refused in {}: {} problem(s), nothing written",
        dir.display(),
        problems.len()
    ))
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
    (registry, mode): (Registry, DefaultMode<'_>),
) -> Result<(), String> {
    // One reading of the cases gives both the versions and the hashes. The versions first:
    // a patch that cannot be read stops the bless before any render.
    let cases = cases(files, dir, registry)?;
    let versions: BTreeMap<String, Versions> = cases
        .iter()
        .map(|case| algorithms(case).map(|current| (case.name.clone(), current)))
        .collect::<Result<_, _>>()?;
    let actual = match hashes(&cases, mode)? {
        Ok(actual) => actual,
        Err(defects) => return refused(out, dir, &defects),
    };
    // Rendered before anything is written: a live vector that does not render stops it.
    let (live, earlier_live) = live_and_earlier(files, dir, mode)?;
    let path = references_path(dir);
    let versions_path = algorithms_path(dir);
    let earlier: Option<References> = read_earlier(files, &path, MAX_REFERENCES_BYTES)?;
    let old: Option<Record> = read_earlier(files, &versions_path, MAX_RECORD_BYTES)?;
    let base: Option<Option<Record>> = options
        .spent
        .as_ref()
        .map(|spent| read_spent(files, spent, "cargo xtask bless"))
        .transpose()?;
    let now = current_fingerprints(mode);
    let told = Told {
        changed: &options.changed,
        live_changed: options.live_changed,
        battery_changed: options.battery_changed,
        from_nothing: options.from_nothing,
    };
    let blessing = Blessing {
        fingerprints: &now,
        cases: &versions,
        references: &actual,
        live: live.as_ref(),
    };
    let mut problems = match (&old, &earlier, options.from_nothing) {
        (Some(record), Some(references), false) => bless_problems(
            &Before {
                record,
                path: &versions_path,
                references,
                live: earlier_live.as_ref(),
                spent: base
                    .as_ref()
                    .map_or(Spent::Replaced, |base| Spent::Base(base.as_ref())),
            },
            &blessing,
            &told,
        ),
        (None, None, true) => first_bless_problems(earlier_live.as_ref(), &blessing, &told),
        (None, None, false) => vec![format!(
            "{status} {} and {}: none to compare with ({hint})",
            path.display(),
            versions_path.display(),
            status = Status::Missing,
            hint = Hint::FirstBless
        )],
        (Some(_), Some(_), true) => vec![format!(
            "{status} --from-nothing: {} and {} exist, to compare with ({hint})",
            path.display(),
            versions_path.display(),
            status = Status::Option,
            hint = Hint::WithoutFromNothing
        )],
        (Some(_), None, _) => vec![missing_beside(&path, &versions_path)],
        (None, Some(_), _) => vec![missing_beside(&versions_path, &path)],
    };
    if live.is_none() {
        problems.extend(orphan_live(files, dir)?);
    }
    if !problems.is_empty() {
        return refused(out, dir, &problems);
    }
    let record = blessed(
        versions,
        &now,
        old.as_ref(),
        base.as_ref().and_then(Option::as_ref),
    );
    files
        .write(&path, blessed_text(&actual).as_bytes())
        .map_err(|error| format!("cannot write {}: {error}", path.display()))?;
    // References first: if the record cannot be written after them, `check` names the
    // versions to bless again, never a reference changed without a new version.
    files
        .write(&versions_path, blessed_text(&record).as_bytes())
        .map_err(|error| unwritten_record(&error, &path, &versions_path, options))?;
    say(
        out,
        &OneLine::new(&format!(
            "blessed {} case(s) in {} and {}",
            actual.len(),
            path.display(),
            versions_path.display()
        )),
    )?;
    live.map_or(Ok(()), |live| write_live(files, out, dir, &live))
}

/// The hashes of the live vector of `dir` ([`live_hashes`]), and, when it holds one, those
/// `bless` wrote beside it that it would replace, none when there are none yet.
///
/// # Errors
///
/// Returns why the vector cannot be rendered, or why its hashes cannot be read or are
/// invalid ([`read_earlier`]).
fn live_and_earlier(
    files: &mut dyn Files,
    dir: &Path,
    mode: DefaultMode<'_>,
) -> Result<(Option<LiveHashes>, Option<LiveHashes>), String> {
    let live = live_hashes(files, dir, mode)?;
    let earlier = if live.is_some() {
        read_earlier(files, &dir.join(LIVE_REFERENCES), MAX_REFERENCES_BYTES)?
    } else {
        None
    };
    Ok((live, earlier))
}

/// Writes `live`, the hashes of the live vector of `dir`, beside it, and says so on `out`.
///
/// # Errors
///
/// Returns why the file or `out` cannot be written.
fn write_live(
    files: &mut dyn Files,
    out: &mut dyn Write,
    dir: &Path,
    live: &LiveHashes,
) -> Result<(), String> {
    let live_path = dir.join(LIVE_REFERENCES);
    files
        .write(&live_path, blessed_text(live).as_bytes())
        .map_err(|error| format!("cannot write {}: {error}", live_path.display()))?;
    say(
        out,
        &OneLine::new(&format!(
            "blessed the live vector {} in {}",
            dir.join(LIVE_VECTOR).display(),
            live_path.display()
        )),
    )
}

/// Why `bless` failed when it could write the references at `written` and not the record at
/// `unwritten`, because of `error`, with what succeeds then, as `options` told it.
fn unwritten_record(
    error: &std::io::Error,
    written: &Path,
    unwritten: &Path,
    options: &BlessOptions,
) -> String {
    // A first bless that wrote only the references leaves a directory that neither bless
    // accepts: the way out is to remove what it wrote. Otherwise the record it replaces is
    // compared with again, by the same command.
    let hint = if options.from_nothing {
        Hint::RemoveThenFirstBless { written }
    } else {
        Hint::BlessAgain {
            changed: &options.changed,
            live_changed: options.live_changed,
            battery_changed: options.battery_changed,
        }
    };
    format!(
        "cannot write {}: {error} ({} was written: {hint})",
        unwritten.display(),
        written.display()
    )
}

/// The line of `bless` for a directory holding `other` without `what`: it compares with
/// both.
fn missing_beside(what: &Path, other: &Path) -> String {
    format!(
        "{status} {}: missing beside {} ({hint})",
        what.display(),
        other.display(),
        status = Status::Missing,
        hint = Hint::RestoreBeside { file: what }
    )
}

/// The text `bless` writes for `value`: JSON indented by two spaces, keys sorted, and a
/// final newline. The files it writes are read back only in exactly this form.
fn blessed_text<T: Serialize>(value: &T) -> String {
    format!("{:#}\n", serde_json::json!(value))
}

/// Reads `path`, a file only `bless` writes, of at most `limit` bytes, refusing it unless
/// it is exactly as `bless` writes it (a repeated key or a hand edit included): its
/// contents decide what `check` accepts.
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
    limit: usize,
) -> Result<T, String> {
    let shown = path.display();
    let text = files.read(path, limit).map_err(|error| {
        let hint = if error.kind() == std::io::ErrorKind::NotFound {
            format!(" ({})", Hint::RestoreOrFirstBless { file: path })
        } else {
            String::new()
        };
        format!("cannot read {shown}: {error}{hint}")
    })?;
    let value: T = serde_json::from_str(&text).map_err(|error| {
        format!(
            "invalid {shown}: {error} ({})",
            Hint::RestoreInvalid { file: path }
        )
    })?;
    if blessed_text(&value) == text {
        Ok(value)
    } else {
        Err(format!(
            "{shown} is not as bless writes it ({})",
            Hint::Rewrite
        ))
    }
}

/// Reads `path`, a file `bless` is about to replace, of at most `limit` bytes: none when it
/// is missing (a first bless), else its contents, in any JSON form.
///
/// # Errors
///
/// Returns why it cannot be read, or is not valid: what it holds could not be compared.
fn read_earlier<T: DeserializeOwned>(
    files: &mut dyn Files,
    path: &Path,
    limit: usize,
) -> Result<Option<T>, String> {
    match files.read(path, limit) {
        Ok(text) => serde_json::from_str(&text).map(Some).map_err(|error| {
            format!(
                "invalid {}: {error} ({})",
                path.display(),
                Hint::RestoreCompared { file: path }
            )
        }),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(format!("cannot read {}: {error}", path.display())),
    }
}

/// Reads `path`, the spent record (`--spent`): `algorithms.json` as `main` holds it, or
/// `null` when `main` has none, as `writer`, the command of `cargo xtask` that runs this
/// one, writes it first.
///
/// # Errors
///
/// Returns why it cannot be read or is not a record or `null`, naming `writer`.
fn read_spent(files: &mut dyn Files, path: &Path, writer: &str) -> Result<Option<Record>, String> {
    let shown = path.display();
    let hint = format!("({})", Hint::WriteSpent { writer });
    let text = files
        .read(path, MAX_RECORD_BYTES)
        .map_err(|error| format!("cannot read the spent record {shown}: {error} {hint}"))?;
    serde_json::from_str(&text)
        .map_err(|error| format!("invalid spent record {shown}: {error} {hint}"))
}

/// `export <dir> <out>`: the WAV file of every case, for tools that check renders
/// independently of this engine. The corpus is listed and read whole before `out` is
/// created, so that a corpus missing, empty, unpaired or unreadable leaves nothing behind;
/// a case refused when it is prepared (an invalid case file or patch) leaves `out` and the
/// WAV files written before it, its own earlier formats included. A render whose output
/// stage replaced a non-finite sample (a box's defect, decision 12) is not written: its line
/// is printed ([`conformant`]), every other render written, and the command fails once all
/// are, naming how many it refused. Files already in `out` are kept, an earlier file of a
/// render it refuses among them: `out` is meant empty (`cargo xtask` exports into a fresh
/// directory).
fn export(
    files: &mut dyn Files,
    out: &mut dyn Write,
    dir: &Path,
    target: &Path,
    (registry, mode): (Registry, DefaultMode<'_>),
) -> Result<(), String> {
    let cases = cases(files, dir, registry)?;
    files
        .create_dir_all(target)
        .map_err(|error| format!("cannot create {}: {error}", target.display()))?;
    let mut refused = 0_usize;
    for case in cases {
        for format in RenderFormat::ALL {
            let rendered = case.prepare(format, mode)?.render(mode);
            let rendered = match conformant(&rendered, &case.name, format) {
                Ok(rendered) => rendered,
                Err(defect) => {
                    say(out, &OneLine::new(&defect))?;
                    refused = refused.saturating_add(1);
                    continue;
                }
            };
            let path = target.join(format!("{}.{format}.wav", case.name));
            files
                .write(&path, &rendered.wav())
                .map_err(|error| format!("cannot write {}: {error}", path.display()))?;
            let hash = hash_of(rendered)?;
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
    if refused > 0 {
        return Err(format!(
            "export failed in {}: {refused} render(s) refused",
            dir.display()
        ));
    }
    Ok(())
}

/// Runs the command named by `args`, in the default floating-point mode of the processor,
/// writing its output to `out` (stdout) and its warnings to `warnings` (stderr: one
/// `warning:` line each, never a failure).
///
/// The mode is set by `fpenv::with_default_mode`, whatever mode the calling thread
/// had: renders are the same bits as WebAssembly's even under a host that flushes
/// subnormals.
///
/// # Errors
///
/// Returns [`CliError::Usage`] for a missing or unknown command or invalid arguments,
/// and [`CliError::Failed`] for an unreadable or invalid case, a conformance failure or
/// an output that cannot be written.
pub fn run(
    args: &[String],
    files: &mut dyn Files,
    out: &mut dyn Write,
    warnings: &mut dyn Write,
) -> Result<(), CliError> {
    run_in(&REGISTRY, args, files, (out, warnings))
}

/// [`run`] with the box types of `registry`: the engine's for every command, the tests
/// giving one with test-only box types.
///
/// # Errors
///
/// Those of [`run`].
pub(crate) fn run_in(
    registry: Registry,
    args: &[String],
    files: &mut dyn Files,
    (out, warnings): (&mut dyn Write, &mut dyn Write),
) -> Result<(), CliError> {
    with_default_mode(|mode| dispatch(args, files, (out, warnings), (registry, mode)))
}

/// Runs the command named by `args`, building its cases from the box types of the
/// registry and rendering in the default floating-point mode it proves ([`run`]).
fn dispatch(
    args: &[String],
    files: &mut dyn Files,
    (out, warnings): (&mut dyn Write, &mut dyn Write),
    engine: (Registry, DefaultMode<'_>),
) -> Result<(), CliError> {
    let Some((command, rest)) = args.split_first() else {
        return Err(CliError::Usage("missing command".to_owned()));
    };
    let expects = |synopsis: &str| CliError::Usage(format!("{command} expects {synopsis}"));
    match (command.as_str(), rest) {
        ("render", _) => render(files, (out, warnings), rest, engine),
        ("check", [dir]) => Ok(check(files, out, Path::new(dir), None, engine)?),
        ("check", [dir, option, spent]) if option == "--spent" => Ok(check(
            files,
            out,
            Path::new(dir),
            Some(Path::new(spent)),
            engine,
        )?),
        ("bless", [dir, options @ ..]) => {
            let options = BlessOptions::parse(options).map_err(CliError::Usage)?;
            Ok(bless(files, out, Path::new(dir), &options, engine)?)
        }
        ("export", [dir, target]) => Ok(export(
            files,
            out,
            Path::new(dir),
            Path::new(target),
            engine,
        )?),
        ("help", []) => Ok(say_usage(out)?),
        ("check", _) => Err(expects("<dir> [--spent <record>]")),
        ("bless", _) => Err(expects(BLESS)),
        ("export", _) => Err(expects("<dir> <out>")),
        ("help", _) => Err(expects("no argument")),
        _ => Err(CliError::Usage(format!("unknown command \"{command}\""))),
    }
}
