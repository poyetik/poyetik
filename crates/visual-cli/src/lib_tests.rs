//! Tests of the commands, with an in-memory file system, and of the real one.

use alloc::collections::BTreeMap;
use std::ffi::OsString;
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use super::{
    BestEffort, CliError, Disk, Files, MAX_CASE_BYTES, MAX_REFERENCES_BYTES, USAGE, arguments,
    parse_case, render_case, run,
};

/// A small valid case.
const SMALL: &str = r#"{"rate": [24000, 1001], "width": 17, "height": 9, "first": 3, "images": 5,
  "square": {"side": 40000, "cycle": [1001, 1000]}}"#;

/// Another valid case, at another rate and size.
const OTHER: &str = r#"{"rate": [60, 1], "width": 4, "height": 3, "first": 0, "images": 2,
  "square": {"side": 65536, "cycle": [1, 2]}}"#;

/// Files kept in memory.
#[derive(Debug, Default)]
struct Memory {
    /// The files, by path.
    files: BTreeMap<PathBuf, Vec<u8>>,
    /// A path whose reads fail.
    unreadable: Option<PathBuf>,
    /// Whether every write fails.
    read_only: bool,
}

impl Memory {
    /// Files holding `entries`, each a path and its text.
    fn with(entries: &[(&str, &str)]) -> Self {
        let mut memory = Self::default();
        for (path, text) in entries {
            drop(
                memory
                    .files
                    .insert(PathBuf::from(path), text.as_bytes().to_vec()),
            );
        }
        memory
    }

    /// The text of the file at `path`.
    fn text(&self, path: &str) -> String {
        String::from_utf8(self.files.get(Path::new(path)).cloned().expect("a file")).expect("UTF-8")
    }
}

impl Files for Memory {
    /// Refuses a file larger than `limit` like [`Disk`], with a message naming the limit.
    fn read(&mut self, path: &Path, limit: usize) -> io::Result<String> {
        if self.unreadable.as_deref() == Some(path) {
            return Err(io::Error::other("unreadable"));
        }
        let bytes = self
            .files
            .get(path)
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "no such file"))?;
        if bytes.len() > limit {
            return Err(io::Error::new(
                io::ErrorKind::FileTooLarge,
                format!("larger than {limit} bytes"),
            ));
        }
        String::from_utf8(bytes.clone())
            .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))
    }

    fn write(&mut self, path: &Path, bytes: &[u8]) -> io::Result<()> {
        if self.read_only {
            return Err(io::Error::other("read-only"));
        }
        drop(self.files.insert(path.to_path_buf(), bytes.to_vec()));
        Ok(())
    }

    /// Lists in reverse name order: a real directory has no order either.
    fn list(&mut self, dir: &Path) -> io::Result<Vec<PathBuf>> {
        let entries: Vec<PathBuf> = self
            .files
            .keys()
            .rev()
            .filter(|path| path.parent() == Some(dir))
            .cloned()
            .collect();
        if entries.is_empty() {
            Err(io::Error::other("no such directory"))
        } else {
            Ok(entries)
        }
    }
}

/// A writer whose every write and flush fails with `kind`.
#[derive(Debug)]
struct Pipe(io::ErrorKind);

impl Write for Pipe {
    fn write(&mut self, _buf: &[u8]) -> io::Result<usize> {
        Err(io::Error::from(self.0))
    }

    fn flush(&mut self) -> io::Result<()> {
        Err(io::Error::from(self.0))
    }
}

/// The words of a command line.
fn args(words: &[&str]) -> Vec<String> {
    words.iter().map(|word| (*word).to_owned()).collect()
}

/// Runs `words` on `memory`, returning the outcome and what was written on stdout.
fn run_with(memory: &mut Memory, words: &[&str]) -> (Result<(), CliError>, String) {
    let mut out = Vec::new();
    let outcome = run(&args(words), memory, &mut out);
    (outcome, String::from_utf8(out).expect("UTF-8 output"))
}

/// The hash of a case's text, as the case module renders it.
fn hash(text: &str) -> String {
    render_case(&parse_case(text).expect("a valid case"))
        .expect("rendered")
        .to_string()
}

