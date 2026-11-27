//! What the tests of xtask share: the fake runner and its answers, the fixtures and the
//! scratch directories. Each module's tests are in its own `*_tests.rs` file; `steps_tests.rs`
//! also holds the tests that run the whole check to see how a step fails when it cannot read
//! its input.

use alloc::collections::BTreeMap;
use core::num::NonZeroUsize;
use core::time::Duration;
use std::ffi::OsString;
use std::io;
use std::path::Path;
use std::process::Command;

use super::{
    Action, CARGO_TOML, CHANGELOG, Cmd, CommandFailure, Coverage, EQUIVALENT_REVIEWED, Exit,
    Failed, Failure, Measured, Mode, PACKAGES, Ran, Runner, Step, SystemRunner, UNSAFE_FILES,
    UNVIABLE_REPORT, UNVIABLE_REVIEWED, check_steps, cli, package_jsons, run_steps,
};

/// A tracked source at `path` holding `text`.
pub(crate) fn source(path: &str, text: &str) -> (String, String) {
    (path.to_owned(), text.to_owned())
}

/// The Rust sources `sources` (path and text), each read into its tokens as the gates read
/// them ([`super::RustSource::parse`]).
pub(crate) fn parsed(sources: &[(String, String)]) -> Vec<super::RustSource> {
    sources
        .iter()
        .map(|(path, text)| super::RustSource::parse(path, text).expect("a Rust source that lexes"))
        .collect()
}

/// The sources of xtask, each its name and its tokens: its tests (`*_tests.rs`) when `tests`,
/// its other code otherwise.
pub(crate) fn own_sources(tests: bool) -> Vec<(String, Vec<crate::rust_source::Token>)> {
    std::fs::read_dir(Path::new(env!("CARGO_MANIFEST_DIR")).join("src"))
        .expect("xtask's sources")
        .map(|entry| entry.expect("a source").path())
        .filter(|path| path.to_string_lossy().ends_with("_tests.rs") == tests)
        .map(|path| {
            let text = std::fs::read_to_string(&path).expect("a source");
            let name = path.file_name().unwrap_or_default().to_string_lossy();
            (name.into_owned(), crate::rust_source::tokens_of(&text))
        })
        .collect()
}

/// [`super::unsafe_gate`] on `sources`, read by [`parsed`].
pub(crate) fn unsafe_gate(sources: &[(String, String)]) -> Result<(), Failure> {
    super::unsafe_gate(&parsed(sources))
}

/// [`super::test_code_gate`] on `sources`, read by [`parsed`].
pub(crate) fn test_code_gate(sources: &[(String, String)]) -> Result<(), Failure> {
    super::test_code_gate(&parsed(sources))
}

/// [`super::test_data_gate`] on `sources`, read by [`parsed`].
pub(crate) fn test_data_gate(sources: &[(String, String)]) -> Result<(), Failure> {
    super::test_data_gate(&parsed(sources))
}

/// [`super::untested_targets_gate`] on `metadata` and `sources`, read by [`parsed`].
pub(crate) fn untested_targets_gate(
    metadata: &str,
    sources: &[(String, String)],
) -> Result<(), Failure> {
    super::untested_targets_gate(metadata, &parsed(sources))
}

/// [`super::unviable_names_gate`] on `reviews` and `sources`, read by [`parsed`].
pub(crate) fn unviable_names_gate(
    reviews: &str,
    sources: &[(String, String)],
) -> Result<(), Failure> {
    super::unviable_names_gate(reviews, &parsed(sources))
}

/// The one git command of xtask's unit tests, ready to start in `dir`: without the machine's
/// and the user's configuration ([`super::HERMETIC_GIT`]), nor the variables an enclosing git
/// process sets ([`super::LOCAL_GIT_VARIABLES`]).
pub(crate) fn hermetic_git_command(dir: &Path, args: &[&str]) -> Command {
    let mut command = Command::new("git");
    let _same = command
        .args(args)
        .current_dir(dir)
        .envs(super::HERMETIC_GIT);
    for variable in super::LOCAL_GIT_VARIABLES {
        let _without = command.env_remove(variable);
    }
    command
}

#[test]
fn starts_every_test_s_git_without_an_enclosing_git_s_variables() {
    // Finding K17-2 of the sixth deep review of visual block 2 (#34): three of these tests'
    // git commands kept the variables of an enclosing git process (a hook's `GIT_DIR`).
    let command = hermetic_git_command(Path::new("."), &["status"]);
    let set: Vec<(&std::ffi::OsStr, Option<&std::ffi::OsStr>)> = command.get_envs().collect();
    for (name, value) in super::HERMETIC_GIT
        .map(|(name, value)| (name, Some(value)))
        .into_iter()
        .chain(super::LOCAL_GIT_VARIABLES.map(|name| (name, None)))
    {
        assert!(
            set.contains(&(std::ffi::OsStr::new(name), value.map(std::ffi::OsStr::new))),
            "{name}: {value:?}: {set:?}"
        );
    }
}

/// Runs git in `dir` hermetically, and returns its stdout: no variable of an enclosing git
/// process (a hook's `GIT_DIR` …) reaches it, nor the user's or the system's configuration,
/// and commits get a fixed identity.
pub(crate) fn hermetic_git(dir: &Path, args: &[&str]) -> String {
    let (succeeded, stdout) = hermetic_git_ending(dir, args);
    assert!(succeeded, "git {args:?} in {}", dir.display());
    stdout
}

/// Runs git in `dir` as [`hermetic_git`] does, for a command expected to fail: its exit
/// code and its error output, trimmed, as this machine's git writes them (the wording and
/// the code of a refusal change between git versions, so a test of how xtask reports one
/// reads them from the git it runs, never from a literal).
pub(crate) fn hermetic_git_refusal(dir: &Path, args: &[&str]) -> (i32, String) {
    let output = hermetic_git_command(dir, args).output().expect("git runs");
    assert!(!output.status.success(), "git {args:?} refuses");
    (
        output.status.code().expect("git exits with a code"),
        String::from_utf8(output.stderr)
            .expect("UTF-8")
            .trim()
            .to_owned(),
    )
}

/// Runs git in `dir` as [`hermetic_git`] does, for a command that may fail on purpose (a
/// merge that conflicts): whether it succeeded, and its stdout.
pub(crate) fn hermetic_git_ending(dir: &Path, args: &[&str]) -> (bool, String) {
    let output = hermetic_git_command(dir, args)
        .env("GIT_AUTHOR_NAME", "Test")
        .env("GIT_AUTHOR_EMAIL", "test@example.invalid")
        .env("GIT_COMMITTER_NAME", "Test")
        .env("GIT_COMMITTER_EMAIL", "test@example.invalid")
        .output()
        .expect("git runs");
    (
        output.status.success(),
        String::from_utf8(output.stdout).expect("UTF-8"),
    )
}

/// The configuration of git for every command a test runs, as [`hermetic_git`] or through
/// [`system`]: none of the machine's ([`super::HERMETIC_GIT`]).
pub(crate) const HERMETIC: [(&str, &str); 4] = super::HERMETIC_GIT;

/// A real runner at `root` whose commands read no git configuration of the machine
/// ([`HERMETIC`]).
pub(crate) fn system(root: std::path::PathBuf) -> SystemRunner {
    SystemRunner {
        root,
        env: &HERMETIC,
    }
}

/// A directory of one test below the system's temporary directory, removed with
/// everything in it when the guard is dropped: when the test returns or unwinds, a failing
/// test or a panicking mutant included. A test process killed (a mutant stopped at its
/// timeout) or aborted runs no drop and leaves it; the next test of the same name in a
/// process of the same id removes it when it starts.
#[derive(Debug)]
#[must_use = "the directory is removed when the guard is dropped"]
pub(crate) struct Scratch(pub(crate) std::path::PathBuf);

impl Scratch {
    /// Its path, owned.
    pub(crate) fn path(&self) -> std::path::PathBuf {
        self.0.clone()
    }
}

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

/// A fresh, empty directory for one test, `xtask-<name>-<process id>` ([`Scratch`]).
#[expect(
    clippy::disallowed_methods,
    reason = "the one place xtask's unit tests name the temporary directory: the guard removes \
              what it creates"
)]
pub(crate) fn scratch(name: &str) -> Scratch {
    let dir = std::env::temp_dir().join(format!("xtask-{name}-{}", std::process::id()));
    if let Err(error) = std::fs::remove_dir_all(&dir) {
        assert_eq!(error.kind(), io::ErrorKind::NotFound, "clean start");
    }
    std::fs::create_dir_all(&dir).expect("scratch directory");
    Scratch(dir)
}

#[test]
fn removes_a_scratch_directory_whatever_the_test_does() {
    let kept = scratch("guard");
    let dropped = kept.path();
    std::fs::write(kept.join("a.txt"), "a").expect("written");
    drop(kept);
    assert!(!dropped.exists(), "removed with its files when dropped");
    let unwound = std::panic::catch_unwind(|| {
        let held = scratch("guard-unwound");
        let inside = held.path();
        std::fs::write(held.join("a.txt"), "a").expect("written");
        std::panic::panic_any(inside);
    });
    let left = unwound
        .expect_err("the test panicked")
        .downcast::<std::path::PathBuf>()
        .expect("the path");
    assert!(!left.exists(), "removed when the test unwinds");
}

/// A way git shows no change of a tracked file whose bytes on disk are not the commit's:
/// each is staged on real git by [`hide`].
#[derive(Debug, Clone, Copy)]
pub(crate) struct Hiding {
    /// What hides the bytes, for a test's messages.
    pub(crate) by: &'static str,
    /// The file that gives `*.rs` the attribute git applies when it adds the file, and that
    /// attribute; none when only an index setting hides the bytes.
    attribute: Option<(&'static str, &'static str)>,
    /// Whether that file is removed once git has applied it.
    removed: bool,
    /// The setting of the repository's configuration it needs.
    setting: (&'static str, &'static str),
    /// The bytes committed.
    committed: &'static [u8],
    /// The bytes on disk, which git shows as no change.
    on_disk: &'static [u8],
    /// Whether the attribute is a filter the check's git reads, so that it is refused by its
    /// name ([`super::filtered`]).
    pub(crate) named: bool,
}

impl Hiding {
    /// Whether the repository's own attributes, which stay, convert the bytes.
    pub(crate) fn of_repository(&self) -> bool {
        self.attribute
            .is_some_and(|(file, _)| file == ".gitattributes")
    }

    /// The first line git writes of the attributes of `path` while the attribute is there.
    pub(crate) fn attribute_named(&self, path: &str) -> Option<String> {
        let (_, attribute) = self.attribute.filter(|_| !self.removed)?;
        let first = attribute.split(' ').next()?;
        let (name, value) = first.split_once('=').unwrap_or((first, "set"));
        Some(format!("{path}: {name}: {value}"))
    }
}

/// Where [`Hiding`] writes the user's attributes file: the repository's own configuration
/// names it (`core.attributesFile`), which the test's git reads and the check's git
/// neutralizes ([`super::NEUTRAL_GIT_CONFIG`]).
pub(crate) const USER_ATTRIBUTES: &str = ".git/user-attributes";

/// The attributes file of the repository that git never versions.
pub(crate) const INFO_ATTRIBUTES: &str = ".git/info/attributes";

/// Every way of hiding bytes the gates over the work tree refuse ([`Hiding`]).
pub(crate) const HIDINGS: [Hiding; 10] = [
    hidden_by_filter("a filter of the user's", USER_ATTRIBUTES, false),
    hidden_by_filter(
        "a filter of the user's, then removed",
        USER_ATTRIBUTES,
        true,
    ),
    hidden_by_filter("a filter of info/attributes", INFO_ATTRIBUTES, false),
    hidden_by_filter(
        "a filter of info/attributes, then removed",
        INFO_ATTRIBUTES,
        true,
    ),
    hidden_by_ident("ident of the user's", USER_ATTRIBUTES),
    hidden_by_ident("ident of info/attributes", INFO_ATTRIBUTES),
    hidden_by_encoding("an encoding of the user's", USER_ATTRIBUTES),
    hidden_by_encoding("an encoding of info/attributes", INFO_ATTRIBUTES),
    Hiding {
        by: "line endings the repository's own attributes convert",
        attribute: Some((".gitattributes", "text=auto eol=lf")),
        removed: false,
        setting: ("core.trustctime", "true"),
        committed: b"fn a() {}\n",
        on_disk: b"fn a() {}\r\n",
        named: false,
    },
    Hiding {
        by: "an edit of the same size and time, the change time not trusted",
        attribute: None,
        removed: false,
        setting: ("core.trustctime", "false"),
        committed: b"fn a() {}\n",
        on_disk: b"fn z() {}\n",
        named: false,
    },
];

/// A clean driver that drops a line, its attribute given by `file`, `removed` or not.
const fn hidden_by_filter(by: &'static str, file: &'static str, removed: bool) -> Hiding {
    Hiding {
        by,
        attribute: Some((file, "filter=hide")),
        removed,
        setting: ("filter.hide.clean", "grep -v HIDDEN"),
        committed: b"fn a() {}\n",
        on_disk: b"fn a() {}\nfn b() {} // HIDDEN\n",
        named: !removed && matches!(file.as_bytes(), b".git/info/attributes"),
    }
}

/// The `ident` conversion, which collapses `$Id: … $`, its attribute given by `file`.
const fn hidden_by_ident(by: &'static str, file: &'static str) -> Hiding {
    Hiding {
        by,
        attribute: Some((file, "ident")),
        removed: false,
        setting: ("core.trustctime", "true"),
        committed: b"// $Id$\n",
        on_disk: b"// $Id: hidden code $\n",
        named: false,
    }
}

/// The `working-tree-encoding` conversion, its attribute given by `file`.
const fn hidden_by_encoding(by: &'static str, file: &'static str) -> Hiding {
    Hiding {
        by,
        attribute: Some((file, "working-tree-encoding=UTF-16LE")),
        removed: false,
        setting: ("core.trustctime", "true"),
        committed: b"a\n",
        on_disk: b"a\0\n\0",
        named: false,
    }
}

/// Writes `bytes` to `path` below `dir`, modified long ago: git then takes the index's word
/// on a file it added, where it reads again one modified as late as the index.
fn written_long_ago(dir: &Path, path: &str, bytes: &[u8]) {
    let file = std::fs::File::create(dir.join(path)).expect("the file");
    io::Write::write_all(&mut &file, bytes).expect("written");
    let long_ago = std::time::SystemTime::UNIX_EPOCH
        .checked_add(Duration::from_secs(1_000_000_000))
        .expect("a time of 2001");
    file.set_modified(long_ago).expect("its time");
}

