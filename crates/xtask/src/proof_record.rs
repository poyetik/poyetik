//! The mutation proof record: what a clean default check proved, reused only when every input is unchanged.

use alloc::collections::BTreeSet;
use std::io;
use std::io::Write;

use crate::{
    actions::{git_output, read_file, say},
    changes::{UNTRACKED_LISTING, nul_separated, unit_test_file},
    coverage::{TreeState, tree_state},
    preflight::{RUST_TOOLCHAIN_FILE, toolchain_channel},
    runner::Runner,
    steps::{Cmd, Dir, Failure},
    workspace::{UNVIABLE_REVIEWED, code_sources},
};

/// Where the default check keeps its mutation proof records ([`ProofRecord`]).
///
/// Below git's common directory (`git rev-parse --git-common-dir`), which every worktree of
/// a repository shares: one file per merge base, named by it.
pub const PROOF_RECORDS: &str = "poyetik/rust-mutants-proven";

/// Where the Rust mutation step of the default check notes the merge base it measured
/// against, for [`Action::MutationProofRecorded`](crate::steps::Action::MutationProofRecorded).
pub const MEASURED_BASE: &str = "target/xtask/mutants-base";

/// The first line of a mutation proof record, before its format: [`PROOF_RECORD_FORMAT`].
pub const PROOF_RECORD_TAG: &str = "poyetik rust-mutants-proven";

/// The format of the mutation proof records this xtask writes and reads, after
/// [`PROOF_RECORD_TAG`] on their first line.
///
/// A record of another format is ignored, its format said. Raised whenever what a record
/// holds or means changes, so that no record written under other rules is read.
pub const PROOF_RECORD_FORMAT: u32 = 6;

/// What decides how the default check scopes its mutation testing, beside the tools.
///
/// Its code (the sources of xtask, its tests apart), the reviews of unviable mutants, and
/// the Cargo configuration: a change to one of these since a record voids it.
pub const PROOF_RECORD_RULES: [&str; 3] = ["crates/xtask/src/", UNVIABLE_REVIEWED, ".cargo/"];

/// Whether `path` is one of the [`PROOF_RECORD_RULES`]: a file of the list, or below one of
/// its directories, unit test files apart.
pub(crate) fn mutation_rule(path: &str) -> bool {
    !unit_test_file(path)
        && PROOF_RECORD_RULES.iter().any(|rule| {
            rule.strip_suffix('/')
                .map_or(path == *rule, |_| path.starts_with(rule))
        })
}

/// A mutation proof record of the default check.
///
/// The Rust mutation step of a default check passed, with its unviable gate, on the clean
/// commit `commit` (of tree `tree`), measured against the merge base `base`, on the host
/// and with the tools, compiler and linker driver it names: every mutant of the changes
/// since `base`, widened as the default check widens, was killed in that state.
///
/// A later default check of the same base and host skips a crate whose inputs are all
/// byte-identical to `commit`'s: its files, those of every crate of the workspace it
/// depends on (from `cargo metadata`, normal, build and development dependencies, at any
/// depth) and its declared test data ([`RustCrate::test_data`](crate::workspace::RustCrate::test_data)); a crate's mutants are
/// tested by its own tests (cargo-mutants' default), so their outcome depends on nothing
/// else but what every test depends on, the mutation rules and the tools it names, a change
/// to which voids the record. The shell xtask's tests run (`sh`), which has no portable
/// version to name, is not: an update of it that changed a test's outcome would fail that
/// test unmutated too (reviewed). Every other crate is mutated exactly as without a record, so the
/// record never changes what the default check proves. `cargo xtask check --full` and CI
/// never read a record.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProofRecord {
    /// The commit proven.
    pub(crate) commit: String,
    /// Its tree.
    pub(crate) tree: String,
    /// The merge base the run measured against.
    pub(crate) base: String,
    /// The tools and the host the run used, each compared whole with those of a later run.
    pub(crate) tools: ProofTools,
}

/// Whether `text` is a full object name of git: 40 or 64 lowercase hexadecimal digits.
pub(crate) fn object_name(text: &str) -> bool {
    matches!(text.len(), 40 | 64)
        && text
            .chars()
            .all(|digit| digit.is_ascii_digit() || ('a'..='f').contains(&digit))
}

