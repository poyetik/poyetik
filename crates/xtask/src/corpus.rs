//! Corpus adequacy by mutation: the visual conformance corpus alone kills every mutant of
//! the code that produces image bytes.
//!
//! The WebAssembly modules, the page module and the host modes prove their images only by
//! replaying the corpus (`conformance/visual`): a mistake of the rendering code that no
//! case reaches is one no hash shows there, however well the native unit tests catch it.
//! So the rendering code is mutated with the corpus's check as its only test
//! ([`corpus_mutants_command`]), and every mutant it lets survive is either a gap of the
//! corpus, which fails the check, or a mutant that cannot change an image byte, reviewed in
//! [`CORPUS_EXEMPT`] with its reason.
//!
//! It holds every operator of cargo-mutants (27.1.0), and those are blind to low bits: none
//! clears or flips one bit of a value (`& !1`, `^ 1`), so a corpus that never reaches an odd
//! coverage passes this step as well as one that does. What holds the corpus's reach of the
//! low bits is the reach test of `visual-cli` (`corpus_reach.rs`), which requires every
//! coverage bit and every sub-pixel bit of each edge of the square reached.

use core::num::NonZeroUsize;
use std::io::Write;

use crate::{
    actions::{git_output, read_file, say},
    changes::{MUTATE_EVERYTHING_WHEN, changed_since, mutation_base, nul_separated},
    reviews::{compare_reviews, count, mutant_key},
    runner::{Exit, Failed, Runner},
    steps::{Action, Cmd, Dir, Failure, MUTANTS_ARGS, MUTANTS_MISSED_EXIT, Step, verdict},
    workspace::{Extent, Mode},
};

/// The files of the rendering code that produces image bytes, which the step mutates.
///
/// The square's placement and coverage, and its per-pixel writes (`visual-boxes`); exact
/// coverage, the image and its rows, the renderer's sequence, and the display encoding
/// (`visual-core`), whose bytes the corpus's `display.json` holds. The image hash
/// (`visual-hash`) is proven by its own vectors, and the case reader (`visual-cli`) by the
/// rejection corpus and its tests: neither writes an image byte.
pub const CORPUS_MUTATED: [&str; 5] = [
    "crates/visual-boxes/src/square.rs",
    "crates/visual-core/src/coverage.rs",
    "crates/visual-core/src/image.rs",
    "crates/visual-core/src/render.rs",
    "crates/visual-core/src/display.rs",
];

/// The reviewed mutants of [`CORPUS_MUTATED`] the corpus does not kill, each with why it
/// cannot change an image byte: `<file>.rs: <mutant> -- <reason>`, as
/// `mutants-unviable.txt`.
pub const CORPUS_EXEMPT: &str = "mutants-corpus-exempt.txt";

/// Where cargo-mutants writes the step's report (git-ignored, as every `mutants.out*`).
pub const CORPUS_MUTANTS_OUT: &str = "mutants.out-corpus";

/// The report's mutants no test caught.
pub const CORPUS_MISSED: &str = "mutants.out-corpus/mutants.out/missed.txt";

/// The one test of the step, as nextest selects it: `visual-cli`'s check of the
/// repository's corpus, through its binary (in the package `visual-cli`).
///
/// That test checks every case but the largest image (2²⁵ pixels, about 15 s unoptimized),
/// which the optimized "Visual conformance (native)" step checks: a subset of the corpus
/// that kills every mutant proves that the whole corpus does.
pub const CORPUS_TEST: [&str; 3] = [
    "--test=binary",
    "-E",
    "test(=tests::checks_the_repository_corpus)",
];

/// The longest a mutant's corpus test may run alone, in seconds, before it counts as hung.
///
/// Scaled by the jobs as every mutation run's timeout is
/// ([`mutant_timeout_args`](crate::steps::mutant_timeout_args)): the test takes about 3 s
/// alone (2026-11-20, an Apple M2 Pro).
pub const CORPUS_TEST_TIMEOUT: usize = 60;