/// Commits `path` alone in a repository at `dir`, as `hiding` commits it.
pub(crate) fn commit_to_hide(dir: &Path, path: &str, hiding: &Hiding) {
    let parent = dir.join(path);
    std::fs::create_dir_all(parent.parent().expect("a parent")).expect("its directory");
    let _init = hermetic_git(dir, &["init", "--quiet", "--initial-branch=main"]);
    written_long_ago(dir, path, hiding.committed);
    if let Some((file, attribute)) = hiding.attribute.filter(|_| hiding.of_repository()) {
        std::fs::write(dir.join(file), format!("* {attribute}\n")).expect("the attributes");
    }
    let (key, value) = hiding.setting;
    let _set = hermetic_git(dir, &["config", key, value]);
    let _add = hermetic_git(dir, &["add", "."]);
    let _commit = hermetic_git(dir, &["commit", "--quiet", "-m", "first"]);
}

/// Hides other bytes of `path` ([`commit_to_hide`]) from git as `hiding` says, and shows that
/// it took effect: git's status is empty.
pub(crate) fn hide(dir: &Path, path: &str, hiding: &Hiding) {
    if let Some((file, attribute)) = hiding.attribute.filter(|_| !hiding.of_repository()) {
        let _user = hermetic_git(dir, &["config", "core.attributesFile", USER_ATTRIBUTES]);
        std::fs::write(dir.join(file), format!("*.rs {attribute}\n")).expect("the attributes");
    }
    written_long_ago(dir, path, hiding.on_disk);
    let _staged = hermetic_git(dir, &["add", "."]);
    if let Some((file, _)) = hiding.attribute.filter(|_| hiding.removed) {
        std::fs::remove_file(dir.join(file)).expect("the attributes removed");
    }
    assert_eq!(
        hermetic_git(dir, &["status", "--porcelain"]),
        "",
        "{}: git shows no change",
        hiding.by
    );
}

/// Does to `path` what the reason says in prose: the attribute removed, or the line endings
/// written as git stores them when the repository's own attributes convert them.
pub(crate) fn end_hiding(dir: &Path, path: &str, hiding: &Hiding) {
    if hiding.of_repository() {
        written_long_ago(dir, path, hiding.committed);
    } else if let Some((file, _)) = hiding.attribute.filter(|_| !hiding.removed) {
        std::fs::remove_file(dir.join(file)).expect("the attribute removed");
    }
}

/// Why the gates over the work tree refuse `path`, its name written `quoted` for a shell,
/// hidden as `hiding` says: by its filter's name when the check's git reads one.
pub(crate) fn hiding_reason(hiding: &Hiding, path: &str, quoted: &str) -> String {
    if hiding.named {
        return format!(
            "`{path}` passes through git's `hide` filter, whose clean driver may hide a change \
             from git (status, diff): the tree checked may not be the one committed: remove its \
             `filter` attribute (from `.gitattributes`, or `info/attributes` in git's common directory (see: git rev-parse --git-common-dir))"
        );
    }
    unshown_reason(path, quoted)
}

/// Why the gates over the work tree refuse `path`, its name written `quoted` for a shell,
/// whose bytes git does not show.
pub(crate) fn unshown_reason(path: &str, quoted: &str) -> String {
    format!(
        "`{path}`: its bytes are not those git shows (an attribute git applied when it was \
         added or checked out, `filter`, `ident`, `working-tree-encoding`, `text`, or an index \
         entry git does not reread): the tree checked may not be the one committed: remove the \
         attribute (see: git -C \"$(git rev-parse --show-toplevel)\" check-attr -a -- '{quoted}'), then add it again (see: git add --renormalize -- ':/{quoted}'), or write \
         its line endings as git stores them when the repository's own attributes convert them"
    )
}

/// Why the gates over the work tree refuse `path`, untracked code, test data or settings
/// git's status does not show, its name written `quoted` for a shell.
pub(crate) fn untracked_reason(path: &str, quoted: &str) -> String {
    format!(
        "`{path}` is untracked code, test data or settings git status does not show (an ignore \
         file of yours, or `info/exclude`, names it), which cargo and the other tools would \
         read: add \
         it (see: git add --force -- ':/{quoted}'), or remove it"
    )
}

/// Why the gates over the work tree refuse `path`, which git hides from its status, cleared
/// by the hints `clear` ([`cleared`]).
pub(crate) fn hidden_reason(path: &str, clear: &str) -> String {
    format!(
        "`{path}` is hidden from git status (assume-unchanged or skip-worktree), so no diff of \
         git shows its changes: clear it {clear}"
    )
}

/// The hint that clears git's mark by `flag` (`--no-skip-worktree`) from the file whose path
/// quoted for a shell, its quotes escaped, is `quoted`.
pub(crate) fn cleared(flag: &str, quoted: &str) -> String {
    format!("(see: git -C \"$(git rev-parse --show-toplevel)\" update-index {flag} -- '{quoted}')")
}

/// The problem of a mutant left at `place`, in the file whose path quoted for a shell,
/// its quotes escaped, is `quoted`, with the hint restoring it.
pub(crate) fn restore(place: &str, quoted: &str) -> String {
    format!(
        "{place} holds cargo-mutants' marker: restore it when it holds no change of yours \
         (see: git restore --worktree -- ':/{quoted}')"
    )
}

/// A fixture of xtask's tests (`tests/fixtures/<name>`), the directory its commands run in.
pub(crate) fn fixture(name: &str) -> std::path::PathBuf {
    let fixtures = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures");
    fixtures.join(name)
}

/// What the real `cargo metadata` ([`super::metadata_command`]) says of the fixture
/// workspace, and the root it names, from which it writes every path.
pub(crate) fn fixture_metadata() -> (String, String) {
    let metadata = system(fixture("workspace"))
        .capture(&super::metadata_command(super::CargoWorkspace::Root))
        .expect("cargo metadata");
    let json: serde_json::Value = serde_json::from_str(&metadata).expect("JSON");
    let root = json
        .get("workspace_root")
        .and_then(serde_json::Value::as_str)
        .expect("a workspace root")
        .to_owned();
    (metadata, root)
}

/// Runs [`cli()`] on UTF-8 arguments.
pub(crate) fn run_cli(
    args: &[String],
    runner: &mut dyn Runner,
    out: &mut dyn io::Write,
    err: &mut dyn io::Write,
) -> u8 {
    let args: Vec<OsString> = args.iter().map(OsString::from).collect();
    cli(&args, runner, out, err)
}

/// The failure a test expects, written as it is shown: its summary line, then one line
/// per detail.
pub(crate) fn failed(text: impl AsRef<str>) -> Result<(), Failure> {
    let mut lines = text.as_ref().lines().map(str::to_owned);
    Err(Failure {
        summary: lines.next().unwrap_or_default(),
        details: lines.collect(),
        usage: false,
    })
}

/// The failure of a gate that found `problems` about `context`, as [`super::steps::verdict`]
/// shows it (tested on its own by `shows_one_problem_inline_and_lists_several`).
pub(crate) fn found(context: &str, problems: &[String]) -> Result<(), Failure> {
    match problems {
        [only] => failed(format!("{context}: {only}")),
        _ => Err(Failure {
            summary: format!("{context}: {} problems, listed above", problems.len()),
            details: problems.to_vec(),
            usage: false,
        }),
    }
}

/// The failure of a gate run only as a step that found `problems`, as
/// [`super::steps::step_verdict`] shows it: the step's name comes from the step.
pub(crate) fn step_found(problems: &[String]) -> Result<(), Failure> {
    match problems {
        [only] => failed(only),
        _ => Err(Failure {
            summary: format!("{} problems, listed above", problems.len()),
            details: problems.to_vec(),
            usage: false,
        }),
    }
}

/// How the fake reads the state of its working tree, and what it answers: one untracked
/// file.
pub(crate) const TREE_STATUS: (&str, &str) = (
    "git status --porcelain --untracked-files=all",
    "?? notes.txt\n",
);

/// The changes of the tracked files of the fake: none.
const TREE_DIFF: (&str, &str) = ("git diff --no-ext-diff --no-textconv HEAD", "");

/// How the fake reads what is checked out, and what it answers: a commit on a branch.
pub(crate) const TREE_HEAD: (&str, &str) = (
    "git rev-parse HEAD --symbolic-full-name HEAD",
    "abc1230000000000000000000000000000000000\nrefs/heads/main\n",
);

/// The Cargo metadata of a workspace at `/r` whose crate `x`, a tested library, depends on
/// another crate of the workspace and on one third-party crate, beside the crates of the
/// audio library, which depend on nothing but the boxes' tests on the test kit, the crates
/// of the visual library and its tool, which depend on its own, and the crates of shared
/// test code, the test kit depending on the counting allocator; each has only the test
/// target reading its test data ([`test_data_reader_sources`]).
pub(crate) const METADATA: &str = r#"{"workspace_root":"/r","packages":[{"license":"MIT OR Apache-2.0","name":"x","manifest_path":"/r/crates/x/Cargo.toml","dependencies":[{"name":"audio-core","kind":null,"path":"/r/crates/audio-core"},{"name":"serde_json","kind":null}],"targets":[{"kind":["lib"],"src_path":"/r/crates/x/src/lib.rs","test":true,"doctest":true}]},{"license":"MIT OR Apache-2.0","name":"audio-core","manifest_path":"/r/crates/audio-core/Cargo.toml","targets":[{"kind":["test"],"src_path":"/r/crates/audio-core/tests/data.rs","test":true,"doctest":false}]},{"license":"MIT OR Apache-2.0","name":"audio-boxes","manifest_path":"/r/crates/audio-boxes/Cargo.toml","dependencies":[{"name":"audio-testkit","kind":"dev","path":"/r/crates/audio-testkit"}],"targets":[{"kind":["test"],"src_path":"/r/crates/audio-boxes/tests/data.rs","test":true,"doctest":false}]},{"license":"MIT OR Apache-2.0","name":"audio-patch","manifest_path":"/r/crates/audio-patch/Cargo.toml","targets":[{"kind":["test"],"src_path":"/r/crates/audio-patch/tests/data.rs","test":true,"doctest":false}]},{"license":"MIT OR Apache-2.0","name":"audio-hash","manifest_path":"/r/crates/audio-hash/Cargo.toml","targets":[{"kind":["test"],"src_path":"/r/crates/audio-hash/tests/data.rs","test":true,"doctest":false}]},{"license":"MIT OR Apache-2.0","name":"audio-host","manifest_path":"/r/crates/audio-host/Cargo.toml","targets":[{"kind":["test"],"src_path":"/r/crates/audio-host/tests/data.rs","test":true,"doctest":false}]},{"license":"MIT OR Apache-2.0","name":"fpenv","manifest_path":"/r/crates/fpenv/Cargo.toml","targets":[{"kind":["test"],"src_path":"/r/crates/fpenv/tests/data.rs","test":true,"doctest":false}]},{"license":"MIT OR Apache-2.0","name":"audio-wasm","manifest_path":"/r/bindings/audio-wasm/Cargo.toml","targets":[{"kind":["test"],"src_path":"/r/bindings/audio-wasm/tests/data.rs","test":true,"doctest":false}]},{"license":"MIT OR Apache-2.0","name":"audio-testkit","manifest_path":"/r/crates/audio-testkit/Cargo.toml","dependencies":[{"name":"alloc-counter","kind":null,"path":"/r/crates/alloc-counter"}],"targets":[{"kind":["test"],"src_path":"/r/crates/audio-testkit/tests/data.rs","test":true,"doctest":false}]},{"license":"MIT OR Apache-2.0","name":"alloc-counter","manifest_path":"/r/crates/alloc-counter/Cargo.toml","targets":[]},{"license":"MIT OR Apache-2.0","name":"audio-battery","manifest_path":"/r/crates/audio-battery/Cargo.toml","dependencies":[{"name":"audio-core","kind":null,"path":"/r/crates/audio-core"}],"targets":[{"kind":["test"],"src_path":"/r/crates/audio-battery/tests/data.rs","test":true,"doctest":false}]},{"license":"MIT OR Apache-2.0","name":"visual-core","manifest_path":"/r/crates/visual-core/Cargo.toml","dependencies":[{"name":"alloc-counter","kind":"dev","path":"/r/crates/alloc-counter"}],"targets":[{"kind":["test"],"src_path":"/r/crates/visual-core/tests/image.rs","test":true,"doctest":false}]},{"license":"MIT OR Apache-2.0","name":"visual-boxes","manifest_path":"/r/crates/visual-boxes/Cargo.toml","dependencies":[{"name":"visual-core","kind":null,"path":"/r/crates/visual-core"},{"name":"visual-testkit","kind":"dev","path":"/r/crates/visual-testkit"}],"targets":[]},{"license":"MIT OR Apache-2.0","name":"visual-hash","manifest_path":"/r/crates/visual-hash/Cargo.toml","targets":[{"kind":["test"],"src_path":"/r/crates/visual-hash/tests/data.rs","test":true,"doctest":false}]},{"license":"MIT OR Apache-2.0","name":"visual-host","manifest_path":"/r/crates/visual-host/Cargo.toml","dependencies":[{"name":"visual-core","kind":null,"path":"/r/crates/visual-core"}],"targets":[]},{"license":"MIT OR Apache-2.0","name":"visual-testkit","manifest_path":"/r/crates/visual-testkit/Cargo.toml","dependencies":[{"name":"visual-core","kind":null,"path":"/r/crates/visual-core"},{"name":"alloc-counter","kind":null,"path":"/r/crates/alloc-counter"}],"targets":[{"kind":["test"],"src_path":"/r/crates/visual-testkit/tests/data.rs","test":true,"doctest":false}]},{"license":"MIT OR Apache-2.0","name":"visual-wasm","manifest_path":"/r/bindings/visual-wasm/Cargo.toml","dependencies":[{"name":"visual-core","kind":null,"path":"/r/crates/visual-core"},{"name":"visual-testkit","kind":"dev","path":"/r/crates/visual-testkit"}],"targets":[]},{"license":"MIT OR Apache-2.0","name":"visual-cli","manifest_path":"/r/crates/visual-cli/Cargo.toml","dependencies":[{"name":"visual-hash","kind":null,"path":"/r/crates/visual-hash"}],"targets":[{"kind":["test"],"src_path":"/r/crates/visual-cli/tests/data.rs","test":true,"doctest":false}]}]}"#;

/// The two license listings, written out: the production dependencies of what ships
/// (the engine package, the playground, the roadmap and the visual library's package), then
/// everything.
pub(crate) const LICENSE_LINES: [&str; 2] = [
    "pnpm licenses list --json --prod --filter ./bindings/audio-wasm --filter ./bindings/visual-wasm --filter ./bindings/web-page --filter ./apps/playground --filter ./apps/roadmap",
    "pnpm licenses list --json --recursive",
];

/// The two history listings, written out: every message, then the files touched by each
/// commit after the changelog boundary.
pub(crate) const HISTORY_LINES: [&str; 3] = [
    "git log --format=%x1e%H%x1f%B HEAD",
    "git log --format=%x1e%H --name-only a6614dfa18a3b69634f7f581f711d3f509bb5115..HEAD",
    "git log --format=%H 8ff0722ec0273e176d23865891563e8bbe8a1c41..HEAD",
];