impl ProofRecord {
    /// The record as its file holds it: [`PROOF_RECORD_TAG`] and [`PROOF_RECORD_FORMAT`],
    /// then one `key value` line per field.
    #[must_use]
    pub fn text(&self) -> String {
        let mut text = format!(
            "{PROOF_RECORD_TAG} {PROOF_RECORD_FORMAT}\ncommit {}\ntree {}\nbase {}\n",
            self.commit, self.tree, self.base
        );
        for (key, value) in self.tools.fields() {
            text.push_str(key);
            text.push(' ');
            text.push_str(value);
            text.push('\n');
        }
        text
    }

    /// Reads a record written by [`ProofRecord::text`], and nothing else.
    ///
    /// # Errors
    ///
    /// Returns the format of a record of another format (its tag, then a number written
    /// as Rust writes one), or what is malformed: another first line, a field missing, out
    /// of order, repeated or empty, an object name that is not one, or a line more.
    pub fn parse(text: &str) -> Result<Self, String> {
        let mut lines = text
            .strip_suffix('\n')
            .ok_or_else(|| "it does not end with a line break".to_owned())?
            .split('\n');
        let first = lines.next().unwrap_or_default();
        let format = first
            .strip_prefix(PROOF_RECORD_TAG)
            .and_then(|rest| rest.strip_prefix(' '))
            .and_then(|number| {
                number
                    .parse::<u32>()
                    .ok()
                    .filter(|parsed| parsed.to_string() == number)
            });
        match format {
            Some(PROOF_RECORD_FORMAT) => {}
            Some(other) => {
                return Err(format!(
                    "it was written in format {other}; this xtask reads format {PROOF_RECORD_FORMAT}"
                ));
            }
            None => {
                return Err(format!(
                    "its first line is not `{PROOF_RECORD_TAG} {PROOF_RECORD_FORMAT}`"
                ));
            }
        }
        let mut field = |key: &str| {
            lines
                .next()
                .and_then(|line| line.strip_prefix(key))
                .and_then(|rest| rest.strip_prefix(' '))
                .filter(|value| !value.trim().is_empty())
                .map(str::to_owned)
                .ok_or_else(|| format!("it has no `{key}` line where expected"))
        };
        // The keys in the order of `ProofTools::fields`, which `text` writes.
        let record = Self {
            commit: field("commit")?,
            tree: field("tree")?,
            base: field("base")?,
            tools: ProofTools {
                mutants: field("cargo-mutants")?,
                nextest: field("cargo-nextest")?,
                toolchain: field("toolchain")?,
                host: field("host")?,
                compiler: field("compiler")?,
                linker: field("linker")?,
                git: field("git")?,
            },
        };
        if lines.next().is_some() {
            return Err("it has a line more".to_owned());
        }
        for (name, value) in [
            ("commit", &record.commit),
            ("tree", &record.tree),
            ("base", &record.base),
        ] {
            if !object_name(value) {
                return Err(format!("its {name} is not an object name"));
            }
        }
        Ok(record)
    }
}

/// The path of the proof record of the merge base `base`, below git's common directory.
///
/// # Errors
///
/// Returns why git could not name its common directory.
pub(crate) fn proof_record_path(runner: &mut dyn Runner, base: &str) -> Result<String, Failure> {
    let common = git_output(
        runner,
        &["rev-parse", "--git-common-dir"],
        "find git's common directory",
    )?;
    Ok(format!("{}/{PROOF_RECORDS}/{base}", common.trim_end()))
}