/// What a change of reaches the step in the default check, which runs it only then.
///
/// The visual library's crates the check builds, the corpus, the reviewed exceptions, the
/// workspace's manifest and lock file, what every Rust test depends on
/// ([`MUTATE_EVERYTHING_WHEN`]), nextest's configuration, and the code of the step itself
/// and of the arguments it shares with every mutation run. A path is reached when it starts
/// with one of them.
pub const CORPUS_INPUTS: [&str; 11] = [
    "crates/visual-core/",
    "crates/visual-boxes/",
    "crates/visual-hash/",
    "crates/visual-cli/",
    "conformance/visual/",
    CORPUS_EXEMPT,
    "Cargo.toml",
    "Cargo.lock",
    ".config/nextest.toml",
    "crates/xtask/src/corpus.rs",
    "crates/xtask/src/steps.rs",
];

/// The corpus test alone, on the unmutated tree.
///
/// What cargo-mutants' own baseline would prove, which its version 27.1.0 runs on the mutated packages instead of the test
/// package given (`--test-package`), so the step skips it (`--baseline=skip`). It fails
/// when the test fails, and when its selection finds no test (nextest's default). Built
/// with the lock file and the test profile of the mutants (`MUTANTS_ARGS`).
#[must_use]
pub fn corpus_baseline_command() -> Cmd {
    Cmd::new(
        "cargo",
        &[
            &["nextest", "run", "--package=visual-cli"][..],
            &CORPUS_TEST[..],
            &["--locked", "--config=profile.test.debug=false"][..],
        ]
        .concat(),
        Dir::Root,
    )
}

/// The cargo-mutants command of the step, in `jobs` jobs: every mutant of
/// [`CORPUS_MUTATED`], tested by [`CORPUS_TEST`] alone.
///
/// The arguments of every run (`MUTANTS_ARGS`) but its report, written to
/// [`CORPUS_MUTANTS_OUT`]; the test package `visual-cli` (`--test-package`) whatever
/// package a mutant is in; no baseline ([`corpus_baseline_command`] runs it first), so a
/// fixed timeout ([`CORPUS_TEST_TIMEOUT`]); and the test selection given to the test runs
/// only (`--cargo-test-arg`), nextest refusing it while building.
#[must_use]
pub fn corpus_mutants_command(jobs: NonZeroUsize) -> Cmd {
    let output = format!("--output={CORPUS_MUTANTS_OUT}");
    let timeout = CORPUS_TEST_TIMEOUT.saturating_mul(jobs.get()).to_string();
    let jobs = jobs.to_string();
    let mut args: Vec<String> = MUTANTS_ARGS
        .iter()
        .map(|&arg| {
            if arg == "--output=." {
                output.clone()
            } else {
                arg.to_owned()
            }
        })
        .collect();
    for file in CORPUS_MUTATED {
        args.extend(["--file".to_owned(), file.to_owned()]);
    }
    args.extend(
        [
            "--test-package=visual-cli",
            "--baseline=skip",
            "--timeout",
            &timeout,
            "--jobs",
            &jobs,
        ]
        .map(str::to_owned),
    );
    // The package and the build settings are cargo-mutants' own: only the target and the
    // filter are the test runs'.
    args.extend(
        CORPUS_TEST
            .iter()
            .map(|arg| format!("--cargo-test-arg={arg}")),
    );
    let args: Vec<&str> = args.iter().map(String::as_str).collect();
    Cmd::new("cargo", &args, Dir::Root)
}