/// How the fake lists the changelog diffs of the commits after the breaking rule's
/// boundary, and what it answers: none.
pub(crate) const BREAKING_LISTED: (&str, &str) = (
    "git log --format=%x1e%H%x1f%s --patch --unified=0 --no-color --no-ext-diff --text \
     --no-textconv 8ff0722ec0273e176d23865891563e8bbe8a1c41..HEAD -- CHANGELOG.md",
    "",
);

/// The threads of the fake's machine: six, so that the jobs of a full check (three) differ
/// from it.
const FAKE_PARALLELISM: NonZeroUsize = NonZeroUsize::new(6).expect("six is not zero");

/// The jobs of the fake's checks: half of [`FAKE_PARALLELISM`].
pub(crate) const FAKE_JOBS: NonZeroUsize = NonZeroUsize::new(3).expect("three is not zero");

/// The default check on the fake's machine.
pub(crate) const CHANGES_MODE: Mode = Mode::Changes { jobs: FAKE_JOBS };

/// What the fake's git answers about its tree and its changes, and its compiler and linker
/// driver about themselves.
const GIT_ANSWERS: [(&str, &str); 19] = [
    STAGED,
    UNTRACKED_CODE,
    TREE_STATUS,
    PUSH_STATUS,
    HIDDEN,
    TRACKED,
    HOST,
    LINKER,
    TREE_DIFF,
    TREE_HEAD,
    MERGE_BASE,
    MAIN_BASE,
    PINS_LISTED,
    SPENT_LISTED,
    CHANGELOG_LISTED,
    CHANGES,
    NEW_RUST_FILES,
    COMMON_DIR,
    HEAD_TREE,
];

/// How the fake reads the status that must be empty for a push or a proof record, and what
/// it answers: nothing uncommitted.
pub(crate) const PUSH_STATUS: (&str, &str) = (
    "git status --porcelain --untracked-files=all --ignore-submodules=none",
    "",
);

/// How the fake lists the tracked files with their tags, and what it answers: none hidden.
pub(crate) const HIDDEN: (&str, &str) =
    ("git ls-files -v -z", "H Cargo.toml\0H crates/a/src/b.rs\0");

/// How the fake lists the tracked files for their filters, and what it answers: none, so
/// that no filter is asked ([`super::filtered`]).
pub(crate) const TRACKED: (&str, &str) = ("git ls-files -z", "");

/// How the fake lists the untracked code, test data and settings, and what it answers: none.
pub(crate) const UNTRACKED_CODE: (&str, &str) = (
    "git ls-files -z --others --exclude-per-directory=.gitignore -- *.rs *.ts *.tsx *.mts *.cts *.js *.jsx *.mjs *.cjs *.astro \
     conformance/test-matrix.json spec/box-contract.md spec/benchmark-baseline.json spec/measurement-vectors.json spec/safe-message-vectors.json \
     spec/render-hash-vectors.json conformance/cases/ conformance/patches/ conformance/live-limiter.json \
     conformance/live-references.json conformance/invalid/live/ spec/image-hash-vectors.json \
     conformance/visual/test-matrix.json conformance/visual/ bindings/visual-wasm/benchmarks/suite.ts \
     conformance/visual/display.json spec/benchmarks.md spec/gpu-time-baseline.txt spec/gpu-time-runs.txt conformance/ \
     bindings/audio-wasm/ts/blocks.ts .githooks/ .github/workflows/quality.yml spec/README.md \
     *.npmrc *.pnpmfile.cjs *.pnpmfile.mjs",
    "",
);

/// How the fake lists the index for the bytes of its files, and what it answers: no entry,
/// so that no byte is asked ([`super::unshown`]).
pub(crate) const STAGED: (&str, &str) = ("git ls-files -s -z", "");

/// How the fake's rustc names its host, and what it answers.
pub(crate) const HOST: (&str, &str) = (
    "rustc -vV",
    "rustc 1.99.1 (58cf8f9c3 2026-10-13)
binary: rustc
commit-hash: 58cf8f9c35b355199727627d07ae34a4e2846d67
commit-date: 2026-10-13
host: aarch64-apple-darwin
release: 1.99.1
LLVM version: 23.1.1
",
);

/// How the fake's linker driver names its version, and what it answers.
pub(crate) const LINKER: (&str, &str) = (
    "cc --version",
    "Apple clang version 14.0.3 (clang-1403.0.22.14.1)
Target: arm64-apple-darwin22.6.0
",
);

/// How the fake names git's common directory, and what it answers: the repository's own.
pub(crate) const COMMON_DIR: (&str, &str) = ("git rev-parse --git-common-dir", ".git\n");

/// How the fake reads the tree of the commit checked out, and what it answers.
const HEAD_TREE: (&str, &str) = (
    "git log -1 --format=%T HEAD",
    "def4560000000000000000000000000000000000\n",
);

/// How the fake finds the base of the changes, and what it answers: the upstream resolves.
pub(crate) const MERGE_BASE: (&str, &str) = (
    "git merge-base HEAD @{upstream}",
    "abc1230000000000000000000000000000000000\n",
);

/// How the fake resolves the revision `dsp/block-2`, and what it answers.
pub(crate) const REVISION: (&str, &str) = (
    "git rev-parse --verify --quiet --end-of-options dsp/block-2^{commit}",
    "fed4560000000000000000000000000000000000\n",
);

/// How the fake finds the repository before it resolves a revision, and what it answers.
pub(crate) const GIT_DIR: (&str, &str) = ("git rev-parse --git-dir", ".git\n");

/// The revision `dsp/block-2` as the fake resolves it ([`REVISION`]).
pub(crate) fn fake_revision() -> crate::Revision {
    crate::Revision {
        name: "dsp/block-2".to_owned(),
        commit: "fed4560000000000000000000000000000000000".to_owned(),
    }
}

/// How the fake lists the mutants of the changes, and what it answers: two mutants.
pub(crate) const LISTED: (&str, &str) = (
    "cargo mutants --no-config --output=. --cap-lints=true --gitignore=true --cargo-arg=--locked --cargo-arg=--config=profile.test.debug=false --cargo-arg=--config=target.'cfg(target_os = \"linux\")'.runner=[\"prlimit\", \"--as=4294967296\"] --test-tool=nextest --cargo-test-arg=--max-fail=1:immediate --cargo-test-arg=--no-tests=warn --workspace --in-diff target/xtask/changes.diff --list -- -- --include-ignored",
    "crates/a/src/b.rs:1:16: replace f -> u8 with 0\ncrates/a/src/b.rs:1:16: replace f -> u8 with 1\n",
);

/// Adds to `fake` the answers of `cargo xtask mutants --changed dsp/block-2`: git finds the
/// repository ([`GIT_DIR`]), the revision resolves ([`REVISION`]), its merge base with `HEAD` is [`MERGE_BASE`]'s base, described,
/// and cargo-mutants lists two mutants of the changes ([`LISTED`]).
pub(crate) fn revision_answers(fake: &mut Fake) {
    for (command, answer) in [
        GIT_DIR,
        REVISION,
        (
            "git merge-base HEAD fed4560000000000000000000000000000000000",
            "abc1230000000000000000000000000000000000\n",
        ),
        (
            "git show -s --format=%h %s abc1230000000000000000000000000000000000",
            "abc1230 feat: the base\n",
        ),
        LISTED,
    ] {
        let _answer = fake
            .files
            .insert(format!("capture {command}"), answer.to_owned());
    }
}

/// How the fake reads the changes since [`MERGE_BASE`], and what it answers: one changed
/// line of a Rust file.
pub(crate) const CHANGES: (&str, &str) = (
    "git -c diff.suppressBlankEmpty=false diff --no-color --no-ext-diff --no-textconv --text --no-renames --unified=3 --inter-hunk-context=0 --diff-algorithm=myers --indent-heuristic --src-prefix=a/ --dst-prefix=b/ abc1230000000000000000000000000000000000 -- *.rs",
    "diff --git a/crates/a/src/b.rs b/crates/a/src/b.rs\n--- a/crates/a/src/b.rs\n+++ b/crates/a/src/b.rs\n@@ -1 +1 @@\n-fn f() {}\n+fn f() -> u8 { 1 }\n",
);

/// How the fake lists the new Rust files, and what it answers: none.
pub(crate) const NEW_RUST_FILES: (&str, &str) = (
    "git ls-files -z --others --exclude-per-directory=.gitignore -- *.rs",
    "",
);

/// A cargo message of a build of `target` of the crate in `dir` (below the root `/r`),
/// with tests or not, whose first output is `output`.
pub(crate) fn artifact(dir: &str, target: &str, test: bool, output: &str) -> String {
    format!(
        r#"{{"reason":"compiler-artifact","package_id":"path+file:///r/{dir}#0.1.0","manifest_path":"/r/{dir}/Cargo.toml","target":{{"kind":["lib"],"name":"{target}"}},"profile":{{"test":{test}}},"filenames":["{output}"],"fresh":true}}"#
    )
}

/// A dep-info file of rustc for the output `output`, listing `files`.
pub(crate) fn dep_info(output: &str, files: &[&str]) -> String {
    let listed = files.join(" ");
    let phony = files.iter().fold(String::new(), |mut phony, file| {
        phony.push_str(file);
        phony.push_str(":\n");
        phony
    });
    format!(
        "{output}.d: {listed}\n\n{output}: {listed}\n\n{phony}\n# env-dep:CARGO_MANIFEST_DIR=/r\n"
    )
}

/// Gives the fake the builds of [`super::loaded_files_commands`]: of each, one of the
/// assembly file's crate (with its tests for the second, the host's test build, without
/// for the others), each reading the assembly file only, and a message of no build and one
/// of a registry package between them.
fn loaded_answers(fake: &mut Fake) {
    let word = super::ASSEMBLY_FILES[0];
    let registry = r#"{"reason":"compiler-artifact","package_id":"registry+https://github.com/rust-lang/crates.io-index#serde@1.0.0","profile":{"test":false}}"#;
    for (number, command) in (1_usize..).zip(super::loaded_files_commands()) {
        let tested = number == 2;
        let stem = format!("/t/debug/deps/fpenv-{number}");
        let output = if tested {
            stem.clone()
        } else {
            format!("/t/debug/deps/libfpenv-{number}.rmeta")
        };
        let answer = format!(
            "{registry}\n{}\n{{\"reason\":\"build-finished\",\"success\":true}}\n",
            artifact("crates/fpenv", "fpenv", tested, &output)
        );
        assert!(
            fake.files
                .insert(format!("stdout {}", command.cmd.line()), answer)
                .is_none(),
            "fresh fake"
        );
        assert!(
            fake.files
                .insert(format!("{stem}.d"), dep_info(&stem, &[word]))
                .is_none(),
            "fresh fake"
        );
    }
}

/// What the doc tests of the fixture say when rustdoc checks the error codes: they fail.
pub(crate) const DOC_CODES_FAILED: &str = "test src/lib.rs - (line 4) - compile fail ... \
    FAILED\n\nfailures:\n\n---- src/lib.rs - (line 4) stdout ----\nSome expected error codes \
    were not found: [\"E0599\"]\n";

/// The fake's doc tests of the fixture fail as they must, saying [`DOC_CODES_FAILED`].
fn doc_codes_answers(fake: &mut Fake) {
    let line = super::doc_codes_fixture_command().line();
    for (key, answer) in [
        (format!("fails {line}"), String::new()),
        (format!("stdout {line}"), DOC_CODES_FAILED.to_owned()),
    ] {
        assert!(fake.files.insert(key, answer).is_none(), "fresh fake");
    }
}

/// Records commands and answers with scripted results.
#[derive(Debug, Default)]
pub(crate) struct Fake {
    pub(crate) ran: Vec<String>,
    pub(crate) fail: Option<String>,
    /// A command that exits with code 2, as a usage error.
    pub(crate) refuse: Option<String>,
    pub(crate) broken: Option<String>,
    /// How many of the calls `broken` or `fail` names are answered before it breaks or fails
    /// one: 0 breaks or fails the first, and so each one after.
    pub(crate) spared: usize,
    pub(crate) killed: Option<String>,
    /// A command that, when it runs, rewrites a file: (command, file, new text).
    pub(crate) rewrite: Option<(String, String, String)>,
    pub(crate) files: BTreeMap<String, String>,
    /// What [`Runner::environment`] answers.
    pub(crate) environment: super::Environment,
    /// Its clock: [`FAKE_TICK`] more at each reading.
    pub(crate) clock: Duration,
}

/// How much the fake's clock advances at each reading: every step of its checks takes that
/// long, and a run of `n` steps `n` times as long.
const FAKE_TICK: Duration = Duration::from_millis(1_500);

impl Fake {
    /// Whether the call `line` is broken: named by `broken`, once `spared` such calls were
    /// answered.
    fn breaks(&mut self, line: &str) -> bool {
        if self.broken.as_deref() != Some(line) {
            return false;
        }
        let Some(left) = self.spared.checked_sub(1) else {
            return true;
        };
        self.spared = left;
        false
    }

    /// Whether the call `line` ends in failure: named by `fail`, once `spared` such calls were
    /// answered.
    fn fails(&mut self, line: &str) -> bool {
        if self.fail.as_deref() != Some(line) {
            return false;
        }
        let Some(left) = self.spared.checked_sub(1) else {
            return true;
        };
        self.spared = left;
        false
    }
}

impl Runner for Fake {
    fn run(&mut self, cmd: &Cmd) -> Result<(), CommandFailure> {
        let line = cmd.line();
        self.ran.push(line.clone());
        if let Some((command, file, text)) = &self.rewrite
            && *command == line
        {
            let _previous = self.files.insert(file.clone(), text.clone());
        }
        let failed = if self.breaks(line.as_str()) {
            Failed::NotStarted("not found".to_owned())
        } else if self.killed.as_deref() == Some(line.as_str()) {
            Failed::Ended {
                exit: Exit::Signal(9_i32),
                reason: None,
            }
        } else if self.refuse.as_deref() == Some(line.as_str())
            || self.files.contains_key(&format!("refuses {line}"))
        {
            // A command the fake holds a file `refuses <command line>` for exits with 2 too:
            // a second command of a test that must exit as the first does.
            Failed::Ended {
                exit: Exit::Code(2_i32),
                reason: None,
            }
        } else if self.fails(line.as_str()) || self.files.contains_key(&format!("fails {line}")) {
            // A failing command exits with 3: a code that is neither 0 nor 1, so that the
            // message is seen to report the real one. A command the fake holds a file
            // `fails <command line>` for fails too: one a passing check needs to fail.
            Failed::Ended {
                exit: Exit::Code(3_i32),
                reason: None,
            }
        } else {
            return Ok(());
        };
        Err(CommandFailure { line, failed })
    }