/// What a record holds of the tools and the host: `cargo mutants --version`, the test
/// runner of each mutant's tests (`cargo nextest --version`), the pinned release, the host
/// and the compiler's build of `rustc -vV`, the linker driver, and git, which xtask's tests
/// run.
///
/// One type, written, read and compared field by field through [`ProofTools::fields`]
/// only, its equality included (written by hand, as [`ProofTools::differing`]): a field
/// left out of it is never read, which the compiler refuses (`dead_code`, an error for the
/// workspace; a derived `PartialEq` would read every field and hide it).
#[derive(Debug, Clone, Eq)]
pub(crate) struct ProofTools {
    /// `cargo mutants --version`.
    pub(crate) mutants: String,
    /// The first line of `cargo nextest --version`: it runs each mutant's tests and decides
    /// when they stop ([`MUTANT_TEST_TOOL`](crate::MUTANT_TEST_TOOL)).
    pub(crate) nextest: String,
    /// The pinned release ([`RUST_TOOLCHAIN_FILE`]).
    pub(crate) toolchain: String,
    /// The host `rustc -vV` names: what a run proves depends on the architecture
    /// (`cfg(target_arch)`).
    pub(crate) host: String,
    /// The compiler's build, as `rustc -vV` names it: its commit, then its LLVM version
    /// (`<commit-hash> LLVM <version>`).
    pub(crate) compiler: String,
    /// The linker driver every build of the host links through, as the first line of
    /// `cc --version` names it ([`LINKER_DRIVER`]): an input of every test program that
    /// the pinned toolchain does not pin (a system update replaces it).
    pub(crate) linker: String,
    /// The first line of `git --version`: xtask's tests run git.
    pub(crate) git: String,
}

impl ProofTools {
    /// Each field with its key in a record, in the order a record writes them.
    fn fields(&self) -> [(&'static str, &str); 7] {
        [
            ("cargo-mutants", &self.mutants),
            ("cargo-nextest", &self.nextest),
            ("toolchain", &self.toolchain),
            ("host", &self.host),
            ("compiler", &self.compiler),
            ("linker", &self.linker),
            ("git", &self.git),
        ]
    }

    /// The first field of `self`, as recorded, that differs from `now`'s, by its key, or
    /// `None` when every field is the same.
    fn differing<'tools>(
        &'tools self,
        now: &'tools Self,
    ) -> Option<(&'static str, &'tools str, &'tools str)> {
        self.fields()
            .into_iter()
            .zip(now.fields())
            .find(|((_, recorded), (_, current))| recorded != current)
            .map(|((key, recorded), (_, current))| (key, recorded, current))
    }
}

impl PartialEq for ProofTools {
    /// Equal when no field differs ([`ProofTools::differing`]).
    fn eq(&self, other: &Self) -> bool {
        self.differing(other).is_none()
    }
}

/// The program the Rust toolchain links through on every platform the checks run on
/// (macOS and Linux): `cc`, which runs the system's linker, or the toolchain's own.
pub const LINKER_DRIVER: &str = "cc";

/// The tools and the host a record holds ([`ProofTools`]).
///
/// # Errors
///
/// Returns why any of them cannot be read.
pub(crate) fn proof_tools(runner: &mut dyn Runner) -> Result<ProofTools, Failure> {
    let mutants = version_line(runner, "cargo", &["mutants"], "cargo-mutants")?;
    let nextest = version_line(runner, "cargo", &["nextest"], "cargo-nextest")?;
    let toolchain = read_file(runner, RUST_TOOLCHAIN_FILE)?;
    let release = toolchain_channel(&toolchain)
        .map(str::to_owned)
        .ok_or_else(|| Failure::from(format!("{RUST_TOOLCHAIN_FILE} pins no channel")))?;
    let verbose = runner
        .capture(&Cmd::new("rustc", &["-vV"], Dir::Root))
        .map_err(|failure| Failure::from(format!("cannot read the host of rustc: {failure}")))?;
    // Each key on exactly one line, as rustc writes it: a record is evidence, never a
    // choice among answers.
    let named = |key: &str| {
        let mut values = verbose
            .lines()
            .filter_map(|line| line.strip_prefix(key)?.strip_prefix(": "))
            .map(str::trim);
        match (values.next(), values.next()) {
            (Some(_), Some(_)) => Err(Failure::from(format!("`rustc -vV` names {key} twice"))),
            (Some(value), None) if !value.is_empty() => Ok(value),
            _ => Err(Failure::from(format!("`rustc -vV` names no {key}"))),
        }
    };
    let host = named("host")?;
    let compiler = format!("{} LLVM {}", named("commit-hash")?, named("LLVM version")?);
    let linker = version_line(runner, LINKER_DRIVER, &[], "the linker driver")?;
    let git = version_line(runner, "git", &[], "git")?;
    Ok(ProofTools {
        mutants,
        nextest,
        toolchain: release,
        host: host.to_owned(),
        compiler,
        linker,
        git,
    })
}

