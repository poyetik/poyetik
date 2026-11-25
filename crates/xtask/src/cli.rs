//! The command line of xtask: its commands, usage and targets.

use core::num::NonZeroUsize;
use std::ffi::OsString;
use std::io;
use std::io::Write;
use std::path::Path;

use crate::{
    actions::{committed, git_output, pre_push, read_file, run_steps, run_steps_then, say},
    changes::{native_parts, rust_test_code, spent_base},
    commits::{ci_outputs, ci_parts, commit_msg_gate},
    corpus::corpus_steps,
    parts::{Part, full_parts, part_steps},
    reviews::{STRYKER_CONFIG, is_test},
    runner::{CommandFailure, Exit, Failed, Runner},
    steps::{
        Action, Cmd, Dir, Failure, Step, VISUAL_CONFORMANCE, cargo_step, check_steps, gates_steps,
        native_steps, opening_gates, run, safe_message, stryker_command, tested_mutants_command,
    },
    workspace::{
        CargoWorkspace, Extent, MUTANTS_OUT, Mode, OnFailure, PACKAGES, Package, RUST_CRATES,
        RustCrate, below, mutation_jobs, pathspec_matches, stryker_sources,
    },
};

/// Usage of the `xtask` command, in the order of its first line.
pub const USAGE: &str = "\
usage: cargo xtask <check [--full [--part <part>]|--gates|--native]|list [--full [--parts]]|mutants <target>|bless [<option>...]|bless-benchmarks|commit-msg <file>|pre-push [pair...]|ci-parts <event> <message> <parts>|help>
  check [--full]        run every quality check, stopping at the first failure;
                        mutation testing covers the changes. Rust: the changes since
                        the merge base with the upstream, origin/main or main; the
                        crates an earlier default check of that base proved, unchanged
                        since, are skipped (its mutation proof record, which --full
                        never reads). TypeScript: since the last local Stryker run.
                        With --full, every mutant, as CI does, and every check runs
                        whatever fails, then every failure is reported. With --full
                        --part <part>, only that part, as one CI job runs it (the
                        parts together run every check once; the final part reads
                        the unviable mutants of every mutation shard)
  check --gates         run the preflight and the static gates of either mode only,
                        every one whatever fails, their tool runs included (rustdoc,
                        cargo-deny, the frozen JavaScript install, pnpm audit,
                        cargo-mutants' list; the loaded-files gate builds the
                        workspace; the network needed): part of every targeted check
  check --native        run here what CI's x86_64 Linux runner cannot, on committed
                        work only: each part a change since the merge base with
                        origin/main, else main, reaches, every part when what
                        decides xtask's checks changes (its sources, the unviable
                        reviews, the Cargo configuration, its build) (every test of
                        the root workspace on this machine's architecture; the audio
                        conformance; the visual conformance; the GPU workspace's
                        tests, time and mutants), stopping at the first failure; on
                        an idle machine, for the GPU's time
  list [--full]         list the checks of `check` (or `check --full`), in order; with
                        --full --parts, the parts of `check --full`
  mutants <target>      run every mutant of one target: a crate (its name), a Rust file
                        of a crate, a TypeScript package (its directory) or one of its
                        sources (their paths from the root); --changed <revision>:
                        the Rust lines changed since the merge base of HEAD with
                        <revision> (a commit, branch or tag) only, not widened;
                        conformance/visual: the CPU reference's code that writes image
                        bytes, with the visual corpus's check as the only test (a step of
                        the full check, and of the default one when a change reaches its
                        inputs)
  bless [<option>...]   rewrite conformance/references.json from the current renders,
                        conformance/algorithms.json from the current box types and
                        engine, and conformance/live-references.json from the live
                        vector's renders (decision 29): runs `audio-cli bless conformance
                        --spent <record>`, the spent record (at the merge base with
                        origin/main, else main), then the options as given, every one
                        but --spent, which audio-cli's help lists
                        (see: cargo run -p audio-cli -- help)
  bless-benchmarks      rewrite spec/benchmark-baseline.json from the current instruction
                        counts of the engine module (spec/benchmarks.md), the visual
                        module's, and this GPU's time in spec/gpu-time-baseline.txt: run
                        it only in the condition the GPU benchmark's help states
                        (see: cargo run --manifest-path \"$(git rev-parse --show-toplevel)/crates/visual-gpu/Cargo.toml\" -- help)
  commit-msg <file>     check a commit message (used by the commit-msg hook)
  pre-push [pair...]    check that the tree is clean and the pushed commits are checked
                        out, then run `check`, or `check --full` if a tag is pushed;
                        each pair is a pushed commit and the remote ref it updates
                        (used by the pre-push hook)
  ci-parts <arg>...     given <event> <message> <parts>, print the parts a run of the
                        workflow checks, as the outputs of its first job: every part
                        for a pull_request, the <parts> given by hand for a
                        workflow_dispatch (empty: every part), those the CI marker
                        of <message> asks for on a push (used by the workflow)
  help                  show this help";

/// What `cargo xtask check --native` says when no change reaches any of its parts: that
/// nothing ran here, never the count of checks its opening gates passed.
pub(crate) const NOTHING_NATIVE: &str =
    "✔ no change since the merge base reaches a native part: nothing to check on this machine";

/// What `cargo xtask mutants` mutates: every mutant of a target, or the lines changed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Target {
    /// A crate of [`RUST_CRATES`], named (`cargo mutants --package`).
    Crate(RustCrate),
    /// A Rust file of a crate (`cargo mutants --package <crate> --file <path>`).
    RustFile {
        /// The crate holding it.
        krate: RustCrate,
        /// Its path, relative to the root.
        path: String,
    },
    /// A package of [`PACKAGES`], by its directory (Stryker in that package).
    Package(Package),
    /// A TypeScript source of a package, its tests apart (Stryker with `--mutate`).
    TypeScriptFile {
        /// The package holding it.
        package: Package,
        /// Its path, relative to the package.
        source: String,
    },
    /// The Rust lines changed since the merge base of `HEAD` with a revision, not widened
    /// (`--changed <revision>`).
    ChangedLines(Revision),
    /// The CPU reference's code that writes image bytes, with the visual corpus's check as
    /// the only test
    /// ([`VISUAL_CONFORMANCE`], [`Action::CorpusMutants`]).
    Corpus,
}

/// The revision `cargo xtask mutants --changed` measures the changes from: as named, and the
/// commit git resolves it to (`git rev-parse --verify`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Revision {
    /// As named on the command line: any revision git resolves (a commit, a branch, a tag).
    pub name: String,
    /// The full hash of the commit it names.
    pub commit: String,
}

impl Target {
    /// The file the target names, relative to the root, if it is a file.
    #[must_use]
    pub fn file(&self) -> Option<String> {
        match self {
            Self::Crate(_) | Self::Package(_) | Self::ChangedLines(_) | Self::Corpus => None,
            Self::RustFile { path, .. } => Some(path.clone()),
            Self::TypeScriptFile { package, source } => Some(format!("{}/{source}", package.dir)),
        }
    }
}

/// The option of `cargo xtask mutants` that names the Rust lines changed since the merge
/// base of `HEAD` with the revision following it ([`Target::ChangedLines`]).
pub(crate) const CHANGED_LINES: &str = "--changed";

/// A usage error of xtask's command line: exit code 2, pointing to the usage.
fn usage_error(summary: String) -> Failure {
    Failure {
        usage: true,
        ..Failure::from(summary)
    }
}

/// The commit git resolves `name` to (`git rev-parse --verify`, past `--end-of-options`, so
/// that no name is read as an option: git 2.30 or later; the preflight's refusal of an
/// older git is #24 step 153), with `name`.
///
/// Git is first asked where the repository is (`git rev-parse --git-dir`): once it answers,
/// git runs on a repository, so its refusal of the name is the user's, whatever its exit
/// code (1 for a name it does not know; 1 or 128 for `nope@{upstream}` or `HEAD@{99999}`,
/// by git's version), and
/// its reason is kept, or said absent when it wrote none (`--quiet` keeps most quiet).
///
/// # Errors
///
/// Returns the usage error naming it when git, on a repository, resolves no commit from it;
/// a failure of the work, with git's own failure, when git did not start, runs on no
/// repository, or wrote what is not text.
pub(crate) fn revision(runner: &mut dyn Runner, name: &str) -> Result<Revision, Failure> {
    let unresolved =
        |failure: CommandFailure| Failure::from(format!("cannot resolve {name:?}: {failure}"));
    let _repository = runner
        .capture(&Cmd::new("git", &["rev-parse", "--git-dir"], Dir::Root))
        .map_err(unresolved)?;
    let commit = format!("{name}^{{commit}}");
    runner
        .capture(&Cmd::new(
            "git",
            &[
                "rev-parse",
                "--verify",
                "--quiet",
                "--end-of-options",
                &commit,
            ],
            Dir::Root,
        ))
        .map(|commit| Revision {
            name: name.to_owned(),
            commit: commit.trim().to_owned(),
        })
        .map_err(|failure| match failure.failed {
            Failed::Ended { reason: None, .. } => usage_error(format!(
                "{name:?} names no commit git resolves: {failure} (no reason written by git)"
            )),
            Failed::Ended { .. } => {
                usage_error(format!("{name:?} names no commit git resolves: {failure}"))
            }
            Failed::NotStarted(_) | Failed::NotText => unresolved(failure),
        })
}

/// The branch's upstream, as git names it in a revision.
const UPSTREAM: &str = "@{upstream}";

/// The usage error of `cargo xtask mutants --changed` without a revision: it proposes the
/// branch's upstream when git names one (the pushed version of the branch, or the local
/// branch it tracks), else says why git names none, in git's own words (a detached `HEAD`,
/// a branch that tracks nothing, git missing).
pub(crate) fn no_revision(runner: &mut dyn Runner) -> String {
    let needed = format!(
        "`{CHANGED_LINES}` takes the revision the changes are measured from, any revision \
         git resolves (a commit, a branch, a tag)"
    );
    runner
        .capture(&Cmd::new(
            "git",
            &[
                "rev-parse",
                "--abbrev-ref",
                "--symbolic-full-name",
                UPSTREAM,
            ],
            Dir::Root,
        ))
        .map_or_else(
            |failure| {
                format!("{needed}; git names no upstream ({failure}): name where the work starts")
            },
            |upstream| {
                format!(
                    "{needed}; the branch's upstream (see: cargo xtask mutants {CHANGED_LINES} {})",
                    upstream.trim()
                )
            },
        )
}

/// Whether `path` is a source Stryker can mutate ([`stryker_sources`]).
pub(crate) fn mutable_source(path: &str) -> bool {
    stryker_sources()
        .into_iter()
        .any(|pattern| pathspec_matches(pattern, path))
}

/// Reads the target of `cargo xtask mutants`.
///
/// A crate name, a package directory, the path of a file below one, relative to the root
/// and written plainly (no `.`, `..` or empty part); `--changed <revision>`
/// ([`Target::ChangedLines`]) is read by the command line, which resolves the revision.
///
/// # Errors
///
/// Returns the usage error, in one line: a path that is not plain, test code (Rust or
/// TypeScript, fixtures included), or anything else that is no target.
pub fn mutation_target(text: &str) -> Result<Target, String> {
    if text == VISUAL_CONFORMANCE {
        return Ok(Target::Corpus);
    }
    if let Some(krate) = RUST_CRATES.iter().find(|krate| krate.name == text) {
        return Ok(Target::Crate(*krate));
    }
    if let Some(package) = PACKAGES.iter().find(|package| package.dir == text) {
        return Ok(Target::Package(*package));
    }
    if text
        .split('/')
        .any(|part| part.is_empty() || part == "." || part == "..")
    {
        return Err(format!(
            "{text:?} is not a plain path from the root (no `.`, `..` or empty part)"
        ));
    }
    if Path::new(text)
        .extension()
        .is_some_and(|extension| extension == "rs")
        && let Some((krate, rel)) = RUST_CRATES
            .iter()
            .find_map(|krate| below(krate.dir, text).map(|rel| (krate, rel)))
    {
        if rust_test_code(rel) {
            return Err(format!("{text:?} is test code: name the source it tests"));
        }
        return Ok(Target::RustFile {
            krate: *krate,
            path: text.to_owned(),
        });
    }
    if let Some((package, source)) = PACKAGES
        .iter()
        .find_map(|package| below(package.dir, text).map(|source| (*package, source)))
        && mutable_source(source)
    {
        if is_test(source) {
            return Err(format!("{text:?} is test code: name the source it tests"));
        }
        return Ok(Target::TypeScriptFile {
            package,
            source: source.to_owned(),
        });
    }
    Err(format!(
        "{text:?} is no crate, Rust file of a crate, TypeScript package or source of one, nor {VISUAL_CONFORMANCE} or `--changed <revision>`"
    ))
}

/// The steps of `cargo xtask mutants`: the preflight ("Tools and platform"), every mutant
/// of `target`, then its gate.
///
/// The preflight comes first, as in every command that starts the tools of the checks, so
/// that a missing or wrong tool (`prlimit` on Linux, which every cargo-mutants build runs
/// its tests under, or cargo-mutants at another version) is named with how to install it
/// before anything runs. Rust runs in `jobs` jobs. The report directory is emptied first,
/// and the gate judges a partial run unless the target is a whole TypeScript package.
#[must_use]
pub fn mutants_steps(target: &Target, jobs: NonZeroUsize) -> Vec<Step> {
    let rust = |what: &str, args: &[&str]| {
        vec![
            Step {
                name: "Fresh Rust mutation report".to_owned(),
                action: Action::FreshDirs(vec![MUTANTS_OUT.to_owned()]),
            },
            Step {
                name: format!("Rust mutation testing of {what}"),
                action: Action::RustMutants(tested_mutants_command(args, jobs)),
            },
            Step {
                name: "Rust unviable mutants reviewed".to_owned(),
                action: Action::UnviableGate(Extent::Partial),
            },
        ]
    };
    let typescript = |package: Package, what: &str, cmd: Cmd, extent: Extent| {
        let [_, mutation] = package.reports();
        vec![
            Step {
                name: format!("Fresh mutation report of {}", package.dir),
                action: Action::FreshDirs(vec![mutation]),
            },
            Step {
                name: format!("Mutation testing of {what}"),
                action: Action::Run(cmd),
            },
            Step {
                name: format!("TypeScript mutants of {what} killed"),
                action: Action::TypeScriptMutationGate { package, extent },
            },
        ]
    };
    // A crate of the GPU workspace (a file of it too) is outside the root's, where
    // cargo-mutants would find none of its mutants and pass: every mutant of that workspace
    // runs instead, the one way it runs (in place, [`crate::gpu_mutants_command`]).
    let gpu = |what: &str| {
        vec![
            Step {
                name: "Fresh Rust mutation report".to_owned(),
                action: Action::FreshDirs(vec![MUTANTS_OUT.to_owned()]),
            },
            Step {
                name: format!(
                    "Rust mutation testing of {what}: every mutant{}",
                    CargoWorkspace::Gpu.label()
                ),
                action: Action::GpuMutants(Extent::Partial),
            },
            Step {
                name: "Rust unviable mutants reviewed".to_owned(),
                action: Action::UnviableGate(Extent::Partial),
            },
        ]
    };
    let mutated = match target {
        Target::Crate(krate) if krate.workspace == CargoWorkspace::Gpu => gpu(krate.name),
        Target::RustFile { krate, path } if krate.workspace == CargoWorkspace::Gpu => gpu(path),
        Target::Crate(krate) => rust(krate.name, &["--package", krate.name]),
        Target::ChangedLines(since) => vec![
            Step {
                name: "Fresh Rust mutation report".to_owned(),
                action: Action::FreshDirs(vec![MUTANTS_OUT.to_owned()]),
            },
            Step {
                name: "Rust mutation testing of the lines changed".to_owned(),
                action: Action::RustMutantsOfChangedLines {
                    jobs,
                    since: since.clone(),
                },
            },
            Step {
                name: "Rust unviable mutants reviewed".to_owned(),
                action: Action::UnviableGate(Extent::Partial),
            },
        ],
        Target::RustFile { krate, path } => rust(path, &["--package", krate.name, "--file", path]),
        Target::Corpus => corpus_steps(Mode::Full { jobs }).to_vec(),
        Target::Package(package) => typescript(
            *package,
            package.dir,
            stryker_command(
                package.dir,
                Mode::Full {
                    jobs: NonZeroUsize::MIN,
                },
            ),
            Extent::Complete,
        ),
        Target::TypeScriptFile { package, source } => typescript(
            *package,
            &format!("{}/{source}", package.dir),
            Cmd::new(
                "pnpm",
                &["exec", "stryker", "run", STRYKER_CONFIG, "--mutate", source],
                Dir::Package(package.dir),
            ),
            Extent::Partial,
        ),
    };
    [opening_gates().to_vec(), mutated].concat()
}

/// The record of the conformance references, `conformance/algorithms.json`.
pub const CONFORMANCE_RECORD: &str = "conformance/algorithms.json";

/// Where [`bless_step`] and the conformance check write the record at the merge base of
/// [`MAIN_BASES`](crate::workspace::MAIN_BASES), for `audio-cli bless --spent` and `audio-cli check --spent`:
/// `null` when the merge base has none.
pub const SPENT_RECORD: &str = "target/xtask/spent-algorithms.json";

/// The one option of `audio-cli bless` that `cargo xtask bless` gives itself, and refuses
/// from its user: the record at the merge base of [`MAIN_BASES`](crate::workspace::MAIN_BASES).
pub const SPENT_OPTION: &str = "--spent";

/// The exit code of a usage error, in every command-line tool of the repository.
pub(crate) const USAGE_EXIT: i32 = 2;

/// The only way to change the reference hashes: renders every conformance case.
///
/// It rewrites `conformance/references.json`, and what they were blessed under,
/// [`CONFORMANCE_RECORD`], and the live vector's hashes beside them; `audio-cli bless`
/// refuses a reference changed under the same versions but for the cases of
/// `--case-changed`, a changed hash of the live vector but with `--live-changed`, a lower
/// version, a box type's outputs changed at a version spent (one the record at the merge base of [`MAIN_BASES`](crate::workspace::MAIN_BASES)
/// holds: successive changes on a branch share one new version until it reaches `main`),
/// a battery change but with `--battery-changed`, and a directory without both files but
/// with `--from-nothing` (`spec/engine-design.md`, decision 29). The commit must explain
/// why they changed.
///
/// `options` are passed on to `audio-cli bless` unread, after its `--spent`: its parser
/// is their one source, so an option it refuses is refused after the preflight and its
/// build, as a usage error.
#[must_use]
pub fn bless_step(options: &[String]) -> Step {
    Step {
        name: "Bless conformance references".to_owned(),
        action: Action::Bless(options.to_vec()),
    }
}

/// The command [`bless_step`] runs once the record at the merge base is in
/// [`SPENT_RECORD`]: `audio-cli bless`, `options` after its own.
#[must_use]
pub fn bless_command(options: &[String]) -> Cmd {
    let mut args = vec![
        "run",
        "--package",
        "audio-cli",
        "--locked",
        "--quiet",
        "--",
        "bless",
        "conformance",
        SPENT_OPTION,
        SPENT_RECORD,
    ];
    args.extend(options.iter().map(String::as_str));
    Cmd::new("cargo", &args, Dir::Root)
}

/// The file at `path` in `base`, as git gives it, or `None` when `base` holds none.
///
/// # Errors
///
/// Returns why git cannot list or show it.
pub fn file_at(runner: &mut dyn Runner, base: &str, path: &str) -> Result<Option<String>, Failure> {
    let doing = format!("read {path} at the merge base {base}");
    let listed = git_output(
        runner,
        &["ls-tree", "--name-only", base, "--", path],
        &doing,
    )?;
    if listed.trim().is_empty() {
        return Ok(None);
    }
    git_output(runner, &["show", &format!("{base}:{path}")], &doing).map(Some)
}

/// Writes the record at the merge base of [`MAIN_BASES`](crate::workspace::MAIN_BASES) to [`SPENT_RECORD`] (`null` when
/// the merge base has none), and says where it was read.
///
/// # Errors
///
/// Returns why there is no merge base, why git cannot give the record there, or why it
/// cannot be written.
pub(crate) fn write_spent(runner: &mut dyn Runner, out: &mut dyn Write) -> Result<(), Failure> {
    let (base, against) = spent_base(runner)?;
    let record = file_at(runner, &base, CONFORMANCE_RECORD)?;
    let (spent, held) = record.map_or_else(
        || ("null\n".to_owned(), "none"),
        |text| (text, "its versions"),
    );
    runner
        .write(SPENT_RECORD, &spent)
        .map_err(|error| Failure::from(format!("cannot write {SPENT_RECORD}: {error}")))?;
    say(
        out,
        &format!("  spent: {held} at {base} (merge base with {against})"),
    )
}

/// The command of the native conformance check, once the record at the merge base of
/// [`MAIN_BASES`](crate::workspace::MAIN_BASES) is in [`SPENT_RECORD`].
///
/// It checks every case against its reference, and every fingerprint against that record's
/// at the versions both hold, so that a record written other than by bless (a conflict
/// resolved by hand) cannot give a version spent on `main` other bits.
#[must_use]
pub fn conformance_command() -> Cmd {
    Cmd::new(
        "cargo",
        &[
            "run",
            "--package",
            "audio-cli",
            "--locked",
            "--quiet",
            "--",
            "check",
            "conformance",
            SPENT_OPTION,
            SPENT_RECORD,
        ],
        Dir::Root,
    )
}

/// Writes the record at the merge base ([`write_spent`]), then checks the conformance
/// references with it.
///
/// # Errors
///
/// Returns why the record could not be written, or why the check failed.
pub(crate) fn conformance(runner: &mut dyn Runner, out: &mut dyn Write) -> Result<(), Failure> {
    write_spent(runner, out)?;
    runner
        .run(&conformance_command())
        .map_err(|failure| Failure::from(failure.to_string()))
}

/// What `audio-cli` ends a usage error with: where its own usage is, which a user of
/// `cargo xtask bless` reads in `cargo xtask help` instead.
pub(crate) const ENGINE_CLI_USAGE_POINTER: &str = " (see: cargo run -p audio-cli -- help)";

/// The reason of `line`, the error line of `audio-cli`'s usage error, without its
/// `error: ` and its pointer to its usage ([`ENGINE_CLI_USAGE_POINTER`]), so that the line
/// of `cargo xtask bless` says the reason once, with its own pointer.
pub(crate) fn child_usage_reason(line: &str) -> &str {
    let reason = line.strip_prefix("error: ").unwrap_or(line);
    reason
        .strip_suffix(ENGINE_CLI_USAGE_POINTER)
        .unwrap_or(reason)
}

/// Writes the record at the merge base ([`write_spent`]), then blesses with it.
///
/// # Errors
///
/// Returns why the record could not be written, or why the bless failed: a usage error
/// ([`Failure::usage`]) when `audio-cli` refused the options, exiting with code 2
/// (cargo's own failures exit with 101, and the other arguments are xtask's), with the
/// reason `audio-cli` gave ([`child_usage_reason`]).
pub(crate) fn bless(
    runner: &mut dyn Runner,
    out: &mut dyn Write,
    options: &[String],
) -> Result<(), Failure> {
    write_spent(runner, out)?;
    // Its result lines (CHANGED, BITS, what it wrote, or what it refuses) are xtask's
    // result too: on xtask's stdout, so that `cargo xtask bless > log` keeps them; then what
    // it wrote on stderr (its summary, cargo's errors), in the order it wrote them, so that a
    // summary never comes before the lines it counts, and xtask's own `error:` line, with the
    // child's reason, is the last.
    let ran = runner.run_for_stdout(&bless_command(options));
    for line in ran.stdout.lines().chain(ran.stderr.lines()) {
        say(out, &format!("  {line}"))?;
    }
    ran.ended.map_err(|failure| match failure.failed {
        Failed::Ended {
            exit: Exit::Code(USAGE_EXIT),
            reason,
        } => Failure {
            usage: true,
            ..Failure::from(format!(
                "`audio-cli bless` refused the options given to cargo xtask bless{}",
                reason.as_deref().map_or_else(String::new, |reason| format!(
                    ": {}",
                    child_usage_reason(reason)
                ))
            ))
        },
        Failed::NotStarted(_) | Failed::Ended { .. } | Failed::NotText => {
            Failure::from(failure.to_string())
        }
    })
}

/// The only way to change the benchmark baseline: no package script runs the bless.
///
/// Builds the engine module, then counts the instructions of every benchmark and rewrites
/// `spec/benchmark-baseline.json`, refusing to when building plans grows faster than
/// n log n; then the same for the visual module and `spec/visual-benchmark-baseline.json`,
/// refusing when an image's cost is not proportional to its pixels; then this machine's
/// GPU time, with its system and driver, in `spec/gpu-time-baseline.txt`. The commit must
/// explain why the counts changed (`spec/benchmarks.md`).
#[must_use]
pub fn bless_benchmarks_steps() -> [Step; 5] {
    [
        run(
            "Build the engine module",
            "pnpm",
            &["run", "build:wasm"],
            Dir::Package("bindings/audio-wasm"),
        ),
        run(
            "Bless the benchmark baseline",
            "node",
            &["scripts/benchmarks.ts", "bless"],
            Dir::Package("bindings/audio-wasm"),
        ),
        run(
            "Build the visual module",
            "pnpm",
            &["run", "build:wasm"],
            Dir::Package("bindings/visual-wasm"),
        ),
        run(
            "Bless the visual benchmark baseline",
            "node",
            &["scripts/benchmarks.ts", "bless"],
            Dir::Package("bindings/visual-wasm"),
        ),
        cargo_step(
            "Bless this GPU's time",
            CargoWorkspace::Gpu,
            "run",
            &["--locked", "--quiet", "--", "bless"],
        ),
    ]
}

/// `cargo xtask bless-benchmarks`: the preflight, then [`bless_benchmarks_steps`].
fn bless_benchmarks(runner: &mut dyn Runner, out: &mut dyn Write) -> Result<(), Failure> {
    run_steps(
        &[opening_gates().as_slice(), &bless_benchmarks_steps()].concat(),
        runner,
        out,
    )
}

/// A command of `cargo xtask`, read from its arguments.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Task {
    /// Every check: every mutant if `true` ([`Mode::Full`]), else the changes.
    Check(bool),
    /// The static gates of either mode ([`gates_steps`]).
    CheckGates,
    /// What CI's runner cannot prove, on this machine, on committed work only: the opening
    /// gates, then the refusal of changes not committed the pre-push hook's check makes
    /// ([`committed`]), then the parts of [`native_steps`] the changes since `main` reach
    /// ([`native_parts`]), read once the preflight passed, since reading them runs cargo;
    /// none reached, it says so ([`NOTHING_NATIVE`]) and runs nothing more.
    CheckNative,
    /// The list of the checks: those of `check --full` if `true`.
    List(bool),
    /// One part of the full check.
    CheckPart(Part),
    /// The list of the parts of the full check.
    ListParts,
    /// Every mutant of one target.
    Mutants(Target),
    /// Rewrite the reference hashes, with these options of `audio-cli bless`.
    Bless(Vec<String>),
    /// Rewrite the benchmark baseline.
    BlessBenchmarks,
    /// Check the commit message in this file.
    CommitMsg(String),
    /// Check what is pushed (each object with the remote ref it updates), then every check:
    /// every mutant if a tag is pushed, since a release requires the full check.
    PrePush(Vec<(String, String)>),
    /// The parts a run of the workflow checks: its event, the last commit's message and
    /// the parts given by hand.
    CiParts(String, String, String),
    /// The usage.
    Help,
}

/// Reads the arguments (the program name excluded): every one must be UTF-8 text, then
/// read as a command ([`command`], where git, through `runner`, resolves the revision of
/// `mutants --changed <revision>`, or proposes one when it is missing: [`revision`],
/// [`no_revision`]).
///
/// # Errors
///
/// Returns the usage error, in one line ([`Failure::usage`]), or why git could not resolve
/// the revision ([`revision`]).
pub(crate) fn task(args: &[OsString], runner: &mut dyn Runner) -> Result<Task, Failure> {
    let args = args
        .iter()
        .map(|arg| {
            arg.to_str().map(str::to_owned).ok_or_else(|| {
                usage_error(format!(
                    "argument is not valid UTF-8: {}",
                    arg.to_string_lossy()
                ))
            })
        })
        .collect::<Result<Vec<String>, Failure>>()?;
    command(&args, runner)
}

/// Reads the command of `args`, every argument text: the one reader of xtask's command
/// line, `mutants --changed <revision>` included, whose revision git resolves
/// ([`revision`]).
///
/// # Errors
///
/// Returns the usage error, in one line ([`Failure::usage`]), or why git could not resolve
/// the revision ([`revision`]).
fn command(args: &[String], runner: &mut dyn Runner) -> Result<Task, Failure> {
    let alone = ["bless-benchmarks", "help"];
    let read = match args {
        [] => Err("no command given".to_owned()),
        [command] if command == "check" => Ok(Task::Check(false)),
        [command, flag] if command == "check" && flag == "--full" => Ok(Task::Check(true)),
        [command, flag] if command == "check" && flag == "--gates" => Ok(Task::CheckGates),
        [command, flag] if command == "check" && flag == "--native" => Ok(Task::CheckNative),
        [command] if command == "list" => Ok(Task::List(false)),
        [command, flag] if command == "list" && flag == "--full" => Ok(Task::List(true)),
        [command, full, flag] if command == "list" && full == "--full" && flag == "--parts" => {
            Ok(Task::ListParts)
        }
        [command, full, flag, name]
            if command == "check" && full == "--full" && flag == "--part" =>
        {
            full_parts()
                .into_iter()
                .find(|part| part.name() == *name)
                .map(Task::CheckPart)
                .ok_or_else(|| {
                    format!("unknown part \"{name}\" (see: cargo xtask list --full --parts)")
                })
        }
        [command, options @ ..] if command == "bless" => {
            if options.iter().any(|option| option == SPENT_OPTION) {
                Err(format!(
                    "`bless` gives `{SPENT_OPTION}` itself: the spent record"
                ))
            } else {
                Ok(Task::Bless(options.to_vec()))
            }
        }
        [command] if command == "bless-benchmarks" => Ok(Task::BlessBenchmarks),
        [command] if command == "help" => Ok(Task::Help),
        [command, file] if command == "commit-msg" => Ok(Task::CommitMsg(file.clone())),
        [command, event, message, parts] if command == "ci-parts" => {
            Ok(Task::CiParts(event.clone(), message.clone(), parts.clone()))
        }
        [command, ..] if command == "ci-parts" => Err(
            "`ci-parts` takes an event, a commit message and the parts given by hand".to_owned(),
        ),
        [command, flag] if command == "mutants" && flag == CHANGED_LINES => {
            Err(no_revision(runner))
        }
        [command, flag, name] if command == "mutants" && flag == CHANGED_LINES => {
            return revision(runner, name).map(|since| Task::Mutants(Target::ChangedLines(since)));
        }
        [command, target] if command == "mutants" => mutation_target(target).map(Task::Mutants),
        [command, ..] if command == "mutants" => Err(format!(
            "`mutants` takes one target, or `{CHANGED_LINES} <revision>`"
        )),
        [command, pushed @ ..] if command == "pre-push" => {
            let (pairs, rest) = pushed.as_chunks::<2>();
            if rest.is_empty() {
                Ok(Task::PrePush(
                    pairs
                        .iter()
                        .map(|[commit, remote]| (commit.clone(), remote.clone()))
                        .collect(),
                ))
            } else {
                Err("`pre-push` takes pairs of a commit and the remote ref it updates".to_owned())
            }
        }
        [command, ..] if alone.contains(&command.as_str()) => {
            Err(format!("`{command}` takes no argument"))
        }
        [command, ..] if command == "check" => Err(
            "`check` takes no argument but `--full`, then `--part <part>`, `--gates` or `--native`"
                .to_owned(),
        ),
        [command, ..] if command == "list" => {
            Err("`list` takes no argument but `--full`, then `--parts`".to_owned())
        }
        [command, ..] if command == "commit-msg" => Err("`commit-msg` takes one file".to_owned()),
        [command, ..] => Err(format!("unknown command \"{command}\"")),
    };
    read.map_err(usage_error)
}

/// Entry point: runs the command named by `args` (the program name excluded) and returns
/// the exit code, as every command-line tool of the repository does.
///
/// 0 on success. 1 when a check or a command failed: one `error: …` line on `err`, its
/// details on `out`. 2 for a usage error: one `error: … (see: cargo xtask help)` line on
/// `err`, nothing on `out`, or, for options `bless` passed on that `audio-cli` refused,
/// after the steps that ran and their lines on `out`. Only `help` prints the usage, on `out`. The
/// error line and each line of the details go through [`safe_message`]: escaped, and bounded.
pub fn cli(
    args: &[OsString],
    runner: &mut dyn Runner,
    out: &mut dyn Write,
    err: &mut dyn Write,
) -> u8 {
    // A file target must exist: cargo-mutants and Stryker would find nothing in a missing
    // one, and pass.
    let checked = task(args, runner).and_then(|task| {
        if let Task::Mutants(target) = &task
            && let Some(path) = target.file()
        {
            let _text = runner
                .read(&path)
                .map_err(|error| usage_error(format!("cannot read {path}: {error}")))?;
        }
        Ok(task)
    });
    let task = match checked {
        Ok(task) => task,
        Err(failure) => return report(&failure, out, err),
    };
    let jobs = mutation_jobs(runner.parallelism());
    let mode = |full: bool| {
        if full {
            Mode::Full { jobs }
        } else {
            Mode::Changes { jobs }
        }
    };
    let result = match task {
        Task::Check(full) => {
            let mode = mode(full);
            run_steps_then(&check_steps(mode), runner, out, mode.on_failure())
        }
        Task::CheckGates => run_steps_then(&gates_steps(), runner, out, OnFailure::RunTheRest),
        // What is checked is what is committed, as the pre-push hook requires of its check:
        // the push that follows carries the commits, never the working tree.
        Task::CheckNative => run_steps(&opening_gates(), runner, out)
            .and_then(|()| committed(runner, &[], "the native check cannot run"))
            .and_then(|commit| native_parts(runner, out, &commit))
            .and_then(|parts| {
                // With no part, the tree's framing proves nothing: said as what happened.
                if parts.is_empty() {
                    say(out, NOTHING_NATIVE)
                } else {
                    run_steps(&native_steps(&parts), runner, out)
                }
            }),
        Task::CheckPart(part) => {
            run_steps_then(&part_steps(part, jobs), runner, out, OnFailure::RunTheRest)
        }
        Task::ListParts => full_parts()
            .into_iter()
            .try_for_each(|part| say(out, &part.name())),
        Task::Bless(options) => run_steps(
            &[opening_gates().as_slice(), &[bless_step(&options)]].concat(),
            runner,
            out,
        ),
        Task::BlessBenchmarks => bless_benchmarks(runner, out),
        Task::CommitMsg(path) => {
            read_file(runner, &path).and_then(|message| commit_msg_gate(&message))
        }
        Task::PrePush(pushed) => {
            let commits: Vec<String> = pushed.iter().map(|(commit, _)| commit.clone()).collect();
            let release = pushed
                .iter()
                .any(|(_, remote)| remote.starts_with("refs/tags/"));
            let mode = mode(release);
            pre_push(runner, &commits)
                .and_then(|()| run_steps_then(&check_steps(mode), runner, out, mode.on_failure()))
        }
        Task::Mutants(target) => {
            let steps = mutants_steps(&target, jobs);
            run_steps(&steps, runner, out)
        }
        Task::CiParts(event, message, parts) => ci_parts(&event, &message, &parts)
            .map_err(Failure::from)
            .and_then(|parts| {
                ci_outputs(&parts)
                    .iter()
                    .try_for_each(|line| say(out, line))
            }),
        Task::Help => say(out, USAGE),
        Task::List(full) => {
            check_steps(mode(full))
                .iter()
                .enumerate()
                .try_for_each(|(index, step)| {
                    say(
                        out,
                        &format!("{:>2}. {}", index.saturating_add(1), step.name),
                    )
                })
        }
    };
    result.map_or_else(|failure| report(&failure, out, err), |()| 0)
}

/// Writes `failure` as every command-line tool of the repository does, and returns its exit
/// code: its details on `out`, one line each, then its one `error:` line on `err`, every line
/// made safe ([`safe_message`]); a usage error ([`Failure::usage`]) points to the usage and
/// exits with 2, any other failure with 1.
fn report(failure: &Failure, out: &mut dyn Write, err: &mut dyn Write) -> u8 {
    // Nothing more can be done if an output itself is broken: the exit code still fails.
    // Each line of a detail on a line of its own, every line made safe.
    for line in failure.details.iter().flat_map(|detail| detail.lines()) {
        drop(writeln!(out, "{}", safe_message(line)));
    }
    // A usage error, xtask's own or one a command xtask passed its arguments on to
    // reported: the pointer to the usage after the bounded reason, which no reason can cut.
    let (pointer, code) = if failure.usage {
        (" (see: cargo xtask help)", 2)
    } else {
        ("", 1)
    };
    drop(writeln!(
        err,
        "error: {}{pointer}",
        safe_message(&failure.summary)
    ));
    code
}

/// Writes to `W`, treating a closed reader (a broken pipe, as with `| head`) as success:
/// what nobody reads is not an error, and the exit code stays the outcome of the checks.
#[derive(Debug)]
pub struct BestEffort<W>(pub W);

impl<W: Write> Write for BestEffort<W> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        match self.0.write(buf) {
            Err(error) if error.kind() == io::ErrorKind::BrokenPipe => Ok(buf.len()),
            other => other,
        }
    }

    fn flush(&mut self) -> io::Result<()> {
        match self.0.flush() {
            Err(error) if error.kind() == io::ErrorKind::BrokenPipe => Ok(()),
            other => other,
        }
    }
}

#[cfg(test)]
#[path = "cli_tests.rs"]
mod tests;