    /// Runs `cmd` as [`Runner::run`] does, its stdout the file named `stdout <command
    /// line>` and its stderr the file `stderr <command line>`, or nothing; a command that
    /// ends with a code gives the reason its stderr holds, as a real one does, and one that
    /// succeeds is refused as not text when the fake holds a file `not text <command line>`.
    fn run_for_stdout(&mut self, cmd: &Cmd) -> Ran {
        let line = cmd.line();
        let ended = self.run(cmd).and_then(|()| {
            if self.files.contains_key(&format!("not text {line}")) {
                Err(CommandFailure::of(cmd, Failed::NotText))
            } else {
                Ok(())
            }
        });
        let read = |stream: &str| {
            self.files
                .get(&format!("{stream} {line}"))
                .cloned()
                .unwrap_or_default()
        };
        let (stdout, stderr) = (read("stdout"), read("stderr"));
        let ended = ended.map_err(|failure| match failure.failed {
            Failed::Ended { exit, .. } => CommandFailure {
                failed: Failed::Ended {
                    exit,
                    reason: super::error_reason(stderr.as_bytes()),
                },
                ..failure
            },
            Failed::NotStarted(_) | Failed::NotText => failure,
        });
        Ran {
            stdout,
            stderr,
            ended,
        }
    }

    /// Answers with the file, fails as absent, as not UTF-8 when the test breaks
    /// `read <path>`, or as denied when the fake holds a file `denies <path>`.
    fn read(&mut self, path: &str) -> io::Result<String> {
        self.ran.push(format!("read {path}"));
        if self.breaks(&format!("read {path}")) {
            return Err(io::Error::new(io::ErrorKind::InvalidData, "not UTF-8"));
        }
        if self.files.contains_key(&format!("denies {path}")) {
            return Err(io::Error::new(io::ErrorKind::PermissionDenied, "denied"));
        }
        self.files
            .get(path)
            .cloned()
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "missing"))
    }

    /// Records the file among its files, or fails when the test breaks `write <path>`.
    fn write(&mut self, path: &str, text: &str) -> io::Result<()> {
        let line = format!("write {path}");
        self.ran.push(line.clone());
        if self.breaks(line.as_str()) {
            return Err(io::Error::other("read-only"));
        }
        let _previous = self.files.insert(path.to_owned(), text.to_owned());
        Ok(())
    }

    /// A machine of [`FAKE_PARALLELISM`] threads.
    fn parallelism(&mut self) -> NonZeroUsize {
        FAKE_PARALLELISM
    }

    fn environment(&mut self) -> super::Environment {
        self.environment.clone()
    }

    fn clock(&mut self) -> Duration {
        self.clock = self.clock.saturating_add(FAKE_TICK);
        self.clock
    }

    fn fresh_dir(&mut self, path: &str) -> io::Result<()> {
        let line = format!("fresh {path}");
        self.ran.push(line.clone());
        if self.breaks(line.as_str()) {
            return Err(io::Error::other("busy"));
        }
        Ok(())
    }

    /// Answers a captured command with the file named `capture <command line>`, or fails
    /// with code 1 saying "failed". The tools of the preflight answer with a version unless
    /// the test breaks them.
    fn capture(&mut self, cmd: &Cmd) -> Result<String, CommandFailure> {
        let line = cmd.line();
        self.ran.push(format!("capture {line}"));
        if self.breaks(line.as_str()) {
            return Err(CommandFailure::of(
                cmd,
                Failed::NotStarted("not found".to_owned()),
            ));
        }
        // A command set to fail ends in failure saying nothing, before any answer the fake
        // would give (a tool's version included).
        if self.fails(line.as_str()) {
            return Err(CommandFailure::of(
                cmd,
                Failed::Ended {
                    exit: Exit::Code(3_i32),
                    reason: None,
                },
            ));
        }
        let tool = super::TOOLS.iter().any(|tool| {
            tool.program == cmd.program
                && tool
                    .args
                    .iter()
                    .copied()
                    .eq(cmd.args.iter().map(String::as_str))
        });
        if tool {
            return Ok(tool_answer(
                cmd.program,
                cmd.args.first().map(String::as_str),
            ));
        }
        // The diff a test scripted holds every file; git shows only its Rust files.
        if cmd.args.last().is_some_and(|arg| arg == "*.rs")
            && let Some(rust) = self.git_from_its_diff(cmd)
        {
            return Ok(rust);
        }
        self.files
            .get(&format!("capture {line}"))
            .cloned()
            .or_else(|| self.git_from_its_diff(cmd))
            .ok_or_else(|| {
                CommandFailure::of(
                    cmd,
                    Failed::Ended {
                        exit: Exit::Code(1_i32),
                        reason: Some("failed".to_owned()),
                    },
                )
            })
    }

    /// Matches the pathspecs this crate uses, `*<suffix>`, against its files (the answers of
    /// captured commands apart).
    fn tracked(&mut self, patterns: &[&str]) -> Result<Vec<String>, CommandFailure> {
        let line = format!("list {}", patterns.join(" "));
        self.ran.push(line.clone());
        if self.breaks(line.as_str()) {
            return Err(CommandFailure {
                line: format!("git ls-files -- {}", patterns.join(" ")),
                failed: Failed::Ended {
                    exit: Exit::Code(128_i32),
                    reason: Some("fatal: not a git repository".to_owned()),
                },
            });
        }
        Ok(self
            .files
            .keys()
            .filter(|path| tracked_in_fake(path))
            // Nor the new files the scripted answer of git lists as untracked.
            .filter(|path| {
                !self
                    .files
                    .get(&format!("capture {}", NEW_RUST_FILES.0))
                    .is_some_and(|new| new.split('\0').any(|listed| listed == path.as_str()))
            })
            .filter(|path| {
                // A pattern without `*` names one file, as git takes it.
                patterns.contains(&path.as_str())
                    || patterns
                        .iter()
                        .filter_map(|pattern| pattern.split_once('*'))
                        .any(|(prefix, suffix)| path.starts_with(prefix) && path.ends_with(suffix))
            })
            .cloned()
            .collect())
    }
}

impl Fake {
    /// What git answers to `cmd`, unscripted, from the diff a test scripted for the same
    /// commit ([`diff_since`]), as real git does: the paths of its sections, NUL-separated
    /// (`diff --name-only -z`), or the sections of its Rust files (`*.rs`) but those below
    /// the directories excluded by `:(exclude)<dir>/` pathspecs and the files excluded by
    /// `:(exclude,literal)<file>` ones.
    fn git_from_its_diff(&self, cmd: &Cmd) -> Option<String> {
        let args: Vec<&str> = cmd.args.iter().map(String::as_str).collect();
        if cmd.program != "git" {
            return None;
        }
        match args.as_slice() {
            [
                "diff",
                "--no-color",
                "--name-only",
                "-z",
                "--no-renames",
                since,
            ] => Some(
                diff_sections(self.files.get(&diff_since(since))?)
                    .into_iter()
                    .fold(String::new(), |mut listed, (path, _)| {
                        listed.push_str(&path);
                        listed.push('\0');
                        listed
                    }),
            ),
            [
                "-c",
                "diff.suppressBlankEmpty=false",
                "diff",
                "--no-color",
                "--no-ext-diff",
                "--no-textconv",
                "--text",
                "--no-renames",
                "--unified=3",
                "--inter-hunk-context=0",
                "--diff-algorithm=myers",
                "--indent-heuristic",
                "--src-prefix=a/",
                "--dst-prefix=b/",
                since,
                "--",
                "*.rs",
                excluded @ ..,
            ] => {
                // A directory excluded by `:(exclude)<dir>/`, a file by
                // `:(exclude,literal)<file>`, exactly.
                let excluded: Vec<(&str, bool)> = excluded
                    .iter()
                    .map(|pathspec| {
                        pathspec
                            .strip_prefix(":(exclude)")
                            .map(|dir| (dir, false))
                            .or_else(|| {
                                pathspec
                                    .strip_prefix(":(exclude,literal)")
                                    .map(|file| (file, true))
                            })
                    })
                    .collect::<Option<_>>()?;
                Some(
                    diff_sections(self.files.get(&diff_since(since))?)
                        .into_iter()
                        .filter(|(path, _)| {
                            Path::new(path)
                                .extension()
                                .is_some_and(|extension| extension == "rs")
                                && !excluded.iter().any(|(spec, literal)| {
                                    if *literal {
                                        path == spec
                                    } else {
                                        path.starts_with(spec)
                                    }
                                })
                        })
                        .map(|(_, section)| section)
                        .collect(),
                )
            }
            _ => None,
        }
    }
}

/// The sections of `diff`, a diff a test scripted, each with the path it changes.
fn diff_sections(diff: &str) -> Vec<(String, String)> {
    let mut sections: Vec<(String, String)> = Vec::new();
    for line in diff.split_inclusive('\n') {
        if let Some(rest) = line.strip_prefix("diff --git a/") {
            let (path, _) = rest
                .split_once(" b/")
                .expect("a scripted diff names its path");
            sections.push((path.to_owned(), String::new()));
        }
        if let Some((_, section)) = sections.last_mut() {
            section.push_str(line);
        }
    }
    sections
}

/// One file of an llvm-cov JSON summary: its path and its (count, covered) lines,
/// functions and branches.
type Measure<'path> = (&'path str, (u64, u64), (u64, u64), (u64, u64));

/// An llvm-cov JSON summary of these files, with their absolute paths.
pub(crate) fn report(files: &[Measure<'_>]) -> String {
    let entry = |(path, lines, functions, branches): &Measure<'_>| {
        format!(
            r#"{{"filename":"/repo/{path}","summary":{{"lines":{{"count":{},"covered":{}}},"functions":{{"count":{},"covered":{}}},"branches":{{"count":{},"covered":{}}}}}}}"#,
            lines.0, lines.1, functions.0, functions.1, branches.0, branches.1
        )
    };
    format!(
        r#"{{"data":[{{"files":[{}]}}]}}"#,
        files.iter().map(entry).collect::<Vec<_>>().join(",")
    )
}

/// A report covering the measured file of `coverage` fully.
pub(crate) fn full_report(coverage: Coverage) -> String {
    let file = match coverage.measured {
        Measured::Library => "lib.rs",
        Measured::Binary => "main.rs",
    };
    report(&[(
        &format!("{}/src/{file}", coverage.krate.dir),
        (10, 10),
        (3, 3),
        (4, 4),
    )])
}

/// An attribute lifting the lint against unsafe code, built so that this file does not
/// contain the lint name itself (the unsafe gate reads it too).
pub(crate) fn lifting() -> String {
    format!(
        "#![expect(\n    {}_{},\n    reason = \"audited\"\n)]\n",
        "unsafe", "code"
    )
}

/// The line an assembly file must hold, written out apart from
/// [`crate::ONE_UNSAFE_OPERATION`] so that the tests do not take it from the code they test.
pub(crate) const ONE_UNSAFE_OPERATION: &str = "#![forbid(clippy::multiple_unsafe_ops_per_block)]";

/// The text of the audited file `path`: [`lifting`], and in the files of `ASSEMBLY_FILES`
/// the forbid of [`ONE_UNSAFE_OPERATION`] and inline assembly, in a library root
/// [`doc_tests_forbid`].
pub(crate) fn audited(path: &str) -> String {
    if super::ASSEMBLY_FILES.contains(&path) {
        format!(
            "{}{ONE_UNSAFE_OPERATION}\nfn f() {{\n    unsafe {{\n        ::core::arch::asm!(\"nop\");\n    }}\n}}\n",
            lifting()
        )
    } else if path.ends_with("/src/lib.rs") {
        format!("{}{}", lifting(), doc_tests_forbid())
    } else {
        lifting()
    }
}

/// The inner attribute every library root holds, so that its doc tests forbid unsafe code,
/// built so that this file does not hold it whole.
pub(crate) fn doc_tests_forbid() -> String {
    format!("#![doc(test(attr(forbid({}_{}))))]\n", "unsafe", "code")
}

/// The licenses of what the apps ship, all allowed.
pub(crate) const SHIPPED: &str =
    r#"{"MIT":[{"name":"react"},{"name":"react-dom"}],"ISC":[{"name":"a"}]}"#;

/// The licenses of all tooling, all allowed (sharp's library by its exception).
pub(crate) const TOOLING: &str = r#"{"Apache-2.0":[{"name":"typescript"}],"MPL-2.0":[{"name":"axe-core"}],"LGPL-3.0-or-later":[{"name":"@img/sharp-libvips-linux-x64"}]}"#;

/// A Vitest coverage summary at 100%.
pub(crate) const TS_COVERED: &str = r#"{"total":{"lines":{"total":4,"covered":4},"statements":{"total":5,"covered":5},"functions":{"total":2,"covered":2},"branches":{"total":3,"covered":3}}}"#;

/// A Stryker report where every mutant was killed (or timed out, or was disabled).
const TS_KILLED: &str = r#"{"files":{"src/a.ts":{"mutants":[{"status":"Killed","mutatorName":"A","location":{"start":{"line":1}}},{"status":"Timeout","mutatorName":"B","location":{"start":{"line":2}}}]}}}"#;

/// A Stryker report of the shared policy: killed mutants, and one ignored where the
/// reviewed disable comment of [`DISABLED_SOURCE`] says so.
pub(crate) const TS_KILLED_OR_DISABLED: &str = r#"{"files":{"src/a.ts":{"mutants":[{"status":"Killed","mutatorName":"A","location":{"start":{"line":1}}}]},"x.ts":{"mutants":[{"status":"Ignored","mutatorName":"EqualityOperator","location":{"start":{"line":3}}}]}}}"#;

/// A TypeScript source of the shared policy whose third line disables one mutator.
pub(crate) const DISABLED_SOURCE: (&str, &str) = (
    "tooling/config/x.ts",
    "const a = 1;\n// Stryker disable next-line EqualityOperator: same result\nwhile (a < 2) {}\n",
);

/// A Stryker configuration that hides nothing.
pub(crate) const STRYKER_CONFIG: &str = r#"{"$schema":"s","testRunner":"vitest","mutate":["src/**/*.ts"],"reporters":["json"],"thresholds":{"break":100}}"#;

/// Gives the fake a toolchain whose float sources are consistent with its `clippy.toml`.
fn float_sources(fake: &mut Fake) {
    let (sources, clippy) = float_fixture();
    let sysroot = fake.files.insert(
        "capture rustc +nightly-2026-11-01 --print sysroot".to_owned(),
        format!("{FAKE_SYSROOT}\n"),
    );
    let version = fake.files.insert(
        "capture rustc +nightly-2026-11-01 --version".to_owned(),
        FAKE_NIGHTLY.to_owned(),
    );
    assert!(version.is_none(), "fresh fake");
    let banned = fake.files.insert(super::CLIPPY_TOML.to_owned(), clippy);
    assert!(sysroot.is_none() && banned.is_none(), "fresh fake");
    for (path, text) in sources {
        let source = fake.files.insert(path, text);
        assert!(source.is_none(), "fresh fake");
    }
}

/// The lints every root of both libraries denies, written out apart from
/// [`crate::LIBRARY_LINTS`] so that the tests do not take them from the code they test.
pub(crate) const LIBRARY_LINTS_DENIED: &str =
    "#![deny(clippy::exhaustive_enums)]\n#![deny(clippy::exhaustive_structs)]\n";