/// The first line of `<program> <subcommand…> --version` (`what` names the program in
/// errors).
///
/// # Errors
///
/// Returns why it cannot be read, or that it is empty.
pub(crate) fn version_line(
    runner: &mut dyn Runner,
    program: &'static str,
    subcommand: &[&str],
    what: &str,
) -> Result<String, Failure> {
    let mut args = subcommand.to_vec();
    args.push("--version");
    let cmd = Cmd::new(program, &args, Dir::Root);
    let answer = runner.capture(&cmd).map_err(|failure| {
        Failure::from(format!("cannot read the version of {what}: {failure}"))
    })?;
    answer
        .lines()
        .next()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(str::to_owned)
        .ok_or_else(|| Failure::from(format!("`{}` names no version", cmd.line())))
}

/// The proof record of `base` the default check may start from, if there is a valid one;
/// a record that is not (unreadable, [`ProofRecord::parse`] refuses it, another base, other
/// tools or host, a commit git no longer has or whose tree differs) is ignored, with a
/// one-line notice on `out`. Only a record that does not exist is passed over in silence.
///
/// # Errors
///
/// Returns why stdout cannot be written.
pub(crate) fn usable_proof(
    runner: &mut dyn Runner,
    out: &mut dyn Write,
    base: &str,
) -> Result<Option<ProofRecord>, Failure> {
    let path = match proof_record_path(runner, base) {
        Ok(path) => path,
        Err(failure) => {
            say(
                out,
                &format!("  no mutation proof record: {}", failure.summary),
            )?;
            return Ok(None);
        }
    };
    let text = match runner.read(&path) {
        Ok(text) => Ok(text),
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => Err(format!("cannot read it: {error}")),
    };
    let checked = text
        .and_then(|text| ProofRecord::parse(&text))
        .and_then(|record| {
            let tools = proof_tools(runner).map_err(|failure| failure.summary)?;
            let tree = runner
                .capture(&Cmd::new(
                    "git",
                    &[
                        "rev-parse",
                        "--verify",
                        "--quiet",
                        &format!("{}^{{tree}}", record.commit),
                    ],
                    Dir::Root,
                ))
                .map(|tree| tree.trim().to_owned())
                .map_err(|failure| {
                    format!(
                        "git cannot find the tree of its commit {}: {failure}",
                        record.commit
                    )
                })?;
            if record.base != base {
                Err(format!("it is of the base {}", record.base))
            } else if let Some((key, recorded, current)) = record.tools.differing(&tools) {
                Err(format!(
                    "it was made with the {key} {recorded}, not {current}"
                ))
            } else if tree != record.tree {
                Err(format!(
                    "git has no commit {} of tree {}",
                    record.commit, record.tree
                ))
            } else {
                Ok(record)
            }
        });
    match checked {
        Ok(record) => Ok(Some(record)),
        Err(reason) => {
            say(
                out,
                &format!("  ignored the mutation proof record {path}: {reason}"),
            )?;
            Ok(None)
        }
    }
}

/// What git says of the work tree, for [`unclean`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TreeView {
    /// `git status --porcelain --untracked-files=all --ignore-submodules=none`.
    pub status: String,
    /// The untracked code the repository does not ignore, NUL-separated: the `code_sources`
    /// among `git ls-files -z --others --exclude-per-directory=.gitignore`.
    pub untracked: String,
    /// `git ls-files -v -z`: each tracked file with its tag.
    pub listed: String,
    /// Why a tracked file passes through a filter, if one does ([`filtered`]).
    pub filtered: Option<String>,
    /// Why the bytes of a tracked file are not those git shows, if they are not
    /// ([`unshown`]).
    pub unshown: Option<String>,
}

