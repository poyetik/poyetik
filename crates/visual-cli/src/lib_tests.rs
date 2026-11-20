//! Tests of the commands, with an in-memory file system, and of the real one.

use alloc::collections::BTreeMap;
use std::ffi::OsString;
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use super::{
    BestEffort, CliError, Disk, Files, MAX_CASE_BYTES, MAX_MESSAGE_CHARS, MAX_REFERENCES_BYTES,
    USAGE, arguments, display_corpus, parse_case, run,
};
use crate::display::display_text;

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
    /// A path whose writes fail.
    unwritable: Option<PathBuf>,
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
        if self.read_only || self.unwritable.as_deref() == Some(path) {
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
    parse_case(text)
        .expect("a valid case")
        .hash()
        .expect("room for the image")
        .to_string()
}

/// A corpus of two cases, `d/cases/small.json` and `d/cases/other.json`, and its references.
fn corpus(references: &str) -> Memory {
    Memory::with(&[
        ("d/cases/small.json", SMALL),
        ("d/cases/other.json", OTHER),
        ("d/references.json", references),
        ("d/display.json", &display_text(&display_corpus())),
    ])
}

/// The line `check` gives for the display corpus as bless writes it.
fn display_ok() -> String {
    format!(
        "ok       display.json {} pixel(s)\n",
        display_corpus().len()
    )
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
        (
            "error: no command (see: cargo run -p visual-cli -- help)".to_owned(),
            2
        ),
        "a usage error points to the help"
    );
    assert_eq!(
        (
            usage.to_string(),
            CliError::Failed("a\nb".to_owned()).to_string()
        ),
        (
            "no command (see: cargo run -p visual-cli -- help)".to_owned(),
            "a\nb".to_owned()
        ),
        "shown as an error: the reason and its pointer, raw (line() makes it safe)"
    );
    let long = CliError::Usage("x".repeat(1200));
    assert_eq!(
        long.line(),
        format!(
            "error: {}… (see: cargo run -p visual-cli -- help)",
            "x".repeat(MAX_MESSAGE_CHARS - 1)
        ),
        "the pointer to the help after a reason cut short"
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
        (&["paint"][..], "unknown command \"paint\""),
        (&["render"][..], "render expects <case.json>"),
        (&["render", "a", "b"][..], "render expects <case.json>"),
        (&["check"][..], "check expects <dir>"),
        (&["check", "a", "b"][..], "check expects <dir>"),
        (&["bless"][..], "bless expects <dir>"),
        (&["help", "me"][..], "help expects no argument"),
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
    assert_eq!(
        failed(&mut memory, "bad.json"),
        Err(CliError::Failed(
            "bad.json: missing field `width` at line 1 column 16".to_owned()
        )),
        "an invalid case, named"
    );
    assert_eq!(
        failed(&mut memory, "odd.json"),
        Err(CliError::Failed(
            "odd.json: images: from 1 to 10000, not 0".to_owned()
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
            format!(
                "ok       other {}\nok       small {}\n{}",
                hash(OTHER),
                hash(SMALL),
                display_ok()
            )
        ),
        "every case, in name order, then the display corpus"
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
            "3 problem(s), listed above; when the renders are meant to change, `cargo run -p visual-cli \
             -- bless d` rewrites the references and display.json"
                .to_owned()
        )),
        "every problem counted"
    );
    assert_eq!(
        said,
        format!(
            "MISSING  other: no reference\nMISMATCH small: {}, reference {wrong}\nSTALE    \
             gone: reference without a case\n{}",
            hash(SMALL),
            display_ok()
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
fn follows_its_bless_hint_from_every_problem_it_gives_it_for() {
    let wrong = "0".repeat(64);
    for (state, references) in [
        (
            "missing",
            format!("{{\n  \"small\": \"{}\"\n}}\n", hash(SMALL)),
        ),
        (
            "mismatch",
            format!(
                "{{\n  \"other\": \"{}\",\n  \"small\": \"{wrong}\"\n}}\n",
                hash(OTHER)
            ),
        ),
        (
            "stale",
            format!(
                "{{\n  \"gone\": \"{wrong}\",\n  \"other\": \"{}\",\n  \"small\": \"{}\"\n}}\n",
                hash(OTHER),
                hash(SMALL)
            ),
        ),
    ] {
        let mut memory = corpus(&references);
        assert_eq!(
            run_with(&mut memory, &["check", "d"]).0,
            Err(CliError::Failed(
                "1 problem(s), listed above; when the renders are meant to change, `cargo run -p \
                 visual-cli -- bless d` rewrites the references and display.json"
                    .to_owned()
            )),
            "{state}: the hint given"
        );
        assert_eq!(
            run_with(&mut memory, &["bless", "d"]).0,
            Ok(()),
            "{state}: followed"
        );
        assert_eq!(
            run_with(&mut memory, &["check", "d"]).0,
            Ok(()),
            "{state}: then checked"
        );
    }
}

#[test]
fn reads_the_references_only_as_bless_writes_them() {
    let blessed = blessed();
    let restore = |why: &str| {
        Err(CliError::Failed(format!(
            "d/references.json is not as bless writes it ({why}): restore it from \
             git; bless rewrites it from the current renders, only when they are meant to change"
        )))
    };
    let rewrite = restore("a name written twice, or another layout or order");
    for (references, what) in [
        (blessed.trim_end().to_owned(), "without its last newline"),
        (blessed.replace("  ", "\t"), "indented otherwise"),
        (format!("{blessed}\n"), "a blank line after it"),
        (
            blessed.replacen("{\n", "{\n  \"small\": \"x\",\n", 1),
            "a repeated name",
        ),
    ] {
        assert_eq!(
            run_with(&mut corpus(&references), &["check", "d"]).0,
            rewrite,
            "{what}"
        );
    }
    for (references, reason) in [
        (
            "[]\n",
            "invalid type: sequence, expected a map at line 1 column 0",
        ),
        (
            "{\"a\": 1}\n",
            "invalid type: integer `1`, expected a string at line 1 column 7",
        ),
        ("not JSON", "expected ident at line 1 column 2"),
        ("<<<<<<< HEAD\n", "expected value at line 1 column 1"),
        ("", "EOF while parsing a value at line 1 column 0"),
    ] {
        assert_eq!(
            run_with(&mut corpus(references), &["check", "d"]).0,
            restore(reason),
            "{references}: restored, not rewritten"
        );
    }
    let mut unreadable = corpus(&blessed);
    unreadable.unreadable = Some(PathBuf::from("d/references.json"));
    assert_eq!(
        run_with(&mut unreadable, &["check", "d"]).0,
        Err(CliError::Failed(
            "cannot read d/references.json: unreadable".to_owned()
        )),
        "an unreadable file: no command would read it"
    );
    let large = " ".repeat(MAX_REFERENCES_BYTES.saturating_add(1));
    assert_eq!(
        run_with(&mut corpus(&large), &["check", "d"]).0,
        Err(CliError::Failed(format!(
            "cannot read d/references.json: larger than {MAX_REFERENCES_BYTES} bytes: restore it \
             from git"
        ))),
        "a file past the limit"
    );
    let mut garbled = corpus("");
    drop(
        garbled
            .files
            .insert(PathBuf::from("d/references.json"), vec![0xff, 0xfe]),
    );
    assert_eq!(
        run_with(&mut garbled, &["check", "d"]).0,
        Err(CliError::Failed(
            "cannot read d/references.json: invalid utf-8 sequence of 1 bytes from index 0: \
             restore it from git"
                .to_owned()
        )),
        "a file that is not text"
    );
}

#[test]
fn reads_a_references_file_of_one_mebibyte_and_no_more() {
    // The limit as documented, written out: a wrong constant fails here, not only past
    // itself.
    let mut written = blessed();
    written.insert_str(1, &" ".repeat(1_048_576_usize - written.len()));
    assert_eq!(written.len(), 1_048_576_usize, "exactly one mebibyte");
    assert_eq!(
        run_with(&mut corpus(&written), &["check", "d"]).0,
        Err(CliError::Failed(
            "d/references.json is not as bless writes it (a name written twice, \
             or another layout or order): restore it from git; bless rewrites it from the \
             current renders, only when they are meant to change"
                .to_owned()
        )),
        "read whole, then refused for its layout only"
    );
    written.push(' ');
    assert_eq!(
        run_with(&mut corpus(&written), &["check", "d"]).0,
        Err(CliError::Failed(format!(
            "cannot read d/references.json: larger than {} bytes: restore it from git",
            1_048_576_usize
        ))),
        "one byte more"
    );
}

#[test]
fn refuses_a_corpus_it_cannot_read_whole() {
    assert_eq!(
        run_with(&mut Memory::default(), &["check", "d"]).0,
        Err(CliError::Failed(
            "cannot read d/references.json: no such file".to_owned()
        )),
        "no references and no cases: no command would help"
    );
    let mut new = corpus("");
    drop(new.files.remove(Path::new("d/references.json")));
    assert_eq!(
        run_with(&mut new, &["check", "d"]).0,
        Err(CliError::Failed(
            "cannot read d/references.json: no such file: restore it from git, or write the \
             references of a new directory with `cargo run -p visual-cli -- bless d`"
                .to_owned()
        )),
        "cases without references"
    );
    assert_eq!(
        run_with(&mut new, &["bless", "d"]).0,
        Ok(()),
        "the hint followed"
    );
    assert_eq!(
        run_with(&mut new, &["check", "d"]).0,
        Ok(()),
        "then checked"
    );
    let mut no_case = Memory::with(&[("d/cases/.DS_Store", "x")]);
    assert_eq!(
        run_with(&mut no_case, &["check", "d"]).0,
        Err(CliError::Failed(
            "cannot read d/references.json: no such file".to_owned()
        )),
        "a cases directory without a case: bless would refuse it, so no hint"
    );
    assert_eq!(
        run_with(&mut Memory::default(), &["bless", "d"]).0,
        Err(CliError::Failed(
            "cannot list d/cases: no such directory".to_owned()
        )),
        "no cases directory"
    );
    let mut stray = corpus(&blessed());
    for file in ["d/cases/.DS_Store", "d/cases/notes.txt", "d/cases/.json"] {
        drop(stray.files.insert(PathBuf::from(file), b"x".to_vec()));
    }
    assert_eq!(
        run_with(&mut stray, &["check", "d"]),
        run_with(&mut corpus(&blessed()), &["check", "d"]),
        "a file that is not a case file is not a case"
    );
    for name in ["Small", "-small", "small_case", "small case", "_é"] {
        let mut named = corpus(&blessed());
        drop(named.files.insert(
            PathBuf::from(format!("d/cases/{name}.json")),
            SMALL.as_bytes().to_vec(),
        ));
        drop(named.files.insert(
            PathBuf::from("d/cases/zz-Bad.json"),
            SMALL.as_bytes().to_vec(),
        ));
        assert_eq!(
            run_with(&mut named, &["check", "d"]).0,
            Err(CliError::Failed(format!(
                "d/cases/{name}.json: not a case name (lowercase ASCII letters, digits, dots and \
                 hyphens, starting with a letter or a digit)"
            ))),
            "{name}: refused, the first in name order"
        );
    }
    for name in ["v2.0-wide", "0.5", "a-b.c"] {
        let mut named = corpus(&blessed());
        drop(named.files.insert(
            PathBuf::from(format!("d/cases/{name}.json")),
            SMALL.as_bytes().to_vec(),
        ));
        let (_, said) = run_with(&mut named, &["check", "d"]);
        assert!(
            said.contains(&format!("MISSING  {name}: no reference")),
            "{name}: a case name, dots and hyphens included, read as a case: {said}"
        );
    }
}

#[test]
fn names_a_case_file_it_cannot_read_by_its_case() {
    let mut unreadable = corpus(&blessed());
    unreadable.unreadable = Some(PathBuf::from("d/cases/small.json"));
    assert_eq!(
        run_with(&mut unreadable, &["check", "d"]),
        (
            Err(CliError::Failed(
                "1 problem(s), listed above; 1 case(s) cannot be rendered: fix them first"
                    .to_owned()
            )),
            format!(
                "ok       other {}\nFAILED   small: cannot read d/cases/small.json: unreadable\n{}",
                hash(OTHER),
                display_ok()
            )
        ),
        "the case named, the file it could not read too"
    );
}

#[test]
fn names_every_case_it_cannot_render_and_blesses_none() {
    let mut broken = corpus(&blessed());
    drop(
        broken
            .files
            .insert(PathBuf::from("d/cases/broken.json"), b"{}".to_vec()),
    );
    let (outcome, said) = run_with(&mut broken, &["check", "d"]);
    assert_eq!(
        (outcome, said),
        (
            Err(CliError::Failed(
                "1 problem(s), listed above; 1 case(s) cannot be rendered: fix them first"
                    .to_owned()
            )),
            format!(
                "FAILED   broken: missing field `rate` at line 1 column 2\nok       other {}\nok       \
                 small {}\n{}",
                hash(OTHER),
                hash(SMALL),
                display_ok()
            )
        ),
        "an invalid case, named on its line, and every other case still checked"
    );
    drop(
        broken
            .files
            .insert(PathBuf::from("d/cases/empty.json"), b"".to_vec()),
    );
    assert_eq!(
        run_with(&mut broken, &["bless", "d"]),
        (
            Err(CliError::Failed(
                "bless refused: 2 case(s) cannot be rendered, listed above; nothing written to \
                 d/references.json"
                    .to_owned()
            )),
            "FAILED   broken: missing field `rate` at line 1 column 2\nFAILED   empty: EOF while \
             parsing a value at line 1 column 0\n"
                .to_owned()
        ),
        "bless lists every case it cannot render, and says it wrote nothing"
    );
    assert_eq!(
        broken.text("d/references.json"),
        blessed(),
        "the references left as they were"
    );
}

#[test]
fn says_when_the_cases_directory_holds_no_case() {
    for command in ["check", "bless"] {
        let mut memory = Memory::with(&[
            ("d/cases/.DS_Store", "x"),
            ("d/cases/notes.txt", "x"),
            ("d/references.json", "{}\n"),
        ]);
        assert_eq!(
            run_with(&mut memory, &[command, "d"]).0,
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
            format!(
                "wrote    2 reference(s) to d/references.json\nwrote    {} pixel(s) to \
                 d/display.json\n",
                display_corpus().len()
            )
        ),
        "every case, then the display corpus"
    );
    assert_eq!(
        memory.text("d/references.json"),
        blessed(),
        "written as read back"
    );
    assert_eq!(
        memory.text("d/display.json"),
        display_text(&display_corpus()),
        "the display corpus written as check reads it"
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
    let mut display_read_only = corpus("stale");
    display_read_only.unwritable = Some(PathBuf::from("d/display.json"));
    assert_eq!(
        run_with(&mut display_read_only, &["bless", "d"]).0,
        Err(CliError::Failed(
            "cannot write d/display.json: read-only".to_owned()
        )),
        "an unwritable display corpus"
    );
}

/// What `check` ends with when the display corpus is a problem: restored from git, and blessed
/// only when the display encoding is meant to change, never to take a changed encoding in.
const DISPLAY_SUMMARY: &str = "1 problem(s), listed above; restore display.json from git: when the \
                               renders or the display encoding are meant to change, `cargo run -p \
                               visual-cli -- bless d` rewrites the references and display.json";

#[test]
fn checks_the_display_corpus_as_bless_writes_it() {
    let cases = format!(
        "ok       other {}\nok       small {}\n",
        hash(OTHER),
        hash(SMALL)
    );
    let mut missing = corpus(&blessed());
    drop(missing.files.remove(Path::new("d/display.json")));
    let mut large = corpus(&blessed());
    drop(large.files.insert(
        PathBuf::from("d/display.json"),
        vec![b' '; MAX_REFERENCES_BYTES.saturating_add(1)],
    ));
    let mut binary = corpus(&blessed());
    drop(
        binary
            .files
            .insert(PathBuf::from("d/display.json"), vec![0xff]),
    );
    let mut unreadable = corpus(&blessed());
    unreadable.unreadable = Some(PathBuf::from("d/display.json"));
    for (mut memory, line, state) in [
        (
            missing,
            "MISSING  display.json: cannot read d/display.json: no such file: restore it from git, \
             or write the display corpus of a new directory with `cargo run -p visual-cli -- \
             bless d`"
                .to_owned(),
            "a missing display corpus",
        ),
        (
            large,
            format!(
                "MISMATCH display.json: cannot read d/display.json: larger than \
                 {MAX_REFERENCES_BYTES} bytes: restore it from git"
            ),
            "a display corpus too large",
        ),
        (
            binary,
            "MISMATCH display.json: cannot read d/display.json: invalid utf-8 sequence of 1 bytes \
             from index 0: restore it from git"
                .to_owned(),
            "a display corpus not text",
        ),
        (
            unreadable,
            "FAILED   display.json: cannot read d/display.json: unreadable".to_owned(),
            "an unreadable display corpus: no remedy that would not help",
        ),
    ] {
        assert_eq!(
            run_with(&mut memory, &["check", "d"]),
            (
                Err(CliError::Failed(DISPLAY_SUMMARY.to_owned())),
                format!("{cases}{line}\n")
            ),
            "{state}"
        );
    }
    let mut changed = corpus(&blessed());
    let text = display_text(&display_corpus());
    // The bytes of the first translucent pixel, one changed: pixel 255 (the 256th).
    let line = text.lines().nth(2 + 255).expect("a pixel line");
    let bytes = line
        .get(line.len() - 11..line.len() - 3)
        .expect("its bytes");
    let other = format!("{:08x}", u32::from_str_radix(bytes, 16).expect("hex") ^ 1);
    let wrong = text.replacen(line, &line.replace(bytes, &other), 1);
    drop(
        changed
            .files
            .insert(PathBuf::from("d/display.json"), wrong.into_bytes()),
    );
    assert_eq!(
        run_with(&mut changed, &["check", "d"]),
        (
            Err(CliError::Failed(DISPLAY_SUMMARY.to_owned())),
            format!(
                "{cases}MISMATCH display.json: not the native encoding of the display corpus, \
                 from pixel 255: restore it from git\n"
            )
        ),
        "a changed byte: restored, never blessed into the corpus"
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
    assert_eq!(
        disk.list(&dir.join("missing"))
            .map_err(|error| error.kind()),
        Err(io::ErrorKind::NotFound),
        "missing directory"
    );
    assert_eq!(
        disk.read(&dir.join("missing"), 5)
            .map_err(|error| error.kind()),
        Err(io::ErrorKind::NotFound),
        "missing file"
    );
    assert_eq!(
        disk.write(&dir.join("missing").join("x"), b"")
            .map_err(|error| error.kind()),
        Err(io::ErrorKind::NotFound),
        "unwritable: its directory is missing"
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

/// Every file of the shared invalid corpus and why `visual-cli` refuses it, so that no file
/// is refused for another fault than the one it holds.
const INVALID_CORPUS: &[(&str, &str)] = &[
    ("case-above-64-kib", "larger than 65536 bytes"),
    (
        "case-as-an-array",
        "invalid type: sequence, expected a JSON object at line 1 column 0",
    ),
    (
        "cycle-0",
        "square: cycle: invalid duration 0/1 s: a reduced, nonzero fraction is required",
    ),
    (
        "cycle-not-reduced",
        "square: cycle: invalid duration 2/4 s: a reduced, nonzero fraction is required",
    ),
    (
        "field-written-twice",
        "duplicate field `first` at line 6 column 9",
    ),
    (
        "first-above-u64",
        "first: invalid type: floating point `1.8446744073709552e+19`, expected u64 at line 5 column 31",
    ),
    (
        "first-negative-zero",
        "first: invalid type: floating point `-0.0`, expected u64 at line 5 column 13",
    ),
    (
        "first-negative",
        "first: invalid value: integer `-1`, expected u64 at line 5 column 13",
    ),
    (
        "height-0",
        "width and height: invalid image size 3x0: each side must be from 1 to 16384 pixels",
    ),
    (
        "height-above-16384",
        "width and height: invalid image size 4x16385: each side must be from 1 to 16384 pixels",
    ),
    (
        "image-above-the-pixel-limit",
        "width and height: invalid image size 4051x8283: an image holds at most 33554432 pixels",
    ),
    ("images-0", "images: from 1 to 10000, not 0"),
    (
        "images-above-the-limit",
        "images: from 1 to 10000, not 10001",
    ),
    (
        "last-image-past-the-largest-index",
        "first and images: invalid image range: 2 images from image 18446744073709551615 pass the largest index, 18446744073709551615",
    ),
    ("missing-field", "missing field `height` at line 7 column 1"),
    (
        "not-an-object",
        "invalid type: string \"a case\", expected a JSON object at line 1 column 8",
    ),
    (
        "number-as-string",
        "width: invalid type: string \"4\", expected u32 at line 3 column 14",
    ),
    (
        "number-with-exponent",
        "width: invalid type: floating point `4.0`, expected u32 at line 3 column 14",
    ),
    (
        "number-with-fraction",
        "images: invalid type: floating point `2.0`, expected u64 at line 6 column 15",
    ),
    (
        "rate-above-1000-per-second",
        "rate: invalid image rate 1001/1: a reduced fraction from 1 to 1000 images per second is required",
    ),
    (
        "rate-below-one-per-second",
        "rate: invalid image rate 1/2: a reduced fraction from 1 to 1000 images per second is required",
    ),
    (
        "rate-denominator-0",
        "rate: invalid image rate 60/0: a reduced fraction from 1 to 1000 images per second is required",
    ),
    (
        "rate-not-a-pair",
        "rate: invalid length 1, expected an array of length 2 at line 2 column 14",
    ),
    (
        "rate-not-reduced",
        "rate: invalid image rate 120/2: a reduced fraction from 1 to 1000 images per second is required",
    ),
    (
        "side-above-a-full-square",
        "square: side: invalid square side 65537: at most 65536, a square as tall as the image (a side is in 65536ths of the image height)",
    ),
    (
        "side-negative-zero",
        "square: side: invalid type: floating point `-0.0`, expected u32 at line 7 column 24",
    ),
    (
        "square-as-an-array",
        "square: invalid type: sequence, expected a JSON object at line 7 column 12",
    ),
    (
        "square-field-written-twice",
        "square: duplicate field `side` at line 7 column 35",
    ),
    (
        "too-many-pixels-rendered",
        "images, width and height: a case renders at most 268435456 pixels in all, this one 301989888",
    ),
    (
        "unknown-field",
        "unknown field `speed`, expected one of `rate`, `width`, `height`, `first`, `images`, `square` at line 7 column 9",
    ),
    (
        "unknown-square-field",
        "square: unknown field `color`, expected `side` or `cycle` at line 7 column 36",
    ),
    (
        "value-above-u32",
        "square: side: invalid value: integer `4294967296`, expected u32 at line 7 column 32",
    ),
    (
        "width-0",
        "width and height: invalid image size 0x3: each side must be from 1 to 16384 pixels",
    ),
    (
        "width-16385",
        "width and height: invalid image size 16385x4: each side must be from 1 to 16384 pixels",
    ),
];

#[test]
fn rejects_every_case_of_the_shared_invalid_corpus_for_its_own_fault() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../conformance/visual/invalid/cases");
    let mut files = Disk.list(&dir).expect("the shared invalid corpus");
    files.sort();
    let names: Vec<String> = files
        .iter()
        .map(|file| {
            file.file_stem()
                .and_then(|stem| stem.to_str())
                .expect("a file name")
                .to_owned()
        })
        .collect();
    let expected: Vec<&str> = INVALID_CORPUS.iter().map(|(name, _)| *name).collect();
    assert_eq!(
        names, expected,
        "every file has its reason, and every reason its file"
    );
    for (file, (name, reason)) in files.iter().zip(INVALID_CORPUS) {
        let refused = match Disk.read(file, MAX_CASE_BYTES) {
            Ok(text) => parse_case(&text).err().map(|error| error.to_string()),
            Err(error) => Some(error.to_string()),
        };
        assert_eq!(refused.as_deref(), Some(*reason), "{name}");
    }
}

#[test]
fn names_a_bless_command_a_shell_takes_as_it_is() {
    for (dir, command) in [
        (
            "conformance/visual",
            "cargo run -p visual-cli -- bless conformance/visual",
        ),
        ("my cases", "cargo run -p visual-cli -- bless 'my cases'"),
        ("it's", "cargo run -p visual-cli -- bless 'it'\\''s'"),
        ("", "cargo run -p visual-cli -- bless ''"),
    ] {
        assert_eq!(
            super::cli::bless_command(Path::new(dir)),
            command,
            "{dir:?}"
        );
    }
}