/// A corpus of two cases, `d/cases/small.json` and `d/cases/other.json`, and its references.
fn corpus(references: &str) -> Memory {
    Memory::with(&[
        ("d/cases/small.json", SMALL),
        ("d/cases/other.json", OTHER),
        ("d/references.json", references),
    ])
}

/// The references file of the corpus, as bless writes it.
fn blessed() -> String {
    format!(
        "{{\n  \"other\": \"{}\",\n  \"small\": \"{}\"\n}}\n",
        hash(OTHER),
        hash(SMALL)
    )
}

#[test]
fn reads_arguments_as_text_only() {
    assert_eq!(
        arguments(vec![OsString::from("render"), OsString::from("a.json")]),
        Ok(args(&["render", "a.json"])),
        "UTF-8 arguments"
    );
    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStringExt;
        assert_eq!(
            arguments(vec![
                OsString::from("render"),
                OsString::from_vec(vec![0xff])
            ]),
            Err(CliError::Usage("argument 2 is not valid UTF-8".to_owned())),
            "the first argument that is not UTF-8, counted from 1"
        );
    }
}

#[test]
fn says_each_error_on_one_line_with_its_exit_code() {
    let usage = CliError::Usage("no command".to_owned());
    assert_eq!(
        (usage.line(), usage.exit_code()),
        ("error: no command (see: visual-cli help)".to_owned(), 2),
        "a usage error points to the help"
    );
    let failed = CliError::Failed("a\nb".to_owned());
    assert_eq!(
        (failed.line(), failed.exit_code()),
        ("error: a\\u{a}b".to_owned(), 1),
        "a failure, made safe on one line"
    );
}

#[test]
fn refuses_arguments_that_form_no_command() {
    for (words, reason) in [
        (&[][..], "no command"),
        (&["paint"][..], "unknown command or arguments: paint"),
        (&["render"][..], "unknown command or arguments: render"),
        (
            &["check", "a", "b"][..],
            "unknown command or arguments: check",
        ),
        (&["bless"][..], "unknown command or arguments: bless"),
        (&["help", "me"][..], "unknown command or arguments: help"),
    ] {
        let (outcome, said) = run_with(&mut Memory::default(), words);
        assert_eq!(
            outcome,
            Err(CliError::Usage(reason.to_owned())),
            "{words:?}"
        );
        assert_eq!(said, "", "{words:?}: nothing on stdout");
    }
}

#[test]
fn prints_the_usage_on_stdout() {
    assert_eq!(
        run_with(&mut Memory::default(), &["help"]),
        (Ok(()), format!("{USAGE}\n")),
        "the usage"
    );
    assert_eq!(
        run(
            &args(&["help"]),
            &mut Memory::default(),
            &mut Pipe(io::ErrorKind::StorageFull)
        ),
        Err(CliError::Failed(
            "cannot write to stdout: no storage space".to_owned()
        )),
        "a stdout that cannot be written"
    );
}

#[test]
fn renders_one_case_to_its_hash() {
    let mut memory = Memory::with(&[("a.json", SMALL)]);
    assert_eq!(
        run_with(&mut memory, &["render", "a.json"]),
        (Ok(()), format!("{}\n", hash(SMALL))),
        "the hash, alone on its line"
    );
    assert_ne!(hash(SMALL), hash(OTHER), "the hash depends on the case");
    assert_eq!(
        run(
            &args(&["render", "a.json"]),
            &mut memory,
            &mut Pipe(io::ErrorKind::StorageFull)
        ),
        Err(CliError::Failed(
            "cannot write to stdout: no storage space".to_owned()
        )),
        "a stdout that cannot be written"
    );
}

