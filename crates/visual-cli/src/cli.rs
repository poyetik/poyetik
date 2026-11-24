//! The `visual-cli` commands.

use alloc::collections::BTreeMap;
use std::ffi::OsString;
use std::io::{ErrorKind, Write};
use std::path::{Path, PathBuf};

use crate::case::{MakeImage, hash_case_with};
use crate::display::{DISPLAY_FILE, display_corpus, display_text, first_difference};
use crate::safe::safe_remedy;
use crate::status::Status;
use crate::{Files, MAX_CASE_BYTES, safe_message};

use visual_core::{Image, VisualError};

/// Usage of the `visual-cli` command, printed on stdout by `visual-cli help`.
pub const USAGE: &str = "\
usage: visual-cli <command>
  render <case.json>  render a visual case, print its image hash
  check <dir>         compare every case of <dir>/cases with <dir>/references.json, and the
                      display corpus's native encoding with <dir>/display.json
  bless <dir>         rewrite <dir>/references.json from the current renders, and
                      <dir>/display.json from the native encoding
  help                print this help";

/// What follows the reason of a usage error: where the usage is.
const USAGE_HINT: &str = " (see: cargo run -p visual-cli -- help)";

/// The largest references file read, in bytes.
pub const MAX_REFERENCES_BYTES: usize = 1024 * 1024;

/// Why a command did not succeed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CliError {
    /// The arguments do not form a command: exit code 2.
    Usage(String),
    /// The command ran and failed: exit code 1.
    Failed(String),
    /// The command ran and failed, and its line says what helps: exit code 1.
    Remedied {
        /// Why it failed, bounded in the line.
        reason: String,
        /// What helps, whole in the line, with the punctuation before it.
        remedy: String,
    },
}

impl CliError {
    /// The error as the one line the tool prints on stderr, without its newline: `error: `
    /// and the message made safe by [`safe_message`], then, for a usage error, its pointer
    /// to the usage, ` (see: cargo run -p visual-cli -- help)`, and for a failure with a remedy,
    /// the remedy, appended after the reason is bounded, so that no reason can cut either.
    #[must_use]
    pub fn line(&self) -> String {
        match self {
            Self::Usage(reason) => format!("error: {}{USAGE_HINT}", safe_message(reason)),
            Self::Failed(message) => format!("error: {}", safe_message(message)),
            Self::Remedied { reason, remedy } => {
                format!("error: {}{}", safe_message(reason), safe_remedy(remedy))
            }
        }
    }

    /// The exit code of the process for this error: 2 for a usage error, 1 for a failure.
    #[must_use]
    pub const fn exit_code(&self) -> u8 {
        match self {
            Self::Usage(_) => 2,
            Self::Failed(_) | Self::Remedied { .. } => 1,
        }
    }
}

impl core::fmt::Display for CliError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Usage(reason) => write!(f, "{reason}{USAGE_HINT}"),
            Self::Failed(message) => f.write_str(message),
            Self::Remedied { reason, remedy } => write!(f, "{reason}{remedy}"),
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

/// Renders the case file at `path`, its image made by `image` ([`Seams`]), and returns its
/// hash as text.
fn hash_of(files: &mut dyn Files, path: &Path, image: MakeImage) -> Result<String, CliError> {
    let text = read(files, path, MAX_CASE_BYTES)?;
    hash_case_with(&text, image)
        .map(|hash| hash.to_string())
        .map_err(|error| CliError::Failed(format!("{}: {error}", path.display())))
}