/// Why the tree may differ from what is committed, or `None` when it is clean.
///
/// The reason names the first change or untracked file of the status, the first untracked
/// code it does not show (`untracked_code`), the first file git hides from the status
/// ([`hidden`]), the first file a filter may hide a change of ([`filtered`]), or the first
/// whose bytes git does not show ([`unshown`]).
#[must_use]
pub fn unclean(view: &TreeView) -> Option<String> {
    view.status
        .lines()
        .find(|line| !line.trim().is_empty())
        .map(|line| format!("`{}` is not committed", line.trim_end()))
        .or_else(|| untracked_code(&view.untracked))
        .or_else(|| hidden(&view.listed))
        .or_else(|| view.filtered.clone())
        .or_else(|| view.unshown.clone())
}

/// The first file of `untracked` (NUL-separated), the untracked code the repository does not
/// ignore, said as a reason.
///
/// Cargo, vitest and Astro find such a file by themselves (`tests/x.rs`), whatever the user's
/// git ignores, so that a check would include a file no commit holds. The reason gives the
/// command that adds it, the name quoted for a POSIX shell.
fn untracked_code(untracked: &str) -> Option<String> {
    nul_separated(untracked).first().map(|path| {
        let quoted = path.replace('\'', r"'\''");
        format!(
            "`{path}` is untracked code git status does not show (an ignore file of yours, or \
             `info/exclude`, names it), which cargo and the other tools would read: add it \
             (see: git add --force -- ':/{quoted}'), or remove it"
        )
    })
}

/// The first tracked file git passes through a filter, said as a reason.
///
/// `tracked` is `git ls-files -z`, and `attributes` git's answer on their `filter`
/// attribute (`git check-attr -z filter -- <each>`: path, attribute and value, each ended
/// by NUL).
///
/// A filter's clean driver rewrites what git reads of the file, so git's status, its diffs
/// and its index can miss a change the work tree holds, which cargo builds and tests: no
/// option of git's turns it off. Only `unspecified` and `unset` mean no filter; an answer
/// that is not one entry per tracked file, in their order, is refused as unread.
///
/// It names the filters the check's git reads now: those of the repository
/// (`.gitattributes`) and of `info/attributes`, even one that changes nothing today. A
/// filter of the user's or of the machine's attributes, which the check's git does not read
/// ([`NEUTRAL_GIT_CONFIG`](crate::NEUTRAL_GIT_CONFIG)), one removed since git applied it, and
/// git's other conversions, are seen by the bytes they hide ([`unshown`]).
#[must_use]
pub fn filtered(tracked: &str, attributes: &str) -> Option<String> {
    let paths = nul_separated(tracked);
    let fields: Vec<&str> = attributes.split('\0').collect();
    let entries = fields
        .split_last()
        .filter(|(last, _)| last.is_empty())
        .map(|(_, entries)| entries)
        .filter(|entries| entries.len() == paths.len().saturating_mul(3))
        .filter(|entries| {
            paths.iter().zip(entries.chunks(3)).all(|(path, entry)| {
                entry.first() == Some(&path.as_str()) && entry.get(1) == Some(&"filter")
            })
        });
    let Some(entries) = entries else {
        return Some(
            "git's answer on the `filter` attribute of the tracked files is not one entry per \
             file, in their order"
                .to_owned(),
        );
    };
    paths
        .iter()
        .zip(entries.chunks(3))
        .find_map(|(path, entry)| {
            let value = entry.get(2).copied().unwrap_or_default();
            (!["unspecified", "unset"].contains(&value)).then(|| {
                format!(
                    "`{path}` passes through git's `{value}` filter, whose clean driver may hide a \
                     change from git (status, diff): the tree checked may not be the one \
                     committed: remove its `filter` attribute (from `.gitattributes`, or \
                     `info/attributes` in git's common directory (see: git rev-parse \
                     --git-common-dir))"
                )
            })
        })
}