#[test]
fn names_the_case_it_cannot_render() {
    let large = " ".repeat(MAX_CASE_BYTES.saturating_add(1));
    let mut memory = Memory::with(&[
        ("bad.json", r#"{"rate": [0, 1]}"#),
        (
            "odd.json",
            r#"{"rate": [60, 1], "width": 4, "height": 3, "first": 0, "images": 0,
          "square": {"side": 1, "cycle": [1, 1]}}"#,
        ),
        ("large.json", &large),
    ]);
    let failed = |files: &mut Memory, file: &str| run_with(files, &["render", file]).0;
    assert_eq!(
        failed(&mut memory, "missing.json"),
        Err(CliError::Failed(
            "cannot read missing.json: no such file".to_owned()
        )),
        "a missing file"
    );
    assert_eq!(
        failed(&mut memory, "large.json"),
        Err(CliError::Failed(format!(
            "cannot read large.json: larger than {MAX_CASE_BYTES} bytes"
        ))),
        "a file past the limit"
    );
    assert!(
        failed(&mut memory, "bad.json")
            .is_err_and(|error| error.to_string().starts_with("bad.json: ")),
        "an invalid case, named"
    );
    assert_eq!(
        failed(&mut memory, "odd.json"),
        Err(CliError::Failed(
            "odd.json: images must be from 1 to 10000, not 0".to_owned()
        )),
        "a value out of range, named"
    );
}

#[test]
fn checks_every_case_against_its_reference() {
    let mut memory = corpus(&blessed());
    assert_eq!(
        run_with(&mut memory, &["check", "d"]),
        (
            Ok(()),
            format!("ok other {}\nok small {}\n", hash(OTHER), hash(SMALL))
        ),
        "every case, in name order"
    );
}

#[test]
fn reports_every_case_that_differs_from_its_reference() {
    let wrong = "0".repeat(64);
    let references = format!("{{\n  \"gone\": \"{wrong}\",\n  \"small\": \"{wrong}\"\n}}\n");
    let (outcome, said) = run_with(&mut corpus(&references), &["check", "d"]);
    assert_eq!(
        outcome,
        Err(CliError::Failed(
            "3 of the visual cases do not match their references, listed above".to_owned()
        )),
        "every problem counted"
    );
    assert_eq!(
        said,
        format!(
            "no reference for other\nchanged small: {}, reference {wrong}\nstale reference \
             gone: no such case\n",
            hash(SMALL)
        ),
        "a case without a reference, a changed one and a stale reference, each on its line"
    );
    assert_eq!(
        run(
            &args(&["check", "d"]),
            &mut corpus(&references),
            &mut Pipe(io::ErrorKind::StorageFull)
        ),
        Err(CliError::Failed(
            "cannot write to stdout: no storage space".to_owned()
        )),
        "a stdout that cannot be written"
    );
    let stale_only = format!(
        "{{\n  \"gone\": \"{wrong}\",\n  \"other\": \"{}\",\n  \"small\": \"{}\"\n}}\n",
        hash(OTHER),
        hash(SMALL)
    );
    assert_eq!(
        run(
            &args(&["check", "d"]),
            &mut corpus(&stale_only),
            &mut Pipe(io::ErrorKind::StorageFull)
        ),
        Err(CliError::Failed(
            "cannot write to stdout: no storage space".to_owned()
        )),
        "the stale line too"
    );
}

#[test]
fn reads_the_references_only_as_bless_writes_them() {
    let blessed = blessed();
    let rewrite = Err(CliError::Failed(
        "d/references.json is not as `visual-cli bless d` writes it: rewrite it with that command"
            .to_owned(),
    ));
    for (references, what) in [
        (blessed.trim_end().to_owned(), "without its last newline"),
        (blessed.replace("  ", "\t"), "indented otherwise"),
        (format!("{blessed}\n"), "a blank line after it"),
        (
            blessed.replacen("{\n", "{\n  \"small\": \"x\",\n", 1),
            "a repeated name",
        ),
        ("[]\n".to_owned(), "not an object"),
        ("{\"a\": 1}\n".to_owned(), "a value that is not a hash"),
        ("not JSON".to_owned(), "not JSON"),
    ] {
        assert_eq!(
            run_with(&mut corpus(&references), &["check", "d"]).0,
            rewrite,
            "{what}"
        );
    }
    let mut unreadable = corpus(&blessed);
    unreadable.unreadable = Some(PathBuf::from("d/references.json"));
    assert_eq!(
        run_with(&mut unreadable, &["check", "d"]).0,
        Err(CliError::Failed(
            "cannot read d/references.json: unreadable".to_owned()
        )),
        "an unreadable file"
    );
    let large = " ".repeat(MAX_REFERENCES_BYTES.saturating_add(1));
    assert_eq!(
        run_with(&mut corpus(&large), &["check", "d"]).0,
        Err(CliError::Failed(format!(
            "cannot read d/references.json: larger than {MAX_REFERENCES_BYTES} bytes"
        ))),
        "a file past the limit"
    );
}