/// Every root of both libraries in the fake, each denying [`LIBRARY_LINTS_DENIED`]:
/// added to the roots other sources of the fake hold, written for the others (forbidding
/// unsafe code, in their doc tests too).
fn library_roots(fake: &mut Fake) {
    for krate in super::RUST_CRATES.iter().filter(|krate| {
        krate.roles.contains(&super::CrateRole::Library)
            || krate.roles.contains(&super::CrateRole::VisualLibrary)
    }) {
        let root = fake
            .files
            .entry(format!("{}/src/lib.rs", krate.dir))
            .or_insert_with(|| {
                format!(
                    "#![forbid({}_{})]\n{}",
                    "unsafe",
                    "code",
                    doc_tests_forbid()
                )
            });
        root.insert_str(0, LIBRARY_LINTS_DENIED);
    }
    // Each `no_std` crate not built for every portability target, listed with its reason,
    // which the portability gate reads as `no_std`.
    for (name, _) in super::NO_STD_NOT_PORTABLE {
        let krate = super::RUST_CRATES
            .iter()
            .find(|krate| krate.name == name)
            .expect("a crate of the registry");
        let root = fake
            .files
            .entry(format!("{}/src/lib.rs", krate.dir))
            .or_insert_with(|| {
                format!(
                    "#![forbid({}_{})]\n{}",
                    "unsafe",
                    "code",
                    doc_tests_forbid()
                )
            });
        root.insert_str(0, "#![no_std]\n");
    }
}

/// For each crate that declares test data, an integration test reading every root of it
/// (path and text).
pub(crate) fn test_data_reader_sources() -> Vec<(String, String)> {
    let mut sources = Vec::new();
    for krate in super::RUST_CRATES {
        if krate.test_data.is_empty() {
            continue;
        }
        let mut text = format!("#![forbid({}_{})]\n", "unsafe", "code");
        for root in krate.test_data {
            text.push_str("fn f() { let _ = Path::new(env!(\"CARGO_MANIFEST_DIR\")).join(\"../../");
            text.push_str(root);
            text.push_str("\"); }\n");
        }
        sources.push((format!("{}/tests/data.rs", krate.dir), text));
    }
    sources
}

/// Adds to the fake [`test_data_reader_sources`].
fn test_data_readers(fake: &mut Fake) {
    for (path, text) in test_data_reader_sources() {
        assert!(fake.files.insert(path, text).is_none(), "fresh fake");
    }
}

/// The unviable mutants of a passing run, their reviews, and the list of every mutant.
fn unviable_answers(fake: &mut Fake) {
    let report = "crates/a/src/b.rs:3:5: replace f -> T with Default::default()\n";
    let reviewed =
        "crates/a/src/b.rs: replace f -> T with Default::default() -- T has no Default\n";
    for (path, text) in [
        (UNVIABLE_REPORT.to_owned(), report.to_owned()),
        (UNVIABLE_REVIEWED.to_owned(), reviewed.to_owned()),
        // The same reviews at the merge base with main: none added since the last version.
        (
            format!(
                "capture git show {}:{UNVIABLE_REVIEWED}",
                MAIN_BASE.1.trim()
            ),
            reviewed.to_owned(),
        ),
        (
            format!("capture {}", super::mutants_list_command().line()),
            format!("{report}crates/a/src/b.rs:9:1: replace g with ()\n"),
        ),
        (
            format!("capture {}", super::gpu_mutants_list_command().line()),
            "src/lib.rs:4:1: replace h with ()\n".to_owned(),
        ),
    ] {
        assert!(fake.files.insert(path, text).is_none(), "fresh fake");
    }
}

/// The fake's answers to the history listings: [`HISTORY_LINES`], then [`BREAKING_LISTED`].
fn history_answers(fake: &mut Fake) {
    for (line, text) in HISTORY_LINES.into_iter().chain([BREAKING_LISTED.0]).zip([
        HISTORY,
        HISTORY_FILES,
        HISTORY_SCOPED,
        BREAKING_LISTED.1,
    ]) {
        let logged = fake
            .files
            .insert(format!("capture {line}"), text.to_owned());
        assert!(logged.is_none(), "fresh fake");
    }
}

/// Each workspace's nextest configuration: the root's slow group of one test, the GPU
/// workspace's of none.
fn nextest_configs(fake: &mut Fake) {
    for (workspace, text) in [
        (
            super::CargoWorkspace::Root,
            "# The slow group.\n\n[[profile.default.overrides]]\nfilter = \
             'binary_id(=audio-core) & test(=limiter::tests::holds)'\npriority = -1\n",
        ),
        (super::CargoWorkspace::Gpu, "# No slow test.\n"),
    ] {
        let config = fake
            .files
            .insert(workspace.nextest_config().to_owned(), text.to_owned());
        assert!(config.is_none(), "fresh fake");
    }
}

/// The fused test build's answer in a passing run: its one test, which proves the option.
fn fused_answer(fake: &mut Fake) {
    let fused = fake.files.insert(
        format!("capture {}", super::fused_command().line()),
        format!("running 1 test\n{}\n", super::FUSED_PROOF),
    );
    assert!(fused.is_none(), "fresh fake");
}

/// What the visual corpus's mutation step reads in a passing fake: no untracked file (so
/// that the default check's step sees only the scripted diff, which reaches no input of
/// it), one mutant missed, its review, and the test of the mutated crate it names.
fn corpus_answers(fake: &mut Fake) {
    let sources = corpus_sources().join("\0");
    for (path, text) in [
        (UNTRACKED, ""),
        (CORPUS_SOURCES_LISTED, sources.as_str()),
        (
            super::CORPUS_MISSED,
            "crates/visual-core/src/image.rs:76:9: replace ImageSize::point -> Option<SubpixelPoint> with None\n",
        ),
        (
            super::CORPUS_EXEMPT,
            "# Reviewed.\ncrates/visual-core/src/image.rs: replace ImageSize::point -> Option<SubpixelPoint> with None -- traced, never rendered; killed by traces_a_point_of_its_image\n",
        ),
        (
            "crates/visual-core/tests/image.rs",
            "#![forbid(unsafe_code)]\n#[test]\nfn traces_a_point_of_its_image() {}\n",
        ),
    ] {
        let fresh = fake.files.insert(path.to_owned(), text.to_owned());
        assert!(fresh.is_none(), "fresh fake");
    }
}

/// An empty fake on a Linux host, CI's, whatever machine runs the tests: every tool of the
/// preflight is then needed.
fn on_linux() -> Fake {
    let mut fake = Fake::default();
    fake.environment.os = "linux";
    fake
}

pub(crate) fn passing() -> Fake {
    let mut fake = on_linux();
    doc_codes_answers(&mut fake);
    fused_answer(&mut fake);
    loaded_answers(&mut fake);
    coverage_reports(&mut fake);
    unviable_answers(&mut fake);
    test_data_readers(&mut fake);
    let equivalent = "tooling/config/x.ts: EqualityOperator at while (a < 2) {} -- same result\n";
    typescript_reports(&mut fake);
    for (command, answer) in GIT_ANSWERS {
        let recorded = fake
            .files
            .insert(format!("capture {command}"), answer.to_owned());
        assert!(recorded.is_none(), "fresh fake");
    }
    corpus_answers(&mut fake);
    for (line, licenses) in LICENSE_LINES.iter().zip([SHIPPED, TOOLING]) {
        let replaced = fake
            .files
            .insert(format!("capture {line}"), licenses.to_owned());
        assert!(replaced.is_none(), "fresh fake");
    }
    for path in UNSAFE_FILES {
        let previous_unsafe = fake.files.insert(path.to_owned(), audited(path));
        assert!(previous_unsafe.is_none(), "fresh fake");
    }
    float_sources(&mut fake);
    nextest_configs(&mut fake);
    history_answers(&mut fake);
    for (path, text) in [
        (super::WORKFLOW, QUALITY),
        (super::NODE_VERSION_FILE, "22.23.4\n"),
        (super::RUST_TOOLCHAIN_FILE, TOOLCHAIN),
    ] {
        let pinned = fake.files.insert(path.to_owned(), text.to_owned());
        assert!(pinned.is_none(), "fresh fake");
    }
    for (path, text) in readmes() {
        let readme = fake.files.insert(path, text);
        assert!(readme.is_none(), "fresh fake");
    }
    let third = fake
        .files
        .insert(DISABLED_SOURCE.0.to_owned(), DISABLED_SOURCE.1.to_owned());
    let bypasses = fake.files.insert(
        super::BYPASSES_REVIEWED.to_owned(),
        SCOPE_REVIEWED.to_owned(),
    );
    assert!(bypasses.is_none(), "fresh fake");
    let fourth = fake
        .files
        .insert(EQUIVALENT_REVIEWED.to_owned(), equivalent.to_owned());
    assert!(third.is_none() && fourth.is_none(), "fresh fake");
    for (path, text) in [
        (CARGO_TOML.to_owned(), CARGO),
        (CHANGELOG.to_owned(), LOG),
        (
            "COMMIT_EDITMSG".to_owned(),
            "feat(audio-core): add the plan\n",
        ),
        (super::CARGO_CONFIG_FILE.to_owned(), super::CARGO_CONFIG),
        (
            super::GIT_ATTRIBUTES.to_owned(),
            "* text=auto eol=lf\n*.wasm binary\n",
        ),
        (super::ROADMAP_BOARD.to_owned(), BOARD),
        (super::VOCABULARY.to_owned(), FAKE_WORDS),
        (
            super::RETIRED_PATTERNS.to_owned(),
            "retire `fake retired pattern` -- the fake list\n",
        ),
        (super::DEPENDABOT.to_owned(), super::DEPENDABOT_CONFIG),
        (super::PNPM_WORKSPACE.to_owned(), PNPM),
        (super::DENY_TOML.to_owned(), super::DENY_CONFIG),
        (
            super::DEPENDENCIES_REVIEWED.to_owned(),
            "x: serde_json (normal) -- reads JSON\n",
        ),
        (
            format!(
                "capture {}",
                super::metadata_command(super::CargoWorkspace::Root).line()
            ),
            METADATA,
        ),
    ]
    .into_iter()
    .chain(build_answers())
    .chain(gpu_workspace_files())
    .chain(LICENSE_TEXTS.map(|(path, text)| (path.to_owned(), text)))
    .chain(package_jsons().into_iter().map(fake_package))
    .chain(super::RUST_CRATES.iter().map(member_manifest))
    {
        let earlier = fake.files.insert(path, text.to_owned());
        assert!(earlier.is_none(), "fresh fake");
    }
    let mut fake = with_math_layer_named(fake);
    library_roots(&mut fake);
    shader_constants(&mut fake);
    with_package_checks(fake)
}

/// Each shader's constant, as the GPU workspace's shader test names it, defined in its
/// crate's root as the validated shaders gate requires: `include_str!` of the shader.
fn shader_constants(fake: &mut Fake) {
    for (root, constant, file) in [
        (
            "crates/visual-boxes/src/lib.rs",
            "SQUARE_WGSL",
            "square.wgsl",
        ),
        (
            "crates/visual-core/src/lib.rs",
            "DISPLAY_WGSL",
            "display.wgsl",
        ),
    ] {
        fake.files
            .get_mut(root)
            .expect("a root of the visual library")
            .push_str(
                &[
                    "pub const ",
                    constant,
                    ": &str = include_str!(\"",
                    file,
                    "\");\n",
                ]
                .concat(),
            );
    }
}

/// The files the math layer gate names, held by `fake` as the gate requires them (unless a
/// source is already there): a box root holding its forbid, reviewed as a bypass as the real
/// one is, every crate root forbidding unsafe code, the math layer empty.
fn with_math_layer_named(mut fake: Fake) -> Fake {
    for named in super::MATH_LAYER_NAMED {
        let text = if super::BOX_ROOT == named || super::VISUAL_BOX_ROOT == named {
            // Its forbid is a bypass the bypass gate needs reviewed, as the real one is.
            if let Some(reviewed) = fake.files.get_mut(super::BYPASSES_REVIEWED) {
                // The attribute's name built in two parts, so that this line is no bypass.
                *reviewed = format!(
                    "{reviewed}{named}: {}_attr at {} -- a lint level only\n",
                    "cfg",
                    super::BOX_FLOAT_FORBID
                );
            }
            format!("{}\n", super::BOX_FLOAT_FORBID)
        } else {
            String::new()
        };
        // A crate root also forbids unsafe code, in its doc tests too, as every root must.
        let root = if named.ends_with("/lib.rs") {
            "#![forbid(unsafe_code)]\n#![doc(test(attr(forbid(unsafe_code))))]\n"
        } else {
            ""
        };
        let _source = fake
            .files
            .entry(named.to_owned())
            .or_insert_with(|| format!("{root}{text}"));
    }
    fake
}

/// The workspace packages each package of [`PACKAGES`] depends on, as its real manifest
/// names them.
pub(crate) fn workspace_dependencies(dir: &str) -> &'static [&'static str] {
    match dir {
        "tooling/oracle" | "tooling/wasm-build" | "apps/roadmap" => &["@poyetik/config"],
        "bindings/audio-wasm" | "bindings/visual-wasm" => {
            &["@poyetik/config", "@poyetik/oracle", "@poyetik/wasm-build"]
        }
        "bindings/web-page" => &[
            "@poyetik/config",
            "@poyetik/visual-wasm",
            "@poyetik/wasm-build",
        ],
        "apps/playground" => &[
            "@poyetik/audio-wasm",
            "@poyetik/config",
            "@poyetik/web-page",
        ],
        _ => &[],
    }
}

/// `fake`, with each package's manifest running the scripts its checks run, named after its
/// directory and depending on the workspace packages its real one does.
fn with_package_checks(mut fake: Fake) -> Fake {
    for package in PACKAGES {
        let (path, text) = fake_package(format!("{}/package.json", package.dir));
        let check = super::check_script(package.checks);
        let name = package.dir.rsplit('/').next().unwrap_or_default();
        let dependencies = workspace_dependencies(package.dir)
            .iter()
            .map(|dependency| format!("\"{dependency}\": \"workspace:*\""))
            .collect::<Vec<_>>()
            .join(", ");
        // A package with build outputs exports a file of them, as its real one does.
        let exports = if package.outputs.is_empty() {
            ""
        } else {
            ", \"exports\": {\"./a\": \"./dist/a.js\"}"
        };
        let with_check = text.replacen(
            "\"name\": \"x\"",
            &format!(
                "\"name\": \"@poyetik/{name}\", \"devDependencies\": {{{dependencies}}}, \
                 \"scripts\": {{\"check\": \"{check}\"}}{exports}"
            ),
            1,
        );
        let _manifest = fake.files.insert(path, with_check);
    }
    fake
}

/// The fake `package.json` at `path`, under the license of its folder: [`APP_PACKAGE`] under
/// `apps/`, [`PACKAGE`] elsewhere.
fn fake_package(path: String) -> (String, &'static str) {
    let text = if path.starts_with("apps/") {
        APP_PACKAGE
    } else {
        PACKAGE
    };
    (path, text)
}