/// Renders the case file at `path` for a status line, which names the case: its hash, or why
/// it cannot be rendered (a file that cannot be read is named, a case's own fault is not).
/// A case's fault is in its content, which no command fixes, and its reason names what to
/// fix (the field, the value refused): it carries no runnable hint. A file that cannot be
/// read for a reason outside its content says what helps, by the kind of the error
/// ([`read_remedy`]), apart from the reason, or nothing: made readable, or a directory in its
/// place moved away, then the corpus of `root` (absolute) checked again; a case is no file git
/// must hold, so none is restored.
fn status_of(
    files: &mut dyn Files,
    path: &Path,
    root: &Path,
    image: MakeImage,
) -> Result<String, (String, String)> {
    let text = files.read(path, MAX_CASE_BYTES).map_err(|error| {
        let hint = match read_remedy(error.kind()) {
            ReadRemedy::Readable => format!(": {}", readable_hint(root)),
            ReadRemedy::MoveDirectory => format!(": {}", directory_hint(root)),
            ReadRemedy::Restore | ReadRemedy::RestoreOrBless | ReadRemedy::Unknown => String::new(),
        };
        (format!("cannot read {}: {error}", path.display()), hint)
    })?;
    hash_case_with(&text, image)
        .map(|hash| hash.to_string())
        .map_err(|error| (error.to_string(), String::new()))
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

/// The renders of a directory's cases: the hash of each case that renders, by case name, and
/// each case that cannot be rendered with why and what helps ([`status_of`]), in the order of their files' paths.
type Renders = (BTreeMap<String, String>, Vec<(String, (String, String))>);

/// The renders of every case of `<dir>/cases`, by case name (the file name without `.json`),
/// each image made by `image`.
fn render_all(files: &mut dyn Files, dir: &Path, image: MakeImage) -> Result<Renders, CliError> {
    let mut hashes = BTreeMap::new();
    let mut failed = Vec::new();
    let root = files.absolute(dir);
    for (name, path) in case_files(files, dir)? {
        match status_of(files, &path, &root, image) {
            Ok(hash) => drop(hashes.insert(name, hash)),
            Err(why) => failed.push((name, why)),
        }
    }
    Ok((hashes, failed))
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
        Err(error) if error.kind() == ErrorKind::BrokenPipe => Ok(done),
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
    say_remedied(out, line, "")
}

/// Writes one line to `out` as [`say`] does, its `remedy` whole after it, which no reason
/// cuts.
fn say_remedied(out: &mut dyn Write, line: &str, remedy: &str) -> Result<(), CliError> {
    writeln!(out, "{}{}", safe_message(line), safe_remedy(remedy))
        .map_err(|error| CliError::Failed(format!("cannot write to stdout: {error}")))
}

/// `path` as one word of a shell command: quoted for a shell when it holds anything but
/// letters, digits and `_./-`.
fn shell_word(path: &Path) -> String {
    let text = path.display().to_string();
    let plain = !text.is_empty()
        && text
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"_./-".contains(&byte));
    if plain {
        text
    } else {
        format!("'{}'", text.replace('\'', "'\\''"))
    }
}

/// What a read failure of a corpus file asks, by the kind of its error ([`read_remedy`]):
/// the one classification every reader of the corpus shares.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ReadRemedy {
    /// A file too large or not text: not as bless writes it, restored by git.
    Restore,
    /// A missing file: restored by git, or written by bless for a new directory.
    RestoreOrBless,
    /// A directory where the file should be: moved away, then the check run again, which
    /// restores the file then missing. Never a restore at once: git deletes the directory with
    /// the untracked files it holds.
    MoveDirectory,
    /// A file its permissions keep unread: made readable, then the check run again.
    Readable,
    /// Any other failure (a disk's), which no command of the repository fixes: the error
    /// alone.
    Unknown,
}

/// The remedy of a read failing with `kind` ([`ReadRemedy`]).
fn read_remedy(kind: ErrorKind) -> ReadRemedy {
    if kind == ErrorKind::FileTooLarge || kind == ErrorKind::InvalidData {
        ReadRemedy::Restore
    } else if kind == ErrorKind::NotFound {
        ReadRemedy::RestoreOrBless
    } else if kind == ErrorKind::IsADirectory {
        ReadRemedy::MoveDirectory
    } else if kind == ErrorKind::PermissionDenied {
        ReadRemedy::Readable
    } else {
        ReadRemedy::Unknown
    }
}

/// The remedy of a directory standing where a file of the corpus `dir`, an absolute path,
/// should be: moved away, then the check run again ([`check_hint`]).
fn directory_hint(dir: &Path) -> String {
    format!(
        "a directory stands where the file should be: move it away, then {}",
        check_hint(dir)
    )
}

/// The check of the corpus `dir`, an absolute path ([`Files::absolute`]), as a runnable hint
/// ([`shell_word`]).
fn check_hint(dir: &Path) -> String {
    format!(
        "check again (see: cargo run -p visual-cli -- check {})",
        shell_word(dir)
    )
}

/// The command that rewrites the references of `dir`, an absolute path
/// ([`Files::absolute`], [`shell_word`]), so that it runs from any directory of the
/// repository: `visual-cli` is no installed program, so the hint names cargo.
pub(crate) fn bless_command(dir: &Path) -> String {
    format!("cargo run -p visual-cli -- bless {}", shell_word(dir))
}