#[test]
fn refuses_a_corpus_it_cannot_read_whole() {
    assert_eq!(
        run_with(&mut Memory::default(), &["check", "d"]).0,
        Err(CliError::Failed(
            "cannot list d/cases: no such directory".to_owned()
        )),
        "no cases directory"
    );
    let mut stray = corpus(&blessed());
    drop(
        stray
            .files
            .insert(PathBuf::from("d/cases/notes.txt"), b"x".to_vec()),
    );
    assert_eq!(
        run_with(&mut stray, &["check", "d"]).0,
        Err(CliError::Failed(
            "d/cases/notes.txt is not a case file (<name>.json)".to_owned()
        )),
        "a file that is not a case"
    );
    let mut broken = corpus(&blessed());
    drop(
        broken
            .files
            .insert(PathBuf::from("d/cases/broken.json"), b"{}".to_vec()),
    );
    assert!(
        run_with(&mut broken, &["check", "d"])
            .0
            .is_err_and(|error| error.to_string().starts_with("d/cases/broken.json: ")),
        "an invalid case, named"
    );
}

#[test]
fn says_when_the_cases_directory_lists_nothing() {
    /// A file system whose every directory is empty.
    #[derive(Debug)]
    struct Empty;
    impl Files for Empty {
        fn read(&mut self, _path: &Path, _limit: usize) -> io::Result<String> {
            Err(io::Error::from(io::ErrorKind::NotFound))
        }
        fn write(&mut self, _path: &Path, _bytes: &[u8]) -> io::Result<()> {
            Err(io::Error::from(io::ErrorKind::ReadOnlyFilesystem))
        }
        fn list(&mut self, _dir: &Path) -> io::Result<Vec<PathBuf>> {
            Ok(Vec::new())
        }
    }
    assert!(Empty.read(Path::new("a"), 1).is_err(), "nothing to read");
    assert!(Empty.write(Path::new("a"), b"").is_err(), "nothing written");
    for command in ["check", "bless"] {
        let mut out = Vec::new();
        assert_eq!(
            run(&args(&[command, "d"]), &mut Empty, &mut out),
            Err(CliError::Failed("no case in d/cases".to_owned())),
            "{command}"
        );
    }
}

#[test]
fn blesses_the_references_the_check_then_accepts() {
    let mut memory = corpus("stale");
    assert_eq!(
        run_with(&mut memory, &["bless", "d"]),
        (
            Ok(()),
            "wrote 2 references to d/references.json\n".to_owned()
        ),
        "every case"
    );
    assert_eq!(
        memory.text("d/references.json"),
        blessed(),
        "written as read back"
    );
    assert_eq!(
        run_with(&mut memory, &["check", "d"]).0,
        Ok(()),
        "then checked"
    );
    let mut read_only = corpus("stale");
    read_only.read_only = true;
    assert_eq!(
        run_with(&mut read_only, &["bless", "d"]).0,
        Err(CliError::Failed(
            "cannot write d/references.json: read-only".to_owned()
        )),
        "an unwritable file"
    );
    assert_eq!(
        read_only.text("d/references.json"),
        "stale",
        "left as it was"
    );
}

#[test]
fn forgives_only_a_closed_stdout() {
    let mut closed = BestEffort(Pipe(io::ErrorKind::BrokenPipe));
    assert_eq!(
        closed.write(b"abc").ok(),
        Some(3),
        "a closed reader: written"
    );
    assert!(closed.flush().is_ok(), "and flushed");
    let mut full = BestEffort(Pipe(io::ErrorKind::StorageFull));
    assert_eq!(
        full.write(b"abc").map_err(|error| error.kind()),
        Err(io::ErrorKind::StorageFull),
        "another error kept"
    );
    assert_eq!(
        full.flush().map_err(|error| error.kind()),
        Err(io::ErrorKind::StorageFull),
        "on flush too"
    );
    let mut fine = BestEffort(Vec::new());
    assert_eq!(fine.write(b"abc").ok(), Some(3), "written");
    assert!(fine.flush().is_ok(), "flushed");
    assert_eq!(fine.0, b"abc", "the bytes passed through");
    assert_eq!(
        run(
            &args(&["help"]),
            &mut Memory::default(),
            &mut BestEffort(Pipe(io::ErrorKind::BrokenPipe))
        ),
        Ok(()),
        "a command whose reader is gone still succeeds"
    );
}