/// What git says of the bytes of the tracked files, for [`unshown`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TreeBytes {
    /// `git ls-files -s -z`: the mode, the object, the stage and the path of each entry of
    /// the index.
    pub staged: String,
    /// `git ls-files -z --deleted`: the tracked files absent from the disk.
    pub deleted: String,
    /// `git hash-object --no-filters`: the object of the bytes on disk of each regular file
    /// staged and not deleted, one per line, in the index's order.
    pub raw: String,
    /// `git hash-object`: the same, of the bytes as the check's git converts them.
    pub converted: String,
    /// `git diff --name-only -z`: the files git shows as changed from the index.
    pub changed: String,
}

/// The object and the path of each regular file of the index (`staged`) present on disk
/// (not `deleted`), in the index's order, or `None` when an entry is not one of git's.
///
/// A deleted file is shown by git; a symbolic link (`120000`) and a submodule (`160000`) are
/// no file whose bytes git converts.
fn present(staged: &str, deleted: &str) -> Option<Vec<(String, String)>> {
    let deleted: BTreeSet<String> = nul_separated(deleted).into_iter().collect();
    let mut files = Vec::new();
    for entry in nul_separated(staged) {
        let (fields, path) = entry.split_once('\t')?;
        let [mode, object, _stage] =
            <[&str; 3]>::try_from(fields.split(' ').collect::<Vec<_>>()).ok()?;
        match mode {
            "100644" | "100755" => {
                if !deleted.contains(path) {
                    files.push((object.to_owned(), path.to_owned()));
                }
            }
            "120000" | "160000" => {}
            _ => return None,
        }
    }
    Some(files)
}

/// The first tracked file whose bytes on disk are not those git shows, said as a reason.
///
/// Cargo builds and tests the bytes on disk. Each tracked file present on disk holds either
/// exactly the bytes of the index, or bytes git shows as a change and converts in nothing:
/// what git commits, shows and diffs is then what is tested, whatever attribute, of any file
/// and at any time, made git convert the bytes when it added or checked out the file
/// (`filter`, `ident`, `working-tree-encoding`, `text`), and whatever setting makes git trust
/// the index without reading the file (`core.trustctime`, `core.checkStat`,
/// `core.fsmonitor`). The rule reads no attribute. An answer of git that is not one object
/// per file is refused as unread.
///
/// Not seen: the executable bit under a `core.fileMode` of `false`, which is no byte; a
/// tracked symbolic link or submodule (none is tracked); a file neither tracked nor shown as
/// untracked, which this reads nothing of. The reason gives the commands that find and end
/// the conversion, the name quoted for a POSIX shell.
#[must_use]
pub fn unshown(tree: &TreeBytes) -> Option<String> {
    let changed: BTreeSet<String> = nul_separated(&tree.changed).into_iter().collect();
    let raw: Vec<&str> = tree.raw.lines().collect();
    let converted: Vec<&str> = tree.converted.lines().collect();
    let files = present(&tree.staged, &tree.deleted)
        .filter(|files| [raw.len(), converted.len()] == [files.len(); 2]);
    let Some(files) = files else {
        return Some(
            "git's answer on the bytes of the tracked files is not one object per file, in \
             their order"
                .to_owned(),
        );
    };
    files
        .iter()
        .zip(raw.iter().zip(&converted))
        .find_map(|((object, path), (raw, converted))| {
            let shown = raw == converted && changed.contains(path);
            (object != raw && !shown).then(|| {
                let quoted = path.replace('\'', r"'\''");
                format!(
                    "`{path}`: its bytes are not those git shows (an attribute git applied when \
                     it was added or checked out, `filter`, `ident`, `working-tree-encoding`, \
                     `text`, or an index entry git does not reread): the tree checked may not \
                     be the one committed: remove the attribute (see: git -C \"$(git rev-parse \
                     --show-toplevel)\" check-attr -a -- '{quoted}'), then add it again (see: \
                     git add --renormalize -- ':/{quoted}'), or write its line endings as git \
                     stores them when the repository's own attributes convert them"
                )
            })
        })
}