/// Gives every coverage measurement of `fake` a full report, and the locality fixture the
/// report the gate must refuse.
fn coverage_reports(fake: &mut Fake) {
    for step in check_steps(CHANGES_MODE) {
        if let Action::Coverage(coverage) = step.action {
            let previous = fake.files.insert(coverage.report(), full_report(coverage));
            assert!(previous.is_none(), "fresh fake");
        }
    }
    let locality = Coverage {
        krate: super::COVERAGE_LOCALITY,
        measured: Measured::Library,
    };
    let refused = report(&[(
        &format!("{}/src/lib.rs", super::COVERAGE_LOCALITY.dir),
        (10, 7),
        (3, 2),
        (2, 2),
    )]);
    let previous = fake.files.insert(locality.report(), refused);
    assert!(previous.is_none(), "fresh fake");
}

/// The review, in the fake's register, of the one source of the fake no package measures.
const SCOPE_REVIEWED: &str = "tooling/config/x.ts: not measured -- a source of the fake\n";

/// A coverage summary like [`TS_COVERED`], measuring `src/a.ts` of the package in `dir`,
/// with the absolute path Vitest writes.
fn ts_covered(dir: &str) -> String {
    TS_COVERED.replacen(
        "{\"total\"",
        &format!("{{\"/r/{dir}/src/a.ts\":{{}},\"total\""),
        1,
    )
}

/// Gives every package of `fake` a source, `src/a.ts`, a fully covered, fully killed report
/// measuring it and a Stryker configuration that hides nothing and mutates it. Each exports
/// its own constant, as no two packages export one alike (`shared_constants_gate`).
fn typescript_reports(fake: &mut Fake) {
    for package in PACKAGES {
        let source = fake.files.insert(
            format!("{}/src/a.ts", package.dir),
            format!("export const A = \"{}\";\n", package.dir),
        );
        assert!(source.is_none(), "fresh fake");
        let summary = fake.files.insert(
            format!("{}/coverage/coverage-summary.json", package.dir),
            ts_covered(package.dir),
        );
        let mutations = if package.dir == "tooling/config" {
            TS_KILLED_OR_DISABLED
        } else {
            TS_KILLED
        };
        let mutation = fake.files.insert(
            format!("{}/reports/mutation/mutation.json", package.dir),
            mutations.to_owned(),
        );
        let config = fake.files.insert(
            format!("{}/stryker.config.json", package.dir),
            STRYKER_CONFIG.to_owned(),
        );
        assert!(
            summary.is_none() && mutation.is_none() && config.is_none(),
            "fresh fake"
        );
    }
}

/// What an installed tool of the preflight answers: its pinned version for a pinned Cargo
/// tool, a recent Node.js, any version for the others.
pub(crate) fn tool_answer(program: &str, subcommand: Option<&str>) -> String {
    let pinned = super::PINNED_TOOLS
        .iter()
        .find(|(name, _)| subcommand == name.strip_prefix("cargo-"));
    match (program, pinned) {
        ("node", _) => "v22.23.4\n".to_owned(),
        ("git", _) => "git version 2.39.2 (Apple Git-143)\n".to_owned(),
        ("rustc", _) => "rustc 1.99.1 (58cf8f9c3 2026-10-13)\n".to_owned(),
        (_, Some((name, version))) => format!("{name} {version}\n"),
        _ => "1.0\n".to_owned(),
    }
}

/// A toolchain file pinning the release the fake's `rustc` answers.
pub(crate) const TOOLCHAIN: &str =
    "# Pinned.\n[toolchain]\nchannel = \"1.99.1\"\nprofile = \"minimal\"\n";

/// A history whose two commits follow the convention, the change of behavior with its
/// changelog entry, as `git log` writes it for the history gate.
pub(crate) const HISTORY: &str = "\u{1e}bbbbbbbbbb\u{1f}feat(audio-core): add the plan\n\nWhy.\n\u{1e}aaaaaaaaaa\u{1f}docs: start\n";

/// The files the commits of [`HISTORY`] touched.
pub(crate) const HISTORY_FILES: &str =
    "\u{1e}bbbbbbbbbb\n\nCHANGELOG.md\ncrates/a.rs\n\u{1e}aaaaaaaaaa\n\nREADME.md\n";

/// The commits of [`HISTORY`] whose scope is held to the registered scopes: both.
pub(crate) const HISTORY_SCOPED: &str = "bbbbbbbbbb\naaaaaaaaaa\n";

/// The READMEs of a repository whose parts are all described: each TypeScript package has
/// its own, each folder names its parts, the root names the folders. The crates' README
/// names every crate of [`super::RUST_CRATES`] under `crates/`, and the bindings' every
/// folder of one under `bindings/`, derived so that they follow the registry.
pub(crate) fn readmes() -> Vec<(String, String)> {
    let crates = super::RUST_CRATES
        .iter()
        .filter(|krate| krate.dir.starts_with("crates/"))
        .map(|krate| format!("`{}`", krate.name))
        .collect::<Vec<_>>()
        .join(" ");
    let bindings = super::RUST_CRATES
        .iter()
        .filter_map(|krate| krate.dir.strip_prefix("bindings/"))
        .map(|folder| format!("- [`{folder}/`]({folder}/)"))
        .collect::<Vec<_>>()
        .join("\n");
    // Each folder of packages names its packages; each package has its own README.
    let packages_in = |folder: &str| {
        PACKAGES
            .iter()
            .filter_map(|package| package.dir.strip_prefix(folder))
            .map(|name| format!("| `{name}/` |"))
            .collect::<Vec<_>>()
            .join(" ")
    };
    let mut readmes: Vec<(String, String)> = [
        (
            "README.md",
            format!("| `apps/` | | `bindings/` | | `crates/` | | `tooling/` |\n{README_LICENSE}"),
        ),
        ("crates/README.md", crates),
        ("apps/README.md", packages_in("apps/")),
        ("bindings/README.md", bindings),
        ("tooling/README.md", packages_in("tooling/")),
    ]
    .into_iter()
    .map(|(path, text)| (path.to_owned(), text))
    .collect();
    readmes.extend(PACKAGES.iter().map(|package| {
        let name = package.dir.rsplit('/').next().unwrap_or(package.dir);
        (
            format!("{}/README.md", package.dir),
            format!("# {name}\n\n| `src/a.ts` | Its one module |\n"),
        )
    }));
    readmes
}

/// The crates of the visual library in [`super::RUST_CRATES`]: the expectations below
/// that count steps, commands and reads were written for the audio engine's crates, each
/// visual crate adding its own (one coverage step, its measurement and the emptying of the
/// coverage build before it, its manifest read by three gates, its coverage report read
/// twice).
pub(crate) fn visual_crates() -> usize {
    super::RUST_CRATES
        .iter()
        .filter(|krate| krate.name.starts_with("visual-"))
        .count()
}

/// The total the check prints for `steps` steps of the fakes' clock, 1.5 s each, rounded
/// down to the second: `2 min 16 s` for 91 steps.
pub(crate) fn elapsed(steps: usize) -> String {
    let seconds = steps.saturating_mul(3) / 2;
    format!("{} min {:02} s", seconds / 60, seconds % 60)
}

/// The visual crates with a binary: each adds the coverage step of its `main.rs`, that
/// measurement and the emptying before it, and its report read twice.
pub(crate) fn visual_binaries() -> usize {
    super::RUST_CRATES
        .iter()
        .filter(|krate| krate.name.starts_with("visual-") && krate.binary)
        .count()
}

/// The steps the visual library adds to the check: a coverage step per crate and per
/// binary, and the native visual conformance check.
pub(crate) fn visual_steps() -> usize {
    visual_crates()
        .saturating_add(visual_binaries())
        .saturating_add(1)
}

/// The TypeScript packages beyond the five these expectations were written for: the shared
/// build of the WebAssembly modules (`tooling/wasm-build`) and the visual library in the
/// browser (`bindings/visual-wasm`).
pub(crate) fn added_packages() -> usize {
    PACKAGES.len().saturating_sub(5)
}

/// The steps the packages of [`added_packages`] add to the check: one per script of its
/// checks, then its mutation testing, its reports gate and its scope gate.
pub(crate) fn added_package_steps() -> usize {
    packages_beyond_the_five()
        .map(|package| package.checks.len().saturating_add(3))
        .sum()
}

/// The builds prepared for the packages that use them beyond the engine's, the one the
/// counts of the checks were written with: each a step of the full and default checks.
pub(crate) fn added_builds() -> usize {
    crate::needed_builds().len().saturating_sub(1)
}

/// The packages beyond the five the counts of the default check were written for: the
/// shared policy, the oracle, the engine in the browser, the playground and the roadmap.
pub(crate) fn packages_beyond_the_five() -> impl Iterator<Item = &'static crate::Package> {
    PACKAGES.iter().filter(|package| {
        ![
            "tooling/config",
            "tooling/oracle",
            "bindings/audio-wasm",
            "apps/playground",
            "apps/roadmap",
        ]
        .contains(&package.dir)
    })
}

pub(crate) const CARGO: &str = "[workspace]\nmembers = []\n\n[workspace.package]\nedition = \"2024\"\nversion = \"0.2.0\"\n\n[workspace.dependencies]\n\n[workspace.lints.rust]\nunsafe_code = \"deny\"\n\n[workspace.lints.rustdoc]\n\n[workspace.lints.clippy]\n\n[profile.release]\n\n[other]\nversion = \"9.9.9\"\n";

/// The manifest of the GPU workspace: the tables it shares with [`CARGO`], then its crate.
pub(crate) const GPU_CARGO: &str = "[workspace]\nresolver = \"3\"\nmembers = [\".\"]\n\n[workspace.package]\nedition = \"2024\"\nversion = \"0.2.0\"\n\n[workspace.lints.rust]\nunsafe_code = \"deny\"\n\n[workspace.lints.rustdoc]\n\n[workspace.lints.clippy]\n\n[package]\nversion.workspace = true\nrepository.workspace = true\n\n[dependencies]\n\n[dev-dependencies]\n\n[lints]\nworkspace = true\n";