/// A directory of one test below the system's temporary directory, removed with everything
/// in it when the guard is dropped: when the test returns or unwinds, a failing test or a
/// panicking mutant included. A test process killed (a mutant stopped at its timeout) or
/// aborted runs no drop and leaves it; the next test of the same name in a process of the
/// same id removes it when it starts.
#[derive(Debug)]
#[must_use = "the directory is removed when the guard is dropped"]
struct Scratch(PathBuf);

impl AsRef<Path> for Scratch {
    fn as_ref(&self) -> &Path {
        &self.0
    }
}

impl core::ops::Deref for Scratch {
    type Target = Path;
    fn deref(&self) -> &Path {
        &self.0
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        // Nothing better can be done in a drop: a directory a test removed itself is gone.
        drop(std::fs::remove_dir_all(&self.0));
    }
}

/// A fresh, empty directory for one test, `visual-cli-<name>-<process id>` ([`Scratch`]).
#[expect(
    clippy::disallowed_methods,
    reason = "the one place visual-cli's unit tests name the temporary directory: the guard \
              removes what it creates"
)]
fn scratch(name: &str) -> Scratch {
    let dir = std::env::temp_dir().join(format!("visual-cli-{name}-{}", std::process::id()));
    if let Err(error) = std::fs::remove_dir_all(&dir) {
        assert_eq!(error.kind(), io::ErrorKind::NotFound, "clean start");
    }
    std::fs::create_dir_all(&dir).expect("scratch directory");
    Scratch(dir)
}

#[test]
fn removes_a_scratch_directory_whatever_the_test_does() {
    let kept = scratch("guard");
    let dropped = kept.to_path_buf();
    std::fs::write(kept.join("a.txt"), "a").expect("written");
    drop(kept);
    assert!(!dropped.exists(), "removed with its files when dropped");
    let unwound = std::panic::catch_unwind(|| {
        let held = scratch("guard-unwound");
        let inside = held.to_path_buf();
        std::fs::write(held.join("a.txt"), "a").expect("written");
        std::panic::panic_any(inside);
    });
    let left = unwound
        .expect_err("the test panicked")
        .downcast::<PathBuf>()
        .expect("the path");
    assert!(!left.exists(), "removed when the test unwinds");
}

#[test]
fn disk_reads_writes_and_lists() {
    let dir = scratch("disk");
    let file = dir.join("a.txt");
    let mut disk = Disk;
    assert!(disk.write(&file, b"hello").is_ok(), "write");
    assert_eq!(disk.read(&file, 5).ok(), Some("hello".to_owned()), "read");
    assert_eq!(
        disk.read(&file, 4)
            .map_err(|error| (error.kind(), error.to_string())),
        Err((
            io::ErrorKind::FileTooLarge,
            "larger than 4 bytes".to_owned()
        )),
        "one byte over the limit"
    );
    assert!(disk.write(&file, b"\xff").is_ok(), "write bytes");
    assert_eq!(
        disk.read(&file, 5).map_err(|error| error.kind()),
        Err(io::ErrorKind::InvalidData),
        "not UTF-8"
    );
    assert_eq!(disk.list(&dir).ok(), Some(vec![file]), "list");
    assert!(
        disk.list(&dir.join("missing")).is_err(),
        "missing directory"
    );
    assert!(disk.read(&dir.join("missing"), 5).is_err(), "missing file");
    assert!(
        disk.write(&dir.join("missing").join("x"), b"").is_err(),
        "unwritable"
    );
}

#[cfg(unix)]
#[test]
fn disk_reads_at_most_one_byte_past_the_limit() {
    // An endless file: reading it whole would never end.
    let error = Disk
        .read(Path::new("/dev/zero"), 1_000)
        .expect_err("too large");
    assert_eq!(error.to_string(), "larger than 1000 bytes", "refused");
}