/// The first file `listed` (`git ls-files -v -z`) shows git hiding, said as a reason.
///
/// Git hides from its status and its diffs a file marked `assume-unchanged` (a lowercase
/// tag) or `skip-worktree` (`S`; both, `s`). The reason gives the commands that clear the
/// marks: one `git update-index` per mark, since git applies only one of
/// `--no-assume-unchanged` and `--no-skip-worktree` given together, the name as git wrote
/// it, unquoted with `-z`, quoted for a POSIX shell.
#[must_use]
pub fn hidden(listed: &str) -> Option<String> {
    listed.split('\0').find_map(|entry| {
        let tag = entry.chars().next()?;
        let path = entry.get(2..).unwrap_or_default();
        let quoted = path.replace('\'', r"'\''");
        let commands: Vec<String> = [
            (tag.is_ascii_lowercase(), "--no-assume-unchanged"),
            (tag.eq_ignore_ascii_case(&'S'), "--no-skip-worktree"),
        ]
        .into_iter()
        .filter(|&(marked, _)| marked)
        .map(|(_, flag)| format!("(see: git update-index {flag} -- '{quoted}')"))
        .collect();
        (!commands.is_empty()).then(|| {
            format!(
                "`{path}` is hidden from git status (assume-unchanged or skip-worktree), so no \
                 diff of git shows its changes: clear it {}",
                commands.join(" then ")
            )
        })
    })
}

/// The status of the tree, its untracked code, its files hidden or filtered, and the bytes
/// git does not show, for [`unclean`].
///
/// # Errors
///
/// Returns why git cannot give them.
pub(crate) fn tree_cleanliness(runner: &mut dyn Runner) -> Result<TreeView, Failure> {
    // Every untracked file and submodule change, whatever the user's configuration hides
    // (`status.showUntrackedFiles`, `diff.ignoreSubmodules`), but the files the user's ignore
    // files name, which the listing below reads where a tool would: the code.
    let status = git_output(
        runner,
        &[
            "status",
            "--porcelain",
            "--untracked-files=all",
            "--ignore-submodules=none",
        ],
        "read the work tree",
    )?;
    let untracked = git_output(
        runner,
        &[&UNTRACKED_LISTING[..], &["--"], &code_sources()].concat(),
        "list the untracked code",
    )?;
    let listed = git_output(runner, &["ls-files", "-v", "-z"], "read the work tree")?;
    Ok(TreeView {
        status,
        untracked,
        listed,
        filtered: filtered_files(runner)?,
        unshown: unshown_files(runner)?,
    })
}

/// Why a tracked file passes through a filter, if one does ([`filtered`]), from git: the
/// tracked files, then the `filter` attribute of each, given as arguments (an argument list
/// too long for the system fails to start, and so fails the check).
///
/// # Errors
///
/// Returns why git cannot give them.
pub(crate) fn filtered_files(runner: &mut dyn Runner) -> Result<Option<String>, Failure> {
    let tracked = git_output(runner, &["ls-files", "-z"], "list the tracked files")?;
    let paths = nul_separated(&tracked);
    if paths.is_empty() {
        return Ok(None);
    }
    let mut args = vec!["check-attr", "-z", "filter", "--"];
    args.extend(paths.iter().map(String::as_str));
    let attributes = git_output(runner, &args, "read the filters of the tracked files")?;
    Ok(filtered(&tracked, &attributes))
}

/// Why the bytes of a tracked file are not those git shows, if they are not ([`unshown`]),
/// from git: the index, the files absent from the disk, the bytes of the others (given as
/// arguments, as [`filtered_files`] gives them) as they are and as git converts them, and
/// the changes git shows. An index without an entry is asked no more.
///
/// # Errors
///
/// Returns why git cannot give them: a file of the index replaced by a directory or by a
/// link to nothing is among the files it cannot read.
pub(crate) fn unshown_files(runner: &mut dyn Runner) -> Result<Option<String>, Failure> {
    let staged = git_output(runner, &["ls-files", "-s", "-z"], "list the index")?;
    if staged.is_empty() {
        return Ok(None);
    }
    let deleted = git_output(
        runner,
        &["ls-files", "-z", "--deleted"],
        "list the tracked files deleted",
    )?;
    let files = present(&staged, &deleted).unwrap_or_default();
    let mut hashed = |filters: &[&str], doing: &str| {
        let mut args = vec!["hash-object"];
        args.extend(filters);
        args.push("--");
        args.extend(files.iter().map(|(_, path)| path.as_str()));
        git_output(runner, &args, doing)
    };
    let raw = hashed(&["--no-filters"], "read the bytes of the tracked files")?;
    let converted = hashed(&[], "read the tracked files as git converts them")?;
    let changed = git_output(
        runner,
        &["diff", "--no-color", "--name-only", "-z", "--no-renames"],
        "list the changes git shows",
    )?;
    Ok(unshown(&TreeBytes {
        staged,
        deleted,
        raw,
        converted,
        changed,
    }))
}