/// Checks that the mutants the corpus missed (`missed`, as cargo-mutants writes
/// `missed.txt`) are exactly the reviewed ones of `reviewed` ([`CORPUS_EXEMPT`]).
///
/// The run mutates every file of [`CORPUS_MUTATED`] whole, so a review of a mutant it no
/// longer misses is stale. A mutant is known by its file and description, not its line, as
/// in the review of unviable mutants.
///
/// # Errors
///
/// Returns every mutant missed and not reviewed, every stale review, every review without
/// a reason and every line that is not a mutant.
pub fn corpus_gate(missed: &str, reviewed: &str) -> Result<(), Failure> {
    let mut problems = Vec::new();
    let mut keys = Vec::new();
    for line in missed.lines().filter(|line| !line.trim().is_empty()) {
        match mutant_key(line) {
            Some(key) => keys.push(key),
            None => problems.push(format!("unrecognized mutant: {line}")),
        }
    }
    compare_reviews(&count(keys), Extent::Complete, reviewed, &mut problems);
    verdict(
        &format!(
            "the visual corpus must kill every mutant of the rendering code: add a case that \
             tells it apart, or review one that cannot change an image byte in {CORPUS_EXEMPT}"
        ),
        problems,
    )
}

/// The step in `mode`, after emptying its report ([`CORPUS_MUTANTS_OUT`]): what the checks
/// and `cargo xtask mutants conformance/visual` run.
#[must_use]
pub fn corpus_steps(mode: Mode) -> [Step; 2] {
    [
        Step {
            name: "Fresh visual corpus mutation report".to_owned(),
            action: Action::FreshDirs(vec![CORPUS_MUTANTS_OUT.to_owned()]),
        },
        Step {
            name: "Visual corpus kills every mutant of the rendering code".to_owned(),
            action: Action::CorpusMutants(mode),
        },
    ]
}

/// The first changed path the step's inputs hold ([`CORPUS_INPUTS`],
/// [`MUTATE_EVERYTHING_WHEN`]), if any.
#[must_use]
pub fn corpus_reached(changed: &[String]) -> Option<&str> {
    changed.iter().map(String::as_str).find(|path| {
        CORPUS_INPUTS
            .iter()
            .chain(&MUTATE_EVERYTHING_WHEN)
            .any(|input| path.starts_with(input))
    })
}

/// Runs the step in `mode`: in the default check only when a change since the merge base
/// reaches it ([`corpus_reached`], the untracked files included), in the full check always;
/// then the corpus test unmutated ([`corpus_baseline_command`]), the mutants
/// ([`corpus_mutants_command`]), and the gate on what they missed ([`corpus_gate`]).
///
/// # Errors
///
/// Returns why the changes cannot be listed, why the unmutated test failed, why the run
/// failed but by missing mutants (a timeout among them), why its report cannot be read, or
/// the gate's refusal.
pub(crate) fn corpus_mutants(
    runner: &mut dyn Runner,
    out: &mut dyn Write,
    mode: Mode,
) -> Result<(), Failure> {
    let jobs = match mode {
        Mode::Changes { jobs } => {
            let (base, against) = mutation_base(runner)?;
            let mut changed = changed_since(runner, &base)?;
            changed.extend(nul_separated(&git_output(
                runner,
                &["ls-files", "-z", "--others", "--exclude-standard"],
                "list the untracked files",
            )?));
            match corpus_reached(&changed) {
                Some(path) => say(out, &format!("  {path} changed: the corpus is mutated"))?,
                None => {
                    return say(
                        out,
                        &format!(
                            "  no change since {base} (merge base with {against}) reaches \
                             the visual corpus or its rendering code: not run"
                        ),
                    );
                }
            }
            jobs
        }
        Mode::Full { jobs } => jobs,
    };
    runner
        .run(&corpus_baseline_command())
        .map_err(|failure| Failure::from(format!("the unmutated corpus test: {failure}")))?;
    match runner.run(&corpus_mutants_command(jobs)) {
        Ok(()) => {}
        Err(failure)
            if matches!(
                failure.failed,
                Failed::Ended {
                    exit: Exit::Code(MUTANTS_MISSED_EXIT),
                    ..
                }
            ) => {}
        Err(failure) => return Err(Failure::from(failure.to_string())),
    }
    let missed = read_file(runner, CORPUS_MISSED)?;
    let reviewed = read_file(runner, CORPUS_EXEMPT)?;
    corpus_gate(&missed, &reviewed)
}

#[cfg(test)]
#[path = "corpus_tests.rs"]
mod tests;