/// The remedy of a corpus file `file`, an absolute path ([`Files::absolute`]), that is not
/// as bless writes it: `noun` restored by git from the last commit,
/// never from the index, which may hold the damage staged, as a runnable hint that runs from
/// any directory of the repository ([`shell_word`]).
pub(crate) fn restore_hint(noun: &str, file: &Path) -> String {
    format!(
        "restore {noun} (see: git checkout HEAD -- {})",
        shell_word(file)
    )
}

/// The remedy of a corpus file of `dir`, an absolute path ([`Files::absolute`]), that
/// cannot be read for a reason no command fixes (its permissions): made readable, then the
/// check run again ([`check_hint`]).
pub(crate) fn readable_hint(dir: &Path) -> String {
    format!("make it readable, then {}", check_hint(dir))
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
/// A file written otherwise (not JSON, a name written twice, another layout or order) is
/// to be restored, rewritten from the renders only when they are meant to change: that would
/// take in any change of the renders, and no reference would show it. A file that cannot be
/// read says what would help by the kind of its error ([`read_remedy`]), each remedy a
/// runnable hint ([`restore_hint`], [`readable_hint`]): restoring a file too large or not
/// text, restoring or blessing a missing one beside case files, moving a directory in its place
/// away or making an unreadable one readable, then checking again;
/// nothing for a directory without a case, which is no corpus, nor for another failure.
fn references(files: &mut dyn Files, dir: &Path) -> Result<BTreeMap<String, String>, CliError> {
    let file = dir.join("references.json");
    let at = files.absolute(dir);
    let restored = at.join("references.json");
    let restore = |why: &str| CliError::Remedied {
        reason: format!("{} is not as bless writes it ({why})", file.display()),
        remedy: format!(
            ": {}, or rewrite it only when the renders are meant to change (see: {})",
            restore_hint("it", &restored),
            bless_command(&at)
        ),
    };
    let text = files.read(&file, MAX_REFERENCES_BYTES).map_err(|error| {
        let hint = match read_remedy(error.kind()) {
            ReadRemedy::Restore => format!(": {}", restore_hint("it", &restored)),
            ReadRemedy::RestoreOrBless if case_files(files, dir).is_ok() => format!(
                ": {}, or write the references of a new directory (see: {})",
                restore_hint("it", &restored),
                bless_command(&at)
            ),
            ReadRemedy::MoveDirectory => format!(": {}", directory_hint(&at)),
            ReadRemedy::Readable => format!(": {}", readable_hint(&at)),
            ReadRemedy::RestoreOrBless | ReadRemedy::Unknown => String::new(),
        };
        CliError::Remedied {
            reason: format!("cannot read {}: {error}", file.display()),
            remedy: hint,
        }
    })?;
    let references: BTreeMap<String, String> =
        serde_json::from_str(&text).map_err(|error| restore(&error.to_string()))?;
    if references_text(&references) == text {
        Ok(references)
    } else {
        Err(restore("a name written twice, or another layout or order"))
    }
}

/// Compares the render of every case of `dir` with its reference, listing every problem:
/// a case that cannot be rendered, a hash that differs, a case without a reference, a
/// reference without a case.
fn check(
    files: &mut dyn Files,
    dir: &Path,
    out: &mut dyn Write,
    seams: Seams,
) -> Result<(), CliError> {
    let references = references(files, dir)?;
    let mut problems = 0_usize;
    let mut failed = 0_usize;
    let mut seen = alloc::collections::BTreeSet::new();
    let root = files.absolute(dir);
    for (name, path) in case_files(files, dir)? {
        let mut remedy = String::new();
        let (status, line) = match status_of(files, &path, &root, seams.image) {
            Err((why, helps)) => {
                failed = failed.saturating_add(1);
                remedy = helps;
                (Status::Failed, format!("{name}: {why}"))
            }
            Ok(hash) => match references.get(&name) {
                Some(expected) if *expected == hash => (Status::Ok, format!("{name} {hash}")),
                Some(expected) => (
                    Status::Mismatch,
                    format!("{name}: {hash}, reference {expected}"),
                ),
                None => (Status::Missing, format!("{name}: no reference")),
            },
        };
        if status != Status::Ok {
            problems = problems.saturating_add(1);
        }
        say_remedied(out, &format!("{status} {line}"), &remedy)?;
        let _new = seen.insert(name);
    }
    for name in references.keys().filter(|name| !seen.contains(*name)) {
        say(
            out,
            &format!("{} {name}: reference without a case", Status::Stale),
        )?;
        problems = problems.saturating_add(1);
    }
    let display = display_matches(files, dir, out, seams.encode)?;
    let at = files.absolute(dir);
    if display != Status::Ok {
        problems = problems.saturating_add(1);
    }
    if problems == 0 {
        Ok(())
    } else if failed > 0 {
        // Bless would refuse these too: no command until they are fixed.
        Err(CliError::Failed(format!(
            "{problems} problem(s), listed above; {failed} case(s) cannot be rendered: fix them first"
        )))
    } else if display == Status::Failed {
        // Its line gives its remedy, which no command here may contradict.
        Err(CliError::Failed(format!(
            "{problems} problem(s), listed above; {DISPLAY_FILE} cannot be read: fix it first"
        )))
    } else if display == Status::Ok {
        Err(CliError::Remedied {
            reason: format!("{problems} problem(s), listed above"),
            remedy: format!(
                "; when the renders are meant to change, bless rewrites the references and {DISPLAY_FILE} (see: {})",
                bless_command(&at)
            ),
        })
    } else {
        // The display corpus guards the native encoding: blessing it would take a changed
        // encoding in, so it is restored unless the encoding is meant to change.
        Err(CliError::Remedied {
            reason: format!("{problems} problem(s), listed above"),
            remedy: format!(
                "; {}: when the renders or the display encoding are meant to change, bless rewrites the references and {DISPLAY_FILE} (see: {})",
                restore_hint(DISPLAY_FILE, &at.join(DISPLAY_FILE)),
                bless_command(&at)
            ),
        })
    }
}

/// Compares the display corpus file of `dir` with the native encoding of the corpus, saying
/// so on `out`: its status.
///
/// A file that differs, or cannot be read as text (too large, not UTF-8), is a mismatch,
/// to be restored by git ([`restore_hint`]): it guards the native encoding, so blessing it
/// would take a changed encoding in. A missing one is missing, and may also be written by
/// `bless`, for a new directory; one that cannot be read for another reason failed, its
/// remedy by the kind of its error ([`read_remedy`]): a directory in its place moved
/// away or made readable, before checking again ([`directory_hint`], [`readable_hint`]); or
/// the error alone.
///
/// # Errors
///
/// Returns why `out` cannot be written to, or why the corpus cannot be encoded
/// ([`encoded_corpus`]).
fn display_matches(
    files: &mut dyn Files,
    dir: &Path,
    out: &mut dyn Write,
    encode: Encode,
) -> Result<Status, CliError> {
    let file = dir.join(DISPLAY_FILE);
    let root = files.absolute(dir);
    let restored = root.join(DISPLAY_FILE);
    let pixels = display_corpus();
    let expected = encoded_corpus(encode, &pixels)?;
    let (status, line, remedy) = match files.read(&file, MAX_REFERENCES_BYTES) {
        Err(error) => {
            let (status, hint) = match read_remedy(error.kind()) {
                ReadRemedy::Restore => (
                    Status::Mismatch,
                    format!(": {}", restore_hint("it", &restored)),
                ),
                ReadRemedy::RestoreOrBless => (
                    Status::Missing,
                    format!(
                        ": {}, or write the display corpus of a new directory (see: {})",
                        restore_hint("it", &restored),
                        bless_command(&root)
                    ),
                ),
                ReadRemedy::MoveDirectory => {
                    (Status::Failed, format!(": {}", directory_hint(&root)))
                }
                ReadRemedy::Readable => (Status::Failed, format!(": {}", readable_hint(&root))),
                ReadRemedy::Unknown => (Status::Failed, String::new()),
            };
            (
                status,
                format!("{DISPLAY_FILE}: cannot read {}: {error}", file.display()),
                hint,
            )
        }
        Ok(held) => first_difference(&held, &expected).map_or_else(
            || {
                (
                    Status::Ok,
                    format!("{DISPLAY_FILE} {} pixel(s)", pixels.len()),
                    String::new(),
                )
            },
            |at| {
                (
                    Status::Mismatch,
                    format!(
                        "{DISPLAY_FILE}: not the native encoding of the display corpus, from {at}"
                    ),
                    format!(": {}", restore_hint("it", &restored)),
                )
            },
        ),
    };
    say_remedied(out, &format!("{status} {line}"), &remedy)?;
    Ok(status)
}

/// Rewrites the references of `dir` from the current renders, only when every case renders:
/// otherwise lists each case that cannot be rendered and writes nothing.
fn bless(
    files: &mut dyn Files,
    dir: &Path,
    out: &mut dyn Write,
    seams: Seams,
) -> Result<(), CliError> {
    let (hashes, failed) = render_all(files, dir, seams.image)?;
    let file = dir.join("references.json");
    if !failed.is_empty() {
        for (name, (why, remedy)) in &failed {
            say_remedied(out, &format!("{} {name}: {why}", Status::Failed), remedy)?;
        }
        return Err(CliError::Failed(format!(
            "bless refused: {} case(s) cannot be rendered, listed above; nothing written to {}",
            failed.len(),
            file.display()
        )));
    }
    // Encoded before anything is written: a corpus that cannot be encoded writes nothing.
    let pixels = display_corpus();
    let display_text = encoded_corpus(seams.encode, &pixels)?;
    files
        .write(&file, references_text(&hashes).as_bytes())
        .map_err(|error| CliError::Failed(format!("cannot write {}: {error}", file.display())))?;
    say(
        out,
        &format!(
            "{} {} reference(s) to {}",
            Status::Wrote,
            hashes.len(),
            file.display()
        ),
    )?;
    let display = dir.join(DISPLAY_FILE);
    files
        .write(&display, display_text.as_bytes())
        .map_err(|error| {
            CliError::Failed(format!("cannot write {}: {error}", display.display()))
        })?;
    say(
        out,
        &format!(
            "{} {} pixel(s) to {}",
            Status::Wrote,
            pixels.len(),
            display.display()
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
    run_with(
        args,
        files,
        out,
        Seams {
            image: Image::try_new,
            encode: display_text,
        },
    )
}

/// The allocations a command makes, which the tests refuse through [`run_with`] as a machine
/// without the memory would: no test machine refuses them otherwise.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Seams {
    /// How each case's image is made ([`Image::try_new`]).
    pub(crate) image: MakeImage,
    /// How the display corpus file is made ([`display_text`]).
    pub(crate) encode: Encode,
}

/// How the display corpus file of some pixels is made: [`display_text`], or, in the tests, an
/// encoding refused as a machine without the memory would refuse it.
pub(crate) type Encode = fn(&[[f32; 4]]) -> Result<String, VisualError>;

/// The display corpus file of `pixels` made by `encode`, its refusal a failure naming it.
///
/// # Errors
///
/// Returns [`CliError::Failed`] with why `encode` refused.
fn encoded_corpus(encode: Encode, pixels: &[[f32; 4]]) -> Result<String, CliError> {
    encode(pixels)
        .map_err(|error| CliError::Failed(format!("cannot encode the display corpus: {error}")))
}

/// [`run`], its allocations made through `seams`: the seam through which the tests refuse a
/// case's image or the display corpus's encoding, and see the lines the commands print then.
///
/// # Errors
///
/// As [`run`].
pub(crate) fn run_with(
    args: &[String],
    files: &mut dyn Files,
    out: &mut dyn Write,
    seams: Seams,
) -> Result<(), CliError> {
    let Some((command, rest)) = args.split_first() else {
        return Err(CliError::Usage("no command".to_owned()));
    };
    let expects = |synopsis: &str| CliError::Usage(format!("{command} expects {synopsis}"));
    match (command.as_str(), rest) {
        ("render", [case]) => {
            let hash = hash_of(files, Path::new(case), seams.image)?;
            say(out, &hash)
        }
        ("check", [dir]) => check(files, Path::new(dir), out, seams),
        ("bless", [dir]) => bless(files, Path::new(dir), out, seams),
        ("help", []) => writeln!(out, "{USAGE}")
            .map_err(|error| CliError::Failed(format!("cannot write to stdout: {error}"))),
        ("render", _) => Err(expects("<case.json>")),
        ("check" | "bless", _) => Err(expects("<dir>")),
        ("help", _) => Err(expects("no argument")),
        _ => Err(CliError::Usage(format!("unknown command \"{command}\""))),
    }
}