/// What the gates on the workspaces' builds read in a passing run: the CPU reference's tree
/// at the root (the GPU workspace's is among [`gpu_workspace_files`]), each workspace's
/// resolved metadata and the build-time code register.
fn build_answers() -> [(String, &'static str); 4] {
    [
        (
            format!(
                "capture {}",
                super::cpu_reference_command(super::CargoWorkspace::Root).line()
            ),
            CPU_REFERENCE_TREE,
        ),
        (
            format!(
                "capture {}",
                super::resolved_metadata_command(super::CargoWorkspace::Root).line()
            ),
            RESOLVED_METADATA,
        ),
        (
            format!(
                "capture {}",
                super::resolved_metadata_command(super::CargoWorkspace::Gpu).line()
            ),
            RESOLVED_METADATA,
        ),
        (
            super::BUILD_TIME_REVIEWED.to_owned(),
            "libm@0.2.16 (build script) -- emits cfgs\n",
        ),
    ]
}

/// The resolved metadata of either workspace of a passing run: one third-party crate with a
/// build script, reviewed in the fake's register.
const RESOLVED_METADATA: &str = r#"{"packages":[{"name":"libm","version":"0.2.16","source":"registry+x","targets":[{"kind":["custom-build"]}]}]}"#;

/// The tree of the CPU reference in either workspace of a passing run: the same.
const CPU_REFERENCE_TREE: &str = "visual-core v0.2.0 (/r/crates/visual-core)\n";

/// The manifest of `krate` in a passing run: the GPU workspace's [`GPU_CARGO`], every
/// other [`MEMBER`].
fn member_manifest(krate: &super::RustCrate) -> (String, &'static str) {
    let manifest = format!("{}/Cargo.toml", krate.dir);
    let text = if manifest == super::GPU_MANIFEST {
        GPU_CARGO
    } else {
        MEMBER
    };
    (manifest, text)
}

/// The files of the GPU workspace a passing run reads: its policy, as xtask derives it, its
/// metadata (its one crate, on the visual library's core) and its mutation report (no
/// unviable mutant).
fn gpu_workspace_files() -> [(String, &'static str); 5] {
    [
        (
            super::loaded_files::SHADER_TEST_SOURCE.to_owned(),
            "#![forbid(unsafe_code)]\n#[test]\nfn validates_the_library_s_shaders() {\n    for (name, source) in [\n        (\"crates/visual-boxes/src/square.wgsl\", visual_boxes::SQUARE_WGSL),\n        (\"crates/visual-core/src/display.wgsl\", visual_core::DISPLAY_WGSL),\n    ] {}\n}\n",
        ),
        (
            format!(
                "capture {}",
                super::cpu_reference_command(super::CargoWorkspace::Gpu).line()
            ),
            CPU_REFERENCE_TREE,
        ),
        (super::GPU_DENY_TOML.to_owned(), GPU_DENY.as_str()),
        (
            format!(
                "capture {}",
                super::metadata_command(super::CargoWorkspace::Gpu).line()
            ),
            GPU_METADATA,
        ),
        (
            format!("{}/mutants.out/unviable.txt", super::GPU_MUTANTS_OUT),
            "",
        ),
    ]
}

/// What `cargo metadata` says of the GPU workspace: its one crate.
pub(crate) const GPU_METADATA: &str = r#"{"workspace_root":"/r/crates/visual-gpu","packages":[{"license":"MIT OR Apache-2.0","name":"visual-gpu","manifest_path":"/r/crates/visual-gpu/Cargo.toml","dependencies":[{"name":"visual-core","kind":null,"path":"/r/crates/visual-core"}],"targets":[{"kind":["test"],"src_path":"/r/crates/visual-gpu/tests/shaders.rs","test":true,"doctest":false},{"kind":["test"],"src_path":"/r/crates/visual-gpu/tests/data.rs","test":true,"doctest":false}]}]}"#;

/// The GPU workspace's policy, as xtask derives it.
static GPU_DENY: std::sync::LazyLock<String> = std::sync::LazyLock::new(crate::gpu_deny_config);

/// The manifest of a crate that takes the workspace version.
pub(crate) const MEMBER: &str = "[package]\nversion.workspace = true\nrepository.workspace = true\n\n[lints]\nworkspace = true\n";

pub(crate) const PACKAGE: &str = r#"{"name": "x", "license": "MIT OR Apache-2.0", "version": "0.2.0", "engines": {"node": ">=22.18"}}"#;

/// [`PACKAGE`] under the license of the apps, as every manifest under `apps/` is.
const APP_PACKAGE: &str = r#"{"name": "x", "license": "LicenseRef-PolyForm-Perimeter-1.0.1-Exports", "version": "0.2.0", "engines": {"node": ">=22.18"}}"#;

/// The license files, each starting with the line of its license, the Apache license's
/// centered by spaces as its official text is.
pub(crate) const LICENSE_TEXTS: [(&str, &str); 4] = [
    ("LICENSE-MIT", "MIT License\n\nCopyright\n"),
    (
        "LICENSE-APACHE",
        "\n                              Apache License\n",
    ),
    ("apps/LICENSE", "# PolyForm Perimeter License 1.0.1\n"),
    (
        "CLAUDE.md",
        "`apps/` under `LicenseRef-PolyForm-Perimeter-1.0.1-Exports`, the rest under `MIT OR Apache-2.0`.\n",
    ),
];

/// The License section of the fake root README, which names both licenses and keeps the
/// notices the exports permission does not waive.
pub(crate) const README_LICENSE: &str = "\n## License\n\n`LicenseRef-PolyForm-Perimeter-1.0.1-Exports` with its \"Additional Permission: Exports\": free of its notices and noncompete; the rest under `MIT OR Apache-2.0`, whose parts and third-party libraries keep their own notices.\n";

pub(crate) const LOG: &str = "# Changelog\n\nIntro.\n\n## [Unreleased]\n\n### Added\n\n- x\n\n## [0.2.0] - 2026-11-07\n\n## [0.1.10] - 2026-10-01\n\n## [0.1.2] - 2026-09-30\n";

pub(crate) fn command_lines(steps: &[Step]) -> Vec<String> {
    steps
        .iter()
        .map(|step| match &step.action {
            // A run of mutants with its doc-test pass shows as the command it starts with.
            Action::Run(cmd) | Action::ToolGate(cmd) | Action::RustMutants(cmd) => {
                format!("{:?}: {} {}", cmd.dir, cmd.program, cmd.args.join(" "))
            }
            Action::Coverage(coverage) => format!("coverage {}", coverage.command().line()),
            Action::CoverageLocality => "coverage locality".to_owned(),
            Action::GpuMutants(extent) => {
                format!(
                    "gpu mutants ({extent:?}) {}",
                    super::gpu_mutants_command().line()
                )
            }
            Action::RustMutantsOfChanges { jobs } => {
                format!("rust mutants of the changes ({jobs} jobs)")
            }
            Action::CorpusMutants(mode) => format!("corpus mutants ({mode:?})"),
            Action::RustMutantsOfChangedLines { jobs, since } => {
                format!(
                    "rust mutants of the lines changed since {} ({jobs} jobs)",
                    since.name
                )
            }
            Action::UnviableGate(extent) => format!("unviable gate ({extent:?})"),
            Action::MutationProofRecorded => "mutation proof recorded".to_owned(),
            Action::Bless(options) => format!("bless {}", super::bless_command(options).line()),
            Action::Conformance => format!("conformance {}", super::conformance_command().line()),
            Action::UnviableReviewsCurrent => "unviable reviews current".to_owned(),
            Action::EquivalentGate => "equivalent gate".to_owned(),
            Action::VersionGate => "version gate".to_owned(),
            Action::ReadmeGate => "readme gate".to_owned(),
            Action::VocabularyGate => "vocabulary gate".to_owned(),
            Action::FreshDirs(paths) => format!("fresh {}", paths.join(" ")),
            Action::TypeScriptReportsGate(package) => {
                format!("typescript reports gate of {}", package.dir)
            }
            Action::StrykerConfigGate => "stryker config gate".to_owned(),
            Action::PackageCheckGate => "package check gate".to_owned(),
            Action::TypeScriptScopeGate(package) => {
                format!("typescript scope gate of {}", package.dir)
            }
            Action::TypeScriptMutationGate { package, extent } => {
                format!("typescript mutation gate of {} ({extent:?})", package.dir)
            }
            Action::BypassGate => "bypass gate".to_owned(),
            Action::MembershipGate => "membership gate".to_owned(),
            Action::LicenseGate => "license gate".to_owned(),
            Action::UnsafeGate => "unsafe gate".to_owned(),
            Action::LibraryLintsGate => "library lints gate".to_owned(),
            Action::CargoConfigGate => "cargo config gate".to_owned(),
            Action::TestCodeGate => "test code gate".to_owned(),
            Action::TestDataGate => "test data gate".to_owned(),
            Action::UntestedTargetsGate => "untested targets gate".to_owned(),
            Action::FloatMethodGate => "float method gate".to_owned(),
            Action::DependencyGate => "dependency gate".to_owned(),
            Action::PinGate => "pin gate".to_owned(),
            Action::LibraryGate => "library gate".to_owned(),
            Action::PartLicenseGate => "part license gate".to_owned(),
            Action::CitationGate => "citation gate".to_owned(),
            Action::ConflictMarkerGate => "conflict marker gate".to_owned(),
            Action::SourceKindsGate => "source kinds gate".to_owned(),
            Action::ListCountsGate => "list counts gate".to_owned(),
            Action::RetiredPatternsGate => "retired patterns gate".to_owned(),
            Action::RunnableHintsGate => "runnable hints gate".to_owned(),
            Action::HermeticGitGate => "hermetic git gate".to_owned(),
            Action::RegisterNamesGate => "register names gate".to_owned(),
            Action::FusedBuild => "fused build".to_owned(),
            Action::DocTests => "doc tests".to_owned(),
            Action::LoadedFilesGate => "loaded files gate".to_owned(),
            Action::UnviableNamesGate => "unviable names gate".to_owned(),
            Action::MathLayerGate => "math layer gate".to_owned(),
            Action::SupplyChainGate => "supply chain gate".to_owned(),
            Action::CpuReferenceGate => "CPU reference gate".to_owned(),
            Action::BuildTimeGate => "build-time code gate".to_owned(),
            Action::Preflight(crate::GpuDriver::Required) => "preflight".to_owned(),
            Action::Preflight(crate::GpuDriver::Unneeded) => "preflight, no GPU driver".to_owned(),
            Action::MutantLeftGate => "mutant left gate".to_owned(),
            Action::HistoryGate => "history gate".to_owned(),
            Action::WorkflowGate => "workflow gate".to_owned(),
            Action::PartsGate => "parts gate".to_owned(),
            Action::ShardsUnviableGate(reports) => {
                format!("unviable gate over {}", reports.join(" "))
            }
            Action::TreeRecorded => "tree recorded".to_owned(),
            Action::TreeUnchanged => "tree unchanged".to_owned(),
        })
        .collect()
}

/// The sysroot the fake nightly answers.
pub(crate) const FAKE_SYSROOT: &str = "/toolchain";

/// The version the fake nightly answers: newer than [`RELEASE`].
pub(crate) const FAKE_NIGHTLY: &str = "rustc 1.101.0-nightly (f9ceb4a4d 2026-10-31)\n";

/// The release of the fake toolchain file, [`TOOLCHAIN`].
pub(crate) const RELEASE: super::Release = (1, 99, 1);

/// A source of the standard library declaring `methods` (name, stability, documentation)
/// in an `impl` block of `float`, between a constant, a call of the reviewed macro
/// `float_impl!` and the closing brace; each method's body calls a macro.
pub(crate) fn float_source(float: &str, methods: &[(&str, bool, &str)]) -> String {
    use core::fmt::Write as _;
    let mut text = format!(
        "//! Floats.\n\npub mod consts {{}}\n\nimpl {float} {{\n    pub const RADIX: u32 = 2;\n\n    float_impl! {{\n        Self = {float},\n    }}\n\n"
    );
    for (name, stable, documented) in methods {
        let attribute = if *stable {
            "#[stable(feature = \"rust1\", since = \"1.0.0\")]"
        } else {
            "#[unstable(feature = \"x\", issue = \"1\")]"
        };
        let _written = writeln!(
            text,
            "    /// {documented}\n    ///\n    /// ```\n    /// let x = 1.0;\n    /// ```\n    {attribute}\n    #[inline]\n    pub const fn {name}(self) -> {float} {{\n        debug_assert!(true);\n        self\n    }}\n"
        );
    }
    text.push_str("}\n\nfn outside() {}\n");
    text
}

/// The source of the reviewed macro `float_impl!`: it defines a constant, and names a
/// function only in a comment and in an attribute.
const FLOAT_MACRO_SOURCE: &str = "#[rustc_macro_transparency = \"semiopaque\"]\npub(crate) macro float_impl(\n    Self = $SelfT:ty,\n) {\n    /// Not a fn here.\n    #[doc = concat!(\"fn get_exp(a: \", stringify!($SelfT), \")\")]\n    pub const RADIX: u32 = 2;\n}\n";

/// Consistent float sources, every reviewed method stable in `core`, `sin` banned (and
/// documented as such) and an unstable `gamma` in `std`, then the source of `float_impl!`,
/// with the `clippy.toml` banning `sin` (and a method that is not a float's).
pub(crate) fn float_fixture() -> (Vec<(String, String)>, String) {
    let reviewed: Vec<(&str, bool, &str)> = super::DETERMINISTIC_FLOAT_METHODS
        .iter()
        .map(|(name, _)| (*name, true, "Exact."))
        .collect();
    let std_methods = [
        ("sin", true, "# Unspecified precision"),
        ("gamma", false, "non-deterministic"),
    ];
    let sources = super::FLOAT_SOURCES
        .iter()
        .map(|path| {
            let float = if path.ends_with("f32.rs") {
                "f32"
            } else {
                "f64"
            };
            let methods: &[(&str, bool, &str)] = if path.contains("/core/") {
                &reviewed
            } else {
                &std_methods
            };
            (
                format!("{FAKE_SYSROOT}/{path}"),
                float_source(float, methods),
            )
        })
        .chain(super::FLOAT_ITEM_MACROS.iter().map(|(_, path)| {
            (
                format!("{FAKE_SYSROOT}/{path}"),
                FLOAT_MACRO_SOURCE.to_owned(),
            )
        }))
        .collect();
    let clippy = "disallowed-methods = [\n  { path = \"std::env::args\", reason = \"r\" },\n  { path = \"f32::sin\", reason = \"r\" },\n  { path = \"f64::sin\", reason = \"r\" },\n]\n".to_owned();
    (sources, clippy)
}

/// The steps of the default check from the Rust mutation testing of the changes to its
/// unviable gate.
pub(crate) fn rust_mutation_of_changes() -> Vec<Step> {
    let steps = check_steps(CHANGES_MODE);
    let start = steps
        .iter()
        .position(|step| matches!(step.action, Action::RustMutantsOfChanges { .. }))
        .expect("the default check mutates the changes");
    steps.into_iter().skip(start).take(2).collect()
}

/// The diff git writes for a change of one line of `path`.
pub(crate) fn one_line_change(path: &str) -> String {
    format!("diff --git a/{path} b/{path}\n--- a/{path}\n+++ b/{path}\n@@ -1 +1 @@\n-a\n+b\n")
}

/// An output that always fails.
pub(crate) struct Broken;

impl io::Write for Broken {
    fn write(&mut self, _buf: &[u8]) -> io::Result<usize> {
        Err(io::Error::other("closed"))
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

/// How the fake finds the base of the versions spent, and what it answers: `origin/main`
/// resolves, to another commit than the upstream of [`MERGE_BASE`].
pub(crate) const MAIN_BASE: (&str, &str) = (
    "git merge-base HEAD origin/main",
    "fed4560000000000000000000000000000000000\n",
);

/// The commits the history gate is pinned to, written out: `SCOPES_SINCE`,
/// `CHANGELOG_SINCE`, `BREAKING_SINCE`, then the one commit of `CI_MARKER_EXEMPT`.
pub(crate) const PINS: [(&str, &str); 4] = [
    ("SCOPES_SINCE", "8ff0722ec0273e176d23865891563e8bbe8a1c41"),
    (
        "CHANGELOG_SINCE",
        "a6614dfa18a3b69634f7f581f711d3f509bb5115",
    ),
    ("BREAKING_SINCE", "8ff0722ec0273e176d23865891563e8bbe8a1c41"),
    (
        "CI_MARKER_EXEMPT",
        "431b8a141a99759fb5c2a5adefd4fc551c43276c",
    ),
];

/// How the fake lists the history of [`MAIN_BASE`], and what it answers: a commit of its
/// own, then every pin of [`PINS`].
pub(crate) const PINS_LISTED: (&str, &str) = (
    "git log --format=%H fed4560000000000000000000000000000000000",
    "fed4560000000000000000000000000000000000\n\
     8ff0722ec0273e176d23865891563e8bbe8a1c41\n\
     431b8a141a99759fb5c2a5adefd4fc551c43276c\n\
     a6614dfa18a3b69634f7f581f711d3f509bb5115\n",
);

/// How the fake lists the record of the conformance references at the base of the
/// versions spent, and what it answers: none, nothing spent.
const SPENT_LISTED: (&str, &str) = (
    "git ls-tree --name-only fed4560000000000000000000000000000000000 -- \
     conformance/algorithms.json",
    "",
);

/// How the fake lists the changelog at the merge base with `main`, and what it answers:
/// none, nothing released.
pub(crate) const CHANGELOG_LISTED: (&str, &str) = (
    "git ls-tree --name-only fed4560000000000000000000000000000000000 -- CHANGELOG.md",
    "",
);

/// A Stryker report of `a.ts` holding one mutant.
pub(crate) fn one_mutant(status: &str, mutator: &str, line: u64) -> String {
    format!(
        r#"{{"files":{{"x.ts":{{"mutants":[{{"status":"{status}","mutatorName":"{mutator}","location":{{"start":{{"line":{line}}}}}}}]}}}}}}"#
    )
}

/// The repository's workflow, as the workflow gate reads it.
pub(crate) const QUALITY: &str = include_str!("../../../.github/workflows/quality.yml");

/// Whether the gate whose failures open with `context` reports `problem` in `found`: alone, in
/// its summary, or as a whole line of its details.
pub(crate) fn reports(found: &Result<(), Failure>, context: &str, problem: &str) -> bool {
    match found {
        Ok(()) => false,
        Err(failure) if failure.details.is_empty() => {
            failure.summary == format!("{context}: {problem}")
        }
        Err(failure) => failure.details.iter().any(|line| line == problem),
    }
}

/// Every rewrap of `text`, a line of the file at `path`, as an editor or a formatter wraps
/// it: one per space after its comment marker (`///`, `//!`, `//`, ` * `, `#` outside
/// Markdown) or its indentation, broken there into two lines, the second opening with the
/// same marker. A break that would open a list item, a heading or a table row
/// ([`opens_paragraph`](crate::logical_text::opens_paragraph),
/// [`stands_alone`](crate::logical_text::stands_alone)) is no rewrap and is skipped.
///
/// The law of every gate that reads text whole across lines: each of its refusal fixtures
/// is refused in every rewrap too.
pub(crate) fn rewraps(path: &str, text: &str) -> Vec<String> {
    let indent = text.len().saturating_sub(text.trim_start().len());
    let (lead, body) = text.split_at(indent);
    let markdown = Path::new(path)
        .extension()
        .is_some_and(|extension| extension.eq_ignore_ascii_case("md"));
    let markers: &[&str] = if markdown {
        &[]
    } else {
        &["///", "//!", "//", "* ", "# "]
    };
    let marker = markers
        .iter()
        .find(|marker| body.starts_with(**marker))
        .map_or("", |marker| marker.trim_end());
    let prefix = format!("{lead}{marker}");
    let words = text.get(prefix.len()..).unwrap_or_default();
    words
        .match_indices(' ')
        .filter_map(|(at, _)| {
            let (before, after) = (words.get(..at)?, words.get(at.saturating_add(1)..)?);
            let next = after.trim_start();
            (!before.trim().is_empty()
                && !next.is_empty()
                && !crate::logical_text::opens_paragraph(next)
                && !crate::logical_text::stands_alone(next))
            .then(|| format!("{prefix}{before}\n{prefix} {after}"))
        })
        .collect()
}

/// The problems of `found` without the line each names (`path:N:`, wherever it is, a
/// verdict of one problem naming it after its context), so that a refusal and its rewraps
/// ([`rewraps`]) compare.
pub(crate) fn problems_anywhere(found: &Result<(), Failure>) -> Vec<String> {
    let numbered = |word: &&str| {
        word.strip_suffix(':')
            .and_then(|at| at.rsplit_once(':'))
            .is_some_and(|(_, number)| number.parse::<usize>().is_ok())
    };
    found.as_ref().err().map_or_else(Vec::new, |failure| {
        failure
            .details
            .iter()
            .chain([&failure.summary])
            .map(|line| {
                line.split(' ')
                    .filter(|word| !numbered(word))
                    .collect::<Vec<_>>()
                    .join(" ")
            })
            .collect()
    })
}

/// Whether the fake lists `key`, one of its files, among the files git tracks: never one
/// of its scripted answers (`capture`, `stdout` or `fails` and a command, keys holding a
/// space), a file outside the repository (the toolchain's, an absolute path), nor one git
/// keeps in its own directory (the commit message, a proof record) or ignores (what a run
/// writes under `target/`).
pub(crate) fn tracked_in_fake(key: &str) -> bool {
    !key.contains(' ')
        && !key.starts_with('/')
        && !key.starts_with(".git/")
        && !key.starts_with("target/")
        && key != "COMMIT_EDITMSG"
}

/// A board of two cards: #12 of three steps, #3 of one.
pub(crate) const BOARD: &str =
    r#"{"version":1,"cards":[{"id":12,"tasks":[{},{},{}]},{"id":3,"tasks":[{"title":"x"}]}]}"#;

/// The citation gate on the board [`BOARD`] and one file, `x.md`, holding `text`. A card is
/// written `\u{23}N` in this file, so that the gate, reading it, sees no citation here.
pub(crate) fn cite(text: &str) -> Result<(), Failure> {
    super::citation_gate(&[
        (super::ROADMAP_BOARD.to_owned(), BOARD.to_owned()),
        ("x.md".to_owned(), text.to_owned()),
    ])
}

/// A pnpm workspace configuration as xtask accepts it: its five keys, with comments, blank
/// lines and values.
pub(crate) const PNPM: &str = "packages:\n  - \"apps/*\"\nnodeLinker: isolated\n# A comment.\nminimumReleaseAge: 10080\n\noverrides:\n  \"qs\": \"^6.16.1\"\n\ncatalog:\n  \"vitest\": \"^5.1.0\"\n";

/// The vocabulary of the fake repository, whose prose it all accepts.
pub(crate) const FAKE_WORDS: &str = "forbid node -> box -- the glossary\n";

/// The parts of the full check, by name, in order: the matrix of the CI workflow, then the
/// final part.
pub(crate) const PART_NAMES: [&str; 21] = [
    "rust",
    "coverage",
    "mutation-0",
    "mutation-1",
    "mutation-2",
    "mutation-3",
    "mutation-4",
    "mutation-5",
    "mutation-6",
    "mutation-7",
    "miri",
    "gates",
    "tooling-config",
    "tooling-oracle",
    "tooling-wasm-build",
    "bindings-audio-wasm",
    "bindings-visual-wasm",
    "bindings-web-page",
    "apps-playground",
    "apps-roadmap",
    "final",
];

/// The failure of the parts gate with `problems`.
pub(crate) fn parts_refused(problems: &[&str]) -> Result<(), Failure> {
    found(
        "the parts of the full check do not run each of its steps once",
        &problems
            .iter()
            .map(|problem| (*problem).to_owned())
            .collect::<Vec<_>>(),
    )
}

/// A commit, a tree and a base of the proof record tests: full object names.
pub(crate) const PROVEN: [&str; 3] = [
    "1111111111111111111111111111111111111111",
    "2222222222222222222222222222222222222222",
    "3333333333333333333333333333333333333333",
];

/// The tools and the host the fake answers with.
pub(crate) fn fake_tools() -> super::proof_record::ProofTools {
    super::proof_record::ProofTools {
        mutants: "cargo-mutants 27.1.0".to_owned(),
        nextest: "cargo-nextest 0.9.152".to_owned(),
        toolchain: "1.99.1".to_owned(),
        host: "aarch64-apple-darwin".to_owned(),
        compiler: "58cf8f9c35b355199727627d07ae34a4e2846d67 LLVM 23.1.1".to_owned(),
        linker: "Apple clang version 14.0.3 (clang-1403.0.22.14.1)".to_owned(),
        git: "git version 2.39.2 (Apple Git-143)".to_owned(),
    }
}

/// A record of the fake's tools and host for `base`, proving [`PROVEN`]'s commit.
pub(crate) fn proof_record(base: &str) -> super::ProofRecord {
    let [commit, tree, _] = PROVEN;
    super::ProofRecord {
        commit: commit.to_owned(),
        tree: tree.to_owned(),
        base: base.to_owned(),
        tools: fake_tools(),
    }
}

/// The path of the proof record of the base `base`, below the fake's common directory.
pub(crate) fn record_path(base: &str) -> String {
    format!(".git/poyetik/rust-mutants-proven/{base}")
}

/// The command reading the changes since `since`.
pub(crate) fn diff_since(since: &str) -> String {
    format!(
        "capture git -c diff.suppressBlankEmpty=false diff --no-color --no-ext-diff --no-textconv --text --no-renames --unified=3 --inter-hunk-context=0 --diff-algorithm=myers --indent-heuristic --src-prefix=a/ --dst-prefix=b/ {since} -- *.rs"
    )
}

/// The command listing the untracked files neither the repository nor the user ignores, for
/// the changes since a proof record; the untracked code, test data and settings
/// ([`UNTRACKED_CODE`]) follow it.
pub(crate) const UNTRACKED: &str = "capture git ls-files -z --others --exclude-standard";

/// The fake's answer key to the corpus step's listing of the CPU reference's sources.
pub(crate) const CORPUS_SOURCES_LISTED: &str = "capture git ls-files -z --others --exclude-per-directory=.gitignore --cached -- crates/visual-boxes/src/ \
     crates/visual-core/src/";

/// The sources of the CPU reference's crates as git lists them: every file the corpus
/// step mutates or leaves out, and a unit test file, which it reads as neither.
pub(crate) fn corpus_sources() -> Vec<&'static str> {
    super::CORPUS_MUTATED
        .into_iter()
        .chain(super::CORPUS_NOT_MUTATED.map(|(path, _)| path))
        .chain([
            "crates/visual-core/src/rate_tests.rs",
            "crates/visual-core/src/display.wgsl",
        ])
        .collect()
}

/// The changes since the base of the proof record tests: a line of the engine core and a
/// line of the render hash.
pub(crate) fn since_base() -> String {
    format!(
        "{}{}",
        one_line_change("crates/audio-core/src/plan.rs"),
        one_line_change("crates/audio-hash/src/lib.rs")
    )
}

/// Runs the mutation of the changes on `fake`, and returns what it said.
pub(crate) fn mutate_changes(fake: &mut Fake) -> String {
    let mut out = Vec::new();
    assert_eq!(
        run_steps(&rust_mutation_of_changes(), fake, &mut out),
        Ok(()),
        "mutated"
    );
    String::from_utf8(out).expect("UTF-8")
}

/// The Cargo metadata of a workspace at `/r` of one crate whose targets are these: kind,
/// root (relative to `/r`), whether it is tested, whether its doc tests are.
pub(crate) fn targets_metadata(targets: &[(&str, &str, bool, bool)]) -> String {
    let targets: Vec<String> = targets
        .iter()
        .map(|(kind, root, test, doc_tested)| {
            format!(
                r#"{{"kind":["{kind}"],"src_path":"/r/{root}","test":{test},"doctest":{doc_tested}}}"#
            )
        })
        .collect();
    format!(
        r#"{{"workspace_root":"/r","packages":[{{"name":"x","manifest_path":"/r/crates/x/Cargo.toml","targets":[{}]}}]}}"#,
        targets.join(",")
    )
}

/// The metadata of a root workspace whose native shell builds the audio core and the
/// floating-point mode, and whose visual tool builds the visual core; with it, the GPU
/// workspace's ([`GPU_METADATA`]), whose crate builds the visual core too.
pub(crate) const NATIVE_METADATA: &str = r#"{"workspace_root":"/r","packages":[{"name":"audio-cli","dependencies":[{"name":"audio-core","kind":null,"path":"/r/crates/audio-core"},{"name":"fpenv","kind":"dev","path":"/r/crates/fpenv"}]},{"name":"fpenv","dependencies":[]},{"name":"audio-core","dependencies":[]},{"name":"visual-cli","dependencies":[{"name":"visual-core","kind":null,"path":"/r/crates/visual-core"}]},{"name":"visual-core","dependencies":[]}]}"#;

/// A passing fake whose changes since the merge base with `main` are `paths`, its crates
/// those of [`NATIVE_METADATA`].
pub(crate) fn native_fake(paths: &[&str]) -> Fake {
    let mut fake = passing();
    let diff: String = paths.iter().map(|path| one_line_change(path)).collect();
    for (key, value) in [
        (diff_since(MAIN_BASE.1.trim()), diff),
        (
            format!(
                "capture {}",
                super::metadata_command(super::CargoWorkspace::Root).line()
            ),
            NATIVE_METADATA.to_owned(),
        ),
        // A clean work tree at a commit: the native check checks what is committed.
        (
            "capture git rev-parse HEAD".to_owned(),
            format!("{}\n", "c".repeat(40)),
        ),
    ] {
        let _replaced = fake.files.insert(key, value);
    }
    fake
}

/// An output that accepts `n` complete lines, then fails.
pub(crate) struct Limited(pub(crate) usize);

impl io::Write for Limited {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        if self.0 == 0 {
            return Err(io::Error::other("closed"));
        }
        if buf.contains(&b'\n') {
            self.0 = self.0.saturating_sub(1);
        }
        Ok(buf.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

/// A formatter's sink that refuses its `refused_at`-th write and every one after, counting
/// the writes it is asked for.
struct RefusedAt {
    /// The write it refuses first, counted from 1.
    refused_at: usize,
    /// The writes asked for so far.
    calls: usize,
}

impl core::fmt::Write for RefusedAt {
    fn write_str(&mut self, _: &str) -> core::fmt::Result {
        self.calls = self.calls.saturating_add(1);
        if self.calls >= self.refused_at {
            Err(core::fmt::Error)
        } else {
            Ok(())
        }
    }
}

/// Holds that `value`'s `Display` stops at whichever write its formatter refuses: for each
/// write of a whole formatting, refusing it fails the formatting and no write follows it.
pub(crate) fn stops_at_a_refused_write(value: &dyn core::fmt::Display) {
    use core::fmt::Write as _;
    let mut whole = RefusedAt {
        refused_at: usize::MAX,
        calls: 0,
    };
    assert_eq!(write!(whole, "{value}"), Ok(()), "{value}: written whole");
    assert!(whole.calls > 1, "{value}: written in more than one write");
    for refused_at in 1..=whole.calls {
        let mut sink = RefusedAt {
            refused_at,
            calls: 0,
        };
        assert_eq!(
            write!(sink, "{value}"),
            Err(core::fmt::Error),
            "{value}: write {refused_at} refused"
        );
        assert_eq!(
            sink.calls, refused_at,
            "{value}: nothing written after write {refused_at}"
        );
    }
}

/// What `run`, passing on the runner `fake` makes, does not refuse when one of its calls is
/// broken, each occurrence in turn, or one of the lines it writes is refused, each in turn:
/// one line per such call or line, labelled `label`. It fails naming the call broken, or
/// goes on and says so on a line naming it (an optional step: a proof record, a test
/// order); a merge base tried before the last of its list gives way to the next.
pub(crate) fn unrefused_by(
    label: &str,
    run: &dyn Fn(&mut Fake, &mut dyn io::Write) -> Result<(), Failure>,
    fake: &dyn Fn() -> Fake,
) -> Vec<String> {
    let mut unnamed = Vec::new();
    let mut whole = fake();
    let mut written = Vec::new();
    assert_eq!(run(&mut whole, &mut written), Ok(()), "{label}: passing");
    // Each line it writes, refused in turn: it fails there, as a closed stdout makes it.
    let passing = String::from_utf8_lossy(&written).into_owned();
    let lines = passing.matches('\n').count();
    for accepted in 0..lines {
        let refused = run(&mut fake(), &mut Limited(accepted));
        if refused != failed("cannot write to stdout: closed") {
            unnamed.push(format!("{label} | line {accepted} refused"));
        }
    }
    // Each call as `broken` names it, with how many times the check makes it: each of
    // them is broken in turn, so that a file read by two steps fails in the second too.
    let mut calls: BTreeMap<String, usize> = BTreeMap::new();
    for call in whole.ran {
        let key = call
            .strip_prefix("capture ")
            .map_or_else(|| call.clone(), str::to_owned);
        let made = calls.entry(key).or_default();
        *made = made.saturating_add(1);
    }
    for (call, spared) in calls
        .iter()
        .flat_map(|(call, made)| (0..*made).map(move |spared| (call, spared)))
    {
        let mut broken = fake();
        broken.broken = Some(call.clone());
        broken.spared = spared;
        let mut out = Vec::new();
        // Either the check fails, or it goes on and says so: an optional step (a proof
        // record, a test order) on a line of its own, one the passing run never writes,
        // naming what it could not do. Either way the call is named, by its last word (a
        // path, a command's last argument).
        let said = match run(&mut broken, &mut out) {
            Err(failure) => failure.summary,
            Ok(()) => String::from_utf8_lossy(&out)
                .lines()
                .filter(|line| !passing.lines().any(|known| known == *line))
                .collect::<Vec<_>>()
                .join("\n"),
        };
        let last = ["read ", "capture ", "list ", "write "]
            .iter()
            .find_map(|kind| call.strip_prefix(kind))
            .unwrap_or(call)
            .split_whitespace()
            .last()
            .unwrap_or_default();
        // A merge base before the last of its list is tried for a later one to stand in:
        // a branch with no upstream is the usual case, which the fallback serves.
        let tried = [&crate::MUTATION_BASES[..], &crate::MAIN_BASES[..]]
            .iter()
            .any(|bases| {
                bases.split_last().is_some_and(|(_, before)| {
                    before
                        .iter()
                        .any(|base| call == &format!("git merge-base HEAD {base}"))
                })
            });
        if !said.contains(last) && !tried {
            unnamed.push(format!("{label} | {call} | after {spared}"));
        }
    }
    unnamed
}

/// [`unrefused_by`] of the run of `steps`.
pub(crate) fn unrefused(label: &str, steps: &[Step], fake: &dyn Fn() -> Fake) -> Vec<String> {
    unrefused_by(label, &|runner, out| run_steps(steps, runner, out), fake)
}