/// What the proof record step found ([`proof_to_record`]).
pub(crate) enum Recordable {
    /// The record of a clean tree, as it was when the checks started (boxed: it is far
    /// larger than a reason).
    Record(Box<ProofRecord>),
    /// Why the tree cannot be recorded.
    Not(String),
}

/// The record of the tree, if it can be recorded: only when the tree is clean
/// ([`unclean`]) and as it was when the checks started (`recorded`: nothing checked out,
/// changed or added while they ran).
///
/// # Errors
///
/// Returns why the tree, the base measured or the tools cannot be read.
pub(crate) fn proof_to_record(
    runner: &mut dyn Runner,
    recorded: Option<&TreeState>,
) -> Result<Recordable, Failure> {
    let now = tree_state(runner)?;
    if recorded != Some(&now) {
        return Ok(Recordable::Not(
            "the tree changed while the checks ran, or was not recorded at their start".to_owned(),
        ));
    }
    let view = tree_cleanliness(runner)?;
    if let Some(reason) = unclean(&view) {
        return Ok(Recordable::Not(format!("the tree is not clean: {reason}")));
    }
    let base = read_file(runner, MEASURED_BASE)?.trim().to_owned();
    let commit = now.head.lines().next().unwrap_or_default().to_owned();
    let tree = git_output(
        runner,
        &["log", "-1", "--format=%T", "HEAD"],
        "read the tree checked out",
    )?
    .trim()
    .to_owned();
    Ok(Recordable::Record(Box::new(ProofRecord {
        commit,
        tree,
        base,
        tools: proof_tools(runner)?,
    })))
}

/// Records the proof of the Rust mutation step of the default check that just passed
/// ([`Action::MutationProofRecorded`](crate::steps::Action::MutationProofRecorded)), for the next default check against the same base
/// ([`ProofRecord`]), when [`proof_to_record`] gives one, so that the commit recorded is
/// exactly the state the mutants were killed in.
///
/// The record is optional: when the tree cannot be recorded, or the environment refuses
/// (the tree, the base measured or the tools cannot be read, git's common directory cannot
/// be found, the record cannot be written), it says why on one line (`not recorded: …`)
/// and succeeds.
///
/// # Errors
///
/// Returns why the reader would refuse the record it would write (a defect of xtask, never
/// of the environment), or why stdout cannot be written.
pub(crate) fn record_mutation_proof(
    runner: &mut dyn Runner,
    out: &mut dyn Write,
    recorded: Option<&TreeState>,
) -> Result<(), Failure> {
    let record = match proof_to_record(runner, recorded) {
        Ok(Recordable::Record(record)) => record,
        Ok(Recordable::Not(reason)) => return say(out, &format!("  not recorded: {reason}")),
        Err(failure) => {
            return say(out, &format!("  not recorded: {}", failure.summary));
        }
    };
    // Written only as the reader reads it: an empty or malformed field fails here.
    let record = ProofRecord::parse(&record.text())
        .map_err(|reason| Failure::from(format!("cannot record the mutation proof: {reason}")))?;
    let written = proof_record_path(runner, &record.base).and_then(|path| {
        runner
            .write(&path, &record.text())
            .map_err(|error| Failure::from(format!("cannot write {path}: {error}")))
    });
    match written {
        Ok(()) => say(
            out,
            &format!(
                "  recorded: the mutants of {} are proven against {}",
                record.commit, record.base
            ),
        ),
        Err(failure) => say(out, &format!("  not recorded: {}", failure.summary)),
    }
}

#[cfg(test)]
#[path = "proof_record_tests.rs"]
mod tests;
